//! Runtime job queues and worker orchestration (slim, engine-only).
#![forbid(unsafe_code)]

mod column_cache;
mod gen_ctx_pool;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, TryRecvError, select, unbounded};
use geist_blocks::{Block, BlockRegistry};
use geist_chunk as chunkbuf;
use geist_lighting::{
    LightAtlas, LightBorders, LightGrid, LightingStore, compute_light_with_borders_buf,
};
use geist_mesh_cpu::{
    ChunkMeshCPU, NeighborsLoaded, build_chunk_wcc_cpu_buf_with_light, build_structure_wcc_cpu_buf,
};
use geist_world::{ChunkCoord, TerrainMetrics, World, voxel::generation::ChunkColumnProfile};
use hashbrown::HashMap;
use rayon::{ThreadPool, ThreadPoolBuilder};

use crate::column_cache::ChunkColumnCache;
use crate::gen_ctx_pool::GenCtxPool;

#[derive(Clone, Debug)]
pub struct BuildJob {
    pub cx: i32,
    pub cy: i32,
    pub cz: i32,
    pub neighbors: NeighborsLoaded,
    pub rev: u64,
    pub job_id: u64,
    pub chunk_edits: Vec<((i32, i32, i32), Block)>,
    pub region_edits: HashMap<(i32, i32, i32), Block>,
    pub prev_buf: Option<chunkbuf::ChunkBuf>,
    pub reg: Arc<BlockRegistry>,
    pub column_profile: Option<Arc<ChunkColumnProfile>>,
}

pub struct JobOut {
    pub cpu: Option<ChunkMeshCPU>,
    pub light_atlas: Option<LightAtlas>,
    pub light_grid: Option<LightGrid>,
    pub buf: Option<chunkbuf::ChunkBuf>,
    pub light_borders: Option<LightBorders>,
    pub cx: i32,
    pub cy: i32,
    pub cz: i32,
    pub rev: u64,
    pub job_id: u64,
    pub occupancy: chunkbuf::ChunkOccupancy,
    pub kind: JobKind,
    pub t_total_ms: u32,
    pub t_gen_ms: u32,
    pub t_apply_ms: u32,
    pub t_light_ms: u32,
    pub t_mesh_ms: u32,
    pub terrain_metrics: TerrainMetrics,
    pub column_profile: Option<Arc<ChunkColumnProfile>>,
}

#[derive(Clone, Debug)]
pub struct StructureBuildJob {
    pub id: u32,
    pub rev: u64,
    pub sx: usize,
    pub sy: usize,
    pub sz: usize,
    pub base_blocks: Arc<[Block]>,
    pub edits: Vec<((i32, i32, i32), Block)>,
    pub reg: Arc<BlockRegistry>,
}

pub struct StructureJobOut {
    pub id: u32,
    pub rev: u64,
    pub cpu: ChunkMeshCPU,
    pub light_grid: LightGrid,
    pub light_borders: LightBorders,
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum Lane {
    Edit,
    Light,
    Bg,
}

#[derive(Clone, Copy, Debug)]
pub enum JobKind {
    Edit,
    Light,
    Bg,
}

fn process_build_job(
    job: BuildJob,
    lane: Lane,
    world: &World,
    lighting: &LightingStore,
    ctx_pool: &GenCtxPool,
    tx: &Sender<JobOut>,
) {
    let BuildJob {
        cx,
        cy,
        cz,
        rev,
        job_id,
        chunk_edits,
        region_edits,
        prev_buf,
        reg,
        column_profile,
        ..
    } = job;

    let t_job_start = Instant::now();
    let mut t_gen_ms: u32 = 0;
    let mut t_mesh_ms: u32 = 0;
    let coord = ChunkCoord::new(cx, cy, cz);

    let mut column_profile_out = column_profile.clone();

    let (mut buf, mut occupancy, terrain_metrics) = if let Some(prev) = prev_buf {
        let occ = if prev.has_non_air() {
            chunkbuf::ChunkOccupancy::Populated
        } else {
            chunkbuf::ChunkOccupancy::Empty
        };
        (prev, occ, TerrainMetrics::default())
    } else if let Some(profile) = column_profile.clone() {
        let t0 = Instant::now();
        let mut pooled_ctx = ctx_pool.acquire(world);
        let generated = chunkbuf::generate_chunk_buffer_from_profile(
            world,
            coord,
            &reg,
            &mut pooled_ctx,
            profile.as_ref(),
        );
        t_gen_ms = t0.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
        column_profile_out = Some(profile);
        (
            generated.buf,
            generated.occupancy,
            generated.terrain_metrics,
        )
    } else {
        let t0 = Instant::now();
        let mut pooled_ctx = ctx_pool.acquire(world);
        let generated =
            chunkbuf::generate_chunk_buffer_with_ctx(world, coord, &reg, &mut pooled_ctx);
        t_gen_ms = t0.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
        column_profile_out = generated.column_profile.map(Arc::new);
        (
            generated.buf,
            generated.occupancy,
            generated.terrain_metrics,
        )
    };

    let base_x = cx * buf.sx as i32;
    let base_y = cy * buf.sy as i32;
    let base_z = cz * buf.sz as i32;

    let mut applied_chunk_edit = false;
    let t_apply_ms = {
        let t0 = Instant::now();
        for ((wx, wy, wz), b) in chunk_edits.iter().copied() {
            if wy < base_y || wy >= base_y + buf.sy as i32 {
                continue;
            }
            let lx = (wx - base_x) as usize;
            let ly = (wy - base_y) as usize;
            let lz = (wz - base_z) as usize;
            if lx < buf.sx && lz < buf.sz {
                let idx = buf.idx(lx, ly, lz);
                buf.blocks[idx] = b;
                applied_chunk_edit = true;
            }
        }
        t0.elapsed().as_millis().min(u128::from(u32::MAX)) as u32
    };

    if applied_chunk_edit {
        occupancy = if buf.has_non_air() {
            chunkbuf::ChunkOccupancy::Populated
        } else {
            chunkbuf::ChunkOccupancy::Empty
        };
    }

    let region_edits_ref = if region_edits.is_empty() {
        None
    } else {
        Some(&region_edits)
    };

    let job_kind = match lane {
        Lane::Edit => JobKind::Edit,
        Lane::Light => JobKind::Light,
        Lane::Bg => JobKind::Bg,
    };

    if !occupancy.has_blocks() {
        let t_total_ms = t_job_start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
        let _ = tx.send(JobOut {
            cpu: None,
            light_atlas: None,
            light_grid: None,
            buf: None,
            light_borders: None,
            cx,
            cy,
            cz,
            rev,
            job_id,
            occupancy,
            kind: job_kind,
            t_total_ms,
            t_gen_ms,
            t_apply_ms,
            t_light_ms: 0,
            t_mesh_ms,
            terrain_metrics,
            column_profile: column_profile_out.clone(),
        });
        return;
    }

    match lane {
        Lane::Light => {
            let t0 = Instant::now();
            let lg = compute_light_with_borders_buf(&buf, lighting, &reg, world);
            let t_light_ms = t0.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
            let borders = LightBorders::from_grid(&lg);
            let t_total_ms = t_job_start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
            let _ = tx.send(JobOut {
                cpu: None,
                light_atlas: None,
                light_grid: Some(lg),
                buf: Some(buf),
                light_borders: Some(borders),
                cx,
                cy,
                cz,
                rev,
                job_id,
                occupancy,
                kind: job_kind,
                t_total_ms,
                t_gen_ms,
                t_apply_ms,
                t_light_ms,
                t_mesh_ms,
                terrain_metrics,
                column_profile: column_profile_out.clone(),
            });
        }
        Lane::Edit | Lane::Bg => {
            let t0 = Instant::now();
            let lg = compute_light_with_borders_buf(&buf, lighting, &reg, world);
            let t_light_ms = t0.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
            let t0 = Instant::now();
            let built =
                build_chunk_wcc_cpu_buf_with_light(&buf, &lg, world, region_edits_ref, coord, &reg);
            t_mesh_ms = t0.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
            if let Some((cpu, light_borders)) = built {
                let t_total_ms = t_job_start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
                let _ = tx.send(JobOut {
                    cpu: Some(cpu),
                    light_atlas: None,
                    light_grid: Some(lg),
                    buf: Some(buf),
                    light_borders,
                    cx,
                    cy,
                    cz,
                    rev,
                    job_id,
                    occupancy,
                    kind: job_kind,
                    t_total_ms,
                    t_gen_ms,
                    t_apply_ms,
                    t_light_ms,
                    t_mesh_ms,
                    terrain_metrics,
                    column_profile: column_profile_out,
                });
            }
        }
    }
}

pub struct Runtime {
    job_tx_edit: Sender<BuildJob>,
    job_tx_light: Sender<BuildJob>,
    job_tx_bg: Sender<BuildJob>,
    res_rx: Receiver<JobOut>,
    _edit_pool: Option<Arc<ThreadPool>>,
    light_pool: Option<Arc<ThreadPool>>,
    bg_pool: Option<Arc<ThreadPool>>,
    s_job_tx: Sender<StructureBuildJob>,
    s_res_rx: Receiver<StructureJobOut>,
    q_edit: Arc<AtomicUsize>,
    q_light: Arc<AtomicUsize>,
    q_bg: Arc<AtomicUsize>,
    inflight_edit: Arc<AtomicUsize>,
    inflight_light: Arc<AtomicUsize>,
    inflight_bg: Arc<AtomicUsize>,
    pub w_edit: usize,
    pub w_light: usize,
    pub w_bg: usize,
    _ctx_pool: Arc<GenCtxPool>,
    column_cache: Arc<ChunkColumnCache>,
}

impl Runtime {
    pub fn new(world: Arc<World>, lighting: Arc<LightingStore>) -> Self {
        let (job_tx_edit, job_rx_edit) = unbounded::<BuildJob>();
        let (job_tx_light, job_rx_light) = unbounded::<BuildJob>();
        let (job_tx_bg, job_rx_bg) = unbounded::<BuildJob>();
        let (res_tx, res_rx) = unbounded::<JobOut>();
        let (s_job_tx, s_job_rx) = unbounded::<StructureBuildJob>();
        let (s_res_tx, s_res_rx) = unbounded::<StructureJobOut>();

        let worker_count: usize = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8);
        let w_edit = 1usize;
        let remaining = worker_count.saturating_sub(w_edit);
        let w_light = if remaining >= 2 { 1 } else { 0 };
        let w_bg = remaining.saturating_sub(w_light);
        let total_workers = w_edit + w_light + w_bg;
        let ctx_pool = GenCtxPool::with_capacity_from_workers(total_workers);
        let cache_capacity = (world.chunks_x.max(4) * world.chunks_z.max(4) * 4).max(64);
        let column_cache = Arc::new(ChunkColumnCache::new(cache_capacity));

        let q_edit_ctr = Arc::new(AtomicUsize::new(0));
        let q_light_ctr = Arc::new(AtomicUsize::new(0));
        let q_bg_ctr = Arc::new(AtomicUsize::new(0));
        let inflight_edit_ctr = Arc::new(AtomicUsize::new(0));
        let inflight_light_ctr = Arc::new(AtomicUsize::new(0));
        let inflight_bg_ctr = Arc::new(AtomicUsize::new(0));

        let edit_pool = if w_edit > 0 {
            let pool = Arc::new(
                ThreadPoolBuilder::new()
                    .num_threads(w_edit)
                    .thread_name(|i| format!("geist-edit-{i}"))
                    .build()
                    .expect("edit pool"),
            );
            for _ in 0..w_edit {
                let rx = job_rx_edit.clone();
                let tx = res_tx.clone();
                let world = world.clone();
                let lighting = lighting.clone();
                let q_edit = q_edit_ctr.clone();
                let inflight_edit = inflight_edit_ctr.clone();
                let ctx_pool = ctx_pool.clone();
                pool.spawn(move || {
                    while let Ok(job) = rx.recv() {
                        q_edit.fetch_sub(1, Ordering::Relaxed);
                        inflight_edit.fetch_add(1, Ordering::Relaxed);
                        process_build_job(
                            job,
                            Lane::Edit,
                            world.as_ref(),
                            lighting.as_ref(),
                            ctx_pool.as_ref(),
                            &tx,
                        );
                        inflight_edit.fetch_sub(1, Ordering::Relaxed);
                    }
                });
            }
            Some(pool)
        } else {
            None
        };

        let light_pool = if w_light > 0 {
            let pool = Arc::new(
                ThreadPoolBuilder::new()
                    .num_threads(w_light)
                    .thread_name(|i| format!("geist-light-{i}"))
                    .build()
                    .expect("light pool"),
            );
            for _ in 0..w_light {
                let rx = job_rx_light.clone();
                let tx = res_tx.clone();
                let world = world.clone();
                let lighting = lighting.clone();
                let q_light = q_light_ctr.clone();
                let inflight_light = inflight_light_ctr.clone();
                let ctx_pool = ctx_pool.clone();
                pool.spawn(move || {
                    while let Ok(job) = rx.recv() {
                        q_light.fetch_sub(1, Ordering::Relaxed);
                        inflight_light.fetch_add(1, Ordering::Relaxed);
                        process_build_job(
                            job,
                            Lane::Light,
                            world.as_ref(),
                            lighting.as_ref(),
                            ctx_pool.as_ref(),
                            &tx,
                        );
                        inflight_light.fetch_sub(1, Ordering::Relaxed);
                    }
                });
            }
            Some(pool)
        } else {
            None
        };

        let bg_pool = if w_bg > 0 {
            let pool = Arc::new(
                ThreadPoolBuilder::new()
                    .num_threads(w_bg)
                    .thread_name(|i| format!("geist-bg-{i}"))
                    .build()
                    .expect("bg pool"),
            );
            for _ in 0..w_bg {
                let bg_rx = job_rx_bg.clone();
                let light_rx = job_rx_light.clone();
                let tx = res_tx.clone();
                let world = world.clone();
                let lighting = lighting.clone();
                let q_bg = q_bg_ctr.clone();
                let inflight_bg = inflight_bg_ctr.clone();
                let q_light = q_light_ctr.clone();
                let inflight_light = inflight_light_ctr.clone();
                let ctx_pool = ctx_pool.clone();
                pool.spawn(move || {
                    loop {
                        match bg_rx.try_recv() {
                            Ok(job) => {
                                q_bg.fetch_sub(1, Ordering::Relaxed);
                                inflight_bg.fetch_add(1, Ordering::Relaxed);
                                process_build_job(
                                    job,
                                    Lane::Bg,
                                    world.as_ref(),
                                    lighting.as_ref(),
                                    ctx_pool.as_ref(),
                                    &tx,
                                );
                                inflight_bg.fetch_sub(1, Ordering::Relaxed);
                                continue;
                            }
                            Err(TryRecvError::Disconnected) => {
                                while let Ok(job) = light_rx.try_recv() {
                                    q_light.fetch_sub(1, Ordering::Relaxed);
                                    inflight_light.fetch_add(1, Ordering::Relaxed);
                                    process_build_job(
                                        job,
                                        Lane::Light,
                                        world.as_ref(),
                                        lighting.as_ref(),
                                        ctx_pool.as_ref(),
                                        &tx,
                                    );
                                    inflight_light.fetch_sub(1, Ordering::Relaxed);
                                }
                                break;
                            }
                            Err(TryRecvError::Empty) => {}
                        }

                        match light_rx.try_recv() {
                            Ok(job) => {
                                q_light.fetch_sub(1, Ordering::Relaxed);
                                inflight_light.fetch_add(1, Ordering::Relaxed);
                                process_build_job(
                                    job,
                                    Lane::Light,
                                    world.as_ref(),
                                    lighting.as_ref(),
                                    ctx_pool.as_ref(),
                                    &tx,
                                );
                                inflight_light.fetch_sub(1, Ordering::Relaxed);
                                continue;
                            }
                            Err(TryRecvError::Disconnected) => match bg_rx.recv() {
                                Ok(job) => {
                                    q_bg.fetch_sub(1, Ordering::Relaxed);
                                    inflight_bg.fetch_add(1, Ordering::Relaxed);
                                    process_build_job(
                                        job,
                                        Lane::Bg,
                                        world.as_ref(),
                                        lighting.as_ref(),
                                        ctx_pool.as_ref(),
                                        &tx,
                                    );
                                    inflight_bg.fetch_sub(1, Ordering::Relaxed);
                                    continue;
                                }
                                Err(_) => break,
                            },
                            Err(TryRecvError::Empty) => {}
                        }

                        select! {
                            recv(bg_rx) -> res => match res {
                                Ok(job) => {
                                    q_bg.fetch_sub(1, Ordering::Relaxed);
                                    inflight_bg.fetch_add(1, Ordering::Relaxed);
                                    process_build_job(
                                        job,
                                        Lane::Bg,
                                        world.as_ref(),
                                        lighting.as_ref(),
                                        ctx_pool.as_ref(),
                                        &tx,
                                    );
                                    inflight_bg.fetch_sub(1, Ordering::Relaxed);
                                }
                                Err(_) => {
                                    while let Ok(job) = light_rx.recv() {
                                        q_light.fetch_sub(1, Ordering::Relaxed);
                                        inflight_light.fetch_add(1, Ordering::Relaxed);
                                        process_build_job(
                                            job,
                                            Lane::Light,
                                            world.as_ref(),
                                            lighting.as_ref(),
                                            ctx_pool.as_ref(),
                                            &tx,
                                        );
                                        inflight_light.fetch_sub(1, Ordering::Relaxed);
                                    }
                                    break;
                                }
                            },
                            recv(light_rx) -> res => match res {
                                Ok(job) => {
                                    q_light.fetch_sub(1, Ordering::Relaxed);
                                    inflight_light.fetch_add(1, Ordering::Relaxed);
                                    process_build_job(
                                        job,
                                        Lane::Light,
                                        world.as_ref(),
                                        lighting.as_ref(),
                                        ctx_pool.as_ref(),
                                        &tx,
                                    );
                                    inflight_light.fetch_sub(1, Ordering::Relaxed);
                                }
                                Err(_) => {}
                            },
                        }
                    }
                });
            }
            Some(pool)
        } else {
            None
        };

        {
            let s_res_tx = s_res_tx.clone();
            let lighting = lighting.clone();
            thread::spawn(move || {
                while let Ok(job) = s_job_rx.recv() {
                    let mut buf = chunkbuf::ChunkBuf::from_blocks_local(
                        ChunkCoord::new(0, 0, 0),
                        job.sx,
                        job.sy,
                        job.sz,
                        job.base_blocks.to_vec(),
                    );
                    for ((lx, ly, lz), b) in job.edits.iter().copied() {
                        if lx < 0 || ly < 0 || lz < 0 {
                            continue;
                        }
                        let (lx, ly, lz) = (lx as usize, ly as usize, lz as usize);
                        if lx < buf.sx && ly < buf.sy && lz < buf.sz {
                            let idx = buf.idx(lx, ly, lz);
                            buf.blocks[idx] = b;
                        }
                    }
                    let skylight_max = lighting.skylight_max();
                    let local_store = LightingStore::new(buf.sx, buf.sy, buf.sz);
                    local_store.set_skylight_max(skylight_max);
                    let light_grid = LightGrid::compute_with_borders_buf(&buf, &local_store, &job.reg);
                    let light_borders = LightBorders::from_grid(&light_grid);
                    let cpu = build_structure_wcc_cpu_buf(&buf, &job.reg, None);
                    let _ = s_res_tx.send(StructureJobOut {
                        id: job.id,
                        rev: job.rev,
                        cpu,
                        light_grid,
                        light_borders,
                    });
                }
            });
        }

        Self {
            job_tx_edit,
            job_tx_light,
            job_tx_bg,
            res_rx,
            _edit_pool: edit_pool,
            light_pool,
            bg_pool,
            s_job_tx,
            s_res_rx,
            q_edit: q_edit_ctr,
            q_light: q_light_ctr,
            q_bg: q_bg_ctr,
            inflight_edit: inflight_edit_ctr,
            inflight_light: inflight_light_ctr,
            inflight_bg: inflight_bg_ctr,
            w_edit,
            w_light,
            w_bg,
            _ctx_pool: ctx_pool,
            column_cache,
        }
    }

    pub fn submit_build_job_edit(&self, job: BuildJob) {
        self.q_edit.fetch_add(1, Ordering::Relaxed);
        if self.job_tx_edit.send(job).is_err() {
            self.q_edit.fetch_sub(1, Ordering::Relaxed);
        }
    }

    pub fn submit_build_job_light(&self, job: BuildJob) {
        if self.light_pool.is_some() {
            self.q_light.fetch_add(1, Ordering::Relaxed);
            if self.job_tx_light.send(job).is_err() {
                self.q_light.fetch_sub(1, Ordering::Relaxed);
            }
        } else if self.bg_pool.is_some() {
            self.submit_build_job_bg(job);
        } else {
            self.submit_build_job_edit(job);
        }
    }

    pub fn submit_build_job_bg(&self, job: BuildJob) {
        if self.bg_pool.is_some() {
            self.q_bg.fetch_add(1, Ordering::Relaxed);
            if self.job_tx_bg.send(job).is_err() {
                self.q_bg.fetch_sub(1, Ordering::Relaxed);
            }
        } else {
            self.submit_build_job_edit(job);
        }
    }

    pub fn drain_worker_results(&self) -> Vec<JobOut> {
        self.res_rx.try_iter().collect()
    }

    pub fn column_cache(&self) -> Arc<ChunkColumnCache> {
        Arc::clone(&self.column_cache)
    }

    pub fn queue_debug_counts(&self) -> (usize, usize, usize, usize, usize, usize) {
        (
            self.q_edit.load(Ordering::Relaxed),
            self.inflight_edit.load(Ordering::Relaxed),
            self.q_light.load(Ordering::Relaxed),
            self.inflight_light.load(Ordering::Relaxed),
            self.q_bg.load(Ordering::Relaxed),
            self.inflight_bg.load(Ordering::Relaxed),
        )
    }

    pub fn submit_structure_build_job(&self, job: StructureBuildJob) {
        let _ = self.s_job_tx.send(job);
    }

    pub fn drain_structure_results(&self) -> Vec<StructureJobOut> {
        self.s_res_rx.try_iter().collect()
    }
}

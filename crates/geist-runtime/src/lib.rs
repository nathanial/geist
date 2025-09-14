//! Runtime job queues and worker orchestration (slim, engine-only).
#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use geist_blocks::{Block, BlockRegistry};
use geist_chunk as chunkbuf;
use geist_lighting::{LightAtlas, LightBorders, LightGrid, LightingStore, compute_light_with_borders_buf};
use geist_mesh_cpu::{ChunkMeshCPU, NeighborsLoaded, build_chunk_wcc_cpu_buf_with_light};
use geist_world::World;

#[derive(Clone, Debug)]
pub struct BuildJob {
    pub cx: i32,
    pub cz: i32,
    pub neighbors: NeighborsLoaded,
    pub rev: u64,
    pub job_id: u64,
    pub chunk_edits: Vec<((i32, i32, i32), Block)>,
    pub region_edits: Vec<((i32, i32, i32), Block)>,
    // Optional previous buffer to reuse instead of regenerating from worldgen
    pub prev_buf: Option<chunkbuf::ChunkBuf>,
    /// Runtime voxel registry for this job
    pub reg: Arc<BlockRegistry>,
}

pub struct JobOut {
    pub cpu: Option<ChunkMeshCPU>,
    pub light_atlas: Option<LightAtlas>,
    pub light_grid: Option<LightGrid>,
    pub buf: chunkbuf::ChunkBuf,
    pub light_borders: Option<LightBorders>,
    pub cx: i32,
    pub cz: i32,
    pub rev: u64,
    pub job_id: u64,
}

#[derive(Clone, Debug)]
pub struct StructureBuildJob {
    pub id: u32, // consumer-defined identifier
    pub rev: u64,
    pub sx: usize,
    pub sy: usize,
    pub sz: usize,
    pub base_blocks: Vec<Block>,
    pub edits: Vec<((i32, i32, i32), Block)>,
    pub reg: Arc<BlockRegistry>,
}

pub struct StructureJobOut {
    pub id: u32,
    pub rev: u64,
    pub cpu: ChunkMeshCPU,
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum Lane {
    Edit,
    Light,
    Bg,
}

pub struct Runtime {
    // Worker infra (three lanes: edit, light, bg)
    job_tx_edit: mpsc::Sender<BuildJob>,
    job_tx_light: mpsc::Sender<BuildJob>,
    job_tx_bg: mpsc::Sender<BuildJob>,
    res_rx: mpsc::Receiver<JobOut>,
    _edit_worker_txs: Vec<mpsc::Sender<BuildJob>>, // hold to keep senders alive
    _light_worker_txs: Vec<mpsc::Sender<BuildJob>>, // hold to keep senders alive
    _bg_worker_txs: Vec<mpsc::Sender<BuildJob>>,   // hold to keep senders alive
    // Structure worker infra
    s_job_tx: mpsc::Sender<StructureBuildJob>,
    s_res_rx: mpsc::Receiver<StructureJobOut>,
    // Debug counters
    q_edit: Arc<AtomicUsize>,
    q_light: Arc<AtomicUsize>,
    q_bg: Arc<AtomicUsize>,
    inflight_edit: Arc<AtomicUsize>,
    inflight_light: Arc<AtomicUsize>,
    inflight_bg: Arc<AtomicUsize>,
    // Worker allocation (public for scheduling heuristics)
    pub w_edit: usize,
    pub w_light: usize,
    pub w_bg: usize,
    // Track lane by job_id (for inflight decrement on completion)
    lane_by_job: std::sync::Mutex<std::collections::HashMap<u64, Lane>>,
}

impl Runtime {
    pub fn new(world: Arc<World>, lighting: Arc<LightingStore>) -> Self {
        // Worker threads (three lanes)
        let (job_tx_edit, job_rx_edit) = mpsc::channel::<BuildJob>();
        let (job_tx_light, job_rx_light) = mpsc::channel::<BuildJob>();
        let (job_tx_bg, job_rx_bg) = mpsc::channel::<BuildJob>();
        let (res_tx, res_rx) = mpsc::channel::<JobOut>();
        // Structure channels
        let (s_job_tx, s_job_rx) = mpsc::channel::<StructureBuildJob>();
        let (s_res_tx, s_res_rx) = mpsc::channel::<StructureJobOut>();
        let worker_count: usize = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8);
        // Split workers: ensure at least 1 edit worker; try to keep 1 light worker; rest bg
        let w_edit = 1usize;
        let remaining = worker_count.saturating_sub(w_edit);
        let w_light = if remaining >= 2 { 1 } else { 0 };
        let w_bg = remaining.saturating_sub(w_light);
        // Per‑worker channels + threads for EDIT, LIGHT, BG pools
        let mut edit_worker_txs: Vec<mpsc::Sender<BuildJob>> = Vec::with_capacity(w_edit);
        let mut light_worker_txs: Vec<mpsc::Sender<BuildJob>> = Vec::with_capacity(w_light);
        let mut bg_worker_txs: Vec<mpsc::Sender<BuildJob>> = Vec::with_capacity(w_bg);
        // Worker factory closure
        let spawn_worker = |wrx: mpsc::Receiver<BuildJob>,
                            tx: mpsc::Sender<JobOut>,
                            w: Arc<World>,
                            ls: Arc<LightingStore>,
                            lane: Lane| {
            thread::spawn(move || {
                while let Ok(job) = wrx.recv() {
                    // Start from previous buffer when provided; else regenerate from worldgen
                    let mut buf = if let Some(prev) = job.prev_buf {
                        prev
                    } else {
                        chunkbuf::generate_chunk_buffer(&w, job.cx, job.cz, &job.reg)
                    };
                    // Apply persistent edits for this chunk before meshing
                    let base_x = job.cx * buf.sx as i32;
                    let base_z = job.cz * buf.sz as i32;
                    for ((wx, wy, wz), b) in job.chunk_edits.iter().copied() {
                        if wy < 0 || wy >= buf.sy as i32 {
                            continue;
                        }
                        let lx = (wx - base_x) as usize;
                        let ly = wy as usize;
                        let lz = (wz - base_z) as usize;
                        if lx < buf.sx && lz < buf.sz {
                            let idx = buf.idx(lx, ly, lz);
                            buf.blocks[idx] = b;
                        }
                    }
                    let snap_map: std::collections::HashMap<(i32, i32, i32), Block> =
                        job.region_edits.into_iter().collect();
                    match lane {
                        Lane::Light => {
                            // Compute light only; upload atlas on main thread.
                            let lg = compute_light_with_borders_buf(&buf, &ls, &job.reg, &w);
                            // Also publish macro light borders so neighbors can stitch without requiring a remesh.
                            let borders = LightBorders::from_grid(&lg);
                            let _ = tx.send(JobOut {
                                cpu: None,
                                light_atlas: None,
                                light_grid: Some(lg),
                                buf,
                                light_borders: Some(borders),
                                cx: job.cx,
                                cz: job.cz,
                                rev: job.rev,
                                job_id: job.job_id,
                            });
                        }
                        _ => {
                            // Compute lighting once; reuse for meshing and atlas.
                            let lg = compute_light_with_borders_buf(&buf, &ls, &job.reg, &w);
                            let built = build_chunk_wcc_cpu_buf_with_light(
                                &buf,
                                &lg,
                                &w,
                                Some(&snap_map),
                                job.cx,
                                job.cz,
                                &job.reg,
                            );
                            if let Some((cpu, light_borders)) = built {
                                let _ = tx.send(JobOut {
                                    cpu: Some(cpu),
                                    light_atlas: None,
                                    light_grid: Some(lg),
                                    buf,
                                    light_borders,
                                    cx: job.cx,
                                    cz: job.cz,
                                    rev: job.rev,
                                    job_id: job.job_id,
                                });
                            }
                        }
                    }
                }
            })
        };
        // Spawn EDIT workers
        for _ in 0..w_edit {
            let (wtx, wrx) = mpsc::channel::<BuildJob>();
            edit_worker_txs.push(wtx);
            let tx = res_tx.clone();
            let w = world.clone();
            let ls = lighting.clone();
            let _handle = spawn_worker(wrx, tx, w, ls, Lane::Edit);
        }
        // Spawn LIGHT workers
        for _ in 0..w_light {
            let (wtx, wrx) = mpsc::channel::<BuildJob>();
            light_worker_txs.push(wtx);
            let tx = res_tx.clone();
            let w = world.clone();
            let ls = lighting.clone();
            let _handle = spawn_worker(wrx, tx, w, ls, Lane::Light);
        }
        // Spawn BG workers
        for _ in 0..w_bg {
            let (wtx, wrx) = mpsc::channel::<BuildJob>();
            bg_worker_txs.push(wtx);
            let tx = res_tx.clone();
            let w = world.clone();
            let ls = lighting.clone();
            let _handle = spawn_worker(wrx, tx, w, ls, Lane::Bg);
        }
        // Counters (shared across threads)
        let q_edit_ctr = Arc::new(AtomicUsize::new(0));
        let q_light_ctr = Arc::new(AtomicUsize::new(0));
        let q_bg_ctr = Arc::new(AtomicUsize::new(0));
        let inflight_edit_ctr = Arc::new(AtomicUsize::new(0));
        let inflight_light_ctr = Arc::new(AtomicUsize::new(0));
        let inflight_bg_ctr = Arc::new(AtomicUsize::new(0));

        // EDIT dispatcher
        {
            let edit_worker_txs_cl = edit_worker_txs.clone();
            let job_rx_edit = job_rx_edit;
            let q_edit_c = q_edit_ctr.clone();
            let inflight_c = inflight_edit_ctr.clone();
            thread::spawn(move || {
                let mut i = 0usize;
                while let Ok(job) = job_rx_edit.recv() {
                    q_edit_c.fetch_sub(1, Ordering::Relaxed);
                    if !edit_worker_txs_cl.is_empty() {
                        let _ = edit_worker_txs_cl[i % edit_worker_txs_cl.len()].send(job);
                        inflight_c.fetch_add(1, Ordering::Relaxed);
                        i = i.wrapping_add(1);
                    }
                }
            });
        }
        // LIGHT dispatcher (no fallback to EDIT; preserve edit exclusivity)
        {
            let light_worker_txs_cl = light_worker_txs.clone();
            let bg_worker_txs_cl = bg_worker_txs.clone();
            let job_rx_light = job_rx_light;
            let q_light_c = q_light_ctr.clone();
            let inflight_c = inflight_light_ctr.clone();
            let q_bg_c = q_bg_ctr.clone();
            let inflight_bg_c = inflight_bg_ctr.clone();
            let w_bg_cl = w_bg;
            thread::spawn(move || {
                let mut i = 0usize;
                while let Ok(job) = job_rx_light.recv() {
                    q_light_c.fetch_sub(1, Ordering::Relaxed);
                    // If BG lane appears idle, let BG workers help with lighting
                    let bg_idle = !bg_worker_txs_cl.is_empty()
                        && q_bg_c.load(Ordering::Relaxed) == 0
                        && inflight_bg_c.load(Ordering::Relaxed) < w_bg_cl;
                    if bg_idle {
                        let _ = bg_worker_txs_cl[i % bg_worker_txs_cl.len()].send(job);
                        inflight_c.fetch_add(1, Ordering::Relaxed);
                        i = i.wrapping_add(1);
                    } else if !light_worker_txs_cl.is_empty() {
                        let _ = light_worker_txs_cl[i % light_worker_txs_cl.len()].send(job);
                        inflight_c.fetch_add(1, Ordering::Relaxed);
                        i = i.wrapping_add(1);
                    } else if !bg_worker_txs_cl.is_empty() {
                        let _ = bg_worker_txs_cl[i % bg_worker_txs_cl.len()].send(job);
                        inflight_c.fetch_add(1, Ordering::Relaxed);
                        i = i.wrapping_add(1);
                    }
                }
            });
        }
        // BG dispatcher: round-robin on BG workers
        {
            let bg_worker_txs_cl = bg_worker_txs.clone();
            let q_bg_c = q_bg_ctr.clone();
            let inflight_c = inflight_bg_ctr.clone();
            thread::spawn(move || {
                let mut i = 0usize;
                while let Ok(job) = job_rx_bg.recv() {
                    q_bg_c.fetch_sub(1, Ordering::Relaxed);
                    if !bg_worker_txs_cl.is_empty() {
                        let _ = bg_worker_txs_cl[i % bg_worker_txs_cl.len()].send(job);
                        inflight_c.fetch_add(1, Ordering::Relaxed);
                        i = i.wrapping_add(1);
                    }
                }
            });
        }

        // Structure worker (single thread is fine for now)
        {
            thread::spawn(move || {
                while let Ok(job) = s_job_rx.recv() {
                    let mut buf = chunkbuf::ChunkBuf::from_blocks_local(
                        0,
                        0,
                        job.sx,
                        job.sy,
                        job.sz,
                        job.base_blocks.clone(),
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
                    let cpu = geist_mesh_cpu::build_voxel_body_cpu_buf(&buf, 96, &job.reg);
                    let _ = s_res_tx.send(StructureJobOut {
                        id: job.id,
                        rev: job.rev,
                        cpu,
                    });
                }
            });
        }

        Self {
            job_tx_edit,
            job_tx_light,
            job_tx_bg,
            res_rx,
            _edit_worker_txs: edit_worker_txs,
            _light_worker_txs: light_worker_txs,
            _bg_worker_txs: bg_worker_txs,
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
            lane_by_job: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub fn submit_build_job_edit(&self, job: BuildJob) {
        self.q_edit.fetch_add(1, Ordering::Relaxed);
        self.lane_by_job
            .lock()
            .unwrap()
            .insert(job.job_id, Lane::Edit);
        if self.job_tx_edit.send(job).is_err() {
            self.q_edit.fetch_sub(1, Ordering::Relaxed);
        }
    }

    pub fn submit_build_job_light(&self, job: BuildJob) {
        self.q_light.fetch_add(1, Ordering::Relaxed);
        self.lane_by_job
            .lock()
            .unwrap()
            .insert(job.job_id, Lane::Light);
        if self.job_tx_light.send(job).is_err() {
            self.q_light.fetch_sub(1, Ordering::Relaxed);
        }
    }

    pub fn submit_build_job_bg(&self, job: BuildJob) {
        self.q_bg.fetch_add(1, Ordering::Relaxed);
        self.lane_by_job
            .lock()
            .unwrap()
            .insert(job.job_id, Lane::Bg);
        if self.job_tx_bg.send(job).is_err() {
            self.q_bg.fetch_sub(1, Ordering::Relaxed);
        }
    }

    pub fn drain_worker_results(&self) -> Vec<JobOut> {
        let mut out = Vec::new();
        for x in self.res_rx.try_iter() {
            // Decrement inflight counter by lane (tracked at submission time)
            if let Some(lane) = self.lane_by_job.lock().unwrap().remove(&x.job_id) {
                match lane {
                    Lane::Edit => self.inflight_edit.fetch_sub(1, Ordering::Relaxed),
                    Lane::Light => self.inflight_light.fetch_sub(1, Ordering::Relaxed),
                    Lane::Bg => self.inflight_bg.fetch_sub(1, Ordering::Relaxed),
                };
            }
            out.push(x);
        }
        out
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
        let mut out = Vec::new();
        for x in self.s_res_rx.try_iter() {
            out.push(x);
        }
        out
    }
}

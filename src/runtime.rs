use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use crate::chunkbuf;
use crate::mesher::{self, ChunkMeshCPU, NeighborsLoaded};
use crate::shaders;
use crate::structure::StructureId;
use crate::voxel::World;
use crate::blocks::BlockRegistry;

#[derive(Clone, Debug)]
pub struct BuildJob {
    pub cx: i32,
    pub cz: i32,
    pub neighbors: NeighborsLoaded,
    pub rev: u64,
    pub job_id: u64,
    pub chunk_edits: Vec<((i32, i32, i32), crate::voxel::Block)>,
    pub region_edits: Vec<((i32, i32, i32), crate::voxel::Block)>,
}

pub struct JobOut {
    pub cpu: ChunkMeshCPU,
    pub buf: chunkbuf::ChunkBuf,
    pub light_borders: Option<crate::lighting::LightBorders>,
    pub cx: i32,
    pub cz: i32,
    pub rev: u64,
    pub job_id: u64,
}

pub struct Runtime {
    // Rendering resources
    pub leaves_shader: Option<shaders::LeavesShader>,
    pub fog_shader: Option<shaders::FogShader>,
    pub tex_cache: mesher::TextureCache,
    pub reg: std::sync::Arc<BlockRegistry>,
    // GPU chunk models
    pub renders: HashMap<(i32, i32), mesher::ChunkRender>,
    pub structure_renders: HashMap<StructureId, mesher::ChunkRender>,

    // Worker infra
    job_tx: mpsc::Sender<BuildJob>,
    res_rx: mpsc::Receiver<JobOut>,
    _worker_txs: Vec<mpsc::Sender<BuildJob>>, // hold to keep senders alive
    // Structure worker infra
    s_job_tx: mpsc::Sender<StructureBuildJob>,
    s_res_rx: mpsc::Receiver<StructureJobOut>,
}

#[derive(Clone, Debug)]
pub struct StructureBuildJob {
    pub id: StructureId,
    pub rev: u64,
    pub sx: usize,
    pub sy: usize,
    pub sz: usize,
    pub base_blocks: Vec<crate::voxel::Block>,
    pub edits: Vec<((i32, i32, i32), crate::voxel::Block)>,
}

pub struct StructureJobOut {
    pub id: StructureId,
    pub rev: u64,
    pub cpu: ChunkMeshCPU,
}

impl Runtime {
    pub fn new(
        rl: &mut raylib::prelude::RaylibHandle,
        thread: &raylib::prelude::RaylibThread,
        world: Arc<World>,
        lighting: Arc<crate::lighting::LightingStore>,
        reg: std::sync::Arc<BlockRegistry>,
    ) -> Self {
        use std::sync::mpsc;
        let leaves_shader = shaders::LeavesShader::load(rl, thread);
        let fog_shader = shaders::FogShader::load(rl, thread);
        let mut tex_cache = mesher::TextureCache::new();

        // Worker threads
        let (job_tx, job_rx) = mpsc::channel::<BuildJob>();
        let (res_tx, res_rx) = mpsc::channel::<JobOut>();
        // Structure channels
        let (s_job_tx, s_job_rx) = mpsc::channel::<StructureBuildJob>();
        let (s_res_tx, s_res_rx) = mpsc::channel::<StructureJobOut>();
        let worker_count: usize = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8);
        // Per‑worker channels + threads
        let mut worker_txs: Vec<mpsc::Sender<BuildJob>> = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let (wtx, wrx) = mpsc::channel::<BuildJob>();
            worker_txs.push(wtx);
            let tx = res_tx.clone();
            let w = world.clone();
            let ls = lighting.clone();
            let reg = reg.clone();
            thread::spawn(move || {
                while let Ok(job) = wrx.recv() {
                    let mut buf = chunkbuf::generate_chunk_buffer(&w, job.cx, job.cz);
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
                    let snap_map: std::collections::HashMap<(i32, i32, i32), crate::voxel::Block> =
                        job.region_edits.into_iter().collect();
                    if let Some((cpu, light_borders)) = mesher::build_chunk_greedy_cpu_buf(
                        &buf,
                        Some(&ls),
                        &w,
                        Some(&snap_map),
                        job.neighbors,
                        job.cx,
                        job.cz,
                        &reg.materials,
                    ) {
                        let _ = tx.send(JobOut {
                            cpu,
                            buf,
                            light_borders,
                            cx: job.cx,
                            cz: job.cz,
                            rev: job.rev,
                            job_id: job.job_id,
                        });
                    }
                }
            });
        }
        // Dispatcher thread: round robin jobs to workers (stable order by arrival)
        {
            let worker_txs_cl = worker_txs.clone();
            thread::spawn(move || {
                let mut i = 0usize;
                while let Ok(job) = job_rx.recv() {
                    if !worker_txs_cl.is_empty() {
                        let _ = worker_txs_cl[i % worker_txs_cl.len()].send(job);
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
                        let (lxu, lyu, lzu) = (lx as usize, ly as usize, lz as usize);
                        if lxu >= buf.sx || lyu >= buf.sy || lzu >= buf.sz {
                            continue;
                        }
                        let idx = buf.idx(lxu, lyu, lzu);
                        buf.blocks[idx] = b;
                    }
                    let cpu = mesher::build_voxel_body_cpu_buf(&buf, 180);
                    let _ = s_res_tx.send(StructureJobOut {
                        id: job.id,
                        rev: job.rev,
                        cpu,
                    });
                }
            });
        }

        Self {
            leaves_shader,
            fog_shader,
            tex_cache,
            reg,
            renders: HashMap::new(),
            structure_renders: HashMap::new(),
            job_tx,
            res_rx,
            _worker_txs: worker_txs,
            s_job_tx,
            s_res_rx,
        }
    }

    pub fn submit_build_job(&self, job: BuildJob) {
        let _ = self.job_tx.send(job);
    }

    pub fn drain_worker_results(&self) -> Vec<JobOut> {
        let mut out = Vec::new();
        for x in self.res_rx.try_iter() {
            out.push(x);
        }
        out
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

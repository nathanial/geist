use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use crate::chunkbuf;
use crate::mesher::{self, ChunkMeshCPU, NeighborsLoaded};
use crate::shaders;
use crate::voxel::World;

#[derive(Clone, Copy, Debug)]
pub struct BuildJob {
    pub cx: i32,
    pub cz: i32,
    pub neighbors: NeighborsLoaded,
    pub rev: u64,
    pub job_id: u64,
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
    // GPU chunk models
    pub renders: HashMap<(i32, i32), mesher::ChunkRender>,

    // Worker infra
    job_tx: mpsc::Sender<BuildJob>,
    res_rx: mpsc::Receiver<JobOut>,
    _worker_txs: Vec<mpsc::Sender<BuildJob>>, // hold to keep senders alive
}

impl Runtime {
    pub fn new(
        rl: &mut raylib::prelude::RaylibHandle,
        thread: &raylib::prelude::RaylibThread,
        world: Arc<World>,
        lighting: Arc<crate::lighting::LightingStore>,
        edits: Arc<crate::edit::EditStore>,
    ) -> Self {
        use std::sync::mpsc;
        let leaves_shader = shaders::LeavesShader::load(rl, thread);
        let fog_shader = shaders::FogShader::load(rl, thread);
        let mut tex_cache = mesher::TextureCache::new();
        // Preload some common textures
        use crate::mesher::FaceMaterial;
        use crate::voxel::TreeSpecies;
        let mut mats: Vec<FaceMaterial> = vec![
            FaceMaterial::GrassTop,
            FaceMaterial::GrassSide,
            FaceMaterial::Dirt,
            FaceMaterial::Stone,
            FaceMaterial::Sand,
            FaceMaterial::Snow,
            FaceMaterial::Glowstone,
            FaceMaterial::Beacon,
        ];
        for sp in [
            TreeSpecies::Oak,
            TreeSpecies::Birch,
            TreeSpecies::Spruce,
            TreeSpecies::Jungle,
            TreeSpecies::Acacia,
            TreeSpecies::DarkOak,
        ] {
            mats.push(FaceMaterial::WoodTop(sp));
            mats.push(FaceMaterial::WoodSide(sp));
            mats.push(FaceMaterial::Leaves(sp));
        }
        for fm in &mats {
            let _ = tex_cache.get_or_load(rl, thread, &fm.texture_candidates());
        }

        // Worker threads
        let (job_tx, job_rx) = mpsc::channel::<BuildJob>();
        let (res_tx, res_rx) = mpsc::channel::<JobOut>();
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
            let edits = edits.clone();
            thread::spawn(move || {
                while let Ok(job) = wrx.recv() {
                    let current_rev = edits.get_rev(job.cx, job.cz);
                    if job.rev > 0 && job.rev < current_rev {
                        continue;
                    }
                    let mut buf = chunkbuf::generate_chunk_buffer(&w, job.cx, job.cz);
                    // Apply persistent edits for this chunk before meshing
                    let base_x = job.cx * buf.sx as i32;
                    let base_z = job.cz * buf.sz as i32;
                    let edits_chunk = edits.snapshot_for_chunk(job.cx, job.cz);
                    for ((wx, wy, wz), b) in edits_chunk {
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
                    let snap_vec = edits.snapshot_for_region(job.cx, job.cz, 1);
                    let snap_map: std::collections::HashMap<(i32, i32, i32), crate::voxel::Block> =
                        snap_vec.into_iter().collect();
                    if let Some((cpu, light_borders)) = mesher::build_chunk_greedy_cpu_buf(
                        &buf,
                        Some(&ls),
                        &w,
                        Some(&snap_map),
                        job.neighbors,
                        job.cx,
                        job.cz,
                    ) {
                        let _ = tx.send(JobOut { cpu, buf, light_borders, cx: job.cx, cz: job.cz, rev: job.rev, job_id: job.job_id });
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

        Self {
            leaves_shader,
            fog_shader,
            tex_cache,
            renders: HashMap::new(),
            job_tx,
            res_rx,
            _worker_txs: worker_txs,
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
}


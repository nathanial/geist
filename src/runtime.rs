use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use crate::chunkbuf;
use crate::mesher::{self, ChunkMeshCPU, NeighborsLoaded};
use raylib::prelude::{RaylibMaterial, RaylibModel, RaylibTexture2D};
use crate::shaders;
use crate::structure::StructureId;
use crate::voxel::World;
use crate::blocks::{BlockRegistry, Block};

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
    tex_event_rx: mpsc::Receiver<String>,
    worldgen_event_rx: mpsc::Receiver<()>,
    world_config_path: String,
    pub world: Arc<World>,
    pub rebuild_on_worldgen: bool,
    worldgen_dirty: bool,
    // GPU chunk models
    pub renders: HashMap<(i32, i32), mesher::ChunkRender>,
    pub structure_renders: HashMap<StructureId, mesher::ChunkRender>,

    // Worker infra (three lanes: edit, light, bg)
    job_tx_edit: mpsc::Sender<BuildJob>,
    job_tx_light: mpsc::Sender<BuildJob>,
    job_tx_bg: mpsc::Sender<BuildJob>,
    res_rx: mpsc::Receiver<JobOut>,
    _edit_worker_txs: Vec<mpsc::Sender<BuildJob>>, // hold to keep senders alive
    _light_worker_txs: Vec<mpsc::Sender<BuildJob>>, // hold to keep senders alive
    _bg_worker_txs: Vec<mpsc::Sender<BuildJob>>, // hold to keep senders alive
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
    pub base_blocks: Vec<Block>,
    pub edits: Vec<((i32, i32, i32), Block)>,
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
        watch_textures: bool,
        watch_worldgen: bool,
        world_config_path: String,
        rebuild_on_worldgen: bool,
    ) -> Self {
        use std::sync::mpsc;
        let leaves_shader = shaders::LeavesShader::load(rl, thread);
        let fog_shader = shaders::FogShader::load(rl, thread);
        let tex_cache = mesher::TextureCache::new();
        // File watcher for texture changes under assets/blocks
        let (tex_tx, tex_rx) = mpsc::channel::<String>();
        if watch_textures {
            let tex_tx = tex_tx.clone();
            std::thread::spawn(move || {
                use notify::{RecursiveMode, Watcher, EventKind};
                let mut watcher = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                    if let Ok(event) = res {
                        match event.kind {
                            EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) | EventKind::Any => {
                                for p in event.paths {
                                    if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                                        let e = ext.to_lowercase();
                                        if e == "png" || e == "jpg" || e == "jpeg" {
                                            let _ = tex_tx.send(p.to_string_lossy().to_string());
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }).unwrap();
                let _ = watcher.watch(std::path::Path::new("assets/blocks"), RecursiveMode::Recursive);
                // Keep thread alive
                loop { std::thread::sleep(std::time::Duration::from_secs(3600)); }
            });
        }

        // File watcher for worldgen config
        let (wg_tx, wg_rx) = mpsc::channel::<()>();
        if watch_worldgen {
            let tx = wg_tx.clone();
            let path = world_config_path.clone();
            std::thread::spawn(move || {
                use notify::{RecursiveMode, Watcher, EventKind};
                if let Ok(mut watcher) = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                    if let Ok(event) = res {
                        match event.kind {
                            EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) | EventKind::Any => {
                                let _ = tx.send(());
                            }
                            _ => {}
                        }
                    }
                }) {
                    let _ = watcher.watch(std::path::Path::new(&path), RecursiveMode::NonRecursive);
                    loop { std::thread::sleep(std::time::Duration::from_secs(3600)); }
                }
            });
        }

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
        let spawn_worker = |wrx: mpsc::Receiver<BuildJob>, tx: mpsc::Sender<JobOut>, w: Arc<World>, ls: Arc<crate::lighting::LightingStore>, reg: Arc<BlockRegistry>| {
            thread::spawn(move || {
                while let Ok(job) = wrx.recv() {
                    // Start from previous buffer when provided; else regenerate from worldgen
                    let mut buf = if let Some(prev) = job.prev_buf {
                        prev
                    } else {
                        chunkbuf::generate_chunk_buffer(&w, job.cx, job.cz, &reg)
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
                    if let Some((cpu, light_borders)) = mesher::build_chunk_greedy_cpu_buf(
                        &buf,
                        Some(&ls),
                        &w,
                        Some(&snap_map),
                        job.neighbors,
                        job.cx,
                        job.cz,
                        &reg,
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
            })
        };
        // Spawn EDIT workers
        for _ in 0..w_edit {
            let (wtx, wrx) = mpsc::channel::<BuildJob>();
            edit_worker_txs.push(wtx);
            let tx = res_tx.clone();
            let w = world.clone();
            let ls = lighting.clone();
            let reg = reg.clone();
            let _handle = spawn_worker(wrx, tx, w, ls, reg);
        }
        // Spawn LIGHT workers
        for _ in 0..w_light {
            let (wtx, wrx) = mpsc::channel::<BuildJob>();
            light_worker_txs.push(wtx);
            let tx = res_tx.clone();
            let w = world.clone();
            let ls = lighting.clone();
            let reg = reg.clone();
            let _handle = spawn_worker(wrx, tx, w, ls, reg);
        }
        // Spawn BG workers
        for _ in 0..w_bg {
            let (wtx, wrx) = mpsc::channel::<BuildJob>();
            bg_worker_txs.push(wtx);
            let tx = res_tx.clone();
            let w = world.clone();
            let ls = lighting.clone();
            let reg = reg.clone();
            let _handle = spawn_worker(wrx, tx, w, ls, reg);
        }
        // EDIT dispatcher
        {
            let edit_worker_txs_cl = edit_worker_txs.clone();
            let job_rx_edit = job_rx_edit;
            thread::spawn(move || {
                let mut i = 0usize;
                while let Ok(job) = job_rx_edit.recv() {
                    if !edit_worker_txs_cl.is_empty() {
                        let _ = edit_worker_txs_cl[i % edit_worker_txs_cl.len()].send(job);
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
            thread::spawn(move || {
                let mut i = 0usize;
                while let Ok(job) = job_rx_light.recv() {
                    if !light_worker_txs_cl.is_empty() {
                        let _ = light_worker_txs_cl[i % light_worker_txs_cl.len()].send(job);
                        i = i.wrapping_add(1);
                    } else if !bg_worker_txs_cl.is_empty() {
                        let _ = bg_worker_txs_cl[i % bg_worker_txs_cl.len()].send(job);
                        i = i.wrapping_add(1);
                    }
                }
            });
        }
        // BG dispatcher: round-robin on BG workers
        {
            let bg_worker_txs_cl = bg_worker_txs.clone();
            thread::spawn(move || {
                let mut i = 0usize;
                while let Ok(job) = job_rx_bg.recv() {
                    if !bg_worker_txs_cl.is_empty() {
                        let _ = bg_worker_txs_cl[i % bg_worker_txs_cl.len()].send(job);
                        i = i.wrapping_add(1);
                    }
                }
            });
        }

        // Structure worker (single thread is fine for now)
        {
            let reg = reg.clone();
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
                    let cpu = mesher::build_voxel_body_cpu_buf(&buf, 180, &reg);
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
            tex_event_rx: tex_rx,
            worldgen_event_rx: wg_rx,
            world_config_path,
            world: world.clone(),
            rebuild_on_worldgen,
            worldgen_dirty: false,
            renders: HashMap::new(),
            structure_renders: HashMap::new(),
            job_tx_edit,
            job_tx_light,
            job_tx_bg,
            res_rx,
            _edit_worker_txs: edit_worker_txs,
            _light_worker_txs: light_worker_txs,
            _bg_worker_txs: bg_worker_txs,
            s_job_tx,
            s_res_rx,
        }
    }

    pub fn submit_build_job_edit(&self, job: BuildJob) {
        let _ = self.job_tx_edit.send(job);
    }

    pub fn submit_build_job_light(&self, job: BuildJob) {
        let _ = self.job_tx_light.send(job);
    }

    pub fn submit_build_job_bg(&self, job: BuildJob) {
        let _ = self.job_tx_bg.send(job);
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

    // Process file watcher events: reload changed textures and rebind them on existing models.
    pub fn process_texture_file_events(
        &mut self,
        rl: &mut raylib::prelude::RaylibHandle,
        thread: &raylib::prelude::RaylibThread,
    ) {
        use std::collections::HashSet;
        let mut changed: HashSet<String> = HashSet::new();
        for p in self.tex_event_rx.try_iter() {
            // Normalize to canonical absolute path when possible
            let canon = std::fs::canonicalize(&p)
                .ok()
                .map(|pb| pb.to_string_lossy().to_string())
                .unwrap_or(p);
            changed.insert(canon);
        }
        if changed.is_empty() {
            return;
        }
        log::info!("Texture changes detected: {} file(s)", changed.len());
        for p in &changed {
            log::debug!(" - {}", p);
        }
        // Helper to choose material path like upload path
        let choose_path = |mid: crate::blocks::MaterialId| -> Option<String> {
            self.reg.materials.get(mid).and_then(|mdef| {
                let candidates: Vec<String> = mdef
                    .texture_candidates
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();
                let chosen = candidates
                    .iter()
                    .find(|p| std::path::Path::new(p.as_str()).exists())
                    .cloned()
                    .or_else(|| candidates.first().cloned());
                chosen.map(|s| std::fs::canonicalize(&s)
                    .ok()
                    .map(|pb| pb.to_string_lossy().to_string())
                    .unwrap_or(s))
            })
        };
        // Reload any changed paths into cache
        for path in changed.iter() {
            // Attempt to load; if fails, skip
            if let Ok(tex) = rl.load_texture(thread, path) {
                tex.set_texture_filter(
                    thread,
                    raylib::consts::TextureFilter::TEXTURE_FILTER_POINT,
                );
                tex.set_texture_wrap(
                    thread,
                    raylib::consts::TextureWrap::TEXTURE_WRAP_REPEAT,
                );
                self.tex_cache.replace_loaded(path.clone(), tex);
                log::debug!("reloaded texture {}", path);
            } else {
                log::warn!("failed to reload texture {}", path);
            }
        }
        let mut rebound: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        // Rebind textures on existing chunk renders
        for (_k, cr) in self.renders.iter_mut() {
            for (mid, model) in cr.parts.iter_mut() {
                let Some(path) = choose_path(*mid) else { continue };
                if !changed.contains(&path) { continue; }
                if let Some(mat) = {
                    use raylib::prelude::RaylibModel;
                    model.materials_mut().get_mut(0)
                } {
                    if let Some(tex) = self.tex_cache.get_ref(&path) {
                        mat.set_material_texture(
                            raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO,
                            tex,
                        );
                        *rebound.entry(path.clone()).or_insert(0) += 1;
                    } else if let Ok(t) = rl.load_texture(thread, &path) {
                        t.set_texture_filter(
                            thread,
                            raylib::consts::TextureFilter::TEXTURE_FILTER_POINT,
                        );
                        t.set_texture_wrap(
                            thread,
                            raylib::consts::TextureWrap::TEXTURE_WRAP_REPEAT,
                        );
                        self.tex_cache.replace_loaded(path.clone(), t);
                        if let Some(tex) = self.tex_cache.get_ref(&path) {
                            mat.set_material_texture(
                                raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO,
                                tex,
                            );
                            *rebound.entry(path.clone()).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        // Rebind for structure renders as well
        for (_id, cr) in self.structure_renders.iter_mut() {
            for (mid, model) in cr.parts.iter_mut() {
                let Some(path) = choose_path(*mid) else { continue };
                if !changed.contains(&path) { continue; }
                if let Some(mat) = {
                    use raylib::prelude::RaylibModel;
                    model.materials_mut().get_mut(0)
                } {
                    if let Some(tex) = self.tex_cache.get_ref(&path) {
                        mat.set_material_texture(
                            raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO,
                            tex,
                        );
                        *rebound.entry(path.clone()).or_insert(0) += 1;
                    } else if let Ok(t) = rl.load_texture(thread, &path) {
                        t.set_texture_filter(
                            thread,
                            raylib::consts::TextureFilter::TEXTURE_FILTER_POINT,
                        );
                        t.set_texture_wrap(
                            thread,
                            raylib::consts::TextureWrap::TEXTURE_WRAP_REPEAT,
                        );
                        self.tex_cache.replace_loaded(path.clone(), t);
                        if let Some(tex) = self.tex_cache.get_ref(&path) {
                            mat.set_material_texture(
                                raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO,
                                tex,
                            );
                            *rebound.entry(path.clone()).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        if rebound.is_empty() {
            log::info!("Texture reload complete; no active models referenced changed textures");
        } else {
            for (p, n) in rebound {
                log::info!("Rebound {} on {} material(s)", p, n);
            }
        }
    }

    pub fn process_worldgen_file_events(&mut self) {
        let mut changed = false;
        for _ in self.worldgen_event_rx.try_iter() { changed = true; }
        if !changed { return; }
        let path = std::path::Path::new(&self.world_config_path);
        if !path.exists() {
            log::warn!("worldgen config missing: {}", self.world_config_path);
            return;
        }
        match crate::worldgen::load_params_from_path(path) {
            Ok(params) => {
                self.world.update_worldgen_params(params);
                log::info!("worldgen config reloaded from {}", self.world_config_path);
                log::info!("Existing chunks unchanged; new gen uses updated params");
                self.worldgen_dirty = true;
            }
            Err(e) => {
                log::warn!("worldgen config reload failed ({}): {}", self.world_config_path, e);
            }
        }
    }

    pub fn take_worldgen_dirty(&mut self) -> bool {
        if self.worldgen_dirty {
            self.worldgen_dirty = false;
            true
        } else { false }
    }
}

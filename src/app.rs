use std::collections::{HashMap, HashSet};

use geist_geom::Vec3;
use geist_render_raylib::conv::{vec3_from_rl, vec3_to_rl};
use raylib::prelude::*;

use crate::event::{Event, EventEnvelope, EventQueue, RebuildCause};
use crate::gamestate::{ChunkEntry, GameState};
use crate::raycast;
use geist_blocks::{Block, BlockRegistry};
use geist_edit::EditStore;
use geist_lighting::LightingStore;
use geist_mesh_cpu::NeighborsLoaded;
use geist_render_raylib::{ChunkRender, FogShader, LeavesShader, TextureCache, upload_chunk_mesh};
use geist_runtime::{BuildJob, JobOut, Runtime, StructureBuildJob};
use geist_structures::{Pose, Structure, StructureId, rotate_yaw, rotate_yaw_inv};
use geist_world::voxel::{World, WorldGenMode};
use serde::Deserialize;

// Scheduling/queue tuning knobs
// Increase per-frame submissions and per-lane queue headroom so workers stay busier.
const JOB_FRAME_CAP_MULT: usize = 4; // was 2
const LANE_QUEUE_EXTRA: usize = 3; // was 1 (target = workers + extra)

#[derive(Deserialize)]
struct HotbarConfig {
    items: Vec<String>,
}

pub struct App {
    pub gs: GameState,
    pub queue: EventQueue,
    pub runtime: Runtime,
    pub cam: crate::camera::FlyCamera,
    pub debug_stats: DebugStats,
    hotbar: Vec<Block>,
    // Renderer-side resources (moved from runtime in Phase 5)
    pub leaves_shader: Option<LeavesShader>,
    pub fog_shader: Option<FogShader>,
    pub tex_cache: TextureCache,
    pub renders: HashMap<(i32, i32), ChunkRender>,
    pub structure_renders: HashMap<StructureId, ChunkRender>,
    pub reg: std::sync::Arc<BlockRegistry>,
    // Session-wide processed event stats
    evt_processed_total: usize,
    evt_processed_by: HashMap<String, usize>,
    // Coalesced rebuild/load intents (priority-scheduled per frame)
    intents: HashMap<(i32, i32), IntentEntry>,
    // File watchers (moved from runtime in Phase 5)
    tex_event_rx: std::sync::mpsc::Receiver<String>,
    worldgen_event_rx: std::sync::mpsc::Receiver<()>,
    world_config_path: String,
    pub rebuild_on_worldgen: bool,
    worldgen_dirty: bool,
    // Assets root for resolving paths
    pub assets_root: std::path::PathBuf,
    // Registry file watcher
    reg_event_rx: std::sync::mpsc::Receiver<()>,
    // Shader file watcher
    shader_event_rx: std::sync::mpsc::Receiver<()>,
}

#[derive(Default)]
pub struct DebugStats {
    pub total_vertices: usize,
    pub total_triangles: usize,
    pub chunks_rendered: usize,
    pub chunks_culled: usize,
    pub structures_rendered: usize,
    pub structures_culled: usize,
    pub draw_calls: usize,
    // Event debug
    pub queued_events_total: usize,
    pub queued_events_by: Vec<(String, usize)>,
    pub intents_size: usize,
}

// Internal prioritization cause for scheduling
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum IntentCause {
    Edit = 0,
    Light = 1,
    StreamLoad = 2,
    #[allow(dead_code)]
    HotReload = 3,
}

#[derive(Clone, Copy, Debug)]
struct IntentEntry {
    rev: u64,
    cause: IntentCause,
    last_tick: u64,
}

impl App {
    fn record_intent(&mut self, cx: i32, cz: i32, cause: IntentCause) {
        // Skip recording if already rendered and no rebuild needed
        let cur_rev = self.gs.edits.get_rev(cx, cz);
        let now = self.gs.tick;
        self.intents
            .entry((cx, cz))
            .and_modify(|e| {
                if cur_rev > e.rev {
                    e.rev = cur_rev;
                }
                // Keep strongest cause (lower enum value = higher priority)
                if cause < e.cause {
                    e.cause = cause;
                }
                e.last_tick = now;
            })
            .or_insert(IntentEntry {
                rev: cur_rev,
                cause,
                last_tick: now,
            });
    }

    fn flush_intents(&mut self) {
        if self.intents.is_empty() {
            return;
        }
        // Compute priorities
        let ccx = (self.cam.position.x / self.gs.world.chunk_size_x as f32).floor() as i32;
        let ccz = (self.cam.position.z / self.gs.world.chunk_size_z as f32).floor() as i32;
        let now = self.gs.tick;
        let mut items: Vec<((i32, i32), IntentEntry, u32, i32)> =
            Vec::with_capacity(self.intents.len());
        for (&key, &ent) in self.intents.iter() {
            let (cx, cz) = key;
            let dx = cx - ccx;
            let dz = cz - ccz;
            // Chebyshev radius (ring distance) in chunk units for granular rings
            let dist_bucket: u32 = dx.abs().max(dz.abs()) as u32;
            // Age: older gets a small boost (negative weight)
            let age = now.saturating_sub(ent.last_tick);
            let age_boost: i32 = if age > 180 {
                -2
            } else if age > 60 {
                -1
            } else {
                0
            }; // ~1-3 seconds at 60Hz
            items.push((key, ent, dist_bucket, age_boost));
        }
        items.sort_by(|a, b| {
            // (cause asc, dist asc, age_boost asc (more negative first))
            a.1.cause
                .cmp(&b.1.cause)
                .then(a.2.cmp(&b.2))
                .then(a.3.cmp(&b.3))
        });

        // Cap submissions per frame (larger multiplier keeps queues primed)
        let worker_n = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8);
        let cap = (worker_n * JOB_FRAME_CAP_MULT).max(8);
        let mut submitted = 0usize;
        let mut submitted_keys: Vec<(i32, i32)> = Vec::new();

        // Backpressure budgets per lane (avoid overfilling runtime FIFOs)
        let (q_e, if_e, q_l, if_l, q_b, if_b) = self.runtime.queue_debug_counts();
        // Allow a larger buffer (workers + extra) to keep workers fed
        let target_edit = self.runtime.w_edit.max(1) + LANE_QUEUE_EXTRA;
        let target_light = self.runtime.w_light.max(1) + LANE_QUEUE_EXTRA;
        let target_bg = self.runtime.w_bg.max(1) + LANE_QUEUE_EXTRA;
        let mut budget_edit = target_edit.saturating_sub(q_e + if_e);
        let mut budget_light = target_light.saturating_sub(q_l + if_l);
        let mut budget_bg = target_bg.saturating_sub(q_b + if_b);

        for (key, ent, dist_bucket, _ab) in items.into_iter() {
            if submitted >= cap {
                break;
            }
            let (cx, cz) = key;
            // inflight gating: skip if same/newer already in flight
            // Skip only if an inflight entry exists and is already at or above this rev
            if self
                .gs
                .inflight_rev
                .get(&key)
                .map(|v| *v >= ent.rev)
                .unwrap_or(false)
            {
                continue;
            }
            // If chunk is not loaded, treat as load intent; else rebuild intent
            let neighbors = self.neighbor_mask(cx, cz);
            let rev = ent.rev;
            let job_id = Self::job_hash(cx, cz, rev, neighbors);
            // Visibility gating (lightweight): do not block edits
            let is_loaded = self.renders.contains_key(&key);
            match ent.cause {
                IntentCause::Edit => {
                    // Schedule even if not loaded: acts as a high-priority load+rebuild
                    if budget_edit == 0 {
                        continue;
                    }
                }
                IntentCause::Light => {
                    // Prioritize and gate by distance; skip far lighting rebuilds
                    let r = self.gs.view_radius_chunks;
                    if dist_bucket as i32 > r + 1 {
                        continue;
                    }
                    if budget_light == 0 {
                        continue;
                    }
                }
                IntentCause::StreamLoad | IntentCause::HotReload => {
                    // StreamLoad: only schedule if still desired (within view radius)
                    let r = self.gs.view_radius_chunks;
                    if !is_loaded && dist_bucket as i32 > r {
                        continue;
                    }
                    // If already loaded, allow HotReload rebuilds only (not implemented here)
                    if is_loaded { /* already loaded; schedule rebuild only if HotReload */ }
                    if budget_bg == 0 {
                        continue;
                    }
                }
            }
            // Emit job request
            // Submit for next tick to avoid stranding events after we've finished this tick's loop
            let cause = match ent.cause {
                IntentCause::Edit => RebuildCause::Edit,
                IntentCause::Light => RebuildCause::LightingBorder,
                IntentCause::StreamLoad | IntentCause::HotReload => RebuildCause::StreamLoad,
            };
            self.queue.emit_after(
                1,
                Event::BuildChunkJobRequested {
                    cx,
                    cz,
                    neighbors,
                    rev,
                    job_id,
                    cause,
                },
            );
            self.gs.inflight_rev.insert(key, rev);
            submitted_keys.push(key);
            submitted += 1;
            // Consume lane budget
            match ent.cause {
                IntentCause::Edit => {
                    budget_edit = budget_edit.saturating_sub(1);
                }
                IntentCause::Light => {
                    budget_light = budget_light.saturating_sub(1);
                }
                IntentCause::StreamLoad | IntentCause::HotReload => {
                    budget_bg = budget_bg.saturating_sub(1);
                }
            }
        }
        // Remove only submitted intents; keep the rest to trickle in subsequent frames
        for k in submitted_keys {
            self.intents.remove(&k);
        }
    }
    fn load_hotbar(reg: &BlockRegistry, assets_root: &std::path::Path) -> Vec<Block> {
        let path = crate::assets::hotbar_path(assets_root);
        if !path.exists() {
            return Vec::new();
        }
        match std::fs::read_to_string(&path) {
            Ok(s) => match toml::from_str::<HotbarConfig>(&s) {
                Ok(cfg) => cfg
                    .items
                    .into_iter()
                    .filter_map(|name| reg.id_by_name(&name).map(|id| Block { id, state: 0 }))
                    .collect(),
                Err(e) => {
                    log::warn!("hotbar.toml parse error: {}", e);
                    Vec::new()
                }
            },
            Err(e) => {
                log::warn!("hotbar.toml read error: {}", e);
                Vec::new()
            }
        }
    }
    #[inline]
    fn structure_block_solid_at_local(
        reg: &BlockRegistry,
        st: &Structure,
        lx: i32,
        ly: i32,
        lz: i32,
    ) -> bool {
        if lx < 0 || ly < 0 || lz < 0 {
            return false;
        }
        let (lxu, lyu, lzu) = (lx as usize, ly as usize, lz as usize);
        if lxu >= st.sx || lyu >= st.sy || lzu >= st.sz {
            return false;
        }
        if let Some(b) = st.edits.get(lx, ly, lz) {
            return reg
                .get(b.id)
                .map(|ty| ty.is_solid(b.state))
                .unwrap_or(false);
        }
        let b = st.blocks[st.idx(lxu, lyu, lzu)];
        reg.get(b.id)
            .map(|ty| ty.is_solid(b.state))
            .unwrap_or(false)
    }

    fn is_feet_on_structure(&self, st: &Structure, feet_world: Vector3) -> bool {
        let rx = (self.gs.walker.radius * 0.85).max(0.05);
        let offsets = [
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(rx, 0.0, 0.0),
            Vector3::new(-rx, 0.0, 0.0),
            Vector3::new(0.0, 0.0, rx),
            Vector3::new(0.0, 0.0, -rx),
            Vector3::new(rx, 0.0, rx),
            Vector3::new(rx, 0.0, -rx),
            Vector3::new(-rx, 0.0, rx),
            Vector3::new(-rx, 0.0, -rx),
        ];
        for off in &offsets {
            let p = feet_world + *off;
            let pv = vec3_from_rl(p);
            let diff = Vec3 {
                x: pv.x - st.pose.pos.x,
                y: pv.y - st.pose.pos.y,
                z: pv.z - st.pose.pos.z,
            };
            let local = rotate_yaw_inv(diff, st.pose.yaw_deg);
            let lx = local.x.floor() as i32;
            let ly = (local.y - 0.08).floor() as i32;
            let lz = local.z.floor() as i32;
            // Be robust to tiny clearance/step resolution by also checking one cell below
            if Self::structure_block_solid_at_local(&self.reg, st, lx, ly, lz)
                || Self::structure_block_solid_at_local(&self.reg, st, lx, ly - 1, lz)
            {
                return true;
            }
        }
        false
    }
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    pub fn new(
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        world: std::sync::Arc<World>,
        lighting: std::sync::Arc<LightingStore>,
        edits: EditStore,
        reg: std::sync::Arc<BlockRegistry>,
        watch_textures: bool,
        watch_worldgen: bool,
        world_config_path: String,
        rebuild_on_worldgen: bool,
        assets_root: std::path::PathBuf,
    ) -> Self {
        // Spawn: if flat world, start a few blocks above the slab; else near world top
        let spawn = if world.is_flat() {
            Vector3::new(
                (world.world_size_x() as f32) * 0.5,
                6.0,
                (world.world_size_z() as f32) * 0.5,
            )
        } else {
            Vector3::new(
                (world.world_size_x() as f32) * 0.5,
                (world.world_size_y() as f32) * 0.8,
                (world.world_size_z() as f32) * 0.5,
            )
        };
        let cam = crate::camera::FlyCamera::new(spawn + Vector3::new(0.0, 5.0, 20.0));

        // Renderer-side resources and file watchers (moved from Runtime in Phase 5)
        let leaves_shader = LeavesShader::load_with_base(rl, thread, &assets_root)
            .or_else(|| LeavesShader::load(rl, thread));
        let fog_shader =
            FogShader::load_with_base(rl, thread, &assets_root).or_else(|| FogShader::load(rl, thread));
        let tex_cache = TextureCache::new();
        // File watcher for textures under assets/blocks
        let (tex_tx, tex_rx) = std::sync::mpsc::channel::<String>();
        if watch_textures {
            let tex_tx = tex_tx.clone();
            let tex_dir = crate::assets::textures_dir(&assets_root);
            std::thread::spawn(move || {
                use notify::{EventKind, RecursiveMode, Watcher};
                let mut watcher = notify::recommended_watcher(
                    move |res: Result<notify::Event, notify::Error>| {
                        if let Ok(event) = res {
                            match event.kind {
                                EventKind::Modify(_)
                                | EventKind::Create(_)
                                | EventKind::Remove(_)
                                | EventKind::Any => {
                                    for p in event.paths {
                                        if let Some(e) = p.extension().and_then(|e| e.to_str()) {
                                            let e = e.to_lowercase();
                                            if e == "png" || e == "jpg" || e == "jpeg" {
                                                let _ =
                                                    tex_tx.send(p.to_string_lossy().to_string());
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    },
                )
                .unwrap();
                let _ = watcher.watch(tex_dir.as_path(), RecursiveMode::Recursive);
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(3600));
                }
            });
        }
        // File watcher for worldgen config
        let (wg_tx, wg_rx) = std::sync::mpsc::channel::<()>();
        if watch_worldgen {
            let tx = wg_tx.clone();
            let path = world_config_path.clone();
            std::thread::spawn(move || {
                use notify::{EventKind, RecursiveMode, Watcher};
                if let Ok(mut watcher) =
                    notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                        if let Ok(event) = res {
                            match event.kind {
                                EventKind::Modify(_)
                                | EventKind::Create(_)
                                | EventKind::Remove(_)
                                | EventKind::Any => {
                                    let _ = tx.send(());
                                }
                                _ => {}
                            }
                        }
                    })
                {
                    let _ = watcher.watch(std::path::Path::new(&path), RecursiveMode::NonRecursive);
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(3600));
                    }
                }
            });
        }

        let runtime = Runtime::new(world.clone(), lighting.clone());
        let mut gs = GameState::new(world.clone(), edits, lighting.clone(), cam.position);
        let mut queue = EventQueue::new();
        let hotbar = Self::load_hotbar(&reg, &assets_root);

        // Discover and load all .schem files in 'schematics/'.
        // Flat worlds: keep existing ground placement.
        // Non-flat worlds: compute a flying platform sized to hold all schematics and stamp them onto it.
        {
            use std::path::Path;
            let dir = crate::assets::schematics_dir(&assets_root);
            if dir.exists() {
                match geist_io::list_schematics_with_size(dir.as_path()) {
                    Ok(mut list) => {
                        if list.is_empty() {
                            log::info!("No .schem files found under {:?}", dir);
                        } else {
                            // Stable order: sort by filename (case-insensitive)
                            list.sort_by(|a, b| {
                                let an = a
                                    .path
                                    .file_name()
                                    .map(|s| s.to_string_lossy().to_lowercase())
                                    .unwrap_or_default();
                                let bn = b
                                    .path
                                    .file_name()
                                    .map(|s| s.to_string_lossy().to_lowercase())
                                    .unwrap_or_default();
                                an.cmp(&bn)
                            });
                            let is_flat = world.is_flat();
                            if is_flat {
                                // Flat placement (existing behavior)
                                let base_y: i32 = match world.mode {
                                    WorldGenMode::Flat { thickness } => {
                                        if thickness > 0 {
                                            1
                                        } else {
                                            0
                                        }
                                    }
                                    _ => 0,
                                };
                                let margin: i32 = 4;
                                let row_width_limit: i32 =
                                    (world.world_size_x() as i32).max(64) - margin;
                                let mut placements: Vec<(
                                    std::path::PathBuf,
                                    (i32, i32, i32),
                                    (i32, i32),
                                )> = Vec::new();
                                let mut cur_x: i32 = 0;
                                let mut cur_z: i32 = 0;
                                let mut row_depth: i32 = 0;
                                for ent in &list {
                                    let (sx, _sy, sz) = ent.size;
                                    if cur_x > 0 && cur_x + sx > row_width_limit {
                                        cur_x = 0;
                                        cur_z += row_depth;
                                        row_depth = 0;
                                    }
                                    placements.push((
                                        ent.path.clone(),
                                        (cur_x, base_y, cur_z),
                                        (sx, sz),
                                    ));
                                    cur_x += sx + margin;
                                    row_depth = row_depth.max(sz + margin);
                                }
                                // Center within world
                                let (mut min_x, mut max_x, mut min_z, mut max_z) =
                                    (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
                                for (_p, (lx, _ly, lz), (sx, sz)) in &placements {
                                    min_x = min_x.min(*lx);
                                    min_z = min_z.min(*lz);
                                    max_x = max_x.max(*lx + sx);
                                    max_z = max_z.max(*lz + sz);
                                }
                                if min_x == i32::MAX {
                                    min_x = 0;
                                    max_x = 0;
                                    min_z = 0;
                                    max_z = 0;
                                }
                                let layout_cx = (min_x + max_x) / 2;
                                let layout_cz = (min_z + max_z) / 2;
                                let world_cx = (world.world_size_x() as i32) / 2;
                                let world_cz = (world.world_size_z() as i32) / 2;
                                let shift_x = world_cx - layout_cx;
                                let shift_z = world_cz - layout_cz;
                                for (p, (lx, ly, lz), (_sx, _sz)) in placements {
                                    let wx = lx + shift_x;
                                    let wy = ly;
                                    let wz = lz + shift_z;
                                    match geist_io::load_any_schematic_apply_edits(
                                        &p,
                                        (wx, wy, wz),
                                        &mut gs.edits,
                                        &reg,
                                    ) {
                                        Ok((sx, sy, sz)) => {
                                            log::info!(
                                                "Loaded schem {:?} at ({},{},{}) ({}x{}x{})",
                                                p,
                                                wx,
                                                wy,
                                                wz,
                                                sx,
                                                sy,
                                                sz
                                            );
                                        }
                                        Err(e) => {
                                            log::warn!("Failed loading schem {:?}: {}", p, e);
                                        }
                                    }
                                }
                            } else {
                                // Non-flat: place schematics directly on terrain surface near world center.
                                // 1) Pack placements into a near-square footprint (same as before, but no platform).
                                let margin: i32 = 4;
                                let total_area: i64 = list
                                    .iter()
                                    .map(|e| (e.size.0 as i64) * (e.size.2 as i64))
                                    .sum();
                                let target_w: i32 =
                                    (((total_area as f64).sqrt()).ceil() as i32).max(32);
                                let row_width_limit: i32 = target_w;
                                let mut placements: Vec<(
                                    std::path::PathBuf,
                                    (i32, i32),
                                    (i32, i32, i32),
                                )> = Vec::new();
                                let mut cur_x: i32 = 0;
                                let mut cur_z: i32 = 0;
                                let mut row_depth: i32 = 0;
                                for ent in &list {
                                    let (sx, _sy, sz) = ent.size;
                                    if cur_x > 0 && cur_x + sx > row_width_limit {
                                        cur_x = 0;
                                        cur_z += row_depth;
                                        row_depth = 0;
                                    }
                                    placements.push((ent.path.clone(), (cur_x, cur_z), ent.size));
                                    cur_x += sx + margin;
                                    row_depth = row_depth.max(sz + margin);
                                }
                                // 2) Center the layout horizontally in world space.
                                let (mut min_x, mut max_x, mut min_z, mut max_z) =
                                    (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
                                for (_p, (lx, lz), (sx, _sy, sz)) in &placements {
                                    min_x = min_x.min(*lx);
                                    min_z = min_z.min(*lz);
                                    max_x = max_x.max(*lx + sx);
                                    max_z = max_z.max(*lz + sz);
                                }
                                if min_x == i32::MAX {
                                    min_x = 0;
                                    max_x = 0;
                                    min_z = 0;
                                    max_z = 0;
                                }
                                let layout_cx = (min_x + max_x) / 2;
                                let layout_cz = (min_z + max_z) / 2;
                                let world_cx = (world.world_size_x() as i32) / 2;
                                let world_cz = (world.world_size_z() as i32) / 2;
                                let shift_x = world_cx - layout_cx;
                                let shift_z = world_cz - layout_cz;

                                // Helper: find terrain surface y given a world (x,z).
                                let find_surface_y = |wx: i32, wz: i32| -> i32 {
                                    let mut y = world.world_size_y() as i32 - 2;
                                    while y >= 1 {
                                        let b = world.block_at_runtime(&reg, wx, y, wz);
                                        if reg
                                            .get(b.id)
                                            .map(|t| t.is_solid(b.state))
                                            .unwrap_or(false)
                                        {
                                            return (y + 1)
                                                .clamp(1, world.world_size_y() as i32 - 1);
                                        }
                                        y -= 1;
                                    }
                                    1
                                };

                                // 3) For each schematic, compute base world (x,z), choose a terrain height, and stamp.
                                for (p, (lx, lz), (sx, _sy, sz)) in placements {
                                    let wx0 = lx + shift_x;
                                    let wz0 = lz + shift_z;
                                    // Use max surface Y among the four corners to avoid burying edges.
                                    let corners = [
                                        (wx0, wz0),
                                        (wx0 + sx - 1, wz0),
                                        (wx0, wz0 + sz - 1),
                                        (wx0 + sx - 1, wz0 + sz - 1),
                                    ];
                                    let mut wy = i32::MIN;
                                    for (cx, cz) in corners {
                                        wy = wy.max(find_surface_y(cx, cz));
                                    }
                                    // Clamp so the schematic fits vertically within world bounds.
                                    let world_y_top = world.world_size_y() as i32 - 2;
                                    let wy = wy.min(world_y_top);

                                    match geist_io::load_any_schematic_apply_edits(
                                        &p,
                                        (wx0, wy, wz0),
                                        &mut gs.edits,
                                        &reg,
                                    ) {
                                        Ok((sx, sy, sz)) => {
                                            log::info!(
                                                "Loaded schem {:?} at terrain ({},{},{}) ({}x{}x{})",
                                                p,
                                                wx0,
                                                wy,
                                                wz0,
                                                sx,
                                                sy,
                                                sz
                                            );
                                        }
                                        Err(e) => {
                                            log::warn!("Failed loading schem {:?}: {}", p, e);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed scanning schematics dir {:?}: {}", dir, e);
                    }
                }
                // mcworld imports removed
            } else {
                log::info!("Schematics dir {:?} not found; skipping.", dir);
            }
        }

        // Bootstrap initial streaming based on camera (after edits are applied)
        let ccx = (cam.position.x / world.chunk_size_x as f32).floor() as i32;
        let ccz = (cam.position.z / world.chunk_size_z as f32).floor() as i32;
        queue.emit_now(Event::ViewCenterChanged { ccx, ccz });
        // Do not spawn a default platform in non-flat: schematics drive platform creation now.
        // Default place_type: stone
        if let Some(id) = reg.id_by_name("stone") {
            gs.place_type = Block { id, state: 0 };
        }

        Self {
            gs,
            queue,
            runtime,
            cam,
            debug_stats: DebugStats::default(),
            hotbar,
            leaves_shader,
            fog_shader,
            tex_cache,
            renders: HashMap::new(),
            structure_renders: HashMap::new(),
            reg: reg.clone(),
            evt_processed_total: 0,
            evt_processed_by: HashMap::new(),
            intents: HashMap::new(),
            tex_event_rx: tex_rx,
            worldgen_event_rx: wg_rx,
            world_config_path,
            rebuild_on_worldgen,
            worldgen_dirty: false,
            assets_root: assets_root.clone(),
            reg_event_rx: {
                let (rtx, rrx) = std::sync::mpsc::channel::<()>();
                let mats = crate::assets::materials_path(&assets_root);
                let blks = crate::assets::blocks_path(&assets_root);
                std::thread::spawn(move || {
                    use notify::{EventKind, RecursiveMode, Watcher};
                    if let Ok(mut watcher) = notify::recommended_watcher(
                        move |res: Result<notify::Event, notify::Error>| {
                            if let Ok(event) = res {
                                match event.kind {
                                    EventKind::Modify(_)
                                    | EventKind::Create(_)
                                    | EventKind::Remove(_)
                                    | EventKind::Any => {
                                        let _ = rtx.send(());
                                    }
                                    _ => {}
                                }
                            }
                        },
                    ) {
                        let _ = watcher.watch(mats.as_path(), RecursiveMode::NonRecursive);
                        let _ = watcher.watch(blks.as_path(), RecursiveMode::NonRecursive);
                        loop {
                            std::thread::sleep(std::time::Duration::from_secs(3600));
                        }
                    }
                });
                rrx
            },
            shader_event_rx: {
                let (stx, srx) = std::sync::mpsc::channel::<()>();
                let sdir = crate::assets::shaders_dir(&assets_root);
                std::thread::spawn(move || {
                    use notify::{EventKind, RecursiveMode, Watcher};
                    if let Ok(mut watcher) = notify::recommended_watcher(
                        move |res: Result<notify::Event, notify::Error>| {
                            if let Ok(event) = res {
                                match event.kind {
                                    EventKind::Modify(_)
                                    | EventKind::Create(_)
                                    | EventKind::Remove(_)
                                    | EventKind::Any => {
                                        let _ = stx.send(());
                                    }
                                    _ => {}
                                }
                            }
                        },
                    ) {
                        let _ = watcher.watch(sdir.as_path(), RecursiveMode::Recursive);
                        loop {
                            std::thread::sleep(std::time::Duration::from_secs(3600));
                        }
                    }
                });
                srx
            },
        }
    }

    fn neighbor_mask(&self, cx: i32, cz: i32) -> NeighborsLoaded {
        NeighborsLoaded {
            neg_x: self.renders.contains_key(&(cx - 1, cz)),
            pos_x: self.renders.contains_key(&(cx + 1, cz)),
            neg_z: self.renders.contains_key(&(cx, cz - 1)),
            pos_z: self.renders.contains_key(&(cx, cz + 1)),
        }
    }

    fn job_hash(cx: i32, cz: i32, rev: u64, n: NeighborsLoaded) -> u64 {
        // Simple stable hash (FNV-1a 64-bit variant)
        let mut h: u64 = 0xcbf29ce484222325;
        let mut write = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x100000001b3);
        };
        write(cx as u64);
        write(cz as u64);
        write(rev);
        let mask = (n.neg_x as u64)
            | ((n.pos_x as u64) << 1)
            | ((n.neg_z as u64) << 2)
            | ((n.pos_z as u64) << 3);
        write(mask);
        h
    }

    fn handle_event(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread, env: EventEnvelope) {
        // Log a concise line for the processed event
        Self::log_event(self.gs.tick, &env.kind);
        match env.kind {
            Event::Tick => {}
            Event::StructurePoseUpdated {
                id,
                pos,
                yaw_deg,
                delta,
            } => {
                if let Some(st) = self.gs.structures.get_mut(&id) {
                    st.last_delta = vec3_from_rl(delta);
                    st.pose.pos = vec3_from_rl(pos);
                    st.pose.yaw_deg = yaw_deg;
                    // Keep player perfectly in sync if attached to this structure
                    if let Some(att) = self.gs.ground_attach {
                        if att.id == id {
                            let wl = rotate_yaw(vec3_from_rl(att.local_offset), st.pose.yaw_deg);
                            let world_from_local = Vec3 {
                                x: wl.x + st.pose.pos.x,
                                y: wl.y + st.pose.pos.y,
                                z: wl.z + st.pose.pos.z,
                            };
                            self.gs.walker.pos = vec3_to_rl(world_from_local);
                        }
                    }
                }
            }
            Event::MovementRequested {
                dt_ms,
                yaw,
                walk_mode: _,
            } => {
                // update camera look first (yaw drives walker forward)
                if self.gs.walk_mode {
                    // Collision sampler: structures > edits > buf > world
                    let sx = self.gs.world.chunk_size_x as i32;
                    let sz = self.gs.world.chunk_size_z as i32;
                    // Platform attachment: handle attachment and movement
                    let feet_world = self.gs.walker.pos;

                    // First, check for new attachment
                    if self.gs.ground_attach.is_none() {
                        for (id, st) in &self.gs.structures {
                            if self.is_feet_on_structure(st, feet_world) {
                                // Capture local feet offset and attach
                                let p = vec3_from_rl(self.gs.walker.pos);
                                let diff = Vec3 {
                                    x: p.x - st.pose.pos.x,
                                    y: p.y - st.pose.pos.y,
                                    z: p.z - st.pose.pos.z,
                                };
                                let local = rotate_yaw_inv(diff, st.pose.yaw_deg);
                                self.gs.ground_attach = Some(crate::gamestate::GroundAttach {
                                    id: *id,
                                    grace: 8,
                                    local_offset: vec3_to_rl(local),
                                });
                                // Emit lifecycle event for observability
                                self.queue.emit_now(Event::PlayerAttachedToStructure {
                                    id: *id,
                                    local_offset: vec3_to_rl(local),
                                });
                                break;
                            }
                        }
                    }

                    // If attached, move with the platform BEFORE physics
                    if let Some(att) = self.gs.ground_attach {
                        if let Some(st) = self.gs.structures.get(&att.id) {
                            // Calculate where we should be based on our local offset and the platform's current position
                            let wl = rotate_yaw(vec3_from_rl(att.local_offset), st.pose.yaw_deg);
                            let target_world_pos = Vec3 {
                                x: wl.x + st.pose.pos.x,
                                y: wl.y + st.pose.pos.y,
                                z: wl.z + st.pose.pos.z,
                            };

                            // Move the player to maintain their position on the platform
                            self.gs.walker.pos = vec3_to_rl(target_world_pos);
                        } else {
                            self.gs.ground_attach = None;
                        }
                    }
                    let reg = &self.reg;
                    let sampler = |wx: i32, wy: i32, wz: i32| -> Block {
                        // Check dynamic structures first
                        for st in self.gs.structures.values() {
                            let p = vec3_from_rl(Vector3::new(
                                wx as f32 + 0.5,
                                wy as f32 + 0.5,
                                wz as f32 + 0.5,
                            ));
                            let diff = Vec3 {
                                x: p.x - st.pose.pos.x,
                                y: p.y - st.pose.pos.y,
                                z: p.z - st.pose.pos.z,
                            };
                            let local = rotate_yaw_inv(diff, st.pose.yaw_deg);
                            let lx = local.x.floor() as i32;
                            let ly = local.y.floor() as i32;
                            let lz = local.z.floor() as i32;
                            if lx >= 0
                                && ly >= 0
                                && lz >= 0
                                && (lx as usize) < st.sx
                                && (ly as usize) < st.sy
                                && (lz as usize) < st.sz
                            {
                                if let Some(b) = st.edits.get(lx, ly, lz) {
                                    if reg.get(b.id).map(|t| t.is_solid(b.state)).unwrap_or(false) {
                                        return b;
                                    }
                                }
                                let idx = st.idx(lx as usize, ly as usize, lz as usize);
                                let b = st.blocks[idx];
                                if reg.get(b.id).map(|t| t.is_solid(b.state)).unwrap_or(false) {
                                    return b;
                                }
                            }
                        }
                        if let Some(b) = self.gs.edits.get(wx, wy, wz) {
                            return b;
                        }
                        let cx = wx.div_euclid(sx);
                        let cz = wz.div_euclid(sz);
                        if let Some(cent) = self.gs.chunks.get(&(cx, cz)) {
                            if let Some(ref buf) = cent.buf {
                                return buf.get_world(wx, wy, wz).unwrap_or(Block::AIR);
                            }
                        }
                        self.gs.world.block_at_runtime(reg, wx, wy, wz)
                    };
                    self.gs.walker.update_with_sampler(
                        rl,
                        &sampler,
                        &self.gs.world,
                        &self.reg,
                        (dt_ms as f32) / 1000.0,
                        yaw,
                        None, // No platform velocity needed - we handle movement via teleportation
                    );
                    // Update attachment after physics - critical for allowing movement on platform
                    if let Some(att) = self.gs.ground_attach {
                        if let Some(st) = self.gs.structures.get(&att.id) {
                            // Calculate new local position after physics (player may have moved)
                            let p = vec3_from_rl(self.gs.walker.pos);
                            let diff = Vec3 {
                                x: p.x - st.pose.pos.x,
                                y: p.y - st.pose.pos.y,
                                z: p.z - st.pose.pos.z,
                            };
                            let new_local = rotate_yaw_inv(diff, st.pose.yaw_deg);

                            // Check if we're still on the structure after physics
                            if self.is_feet_on_structure(st, self.gs.walker.pos) {
                                // Update attachment with new local offset (this allows movement on the platform)
                                self.gs.ground_attach = Some(crate::gamestate::GroundAttach {
                                    id: att.id,
                                    grace: 8,
                                    local_offset: vec3_to_rl(new_local),
                                });
                            } else if att.grace > 0 {
                                // We've left the structure surface but have grace period (jumping/stepping off edge)
                                self.gs.ground_attach = Some(crate::gamestate::GroundAttach {
                                    id: att.id,
                                    grace: att.grace - 1,
                                    local_offset: vec3_to_rl(new_local),
                                });
                            } else {
                                // Grace period expired, detach
                                self.gs.ground_attach = None;
                                self.queue
                                    .emit_now(Event::PlayerDetachedFromStructure { id: att.id });
                            }
                        } else {
                            self.gs.ground_attach = None;
                        }
                    }
                    self.cam.position = self.gs.walker.eye_position();
                    // Emit ViewCenterChanged if center moved
                    let ccx =
                        (self.cam.position.x / self.gs.world.chunk_size_x as f32).floor() as i32;
                    let ccz =
                        (self.cam.position.z / self.gs.world.chunk_size_z as f32).floor() as i32;
                    if (ccx, ccz) != self.gs.center_chunk {
                        self.queue.emit_now(Event::ViewCenterChanged { ccx, ccz });
                    }
                } else {
                    // Fly camera mode moves the camera in step(); update view center from camera
                    let ccx =
                        (self.cam.position.x / self.gs.world.chunk_size_x as f32).floor() as i32;
                    let ccz =
                        (self.cam.position.z / self.gs.world.chunk_size_z as f32).floor() as i32;
                    if (ccx, ccz) != self.gs.center_chunk {
                        self.queue.emit_now(Event::ViewCenterChanged { ccx, ccz });
                    }
                }
            }
            Event::PlayerAttachedToStructure { id, local_offset } => {
                // Idempotent: set/refresh attachment state
                if self.gs.structures.contains_key(&id) {
                    self.gs.ground_attach = Some(crate::gamestate::GroundAttach {
                        id,
                        grace: 8,
                        local_offset,
                    });
                }
            }
            Event::PlayerDetachedFromStructure { id } => {
                if let Some(att) = self.gs.ground_attach {
                    if att.id == id {
                        self.gs.ground_attach = None;
                    }
                }
            }
            Event::ViewCenterChanged { ccx, ccz } => {
                self.gs.center_chunk = (ccx, ccz);
                // Determine desired set
                let r = self.gs.view_radius_chunks;
                let mut desired: HashSet<(i32, i32)> = HashSet::new();
                for dz in -r..=r {
                    for dx in -r..=r {
                        desired.insert((ccx + dx, ccz + dz));
                    }
                }
                // Unload far ones
                let current: Vec<(i32, i32)> = self.renders.keys().cloned().collect();
                for key in current {
                    if !desired.contains(&key) {
                        self.queue.emit_now(Event::EnsureChunkUnloaded {
                            cx: key.0,
                            cz: key.1,
                        });
                    }
                }
                // No explicit inflight cancellation for far chunks; allow in-flight jobs to complete.
                // Prune stream-load intents well outside the new radius (hysteresis: r+1)
                let mut to_remove: Vec<(i32, i32)> = Vec::new();
                for (&(ix, iz), ent) in self.intents.iter() {
                    if matches!(ent.cause, IntentCause::StreamLoad) {
                        let dx = (ix - ccx).abs();
                        let dz = (iz - ccz).abs();
                        let ring = dx.max(dz);
                        if ring > r + 1 {
                            to_remove.push((ix, iz));
                        }
                    }
                }
                for k in to_remove {
                    self.intents.remove(&k);
                }
                // Load new ones
                for key in desired {
                    if !self.renders.contains_key(&key) && !self.gs.inflight_rev.contains_key(&key)
                    {
                        self.queue.emit_now(Event::EnsureChunkLoaded {
                            cx: key.0,
                            cz: key.1,
                        });
                    }
                }
            }
            Event::EnsureChunkUnloaded { cx, cz } => {
                self.renders.remove(&(cx, cz));
                self.gs.chunks.remove(&(cx, cz));
                self.gs.loaded.remove(&(cx, cz));
                self.gs.inflight_rev.remove(&(cx, cz));
                // Also drop any persisted lighting state for this chunk to prevent growth
                self.gs.lighting.clear_chunk(cx, cz);
            }
            Event::EnsureChunkLoaded { cx, cz } => {
                if self.renders.contains_key(&(cx, cz))
                    || self.gs.inflight_rev.contains_key(&(cx, cz))
                {
                    return;
                }
                // Record load intent; scheduler will cap and prioritize
                self.record_intent(cx, cz, IntentCause::StreamLoad);
            }
            Event::BuildChunkJobRequested {
                cx,
                cz,
                neighbors,
                rev,
                job_id,
                cause,
            } => {
                // Prepare edit snapshots for workers (pure)
                let chunk_edits = self.gs.edits.snapshot_for_chunk(cx, cz);
                let region_edits = self.gs.edits.snapshot_for_region(cx, cz, 1);
                // Try to reuse previous buffer if present (and not invalidated)
                let prev_buf = self
                    .gs
                    .chunks
                    .get(&(cx, cz))
                    .and_then(|c| c.buf.as_ref())
                    .cloned();
                let job = BuildJob {
                    cx,
                    cz,
                    neighbors,
                    rev,
                    job_id,
                    chunk_edits,
                    region_edits,
                    prev_buf,
                    reg: self.reg.clone(),
                };
                match cause {
                    RebuildCause::Edit => {
                        self.runtime.submit_build_job_edit(job);
                    }
                    RebuildCause::LightingBorder => {
                        self.runtime.submit_build_job_light(job);
                    }
                    RebuildCause::StreamLoad => {
                        self.runtime.submit_build_job_bg(job);
                    }
                }
                // inflight_rev was set by the emitter (EnsureChunkLoaded/ChunkRebuildRequested) or requeue branch.
            }
            Event::StructureBuildRequested { id, rev } => {
                if let Some(st) = self.gs.structures.get(&id) {
                    let job = StructureBuildJob {
                        id,
                        rev,
                        sx: st.sx,
                        sy: st.sy,
                        sz: st.sz,
                        base_blocks: st.blocks.clone(),
                        edits: st.edits.snapshot_all(),
                        reg: self.reg.clone(),
                    };
                    self.runtime.submit_structure_build_job(job);
                }
            }
            Event::StructureBuildCompleted { id, rev, cpu } => {
                if let Some(mut cr) =
                    upload_chunk_mesh(rl, thread, cpu, &mut self.tex_cache, &self.reg.materials)
                {
                    for (mid, model) in &mut cr.parts {
                        if let Some(mat) = model.materials_mut().get_mut(0) {
                            let leaves = self
                                .reg
                                .materials
                                .get(*mid)
                                .and_then(|m| m.render_tag.as_deref())
                                == Some("leaves");
                            if leaves {
                                if let Some(ref ls) = self.leaves_shader {
                                    let dest = mat.shader_mut();
                                    let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                    let src_ptr: *const raylib::ffi::Shader = ls.shader.as_ref();
                                    unsafe {
                                        std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1);
                                    }
                                }
                            } else if let Some(ref fs) = self.fog_shader {
                                let dest = mat.shader_mut();
                                let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                let src_ptr: *const raylib::ffi::Shader = fs.shader.as_ref();
                                unsafe {
                                    std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1);
                                }
                            }
                        }
                    }
                    self.structure_renders.insert(id, cr);
                }
                if let Some(st) = self.gs.structures.get_mut(&id) {
                    st.built_rev = rev;
                }
            }
            Event::BuildChunkJobCompleted {
                cx,
                cz,
                rev,
                cpu,
                buf,
                light_borders,
                job_id: _,
            } => {
                // Drop if stale
                let cur_rev = self.gs.edits.get_rev(cx, cz);
                if rev < cur_rev {
                    // Only re-enqueue if there isn't already a newer inflight job
                    let inflight = self.gs.inflight_rev.get(&(cx, cz)).copied().unwrap_or(0);
                    if inflight < cur_rev {
                        let neighbors = self.neighbor_mask(cx, cz);
                        let job_id = Self::job_hash(cx, cz, cur_rev, neighbors);
                        self.queue.emit_now(Event::BuildChunkJobRequested {
                            cx,
                            cz,
                            neighbors,
                            rev: cur_rev,
                            job_id,
                            cause: RebuildCause::Edit,
                        });
                        // Ensure inflight_rev reflects latest
                        self.gs.inflight_rev.insert((cx, cz), cur_rev);
                    }
                    return;
                }
                // Gate completion by desired radius: if chunk is no longer desired, drop
                let (ccx, ccz) = self.gs.center_chunk;
                let dx = (cx - ccx).abs();
                let dz = (cz - ccz).abs();
                let ring = dx.max(dz);
                if ring > self.gs.view_radius_chunks {
                    // Not desired anymore: clear inflight and abandon result
                    self.gs.inflight_rev.remove(&(cx, cz));
                    // Do not upload or mark built; also avoid lighting border updates
                    return;
                }
                // Upload to GPU
                if let Some(mut cr) =
                    upload_chunk_mesh(rl, thread, cpu, &mut self.tex_cache, &self.reg.materials)
                {
                    // Assign biome-based leaf tint for this chunk (center sample)
                    let sx = self.gs.world.chunk_size_x as i32;
                    let sz = self.gs.world.chunk_size_z as i32;
                    let wx = cx * sx + sx / 2;
                    let wz = cz * sz + sz / 2;
                    if let Some(b) = self.gs.world.biome_at(wx, wz) {
                        if let Some(t) = b.leaf_tint {
                            cr.leaf_tint = Some(t);
                        }
                    }
                    // Assign shaders
                    for (mid, model) in &mut cr.parts {
                        if let Some(mat) = model.materials_mut().get_mut(0) {
                            let leaves = self
                                .reg
                                .materials
                                .get(*mid)
                                .and_then(|m| m.render_tag.as_deref())
                                == Some("leaves");
                            if leaves {
                                if let Some(ref ls) = self.leaves_shader {
                                    let dest = mat.shader_mut();
                                    let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                    let src_ptr: *const raylib::ffi::Shader = ls.shader.as_ref();
                                    unsafe {
                                        std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1);
                                    }
                                }
                            } else if let Some(ref fs) = self.fog_shader {
                                let dest = mat.shader_mut();
                                let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                let src_ptr: *const raylib::ffi::Shader = fs.shader.as_ref();
                                unsafe {
                                    std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1);
                                }
                            }
                        }
                    }
                    self.renders.insert((cx, cz), cr);
                }
                // Update CPU buf & built rev
                self.gs.chunks.insert(
                    (cx, cz),
                    ChunkEntry {
                        buf: Some(buf),
                        built_rev: rev,
                    },
                );
                self.gs.loaded.insert((cx, cz));
                self.gs.inflight_rev.remove(&(cx, cz));
                self.gs.edits.mark_built(cx, cz, rev);

                // Update light borders in main thread; if changed, emit a dedicated event
                if let Some(lb) = light_borders {
                    let _changed = self.gs.lighting.update_borders(cx, cz, lb);
                    // Always notify neighbors so they can pick up new micro seam planes
                    self.queue.emit_now(Event::LightBordersUpdated { cx, cz });
                }
            }
            Event::ChunkRebuildRequested { cx, cz, cause } => {
                if !self.renders.contains_key(&(cx, cz)) {
                    return;
                }
                // Record rebuild intent; scheduler will cap and prioritize
                let ic = match cause {
                    RebuildCause::Edit => IntentCause::Edit,
                    RebuildCause::LightingBorder => IntentCause::Light,
                    RebuildCause::StreamLoad => IntentCause::StreamLoad,
                };
                self.record_intent(cx, cz, ic);
            }
            Event::RaycastEditRequested { place, block } => {
                // Perform world + structure raycast and emit edit events
                let org = self.cam.position;
                let dir = self.cam.forward();
                let sx = self.gs.world.chunk_size_x as i32;
                let sz = self.gs.world.chunk_size_z as i32;
                let reg = self.reg.clone();
                let sampler = |wx: i32, wy: i32, wz: i32| -> Block {
                    if let Some(b) = self.gs.edits.get(wx, wy, wz) {
                        return b;
                    }
                    let cx = wx.div_euclid(sx);
                    let cz = wz.div_euclid(sz);
                    if let Some(cent) = self.gs.chunks.get(&(cx, cz)) {
                        if let Some(ref buf) = cent.buf {
                            return buf.get_world(wx, wy, wz).unwrap_or(Block {
                                id: reg.id_by_name("air").unwrap_or(0),
                                state: 0,
                            });
                        }
                    }
                    // Outside loaded buffers: treat as air
                    Block {
                        id: reg.id_by_name("air").unwrap_or(0),
                        state: 0,
                    }
                };
                let world_hit =
                    raycast::raycast_first_hit_with_face(org, dir, 8.0 * 32.0, |x, y, z| {
                        let b = sampler(x, y, z);
                        self.reg
                            .get(b.id)
                            .map(|ty| ty.is_solid(b.state))
                            .unwrap_or(false)
                    });
                let mut struct_hit: Option<(StructureId, raycast::RayHit, f32)> = None;
                for (id, st) in &self.gs.structures {
                    let o = vec3_from_rl(org);
                    let diff = Vec3 {
                        x: o.x - st.pose.pos.x,
                        y: o.y - st.pose.pos.y,
                        z: o.z - st.pose.pos.z,
                    };
                    let local_org = vec3_to_rl(rotate_yaw_inv(diff, st.pose.yaw_deg));
                    let local_dir = vec3_to_rl(rotate_yaw_inv(vec3_from_rl(dir), st.pose.yaw_deg));
                    let is_solid_local = |lx: i32, ly: i32, lz: i32| -> bool {
                        if lx < 0 || ly < 0 || lz < 0 {
                            return false;
                        }
                        let (lxu, lyu, lzu) = (lx as usize, ly as usize, lz as usize);
                        if lxu >= st.sx || lyu >= st.sy || lzu >= st.sz {
                            return false;
                        }
                        if let Some(b) = st.edits.get(lx, ly, lz) {
                            return self
                                .reg
                                .get(b.id)
                                .map(|ty| ty.is_solid(b.state))
                                .unwrap_or(false);
                        }
                        let b = st.blocks[st.idx(lxu, lyu, lzu)];
                        self.reg
                            .get(b.id)
                            .map(|ty| ty.is_solid(b.state))
                            .unwrap_or(false)
                    };
                    if let Some(hit) = raycast::raycast_first_hit_with_face(
                        local_org,
                        local_dir,
                        8.0 * 32.0,
                        is_solid_local,
                    ) {
                        let cc_local = Vector3::new(
                            hit.bx as f32 + 0.5,
                            hit.by as f32 + 0.5,
                            hit.bz as f32 + 0.5,
                        );
                        let wl = rotate_yaw(vec3_from_rl(cc_local), st.pose.yaw_deg);
                        let cc_world = Vec3 {
                            x: wl.x + st.pose.pos.x,
                            y: wl.y + st.pose.pos.y,
                            z: wl.z + st.pose.pos.z,
                        };
                        let cw = vec3_to_rl(cc_world);
                        let d = Vector3::new(cw.x - org.x, cw.y - org.y, cw.z - org.z);
                        let dist2 = d.x * d.x + d.y * d.y + d.z * d.z;
                        struct_hit = Some((*id, hit, dist2));
                        break;
                    }
                }
                let choose_struct = match (world_hit.as_ref(), struct_hit.as_ref()) {
                    (None, Some(_)) => true,
                    (Some(_), None) => false,
                    (Some(wh), Some((_id, _sh, sdist2))) => {
                        let wc = Vector3::new(
                            wh.bx as f32 + 0.5,
                            wh.by as f32 + 0.5,
                            wh.bz as f32 + 0.5,
                        );
                        let dw = wc - org;
                        let wdist2 = dw.x * dw.x + dw.y * dw.y + dw.z * dw.z;
                        *sdist2 < wdist2
                    }
                    _ => false,
                };
                if choose_struct {
                    if let Some((id, hit, _)) = struct_hit {
                        if place {
                            // Place on the adjacent empty cell directly (no extra normal offset)
                            let (lx, ly, lz) = (hit.px, hit.py, hit.pz);
                            self.queue.emit_now(Event::StructureBlockPlaced {
                                id,
                                lx,
                                ly,
                                lz,
                                block,
                            });
                        } else {
                            self.queue.emit_now(Event::StructureBlockRemoved {
                                id,
                                lx: hit.bx,
                                ly: hit.by,
                                lz: hit.bz,
                            });
                        }
                    }
                } else if let Some(hit) = world_hit {
                    if place {
                        let wx = hit.px;
                        let wy = hit.py;
                        let wz = hit.pz;
                        if wy >= 0 && wy < self.gs.world.chunk_size_y as i32 {
                            self.queue
                                .emit_now(Event::BlockPlaced { wx, wy, wz, block });
                        }
                    } else {
                        let wx = hit.bx;
                        let wy = hit.by;
                        let wz = hit.bz;
                        let prev = sampler(wx, wy, wz);
                        if self
                            .reg
                            .get(prev.id)
                            .map(|t| t.is_solid(prev.state))
                            .unwrap_or(false)
                        {
                            self.queue.emit_now(Event::BlockRemoved { wx, wy, wz });
                        }
                    }
                }
            }
            Event::StructureBlockPlaced {
                id,
                lx,
                ly,
                lz,
                block,
            } => {
                if let Some(st) = self.gs.structures.get_mut(&id) {
                    st.set_local(lx, ly, lz, block);
                    let rev = st.dirty_rev;
                    self.queue
                        .emit_now(Event::StructureBuildRequested { id, rev });
                }
            }
            Event::StructureBlockRemoved { id, lx, ly, lz } => {
                if let Some(st) = self.gs.structures.get_mut(&id) {
                    st.remove_local(lx, ly, lz);
                    let rev = st.dirty_rev;
                    self.queue
                        .emit_now(Event::StructureBuildRequested { id, rev });
                }
            }
            Event::BlockPlaced { wx, wy, wz, block } => {
                self.gs.edits.set(wx, wy, wz, block);
                let em = self
                    .reg
                    .get(block.id)
                    .map(|t| t.light_emission(block.state))
                    .unwrap_or(0);
                if em > 0 {
                    let is_beacon = self
                        .reg
                        .get(block.id)
                        .map(|t| t.light_is_beam())
                        .unwrap_or(false);
                    self.queue.emit_now(Event::LightEmitterAdded {
                        wx,
                        wy,
                        wz,
                        level: em,
                        is_beacon,
                    });
                }
                let _ = self.gs.edits.bump_region_around(wx, wz);
                // Rebuild edited chunk and any boundary-adjacent neighbors that are loaded
                for (cx, cz) in self.gs.edits.get_affected_chunks(wx, wz) {
                    if self.renders.contains_key(&(cx, cz)) {
                        self.queue.emit_now(Event::ChunkRebuildRequested {
                            cx,
                            cz,
                            cause: RebuildCause::Edit,
                        });
                    }
                }
            }
            Event::BlockRemoved { wx, wy, wz } => {
                // Determine previous block to update lighting
                let sx = self.gs.world.chunk_size_x as i32;
                let sz = self.gs.world.chunk_size_z as i32;
                let reg = &self.reg;
                let sampler = |wx: i32, wy: i32, wz: i32| -> Block {
                    if let Some(b) = self.gs.edits.get(wx, wy, wz) {
                        return b;
                    }
                    let cx = wx.div_euclid(sx);
                    let cz = wz.div_euclid(sz);
                    if let Some(cent) = self.gs.chunks.get(&(cx, cz)) {
                        if let Some(ref buf) = cent.buf {
                            return buf.get_world(wx, wy, wz).unwrap_or(Block::AIR);
                        }
                    }
                    self.gs.world.block_at_runtime(reg, wx, wy, wz)
                };
                let prev = sampler(wx, wy, wz);
                let prev_em = self
                    .reg
                    .get(prev.id)
                    .map(|t| t.light_emission(prev.state))
                    .unwrap_or(0);
                if prev_em > 0 {
                    self.queue
                        .emit_now(Event::LightEmitterRemoved { wx, wy, wz });
                }
                self.gs.edits.set(wx, wy, wz, Block::AIR);
                let _ = self.gs.edits.bump_region_around(wx, wz);
                for (cx, cz) in self.gs.edits.get_affected_chunks(wx, wz) {
                    if self.renders.contains_key(&(cx, cz)) {
                        self.queue.emit_now(Event::ChunkRebuildRequested {
                            cx,
                            cz,
                            cause: RebuildCause::Edit,
                        });
                    }
                }
            }
            Event::LightEmitterAdded {
                wx,
                wy,
                wz,
                level,
                is_beacon,
            } => {
                if is_beacon {
                    self.gs.lighting.add_beacon_world(wx, wy, wz, level);
                } else {
                    self.gs.lighting.add_emitter_world(wx, wy, wz, level);
                }
                // schedule rebuild of that chunk
                let sx = self.gs.world.chunk_size_x as i32;
                let sz = self.gs.world.chunk_size_z as i32;
                let cx = wx.div_euclid(sx);
                let cz = wz.div_euclid(sz);
                self.queue.emit_now(Event::ChunkRebuildRequested {
                    cx,
                    cz,
                    cause: RebuildCause::Edit,
                });
            }
            Event::LightEmitterRemoved { wx, wy, wz } => {
                self.gs.lighting.remove_emitter_world(wx, wy, wz);
                let sx = self.gs.world.chunk_size_x as i32;
                let sz = self.gs.world.chunk_size_z as i32;
                let cx = wx.div_euclid(sx);
                let cz = wz.div_euclid(sz);
                self.queue.emit_now(Event::ChunkRebuildRequested {
                    cx,
                    cz,
                    cause: RebuildCause::Edit,
                });
            }
            Event::LightBordersUpdated { cx, cz } => {
                // Neighbor rebuilds in response to border changes, if loaded and not already inflight
                let (ccx, ccz) = self.gs.center_chunk;
                let r_gate = self.gs.view_radius_chunks + 1; // small hysteresis
                for (nx, nz) in [(cx - 1, cz), (cx + 1, cz), (cx, cz - 1), (cx, cz + 1)] {
                    // Distance gating to avoid far-away lighting backlogs
                    let ring = (nx - ccx).abs().max((nz - ccz).abs());
                    if ring > r_gate {
                        continue;
                    }
                    if self.renders.contains_key(&(nx, nz))
                        && !self.gs.inflight_rev.contains_key(&(nx, nz))
                    {
                        self.queue.emit_now(Event::ChunkRebuildRequested {
                            cx: nx,
                            cz: nz,
                            cause: RebuildCause::LightingBorder,
                        });
                    }
                }
            }
            Event::WalkModeToggled => {
                let new_mode = !self.gs.walk_mode;
                self.gs.walk_mode = new_mode;
                if new_mode {
                    // Entering walk mode: align walker to current camera eye position
                    self.gs.walker.yaw = self.cam.yaw;
                    let mut p = self.cam.position;
                    p.y -= self.gs.walker.eye_height; // convert eye -> feet position
                    // Only clamp to ground (min Y); allow above-ceiling positions (e.g., flying structures)
                    p.y = p.y.max(0.0);
                    self.gs.walker.pos = p;
                    self.gs.walker.vel = Vector3::zero();
                    self.gs.walker.on_ground = false;
                    // Keep camera exactly at walker eye to avoid any snap
                    self.cam.position = self.gs.walker.eye_position();
                } else {
                    // Entering fly mode: camera already at walker eye; continue from here
                }
            }
            Event::GridToggled => {
                self.gs.show_grid = !self.gs.show_grid;
            }
            Event::WireframeToggled => {
                self.gs.wireframe = !self.gs.wireframe;
            }
            Event::ChunkBoundsToggled => {
                self.gs.show_chunk_bounds = !self.gs.show_chunk_bounds;
            }
            Event::FrustumCullingToggled => {
                self.gs.frustum_culling_enabled = !self.gs.frustum_culling_enabled;
            }
            Event::BiomeLabelToggled => {
                self.gs.show_biome_label = !self.gs.show_biome_label;
            }
            Event::PlaceTypeSelected { block } => {
                self.gs.place_type = block;
            }
        }
    }

    pub fn step(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread, dt: f32) {
        // Shader hot-reload
        if self.shader_event_rx.try_iter().next().is_some() {
            // Attempt to reload both shaders; fall back to previous if load fails
            if let Some(ls) = geist_render_raylib::LeavesShader::load_with_base(
                rl,
                thread,
                &self.assets_root,
            ) {
                self.leaves_shader = Some(ls);
            }
            if let Some(fs) = geist_render_raylib::FogShader::load_with_base(
                rl,
                thread,
                &self.assets_root,
            ) {
                self.fog_shader = Some(fs);
            }
            // Rebind shaders on all existing models
            let mut rebind = |parts: &mut Vec<(geist_blocks::types::MaterialId, raylib::core::models::Model)>| {
                for (mid, model) in parts.iter_mut() {
                    if let Some(mat) = model.materials_mut().get_mut(0) {
                        let leaves = self
                            .reg
                            .materials
                            .get(*mid)
                            .and_then(|m| m.render_tag.as_deref())
                            == Some("leaves");
                        if leaves {
                            if let Some(ref ls) = self.leaves_shader {
                                let dest = mat.shader_mut();
                                let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                let src_ptr: *const raylib::ffi::Shader = ls.shader.as_ref();
                                unsafe { std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1) };
                            }
                        } else if let Some(ref fs) = self.fog_shader {
                            let dest = mat.shader_mut();
                            let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                            let src_ptr: *const raylib::ffi::Shader = fs.shader.as_ref();
                            unsafe { std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1) };
                        }
                    }
                }
            };
            for (_k, cr) in self.renders.iter_mut() {
                rebind(&mut cr.parts);
            }
            for (_id, cr) in self.structure_renders.iter_mut() {
                rebind(&mut cr.parts);
            }
            log::info!("Reloaded shaders and rebound on existing models");
        }
        // Registry hot-reload (materials/blocks)
        if self.reg_event_rx.try_iter().next().is_some() {
            let mats = crate::assets::materials_path(&self.assets_root);
            let blks = crate::assets::blocks_path(&self.assets_root);
            match geist_blocks::BlockRegistry::load_from_paths(&mats, &blks) {
                Ok(mut newreg) => {
                    for m in &mut newreg.materials.materials {
                        for p in &mut m.texture_candidates {
                            if p.is_relative() {
                                *p = self.assets_root.join(&p);
                            }
                        }
                    }
                    self.reg = std::sync::Arc::new(newreg);
                    self.tex_cache.map.clear();
                    let keys: Vec<(i32, i32)> = self.renders.keys().cloned().collect();
                    for (cx, cz) in keys {
                        self.queue.emit_now(Event::ChunkRebuildRequested { cx, cz, cause: RebuildCause::StreamLoad });
                    }
                    for (id, st) in self.gs.structures.iter() {
                        let next_rev = st.built_rev.wrapping_add(1);
                        self.queue.emit_now(Event::StructureBuildRequested { id: *id, rev: next_rev });
                    }
                    log::info!("Reloaded voxel registry and scheduled rebuilds");
                }
                Err(e) => log::warn!("Registry reload failed: {}", e),
            }
        }
        // Handle worldgen hot-reload
        // Always invalidate previous CPU buffers on change; optionally schedule rebuilds
        if self.take_worldgen_dirty() {
            let keys: Vec<(i32, i32)> = self.renders.keys().cloned().collect();
            for (cx, cz) in keys.iter().copied() {
                if let Some(ent) = self.gs.chunks.get_mut(&(cx, cz)) {
                    ent.buf = None; // prevent reuse across worldgen param changes
                }
            }
            if self.rebuild_on_worldgen {
                for (cx, cz) in keys.iter().copied() {
                    self.queue.emit_now(Event::ChunkRebuildRequested {
                        cx,
                        cz,
                        cause: RebuildCause::StreamLoad,
                    });
                }
                log::info!(
                    "Scheduled rebuild of {} loaded chunks due to worldgen change",
                    keys.len()
                );
            } else {
                log::info!(
                    "Worldgen changed; invalidated {} chunk buffers (rebuild on demand)",
                    keys.len()
                );
            }
        }
        // Input handling → emit events
        if rl.is_key_pressed(KeyboardKey::KEY_V) {
            self.queue.emit_now(Event::WalkModeToggled);
        }
        if self.gs.walk_mode {
            self.cam.update_look_only(rl, dt);
        } else {
            self.cam.update(rl, dt);
        }

        if rl.is_key_pressed(KeyboardKey::KEY_G) {
            self.queue.emit_now(Event::GridToggled);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_F) {
            self.queue.emit_now(Event::WireframeToggled);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_B) {
            self.queue.emit_now(Event::ChunkBoundsToggled);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_C) {
            self.queue.emit_now(Event::FrustumCullingToggled);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_H) {
            self.queue.emit_now(Event::BiomeLabelToggled);
        }
        // Hotbar selection: if config present, use it; else fallback to legacy mapping
        if !self.hotbar.is_empty() {
            let keys = [
                KeyboardKey::KEY_ONE,
                KeyboardKey::KEY_TWO,
                KeyboardKey::KEY_THREE,
                KeyboardKey::KEY_FOUR,
                KeyboardKey::KEY_FIVE,
                KeyboardKey::KEY_SIX,
                KeyboardKey::KEY_SEVEN,
                KeyboardKey::KEY_EIGHT,
                KeyboardKey::KEY_NINE,
            ];
            for (i, key) in keys.iter().enumerate() {
                if i < self.hotbar.len() && rl.is_key_pressed(*key) {
                    self.queue.emit_now(Event::PlaceTypeSelected {
                        block: self.hotbar[i],
                    });
                }
            }
        } else {
            let id_of = |name: &str| self.reg.id_by_name(name).unwrap_or(0);
            if rl.is_key_pressed(KeyboardKey::KEY_ONE) {
                self.queue.emit_now(Event::PlaceTypeSelected {
                    block: Block {
                        id: id_of("dirt"),
                        state: 0,
                    },
                });
            }
            if rl.is_key_pressed(KeyboardKey::KEY_TWO) {
                self.queue.emit_now(Event::PlaceTypeSelected {
                    block: Block {
                        id: id_of("stone"),
                        state: 0,
                    },
                });
            }
            if rl.is_key_pressed(KeyboardKey::KEY_THREE) {
                self.queue.emit_now(Event::PlaceTypeSelected {
                    block: Block {
                        id: id_of("sand"),
                        state: 0,
                    },
                });
            }
            if rl.is_key_pressed(KeyboardKey::KEY_FOUR) {
                self.queue.emit_now(Event::PlaceTypeSelected {
                    block: Block {
                        id: id_of("grass"),
                        state: 0,
                    },
                });
            }
            if rl.is_key_pressed(KeyboardKey::KEY_FIVE) {
                self.queue.emit_now(Event::PlaceTypeSelected {
                    block: Block {
                        id: id_of("snow"),
                        state: 0,
                    },
                });
            }
            if rl.is_key_pressed(KeyboardKey::KEY_SIX) {
                self.queue.emit_now(Event::PlaceTypeSelected {
                    block: Block {
                        id: id_of("glowstone"),
                        state: 0,
                    },
                });
            }
            if rl.is_key_pressed(KeyboardKey::KEY_SEVEN) {
                self.queue.emit_now(Event::PlaceTypeSelected {
                    block: Block {
                        id: id_of("beacon"),
                        state: 0,
                    },
                });
            }
        }

        // Structure speed controls (horizontal X)
        if rl.is_key_pressed(KeyboardKey::KEY_MINUS) {
            self.gs.structure_speed = (self.gs.structure_speed - 1.0).max(0.0);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_EQUAL) {
            self.gs.structure_speed = (self.gs.structure_speed + 1.0).min(64.0);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_ZERO) {
            self.gs.structure_speed = 0.0;
        }

        // Structure elevation controls (vertical Y)
        if rl.is_key_pressed(KeyboardKey::KEY_LEFT_BRACKET) {
            self.gs.structure_elev_speed = (self.gs.structure_elev_speed - 1.0).max(-64.0);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_RIGHT_BRACKET) {
            self.gs.structure_elev_speed = (self.gs.structure_elev_speed + 1.0).min(64.0);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_BACKSLASH) {
            self.gs.structure_elev_speed = 0.0;
        }

        // Light emitters via hotkeys
        if rl.is_key_pressed(KeyboardKey::KEY_L) {
            let fwd = self.cam.forward();
            let p = self.cam.position + fwd * 4.0;
            let wx = p.x.floor() as i32;
            let wy = p.y.floor() as i32;
            let wz = p.z.floor() as i32;
            self.queue.emit_now(Event::LightEmitterAdded {
                wx,
                wy,
                wz,
                level: 255,
                is_beacon: false,
            });
        }
        if rl.is_key_pressed(KeyboardKey::KEY_K) {
            let fwd = self.cam.forward();
            let p = self.cam.position + fwd * 4.0;
            let wx = p.x.floor() as i32;
            let wy = p.y.floor() as i32;
            let wz = p.z.floor() as i32;
            self.queue
                .emit_now(Event::LightEmitterRemoved { wx, wy, wz });
        }

        // Mouse edit intents
        let want_edit = rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT)
            || rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT);
        if want_edit {
            let place = rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT);
            let block = self.gs.place_type;
            self.queue
                .emit_now(Event::RaycastEditRequested { place, block });
        }

        // Update structure poses: translate along +X and vertical Y with adjustable speeds
        let step_dx = self.gs.structure_speed * dt.max(0.0);
        let step_dy = self.gs.structure_elev_speed * dt.max(0.0);
        for (id, st) in self.gs.structures.iter() {
            let prev = st.pose.pos;
            let newp = Vec3 {
                x: prev.x + step_dx,
                y: prev.y + step_dy,
                z: prev.z,
            };
            let delta = Vector3::new(newp.x - prev.x, newp.y - prev.y, newp.z - prev.z);
            // Keep yaw fixed so collisions match visuals
            let yaw = 0.0_f32;
            self.queue.emit_now(Event::StructurePoseUpdated {
                id: *id,
                pos: vec3_to_rl(newp),
                yaw_deg: yaw,
                delta,
            });
        }

        // Movement intent for this tick (dt→ms)
        let dt_ms = (dt.max(0.0) * 1000.0) as u32;
        self.queue.emit_now(Event::MovementRequested {
            dt_ms,
            yaw: self.cam.yaw,
            walk_mode: self.gs.walk_mode,
        });

        // Drain worker results, sort deterministically by job_id, and emit completion events for this tick
        let mut results: Vec<JobOut> = self.runtime.drain_worker_results();
        results.sort_by_key(|r| r.job_id);
        for r in results {
            self.queue.emit_now(Event::BuildChunkJobCompleted {
                cx: r.cx,
                cz: r.cz,
                rev: r.rev,
                cpu: r.cpu,
                buf: r.buf,
                light_borders: r.light_borders,
                job_id: r.job_id,
            });
        }

        // Drain structure worker results
        for r in self.runtime.drain_structure_results() {
            self.queue.emit_now(Event::StructureBuildCompleted {
                id: r.id,
                rev: r.rev,
                cpu: r.cpu,
            });
        }

        // Snapshot queued events before processing (for debug overlay)
        {
            let (total, by) = self.queue.queued_counts();
            self.debug_stats.queued_events_total = total;
            // Sort for stable presentation
            let mut pairs: Vec<(String, usize)> =
                by.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
            pairs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            self.debug_stats.queued_events_by = pairs;
        }

        // Process events scheduled for this tick with a budget
        let mut processed = 0usize;
        let max_events = 20_000usize;
        let label_of = |ev: &Event| -> &'static str {
            match ev {
                Event::Tick => "Tick",
                Event::WalkModeToggled => "WalkModeToggled",
                Event::GridToggled => "GridToggled",
                Event::WireframeToggled => "WireframeToggled",
                Event::ChunkBoundsToggled => "ChunkBoundsToggled",
                Event::FrustumCullingToggled => "FrustumCullingToggled",
                Event::BiomeLabelToggled => "BiomeLabelToggled",
                Event::PlaceTypeSelected { .. } => "PlaceTypeSelected",
                Event::MovementRequested { .. } => "MovementRequested",
                Event::RaycastEditRequested { .. } => "RaycastEditRequested",
                Event::BlockPlaced { .. } => "BlockPlaced",
                Event::BlockRemoved { .. } => "BlockRemoved",
                Event::ViewCenterChanged { .. } => "ViewCenterChanged",
                Event::EnsureChunkLoaded { .. } => "EnsureChunkLoaded",
                Event::EnsureChunkUnloaded { .. } => "EnsureChunkUnloaded",
                Event::ChunkRebuildRequested { .. } => "ChunkRebuildRequested",
                Event::BuildChunkJobRequested { .. } => "BuildChunkJobRequested",
                Event::BuildChunkJobCompleted { .. } => "BuildChunkJobCompleted",
                Event::StructureBuildRequested { .. } => "StructureBuildRequested",
                Event::StructureBuildCompleted { .. } => "StructureBuildCompleted",
                Event::StructurePoseUpdated { .. } => "StructurePoseUpdated",
                Event::StructureBlockPlaced { .. } => "StructureBlockPlaced",
                Event::StructureBlockRemoved { .. } => "StructureBlockRemoved",
                Event::PlayerAttachedToStructure { .. } => "PlayerAttachedToStructure",
                Event::PlayerDetachedFromStructure { .. } => "PlayerDetachedFromStructure",
                Event::LightEmitterAdded { .. } => "LightEmitterAdded",
                Event::LightEmitterRemoved { .. } => "LightEmitterRemoved",
                Event::LightBordersUpdated { .. } => "LightBordersUpdated",
            }
        };
        while let Some(env) = self.queue.pop_ready() {
            // Tally processed stats (session-wide)
            let label = label_of(&env.kind).to_string();
            self.evt_processed_total = self.evt_processed_total.saturating_add(1);
            *self.evt_processed_by.entry(label).or_insert(0) += 1;
            self.handle_event(rl, thread, env);
            processed += 1;
            if processed >= max_events {
                break;
            }
        }
        // After handling events for this tick, flush prioritized intents.
        self.flush_intents();
        // Snapshot current intents backlog for debug overlay
        self.debug_stats.intents_size = self.intents.len();
        self.gs.tick = self.gs.tick.wrapping_add(1);
        self.queue.advance_tick();
        // Sanity check: events left in past ticks will never be processed; warn if detected
        let stale = self.queue.count_stale_events();
        if stale > 0 {
            let mut details = String::new();
            for (t, n) in self.queue.stale_summary() {
                use std::fmt::Write as _;
                let _ = write!(&mut details, "[t={} n={}] ", t, n);
            }
            log::error!(
                target: "events",
                "Detected {} stale event(s) in past tick buckets; details: {}",
                stale,
                details
            );
        }
    }

    pub fn render(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread) {
        // Preserve queued-events snapshot captured during step() before processing,
        // then reset per-frame stats for rendering accumulation.
        let prev_q_total = self.debug_stats.queued_events_total;
        let prev_q_by = self.debug_stats.queued_events_by.clone();
        let prev_intents = self.debug_stats.intents_size;
        self.debug_stats = DebugStats::default();
        self.debug_stats.queued_events_total = prev_q_total;
        self.debug_stats.queued_events_by = prev_q_by;
        self.debug_stats.intents_size = prev_intents;

        // Calculate frustum for culling
        let screen_width = rl.get_screen_width() as f32;
        let screen_height = rl.get_screen_height() as f32;
        let aspect_ratio = screen_width / screen_height;
        let frustum = self.cam.calculate_frustum(aspect_ratio, 0.1, 10000.0); // Increased far plane

        let camera3d = self.cam.to_camera3d();
        let mut d = rl.begin_drawing(thread);
        d.clear_background(Color::new(210, 221, 235, 255));
        // Ensure the depth buffer is cleared every frame to avoid ghost silhouettes when moving
        unsafe {
            raylib::ffi::rlClearScreenBuffers();
        }
        {
            let mut d3 = d.begin_mode3D(camera3d);
            if self.gs.show_grid {
                d3.draw_grid(64, 1.0);
            }

            // Update shader uniforms
            let surface_fog = [210.0 / 255.0, 221.0 / 255.0, 235.0 / 255.0];
            let cave_fog = [0.0, 0.0, 0.0];
            let world_h = self.gs.world.world_size_y() as f32;
            let underground_thr = 0.30_f32 * world_h;
            let underground = self.cam.position.y < underground_thr;
            let fog_color = if underground { cave_fog } else { surface_fog };
            // Diminish fog: push start/end farther out
            let fog_start = 64.0f32;
            let fog_end = 512.0f32 * 0.9;
            if let Some(ref mut ls) = self.leaves_shader {
                ls.update_frame_uniforms(self.cam.position, fog_color, fog_start, fog_end);
            }
            if let Some(ref mut fs) = self.fog_shader {
                fs.update_frame_uniforms(self.cam.position, fog_color, fog_start, fog_end);
            }

            for cr in self.renders.values() {
                // Check if chunk is within frustum
                if self.gs.frustum_culling_enabled && !frustum.contains_bounding_box(&cr.bbox) {
                    self.debug_stats.chunks_culled += 1;
                    continue;
                }

                self.debug_stats.chunks_rendered += 1;
                // Set biome-based leaf palette per chunk if available
                if let Some(ref mut ls) = self.leaves_shader {
                    if let Some(t) = cr.leaf_tint {
                        let p0 = t;
                        let p1 = [t[0] * 0.85, t[1] * 0.85, t[2] * 0.85];
                        let p2 = [t[0] * 0.7, t[1] * 0.7, t[2] * 0.7];
                        let p3 = [t[0] * 0.5, t[1] * 0.5, t[2] * 0.5];
                        ls.set_autumn_palette(p0, p1, p2, p3, 1.0);
                    } else {
                        // Default greenish palette
                        ls.set_autumn_palette(
                            [0.32, 0.55, 0.25],
                            [0.28, 0.48, 0.22],
                            [0.20, 0.40, 0.18],
                            [0.12, 0.28, 0.10],
                            1.0,
                        );
                    }
                }
                for (_fm, model) in &cr.parts {
                    // Get mesh stats from the model
                    unsafe {
                        let mesh = &*model.meshes;
                        self.debug_stats.total_vertices += mesh.vertexCount as usize;
                        self.debug_stats.total_triangles += mesh.triangleCount as usize;
                    }
                    self.debug_stats.draw_calls += 1;

                    if self.gs.wireframe {
                        d3.draw_model_wires(model, Vector3::zero(), 1.0, Color::WHITE);
                    } else {
                        d3.draw_model(model, Vector3::zero(), 1.0, Color::WHITE);
                    }
                }
            }

            // Draw structures with transform (translation + yaw)
            for (id, cr) in &self.structure_renders {
                if let Some(st) = self.gs.structures.get(id) {
                    // Translate bounding box to structure position for frustum check
                    let translated_bbox = raylib::core::math::BoundingBox {
                        min: cr.bbox.min + vec3_to_rl(st.pose.pos),
                        max: cr.bbox.max + vec3_to_rl(st.pose.pos),
                    };

                    // Check if structure is within frustum
                    if self.gs.frustum_culling_enabled
                        && !frustum.contains_bounding_box(&translated_bbox)
                    {
                        self.debug_stats.structures_culled += 1;
                        continue;
                    }

                    self.debug_stats.structures_rendered += 1;
                    for (_fm, model) in &cr.parts {
                        // Get mesh stats from the model
                        unsafe {
                            let mesh = &*model.meshes;
                            self.debug_stats.total_vertices += mesh.vertexCount as usize;
                            self.debug_stats.total_triangles += mesh.triangleCount as usize;
                        }
                        self.debug_stats.draw_calls += 1;

                        // Yaw is ignored here if draw_model_ex isn't available; translation still applies
                        d3.draw_model(model, vec3_to_rl(st.pose.pos), 1.0, Color::WHITE);
                    }
                }
            }

            // Raycast highlight: show where a placed block would go (world only for now)
            // Sample order: edits > loaded chunk buffers > world
            let org = self.cam.position;
            let dir = self.cam.forward();
            let sx = self.gs.world.chunk_size_x as i32;
            let sy = self.gs.world.chunk_size_y as i32;
            let sz = self.gs.world.chunk_size_z as i32;
            let sampler = |wx: i32, wy: i32, wz: i32| -> Block {
                if let Some(b) = self.gs.edits.get(wx, wy, wz) {
                    return b;
                }
                let cx = wx.div_euclid(sx);
                let cz = wz.div_euclid(sz);
                if let Some(cent) = self.gs.chunks.get(&(cx, cz)) {
                    if let Some(ref buf) = cent.buf {
                        return buf.get_world(wx, wy, wz).unwrap_or(Block::AIR);
                    }
                }
                self.gs.world.block_at_runtime(&self.reg, wx, wy, wz)
            };
            let is_solid = |wx: i32, wy: i32, wz: i32| -> bool {
                let b = sampler(wx, wy, wz);
                self.reg
                    .get(b.id)
                    .map(|ty| ty.is_solid(b.state))
                    .unwrap_or(false)
            };
            if let Some(hit) = raycast::raycast_first_hit_with_face(org, dir, 5.0, is_solid) {
                // Outline only the struck face of the solid block (bx,by,bz)
                let (bx, by, bz) = (hit.bx, hit.by, hit.bz);
                if by >= 0 && by < sy {
                    let (x0, y0, z0) = (bx as f32, by as f32, bz as f32);
                    let (x1, y1, z1) = (x0 + 1.0, y0 + 1.0, z0 + 1.0);
                    let eps = 0.002f32;
                    if hit.nx != 0 {
                        let xf = if hit.nx > 0 { x1 } else { x0 } + (hit.nx as f32) * eps;
                        let p1 = Vector3::new(xf, y0, z0);
                        let p2 = Vector3::new(xf, y1, z0);
                        let p3 = Vector3::new(xf, y1, z1);
                        let p4 = Vector3::new(xf, y0, z1);
                        d3.draw_line_3D(p1, p2, Color::YELLOW);
                        d3.draw_line_3D(p2, p3, Color::YELLOW);
                        d3.draw_line_3D(p3, p4, Color::YELLOW);
                        d3.draw_line_3D(p4, p1, Color::YELLOW);
                    } else if hit.ny != 0 {
                        let yf = if hit.ny > 0 { y1 } else { y0 } + (hit.ny as f32) * eps;
                        let p1 = Vector3::new(x0, yf, z0);
                        let p2 = Vector3::new(x1, yf, z0);
                        let p3 = Vector3::new(x1, yf, z1);
                        let p4 = Vector3::new(x0, yf, z1);
                        d3.draw_line_3D(p1, p2, Color::YELLOW);
                        d3.draw_line_3D(p2, p3, Color::YELLOW);
                        d3.draw_line_3D(p3, p4, Color::YELLOW);
                        d3.draw_line_3D(p4, p1, Color::YELLOW);
                    } else if hit.nz != 0 {
                        let zf = if hit.nz > 0 { z1 } else { z0 } + (hit.nz as f32) * eps;
                        let p1 = Vector3::new(x0, y0, zf);
                        let p2 = Vector3::new(x1, y0, zf);
                        let p3 = Vector3::new(x1, y1, zf);
                        let p4 = Vector3::new(x0, y1, zf);
                        d3.draw_line_3D(p1, p2, Color::YELLOW);
                        d3.draw_line_3D(p2, p3, Color::YELLOW);
                        d3.draw_line_3D(p3, p4, Color::YELLOW);
                        d3.draw_line_3D(p4, p1, Color::YELLOW);
                    }
                }
            }

            if self.gs.show_chunk_bounds {
                let col = Color::new(255, 64, 32, 200);
                for cr in self.renders.values() {
                    let min = cr.bbox.min;
                    let max = cr.bbox.max;
                    let center = Vector3::new(
                        (min.x + max.x) * 0.5,
                        (min.y + max.y) * 0.5,
                        (min.z + max.z) * 0.5,
                    );
                    let size = Vector3::new(
                        (max.x - min.x).abs(),
                        (max.y - min.y).abs(),
                        (max.z - min.z).abs(),
                    );
                    d3.draw_cube_wires(center, size.x, size.y, size.z, col);
                }
            }
        }

        // Showcase labels: draw block (or variant) names above each showcased block
        if matches!(self.gs.world.mode, WorldGenMode::Showcase) {
            // Snapshot params
            let params = { self.gs.world.gen_params.read().map(|g| g.clone()).ok() };
            if let Some(p) = params {
                // Compute showcase row Y and Z
                let mut row_y = (self.gs.world.chunk_size_y as f32 * p.platform_y_ratio
                    + p.platform_y_offset)
                    .round() as i32;
                row_y = row_y.clamp(1, self.gs.world.chunk_size_y as i32 - 2);
                let cz = (self.gs.world.world_size_z() as i32) / 2;
                // Build showcase entries (mirrors worldgen layout)
                let entries = geist_world::voxel::build_showcase_entries(&self.reg);
                if !entries.is_empty() {
                    let spacing = 2i32; // air gap of 1 block between entries
                    let row_len = (entries.len() as i32) * spacing - 1;
                    let cx = (self.gs.world.world_size_x() as i32) / 2;
                    let start_x = cx - row_len / 2;
                    // Draw each label
                    let font_size = 16;
                    for (i, e) in entries.iter().enumerate() {
                        let bx = start_x + (i as i32) * spacing;
                        if bx < 0 || bx >= self.gs.world.world_size_x() as i32 {
                            continue;
                        }
                        let pos3 =
                            Vector3::new(bx as f32 + 0.5, row_y as f32 + 1.25, cz as f32 + 0.5);
                        // Project to screen and draw text centered
                        let sp = d.get_world_to_screen(pos3, camera3d);
                        let text = e.label.as_str();
                        let w = d.measure_text(text, font_size);
                        let x = (sp.x as i32) - (w / 2);
                        let y = (sp.y as i32) - (font_size + 2);
                        // Shadow + main for readability
                        d.draw_text(text, x + 1, y + 1, font_size, Color::BLACK);
                        d.draw_text(text, x, y, font_size, Color::WHITE);
                    }
                }

                // Stairs cluster labels (adjacency scenarios)
                let stair_base_z = cz + 3; // matches worldgen placement
                let placements = geist_world::voxel::build_showcase_stairs_cluster(&self.reg);
                if !placements.is_empty() {
                    let max_dx = placements.iter().map(|p| p.dx).max().unwrap_or(0);
                    let cluster_w = max_dx + 1;
                    let cx = (self.gs.world.world_size_x() as i32) / 2;
                    let start_x = cx - cluster_w / 2;
                    let font_size = 14;
                    for p in &placements {
                        let bx = start_x + p.dx;
                        let bz = stair_base_z + p.dz;
                        if bx < 0
                            || bx >= self.gs.world.world_size_x() as i32
                            || bz < 0
                            || bz >= self.gs.world.world_size_z() as i32
                        {
                            continue;
                        }
                        let pos3 =
                            Vector3::new(bx as f32 + 0.5, row_y as f32 + 1.25, bz as f32 + 0.5);
                        let sp = d.get_world_to_screen(pos3, camera3d);
                        let text = p.label.as_str();
                        let w = d.measure_text(text, font_size);
                        let x = (sp.x as i32) - (w / 2);
                        let y = (sp.y as i32) - (font_size + 2);
                        d.draw_text(text, x + 1, y + 1, font_size, Color::BLACK);
                        d.draw_text(text, x, y, font_size, Color::WHITE);
                    }
                }
            }
        }

        // Debug overlay (lower left)
        let fps = d.get_fps();
        let mut debug_text = format!(
            "FPS: {}\nVertices: {}\nTriangles: {}\nChunks: {} (culled: {})\nStructures: {} (culled: {})\nDraw Calls: {}",
            fps,
            self.debug_stats.total_vertices,
            self.debug_stats.total_triangles,
            self.debug_stats.chunks_rendered,
            self.debug_stats.chunks_culled,
            self.debug_stats.structures_rendered,
            self.debug_stats.structures_culled,
            self.debug_stats.draw_calls
        );
        let mut text_lines = 6; // Base number of lines in debug text
        if self.gs.show_biome_label {
            let wx = self.cam.position.x.floor() as i32;
            let wz = self.cam.position.z.floor() as i32;
            if let Some(biome) = self.gs.world.biome_at(wx, wz) {
                debug_text.push_str(&format!("\nBiome: {}", biome.name));
                text_lines += 1;
            }
        }
        // (moved event stats to right-side overlay)
        let screen_height = d.get_screen_height();
        let line_height = 22; // Approximate height per line with font size 20
        let y_pos = screen_height - (text_lines * line_height) - 10; // 10px margin from bottom
        d.draw_text(&debug_text, 10, y_pos, 20, Color::WHITE);
        d.draw_text(&debug_text, 11, y_pos + 1, 20, Color::BLACK); // Shadow for readability

        // Right-side event overlay
        let mut right_text = format!("Queued Events: {}", self.debug_stats.queued_events_total);
        if self.debug_stats.queued_events_total > 0 {
            let max_show = 10usize;
            let shown = self.debug_stats.queued_events_by.len().min(max_show);
            for (k, v) in self.debug_stats.queued_events_by.iter().take(shown) {
                right_text.push_str(&format!("\n  {}: {}", k, v));
            }
            if self.debug_stats.queued_events_by.len() > shown {
                right_text.push_str("\n  …");
            }
        }
        right_text.push_str(&format!(
            "\nProcessed Events (session): {}",
            self.evt_processed_total
        ));
        right_text.push_str(&format!("\nIntents: {}", self.debug_stats.intents_size));
        if self.evt_processed_total > 0 {
            let max_show = 10usize;
            // Build a sorted list by count desc
            let mut pairs: Vec<(&str, usize)> = self
                .evt_processed_by
                .iter()
                .map(|(k, v)| (k.as_str(), *v))
                .collect();
            pairs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let shown = pairs.len().min(max_show);
            for (k, v) in pairs.into_iter().take(shown) {
                right_text.push_str(&format!("\n  {}: {}", k, v));
            }
            if self.evt_processed_by.len() > shown {
                right_text.push_str("\n  …");
            }
        }
        // Runtime queue debug (vertical layout)
        let (q_e, if_e, q_l, if_l, q_b, if_b) = self.runtime.queue_debug_counts();
        right_text.push_str("\nRuntime Queues:");
        right_text.push_str(&format!("\n  Edit  - q={} inflight={}", q_e, if_e));
        right_text.push_str(&format!("\n  Light - q={} inflight={}", q_l, if_l));
        right_text.push_str(&format!("\n  BG    - q={} inflight={}", q_b, if_b));

        let screen_width = d.get_screen_width();
        let font_size = 20;
        // Fixed panel width: assume up to 1,000,000 counts and widest queue line
        let panel_templates = [
            "Queued Events: 1,000,000",
            "Processed Events (session): 1,000,000",
            "Runtime Queues:",
            "  Edit  - q=1,000,000 inflight=1,000,000",
            "  Light - q=1,000,000 inflight=1,000,000",
            "  BG    - q=1,000,000 inflight=1,000,000",
        ];
        let mut panel_w = 0;
        for t in panel_templates.iter() {
            let w = d.measure_text(t, font_size);
            if w > panel_w {
                panel_w = w;
            }
        }
        // Small padding so text doesn't hug the edge
        panel_w += 8;
        let margin = 10;
        let rx = screen_width - panel_w - margin;
        // Align bottom similar to left overlay
        let lines = right_text.split('\n').count();
        let ry = screen_height - (lines as i32 * line_height) - 10;
        d.draw_text(&right_text, rx, ry, font_size, Color::WHITE);
        d.draw_text(&right_text, rx + 1, ry + 1, font_size, Color::BLACK);

        // HUD
        let hud_mode = if self.gs.walk_mode { "Walk" } else { "Fly" };
        let hud = format!(
            "{}: Tab capture, WASD{} move{}, V toggle mode, F wireframe, G grid, B bounds, C culling, H biome label, L add light, K remove light | Place: {:?} (1-7) | Castle vX={:.1} (-/= adj, 0 stop) vY={:.1} ([/] adj, \\ stop)",
            hud_mode,
            if self.gs.walk_mode { "" } else { "+QE" },
            if self.gs.walk_mode {
                ", Space jump, Shift run"
            } else {
                ""
            },
            self.gs.place_type,
            self.gs.structure_speed,
            self.gs.structure_elev_speed,
        );
        d.draw_text(&hud, 12, 12, 18, Color::DARKGRAY);
        d.draw_fps(12, 36);

        // Biome label moved to debug overlay above

        // Debug overlay for attachment status
        let mut debug_y = 60;
        d.draw_text("=== ATTACHMENT DEBUG ===", 12, debug_y, 16, Color::RED);
        debug_y += 20;

        // Show attachment status
        if let Some(att) = self.gs.ground_attach {
            d.draw_text(
                &format!("ATTACHED to structure ID: {}", att.id),
                12,
                debug_y,
                16,
                Color::GREEN,
            );
            debug_y += 18;
            d.draw_text(
                &format!("  Grace period: {}", att.grace),
                12,
                debug_y,
                16,
                Color::GREEN,
            );
            debug_y += 18;
            d.draw_text(
                &format!(
                    "  Local offset: ({:.2}, {:.2}, {:.2})",
                    att.local_offset.x, att.local_offset.y, att.local_offset.z
                ),
                12,
                debug_y,
                16,
                Color::GREEN,
            );
            debug_y += 18;
        } else {
            d.draw_text("NOT ATTACHED", 12, debug_y, 16, Color::ORANGE);
            debug_y += 18;
        }

        // Show walker position
        d.draw_text(
            &format!(
                "Walker pos: ({:.2}, {:.2}, {:.2})",
                self.gs.walker.pos.x, self.gs.walker.pos.y, self.gs.walker.pos.z
            ),
            12,
            debug_y,
            16,
            Color::DARKGRAY,
        );
        debug_y += 18;

        // Show on_ground status
        d.draw_text(
            &format!("On ground: {}", self.gs.walker.on_ground),
            12,
            debug_y,
            16,
            Color::DARKGRAY,
        );
        debug_y += 18;

        // Check each structure and show detection status
        for (id, st) in &self.gs.structures {
            let on_structure = self.is_feet_on_structure(st, self.gs.walker.pos);
            let color = if on_structure {
                Color::GREEN
            } else {
                Color::GRAY
            };
            d.draw_text(
                &format!(
                    "Structure {}: on={} pos=({:.1},{:.1},{:.1}) delta=({:.3},{:.3},{:.3})",
                    id,
                    on_structure,
                    st.pose.pos.x,
                    st.pose.pos.y,
                    st.pose.pos.z,
                    st.last_delta.x,
                    st.last_delta.y,
                    st.last_delta.z
                ),
                12,
                debug_y,
                16,
                color,
            );
            debug_y += 18;

            // Show detailed detection info
            let p = vec3_from_rl(self.gs.walker.pos);
            let diff = Vec3 {
                x: p.x - st.pose.pos.x,
                y: p.y - st.pose.pos.y,
                z: p.z - st.pose.pos.z,
            };
            let local = rotate_yaw_inv(diff, st.pose.yaw_deg);
            let test_y = local.y - 0.08;
            let lx = local.x.floor() as i32;
            let ly = test_y.floor() as i32;
            let lz = local.z.floor() as i32;

            d.draw_text(
                &format!(
                    "  Local: ({:.2}, {:.2}, {:.2}) Test Y: {:.2} -> Grid: ({}, {}, {})",
                    local.x, local.y, local.z, test_y, lx, ly, lz
                ),
                12,
                debug_y,
                14,
                color,
            );
            debug_y += 16;

            // Check if we're in bounds
            let in_bounds = lx >= 0
                && ly >= 0
                && lz >= 0
                && (lx as usize) < st.sx
                && (ly as usize) < st.sy
                && (lz as usize) < st.sz;

            // Get the actual block at this position (direct sample)
            let (block_at_pos, block_solid) = if in_bounds {
                // Check edits first
                if let Some(b) = st.edits.get(lx, ly, lz) {
                    (
                        format!("id:{} state:{} (edit)", b.id, b.state),
                        self.reg
                            .get(b.id)
                            .map(|ty| ty.is_solid(b.state))
                            .unwrap_or(false),
                    )
                } else {
                    // Check base blocks
                    let idx = st.idx(lx as usize, ly as usize, lz as usize);
                    let b = st.blocks[idx];
                    (
                        format!("id:{} state:{}", b.id, b.state),
                        self.reg
                            .get(b.id)
                            .map(|ty| ty.is_solid(b.state))
                            .unwrap_or(false),
                    )
                }
            } else {
                ("out of bounds".to_string(), false)
            };

            d.draw_text(
                &format!(
                    "  Bounds: 0..{} x 0..{} x 0..{} | In bounds: {}",
                    st.sx, st.sy, st.sz, in_bounds
                ),
                12,
                debug_y,
                14,
                color,
            );
            debug_y += 16;

            d.draw_text(
                &format!(
                    "  Block at ({},{},{}): {} | Solid: {}",
                    lx, ly, lz, block_at_pos, block_solid
                ),
                12,
                debug_y,
                14,
                color,
            );
            debug_y += 16;

            // Also show the block one cell below the sample (helps diagnose edge cases)
            if ly > 0 {
                let by = ly - 1;
                let (block_below, solid_below) = if lx >= 0
                    && by >= 0
                    && lz >= 0
                    && (lx as usize) < st.sx
                    && (by as usize) < st.sy
                    && (lz as usize) < st.sz
                {
                    if let Some(b) = st.edits.get(lx, by, lz) {
                        (
                            format!("id:{} state:{} (edit)", b.id, b.state),
                            self.reg
                                .get(b.id)
                                .map(|ty| ty.is_solid(b.state))
                                .unwrap_or(false),
                        )
                    } else {
                        let idx = st.idx(lx as usize, by as usize, lz as usize);
                        let b = st.blocks[idx];
                        (
                            format!("id:{} state:{}", b.id, b.state),
                            self.reg
                                .get(b.id)
                                .map(|ty| ty.is_solid(b.state))
                                .unwrap_or(false),
                        )
                    }
                } else {
                    ("out of bounds".to_string(), false)
                };
                d.draw_text(
                    &format!(
                        "  Block at below ({},{},{}): {} | Solid: {}",
                        lx, by, lz, block_below, solid_below
                    ),
                    12,
                    debug_y,
                    14,
                    color,
                );
                debug_y += 16;
            }

            // Show deck info and check what's at deck level
            let deck_y = (st.sy as f32 * 0.33) as i32;
            d.draw_text(
                &format!("  Deck Y level: {} (expecting solid blocks here)", deck_y),
                12,
                debug_y,
                14,
                Color::BLUE,
            );
            debug_y += 16;

            // Debug: Check what's actually at the deck level at player's X,Z
            if lx >= 0 && lz >= 0 && (lx as usize) < st.sx && (lz as usize) < st.sz {
                let deck_idx = st.idx(lx as usize, deck_y as usize, lz as usize);
                let deck_block = st.blocks[deck_idx];
                d.draw_text(
                    &format!(
                        "  Block at deck level ({},{},{}): {:?}",
                        lx, deck_y, lz, deck_block
                    ),
                    12,
                    debug_y,
                    14,
                    Color::MAGENTA,
                );
                debug_y += 16;
            }
        }
    }
}

impl App {
    fn log_event(tick: u64, ev: &crate::event::Event) {
        use crate::event::Event as E;
        match ev {
            E::Tick => {
                log::trace!(target: "events", "[tick {}] Tick", tick);
            }
            E::WalkModeToggled => {
                log::info!(target: "events", "[tick {}] WalkModeToggled", tick);
            }
            E::GridToggled => {
                log::info!(target: "events", "[tick {}] GridToggled", tick);
            }
            E::WireframeToggled => {
                log::info!(target: "events", "[tick {}] WireframeToggled", tick);
            }
            E::ChunkBoundsToggled => {
                log::info!(target: "events", "[tick {}] ChunkBoundsToggled", tick);
            }
            E::FrustumCullingToggled => {
                log::info!(target: "events", "[tick {}] FrustumCullingToggled", tick);
            }
            E::BiomeLabelToggled => {
                log::info!(target: "events", "[tick {}] BiomeLabelToggled", tick);
            }
            E::PlaceTypeSelected { block } => {
                log::info!(target: "events", "[tick {}] PlaceTypeSelected block={:?}", tick, block);
            }
            E::MovementRequested {
                dt_ms,
                yaw,
                walk_mode,
            } => {
                log::trace!(target: "events", "[tick {}] MovementRequested dt_ms={} yaw={:.1} mode={}",
                    tick, dt_ms, yaw, if *walk_mode {"walk"} else {"fly"});
            }
            E::RaycastEditRequested { place, block } => {
                log::info!(target: "events", "[tick {}] RaycastEditRequested {} block={:?}",
                    tick, if *place {"place"} else {"remove"}, block);
            }
            E::BlockPlaced { wx, wy, wz, block } => {
                log::info!(target: "events", "[tick {}] BlockPlaced ({},{},{}) block={:?}", tick, wx, wy, wz, block);
            }
            E::BlockRemoved { wx, wy, wz } => {
                log::info!(target: "events", "[tick {}] BlockRemoved ({},{},{})", tick, wx, wy, wz);
            }
            E::ViewCenterChanged { ccx, ccz } => {
                log::info!(target: "events", "[tick {}] ViewCenterChanged cc=({}, {})", tick, ccx, ccz);
            }
            E::EnsureChunkLoaded { cx, cz } => {
                log::info!(target: "events", "[tick {}] EnsureChunkLoaded ({}, {})", tick, cx, cz);
            }
            E::EnsureChunkUnloaded { cx, cz } => {
                log::info!(target: "events", "[tick {}] EnsureChunkUnloaded ({}, {})", tick, cx, cz);
            }
            E::ChunkRebuildRequested { cx, cz, cause } => {
                log::debug!(target: "events", "[tick {}] ChunkRebuildRequested ({}, {}) cause={:?}", tick, cx, cz, cause);
            }
            E::BuildChunkJobRequested {
                cx,
                cz,
                neighbors,
                rev,
                job_id,
                cause,
            } => {
                let mask = [
                    neighbors.neg_x,
                    neighbors.pos_x,
                    neighbors.neg_z,
                    neighbors.pos_z,
                ];
                log::debug!(target: "events", "[tick {}] BuildChunkJobRequested ({}, {}) rev={} cause={:?} nmask={:?} job_id={:#x}",
                    tick, cx, cz, rev, cause, mask, job_id);
            }
            E::BuildChunkJobCompleted {
                cx,
                cz,
                rev,
                job_id,
                ..
            } => {
                log::debug!(target: "events", "[tick {}] BuildChunkJobCompleted ({}, {}) rev={} job_id={:#x}",
                    tick, cx, cz, rev, job_id);
            }
            E::StructureBuildRequested { id, rev } => {
                log::info!(target: "events", "[tick {}] StructureBuildRequested id={} rev={}", tick, id, rev);
            }
            E::StructureBuildCompleted { id, rev, .. } => {
                log::info!(target: "events", "[tick {}] StructureBuildCompleted id={} rev={}", tick, id, rev);
            }
            E::StructurePoseUpdated {
                id,
                pos,
                yaw_deg,
                delta,
            } => {
                log::trace!(target: "events", "[tick {}] StructurePoseUpdated id={} pos=({:.2},{:.2},{:.2}) yaw={:.1} delta=({:.2},{:.2},{:.2})",
                    tick, id, pos.x, pos.y, pos.z, yaw_deg, delta.x, delta.y, delta.z);
            }
            E::StructureBlockPlaced {
                id,
                lx,
                ly,
                lz,
                block,
            } => {
                log::info!(target: "events", "[tick {}] StructureBlockPlaced id={} ({},{},{}) block={:?}", tick, id, lx, ly, lz, block);
            }
            E::StructureBlockRemoved { id, lx, ly, lz } => {
                log::info!(target: "events", "[tick {}] StructureBlockRemoved id={} ({},{},{})", tick, id, lx, ly, lz);
            }
            E::PlayerAttachedToStructure { id, local_offset } => {
                log::info!(target: "events", "[tick {}] PlayerAttachedToStructure id={} local=({:.2},{:.2},{:.2})",
                    tick, id, local_offset.x, local_offset.y, local_offset.z);
            }
            E::PlayerDetachedFromStructure { id } => {
                log::info!(target: "events", "[tick {}] PlayerDetachedFromStructure id={}", tick, id);
            }
            E::LightEmitterAdded {
                wx,
                wy,
                wz,
                level,
                is_beacon,
            } => {
                log::info!(target: "events", "[tick {}] LightEmitterAdded ({},{},{}) level={} beacon={}",
                    tick, wx, wy, wz, level, is_beacon);
            }
            E::LightEmitterRemoved { wx, wy, wz } => {
                log::info!(target: "events", "[tick {}] LightEmitterRemoved ({},{},{})", tick, wx, wy, wz);
            }
            E::LightBordersUpdated { cx, cz } => {
                log::debug!(target: "events", "[tick {}] LightBordersUpdated ({}, {})", tick, cx, cz);
            }
        }
    }
}

impl App {
    // Process file watcher events: reload changed textures and rebind them on existing models.
    pub fn process_texture_file_events(
        &mut self,
        rl: &mut raylib::prelude::RaylibHandle,
        thread: &raylib::prelude::RaylibThread,
    ) {
        use std::collections::HashSet;
        let mut changed: HashSet<String> = HashSet::new();
        for p in self.tex_event_rx.try_iter() {
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
        let choose_path = |mid: geist_blocks::types::MaterialId| -> Option<String> {
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
                chosen.map(|s| {
                    std::fs::canonicalize(&s)
                        .ok()
                        .map(|pb| pb.to_string_lossy().to_string())
                        .unwrap_or(s)
                })
            })
        };
        // Reload any changed paths into cache
        for path in changed.iter() {
            if let Ok(tex) = rl.load_texture(thread, path) {
                tex.set_texture_filter(thread, raylib::consts::TextureFilter::TEXTURE_FILTER_POINT);
                tex.set_texture_wrap(thread, raylib::consts::TextureWrap::TEXTURE_WRAP_REPEAT);
                self.tex_cache.replace_loaded(path.clone(), tex);
                log::debug!("reloaded texture {}", path);
            } else {
                log::warn!("failed to reload texture {}", path);
            }
        }
        let mut rebound: std::collections::HashMap<String, usize> = Default::default();
        // Rebind textures on existing chunk renders
        for (_k, cr) in self.renders.iter_mut() {
            for (mid, model) in cr.parts.iter_mut() {
                let Some(path) = choose_path(*mid) else {
                    continue;
                };
                if !changed.contains(&path) {
                    continue;
                }
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
                let Some(path) = choose_path(*mid) else {
                    continue;
                };
                if !changed.contains(&path) {
                    continue;
                }
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
        for _ in self.worldgen_event_rx.try_iter() {
            changed = true;
        }
        if !changed {
            return;
        }
        let path = std::path::Path::new(&self.world_config_path);
        if !path.exists() {
            log::warn!("worldgen config missing: {}", self.world_config_path);
            return;
        }
        match geist_world::worldgen::load_params_from_path(path) {
            Ok(params) => {
                self.gs.world.update_worldgen_params(params);
                log::info!("worldgen config reloaded from {}", self.world_config_path);
                log::info!("Existing chunks unchanged; new gen uses updated params");
                self.worldgen_dirty = true;
            }
            Err(e) => {
                log::warn!(
                    "worldgen config reload failed ({}): {}",
                    self.world_config_path,
                    e
                );
            }
        }
    }

    pub fn take_worldgen_dirty(&mut self) -> bool {
        if self.worldgen_dirty {
            self.worldgen_dirty = false;
            true
        } else {
            false
        }
    }
}

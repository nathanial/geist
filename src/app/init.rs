use std::collections::HashMap;

use raylib::prelude::*;
use serde::Deserialize;

use super::{App, DebugStats};
use crate::event::{Event, EventQueue};
use crate::gamestate::GameState;
use geist_blocks::{Block, BlockRegistry};
use geist_edit::EditStore;
use geist_lighting::LightingStore;
use geist_render_raylib::{FogShader, LeavesShader, TextureCache};
use geist_runtime::Runtime;
use geist_world::voxel::{World, WorldGenMode};

#[derive(Deserialize)]
struct HotbarConfig {
    items: Vec<String>,
}

impl App {
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
                (world.world_height_hint() as f32) * 0.8,
                (world.world_size_z() as f32) * 0.5,
            )
        };
        let cam = crate::camera::FlyCamera::new(spawn + Vector3::new(0.0, 5.0, 20.0));

        // Renderer-side resources and file watchers (moved from Runtime in Phase 5)
        let leaves_shader = LeavesShader::load_with_base(rl, thread, &assets_root)
            .or_else(|| LeavesShader::load(rl, thread));
        let fog_shader = FogShader::load_with_base(rl, thread, &assets_root)
            .or_else(|| FogShader::load(rl, thread));
        let water_shader =
            geist_render_raylib::WaterShader::load_with_base(rl, thread, &assets_root);
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
                                    let mut y = world.world_height_hint() as i32 - 2;
                                    while y >= 1 {
                                        let b = world.block_at_runtime(&reg, wx, y, wz);
                                        if reg
                                            .get(b.id)
                                            .map(|t| t.is_solid(b.state))
                                            .unwrap_or(false)
                                        {
                                            return (y + 1)
                                                .clamp(1, world.world_height_hint() as i32 - 1);
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
                                    let world_y_top = world.world_height_hint() as i32 - 2;
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
        let ccy = (cam.position.y / world.chunk_size_y as f32).floor() as i32;
        let ccz = (cam.position.z / world.chunk_size_z as f32).floor() as i32;
        queue.emit_now(Event::ViewCenterChanged { ccx, ccy, ccz });
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
            water_shader,
            tex_cache,
            renders: HashMap::new(),
            structure_renders: HashMap::new(),
            minimap_rt: None,
            minimap_zoom: 1.0,
            minimap_yaw: 0.85,
            minimap_pitch: 0.9,
            minimap_pan: Vector3::zero(),
            minimap_ui_rect: None,
            minimap_drag_button: None,
            minimap_drag_pan: false,
            minimap_last_cursor: None,
            event_histogram_pos: Vector2::new(40.0, 40.0),
            event_histogram_dragging: false,
            event_histogram_drag_offset: Vector2::new(0.0, 0.0),
            event_histogram_rect: None,
            event_histogram_size: (360, 220),
            intent_histogram_pos: Vector2::new(420.0, 40.0),
            intent_histogram_dragging: false,
            intent_histogram_drag_offset: Vector2::new(0.0, 0.0),
            intent_histogram_rect: None,
            intent_histogram_size: (380, 260),
            height_histogram_pos: Vector2::new(800.0, 40.0),
            height_histogram_dragging: false,
            height_histogram_drag_offset: Vector2::new(0.0, 0.0),
            height_histogram_rect: None,
            height_histogram_size: (360, 240),
            reg: reg.clone(),
            evt_processed_total: 0,
            evt_processed_by: HashMap::new(),
            intents: HashMap::new(),
            perf_remove_start: HashMap::new(),
            perf_mesh_ms: std::collections::VecDeque::new(),
            perf_light_ms: std::collections::VecDeque::new(),
            perf_total_ms: std::collections::VecDeque::new(),
            perf_remove_ms: std::collections::VecDeque::new(),
            perf_gen_ms: std::collections::VecDeque::new(),
            height_tile_us: std::collections::VecDeque::new(),
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
}

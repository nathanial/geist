use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use raylib::consts::TextureFilter;
use raylib::core::texture::RaylibTexture2D;
use raylib::prelude::*;
use serde::Deserialize;

use super::{
    App, DayCycle, DebugOverlayTab, DebugStats, DiagnosticsTab, OverlayWindow,
    OverlayWindowManager, SUN_STRUCTURE_ID, SchematicOrbit, SunBody, WindowId, WindowTheme,
    render::MINIMAP_MIN_CONTENT_SIDE,
};
use crate::event::{Event, EventQueue};
use crate::gamestate::GameState;
use geist_blocks::{Block, BlockRegistry};
use geist_edit::EditStore;
use geist_geom::Vec3;
use geist_lighting::LightingStore;
use geist_render_raylib::{FogShader, LeavesShader, TextureCache, conv::vec3_from_rl};
use geist_runtime::Runtime;
use geist_structures::{Pose, Structure, StructureEditStore, StructureId};
use geist_world::voxel::generation::TOWER_OUTER_RADIUS;
use geist_world::voxel::{World, WorldGenMode};

#[derive(Deserialize)]
struct HotbarConfig {
    items: Vec<String>,
}

const MONO_FONT_BASE_SIZE: i32 = 96;

const MONO_FONT_CANDIDATES: &[&str] = &[
    "/System/Applications/Utilities/Terminal.app/Contents/Resources/Fonts/SFMono-Regular.otf",
    "/System/Library/Fonts/SFMono-Regular.otf",
    "/System/Library/Fonts/SFNSMono.ttf",
    "/System/Library/Fonts/Menlo.ttc",
    "/Library/Fonts/Menlo.ttc",
    "/System/Library/Fonts/Monaco.ttf",
    "/System/Library/Fonts/Courier New.ttf",
];

const SCHEM_STRUCTURE_ID_BASE: StructureId = 1000;

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

        let ui_font = Self::load_system_mono_font(rl, thread).map(std::sync::Arc::new);

        let runtime = Runtime::new(world.clone(), lighting.clone());
        let mut gs = GameState::new(world.clone(), edits, lighting.clone(), cam.position);
        let mut queue = EventQueue::new();
        let hotbar = Self::load_hotbar(&reg, &assets_root);
        let mut schem_orbits = Vec::new();

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
                                // Non-flat worlds: spawn schematics on orbital platforms that circle the central tower.
                                let height_hint = world.world_height_hint() as f32;
                                let orbit_height = if height_hint > 96.0 {
                                    (height_hint * 0.65).max(64.0).min(height_hint - 32.0)
                                } else {
                                    (height_hint * 0.5).max(32.0)
                                };
                                let max_span = list
                                    .iter()
                                    .map(|ent| ent.size.0.max(ent.size.2) as f32)
                                    .fold(0.0_f32, f32::max);
                                let base_radius =
                                    (TOWER_OUTER_RADIUS as f32 + 48.0).max(max_span * 0.75 + 32.0);
                                let total = list.len() as f32;
                                let mut next_structure_id: StructureId = SCHEM_STRUCTURE_ID_BASE;
                                let center_x = (world.world_size_x() as f32) * 0.5;
                                let center_z = (world.world_size_z() as f32) * 0.5;
                                let platform_block = reg
                                    .id_by_name("stone_bricks")
                                    .or_else(|| reg.id_by_name("stone"))
                                    .map(|id| Block { id, state: 0 })
                                    .unwrap_or(Block::AIR);
                                let glow_block =
                                    reg.id_by_name("glowstone").map(|id| Block { id, state: 0 });
                                let angular_speed = 0.035_f32;

                                for (idx, ent) in list.iter().enumerate() {
                                    let schem_path = ent.path.clone();
                                    let schem_width = ent.size.0.max(1) as usize;
                                    let schem_height = ent.size.1.max(1) as usize;
                                    let schem_depth = ent.size.2.max(1) as usize;
                                    let padding: usize = 6;
                                    let foundation_layers: usize = 2;
                                    let top_clearance: usize = 6;
                                    let struct_sx = (schem_width + padding * 2).max(8);
                                    let struct_sz = (schem_depth + padding * 2).max(8);
                                    let struct_sy =
                                        (schem_height + foundation_layers + top_clearance).max(8);

                                    let mut blocks =
                                        vec![Block::AIR; struct_sx * struct_sy * struct_sz];
                                    for y in 0..foundation_layers {
                                        for z in 0..struct_sz {
                                            for x in 0..struct_sx {
                                                let idx = (y * struct_sz + z) * struct_sx + x;
                                                blocks[idx] = platform_block;
                                            }
                                        }
                                    }
                                    if let Some(glow) = glow_block {
                                        if struct_sx >= 4 && struct_sz >= 4 {
                                            let deck_y = foundation_layers.saturating_sub(1);
                                            let corners = [
                                                (1usize, 1usize),
                                                (struct_sx - 2, 1usize),
                                                (1usize, struct_sz - 2),
                                                (struct_sx - 2, struct_sz - 2),
                                            ];
                                            for &(cx, cz) in &corners {
                                                let idx =
                                                    (deck_y * struct_sz + cz) * struct_sx + cx;
                                                blocks[idx] = glow;
                                            }
                                        }
                                    }

                                    let radius = base_radius;
                                    let angle = if total > 0.0 {
                                        (idx as f32) * (std::f32::consts::TAU / total)
                                    } else {
                                        0.0
                                    };
                                    let target_center_x = center_x + radius * angle.cos();
                                    let target_center_z = center_z + radius * angle.sin();
                                    let pose = Pose {
                                        pos: Vec3::new(
                                            target_center_x - struct_sx as f32 * 0.5,
                                            orbit_height,
                                            target_center_z - struct_sz as f32 * 0.5,
                                        ),
                                        yaw_deg: 0.0,
                                    };

                                    let id = next_structure_id;
                                    next_structure_id = next_structure_id.wrapping_add(1);
                                    let mut structure = Structure {
                                        id,
                                        sx: struct_sx,
                                        sy: struct_sy,
                                        sz: struct_sz,
                                        blocks: Arc::from(blocks.into_boxed_slice()),
                                        edits: StructureEditStore::new(),
                                        pose,
                                        last_delta: Vec3::ZERO,
                                        dirty_rev: 1,
                                        built_rev: 0,
                                    };

                                    match geist_io::load_any_schematic_apply_into_structure(
                                        schem_path.as_path(),
                                        (padding as i32, foundation_layers as i32, padding as i32),
                                        &mut structure,
                                        &reg,
                                    ) {
                                        Ok((sx, sy, sz)) => {
                                            log::info!(
                                                "Loaded schem {:?} into orbital structure {} ({}x{}x{})",
                                                schem_path,
                                                id,
                                                sx,
                                                sy,
                                                sz
                                            );
                                        }
                                        Err(e) => {
                                            log::warn!(
                                                "Failed loading schem {:?}: {}",
                                                schem_path,
                                                e
                                            );
                                        }
                                    }

                                    let rev = structure.dirty_rev;
                                    gs.structures.insert(id, structure);
                                    queue.emit_now(Event::StructureBuildRequested { id, rev });
                                    schem_orbits.push(SchematicOrbit {
                                        id,
                                        radius,
                                        height: orbit_height,
                                        angle,
                                        angular_speed,
                                    });
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

        let window_theme = WindowTheme::default();
        let mut overlay_windows = OverlayWindowManager::new(window_theme);
        overlay_windows.insert(OverlayWindow::new(
            WindowId::DebugTabs,
            Vector2::new(40.0, 40.0),
            (720, 360),
            (540, 260),
        ));
        overlay_windows.insert(OverlayWindow::new(
            WindowId::DiagnosticsTabs,
            Vector2::new(40.0, 360.0),
            (560, 320),
            (420, 260),
        ));
        overlay_windows.insert(OverlayWindow::new(
            WindowId::ChunkVoxels,
            Vector2::new(840.0, 40.0),
            (520, 360),
            (360, 240),
        ));
        let minimap_side =
            App::minimap_side_px(gs.view_radius_chunks).max(MINIMAP_MIN_CONTENT_SIDE);
        let minimap_size = (
            window_theme.padding_x * 2 + minimap_side,
            window_theme.titlebar_height + window_theme.padding_y * 2 + minimap_side,
        );
        let minimap_min = (
            window_theme.padding_x * 2 + MINIMAP_MIN_CONTENT_SIDE,
            window_theme.titlebar_height + window_theme.padding_y * 2 + MINIMAP_MIN_CONTENT_SIDE,
        );
        overlay_windows.insert(OverlayWindow::new(
            WindowId::Minimap,
            Vector2::new(840.0, 360.0),
            minimap_size,
            minimap_min,
        ));
        overlay_windows.clamp_all((rl.get_screen_width(), rl.get_screen_height()));

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

        let day_cycle = DayCycle::new(15.0 * 60.0);
        let day_sample = day_cycle.sample();
        let mut sun = None;
        if let Some((body, structure)) = SunBody::new(
            SUN_STRUCTURE_ID,
            reg.as_ref(),
            vec3_from_rl(cam.position),
            &day_sample,
        ) {
            let rev = structure.dirty_rev;
            gs.structures.insert(body.id, structure);
            queue.emit_now(Event::StructureBuildRequested { id: body.id, rev });
            sun = Some(body);
        }

        Self {
            gs,
            queue,
            runtime,
            cam,
            debug_stats: DebugStats::default(),
            day_cycle,
            day_sample,
            sun,
            schem_orbits,
            hotbar,
            leaves_shader,
            fog_shader,
            water_shader,
            tex_cache,
            renders: HashMap::new(),
            structure_renders: HashMap::new(),
            structure_lights: HashMap::new(),
            structure_light_borders: HashMap::new(),
            ui_font,
            minimap_rt: None,
            minimap_zoom: 1.0,
            minimap_yaw: 0.85,
            minimap_pitch: 0.9,
            minimap_pan: Vector3::zero(),
            minimap_ui_rect: None,
            minimap_drag_button: None,
            minimap_drag_pan: false,
            minimap_last_cursor: None,
            overlay_windows,
            overlay_hover: None,
            overlay_debug_tab: DebugOverlayTab::default(),
            overlay_diagnostics_tab: DiagnosticsTab::default(),
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
            terrain_stage_us: std::array::from_fn(|_| std::collections::VecDeque::new()),
            terrain_stage_calls: std::array::from_fn(|_| std::collections::VecDeque::new()),
            terrain_height_tile_us: std::collections::VecDeque::new(),
            terrain_height_tile_reused: std::collections::VecDeque::new(),
            terrain_cache_hits: std::collections::VecDeque::new(),
            terrain_cache_misses: std::collections::VecDeque::new(),
            terrain_tile_cache_hits: std::collections::VecDeque::new(),
            terrain_tile_cache_misses: std::collections::VecDeque::new(),
            terrain_tile_cache_evictions: std::collections::VecDeque::new(),
            terrain_tile_cache_entries: std::collections::VecDeque::new(),
            terrain_chunk_total_us: std::collections::VecDeque::new(),
            terrain_chunk_fill_us: std::collections::VecDeque::new(),
            terrain_chunk_feature_us: std::collections::VecDeque::new(),
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

    fn load_system_mono_font(rl: &mut RaylibHandle, thread: &RaylibThread) -> Option<Font> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(env_path) = std::env::var("GEIST_MONO_FONT") {
            candidates.push(PathBuf::from(env_path));
        }
        candidates.extend(MONO_FONT_CANDIDATES.iter().map(PathBuf::from));

        for path in candidates {
            if !path.exists() {
                continue;
            }
            let Some(path_str) = path.to_str() else {
                log::warn!("Skipping monospace font with non-UTF8 path: {:?}", path);
                continue;
            };
            match rl.load_font_ex(thread, path_str, MONO_FONT_BASE_SIZE, None) {
                Ok(font) => {
                    font.texture()
                        .set_texture_filter(thread, TextureFilter::TEXTURE_FILTER_TRILINEAR);
                    log::info!(
                        "Loaded UI monospace font {:?} at base size {}",
                        path,
                        MONO_FONT_BASE_SIZE
                    );
                    return Some(font);
                }
                Err(err) => {
                    log::warn!("Failed to load monospace font {:?}: {}", path, err);
                }
            }
        }

        log::warn!("Falling back to raylib built-in font; monospace override unavailable");
        None
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

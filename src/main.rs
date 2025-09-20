mod app;
mod assets;
mod camera;
mod event;
mod gamestate;
mod player;
mod raycast;
#[cfg(test)]
mod stairs_tests;

use clap::{Args, Parser, Subcommand, ValueEnum};
use geist_blocks::BlockRegistry;
use geist_world::{
    ChunkCoord, TERRAIN_STAGE_COUNT, TERRAIN_STAGE_LABELS, TerrainMetrics, TerrainTileCacheStats,
    World, WorldGenMode,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(
    name = "geist",
    version,
    about = "Geist voxel viewer",
    propagate_version = true
)]
struct Cli {
    /// Log to a file; optional path (defaults to geist.log if omitted)
    #[arg(long, global = true, num_args = 0..=1, value_name = "PATH", default_missing_value = "geist.log")]
    log_file: Option<String>,

    /// Assets root directory (overrides GEIST_ASSETS and auto-detect)
    #[arg(long, global = true, value_name = "DIR")]
    assets_root: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the voxel viewer
    Run(RunArgs),

    /// Tools to inspect or analyze schematics
    Schem {
        #[command(subcommand)]
        cmd: SchemCmd,
    },
}

#[derive(Args, Debug)]
struct RunArgs {
    /// World generation preset
    #[arg(long, value_enum, default_value_t = WorldKind::Normal)]
    world: WorldKind,

    /// Flat world thickness (used when --world=flat)
    #[arg(long)]
    flat_thickness: Option<i32>,

    /// World seed
    #[arg(long, default_value_t = 1337)]
    seed: i32,

    /// Number of chunks along X
    #[arg(long, default_value_t = 4)]
    chunks_x: usize,

    /// Hint for the number of vertical chunks to pre-stream near spawn (world height hint = chunks_y_hint × CHUNK_SIZE)
    #[arg(long = "chunks-y-hint", alias = "chunks-y", default_value_t = 8)]
    chunks_y_hint: usize,
    /// Number of chunks along Z
    #[arg(long, default_value_t = 4)]
    chunks_z: usize,

    /// Watch assets/blocks for changes and hot-reload textures
    #[arg(long, default_value_t = true)]
    watch_textures: bool,

    /// Worldgen config path (TOML)
    #[arg(
        long,
        value_name = "PATH",
        default_value = "assets/worldgen/worldgen.toml"
    )]
    world_config: String,

    /// Watch worldgen config for changes and hot-reload params
    #[arg(long, default_value_t = true)]
    watch_worldgen: bool,

    /// Rebuild loaded chunks automatically when worldgen config changes
    #[arg(long, default_value_t = true)]
    rebuild_on_worldgen_change: bool,

    /// Disable frustum culling (render all loaded chunks)
    #[arg(long, default_value_t = false)]
    no_frustum_culling: bool,

    /// Generate chunks up to radius 1 and print terrain metrics instead of launching the viewer
    #[arg(long, default_value_t = false)]
    terrain_metrics: bool,

    /// Horizontal radius (in chunks) when sampling terrain metrics
    #[arg(long, default_value_t = 6)]
    terrain_metrics_radius: i32,

    /// Vertical half-span (in chunks) when sampling terrain metrics; defaults to the radius, capped by chunks_y_hint
    #[arg(long)]
    terrain_metrics_vertical: Option<i32>,
}

impl Default for RunArgs {
    fn default() -> Self {
        Self {
            world: WorldKind::Normal,
            flat_thickness: None,
            seed: 1337,
            chunks_x: 4,
            chunks_y_hint: 8,
            chunks_z: 4,
            watch_textures: true,
            world_config: "assets/worldgen/worldgen.toml".to_string(),
            watch_worldgen: true,
            rebuild_on_worldgen_change: true,
            no_frustum_culling: false,
            terrain_metrics: false,
            terrain_metrics_radius: 6,
            terrain_metrics_vertical: None,
        }
    }
}

#[derive(Clone, Debug, ValueEnum, Default)]
enum WorldKind {
    #[default]
    Normal,
    Flat,
    SchemOnly,
}

#[derive(Subcommand, Debug)]
enum SchemCmd {
    /// Report unsupported blocks or counts from a schematic
    Report(SchemReportArgs),
}

#[derive(Args, Debug)]
struct SchemReportArgs {
    /// Show block counts instead of unsupported list
    #[arg(long, alias = "counts")]
    counts: bool,

    /// Optional schematic path
    #[arg(value_name = "SCHEM_PATH")]
    path: Option<PathBuf>,
}

fn load_block_registry(assets_root: &Path) -> Arc<BlockRegistry> {
    let mats_path = crate::assets::materials_path(assets_root);
    let blocks_path = crate::assets::blocks_path(assets_root);
    let mut reg = BlockRegistry::load_from_paths(&mats_path, &blocks_path).unwrap_or_else(|e| {
        log::warn!(
            "Failed to load runtime voxel registry from {:?} / {:?}: {}",
            mats_path,
            blocks_path,
            e
        );
        BlockRegistry::new()
    });
    for material in &mut reg.materials.materials {
        for tex_path in &mut material.texture_candidates {
            if tex_path.is_relative() {
                *tex_path = assets_root.join(&*tex_path);
            }
        }
    }
    Arc::new(reg)
}

fn load_worldgen_params(world: &World, assets_root: &Path, config_path: &str) {
    let cfg_path = Path::new(config_path);
    let cfg_path_abs = if cfg_path.exists() {
        cfg_path.to_path_buf()
    } else {
        assets_root.join(cfg_path)
    };

    if cfg_path_abs.exists() {
        match geist_world::worldgen::load_params_from_path(&cfg_path_abs) {
            Ok(params) => {
                world.update_worldgen_params(params);
                log::info!("Loaded worldgen config from {:?}", cfg_path_abs);
            }
            Err(e) => {
                log::warn!(
                    "worldgen config load failed (path={:?}): {}",
                    cfg_path_abs,
                    e
                );
            }
        }
    } else {
        log::info!(
            "worldgen config not found at {}; using defaults",
            config_path
        );
    }
}

#[derive(Clone)]
struct ChunkReport {
    coord: ChunkCoord,
    metrics: TerrainMetrics,
}

fn chunk_coords_within_radius(
    center: ChunkCoord,
    radius: i32,
    vertical_limit: i32,
) -> Vec<ChunkCoord> {
    if radius < 0 {
        return Vec::new();
    }
    let v_limit = vertical_limit.max(0).min(radius);
    let mut coords = Vec::new();
    let r_sq = i64::from(radius) * i64::from(radius);
    for dy in -v_limit..=v_limit {
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                let dx64 = i64::from(dx);
                let dy64 = i64::from(dy);
                let dz64 = i64::from(dz);
                let dist_sq = dx64 * dx64 + dy64 * dy64 + dz64 * dz64;
                if dist_sq <= r_sq {
                    coords.push(center.offset(dx, dy, dz));
                }
            }
        }
    }
    coords
}

fn run_terrain_metrics(run: &RunArgs, assets_root: &Path) {
    let mut radius = run.terrain_metrics_radius.max(0);
    if radius == 0 {
        radius = 1;
    }
    println!("== Terrain Metrics Probe (radius {radius}) ==");

    let reg = load_block_registry(assets_root);
    println!(
        "Loaded voxel registry: {} materials, {} blocks",
        reg.materials.materials.len(),
        reg.blocks.len()
    );

    let mut chunks_y_hint = run.chunks_y_hint;
    if chunks_y_hint == 0 {
        log::warn!("--chunks-y-hint must be at least 1; using 1 instead");
        chunks_y_hint = 1;
    }

    let world_mode = match run.world {
        WorldKind::SchemOnly => WorldGenMode::Flat { thickness: 0 },
        WorldKind::Flat => WorldGenMode::Flat {
            thickness: run.flat_thickness.unwrap_or(1),
        },
        WorldKind::Normal => WorldGenMode::Normal,
    };

    let world = World::new(
        run.chunks_x,
        chunks_y_hint,
        run.chunks_z,
        run.seed,
        world_mode,
    );

    load_worldgen_params(&world, assets_root, &run.world_config);

    let mut vertical_limit = run
        .terrain_metrics_vertical
        .unwrap_or(radius)
        .clamp(0, run.chunks_y_hint as i32);
    if vertical_limit == 0 && chunks_y_hint > 1 && radius > 0 {
        vertical_limit = 1;
    }
    let vertical_limit = vertical_limit.min(radius);
    let center = ChunkCoord::new(0, 0, 0);
    let coords = chunk_coords_within_radius(center, radius, vertical_limit);
    let mut columns: BTreeMap<(i32, i32), Vec<ChunkCoord>> = BTreeMap::new();
    for coord in coords {
        columns.entry((coord.cx, coord.cz)).or_default().push(coord);
    }
    for column in columns.values_mut() {
        column.sort_by_key(|c| c.cy);
    }

    let mut reports: Vec<ChunkReport> = Vec::new();
    for (_, column_coords) in columns.into_iter() {
        let mut ctx = world.make_gen_ctx();
        for coord in column_coords {
            let chunk_result =
                geist_chunk::generate_chunk_buffer_with_ctx(&world, coord, &reg, &mut ctx);
            let geist_chunk::ChunkGenerateResult {
                buf: _,
                occupancy: _,
                terrain_metrics,
            } = chunk_result;
            reports.push(ChunkReport {
                coord,
                metrics: terrain_metrics,
            });
        }
    }

    reports.sort_by(|a, b| {
        center
            .distance_sq(a.coord)
            .cmp(&center.distance_sq(b.coord))
            .then(a.coord.cy.cmp(&b.coord.cy))
            .then(a.coord.cz.cmp(&b.coord.cz))
            .then(a.coord.cx.cmp(&b.coord.cx))
    });

    print_terrain_metrics_summary(run, &world, &reports, radius, vertical_limit);
}

fn print_terrain_metrics_summary(
    run: &RunArgs,
    world: &World,
    reports: &[ChunkReport],
    radius: i32,
    vertical_limit: i32,
) {
    if reports.is_empty() {
        println!("No chunk metrics captured.");
        return;
    }

    let world_kind = match run.world {
        WorldKind::Normal => "Normal",
        WorldKind::Flat => "Flat",
        WorldKind::SchemOnly => "SchemOnly",
    };

    println!(
        "Seed: {} | World: {} | Chunk size: {}³",
        run.seed, world_kind, world.chunk_size_x
    );
    println!(
        "Density hints: chunks_x={} chunks_y_hint={} chunks_z={}",
        run.chunks_x, run.chunks_y_hint, run.chunks_z
    );

    println!(
        "Generated {} chunk(s) within radius {} (vertical limit {}):",
        reports.len(),
        radius,
        vertical_limit
    );

    let chunk_count = reports.len() as f64;
    let mut total_us_sum = 0u64;
    let mut total_us_min = u32::MAX;
    let mut total_us_max = 0u32;
    let mut fill_us_sum = 0u64;
    let mut feature_us_sum = 0u64;
    let mut height_tile_total_us_sum = 0u64;
    let mut height_tile_unique_us_sum = 0u64;
    let mut unique_height_tiles = 0u64;
    let mut reused_height_tiles = 0u64;
    let mut height_tile_columns_unique = 0u64;
    let mut height_tile_columns_total = 0u64;
    let mut stage_time_sum = [0u64; TERRAIN_STAGE_COUNT];
    let mut stage_call_sum = [0u64; TERRAIN_STAGE_COUNT];
    let mut height_cache_hits_sum = 0u64;
    let mut height_cache_misses_sum = 0u64;
    let mut latest_tile_cache = TerrainTileCacheStats::default();

    for report in reports {
        let metrics = &report.metrics;
        let timing = &metrics.chunk_timing;

        total_us_sum += u64::from(timing.total_us);
        fill_us_sum += u64::from(timing.voxel_fill_us);
        feature_us_sum += u64::from(timing.feature_us);
        height_tile_total_us_sum += u64::from(timing.height_tile_us);
        height_tile_columns_total += u64::from(metrics.height_tile.columns);

        if timing.total_us < total_us_min {
            total_us_min = timing.total_us;
        }
        if timing.total_us > total_us_max {
            total_us_max = timing.total_us;
        }

        if metrics.height_tile.reused {
            reused_height_tiles = reused_height_tiles.saturating_add(1);
        } else {
            unique_height_tiles = unique_height_tiles.saturating_add(1);
            height_tile_unique_us_sum += u64::from(metrics.height_tile.duration_us);
            height_tile_columns_unique += u64::from(metrics.height_tile.columns);
        }

        height_cache_hits_sum += u64::from(metrics.height_cache_hits);
        height_cache_misses_sum += u64::from(metrics.height_cache_misses);

        latest_tile_cache = metrics.tile_cache;

        for idx in 0..TERRAIN_STAGE_COUNT {
            stage_time_sum[idx] += u64::from(metrics.stages[idx].time_us);
            stage_call_sum[idx] += u64::from(metrics.stages[idx].calls);
        }
    }

    let avg_total_ms = total_us_sum as f64 / chunk_count / 1000.0;
    let avg_fill_ms = fill_us_sum as f64 / chunk_count / 1000.0;
    let avg_feature_ms = feature_us_sum as f64 / chunk_count / 1000.0;
    let avg_height_tile_ms = height_tile_total_us_sum as f64 / chunk_count / 1000.0;
    let min_total_ms = total_us_min as f64 / 1000.0;
    let max_total_ms = total_us_max as f64 / 1000.0;

    println!(
        "Chunk timing: avg {:.3} ms (min {:.3}, max {:.3}) | fill avg {:.3} ms | feature avg {:.3} ms | height tile avg {:.3} ms",
        avg_total_ms, min_total_ms, max_total_ms, avg_fill_ms, avg_feature_ms, avg_height_tile_ms
    );

    let reuse_ratio = if reports.is_empty() {
        0.0
    } else {
        reused_height_tiles as f64 / reports.len() as f64 * 100.0
    };
    let avg_unique_tile_ms = if unique_height_tiles > 0 {
        height_tile_unique_us_sum as f64 / unique_height_tiles as f64 / 1000.0
    } else {
        0.0
    };

    println!(
        "Height tiles: {} unique (avg recompute {:.3} ms) | reused {} chunk(s) ({:.1}% reuse)",
        unique_height_tiles, avg_unique_tile_ms, reused_height_tiles, reuse_ratio
    );

    println!(
        "Height columns processed: total {} (unique {})",
        height_tile_columns_total, height_tile_columns_unique
    );

    let cache_total = height_cache_hits_sum + height_cache_misses_sum;
    let cache_hit_rate = if cache_total == 0 {
        0.0
    } else {
        height_cache_hits_sum as f64 / cache_total as f64 * 100.0
    };
    println!(
        "Height cache: {} hits, {} misses (hit rate {:.1}%)",
        height_cache_hits_sum, height_cache_misses_sum, cache_hit_rate
    );

    let tile_total = latest_tile_cache.hits + latest_tile_cache.misses;
    let tile_hit_rate = if tile_total == 0 {
        0.0
    } else {
        latest_tile_cache.hits as f64 / tile_total as f64 * 100.0
    };
    println!(
        "Tile cache: {} hits, {} misses, {} evictions, {} entries (hit rate {:.1}%)",
        latest_tile_cache.hits,
        latest_tile_cache.misses,
        latest_tile_cache.evictions,
        latest_tile_cache.entries,
        tile_hit_rate
    );

    println!("Stage timings (avg per chunk):");
    for idx in 0..TERRAIN_STAGE_COUNT {
        let avg_stage_ms = stage_time_sum[idx] as f64 / chunk_count / 1000.0;
        let avg_calls = stage_call_sum[idx] as f64 / chunk_count;
        println!(
            "  - {:<7}: {:>6.3} ms (avg calls {:>5.2})",
            TERRAIN_STAGE_LABELS[idx], avg_stage_ms, avg_calls
        );
    }

    println!(
        "Use --terrain-metrics-radius/--terrain-metrics-vertical to adjust coverage; per-chunk dumps are suppressed to keep output concise."
    );
}

fn main() {
    // Parse CLI args
    let cli = Cli::parse();

    // Initialize logging: to file if --log-file used; else env_logger to stderr
    if let Some(path) = cli.log_file.clone() {
        let level = match std::env::var("RUST_LOG")
            .ok()
            .unwrap_or_else(|| "info".to_string())
            .to_lowercase()
            .as_str()
        {
            "trace" => simplelog::LevelFilter::Trace,
            "debug" => simplelog::LevelFilter::Debug,
            "warn" => simplelog::LevelFilter::Warn,
            "error" => simplelog::LevelFilter::Error,
            _ => simplelog::LevelFilter::Info,
        };
        let config = simplelog::ConfigBuilder::new()
            .set_target_level(simplelog::LevelFilter::Info)
            .build();
        match std::fs::File::create(&path) {
            Ok(file) => {
                let _ = simplelog::WriteLogger::init(level, config, file);
                eprintln!("Logging to file: {} (level: {:?})", path, level);
            }
            Err(e) => {
                eprintln!(
                    "Failed to open log file {}: {}. Falling back to stderr.",
                    path, e
                );
                env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
                    .init();
            }
        }
    } else {
        // Initialize logging (RUST_LOG=info by default; override with env)
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    }

    // Resolve assets root now (CLI overrides env and auto-detect)
    let assets_root = crate::assets::resolve_assets_root(cli.assets_root.clone());

    // Determine command (default to Run with defaults)
    let command = cli.command.unwrap_or(Command::Run(RunArgs::default()));

    match command {
        Command::Schem {
            cmd: SchemCmd::Report(args),
        } => {
            let schem_path = args
                .path
                .clone()
                .unwrap_or_else(|| PathBuf::from("schematics/anvilstead.schem"));
            if args.counts {
                match geist_io::count_blocks_in_file(std::path::Path::new(&schem_path)) {
                    Ok(mut entries) => {
                        entries.sort_by(|a, b| b.1.cmp(&a.1));
                        println!("Block counts in {:?} (excluding air):", schem_path);
                        for (id, c) in entries {
                            println!("{:>8}  {}", c, id);
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to analyze {:?}: {}", schem_path, e);
                        std::process::exit(2);
                    }
                }
            } else {
                match geist_io::find_unsupported_blocks_in_file(std::path::Path::new(&schem_path)) {
                    Ok(list) => {
                        if list.is_empty() {
                            println!(
                                "All blocks in {:?} are supported by current mapper.",
                                schem_path
                            );
                        } else {
                            println!("Unsupported block types ({}):", list.len());
                            for id in list {
                                println!("- {}", id);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to analyze {:?}: {}", schem_path, e);
                        std::process::exit(2);
                    }
                }
            }
        }
        Command::Run(run) => {
            if run.terrain_metrics {
                run_terrain_metrics(&run, assets_root.as_path());
            } else {
                run_app(run, assets_root);
            }
        }
    }
}

fn run_app(run: RunArgs, assets_root: std::path::PathBuf) {
    // Silence raylib's internal logging unless debugging raylib itself
    unsafe {
        // 7 == LOG_NONE in raylib (0 was LOG_NONE; 0 was LOG_ALL and was too chatty)
        raylib::ffi::SetTraceLogLevel(7);
    }

    let (mut rl, thread) = raylib::init()
        .size(1280, 720)
        .title("Geist Voxel View (Rust)")
        .msaa_4x()
        .resizable()
        .build();

    // Some raylib builds reset the trace level during init; set it again after init
    unsafe {
        raylib::ffi::SetTraceLogLevel(7);
    }

    rl.set_target_fps(60);

    // Load runtime voxel registry (materials + block types)
    let reg = load_block_registry(&assets_root);
    log::info!(
        "Loaded voxel registry: {} materials, {} blocks",
        reg.materials.materials.len(),
        reg.blocks.len()
    );
    rl.disable_cursor();
    // World + stores (configurable via CLI)
    let chunks_x = run.chunks_x;
    let mut chunks_y_hint = run.chunks_y_hint;
    if chunks_y_hint == 0 {
        log::warn!("--chunks-y-hint must be at least 1; using 1 instead");
        chunks_y_hint = 1;
    }
    let chunks_z = run.chunks_z;
    let world_seed = run.seed;
    let world_mode = match run.world {
        WorldKind::SchemOnly => WorldGenMode::Flat { thickness: 0 },
        WorldKind::Flat => WorldGenMode::Flat {
            thickness: run.flat_thickness.unwrap_or(1),
        },
        WorldKind::Normal => WorldGenMode::Normal,
    };
    let world = Arc::new(World::new(
        chunks_x,
        chunks_y_hint,
        chunks_z,
        world_seed,
        world_mode,
    ));
    // Initial worldgen params load (optional)
    load_worldgen_params(world.as_ref(), &assets_root, &run.world_config);
    let lighting_store = Arc::new(geist_lighting::LightingStore::new(
        world.chunk_size_x,
        world.chunk_size_y,
        world.chunk_size_z,
    ));
    let edit_store = geist_edit::EditStore::new(
        world.chunk_size_x as i32,
        world.chunk_size_y as i32,
        world.chunk_size_z as i32,
    );

    let mut app = crate::app::App::new(
        &mut rl,
        &thread,
        world.clone(),
        lighting_store.clone(),
        edit_store,
        reg.clone(),
        run.watch_textures,
        run.watch_worldgen,
        // Use absolute path for worldgen watcher if available
        {
            use std::path::Path;
            let cfgp = Path::new(&run.world_config);
            let abs = if cfgp.is_absolute() {
                cfgp.to_path_buf()
            } else {
                let p = assets_root.join(cfgp);
                p
            };
            abs.to_string_lossy().to_string()
        },
        run.rebuild_on_worldgen_change,
        assets_root.clone(),
    );

    // Apply initial frustum culling preference from CLI
    app.gs.frustum_culling_enabled = !run.no_frustum_culling;

    while !rl.window_should_close() {
        let dt = rl.get_frame_time();
        // Hot-reload textures modified under assets/blocks
        app.process_texture_file_events(&mut rl, &thread);
        // Hot-reload worldgen params when config changes
        app.process_worldgen_file_events();
        app.step(&mut rl, &thread, dt);
        app.render(&mut rl, &thread);
    }
}

#[derive(Args, Debug)]
pub struct SnapArgs {
    /// Screenshot width in pixels
    #[arg(long, default_value_t = 512)]
    pub width: i32,

    /// Screenshot height in pixels
    #[arg(long, default_value_t = 512)]
    pub height: i32,

    /// Number of camera angles around each item (e.g., 4 or 8)
    #[arg(long, default_value_t = 8)]
    pub angles: usize,

    /// World seed
    #[arg(long, default_value_t = 1337)]
    pub seed: i32,

    /// Number of chunks along X
    #[arg(long, default_value_t = 4)]
    pub chunks_x: usize,

    /// Hint for the number of vertical chunks to pre-stream
    #[arg(long = "chunks-y-hint", alias = "chunks-y", default_value_t = 8)]
    pub chunks_y_hint: usize,

    /// Number of chunks along Z
    #[arg(long, default_value_t = 4)]
    pub chunks_z: usize,

    /// Worldgen config path (TOML)
    #[arg(
        long,
        value_name = "PATH",
        default_value = "assets/worldgen/worldgen.toml"
    )]
    pub world_config: String,
}

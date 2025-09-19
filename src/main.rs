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
use geist_world::voxel::{World, WorldGenMode};
use std::path::PathBuf;
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
        Command::Run(run) => run_app(run, assets_root),
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
    let mats_path = crate::assets::materials_path(&assets_root);
    let blocks_path = crate::assets::blocks_path(&assets_root);
    let mut reg0 = BlockRegistry::load_from_paths(&mats_path, &blocks_path).unwrap_or_else(|e| {
        log::warn!(
            "Failed to load runtime voxel registry from {:?} / {:?}: {}",
            mats_path,
            blocks_path,
            e
        );
        BlockRegistry::new()
    });
    // Normalize material texture paths to absolute so they load regardless of CWD
    {
        use std::path::PathBuf;
        for m in &mut reg0.materials.materials {
            for p in &mut m.texture_candidates {
                if p.is_relative() {
                    let joined: PathBuf = assets_root.join(&p);
                    *p = joined;
                }
            }
        }
    }
    let reg = std::sync::Arc::new(reg0);
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
    {
        use std::path::Path;
        let cfg_path = Path::new(&run.world_config);
        let cfg_path_abs = if cfg_path.exists() {
            cfg_path.to_path_buf()
        } else {
            let alt = assets_root.join(cfg_path);
            alt
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
                run.world_config
            );
        }
    }
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

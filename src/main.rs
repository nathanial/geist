mod app;
mod camera;
mod chunkbuf;
mod edit;
mod event;
mod gamestate;
mod lighting;
mod mcworld;
mod mesher;
mod meshing_core;
mod player;
mod raycast;
mod runtime;
mod schem;
mod shaders;
mod structure;
mod voxel;

use clap::{Args, Parser, Subcommand, ValueEnum};
use raylib::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;
use voxel::{World, WorldGenMode};

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

#[derive(Args, Debug, Default)]
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
    /// Number of chunks along Z
    #[arg(long, default_value_t = 4)]
    chunks_z: usize,

    /// Chunk size along X
    #[arg(long, default_value_t = 32)]
    chunk_size_x: usize,
    /// Chunk size along Y
    #[arg(long, default_value_t = 48)]
    chunk_size_y: usize,
    /// Chunk size along Z
    #[arg(long, default_value_t = 32)]
    chunk_size_z: usize,
}

#[derive(Clone, Debug, ValueEnum)]
enum WorldKind {
    Normal,
    Flat,
    SchemOnly,
}

impl Default for WorldKind {
    fn default() -> Self {
        WorldKind::Normal
    }
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
                match crate::schem::count_blocks_in_file(std::path::Path::new(&schem_path)) {
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
                match crate::schem::find_unsupported_blocks_in_file(std::path::Path::new(
                    &schem_path,
                )) {
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
            return;
        }
        Command::Run(run) => run_app(run),
    }
}

fn run_app(run: RunArgs) {
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
    rl.disable_cursor();
    // World + stores (configurable via CLI)
    let chunk_size_x = run.chunk_size_x;
    let chunk_size_y = run.chunk_size_y;
    let chunk_size_z = run.chunk_size_z;
    let chunks_x = run.chunks_x;
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
        chunks_z,
        chunk_size_x,
        chunk_size_y,
        chunk_size_z,
        world_seed,
        world_mode,
    ));
    let lighting_store = Arc::new(lighting::LightingStore::new(
        chunk_size_x,
        chunk_size_y,
        chunk_size_z,
    ));
    let edit_store = edit::EditStore::new(
        chunk_size_x as i32,
        chunk_size_y as i32,
        chunk_size_z as i32,
    );

    let mut app = crate::app::App::new(
        &mut rl,
        &thread,
        world.clone(),
        lighting_store.clone(),
        edit_store,
    );

    while !rl.window_should_close() {
        let dt = rl.get_frame_time();
        app.step(&mut rl, &thread, dt);
        app.render(&mut rl, &thread);
    }
}

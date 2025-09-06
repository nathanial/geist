mod app;
mod camera;
mod chunkbuf;
mod edit;
mod event;
mod gamestate;
mod lighting;
mod mesher;
mod meshing_core;
mod structure;
mod player;
mod raycast;
mod runtime;
mod shaders;
mod voxel;
mod schem;

use raylib::prelude::*;
use std::sync::Arc;
use voxel::World;

fn main() {
    // Handle CLI mode: schematic support report
    {
        let mut args = std::env::args().skip(1).collect::<Vec<String>>();
        let mut report_mode = false;
        let mut schem_path = String::from("schematics/anvilstead.schem");
        let mut i = 0usize;
        while i < args.len() {
            let a = &args[i];
            if a == "--schem-report" {
                report_mode = true;
                if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                    schem_path = args[i + 1].clone();
                    i += 1;
                }
            }
            i += 1;
        }
        if report_mode {
            match crate::schem::find_unsupported_blocks_in_file(std::path::Path::new(&schem_path)) {
                Ok(list) => {
                    if list.is_empty() {
                        println!("All blocks in {:?} are supported by current mapper.", schem_path);
                    } else {
                        println!("Unsupported block types ({}):", list.len());
                        for id in list { println!("- {}", id); }
                    }
                    return;
                }
                Err(e) => {
                    eprintln!("Failed to analyze {:?}: {}", schem_path, e);
                    std::process::exit(2);
                }
            }
        }
    }

    // Initialize logging (RUST_LOG=info by default; override with env)
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Silence raylib's internal logging unless debugging raylib itself
    unsafe {
        // 7 == LOG_NONE in raylib (0 was LOG_ALL and was too chatty)
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
    // World + stores
    let chunk_size_x = 32usize;
    let chunk_size_y = 48usize;
    let chunk_size_z = 32usize;
    let chunks_x = 4usize;
    let chunks_z = 4usize;
    let world_seed = 1337;
    let world = Arc::new(World::new(
        chunks_x,
        chunks_z,
        chunk_size_x,
        chunk_size_y,
        chunk_size_z,
        world_seed,
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

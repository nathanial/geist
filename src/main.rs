mod camera;
mod chunkbuf;
mod edit;
mod app;
mod event;
mod gamestate;
mod lighting;
mod mesher;
mod player;
mod raycast;
mod runtime;
mod shaders;
mod voxel;

use raylib::prelude::*;
use voxel::World;
use std::sync::Arc;

fn main() {
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
        .build();

    // Some raylib builds reset the trace level during init; set it again after init
    unsafe { raylib::ffi::SetTraceLogLevel(7); }

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
    let lighting_store = Arc::new(lighting::LightingStore::new(chunk_size_x, chunk_size_y, chunk_size_z));
    let edit_store = edit::EditStore::new(
        chunk_size_x as i32,
        chunk_size_y as i32,
        chunk_size_z as i32,
    );

    let mut app = crate::app::App::new(&mut rl, &thread, world.clone(), lighting_store.clone(), edit_store);

    while !rl.window_should_close() {
        let dt = rl.get_frame_time();
        app.step(&mut rl, &thread, dt);
        app.render(&mut rl, &thread);
    }
}

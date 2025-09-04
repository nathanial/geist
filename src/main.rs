mod camera;
mod voxel;

use camera::FlyCamera;
use raylib::prelude::*;
use voxel::{generate_heightmap_chunk, Block};

fn main() {
    let (mut rl, thread) = raylib::init()
        .size(1280, 720)
        .title("Geist Voxel View (Rust)")
        .msaa_4x()
        .build();

    rl.set_target_fps(60);
    rl.disable_cursor();

    // Build a simple heightmap chunk
    let size_x = 48usize;
    let size_y = 48usize;
    let size_z = 48usize;
    let chunk = generate_heightmap_chunk(size_x, size_y, size_z, 1337);

    // Place camera to see the scene
    let mut cam = FlyCamera::new(Vector3::new(24.0, 24.0, 64.0));

    // Rendering options
    let mut show_grid = true;
    let mut wireframe = false;

    while !rl.window_should_close() {
        let dt = rl.get_frame_time();
        cam.update(&mut rl, dt);

        if rl.is_key_pressed(KeyboardKey::KEY_G) {
            show_grid = !show_grid;
        }
        if rl.is_key_pressed(KeyboardKey::KEY_F) {
            wireframe = !wireframe;
        }

        let mut d = rl.begin_drawing(&thread);
        d.clear_background(Color::new(210, 221, 235, 255));

        let camera3d = cam.to_camera3d();
        {
            let mut d3 = d.begin_mode3D(camera3d);
            if show_grid {
                d3.draw_grid(64, 1.0);
            }

            // Draw exposed voxels only
            let base_color = Color::new(116, 178, 102, 255); // grass-like
            let rock_color = Color::new(130, 126, 132, 255);
            for y in 0..chunk.size_y {
                for z in 0..chunk.size_z {
                    for x in 0..chunk.size_x {
                        if chunk.get(x, y, z) == Block::Solid && chunk.is_exposed(x, y, z) {
                            let pos = Vector3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
                            let color = if y > (chunk.size_y as f32 * 0.45) as usize {
                                base_color
                            } else {
                                rock_color
                            };
                            if wireframe {
                                d3.draw_cube_wires(pos, 1.0, 1.0, 1.0, color);
                            } else {
                                d3.draw_cube(pos, 1.0, 1.0, 1.0, color);
                            }
                        }
                    }
                }
            }
        }

        d.draw_text("Voxel view: Tab capture, WASD+QE fly, F wireframe, G grid", 12, 12, 18, Color::DARKGRAY);
        d.draw_fps(12, 36);
    }
}

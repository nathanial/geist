mod camera;
mod voxel;

use camera::FlyCamera;
use raylib::prelude::*;
use voxel::{generate_heightmap_chunk, Block};
use std::path::Path;

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

    // Prepare textured cube models per block type (if assets available)
    // Fallback to colored cubes if textures are unavailable
    let assets_dir = Path::new("assets");
    let has_assets = assets_dir.exists();

    // Generate a unit cube mesh and models per texture if possible
    let mut model_grass = None;
    let mut model_dirt = None;
    let mut model_stone = None;
    // Hold textures to keep them alive for the lifetime of models
    let mut _tex_grass = None;
    let mut _tex_dirt = None;
    let mut _tex_stone = None;

    if has_assets {
        // helper to build a model+texture
        let mut build_model = |tex_candidates: &[&str]| -> Option<(raylib::core::models::Model, raylib::core::texture::Texture2D)> {
            // try first path that loads
            let mut tex_opt = None;
            for p in tex_candidates {
                if let Ok(t) = rl.load_texture(&thread, p) { tex_opt = Some(t); break; }
            }
            let tex = tex_opt?;
            let cube_mesh = raylib::core::models::Mesh::gen_mesh_cube(&thread, 1.0, 1.0, 1.0);
            let model = rl.load_model_from_mesh(&thread, unsafe { cube_mesh.make_weak() }).ok()?;
            let mut model = model; // mutable to set material
            if let Some(mat) = model.materials_mut().get_mut(0) {
                mat.set_material_texture(raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO, &tex);
            }
            Some((model, tex))
        };

        if let Some((m, t)) = build_model(&["assets/grass.png", "assets/blocks/grass_side.png", "assets/blocks/grass_top.png"]) {
            model_grass = Some(m);
            _tex_grass = Some(t);
        }
        if let Some((m, t)) = build_model(&["assets/dirt.png", "assets/blocks/dirt.png"]) {
            model_dirt = Some(m);
            _tex_dirt = Some(t);
        }
        if let Some((m, t)) = build_model(&["assets/stone.png", "assets/blocks/stone.png"]) {
            model_stone = Some(m);
            _tex_stone = Some(t);
        }
    }

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
            let dirt_color = Color::new(143, 119, 84, 255);
            let rock_color = Color::new(130, 126, 132, 255);
            for y in 0..chunk.size_y {
                for z in 0..chunk.size_z {
                    for x in 0..chunk.size_x {
                        let b = chunk.get(x, y, z);
                        if b.is_solid() && chunk.is_exposed(x, y, z) {
                            let pos = Vector3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
                            match b {
                                Block::Grass => {
                                    if let Some(m) = model_grass.as_ref() {
                                        if wireframe { d3.draw_model_wires(m, pos, 1.0, Color::WHITE); }
                                        else { d3.draw_model(m, pos, 1.0, Color::WHITE); }
                                    } else {
                                        if wireframe { d3.draw_cube_wires(pos, 1.0, 1.0, 1.0, base_color); }
                                        else { d3.draw_cube(pos, 1.0, 1.0, 1.0, base_color); }
                                    }
                                }
                                Block::Dirt => {
                                    if let Some(m) = model_dirt.as_ref() {
                                        if wireframe { d3.draw_model_wires(m, pos, 1.0, Color::WHITE); }
                                        else { d3.draw_model(m, pos, 1.0, Color::WHITE); }
                                    } else {
                                        if wireframe { d3.draw_cube_wires(pos, 1.0, 1.0, 1.0, dirt_color); }
                                        else { d3.draw_cube(pos, 1.0, 1.0, 1.0, dirt_color); }
                                    }
                                }
                                Block::Stone => {
                                    if let Some(m) = model_stone.as_ref() {
                                        if wireframe { d3.draw_model_wires(m, pos, 1.0, Color::WHITE); }
                                        else { d3.draw_model(m, pos, 1.0, Color::WHITE); }
                                    } else {
                                        if wireframe { d3.draw_cube_wires(pos, 1.0, 1.0, 1.0, rock_color); }
                                        else { d3.draw_cube(pos, 1.0, 1.0, 1.0, rock_color); }
                                    }
                                }
                                Block::Air => { /* skip */ }
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

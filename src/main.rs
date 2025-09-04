mod camera;
mod voxel;

use camera::FlyCamera;
use raylib::prelude::*;
use raylib::core::texture::Image;
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
    let mut model_sand = None;
    let mut model_snow = None;
    let mut model_logoak = None;
    // Hold textures to keep them alive for the lifetime of models
    let mut _tex_grass = None;
    let mut _tex_dirt = None;
    let mut _tex_stone = None;
    let mut _tex_sand = None;
    let mut _tex_snow = None;
    let mut _tex_logoak = None;

    if has_assets {
        // Build grass with per-face mapping (top/side/bottom atlas)
        if let Some((m, t)) = build_per_face_cube_model(
            &mut rl,
            &thread,
            &["assets/blocks/grass_top.png"],
            &["assets/blocks/grass_side.png"],
            &["assets/blocks/dirt.png"],
            true,
        ) {
            model_grass = Some(m);
            _tex_grass = Some(t);
        }

        // Dirt: uniform texture on all faces
        if let Some((m, t)) = build_uniform_cube_model(&mut rl, &thread, &["assets/dirt.png", "assets/blocks/dirt.png"]) {
            model_dirt = Some(m);
            _tex_dirt = Some(t);
        }

        // Stone: uniform texture on all faces
        if let Some((m, t)) = build_uniform_cube_model(&mut rl, &thread, &["assets/stone.png", "assets/blocks/stone.png"]) {
            model_stone = Some(m);
            _tex_stone = Some(t);
        }
        // Sand (uniform)
        if let Some((m, t)) = build_uniform_cube_model(&mut rl, &thread, &["assets/sand.png", "assets/blocks/sand.png"]) {
            model_sand = Some(m);
            _tex_sand = Some(t);
        }
        // Snow (uniform)
        if let Some((m, t)) = build_uniform_cube_model(&mut rl, &thread, &["assets/snow.png", "assets/blocks/snow.png"]) {
            model_snow = Some(m);
            _tex_snow = Some(t);
        }
        // Log oak (per-face)
        if let Some((m, t)) = build_per_face_cube_model(
            &mut rl,
            &thread,
            &["assets/blocks/log_oak_top.png"],
            &["assets/blocks/log_oak.png"],
            &["assets/blocks/log_oak_top.png"],
            false,
        ) {
            model_logoak = Some(m);
            _tex_logoak = Some(t);
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
            let sand_color = Color::new(218, 210, 158, 255);
            let snow_color = Color::new(240, 247, 255, 255);
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
                                Block::Sand => {
                                    if let Some(m) = model_sand.as_ref() {
                                        if wireframe { d3.draw_model_wires(m, pos, 1.0, Color::WHITE); }
                                        else { d3.draw_model(m, pos, 1.0, Color::WHITE); }
                                    } else {
                                        if wireframe { d3.draw_cube_wires(pos, 1.0, 1.0, 1.0, sand_color); }
                                        else { d3.draw_cube(pos, 1.0, 1.0, 1.0, sand_color); }
                                    }
                                }
                                Block::Snow => {
                                    if let Some(m) = model_snow.as_ref() {
                                        if wireframe { d3.draw_model_wires(m, pos, 1.0, Color::WHITE); }
                                        else { d3.draw_model(m, pos, 1.0, Color::WHITE); }
                                    } else {
                                        if wireframe { d3.draw_cube_wires(pos, 1.0, 1.0, 1.0, snow_color); }
                                        else { d3.draw_cube(pos, 1.0, 1.0, 1.0, snow_color); }
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

// Build a cube model with top/side/bottom textures; optional vertical flip for side faces
fn build_per_face_cube_model(
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    top_paths: &[&str],
    side_paths: &[&str],
    bottom_paths: &[&str],
    flip_side_v: bool,
) -> Option<(raylib::core::models::Model, raylib::core::texture::Texture2D)> {
    let load_first = |cands: &[&str]| -> Option<Image> {
        for p in cands { if let Ok(img) = Image::load_image(p) { return Some(img); } }
        None
    };
    let top = load_first(top_paths)?;
    let side = load_first(side_paths)?;
    let bottom = load_first(bottom_paths)?;

    let (tw, th) = (top.width(), top.height());
    let mut atlas = Image::gen_image_color(tw * 3, th, Color::BLANK);
    atlas.draw(&top, Rectangle::new(0.0, 0.0, tw as f32, th as f32), Rectangle::new(0.0, 0.0, tw as f32, th as f32), Color::WHITE);
    atlas.draw(&side, Rectangle::new(0.0, 0.0, side.width() as f32, side.height() as f32), Rectangle::new(tw as f32, 0.0, tw as f32, th as f32), Color::WHITE);
    atlas.draw(&bottom, Rectangle::new(0.0, 0.0, bottom.width() as f32, bottom.height() as f32), Rectangle::new((tw * 2) as f32, 0.0, tw as f32, th as f32), Color::WHITE);

    let tex = rl.load_texture_from_image(thread, &atlas).ok()?;
    tex.set_texture_filter(thread, raylib::consts::TextureFilter::TEXTURE_FILTER_POINT);

    let mut mesh = raylib::core::models::Mesh::gen_mesh_cube(thread, 1.0, 1.0, 1.0);
    let vc = mesh.as_ref().vertexCount as usize;
    let src_uvs: &[f32] = unsafe { std::slice::from_raw_parts(mesh.as_ref().texcoords as *const f32, vc * 2) };
    let mut uvs = src_uvs.to_vec();

    // Face order: 0=front,1=back,2=top,3=bottom,4=right,5=left
    let tiles = 3.0f32;
    let tile_scale = 1.0 / tiles;
    let face_to_tile = [1.0, 1.0, 0.0, 2.0, 1.0, 1.0];
    for face in 0..6 {
        let u_off = face_to_tile[face] * tile_scale;
        let is_side = face_to_tile[face] == 1.0;
        for i in 0..4 {
            let idx = face * 8 + i * 2;
            let u = uvs[idx];
            let mut v = uvs[idx + 1];
            if is_side && flip_side_v { v = 1.0 - v; }
            uvs[idx] = u * tile_scale + u_off;
            uvs[idx + 1] = v;
        }
    }

    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(uvs.as_ptr() as *const u8, uvs.len() * std::mem::size_of::<f32>()) };
    unsafe { mesh.update_buffer::<f32>(1, bytes, 0) };

    let model = rl.load_model_from_mesh(thread, unsafe { mesh.make_weak() }).ok()?;
    let mut model = model;
    if let Some(mat) = model.materials_mut().get_mut(0) {
        mat.set_material_texture(raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO, &tex);
    }
    Some((model, tex))
}

// Helper to build a simple cube model with a uniform texture on all faces
fn build_uniform_cube_model(
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    tex_candidates: &[&str],
) -> Option<(raylib::core::models::Model, raylib::core::texture::Texture2D)> {
    let mut tex_opt = None;
    for p in tex_candidates {
        if let Ok(t) = rl.load_texture(thread, p) { tex_opt = Some(t); break; }
    }
    let tex = tex_opt?;
    tex.set_texture_filter(thread, raylib::consts::TextureFilter::TEXTURE_FILTER_POINT);
    let cube_mesh = raylib::core::models::Mesh::gen_mesh_cube(thread, 1.0, 1.0, 1.0);
    let model = rl.load_model_from_mesh(thread, unsafe { cube_mesh.make_weak() }).ok()?;
    let mut model = model; // mutable to set material
    if let Some(mat) = model.materials_mut().get_mut(0) {
        mat.set_material_texture(raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO, &tex);
    }
    Some((model, tex))
}

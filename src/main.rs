mod camera;
mod voxel;
mod mesher;

use camera::FlyCamera;
use raylib::prelude::*;
use raylib::core::texture::Image;
use voxel::{World, Block};
use mesher::build_chunk_greedy;
use std::path::Path;
use std::collections::{HashMap, HashSet};

fn main() {
    let (mut rl, thread) = raylib::init()
        .size(1280, 720)
        .title("Geist Voxel View (Rust)")
        .msaa_4x()
        .build();

    rl.set_target_fps(60);
    rl.disable_cursor();

    // Build a multi-chunk world
    let chunk_size_x = 32usize;
    let chunk_size_y = 48usize;
    let chunk_size_z = 32usize;
    let chunks_x = 4usize;
    let chunks_z = 4usize;
    let world_seed = 1337;
    let world = World::new(chunks_x, chunks_z, chunk_size_x, chunk_size_y, chunk_size_z, world_seed);

    // Place camera to see the scene
    let mut cam = FlyCamera::new(Vector3::new(
        (world.world_size_x() as f32) * 0.5,
        (chunk_size_y as f32) * 0.8,
        (world.world_size_z() as f32) * 0.5 + 20.0,
    ));

    // Rendering options
    let mut show_grid = true;
    let mut wireframe = false;

    // Ensure assets dir exists (mesher will load textures directly)
    let _assets_dir = Path::new("assets");

    // Streaming chunk state
    let mut loaded: HashMap<(i32,i32), mesher::ChunkRender> = HashMap::new();
    let view_radius_chunks: i32 = 3;
    let mut last_center_chunk: (i32, i32) = (i32::MIN, i32::MIN);

    while !rl.window_should_close() {
        let dt = rl.get_frame_time();
        cam.update(&mut rl, dt);

        if rl.is_key_pressed(KeyboardKey::KEY_G) {
            show_grid = !show_grid;
        }
        if rl.is_key_pressed(KeyboardKey::KEY_F) {
            wireframe = !wireframe;
        }

        // Update streaming set based on camera position
        let cam_pos = cam.position;
        let ccx = (cam_pos.x / chunk_size_x as f32).floor() as i32;
        let ccz = (cam_pos.z / chunk_size_z as f32).floor() as i32;
            if (ccx, ccz) != last_center_chunk {
                last_center_chunk = (ccx, ccz);
                let mut desired: HashSet<(i32,i32)> = HashSet::new();
                for dz in -view_radius_chunks..=view_radius_chunks {
                    for dx in -view_radius_chunks..=view_radius_chunks {
                        desired.insert((ccx + dx, ccz + dz));
                    }
                }
                // Unload far chunks
                let current_keys: Vec<(i32,i32)> = loaded.keys().cloned().collect();
                for key in current_keys {
                    if !desired.contains(&key) {
                        loaded.remove(&key);
                    }
                }
                // Load new chunks
                for key in desired {
                    if !loaded.contains_key(&key) {
                        let (cx, cz) = key;
                        if let Some(cr) = build_chunk_greedy(&world, cx, cz, &mut rl, &thread) {
                            loaded.insert(key, cr);
                        }
                    }
                }
            }

        let mut d = rl.begin_drawing(&thread);
        d.clear_background(Color::new(210, 221, 235, 255));

        let camera3d = cam.to_camera3d();
        {
            let mut d3 = d.begin_mode3D(camera3d);
            if show_grid {
                d3.draw_grid(64, 1.0);
            }

            // Draw loaded chunks
            for (_key, cr) in &loaded {
                for (_fm, model, _tex) in &cr.parts {
                    if wireframe { d3.draw_model_wires(model, Vector3::zero(), 1.0, Color::WHITE); }
                    else { d3.draw_model(model, Vector3::zero(), 1.0, Color::WHITE); }
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

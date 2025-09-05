mod camera;
mod voxel;
mod mesher;
mod shaders;
mod lighting;
mod chunkbuf;
mod player;

use camera::FlyCamera;
use raylib::prelude::*;
use raylib::core::texture::Image;
use voxel::World;
use mesher::{FaceMaterial, build_chunk_greedy_cpu_buf, upload_chunk_mesh, ChunkMeshCPU, TextureCache};
use voxel::TreeSpecies;
use lighting::LightingStore;
// Frustum culling removed for stability
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
    use std::sync::{Arc, mpsc};
    use std::thread;
    let world = Arc::new(World::new(chunks_x, chunks_z, chunk_size_x, chunk_size_y, chunk_size_z, world_seed));
    let lighting_store = Arc::new(LightingStore::new(chunk_size_x, chunk_size_y, chunk_size_z));

    // Place camera/player
    let spawn = Vector3::new(
        (world.world_size_x() as f32) * 0.5,
        (chunk_size_y as f32) as f32,
        (world.world_size_z() as f32) * 0.5,
    );
    let mut cam = FlyCamera::new(spawn + Vector3::new(0.0, 5.0, 20.0));
    let mut walker = player::Walker::new(Vector3::new(spawn.x, (chunk_size_y as f32) * 0.8, spawn.z));
    let mut walk_mode = true; // start in walk mode

    // Rendering options
    let mut show_grid = true;
    let mut wireframe = false;
    let mut show_chunk_bounds = false;

    // Ensure assets dir exists (mesher will load textures directly)
    let _assets_dir = Path::new("assets");

    // Streaming chunk state
    let mut loaded: HashMap<(i32,i32), mesher::ChunkRender> = HashMap::new();
    let mut loaded_bufs: HashMap<(i32,i32), chunkbuf::ChunkBuf> = HashMap::new();
    let mut pending: HashSet<(i32,i32)> = HashSet::new();
    let view_radius_chunks: i32 = 6;
    let mut last_center_chunk: (i32, i32) = (i32::MIN, i32::MIN);

    // Mesh worker threads
    let (job_tx, job_rx) = mpsc::channel::<(i32,i32)>();
    struct JobOut { cpu: ChunkMeshCPU, buf: chunkbuf::ChunkBuf }
    let (res_tx, res_rx) = mpsc::channel::<JobOut>();
    let worker_count: usize = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);
    // Per‑worker channels
    let mut worker_txs: Vec<mpsc::Sender<(i32,i32)>> = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let (wtx, wrx) = mpsc::channel::<(i32,i32)>();
        worker_txs.push(wtx);
        let tx = res_tx.clone();
        let w = world.clone();
        let ls = lighting_store.clone();
        thread::spawn(move || {
            while let Ok((cx, cz)) = wrx.recv() {
                let buf = chunkbuf::generate_chunk_buffer(&w, cx, cz);
                if let Some(cpu) = build_chunk_greedy_cpu_buf(&buf, Some(&ls), cx, cz) {
                    let _ = tx.send(JobOut { cpu, buf });
                }
            }
        });
    }
    // Dispatcher to round‑robin jobs across workers
    {
        let worker_txs = worker_txs.clone();
        thread::spawn(move || {
            let mut i = 0usize;
            while let Ok(job) = job_rx.recv() {
                if !worker_txs.is_empty() {
                    let _ = worker_txs[i % worker_txs.len()].send(job);
                    i = i.wrapping_add(1);
                }
            }
        });
    }

    // Fog shaders
    let mut leaves_shader = shaders::LeavesShader::load(&mut rl, &thread);
    let mut fog_shader = shaders::FogShader::load(&mut rl, &thread);
    // Texture cache
    let mut tex_cache = TextureCache::new();
    // Preload all textures used by face materials to avoid first-use hitches
    let species = [
        TreeSpecies::Oak,
        TreeSpecies::Birch,
        TreeSpecies::Spruce,
        TreeSpecies::Jungle,
        TreeSpecies::Acacia,
        TreeSpecies::DarkOak,
    ];
    let mut mats: Vec<FaceMaterial> = vec![
        FaceMaterial::GrassTop,
        FaceMaterial::GrassSide,
        FaceMaterial::Dirt,
        FaceMaterial::Stone,
        FaceMaterial::Sand,
        FaceMaterial::Snow,
        FaceMaterial::Glowstone,
    ];
    for sp in species.iter().copied() {
        mats.push(FaceMaterial::WoodTop(sp));
        mats.push(FaceMaterial::WoodSide(sp));
        mats.push(FaceMaterial::Leaves(sp));
    }
    for fm in &mats {
        let _ = tex_cache.get_or_load(&mut rl, &thread, &fm.texture_candidates());
    }

    while !rl.window_should_close() {
        let dt = rl.get_frame_time();
        // Toggle walk/fly
        if rl.is_key_pressed(KeyboardKey::KEY_V) { walk_mode = !walk_mode; }
        if walk_mode { cam.update_look_only(&mut rl, dt); } else { cam.update(&mut rl, dt); }

        if rl.is_key_pressed(KeyboardKey::KEY_G) {
            show_grid = !show_grid;
        }
        if rl.is_key_pressed(KeyboardKey::KEY_F) {
            wireframe = !wireframe;
        }
        if rl.is_key_pressed(KeyboardKey::KEY_B) {
            show_chunk_bounds = !show_chunk_bounds;
        }
        // Place/remove dynamic emitter at a point in front of the camera
        if rl.is_key_pressed(KeyboardKey::KEY_L) {
            let fwd = cam.forward();
            let p = cam.position + fwd * 4.0;
            let wx = p.x.floor() as i32; let wy = p.y.floor() as i32; let wz = p.z.floor() as i32;
            lighting_store.add_emitter_world(wx, wy, wz, 255);
            let cx = wx.div_euclid(chunk_size_x as i32); let cz = wz.div_euclid(chunk_size_z as i32);
            if !pending.contains(&(cx,cz)) { let _ = job_tx.send((cx,cz)); pending.insert((cx,cz)); }
        }
        if rl.is_key_pressed(KeyboardKey::KEY_K) {
            let fwd = cam.forward();
            let p = cam.position + fwd * 4.0;
            let wx = p.x.floor() as i32; let wy = p.y.floor() as i32; let wz = p.z.floor() as i32;
            lighting_store.remove_emitter_world(wx, wy, wz);
            let cx = wx.div_euclid(chunk_size_x as i32); let cz = wz.div_euclid(chunk_size_z as i32);
            if !pending.contains(&(cx,cz)) { let _ = job_tx.send((cx,cz)); pending.insert((cx,cz)); }
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
                        loaded_bufs.remove(&key);
                    }
                }
                // Cancel pending for far chunks
                pending.retain(|k| desired.contains(k));
                // Load new chunks
                for key in desired {
                    if !loaded.contains_key(&key) && !pending.contains(&key) {
                        let _ = job_tx.send(key);
                        pending.insert(key);
                    }
                }
            }

        // Drain completed meshes (upload to GPU) before drawing
        for out in res_rx.try_iter() {
            let cpu = out.cpu;
            let key = (cpu.cx, cpu.cz);
            if let Some(cr_loaded_read) = loaded.get(&key) {
                // Decide if we can color-only update: geometry and part set must match
                let mut ok = true;
                // Check GPU vs CPU vertex counts for each existing part
                for (fm, model) in &cr_loaded_read.parts {
                    match cpu.parts.get(fm) {
                        Some(mb) => {
                            if let Some(mesh) = model.meshes().get(0) {
                                let gpu_v = mesh.as_ref().vertexCount as usize;
                                let cpu_v = mb.vertex_count();
                                if gpu_v != cpu_v { ok = false; break; }
                            } else { ok = false; break; }
                        }
                        None => { ok = false; break; }
                    }
                }
                // Ensure no extra CPU parts appear
                if ok {
                    for fm in cpu.parts.keys() {
                        if !cr_loaded_read.parts.iter().any(|(f,_)| f == fm) { ok = false; break; }
                    }
                }

                if ok {
                    // Do color-only updates now with a mutable borrow
                    if let Some(cr_loaded) = loaded.get_mut(&key) {
                        for (fm, model) in &mut cr_loaded.parts {
                            if let Some(mb) = cpu.parts.get(fm) {
                                let colors: &[u8] = mb.colors();
                                if let Some(mesh) = model.meshes_mut().get_mut(0) {
                                    unsafe { mesh.update_buffer::<u8>(3, colors, 0); }
                                }
                            }
                        }
                    }
                } else {
                    // Geometry changed or part set differs: rebuild the GPU mesh for this chunk
                    if let Some(mut cr) = upload_chunk_mesh(&mut rl, &thread, cpu, &mut tex_cache) {
                        // Assign shaders to materials (leaves vs fog)
                        for (fm, model) in &mut cr.parts {
                            if let Some(mat) = model.materials_mut().get_mut(0) {
                                match fm {
                                    FaceMaterial::Leaves(_) => {
                                        if let Some(ref ls) = leaves_shader {
                                            let dest = mat.shader_mut();
                                            let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                            let src_ptr: *const raylib::ffi::Shader = ls.shader.as_ref();
                                            unsafe { std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1); }
                                        }
                                    }
                                    _ => {
                                        if let Some(ref fs) = fog_shader {
                                            let dest = mat.shader_mut();
                                            let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                            let src_ptr: *const raylib::ffi::Shader = fs.shader.as_ref();
                                            unsafe { std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1); }
                                        }
                                    }
                                }
                            }
                        }
                        loaded.insert(key, cr);
                    }
                }
            } else if let Some(mut cr) = upload_chunk_mesh(&mut rl, &thread, cpu, &mut tex_cache) {
                // Assign shaders to materials (leaves vs fog)
                for (fm, model) in &mut cr.parts {
                    if let Some(mat) = model.materials_mut().get_mut(0) {
                        match fm {
                            FaceMaterial::Leaves(_) => {
                                if let Some(ref ls) = leaves_shader {
                                    let dest = mat.shader_mut();
                                    let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                    let src_ptr: *const raylib::ffi::Shader = ls.shader.as_ref();
                                    unsafe { std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1); }
                                }
                            }
                            _ => {
                                if let Some(ref fs) = fog_shader {
                                    let dest = mat.shader_mut();
                                    let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                    let src_ptr: *const raylib::ffi::Shader = fs.shader.as_ref();
                                    unsafe { std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1); }
                                }
                            }
                        }
                    }
                }
                loaded.insert(key, cr);
            }
            // Track buffer for this chunk
            loaded_bufs.insert(key, out.buf);
            // Requeue neighbors to converge lighting across borders
            let neighbors = [(key.0-1,key.1),(key.0+1,key.1),(key.0,key.1-1),(key.0,key.1+1)];
            pending.remove(&key);
            for nk in neighbors.iter() {
                if !loaded.contains_key(nk) && !pending.contains(nk) { continue; }
                if !pending.contains(nk) { let _ = job_tx.send(*nk); pending.insert(*nk); }
            }
        }

        // Player/walker update and camera follow (tight coupling: collide only with loaded buffers)
        if walk_mode {
            let sx = chunk_size_x as i32; let sz = chunk_size_z as i32;
            let sampler = |wx: i32, wy: i32, wz: i32| -> voxel::Block {
                let cx = wx.div_euclid(sx); let cz = wz.div_euclid(sz);
                if let Some(buf) = loaded_bufs.get(&(cx,cz)) { buf.get_world(wx, wy, wz).unwrap_or(voxel::Block::Air) } else { voxel::Block::Air }
            };
            walker.update_with_sampler(&mut rl, &sampler, &world, dt, cam.yaw);
            cam.position = walker.eye_position();
        }

        // Prepare camera for drawing
        let camera3d = cam.to_camera3d();

        let mut d = rl.begin_drawing(&thread);
        d.clear_background(Color::new(210, 221, 235, 255));

        {
            let mut d3 = d.begin_mode3D(camera3d);
            if show_grid {
                d3.draw_grid(64, 1.0);
            }

            // Update leaves shader uniforms once per frame
            // Depth-based fog color: white above ground, black underground
            let surface_fog = [210.0/255.0, 221.0/255.0, 235.0/255.0];
            let cave_fog = [0.0, 0.0, 0.0];
            let world_h = world.world_size_y() as f32;
            let underground_thr = 0.30_f32 * world_h; // simple depth cutoff
            let underground = cam.position.y < underground_thr;
            let fog_color = if underground { cave_fog } else { surface_fog };
            if let Some(ref mut ls) = leaves_shader {
                let fog_start = 64.0f32;
                let fog_end = 180.0f32;
                ls.update_frame_uniforms(cam.position, fog_color, fog_start, fog_end);
            }
            if let Some(ref mut fs) = fog_shader {
                let fog_start = 64.0f32;
                let fog_end = 180.0f32;
                fs.update_frame_uniforms(cam.position, fog_color, fog_start, fog_end);
            }

            // Draw loaded chunks (no frustum culling)
            for (_key, cr) in &loaded {
                for (_fm, model) in &cr.parts {
                    if wireframe { d3.draw_model_wires(model, Vector3::zero(), 1.0, Color::WHITE); }
                    else { d3.draw_model(model, Vector3::zero(), 1.0, Color::WHITE); }
                }
            }

            // Optional: show chunk bounding boxes to debug seams
            if show_chunk_bounds {
                let col = Color::new(255, 64, 32, 200);
                for (_key, cr) in &loaded {
                    let min = cr.bbox.min;
                    let max = cr.bbox.max;
                    let center = Vector3::new(
                        (min.x + max.x) * 0.5,
                        (min.y + max.y) * 0.5,
                        (min.z + max.z) * 0.5,
                    );
                    let size = Vector3::new(
                        (max.x - min.x).abs(),
                        (max.y - min.y).abs(),
                        (max.z - min.z).abs(),
                    );
                    d3.draw_cube_wires(center, size.x, size.y, size.z, col);
                }
            }
        }

        let hud = if walk_mode {
            "Walk: Tab capture, WASD move, Space jump, Shift run, V toggle fly, F wireframe, G grid, B chunk bounds, L add light, K remove light"
        } else {
            "Fly: Tab capture, WASD+QE move, V toggle walk, F wireframe, G grid, B chunk bounds, L add light, K remove light"
        };
        d.draw_text(hud, 12, 12, 18, Color::DARKGRAY);
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

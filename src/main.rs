mod camera;
mod voxel;
mod mesher;
mod shaders;
mod lighting;
mod chunkbuf;
mod player;
mod raycast;
mod edit;

use camera::FlyCamera;
use raylib::prelude::*;
use voxel::World;
use mesher::{FaceMaterial, build_chunk_greedy_cpu_buf, upload_chunk_mesh, ChunkMeshCPU, TextureCache, NeighborsLoaded};
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
    let edit_store = Arc::new(edit::EditStore::new(chunk_size_x as i32, chunk_size_y as i32, chunk_size_z as i32));

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
    #[derive(Clone, Copy, Debug)]
    struct BuildJob { cx: i32, cz: i32, neighbors: NeighborsLoaded, rev: u64 }
    let (job_tx, job_rx) = mpsc::channel::<BuildJob>();
    struct JobOut { cpu: ChunkMeshCPU, buf: chunkbuf::ChunkBuf, cx: i32, cz: i32, rev: u64 }
    let (res_tx, res_rx) = mpsc::channel::<JobOut>();
    let worker_count: usize = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);
    // Per‑worker channels
    let mut worker_txs: Vec<mpsc::Sender<BuildJob>> = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let (wtx, wrx) = mpsc::channel::<BuildJob>();
        worker_txs.push(wtx);
        let tx = res_tx.clone();
        let w = world.clone();
        let ls = lighting_store.clone();
        let edits = edit_store.clone();
        thread::spawn(move || {
            while let Ok(job) = wrx.recv() {
                // Skip if this exact job is outdated
                let current_rev = edits.get_rev(job.cx, job.cz);
                if job.rev > 0 && job.rev < current_rev { continue; }  // Skip outdated jobs
                let mut buf = chunkbuf::generate_chunk_buffer(&w, job.cx, job.cz);
                // Apply persistent edits for this chunk before meshing
                let base_x = job.cx * buf.sx as i32; let base_z = job.cz * buf.sz as i32;
                let edits_chunk = edits.snapshot_for_chunk(job.cx, job.cz);
                for ((wx,wy,wz), b) in edits_chunk {
                    if wy < 0 || wy >= buf.sy as i32 { continue; }
                    let lx = (wx - base_x) as usize; let ly = wy as usize; let lz = (wz - base_z) as usize;
                    if lx < buf.sx && lz < buf.sz {
                        let idx = buf.idx(lx,ly,lz);
                        buf.blocks[idx] = b;
                    }
                }
                // Take a consistent edits snapshot for this chunk and its immediate neighbors for border occlusion
                let snap_vec = edits.snapshot_for_region(job.cx, job.cz, 1);
                let snap_map: std::collections::HashMap<(i32,i32,i32), voxel::Block> = snap_vec.into_iter().collect();
                if let Some(cpu) = build_chunk_greedy_cpu_buf(&buf, Some(&ls), &w, Some(&snap_map), job.neighbors, job.cx, job.cz) {
                    let _ = tx.send(JobOut { cpu, buf, cx: job.cx, cz: job.cz, rev: job.rev });
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
    // Current placement block type (number keys change this)
    let mut place_type: voxel::Block = voxel::Block::Stone;
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
        FaceMaterial::Beacon,
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
        // Placement block selection (number keys)
        if rl.is_key_pressed(KeyboardKey::KEY_ONE) { place_type = voxel::Block::Dirt; }
        if rl.is_key_pressed(KeyboardKey::KEY_TWO) { place_type = voxel::Block::Stone; }
        if rl.is_key_pressed(KeyboardKey::KEY_THREE) { place_type = voxel::Block::Sand; }
        if rl.is_key_pressed(KeyboardKey::KEY_FOUR) { place_type = voxel::Block::Grass; }
        if rl.is_key_pressed(KeyboardKey::KEY_FIVE) { place_type = voxel::Block::Snow; }
        if rl.is_key_pressed(KeyboardKey::KEY_SIX) { place_type = voxel::Block::Glowstone; }
        if rl.is_key_pressed(KeyboardKey::KEY_SEVEN) { place_type = voxel::Block::Beacon; }
        // Place/remove dynamic emitter at a point in front of the camera
        if rl.is_key_pressed(KeyboardKey::KEY_L) {
            let fwd = cam.forward();
            let p = cam.position + fwd * 4.0;
            let wx = p.x.floor() as i32; let wy = p.y.floor() as i32; let wz = p.z.floor() as i32;
            lighting_store.add_emitter_world(wx, wy, wz, 255);
            let cx = wx.div_euclid(chunk_size_x as i32); let cz = wz.div_euclid(chunk_size_z as i32);
            if !pending.contains(&(cx,cz)) {
                let nmask = NeighborsLoaded {
                    neg_x: loaded.contains_key(&(cx-1,cz)),
                    pos_x: loaded.contains_key(&(cx+1,cz)),
                    neg_z: loaded.contains_key(&(cx,cz-1)),
                    pos_z: loaded.contains_key(&(cx,cz+1)),
                };
                let rev = edit_store.get_rev(cx, cz);
                let _ = job_tx.send(BuildJob { cx, cz, neighbors: nmask, rev });
                pending.insert((cx,cz));
            }
        }
        if rl.is_key_pressed(KeyboardKey::KEY_K) {
            let fwd = cam.forward();
            let p = cam.position + fwd * 4.0;
            let wx = p.x.floor() as i32; let wy = p.y.floor() as i32; let wz = p.z.floor() as i32;
            lighting_store.remove_emitter_world(wx, wy, wz);
            let cx = wx.div_euclid(chunk_size_x as i32); let cz = wz.div_euclid(chunk_size_z as i32);
            if !pending.contains(&(cx,cz)) {
                let nmask = NeighborsLoaded {
                    neg_x: loaded.contains_key(&(cx-1,cz)),
                    pos_x: loaded.contains_key(&(cx+1,cz)),
                    neg_z: loaded.contains_key(&(cx,cz-1)),
                    pos_z: loaded.contains_key(&(cx,cz+1)),
                };
                let rev = edit_store.get_rev(cx, cz);
                let _ = job_tx.send(BuildJob { cx, cz, neighbors: nmask, rev });
                pending.insert((cx,cz));
            }
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
                        let (cx, cz) = key;
                        let nmask = NeighborsLoaded {
                            neg_x: loaded.contains_key(&(cx-1,cz)),
                            pos_x: loaded.contains_key(&(cx+1,cz)),
                            neg_z: loaded.contains_key(&(cx,cz-1)),
                            pos_z: loaded.contains_key(&(cx,cz+1)),
                        };
                        let rev = edit_store.get_rev(cx, cz);
                        let _ = job_tx.send(BuildJob { cx, cz, neighbors: nmask, rev });
                        pending.insert(key);
                    }
                }
            }

        // Drain completed meshes (upload to GPU) before drawing
        for out in res_rx.try_iter() {
            let borders_changed = out.cpu.borders_changed;
            let cpu = out.cpu;
            let key = (out.cx, out.cz);
            // Check if this result is still valid
            let cur_rev = edit_store.get_rev(key.0, key.1);
            if out.rev < cur_rev {
                // A newer revision exists, re-enqueue
                pending.remove(&key);
                let nmask = NeighborsLoaded {
                    neg_x: loaded.contains_key(&(key.0-1,key.1)),
                    pos_x: loaded.contains_key(&(key.0+1,key.1)),
                    neg_z: loaded.contains_key(&(key.0,key.1-1)),
                    pos_z: loaded.contains_key(&(key.0,key.1+1)),
                };
                let _ = job_tx.send(BuildJob { cx: key.0, cz: key.1, neighbors: nmask, rev: cur_rev });
                pending.insert(key);
                continue;
            }
            // Always rebuild GPU geometry for simplicity and correctness with edits
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
            // Track buffer and mark built revision for this chunk
            loaded_bufs.insert(key, out.buf);
            edit_store.mark_built(key.0, key.1, out.rev);
            pending.remove(&key);
            
            // Check if any neighbors need rebuilding due to version mismatch
            // This ensures neighbors are rebuilt when edits affect boundaries
            let neighbors = [(key.0-1,key.1),(key.0+1,key.1),(key.0,key.1-1),(key.0,key.1+1)];
            for nk in neighbors.iter() {
                if !loaded.contains_key(nk) { continue; }
                // Check if neighbor needs rebuild due to version mismatch
                if edit_store.needs_rebuild(nk.0, nk.1) && !pending.contains(nk) {
                    let (cx, cz) = *nk;
                    let nmask = NeighborsLoaded {
                        neg_x: loaded.contains_key(&(cx-1,cz)),
                        pos_x: loaded.contains_key(&(cx+1,cz)),
                        neg_z: loaded.contains_key(&(cx,cz-1)),
                        pos_z: loaded.contains_key(&(cx,cz+1)),
                    };
                    let rev = edit_store.get_rev(cx, cz);
                    let _ = job_tx.send(BuildJob { cx, cz, neighbors: nmask, rev });
                    pending.insert(*nk);
                }
            }
            
            // Also requeue neighbors if lighting borders changed
            if borders_changed {
                for nk in neighbors.iter() {
                    if !loaded.contains_key(nk) || pending.contains(nk) { continue; }
                    let (cx, cz) = *nk;
                    let nmask = NeighborsLoaded {
                        neg_x: loaded.contains_key(&(cx-1,cz)),
                        pos_x: loaded.contains_key(&(cx+1,cz)),
                        neg_z: loaded.contains_key(&(cx,cz-1)),
                        pos_z: loaded.contains_key(&(cx,cz+1)),
                    };
                    let rev = edit_store.get_rev(cx, cz);
                    let _ = job_tx.send(BuildJob { cx, cz, neighbors: nmask, rev });
                    pending.insert(*nk);
                }
            }
        }

        // Handle block edits (remove/place) via mouse buttons
        {
            let org = cam.position;
            let dir = cam.forward();
            let sx = chunk_size_x as i32; let sz = chunk_size_z as i32;
            let sampler = |wx: i32, wy: i32, wz: i32| -> voxel::Block {
                if let Some(b) = edit_store.get(wx, wy, wz) { return b; }
                let cx = wx.div_euclid(sx); let cz = wz.div_euclid(sz);
                if let Some(buf) = loaded_bufs.get(&(cx,cz)) { buf.get_world(wx, wy, wz).unwrap_or(voxel::Block::Air) }
                else { world.block_at(wx, wy, wz) }
            };
            let is_solid = |wx: i32, wy: i32, wz: i32| -> bool { sampler(wx,wy,wz).is_solid() };
            let ray_hit = rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) || rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT);
            if ray_hit {
                if let Some(hit) = raycast::raycast_first_hit_with_face(org, dir, 5.0, is_solid) {
                    // Helper to enqueue rebuild for a chunk
                    let mut enqueue = |cx: i32, cz: i32| {
                        if !pending.contains(&(cx,cz)) {
                            let nmask = NeighborsLoaded {
                                neg_x: loaded.contains_key(&(cx-1,cz)),
                                pos_x: loaded.contains_key(&(cx+1,cz)),
                                neg_z: loaded.contains_key(&(cx,cz-1)),
                                pos_z: loaded.contains_key(&(cx,cz+1)),
                            };
                            let rev = edit_store.get_rev(cx, cz);
                            let _ = job_tx.send(BuildJob { cx, cz, neighbors: nmask, rev });
                            pending.insert((cx,cz));
                        }
                    };
                    if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                        // Remove at hit block
                        let wx = hit.bx; let wy = hit.by; let wz = hit.bz;
                        let prev = sampler(wx,wy,wz);
                        if prev.is_solid() {
                            edit_store.set(wx, wy, wz, voxel::Block::Air);
                            if prev.emission() > 0 { 
                                lighting_store.remove_emitter_world(wx, wy, wz); 
                            }
                            // Bump change revision and get all affected chunks
                            let _ = edit_store.bump_region_around(wx, wz);
                            let affected = edit_store.get_affected_chunks(wx, wz);
                            // Enqueue all affected chunks for rebuild
                            for (cx, cz) in affected {
                                if loaded.contains_key(&(cx, cz)) {
                                    enqueue(cx, cz);
                                }
                            }
                        }
                    }
                    if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT) {
                        // Place at previous air voxel
                        let wx = hit.px; let wy = hit.py; let wz = hit.pz;
                        if wy >= 0 && wy < chunk_size_y as i32 {
                            edit_store.set(wx, wy, wz, place_type);
                            if place_type.emission() > 0 { 
                                if matches!(place_type, voxel::Block::Beacon) {
                                    lighting_store.add_beacon_world(wx, wy, wz, place_type.emission());
                                } else {
                                    lighting_store.add_emitter_world(wx, wy, wz, place_type.emission());
                                }
                            }
                            let _ = edit_store.bump_region_around(wx, wz);
                            let ccx = wx.div_euclid(sx); let ccz = wz.div_euclid(sz);
                            // Only enqueue the edited chunk; neighbors will follow if borders changed
                            enqueue(ccx, ccz);
                        }
                    }
                }
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

            // Placement/removal preview: face outline on targeted voxel face
            {
                let org = cam.position;
                let dir = cam.forward();
                let max_dist = 5.0_f32;
                // Sampler considers edits first, then loaded buffers, then generated world
                let sx = chunk_size_x as i32; let sz = chunk_size_z as i32;
                let sampler = |wx: i32, wy: i32, wz: i32| -> voxel::Block {
                    if let Some(b) = edit_store.get(wx, wy, wz) { return b; }
                    let cx = wx.div_euclid(sx); let cz = wz.div_euclid(sz);
                    if let Some(buf) = loaded_bufs.get(&(cx,cz)) { buf.get_world(wx, wy, wz).unwrap_or(voxel::Block::Air) }
                    else { world.block_at(wx, wy, wz) }
                };
                let is_solid = |wx: i32, wy: i32, wz: i32| -> bool { sampler(wx,wy,wz).is_solid() };
                if let Some(hit) = raycast::raycast_first_hit_with_face(org, dir, max_dist, is_solid) {
                    let eps = 0.01_f32;
                    let c = Color::WHITE;
                    if hit.nx != 0 {
                        let xp = (hit.bx as f32) + if hit.nx > 0 { 1.0 + eps } else { -eps };
                        let y0 = hit.by as f32; let y1 = y0 + 1.0;
                        let z0 = hit.bz as f32; let z1 = z0 + 1.0;
                        let p1 = Vector3::new(xp, y0, z0); let p2 = Vector3::new(xp, y1, z0);
                        let p3 = Vector3::new(xp, y1, z1); let p4 = Vector3::new(xp, y0, z1);
                        d3.draw_line_3D(p1, p2, c); d3.draw_line_3D(p2, p3, c);
                        d3.draw_line_3D(p3, p4, c); d3.draw_line_3D(p4, p1, c);
                    } else if hit.ny != 0 {
                        let yp = (hit.by as f32) + if hit.ny > 0 { 1.0 + eps } else { -eps };
                        let x0 = hit.bx as f32; let x1 = x0 + 1.0;
                        let z0 = hit.bz as f32; let z1 = z0 + 1.0;
                        let p1 = Vector3::new(x0, yp, z0); let p2 = Vector3::new(x1, yp, z0);
                        let p3 = Vector3::new(x1, yp, z1); let p4 = Vector3::new(x0, yp, z1);
                        d3.draw_line_3D(p1, p2, c); d3.draw_line_3D(p2, p3, c);
                        d3.draw_line_3D(p3, p4, c); d3.draw_line_3D(p4, p1, c);
                    } else if hit.nz != 0 {
                        let zp = (hit.bz as f32) + if hit.nz > 0 { 1.0 + eps } else { -eps };
                        let x0 = hit.bx as f32; let x1 = x0 + 1.0;
                        let y0 = hit.by as f32; let y1 = y0 + 1.0;
                        let p1 = Vector3::new(x0, y0, zp); let p2 = Vector3::new(x1, y0, zp);
                        let p3 = Vector3::new(x1, y1, zp); let p4 = Vector3::new(x0, y1, zp);
                        d3.draw_line_3D(p1, p2, c); d3.draw_line_3D(p2, p3, c);
                        d3.draw_line_3D(p3, p4, c); d3.draw_line_3D(p4, p1, c);
                    }
                }
            }

            // Optional: show chunk bounding boxes to debug seams and neighbor-load status
            if show_chunk_bounds {
                let col = Color::new(255, 64, 32, 200);
                for ((_key_cx, _key_cz), cr) in &loaded {
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

                    // Neighbor overlay: draw small outward lines per face (green=loaded, red=missing)
                    let cx = cr.cx; let cz = cr.cz;
                    let neg_x_loaded = loaded.contains_key(&(cx-1, cz));
                    let pos_x_loaded = loaded.contains_key(&(cx+1, cz));
                    let neg_z_loaded = loaded.contains_key(&(cx, cz-1));
                    let pos_z_loaded = loaded.contains_key(&(cx, cz+1));
                    let mid_y = center.y; // mid-height indicator
                    let len = 1.5_f32;    // arrow length outward
                    let eps = 0.06_f32;   // small offset to avoid z-fighting

                    // West (-X)
                    {
                        let c = if neg_x_loaded { Color::GREEN } else { Color::RED };
                        let start = Vector3::new(min.x - eps, mid_y, center.z);
                        let end = Vector3::new(min.x - eps - len, mid_y, center.z);
                        d3.draw_line_3D(start, end, c);
                    }
                    // East (+X)
                    {
                        let c = if pos_x_loaded { Color::GREEN } else { Color::RED };
                        let start = Vector3::new(max.x + eps, mid_y, center.z);
                        let end = Vector3::new(max.x + eps + len, mid_y, center.z);
                        d3.draw_line_3D(start, end, c);
                    }
                    // North (-Z)
                    {
                        let c = if neg_z_loaded { Color::GREEN } else { Color::RED };
                        let start = Vector3::new(center.x, mid_y, min.z - eps);
                        let end = Vector3::new(center.x, mid_y, min.z - eps - len);
                        d3.draw_line_3D(start, end, c);
                    }
                    // South (+Z)
                    {
                        let c = if pos_z_loaded { Color::GREEN } else { Color::RED };
                        let start = Vector3::new(center.x, mid_y, max.z + eps);
                        let end = Vector3::new(center.x, mid_y, max.z + eps + len);
                        d3.draw_line_3D(start, end, c);
                    }
                }
            }
        }

        // Debug overlay: current chunk coordinates and camera facing
        {
            let ccx_dbg = (cam.position.x / chunk_size_x as f32).floor() as i32;
            let ccz_dbg = (cam.position.z / chunk_size_z as f32).floor() as i32;
            let mut yawn = cam.yaw % 360.0; if yawn < 0.0 { yawn += 360.0; }
            let dir = if yawn >= 315.0 || yawn < 45.0 { "E" }
                      else if yawn < 135.0 { "S" }
                      else if yawn < 225.0 { "W" }
                      else { "N" };
            let f = cam.forward();
            let line1 = format!("Chunk: ({}, {}), Pos: ({:.1}, {:.1}, {:.1})", ccx_dbg, ccz_dbg, cam.position.x, cam.position.y, cam.position.z);
            let line2 = format!("Facing: {}  yaw={:.1}°  fwd=({:.2},{:.2},{:.2})", dir, yawn, f.x, f.y, f.z);
            // Local-in-chunk and distance to chunk edges (for seam debugging)
            let x0 = (ccx_dbg * chunk_size_x as i32) as f32;
            let z0 = (ccz_dbg * chunk_size_z as i32) as f32;
            let lx = cam.position.x - x0;
            let lz = cam.position.z - z0;
            let dx_edge = (lx.min(chunk_size_x as f32 - lx)).abs();
            let dz_edge = (lz.min(chunk_size_z as f32 - lz)).abs();
            let line3 = format!("Local: ({:.2}, {:.2})  edge_dx={:.2} edge_dz={:.2}", lx, lz, dx_edge, dz_edge);
            // Neighbor mask for current chunk (G=loaded, R=missing)
            let w_loaded = loaded.contains_key(&(ccx_dbg-1, ccz_dbg));
            let e_loaded = loaded.contains_key(&(ccx_dbg+1, ccz_dbg));
            let n_loaded = loaded.contains_key(&(ccx_dbg, ccz_dbg-1));
            let s_loaded = loaded.contains_key(&(ccx_dbg, ccz_dbg+1));
            let sym = |b: bool| if b { 'G' } else { 'R' };
            let line4 = format!("Neighbors: W={} E={} N={} S={}", sym(w_loaded), sym(e_loaded), sym(n_loaded), sym(s_loaded));
            d.draw_text(&line1, 12, 60, 18, Color::DARKGREEN);
            d.draw_text(&line2, 12, 80, 18, Color::DARKGREEN);
            d.draw_text(&line3, 12, 100, 18, Color::DARKGREEN);
            d.draw_text(&line4, 12, 120, 18, Color::DARKGREEN);
        }

        let hud_mode = if walk_mode { "Walk" } else { "Fly" };
        let hud = format!("{}: Tab capture, WASD{} move{}, V toggle mode, F wireframe, G grid, B bounds, L add light, K remove light | Place: {:?} (1-7 to select) | LMB remove, RMB place", 
                          hud_mode, if walk_mode {""} else {"+QE"}, if walk_mode {", Space jump, Shift run"} else {""}, place_type);
        d.draw_text(&hud, 12, 12, 18, Color::DARKGRAY);
        d.draw_fps(12, 36);
    }
}

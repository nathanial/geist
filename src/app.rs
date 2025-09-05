use std::collections::{HashMap, HashSet};

use raylib::prelude::*;

use crate::event::{Event, EventEnvelope, EventQueue, RebuildCause};
use crate::gamestate::{ChunkEntry, GameState};
use crate::lighting::LightingStore;
use crate::mesher::{upload_chunk_mesh, NeighborsLoaded};
use crate::player::Walker;
use crate::raycast;
use crate::runtime::{BuildJob, JobOut, Runtime};
use crate::voxel::{Block, World};

pub struct App {
    pub gs: GameState,
    pub queue: EventQueue,
    pub runtime: Runtime,
    pub cam: crate::camera::FlyCamera,
}

impl App {
    pub fn new(
        mut rl: &mut RaylibHandle,
        thread: &RaylibThread,
        world: std::sync::Arc<World>,
        lighting: std::sync::Arc<LightingStore>,
        edits: std::sync::Arc<crate::edit::EditStore>,
    ) -> Self {
        let spawn = Vector3::new(
            (world.world_size_x() as f32) * 0.5,
            (world.world_size_y() as f32) * 0.8,
            (world.world_size_z() as f32) * 0.5,
        );
        let cam = crate::camera::FlyCamera::new(spawn + Vector3::new(0.0, 5.0, 20.0));

        let runtime = Runtime::new(&mut rl, thread, world.clone(), lighting.clone(), edits.clone());
        let gs = GameState::new(world.clone(), edits.clone(), lighting.clone(), cam.position);
        let mut queue = EventQueue::new();
        // Bootstrap initial streaming based on camera
        let ccx = (cam.position.x / world.chunk_size_x as f32).floor() as i32;
        let ccz = (cam.position.z / world.chunk_size_z as f32).floor() as i32;
        queue.emit_now(Event::ViewCenterChanged { ccx, ccz });
        Self { gs, queue, runtime, cam }
    }

    fn neighbor_mask(&self, cx: i32, cz: i32) -> NeighborsLoaded {
        NeighborsLoaded {
            neg_x: self.runtime.renders.contains_key(&(cx - 1, cz)),
            pos_x: self.runtime.renders.contains_key(&(cx + 1, cz)),
            neg_z: self.runtime.renders.contains_key(&(cx, cz - 1)),
            pos_z: self.runtime.renders.contains_key(&(cx, cz + 1)),
        }
    }

    fn job_hash(cx: i32, cz: i32, rev: u64, n: NeighborsLoaded) -> u64 {
        // Simple stable hash (FNV-1a 64-bit variant)
        let mut h: u64 = 0xcbf29ce484222325;
        let mut write = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x100000001b3);
        };
        write(cx as u64 as u64);
        write(cz as u64 as u64);
        write(rev);
        let mask = (n.neg_x as u64)
            | ((n.pos_x as u64) << 1)
            | ((n.neg_z as u64) << 2)
            | ((n.pos_z as u64) << 3);
        write(mask);
        h
    }

    fn handle_event(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        env: EventEnvelope,
    ) {
        // Log a concise line for the processed event
        Self::log_event(self.gs.tick, &env.kind);
        match env.kind {
            Event::Tick => {}
            Event::MovementRequested { dt_ms, yaw, walk_mode } => {
                // update camera look first (yaw drives walker forward)
                self.gs.walk_mode = walk_mode;
                if self.gs.walk_mode {
                    // Collision sampler: edits > buf > world
                    let sx = self.gs.world.chunk_size_x as i32;
                    let sz = self.gs.world.chunk_size_z as i32;
                    let sampler = |wx: i32, wy: i32, wz: i32| -> Block {
                        if let Some(b) = self.gs.edits.get(wx, wy, wz) {
                            return b;
                        }
                        let cx = wx.div_euclid(sx);
                        let cz = wz.div_euclid(sz);
                        if let Some(cent) = self.gs.chunks.get(&(cx, cz)) {
                            if let Some(ref buf) = cent.buf {
                                return buf.get_world(wx, wy, wz).unwrap_or(Block::Air);
                            }
                        }
                        self.gs.world.block_at(wx, wy, wz)
                    };
                    self.gs.walker.update_with_sampler(
                        rl,
                        &sampler,
                        &self.gs.world,
                        (dt_ms as f32) / 1000.0,
                        yaw,
                    );
                    self.cam.position = self.gs.walker.eye_position();
                    // Emit ViewCenterChanged if center moved
                    let ccx = (self.cam.position.x / self.gs.world.chunk_size_x as f32).floor() as i32;
                    let ccz = (self.cam.position.z / self.gs.world.chunk_size_z as f32).floor() as i32;
                    if (ccx, ccz) != self.gs.center_chunk {
                        self.queue.emit_now(Event::ViewCenterChanged { ccx, ccz });
                    }
                } else {
                    // Fly camera mode moves separately; no player update
                }
            }
            Event::ViewCenterChanged { ccx, ccz } => {
                self.gs.center_chunk = (ccx, ccz);
                // Determine desired set
                let r = self.gs.view_radius_chunks;
                let mut desired: HashSet<(i32, i32)> = HashSet::new();
                for dz in -r..=r {
                    for dx in -r..=r {
                        desired.insert((ccx + dx, ccz + dz));
                    }
                }
                // Unload far ones
                let current: Vec<(i32, i32)> = self.runtime.renders.keys().cloned().collect();
                for key in current {
                    if !desired.contains(&key) {
                        self.queue.emit_now(Event::EnsureChunkUnloaded { cx: key.0, cz: key.1 });
                    }
                }
                // Cancel pending for far chunks
                self.gs.pending.retain(|k| desired.contains(k));
                // Load new ones
                for key in desired {
                    if !self.runtime.renders.contains_key(&key) && !self.gs.pending.contains(&key) {
                        self.queue.emit_now(Event::EnsureChunkLoaded { cx: key.0, cz: key.1 });
                    }
                }
            }
            Event::EnsureChunkUnloaded { cx, cz } => {
                self.runtime.renders.remove(&(cx, cz));
                self.gs.chunks.remove(&(cx, cz));
                self.gs.loaded.remove(&(cx, cz));
                self.gs.pending.remove(&(cx, cz));
            }
            Event::EnsureChunkLoaded { cx, cz } => {
                if self.runtime.renders.contains_key(&(cx, cz)) || self.gs.pending.contains(&(cx, cz)) {
                    return;
                }
                let neighbors = self.neighbor_mask(cx, cz);
                let rev = self.gs.edits.get_rev(cx, cz);
                let job_id = Self::job_hash(cx, cz, rev, neighbors);
                self.queue.emit_now(Event::BuildChunkJobRequested { cx, cz, neighbors, rev, job_id });
                self.gs.pending.insert((cx, cz));
            }
            Event::BuildChunkJobRequested { cx, cz, neighbors, rev, job_id } => {
                self.runtime.submit_build_job(BuildJob { cx, cz, neighbors, rev, job_id });
            }
            Event::BuildChunkJobCompleted { cx, cz, rev, cpu, buf, borders_changed, job_id: _ } => {
                // Drop if stale
                let cur_rev = self.gs.edits.get_rev(cx, cz);
                if rev < cur_rev {
                    // Re-enqueue latest
                    let neighbors = self.neighbor_mask(cx, cz);
                    let job_id = Self::job_hash(cx, cz, cur_rev, neighbors);
                    self.queue.emit_now(Event::BuildChunkJobRequested { cx, cz, neighbors, rev: cur_rev, job_id });
                    return;
                }
                // Upload to GPU
                if let Some(mut cr) = upload_chunk_mesh(rl, thread, cpu, &mut self.runtime.tex_cache) {
                    // Assign shaders
                    for (fm, model) in &mut cr.parts {
                        if let Some(mat) = model.materials_mut().get_mut(0) {
                            match fm {
                                crate::mesher::FaceMaterial::Leaves(_) => {
                                    if let Some(ref ls) = self.runtime.leaves_shader {
                                        let dest = mat.shader_mut();
                                        let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                        let src_ptr: *const raylib::ffi::Shader = ls.shader.as_ref();
                                        unsafe { std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1); }
                                    }
                                }
                                _ => {
                                    if let Some(ref fs) = self.runtime.fog_shader {
                                        let dest = mat.shader_mut();
                                        let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                        let src_ptr: *const raylib::ffi::Shader = fs.shader.as_ref();
                                        unsafe { std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1); }
                                    }
                                }
                            }
                        }
                    }
                    self.runtime.renders.insert((cx, cz), cr);
                }
                // Update CPU buf & built rev
                self.gs.chunks.insert((cx, cz), ChunkEntry { buf: Some(buf), built_rev: rev });
                self.gs.loaded.insert((cx, cz));
                self.gs.pending.remove(&(cx, cz));
                self.gs.edits.mark_built(cx, cz, rev);

                // Requeue neighbors if borders changed
                if borders_changed {
                    for (nx, nz) in [(cx - 1, cz), (cx + 1, cz), (cx, cz - 1), (cx, cz + 1)] {
                        if self.runtime.renders.contains_key(&(nx, nz)) && !self.gs.pending.contains(&(nx, nz)) {
                            self.queue.emit_now(Event::ChunkRebuildRequested { cx: nx, cz: nz, cause: RebuildCause::LightingBorder });
                        }
                    }
                }
            }
            Event::ChunkRebuildRequested { cx, cz, cause: _ } => {
                if !self.runtime.renders.contains_key(&(cx, cz)) || self.gs.pending.contains(&(cx, cz)) {
                    return;
                }
                let neighbors = self.neighbor_mask(cx, cz);
                let rev = self.gs.edits.get_rev(cx, cz);
                let job_id = Self::job_hash(cx, cz, rev, neighbors);
                self.queue.emit_now(Event::BuildChunkJobRequested { cx, cz, neighbors, rev, job_id });
                self.gs.pending.insert((cx, cz));
            }
            Event::RaycastEditRequested { place, block } => {
                // Perform raycast and apply edit
                let org = self.cam.position;
                let dir = self.cam.forward();
                let sx = self.gs.world.chunk_size_x as i32;
                let sz = self.gs.world.chunk_size_z as i32;
                let sampler = |wx: i32, wy: i32, wz: i32| -> Block {
                    if let Some(b) = self.gs.edits.get(wx, wy, wz) { return b; }
                    let cx = wx.div_euclid(sx);
                    let cz = wz.div_euclid(sz);
                    if let Some(cent) = self.gs.chunks.get(&(cx, cz)) {
                        if let Some(ref buf) = cent.buf {
                            return buf.get_world(wx, wy, wz).unwrap_or(Block::Air);
                        }
                    }
                    self.gs.world.block_at(wx, wy, wz)
                };
                let is_solid = |wx: i32, wy: i32, wz: i32| -> bool { sampler(wx, wy, wz).is_solid() };
                if let Some(hit) = raycast::raycast_first_hit_with_face(org, dir, 5.0, is_solid) {
                    if place {
                        let wx = hit.px; let wy = hit.py; let wz = hit.pz;
                        if wy >= 0 && wy < self.gs.world.chunk_size_y as i32 {
                            self.gs.edits.set(wx, wy, wz, block);
                            if block.emission() > 0 {
                                if matches!(block, Block::Beacon) {
                                    self.gs.lighting.add_beacon_world(wx, wy, wz, block.emission());
                                } else {
                                    self.gs.lighting.add_emitter_world(wx, wy, wz, block.emission());
                                }
                            }
                            let _ = self.gs.edits.bump_region_around(wx, wz);
                            // Rebuild edited chunk and any boundary-adjacent neighbors that are loaded
                            for (cx, cz) in self.gs.edits.get_affected_chunks(wx, wz) {
                                if self.runtime.renders.contains_key(&(cx, cz)) {
                                    self.queue.emit_now(Event::ChunkRebuildRequested { cx, cz, cause: RebuildCause::Edit });
                                }
                            }
                        }
                    } else {
                        // remove at hit
                        let wx = hit.bx; let wy = hit.by; let wz = hit.bz;
                        let prev = sampler(wx, wy, wz);
                        if prev.is_solid() {
                            self.gs.edits.set(wx, wy, wz, Block::Air);
                            if prev.emission() > 0 { self.gs.lighting.remove_emitter_world(wx, wy, wz); }
                            let _ = self.gs.edits.bump_region_around(wx, wz);
                            for (cx, cz) in self.gs.edits.get_affected_chunks(wx, wz) {
                                if self.runtime.renders.contains_key(&(cx, cz)) {
                                    self.queue.emit_now(Event::ChunkRebuildRequested { cx, cz, cause: RebuildCause::Edit });
                                }
                            }
                        }
                    }
                }
            }
            Event::LightEmitterAdded { wx, wy, wz, level, is_beacon } => {
                if is_beacon { self.gs.lighting.add_beacon_world(wx, wy, wz, level); }
                else { self.gs.lighting.add_emitter_world(wx, wy, wz, level); }
                // schedule rebuild of that chunk
                let sx = self.gs.world.chunk_size_x as i32;
                let sz = self.gs.world.chunk_size_z as i32;
                let cx = wx.div_euclid(sx);
                let cz = wz.div_euclid(sz);
                self.queue.emit_now(Event::ChunkRebuildRequested { cx, cz, cause: RebuildCause::Edit });
            }
            Event::LightEmitterRemoved { wx, wy, wz } => {
                self.gs.lighting.remove_emitter_world(wx, wy, wz);
                let sx = self.gs.world.chunk_size_x as i32;
                let sz = self.gs.world.chunk_size_z as i32;
                let cx = wx.div_euclid(sx);
                let cz = wz.div_euclid(sz);
                self.queue.emit_now(Event::ChunkRebuildRequested { cx, cz, cause: RebuildCause::Edit });
            }
            Event::LightBordersUpdated { .. } => {}
        }
    }

    pub fn step(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread, dt: f32) {
        // Input handling → emit events
        if rl.is_key_pressed(KeyboardKey::KEY_V) { self.gs.walk_mode = !self.gs.walk_mode; }
        if self.gs.walk_mode {
            self.cam.update_look_only(rl, dt);
        } else {
            self.cam.update(rl, dt);
        }

        if rl.is_key_pressed(KeyboardKey::KEY_G) { self.gs.show_grid = !self.gs.show_grid; }
        if rl.is_key_pressed(KeyboardKey::KEY_F) { self.gs.wireframe = !self.gs.wireframe; }
        if rl.is_key_pressed(KeyboardKey::KEY_B) { self.gs.show_chunk_bounds = !self.gs.show_chunk_bounds; }
        if rl.is_key_pressed(KeyboardKey::KEY_ONE) { self.gs.place_type = Block::Dirt; }
        if rl.is_key_pressed(KeyboardKey::KEY_TWO) { self.gs.place_type = Block::Stone; }
        if rl.is_key_pressed(KeyboardKey::KEY_THREE) { self.gs.place_type = Block::Sand; }
        if rl.is_key_pressed(KeyboardKey::KEY_FOUR) { self.gs.place_type = Block::Grass; }
        if rl.is_key_pressed(KeyboardKey::KEY_FIVE) { self.gs.place_type = Block::Snow; }
        if rl.is_key_pressed(KeyboardKey::KEY_SIX) { self.gs.place_type = Block::Glowstone; }
        if rl.is_key_pressed(KeyboardKey::KEY_SEVEN) { self.gs.place_type = Block::Beacon; }

        // Light emitters via hotkeys
        if rl.is_key_pressed(KeyboardKey::KEY_L) {
            let fwd = self.cam.forward();
            let p = self.cam.position + fwd * 4.0;
            let wx = p.x.floor() as i32; let wy = p.y.floor() as i32; let wz = p.z.floor() as i32;
            self.queue.emit_now(Event::LightEmitterAdded { wx, wy, wz, level: 255, is_beacon: false });
        }
        if rl.is_key_pressed(KeyboardKey::KEY_K) {
            let fwd = self.cam.forward();
            let p = self.cam.position + fwd * 4.0;
            let wx = p.x.floor() as i32; let wy = p.y.floor() as i32; let wz = p.z.floor() as i32;
            self.queue.emit_now(Event::LightEmitterRemoved { wx, wy, wz });
        }

        // Mouse edit intents
        let want_edit = rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) || rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT);
        if want_edit {
            let place = rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT);
            let block = self.gs.place_type;
            self.queue.emit_now(Event::RaycastEditRequested { place, block });
        }

        // Movement intent for this tick (dt→ms)
        let dt_ms = (dt.max(0.0) * 1000.0) as u32;
        self.queue.emit_now(Event::MovementRequested { dt_ms, yaw: self.cam.yaw, walk_mode: self.gs.walk_mode });

        // Drain worker results, sort deterministically by job_id, and emit completion events for this tick
        let mut results: Vec<JobOut> = self.runtime.drain_worker_results();
        results.sort_by_key(|r| r.job_id);
        for r in results {
            let borders = r.cpu.borders_changed;
            self.queue.emit_now(Event::BuildChunkJobCompleted {
                cx: r.cx,
                cz: r.cz,
                rev: r.rev,
                cpu: r.cpu,
                buf: r.buf,
                borders_changed: borders,
                job_id: r.job_id,
            });
        }

        // Process events scheduled for this tick with a budget
        let mut processed = 0usize;
        let max_events = 20_000usize;
        while let Some(env) = self.queue.pop_ready() {
            self.handle_event(rl, thread, env);
            processed += 1;
            if processed >= max_events { break; }
        }
        self.gs.tick = self.gs.tick.wrapping_add(1);
        self.queue.advance_tick();
    }

    pub fn render(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread) {
        let camera3d = self.cam.to_camera3d();
        let mut d = rl.begin_drawing(thread);
        d.clear_background(Color::new(210, 221, 235, 255));
        {
            let mut d3 = d.begin_mode3D(camera3d);
            if self.gs.show_grid { d3.draw_grid(64, 1.0); }

            // Update shader uniforms
            let surface_fog = [210.0 / 255.0, 221.0 / 255.0, 235.0 / 255.0];
            let cave_fog = [0.0, 0.0, 0.0];
            let world_h = self.gs.world.world_size_y() as f32;
            let underground_thr = 0.30_f32 * world_h;
            let underground = self.cam.position.y < underground_thr;
            let fog_color = if underground { cave_fog } else { surface_fog };
            if let Some(ref mut ls) = self.runtime.leaves_shader {
                let fog_start = 64.0f32; let fog_end = 180.0f32;
                ls.update_frame_uniforms(self.cam.position, fog_color, fog_start, fog_end);
            }
            if let Some(ref mut fs) = self.runtime.fog_shader {
                let fog_start = 64.0f32; let fog_end = 180.0f32;
                fs.update_frame_uniforms(self.cam.position, fog_color, fog_start, fog_end);
            }

            for ((_key, cr)) in &self.runtime.renders {
                for (_fm, model) in &cr.parts {
                    if self.gs.wireframe {
                        d3.draw_model_wires(model, Vector3::zero(), 1.0, Color::WHITE);
                    } else {
                        d3.draw_model(model, Vector3::zero(), 1.0, Color::WHITE);
                    }
                }
            }

            if self.gs.show_chunk_bounds {
                let col = Color::new(255, 64, 32, 200);
                for ((_k, cr)) in &self.runtime.renders {
                    let min = cr.bbox.min;
                    let max = cr.bbox.max;
                    let center = Vector3::new((min.x + max.x) * 0.5, (min.y + max.y) * 0.5, (min.z + max.z) * 0.5);
                    let size = Vector3::new((max.x - min.x).abs(), (max.y - min.y).abs(), (max.z - min.z).abs());
                    d3.draw_cube_wires(center, size.x, size.y, size.z, col);
                }
            }
        }

        // HUD
        let hud_mode = if self.gs.walk_mode { "Walk" } else { "Fly" };
        let hud = format!(
            "{}: Tab capture, WASD{} move{}, V toggle mode, F wireframe, G grid, B bounds, L add light, K remove light | Place: {:?} (1-7)",
            hud_mode,
            if self.gs.walk_mode { "" } else { "+QE" },
            if self.gs.walk_mode { ", Space jump, Shift run" } else { "" },
            self.gs.place_type,
        );
        d.draw_text(&hud, 12, 12, 18, Color::DARKGRAY);
        d.draw_fps(12, 36);
    }
}

impl App {
    fn log_event(tick: u64, ev: &crate::event::Event) {
        use crate::event::Event as E;
        match ev {
            E::Tick => {
                log::trace!(target: "events", "[tick {}] Tick", tick);
            }
            E::MovementRequested { dt_ms, yaw, walk_mode } => {
                log::trace!(target: "events", "[tick {}] MovementRequested dt_ms={} yaw={:.1} mode={}",
                    tick, dt_ms, yaw, if *walk_mode {"walk"} else {"fly"});
            }
            E::RaycastEditRequested { place, block } => {
                log::info!(target: "events", "[tick {}] RaycastEditRequested {} block={:?}",
                    tick, if *place {"place"} else {"remove"}, block);
            }
            E::ViewCenterChanged { ccx, ccz } => {
                log::info!(target: "events", "[tick {}] ViewCenterChanged cc=({}, {})", tick, ccx, ccz);
            }
            E::EnsureChunkLoaded { cx, cz } => {
                log::info!(target: "events", "[tick {}] EnsureChunkLoaded ({}, {})", tick, cx, cz);
            }
            E::EnsureChunkUnloaded { cx, cz } => {
                log::info!(target: "events", "[tick {}] EnsureChunkUnloaded ({}, {})", tick, cx, cz);
            }
            E::ChunkRebuildRequested { cx, cz, cause } => {
                log::debug!(target: "events", "[tick {}] ChunkRebuildRequested ({}, {}) cause={:?}", tick, cx, cz, cause);
            }
            E::BuildChunkJobRequested { cx, cz, neighbors, rev, job_id } => {
                let mask = [neighbors.neg_x, neighbors.pos_x, neighbors.neg_z, neighbors.pos_z];
                log::info!(target: "events", "[tick {}] BuildChunkJobRequested ({}, {}) rev={} nmask={:?} job_id={:#x}",
                    tick, cx, cz, rev, mask, job_id);
            }
            E::BuildChunkJobCompleted { cx, cz, rev, job_id, .. } => {
                log::info!(target: "events", "[tick {}] BuildChunkJobCompleted ({}, {}) rev={} job_id={:#x}",
                    tick, cx, cz, rev, job_id);
            }
            E::LightEmitterAdded { wx, wy, wz, level, is_beacon } => {
                log::info!(target: "events", "[tick {}] LightEmitterAdded ({},{},{}) level={} beacon={}",
                    tick, wx, wy, wz, level, is_beacon);
            }
            E::LightEmitterRemoved { wx, wy, wz } => {
                log::info!(target: "events", "[tick {}] LightEmitterRemoved ({},{},{})", tick, wx, wy, wz);
            }
            E::LightBordersUpdated { cx, cz } => {
                log::debug!(target: "events", "[tick {}] LightBordersUpdated ({}, {})", tick, cx, cz);
            }
        }
    }
}

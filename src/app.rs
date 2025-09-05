use std::collections::HashSet;

use raylib::prelude::*;

use crate::event::{Event, EventEnvelope, EventQueue, RebuildCause};
use crate::gamestate::{ChunkEntry, GameState};
use crate::lighting::LightingStore;
use crate::mesher::{NeighborsLoaded, upload_chunk_mesh};
use crate::raycast;
use crate::runtime::{BuildJob, JobOut, Runtime, StructureBuildJob};
use crate::structure::{Structure, Pose, StructureId, rotate_yaw, rotate_yaw_inv};
use crate::voxel::{Block, World};

pub struct App {
    pub gs: GameState,
    pub queue: EventQueue,
    pub runtime: Runtime,
    pub cam: crate::camera::FlyCamera,
}

impl App {
    #[inline]
    fn structure_block_solid_at_local(st: &crate::structure::Structure, lx: i32, ly: i32, lz: i32) -> bool {
        if lx < 0 || ly < 0 || lz < 0 { return false; }
        let (lxu, lyu, lzu) = (lx as usize, ly as usize, lz as usize);
        if lxu >= st.sx || lyu >= st.sy || lzu >= st.sz { return false; }
        if let Some(b) = st.edits.get(lx, ly, lz) { return b.is_solid(); }
        st.blocks[st.idx(lxu, lyu, lzu)].is_solid()
    }

    fn is_feet_on_structure(&self, st: &crate::structure::Structure, feet_world: Vector3) -> bool {
        let rx = (self.gs.walker.radius * 0.85).max(0.05);
        let offsets = [
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(rx, 0.0, 0.0),
            Vector3::new(-rx, 0.0, 0.0),
            Vector3::new(0.0, 0.0, rx),
            Vector3::new(0.0, 0.0, -rx),
            Vector3::new(rx, 0.0, rx),
            Vector3::new(rx, 0.0, -rx),
            Vector3::new(-rx, 0.0, rx),
            Vector3::new(-rx, 0.0, -rx),
        ];
        for off in &offsets {
            let p = feet_world + *off;
            let local = crate::structure::rotate_yaw_inv(p - st.pose.pos, st.pose.yaw_deg);
            let lx = local.x.floor() as i32;
            let ly = (local.y - 0.08).floor() as i32;
            let lz = local.z.floor() as i32;
            if Self::structure_block_solid_at_local(st, lx, ly, lz) { return true; }
        }
        false
    }
    pub fn new(
        mut rl: &mut RaylibHandle,
        thread: &RaylibThread,
        world: std::sync::Arc<World>,
        lighting: std::sync::Arc<LightingStore>,
        edits: crate::edit::EditStore,
    ) -> Self {
        let spawn = Vector3::new(
            (world.world_size_x() as f32) * 0.5,
            (world.world_size_y() as f32) * 0.8,
            (world.world_size_z() as f32) * 0.5,
        );
        let cam = crate::camera::FlyCamera::new(spawn + Vector3::new(0.0, 5.0, 20.0));

        let runtime = Runtime::new(&mut rl, thread, world.clone(), lighting.clone());
        let mut gs = GameState::new(world.clone(), edits, lighting.clone(), cam.position);
        let mut queue = EventQueue::new();
        // Bootstrap initial streaming based on camera
        let ccx = (cam.position.x / world.chunk_size_x as f32).floor() as i32;
        let ccz = (cam.position.z / world.chunk_size_z as f32).floor() as i32;
        queue.emit_now(Event::ViewCenterChanged { ccx, ccz });
        // Spawn a flying castle structure at high altitude (original placement)
        let castle_id: StructureId = 1;
        let world_center = Vector3::new(
            (world.world_size_x() as f32) * 0.5,
            (world.world_size_y() as f32) * 0.7,
            (world.world_size_z() as f32) * 0.5,
        );
        let st_sx = 32usize;
        let st_sy = 24usize;
        let st_sz = 32usize;
        let pose = Pose { pos: world_center + Vector3::new(0.0, 16.0, 40.0), yaw_deg: 0.0 };
        let st = Structure::new(castle_id, st_sx, st_sy, st_sz, pose);
        gs.structures.insert(castle_id, st);
        queue.emit_now(Event::StructureBuildRequested { id: castle_id, rev: 1 });
        Self {
            gs,
            queue,
            runtime,
            cam,
        }
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

    fn handle_event(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread, env: EventEnvelope) {
        // Log a concise line for the processed event
        Self::log_event(self.gs.tick, &env.kind);
        match env.kind {
            Event::Tick => {}
            Event::MovementRequested {
                dt_ms,
                yaw,
                walk_mode: _,
            } => {
                // update camera look first (yaw drives walker forward)
                if self.gs.walk_mode {
                    // Collision sampler: structures > edits > buf > world
                    let sx = self.gs.world.chunk_size_x as i32;
                    let sz = self.gs.world.chunk_size_z as i32;
                    // Platform attachment: apply structure delta if attached; otherwise attach when detected
                    let feet_world = self.gs.walker.pos;
                    if let Some(att) = self.gs.ground_attach {
                        if let Some(st) = self.gs.structures.get(&att.id) {
                            // Re-anchor to exact local coordinate each tick (local frame lock)
                            let world_from_local = rotate_yaw(att.local_offset, st.pose.yaw_deg) + st.pose.pos;
                            self.gs.walker.pos = world_from_local;
                        } else {
                            self.gs.ground_attach = None;
                        }
                    } else {
                        for (id, st) in &self.gs.structures {
                            if self.is_feet_on_structure(st, feet_world) {
                                // Capture local feet offset and attach
                                let local = rotate_yaw_inv(self.gs.walker.pos - st.pose.pos, st.pose.yaw_deg);
                                self.gs.ground_attach = Some(crate::gamestate::GroundAttach { id: *id, grace: 8, local_offset: local });
                                // Snap to exact projection this tick
                                self.gs.walker.pos = rotate_yaw(local, st.pose.yaw_deg) + st.pose.pos;
                                break;
                            }
                        }
                    }
                    let sampler = |wx: i32, wy: i32, wz: i32| -> Block {
                        // Check dynamic structures first
                        for (_id, st) in &self.gs.structures {
                            let local = rotate_yaw_inv(
                                Vector3::new(wx as f32 + 0.5, wy as f32 + 0.5, wz as f32 + 0.5) - st.pose.pos,
                                st.pose.yaw_deg,
                            );
                            let lx = local.x.floor() as i32;
                            let ly = local.y.floor() as i32;
                            let lz = local.z.floor() as i32;
                            if lx >= 0 && ly >= 0 && lz >= 0
                                && (lx as usize) < st.sx && (ly as usize) < st.sy && (lz as usize) < st.sz
                            {
                                if let Some(b) = st.edits.get(lx, ly, lz) { if b.is_solid() { return b; } }
                                let idx = st.idx(lx as usize, ly as usize, lz as usize);
                                let b = st.blocks[idx];
                                if b.is_solid() { return b; }
                            }
                        }
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
                    // Refresh/detach attachment after movement
                    if let Some(att) = self.gs.ground_attach {
                        if let Some(st) = self.gs.structures.get(&att.id) {
                            if self.is_feet_on_structure(st, self.gs.walker.pos) {
                                // Update local offset and refresh grace
                                let new_local = rotate_yaw_inv(self.gs.walker.pos - st.pose.pos, st.pose.yaw_deg);
                                self.gs.ground_attach = Some(crate::gamestate::GroundAttach { id: att.id, grace: 8, local_offset: new_local });
                            } else if att.grace > 0 {
                                // Keep last local offset; decrease grace
                                self.gs.ground_attach = Some(crate::gamestate::GroundAttach { id: att.id, grace: att.grace - 1, local_offset: att.local_offset });
                            } else {
                                self.gs.ground_attach = None;
                            }
                        } else {
                            self.gs.ground_attach = None;
                        }
                    }
                    self.cam.position = self.gs.walker.eye_position();
                    // Emit ViewCenterChanged if center moved
                    let ccx =
                        (self.cam.position.x / self.gs.world.chunk_size_x as f32).floor() as i32;
                    let ccz =
                        (self.cam.position.z / self.gs.world.chunk_size_z as f32).floor() as i32;
                    if (ccx, ccz) != self.gs.center_chunk {
                        self.queue.emit_now(Event::ViewCenterChanged { ccx, ccz });
                    }
                } else {
                    // Fly camera mode moves the camera in step(); update view center from camera
                    let ccx =
                        (self.cam.position.x / self.gs.world.chunk_size_x as f32).floor() as i32;
                    let ccz =
                        (self.cam.position.z / self.gs.world.chunk_size_z as f32).floor() as i32;
                    if (ccx, ccz) != self.gs.center_chunk {
                        self.queue.emit_now(Event::ViewCenterChanged { ccx, ccz });
                    }
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
                        self.queue.emit_now(Event::EnsureChunkUnloaded {
                            cx: key.0,
                            cz: key.1,
                        });
                    }
                }
                // Cancel pending for far chunks
                self.gs.pending.retain(|k| desired.contains(k));
                // Load new ones
                for key in desired {
                    if !self.runtime.renders.contains_key(&key) && !self.gs.pending.contains(&key) {
                        self.queue.emit_now(Event::EnsureChunkLoaded {
                            cx: key.0,
                            cz: key.1,
                        });
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
                if self.runtime.renders.contains_key(&(cx, cz))
                    || self.gs.pending.contains(&(cx, cz))
                {
                    return;
                }
                let neighbors = self.neighbor_mask(cx, cz);
                let rev = self.gs.edits.get_rev(cx, cz);
                let job_id = Self::job_hash(cx, cz, rev, neighbors);
                self.queue.emit_now(Event::BuildChunkJobRequested {
                    cx,
                    cz,
                    neighbors,
                    rev,
                    job_id,
                });
                self.gs.pending.insert((cx, cz));
            }
            Event::BuildChunkJobRequested {
                cx,
                cz,
                neighbors,
                rev,
                job_id,
            } => {
                // Prepare edit snapshots for workers (pure)
                let chunk_edits = self.gs.edits.snapshot_for_chunk(cx, cz);
                let region_edits = self.gs.edits.snapshot_for_region(cx, cz, 1);
                self.runtime.submit_build_job(BuildJob {
                    cx,
                    cz,
                    neighbors,
                    rev,
                    job_id,
                    chunk_edits,
                    region_edits,
                });
            }
            Event::StructureBuildRequested { id, rev } => {
                if let Some(st) = self.gs.structures.get(&id) {
                    let job = StructureBuildJob {
                        id,
                        rev,
                        sx: st.sx,
                        sy: st.sy,
                        sz: st.sz,
                        base_blocks: st.blocks.clone(),
                        edits: st.edits.snapshot_all(),
                    };
                    self.runtime.submit_structure_build_job(job);
                }
            }
            Event::StructureBuildCompleted { id, rev, cpu } => {
                if let Some(mut cr) = upload_chunk_mesh(rl, thread, cpu, &mut self.runtime.tex_cache) {
                    for (_fm, model) in &mut cr.parts {
                        if let Some(mat) = model.materials_mut().get_mut(0) {
                            if let Some(ref fs) = self.runtime.fog_shader {
                                let dest = mat.shader_mut();
                                let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                let src_ptr: *const raylib::ffi::Shader = fs.shader.as_ref();
                                unsafe { std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1); }
                            }
                        }
                    }
                    self.runtime.structure_renders.insert(id, cr);
                }
                if let Some(st) = self.gs.structures.get_mut(&id) { st.built_rev = rev; }
            }
            Event::BuildChunkJobCompleted {
                cx,
                cz,
                rev,
                cpu,
                buf,
                light_borders,
                job_id: _,
            } => {
                // Drop if stale
                let cur_rev = self.gs.edits.get_rev(cx, cz);
                if rev < cur_rev {
                    // Re-enqueue latest
                    let neighbors = self.neighbor_mask(cx, cz);
                    let job_id = Self::job_hash(cx, cz, cur_rev, neighbors);
                    self.queue.emit_now(Event::BuildChunkJobRequested {
                        cx,
                        cz,
                        neighbors,
                        rev: cur_rev,
                        job_id,
                    });
                    return;
                }
                // Upload to GPU
                if let Some(mut cr) =
                    upload_chunk_mesh(rl, thread, cpu, &mut self.runtime.tex_cache)
                {
                    // Assign shaders
                    for (fm, model) in &mut cr.parts {
                        if let Some(mat) = model.materials_mut().get_mut(0) {
                            match fm {
                                crate::mesher::FaceMaterial::Leaves(_) => {
                                    if let Some(ref ls) = self.runtime.leaves_shader {
                                        let dest = mat.shader_mut();
                                        let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                        let src_ptr: *const raylib::ffi::Shader =
                                            ls.shader.as_ref();
                                        unsafe {
                                            std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1);
                                        }
                                    }
                                }
                                _ => {
                                    if let Some(ref fs) = self.runtime.fog_shader {
                                        let dest = mat.shader_mut();
                                        let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                        let src_ptr: *const raylib::ffi::Shader =
                                            fs.shader.as_ref();
                                        unsafe {
                                            std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    self.runtime.renders.insert((cx, cz), cr);
                }
                // Update CPU buf & built rev
                self.gs.chunks.insert(
                    (cx, cz),
                    ChunkEntry {
                        buf: Some(buf),
                        built_rev: rev,
                    },
                );
                self.gs.loaded.insert((cx, cz));
                self.gs.pending.remove(&(cx, cz));
                self.gs.edits.mark_built(cx, cz, rev);

                // Update light borders in main thread; if changed, emit a dedicated event
                if let Some(lb) = light_borders {
                    let changed = self.gs.lighting.update_borders(cx, cz, lb);
                    if changed {
                        self.queue
                            .emit_now(Event::LightBordersUpdated { cx, cz });
                    }
                }
            }
            Event::ChunkRebuildRequested { cx, cz, cause: _ } => {
                if !self.runtime.renders.contains_key(&(cx, cz))
                    || self.gs.pending.contains(&(cx, cz))
                {
                    return;
                }
                let neighbors = self.neighbor_mask(cx, cz);
                let rev = self.gs.edits.get_rev(cx, cz);
                let job_id = Self::job_hash(cx, cz, rev, neighbors);
                self.queue.emit_now(Event::BuildChunkJobRequested {
                    cx,
                    cz,
                    neighbors,
                    rev,
                    job_id,
                });
                self.gs.pending.insert((cx, cz));
            }
            Event::RaycastEditRequested { place, block } => {
                // Perform world + structure raycast and emit edit events
                let org = self.cam.position;
                let dir = self.cam.forward();
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
                let world_hit = raycast::raycast_first_hit_with_face(org, dir, 8.0 * 32.0, |x,y,z| sampler(x,y,z).is_solid());
                let mut struct_hit: Option<(StructureId, raycast::RayHit, f32)> = None;
                for (id, st) in &self.gs.structures {
                    let local_org = rotate_yaw_inv(org - st.pose.pos, st.pose.yaw_deg);
                    let local_dir = rotate_yaw_inv(dir, st.pose.yaw_deg);
                    let is_solid_local = |lx: i32, ly: i32, lz: i32| -> bool {
                        if lx < 0 || ly < 0 || lz < 0 { return false; }
                        let (lxu, lyu, lzu) = (lx as usize, ly as usize, lz as usize);
                        if lxu >= st.sx || lyu >= st.sy || lzu >= st.sz { return false; }
                        if let Some(b) = st.edits.get(lx, ly, lz) { return b.is_solid(); }
                        st.blocks[st.idx(lxu, lyu, lzu)].is_solid()
                    };
                    if let Some(hit) = raycast::raycast_first_hit_with_face(local_org, local_dir, 8.0 * 32.0, |x,y,z| is_solid_local(x,y,z)) {
                        let cc_local = Vector3::new(hit.bx as f32 + 0.5, hit.by as f32 + 0.5, hit.bz as f32 + 0.5);
                        let cc_world = rotate_yaw(cc_local, st.pose.yaw_deg) + st.pose.pos;
                        let d = cc_world - org;
                        let dist2 = d.x * d.x + d.y * d.y + d.z * d.z;
                        struct_hit = Some((*id, hit, dist2));
                        break;
                    }
                }
                let choose_struct = match (world_hit.as_ref(), struct_hit.as_ref()) {
                    (None, Some(_)) => true,
                    (Some(_), None) => false,
                    (Some(wh), Some((_id, _sh, sdist2))) => {
                        let wc = Vector3::new(wh.bx as f32 + 0.5, wh.by as f32 + 0.5, wh.bz as f32 + 0.5);
                        let dw = wc - org;
                        let wdist2 = dw.x * dw.x + dw.y * dw.y + dw.z * dw.z;
                        *sdist2 < wdist2
                    }
                    _ => false,
                };
                if choose_struct {
                    if let Some((id, hit, _)) = struct_hit {
                        if place {
                            let (lx, ly, lz) = (hit.px + hit.nx, hit.py + hit.ny, hit.pz + hit.nz);
                            self.queue.emit_now(Event::StructureBlockPlaced { id, lx, ly, lz, block });
                        } else {
                            self.queue.emit_now(Event::StructureBlockRemoved { id, lx: hit.bx, ly: hit.by, lz: hit.bz });
                        }
                    }
                } else if let Some(hit) = world_hit {
                    if place {
                        let wx = hit.px;
                        let wy = hit.py;
                        let wz = hit.pz;
                        if wy >= 0 && wy < self.gs.world.chunk_size_y as i32 {
                            self.queue.emit_now(Event::BlockPlaced { wx, wy, wz, block });
                        }
                    } else {
                        let wx = hit.bx;
                        let wy = hit.by;
                        let wz = hit.bz;
                        let prev = sampler(wx, wy, wz);
                        if prev.is_solid() { self.queue.emit_now(Event::BlockRemoved { wx, wy, wz }); }
                    }
                }
            }
            Event::StructureBlockPlaced { id, lx, ly, lz, block } => {
                if let Some(st) = self.gs.structures.get_mut(&id) {
                    st.set_local(lx, ly, lz, block);
                    let rev = st.dirty_rev;
                    self.queue.emit_now(Event::StructureBuildRequested { id, rev });
                }
            }
            Event::StructureBlockRemoved { id, lx, ly, lz } => {
                if let Some(st) = self.gs.structures.get_mut(&id) {
                    st.remove_local(lx, ly, lz);
                    let rev = st.dirty_rev;
                    self.queue.emit_now(Event::StructureBuildRequested { id, rev });
                }
            }
            Event::BlockPlaced { wx, wy, wz, block } => {
                self.gs.edits.set(wx, wy, wz, block);
                if block.emission() > 0 {
                    let is_beacon = matches!(block, Block::Beacon);
                    self.queue.emit_now(Event::LightEmitterAdded {
                        wx,
                        wy,
                        wz,
                        level: block.emission(),
                        is_beacon,
                    });
                }
                let _ = self.gs.edits.bump_region_around(wx, wz);
                // Rebuild edited chunk and any boundary-adjacent neighbors that are loaded
                for (cx, cz) in self.gs.edits.get_affected_chunks(wx, wz) {
                    if self.runtime.renders.contains_key(&(cx, cz)) {
                        self.queue.emit_now(Event::ChunkRebuildRequested {
                            cx,
                            cz,
                            cause: RebuildCause::Edit,
                        });
                    }
                }
            }
            Event::BlockRemoved { wx, wy, wz } => {
                // Determine previous block to update lighting
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
                let prev = sampler(wx, wy, wz);
                if prev.emission() > 0 {
                    self.queue
                        .emit_now(Event::LightEmitterRemoved { wx, wy, wz });
                }
                self.gs.edits.set(wx, wy, wz, Block::Air);
                let _ = self.gs.edits.bump_region_around(wx, wz);
                for (cx, cz) in self.gs.edits.get_affected_chunks(wx, wz) {
                    if self.runtime.renders.contains_key(&(cx, cz)) {
                        self.queue.emit_now(Event::ChunkRebuildRequested {
                            cx,
                            cz,
                            cause: RebuildCause::Edit,
                        });
                    }
                }
            }
            Event::LightEmitterAdded {
                wx,
                wy,
                wz,
                level,
                is_beacon,
            } => {
                if is_beacon {
                    self.gs.lighting.add_beacon_world(wx, wy, wz, level);
                } else {
                    self.gs.lighting.add_emitter_world(wx, wy, wz, level);
                }
                // schedule rebuild of that chunk
                let sx = self.gs.world.chunk_size_x as i32;
                let sz = self.gs.world.chunk_size_z as i32;
                let cx = wx.div_euclid(sx);
                let cz = wz.div_euclid(sz);
                self.queue.emit_now(Event::ChunkRebuildRequested {
                    cx,
                    cz,
                    cause: RebuildCause::Edit,
                });
            }
            Event::LightEmitterRemoved { wx, wy, wz } => {
                self.gs.lighting.remove_emitter_world(wx, wy, wz);
                let sx = self.gs.world.chunk_size_x as i32;
                let sz = self.gs.world.chunk_size_z as i32;
                let cx = wx.div_euclid(sx);
                let cz = wz.div_euclid(sz);
                self.queue.emit_now(Event::ChunkRebuildRequested {
                    cx,
                    cz,
                    cause: RebuildCause::Edit,
                });
            }
            Event::LightBordersUpdated { cx, cz } => {
                // Neighbor rebuilds in response to border changes, if loaded and not pending
                for (nx, nz) in [(cx - 1, cz), (cx + 1, cz), (cx, cz - 1), (cx, cz + 1)] {
                    if self.runtime.renders.contains_key(&(nx, nz))
                        && !self.gs.pending.contains(&(nx, nz))
                    {
                        self.queue.emit_now(Event::ChunkRebuildRequested {
                            cx: nx,
                            cz: nz,
                            cause: RebuildCause::LightingBorder,
                        });
                    }
                }
            }
            Event::WalkModeToggled => {
                let new_mode = !self.gs.walk_mode;
                self.gs.walk_mode = new_mode;
                if new_mode {
                    // Entering walk mode: align walker to current camera eye position
                    self.gs.walker.yaw = self.cam.yaw;
                    let mut p = self.cam.position;
                    p.y -= self.gs.walker.eye_height; // convert eye -> feet position
                    // Only clamp to ground (min Y); allow above-ceiling positions (e.g., flying structures)
                    p.y = p.y.max(0.0);
                    self.gs.walker.pos = p;
                    self.gs.walker.vel = Vector3::zero();
                    self.gs.walker.on_ground = false;
                    // Keep camera exactly at walker eye to avoid any snap
                    self.cam.position = self.gs.walker.eye_position();
                } else {
                    // Entering fly mode: camera already at walker eye; continue from here
                }
            }
            Event::GridToggled => {
                self.gs.show_grid = !self.gs.show_grid;
            }
            Event::WireframeToggled => {
                self.gs.wireframe = !self.gs.wireframe;
            }
            Event::ChunkBoundsToggled => {
                self.gs.show_chunk_bounds = !self.gs.show_chunk_bounds;
            }
            Event::PlaceTypeSelected { block } => {
                self.gs.place_type = block;
            }
        }
    }

    pub fn step(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread, dt: f32) {
        // Input handling → emit events
        if rl.is_key_pressed(KeyboardKey::KEY_V) {
            self.queue.emit_now(Event::WalkModeToggled);
        }
        if self.gs.walk_mode {
            self.cam.update_look_only(rl, dt);
        } else {
            self.cam.update(rl, dt);
        }

        if rl.is_key_pressed(KeyboardKey::KEY_G) {
            self.queue.emit_now(Event::GridToggled);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_F) {
            self.queue.emit_now(Event::WireframeToggled);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_B) {
            self.queue.emit_now(Event::ChunkBoundsToggled);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_ONE) {
            self.queue.emit_now(Event::PlaceTypeSelected { block: Block::Dirt });
        }
        if rl.is_key_pressed(KeyboardKey::KEY_TWO) {
            self.queue.emit_now(Event::PlaceTypeSelected { block: Block::Stone });
        }
        if rl.is_key_pressed(KeyboardKey::KEY_THREE) {
            self.queue.emit_now(Event::PlaceTypeSelected { block: Block::Sand });
        }
        if rl.is_key_pressed(KeyboardKey::KEY_FOUR) {
            self.queue.emit_now(Event::PlaceTypeSelected { block: Block::Grass });
        }
        if rl.is_key_pressed(KeyboardKey::KEY_FIVE) {
            self.queue.emit_now(Event::PlaceTypeSelected { block: Block::Snow });
        }
        if rl.is_key_pressed(KeyboardKey::KEY_SIX) {
            self.queue.emit_now(Event::PlaceTypeSelected { block: Block::Glowstone });
        }
        if rl.is_key_pressed(KeyboardKey::KEY_SEVEN) {
            self.queue.emit_now(Event::PlaceTypeSelected { block: Block::Beacon });
        }

        // Light emitters via hotkeys
        if rl.is_key_pressed(KeyboardKey::KEY_L) {
            let fwd = self.cam.forward();
            let p = self.cam.position + fwd * 4.0;
            let wx = p.x.floor() as i32;
            let wy = p.y.floor() as i32;
            let wz = p.z.floor() as i32;
            self.queue.emit_now(Event::LightEmitterAdded {
                wx,
                wy,
                wz,
                level: 255,
                is_beacon: false,
            });
        }
        if rl.is_key_pressed(KeyboardKey::KEY_K) {
            let fwd = self.cam.forward();
            let p = self.cam.position + fwd * 4.0;
            let wx = p.x.floor() as i32;
            let wy = p.y.floor() as i32;
            let wz = p.z.floor() as i32;
            self.queue
                .emit_now(Event::LightEmitterRemoved { wx, wy, wz });
        }

        // Mouse edit intents
        let want_edit = rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT)
            || rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT);
        if want_edit {
            let place = rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT);
            let block = self.gs.place_type;
            self.queue
                .emit_now(Event::RaycastEditRequested { place, block });
        }

        // Update structure poses (simple circular motion)
        let world_center = Vector3::new(
            (self.gs.world.world_size_x() as f32) * 0.5,
            (self.gs.world.world_size_y() as f32) * 0.7,
            (self.gs.world.world_size_z() as f32) * 0.5,
        );
        let radius = 40.0f32;
        let ang = (self.gs.tick as f32) * 0.004;
        for (_id, st) in self.gs.structures.iter_mut() {
            let new_x = world_center.x + radius * ang.cos();
            let new_z = world_center.z + radius * ang.sin();
            let prev = st.pose.pos;
            let newp = Vector3::new(new_x, prev.y, new_z);
            st.last_delta = newp - prev;
            st.pose.pos = newp;
            // Keep yaw fixed until render rotation is wired, so collisions match visuals
            st.pose.yaw_deg = 0.0;
        }

        // Movement intent for this tick (dt→ms)
        let dt_ms = (dt.max(0.0) * 1000.0) as u32;
        self.queue.emit_now(Event::MovementRequested {
            dt_ms,
            yaw: self.cam.yaw,
            walk_mode: self.gs.walk_mode,
        });

        // Drain worker results, sort deterministically by job_id, and emit completion events for this tick
        let mut results: Vec<JobOut> = self.runtime.drain_worker_results();
        results.sort_by_key(|r| r.job_id);
        for r in results {
            self.queue.emit_now(Event::BuildChunkJobCompleted {
                cx: r.cx,
                cz: r.cz,
                rev: r.rev,
                cpu: r.cpu,
                buf: r.buf,
                light_borders: r.light_borders,
                job_id: r.job_id,
            });
        }

        // Drain structure worker results
        for r in self.runtime.drain_structure_results() {
            self.queue.emit_now(Event::StructureBuildCompleted { id: r.id, rev: r.rev, cpu: r.cpu });
        }

        // Process events scheduled for this tick with a budget
        let mut processed = 0usize;
        let max_events = 20_000usize;
        while let Some(env) = self.queue.pop_ready() {
            self.handle_event(rl, thread, env);
            processed += 1;
            if processed >= max_events {
                break;
            }
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
            if self.gs.show_grid {
                d3.draw_grid(64, 1.0);
            }

            // Update shader uniforms
            let surface_fog = [210.0 / 255.0, 221.0 / 255.0, 235.0 / 255.0];
            let cave_fog = [0.0, 0.0, 0.0];
            let world_h = self.gs.world.world_size_y() as f32;
            let underground_thr = 0.30_f32 * world_h;
            let underground = self.cam.position.y < underground_thr;
            let fog_color = if underground { cave_fog } else { surface_fog };
            if let Some(ref mut ls) = self.runtime.leaves_shader {
                let fog_start = 64.0f32;
                let fog_end = 180.0f32;
                ls.update_frame_uniforms(self.cam.position, fog_color, fog_start, fog_end);
            }
            if let Some(ref mut fs) = self.runtime.fog_shader {
                let fog_start = 64.0f32;
                let fog_end = 180.0f32;
                fs.update_frame_uniforms(self.cam.position, fog_color, fog_start, fog_end);
            }

            for (_key, cr) in &self.runtime.renders {
                for (_fm, model) in &cr.parts {
                    if self.gs.wireframe {
                        d3.draw_model_wires(model, Vector3::zero(), 1.0, Color::WHITE);
                    } else {
                        d3.draw_model(model, Vector3::zero(), 1.0, Color::WHITE);
                    }
                }
            }

            // Draw structures with transform (translation + yaw)
            for (id, cr) in &self.runtime.structure_renders {
                if let Some(st) = self.gs.structures.get(id) {
                    for (_fm, model) in &cr.parts {
                        // Yaw is ignored here if draw_model_ex isn't available; translation still applies
                        d3.draw_model(model, st.pose.pos, 1.0, Color::WHITE);
                    }
                }
            }

            // Raycast highlight: show where a placed block would go (world only for now)
            // Sample order: edits > loaded chunk buffers > world
            let org = self.cam.position;
            let dir = self.cam.forward();
            let sx = self.gs.world.chunk_size_x as i32;
            let sy = self.gs.world.chunk_size_y as i32;
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
            let is_solid = |wx: i32, wy: i32, wz: i32| -> bool { sampler(wx, wy, wz).is_solid() };
            if let Some(hit) = raycast::raycast_first_hit_with_face(org, dir, 5.0, is_solid) {
                // Outline only the struck face of the solid block (bx,by,bz)
                let (bx, by, bz) = (hit.bx, hit.by, hit.bz);
                if by >= 0 && by < sy {
                    let (x0, y0, z0) = (bx as f32, by as f32, bz as f32);
                    let (x1, y1, z1) = (x0 + 1.0, y0 + 1.0, z0 + 1.0);
                    let eps = 0.002f32;
                    if hit.nx != 0 {
                        let xf = if hit.nx > 0 { x1 } else { x0 } + (hit.nx as f32) * eps;
                        let p1 = Vector3::new(xf, y0, z0);
                        let p2 = Vector3::new(xf, y1, z0);
                        let p3 = Vector3::new(xf, y1, z1);
                        let p4 = Vector3::new(xf, y0, z1);
                        d3.draw_line_3D(p1, p2, Color::YELLOW);
                        d3.draw_line_3D(p2, p3, Color::YELLOW);
                        d3.draw_line_3D(p3, p4, Color::YELLOW);
                        d3.draw_line_3D(p4, p1, Color::YELLOW);
                    } else if hit.ny != 0 {
                        let yf = if hit.ny > 0 { y1 } else { y0 } + (hit.ny as f32) * eps;
                        let p1 = Vector3::new(x0, yf, z0);
                        let p2 = Vector3::new(x1, yf, z0);
                        let p3 = Vector3::new(x1, yf, z1);
                        let p4 = Vector3::new(x0, yf, z1);
                        d3.draw_line_3D(p1, p2, Color::YELLOW);
                        d3.draw_line_3D(p2, p3, Color::YELLOW);
                        d3.draw_line_3D(p3, p4, Color::YELLOW);
                        d3.draw_line_3D(p4, p1, Color::YELLOW);
                    } else if hit.nz != 0 {
                        let zf = if hit.nz > 0 { z1 } else { z0 } + (hit.nz as f32) * eps;
                        let p1 = Vector3::new(x0, y0, zf);
                        let p2 = Vector3::new(x1, y0, zf);
                        let p3 = Vector3::new(x1, y1, zf);
                        let p4 = Vector3::new(x0, y1, zf);
                        d3.draw_line_3D(p1, p2, Color::YELLOW);
                        d3.draw_line_3D(p2, p3, Color::YELLOW);
                        d3.draw_line_3D(p3, p4, Color::YELLOW);
                        d3.draw_line_3D(p4, p1, Color::YELLOW);
                    }
                }
            }

            if self.gs.show_chunk_bounds {
                let col = Color::new(255, 64, 32, 200);
                for (_k, cr) in &self.runtime.renders {
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

        // HUD
        let hud_mode = if self.gs.walk_mode { "Walk" } else { "Fly" };
        let hud = format!(
            "{}: Tab capture, WASD{} move{}, V toggle mode, F wireframe, G grid, B bounds, L add light, K remove light | Place: {:?} (1-7) | Castle: moving",
            hud_mode,
            if self.gs.walk_mode { "" } else { "+QE" },
            if self.gs.walk_mode {
                ", Space jump, Shift run"
            } else {
                ""
            },
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
            E::WalkModeToggled => {
                log::info!(target: "events", "[tick {}] WalkModeToggled", tick);
            }
            E::GridToggled => {
                log::info!(target: "events", "[tick {}] GridToggled", tick);
            }
            E::WireframeToggled => {
                log::info!(target: "events", "[tick {}] WireframeToggled", tick);
            }
            E::ChunkBoundsToggled => {
                log::info!(target: "events", "[tick {}] ChunkBoundsToggled", tick);
            }
            E::PlaceTypeSelected { block } => {
                log::info!(target: "events", "[tick {}] PlaceTypeSelected block={:?}", tick, block);
            }
            E::MovementRequested {
                dt_ms,
                yaw,
                walk_mode,
            } => {
                log::trace!(target: "events", "[tick {}] MovementRequested dt_ms={} yaw={:.1} mode={}",
                    tick, dt_ms, yaw, if *walk_mode {"walk"} else {"fly"});
            }
            E::RaycastEditRequested { place, block } => {
                log::info!(target: "events", "[tick {}] RaycastEditRequested {} block={:?}",
                    tick, if *place {"place"} else {"remove"}, block);
            }
            E::BlockPlaced { wx, wy, wz, block } => {
                log::info!(target: "events", "[tick {}] BlockPlaced ({},{},{}) block={:?}", tick, wx, wy, wz, block);
            }
            E::BlockRemoved { wx, wy, wz } => {
                log::info!(target: "events", "[tick {}] BlockRemoved ({},{},{})", tick, wx, wy, wz);
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
            E::BuildChunkJobRequested {
                cx,
                cz,
                neighbors,
                rev,
                job_id,
            } => {
                let mask = [
                    neighbors.neg_x,
                    neighbors.pos_x,
                    neighbors.neg_z,
                    neighbors.pos_z,
                ];
                log::info!(target: "events", "[tick {}] BuildChunkJobRequested ({}, {}) rev={} nmask={:?} job_id={:#x}",
                    tick, cx, cz, rev, mask, job_id);
            }
            E::BuildChunkJobCompleted {
                cx,
                cz,
                rev,
                job_id,
                ..
            } => {
                log::info!(target: "events", "[tick {}] BuildChunkJobCompleted ({}, {}) rev={} job_id={:#x}",
                    tick, cx, cz, rev, job_id);
            }
            E::StructureBuildRequested { id, rev } => {
                log::info!(target: "events", "[tick {}] StructureBuildRequested id={} rev={}", tick, id, rev);
            }
            E::StructureBuildCompleted { id, rev, .. } => {
                log::info!(target: "events", "[tick {}] StructureBuildCompleted id={} rev={}", tick, id, rev);
            }
            E::StructureBlockPlaced { id, lx, ly, lz, block } => {
                log::info!(target: "events", "[tick {}] StructureBlockPlaced id={} ({},{},{}) block={:?}", tick, id, lx, ly, lz, block);
            }
            E::StructureBlockRemoved { id, lx, ly, lz } => {
                log::info!(target: "events", "[tick {}] StructureBlockRemoved id={} ({},{},{})", tick, id, lx, ly, lz);
            }
            E::LightEmitterAdded {
                wx,
                wy,
                wz,
                level,
                is_beacon,
            } => {
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

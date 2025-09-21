use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use raylib::prelude::*;

use super::state::IntentCause;
use super::{
    App, anchor_world_position, anchor_world_velocity, structure_local_sampler,
    structure_world_to_local,
};
use crate::event::{Event, EventEnvelope, RebuildCause};
use crate::gamestate::{FinalizeState, StructureAnchor, WalkerAnchor};
use crate::raycast;
use geist_blocks::{Block, BlockRegistry};
use geist_chunk::ChunkOccupancy;
use geist_geom::Vec3;
use geist_lighting::{LightBorders, NeighborBorders, pack_light_grid_atlas_with_neighbors};
use geist_render_raylib::conv::{vec3_from_rl, vec3_to_rl};
use geist_render_raylib::{update_chunk_light_texture, upload_chunk_mesh};
use geist_runtime::{BuildJob, StructureBuildJob};
use geist_structures::{Structure, StructureId, rotate_yaw, rotate_yaw_inv};
use geist_world::ChunkCoord;
use hashbrown::HashMap;

fn structure_neighbor_borders(lb: &LightBorders) -> NeighborBorders {
    NeighborBorders {
        xn: Some(lb.xn.clone()),
        xp: Some(lb.xp.clone()),
        zn: Some(lb.zn.clone()),
        zp: Some(lb.zp.clone()),
        yn: Some(lb.yn.clone()),
        yp: Some(lb.yp.clone()),
        sk_xn: Some(lb.sk_xn.clone()),
        sk_xp: Some(lb.sk_xp.clone()),
        sk_zn: Some(lb.sk_zn.clone()),
        sk_zp: Some(lb.sk_zp.clone()),
        sk_yn: Some(lb.sk_yn.clone()),
        sk_yp: Some(lb.sk_yp.clone()),
        bcn_xn: Some(lb.bcn_xn.clone()),
        bcn_xp: Some(lb.bcn_xp.clone()),
        bcn_zn: Some(lb.bcn_zn.clone()),
        bcn_zp: Some(lb.bcn_zp.clone()),
        bcn_yn: Some(lb.bcn_yn.clone()),
        bcn_yp: Some(lb.bcn_yp.clone()),
        bcn_dir_xn: Some(lb.bcn_dir_xn.clone()),
        bcn_dir_xp: Some(lb.bcn_dir_xp.clone()),
        bcn_dir_zn: Some(lb.bcn_dir_zn.clone()),
        bcn_dir_zp: Some(lb.bcn_dir_zp.clone()),
    }
}

fn spherical_chunk_coords(center: ChunkCoord, radius: i32) -> Vec<ChunkCoord> {
    if radius < 0 {
        return Vec::new();
    }
    let mut coords = Vec::new();
    let r_sq = i64::from(radius) * i64::from(radius);
    for dy in -radius..=radius {
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                let dist_sq = {
                    let dx64 = i64::from(dx);
                    let dy64 = i64::from(dy);
                    let dz64 = i64::from(dz);
                    dx64 * dx64 + dy64 * dy64 + dz64 * dz64
                };
                if dist_sq <= r_sq {
                    coords.push(center.offset(dx, dy, dz));
                }
            }
        }
    }
    coords
}

impl App {
    fn mark_empty_chunk_ready(&mut self, coord: ChunkCoord) {
        let st = self
            .gs
            .finalize
            .entry(coord)
            .or_insert_with(FinalizeState::default);
        st.owner_neg_x_ready = true;
        st.owner_neg_y_ready = true;
        st.owner_neg_z_ready = true;
        st.finalize_requested = false;
        st.finalized = true;

        let neighbors = [(1, 0, 0), (0, 1, 0), (0, 0, 1)];

        for &(dx, dy, dz) in &neighbors {
            let ncoord = coord.offset(dx, dy, dz);
            if let Some(nstate) = self.gs.finalize.get_mut(&ncoord) {
                match (dx, dy, dz) {
                    (1, 0, 0) => nstate.owner_neg_x_ready = true,
                    (0, 1, 0) => nstate.owner_neg_y_ready = true,
                    (0, 0, 1) => nstate.owner_neg_z_ready = true,
                    _ => {}
                }
                if nstate.owner_neg_x_ready
                    && nstate.owner_neg_y_ready
                    && nstate.owner_neg_z_ready
                    && !nstate.finalized
                    && !nstate.finalize_requested
                {
                    self.try_schedule_finalize(ncoord);
                }
            }
        }
    }

    fn prepare_chunk_for_edit(&mut self, coord: ChunkCoord) {
        self.gs.chunks.mark_loading(coord);

        // Recompute negative-owner readiness from available data so finalize scheduling remains accurate.
        let nb = self.gs.lighting.get_neighbor_borders(coord);
        let mut owner_neg_x_ready = nb.xn.is_some();
        let mut owner_neg_y_ready = nb.yn.is_some();
        let mut owner_neg_z_ready = nb.zn.is_some();
        let neg_neighbors = [(-1, 0, 0), (0, -1, 0), (0, 0, -1)];
        for &(dx, dy, dz) in &neg_neighbors {
            let ncoord = coord.offset(dx, dy, dz);
            let empty_neighbor = self
                .gs
                .chunks
                .get(&ncoord)
                .map(|entry| entry.occupancy_or_empty().is_empty())
                .unwrap_or(false);
            let finalized_neighbor = self
                .gs
                .finalize
                .get(&ncoord)
                .map(|state| state.finalized)
                .unwrap_or(false);
            if empty_neighbor || finalized_neighbor {
                match (dx, dy, dz) {
                    (-1, 0, 0) => owner_neg_x_ready = true,
                    (0, -1, 0) => owner_neg_y_ready = true,
                    (0, 0, -1) => owner_neg_z_ready = true,
                    _ => {}
                }
            }
        }

        {
            let st = self
                .gs
                .finalize
                .entry(coord)
                .or_insert_with(FinalizeState::default);
            st.owner_neg_x_ready = owner_neg_x_ready;
            st.owner_neg_y_ready = owner_neg_y_ready;
            st.owner_neg_z_ready = owner_neg_z_ready;
            st.finalize_requested = false;
            st.finalized = false;
        }

        // Invalidate positive-owner readiness on neighbors that previously counted on this chunk being empty.
        let pos_neighbors = [(1, 0, 0), (0, 1, 0), (0, 0, 1)];
        for &(dx, dy, dz) in &pos_neighbors {
            let ncoord = coord.offset(dx, dy, dz);
            if let Some(nstate) = self.gs.finalize.get_mut(&ncoord) {
                let neighbor_has_blocks = self
                    .gs
                    .chunks
                    .get(&ncoord)
                    .map(|entry| entry.occupancy_or_empty().has_blocks())
                    .unwrap_or(true);
                if !neighbor_has_blocks {
                    continue;
                }
                match (dx, dy, dz) {
                    (1, 0, 0) => nstate.owner_neg_x_ready = false,
                    (0, 1, 0) => nstate.owner_neg_y_ready = false,
                    (0, 0, 1) => nstate.owner_neg_z_ready = false,
                    _ => {}
                }
                nstate.finalize_requested = false;
                nstate.finalized = false;
            }
        }

        self.record_intent(coord, IntentCause::Edit);
    }

    fn structure_block_solid_at_local(
        reg: &BlockRegistry,
        st: &Structure,
        lx: i32,
        ly: i32,
        lz: i32,
    ) -> bool {
        if lx < 0 || ly < 0 || lz < 0 {
            return false;
        }
        let (lxu, lyu, lzu) = (lx as usize, ly as usize, lz as usize);
        if lxu >= st.sx || lyu >= st.sy || lzu >= st.sz {
            return false;
        }
        if let Some(b) = st.edits.get(lx, ly, lz) {
            return reg
                .get(b.id)
                .map(|ty| ty.is_solid(b.state))
                .unwrap_or(false);
        }
        let b = st.blocks[st.idx(lxu, lyu, lzu)];
        reg.get(b.id)
            .map(|ty| ty.is_solid(b.state))
            .unwrap_or(false)
    }

    pub(super) fn is_feet_on_structure(&self, st: &Structure, feet_world: Vector3) -> bool {
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
            let pv = vec3_from_rl(p);
            let local = structure_world_to_local(pv, st.pose.pos, st.pose.yaw_deg);
            let lx = local.x.floor() as i32;
            let ly = (local.y - 0.08).floor() as i32;
            let lz = local.z.floor() as i32;
            // Be robust to tiny clearance/step resolution by also checking one cell below
            if Self::structure_block_solid_at_local(&self.reg, st, lx, ly, lz)
                || Self::structure_block_solid_at_local(&self.reg, st, lx, ly - 1, lz)
            {
                return true;
            }
        }
        false
    }

    pub(super) fn handle_event(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        env: EventEnvelope,
    ) {
        // Log a concise line for the processed event
        Self::log_event(self.gs.tick, &env.kind);
        match env.kind {
            Event::Tick => {}
            Event::StructurePoseUpdated {
                id,
                pos,
                yaw_deg,
                delta,
                velocity,
            } => {
                if let Some(st) = self.gs.structures.get_mut(&id) {
                    st.last_delta = vec3_from_rl(delta);
                    st.last_velocity = vec3_from_rl(velocity);
                    st.pose.pos = vec3_from_rl(pos);
                    st.pose.yaw_deg = yaw_deg;
                    if matches!(self.gs.anchor, WalkerAnchor::Structure(anchor) if anchor.id == id)
                    {
                        self.sync_anchor_world_pose();
                    }
                }
            }
            Event::MovementRequested {
                dt_ms: _,
                yaw,
                walk_mode: _,
            } => {
                // update camera look first (yaw drives walker forward)
                if self.gs.walk_mode {
                    // Collision sampler: structures > edits > buf > world
                    let sx = self.gs.world.chunk_size_x as i32;
                    let sz = self.gs.world.chunk_size_z as i32;
                    // Platform attachment: handle attachment and movement

                    // First, check for new attachment
                    if matches!(self.gs.anchor, WalkerAnchor::World) {
                        let sun_id = self.sun.as_ref().map(|s| s.id);
                        for (id, st) in &self.gs.structures {
                            if Some(*id) == sun_id {
                                continue;
                            }
                            if self.is_feet_on_structure(st, self.gs.walker.pos) {
                                let walker_world = vec3_from_rl(self.gs.walker.pos);
                                let local =
                                    StructureAnchor::structure_local_from_world(st, walker_world);
                                let yaw_offset = self.gs.walker.yaw - st.pose.yaw_deg;
                                let anchor = StructureAnchor::new(*id, local, yaw_offset);
                                self.gs.anchor = WalkerAnchor::Structure(anchor);
                                self.queue.emit_now(Event::PlayerAttachedToStructure {
                                    id: *id,
                                    local_offset: vec3_to_rl(local),
                                });
                                break;
                            }
                        }
                    }

                    let reg = &self.reg;
                    let world_sampler = |wx: i32, wy: i32, wz: i32| -> Block {
                        // Check dynamic structures first
                        let sun_id = self.sun.as_ref().map(|s| s.id);
                        for st in self.gs.structures.values() {
                            if Some(st.id) == sun_id {
                                continue;
                            }
                            let p = vec3_from_rl(Vector3::new(
                                wx as f32 + 0.5,
                                wy as f32 + 0.5,
                                wz as f32 + 0.5,
                            ));
                            let diff = Vec3 {
                                x: p.x - st.pose.pos.x,
                                y: p.y - st.pose.pos.y,
                                z: p.z - st.pose.pos.z,
                            };
                            let local = rotate_yaw_inv(diff, st.pose.yaw_deg);
                            let lx = local.x.floor() as i32;
                            let ly = local.y.floor() as i32;
                            let lz = local.z.floor() as i32;
                            if lx >= 0
                                && ly >= 0
                                && lz >= 0
                                && (lx as usize) < st.sx
                                && (ly as usize) < st.sy
                                && (lz as usize) < st.sz
                            {
                                if let Some(b) = st.edits.get(lx, ly, lz) {
                                    if reg.get(b.id).map(|t| t.is_solid(b.state)).unwrap_or(false) {
                                        return b;
                                    }
                                }
                                let idx = st.idx(lx as usize, ly as usize, lz as usize);
                                let b = st.blocks[idx];
                                if reg.get(b.id).map(|t| t.is_solid(b.state)).unwrap_or(false) {
                                    return b;
                                }
                            }
                        }
                        if let Some(b) = self.gs.edits.get(wx, wy, wz) {
                            return b;
                        }
                        let cx = wx.div_euclid(sx);
                        let cy = wy.div_euclid(self.gs.world.chunk_size_y as i32);
                        let cz = wz.div_euclid(sz);
                        if let Some(cent) = self.gs.chunks.get(&ChunkCoord::new(cx, cy, cz)) {
                            match (cent.occupancy_or_empty(), cent.buf.as_ref()) {
                                (ChunkOccupancy::Empty, _) => return Block::AIR,
                                (_, Some(buf)) => {
                                    return buf.get_world(wx, wy, wz).unwrap_or(Block::AIR);
                                }
                                (_, None) => {}
                            }
                        }
                        self.gs.world.block_at_runtime(reg, wx, wy, wz)
                    };
                    let dt_sec = self.last_frame_dt.max(0.0);
                    let mut detach_request: Option<StructureId> = None;
                    let mut predicted_anchor: Option<(StructureId, Vector3)> = None;
                    {
                        if let WalkerAnchor::Structure(ref mut anchor) = self.gs.anchor {
                            if let Some(st) = self.gs.structures.get(&anchor.id) {
                                anchor.update_yaw_offset(st.pose.yaw_deg, yaw);

                                let local_before = StructureAnchor::structure_local_from_world(
                                    st,
                                    vec3_from_rl(self.gs.walker.pos),
                                );
                                let relative_vel_world =
                                    vec3_from_rl(self.gs.walker.vel) - st.last_velocity;
                                let local_vel_before =
                                    rotate_yaw_inv(relative_vel_world, st.pose.yaw_deg);

                                self.gs.walker.pos = vec3_to_rl(local_before);
                                self.gs.walker.vel = vec3_to_rl(local_vel_before);

                                let structure_sampler =
                                    structure_local_sampler(st, |wx, wy, wz| {
                                        world_sampler(wx, wy, wz)
                                    });
                                let prev_local = local_before;
                                self.gs.walker.update_structure_space(
                                    rl,
                                    &structure_sampler,
                                    &self.reg,
                                    dt_sec,
                                    yaw,
                                    anchor.yaw_offset,
                                );

                                let new_local = vec3_from_rl(self.gs.walker.pos);
                                let local_velocity = if dt_sec > 0.0001 {
                                    (new_local - prev_local) * (1.0 / dt_sec)
                                } else {
                                    Vec3::ZERO
                                };
                                anchor.local_pos = new_local;
                                anchor.update_local_velocity(local_velocity);

                                let world_pos = anchor_world_position(anchor, st);
                                let world_vel = anchor_world_velocity(anchor, st);
                                self.gs.walker.pos = vec3_to_rl(world_pos);
                                self.gs.walker.vel = vec3_to_rl(world_vel);

                                predicted_anchor = Some((anchor.id, vec3_to_rl(world_pos)));
                            } else {
                                detach_request = Some(anchor.id);
                                self.gs.walker.update_world_space(
                                    rl,
                                    &world_sampler,
                                    &self.reg,
                                    dt_sec,
                                    yaw,
                                );
                            }
                        } else {
                            self.gs.walker.update_world_space(
                                rl,
                                &world_sampler,
                                &self.reg,
                                dt_sec,
                                yaw,
                            );
                        }
                    }
                    if let Some((anchor_id, predicted_rl)) = predicted_anchor {
                        if let Some(st) = self.gs.structures.get(&anchor_id) {
                            let on_structure = self.is_feet_on_structure(st, predicted_rl);
                            if let WalkerAnchor::Structure(ref mut anchor) = self.gs.anchor {
                                if anchor.id == anchor_id {
                                    if on_structure {
                                        anchor.grace = 60;
                                    } else if anchor.grace > 0 {
                                        anchor.grace = anchor.grace.saturating_sub(1);
                                    } else {
                                        detach_request = Some(anchor.id);
                                    }
                                }
                            }
                        }
                    }
                    // Update attachment after physics - critical for allowing movement on platform
                    // `detach_request` already captures transitions from the anchored branch.
                    if let Some(id) = detach_request {
                        if let WalkerAnchor::Structure(anchor_state) = self.gs.anchor {
                            if let Some(st) = self.gs.structures.get(&id) {
                                let world_vel = anchor_world_velocity(&anchor_state, st);
                                let world_vel_rl = vec3_to_rl(world_vel);
                                let mut walker_vel = self.gs.walker.vel;
                                walker_vel.x = world_vel_rl.x;
                                walker_vel.z = world_vel_rl.z;
                                walker_vel.y += world_vel_rl.y;
                                self.gs.walker.vel = walker_vel;
                            }
                        }
                        self.gs.anchor = WalkerAnchor::World;
                        self.queue
                            .emit_now(Event::PlayerDetachedFromStructure { id });
                    }
                    self.cam.position = self.gs.walker.eye_position();
                    // Emit ViewCenterChanged if center moved
                    let ccx =
                        (self.cam.position.x / self.gs.world.chunk_size_x as f32).floor() as i32;
                    let ccy =
                        (self.cam.position.y / self.gs.world.chunk_size_y as f32).floor() as i32;
                    let ccz =
                        (self.cam.position.z / self.gs.world.chunk_size_z as f32).floor() as i32;
                    let new_center = ChunkCoord::new(ccx, ccy, ccz);
                    if new_center != self.gs.center_chunk {
                        self.queue
                            .emit_now(Event::ViewCenterChanged { ccx, ccy, ccz });
                    }
                } else {
                    // Fly camera mode moves the camera in step(); update view center from camera
                    let ccx =
                        (self.cam.position.x / self.gs.world.chunk_size_x as f32).floor() as i32;
                    let ccy =
                        (self.cam.position.y / self.gs.world.chunk_size_y as f32).floor() as i32;
                    let ccz =
                        (self.cam.position.z / self.gs.world.chunk_size_z as f32).floor() as i32;
                    let new_center = ChunkCoord::new(ccx, ccy, ccz);
                    if new_center != self.gs.center_chunk {
                        self.queue
                            .emit_now(Event::ViewCenterChanged { ccx, ccy, ccz });
                    }
                }
            }
            Event::PlayerAttachedToStructure { id, local_offset } => {
                // Idempotent: set/refresh attachment state
                if let Some(st) = self.gs.structures.get(&id) {
                    let local = vec3_from_rl(local_offset);
                    let yaw_offset = self.gs.walker.yaw - st.pose.yaw_deg;
                    self.gs.anchor =
                        WalkerAnchor::Structure(StructureAnchor::new(id, local, yaw_offset));
                }
            }
            Event::PlayerDetachedFromStructure { id } => {
                if let WalkerAnchor::Structure(anchor) = self.gs.anchor {
                    if anchor.id == id {
                        self.gs.anchor = WalkerAnchor::World;
                    }
                }
            }
            Event::ViewCenterChanged { ccx, ccy, ccz } => {
                let center = ChunkCoord::new(ccx, ccy, ccz);
                self.gs.center_chunk = center;
                let load_radius = self.stream_load_radius();
                let evict_radius = self.stream_evict_radius();
                let desired: HashSet<ChunkCoord> = spherical_chunk_coords(center, load_radius)
                    .into_iter()
                    .collect();
                let evict_limit_sq = {
                    let er = evict_radius;
                    i64::from(er) * i64::from(er)
                };
                for key in self.gs.chunks.coords_any().collect::<Vec<_>>() {
                    if center.distance_sq(key) > evict_limit_sq {
                        self.queue.emit_now(Event::EnsureChunkUnloaded {
                            cx: key.cx,
                            cy: key.cy,
                            cz: key.cz,
                        });
                    }
                }
                // Prune stream-load intents well outside the new radius (hysteresis: r+1)
                let mut to_remove: Vec<ChunkCoord> = Vec::new();
                let drop_sq = i64::from(load_radius) * i64::from(load_radius);
                for (&coord, ent) in self.intents.iter() {
                    if matches!(ent.cause, IntentCause::StreamLoad) {
                        let dist_sq = center.distance_sq(coord);
                        if dist_sq > drop_sq {
                            to_remove.push(coord);
                        }
                    }
                }
                for k in to_remove {
                    self.intents.remove(&k);
                }
                // Load new ones
                for key in desired {
                    if !self.gs.chunks.mesh_ready(key) && !self.gs.inflight_rev.contains_key(&key) {
                        self.queue.emit_now(Event::EnsureChunkLoaded {
                            cx: key.cx,
                            cy: key.cy,
                            cz: key.cz,
                        });
                    }
                }
            }
            Event::EnsureChunkUnloaded { cx, cy, cz } => {
                let coord = ChunkCoord::new(cx, cy, cz);
                self.renders.remove(&coord);
                self.gs.chunks.mark_missing(coord);
                self.gs.inflight_rev.remove(&coord);
                self.gs.finalize.remove(&coord);
                // Also drop any persisted lighting state for this chunk to prevent growth
                self.gs.lighting.clear_chunk(coord);
            }
            Event::EnsureChunkLoaded { cx, cy, cz } => {
                let coord = ChunkCoord::new(cx, cy, cz);
                if let Some(entry) = self.gs.chunks.get(&coord) {
                    if entry.occupancy_or_empty().is_empty() {
                        self.mark_empty_chunk_ready(coord);
                        return;
                    }
                }
                if self.gs.chunks.mesh_ready(coord) || self.gs.inflight_rev.contains_key(&coord) {
                    return;
                }
                self.gs.chunks.mark_loading(coord);
                // Init finalization tracking entry
                {
                    let nb = self.gs.lighting.get_neighbor_borders(coord);
                    let mut owner_neg_x_ready = nb.xn.is_some();
                    let mut owner_neg_y_ready = nb.yn.is_some();
                    let mut owner_neg_z_ready = nb.zn.is_some();
                    let neg_neighbors = [(-1, 0, 0), (0, -1, 0), (0, 0, -1)];
                    for &(dx, dy, dz) in &neg_neighbors {
                        let ncoord = coord.offset(dx, dy, dz);
                        let empty_neighbor = self
                            .gs
                            .chunks
                            .get(&ncoord)
                            .map(|entry| entry.occupancy_or_empty().is_empty())
                            .unwrap_or(false);
                        let finalized_neighbor = self
                            .gs
                            .finalize
                            .get(&ncoord)
                            .map(|state| state.finalized)
                            .unwrap_or(false);
                        if empty_neighbor || finalized_neighbor {
                            match (dx, dy, dz) {
                                (-1, 0, 0) => owner_neg_x_ready = true,
                                (0, -1, 0) => owner_neg_y_ready = true,
                                (0, 0, -1) => owner_neg_z_ready = true,
                                _ => {}
                            }
                        }
                    }
                    let st = self
                        .gs
                        .finalize
                        .entry(coord)
                        .or_insert(FinalizeState::default());
                    // Prime readiness from currently available owner planes, so we don't wait for future events
                    if owner_neg_x_ready {
                        st.owner_neg_x_ready = true;
                    }
                    if owner_neg_y_ready {
                        st.owner_neg_y_ready = true;
                    }
                    if owner_neg_z_ready {
                        st.owner_neg_z_ready = true;
                    }
                }
                // Record load intent; scheduler will cap and prioritize
                self.record_intent(coord, IntentCause::StreamLoad);
            }
            Event::BuildChunkJobRequested {
                cx,
                cy,
                cz,
                neighbors,
                rev,
                job_id,
                cause,
            } => {
                let coord = ChunkCoord::new(cx, cy, cz);
                // Prepare edit snapshots for workers (pure)
                let chunk_edits = self.gs.edits.snapshot_for_chunk(cx, cy, cz);
                let region_edits = self
                    .gs
                    .edits
                    .snapshot_for_region(cx, cy, cz, 1, 1)
                    .into_iter()
                    .collect::<HashMap<_, _>>();
                let expected_rev = self.gs.world.current_worldgen_rev();
                let mut column_profile = self.runtime.column_cache().get(coord, expected_rev);
                if column_profile.is_none() {
                    column_profile = self.gs.chunks.column_profile(&coord);
                }
                // Try to reuse previous buffer if present (and not invalidated)
                let prev_buf = self
                    .gs
                    .chunks
                    .get(&coord)
                    .and_then(|c| if c.has_blocks() { c.buf.as_ref() } else { None })
                    .cloned();
                let job = BuildJob {
                    cx,
                    cy,
                    cz,
                    neighbors,
                    rev,
                    job_id,
                    chunk_edits,
                    region_edits,
                    prev_buf,
                    reg: self.reg.clone(),
                    column_profile,
                };
                match cause {
                    RebuildCause::Edit => {
                        self.runtime.submit_build_job_edit(job);
                    }
                    RebuildCause::LightingBorder => {
                        self.runtime.submit_build_job_light(job);
                    }
                    RebuildCause::StreamLoad | RebuildCause::HotReload => {
                        self.runtime.submit_build_job_bg(job);
                    }
                }
                // inflight_rev was set by the emitter (EnsureChunkLoaded/ChunkRebuildRequested) or requeue branch.
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
                        reg: self.reg.clone(),
                    };
                    self.runtime.submit_structure_build_job(job);
                }
            }
            Event::StructureBuildCompleted {
                id,
                rev,
                cpu,
                light_grid,
                light_borders,
            } => {
                if let Some(mut cr) =
                    upload_chunk_mesh(rl, thread, cpu, &mut self.tex_cache, &self.reg.materials)
                {
                    for part in &mut cr.parts {
                        if let Some(mat) = part.model.materials_mut().get_mut(0) {
                            let tag = self
                                .reg
                                .materials
                                .get(part.mid)
                                .and_then(|m| m.render_tag.as_deref());
                            if tag == Some("leaves") {
                                if let Some(ref ls) = self.leaves_shader {
                                    let dest = mat.shader_mut();
                                    let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                    let src_ptr: *const raylib::ffi::Shader = ls.shader.as_ref();
                                    unsafe {
                                        std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1);
                                    }
                                }
                            } else if tag == Some("water") {
                                if let Some(ref ws) = self.water_shader {
                                    let dest = mat.shader_mut();
                                    let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                    let src_ptr: *const raylib::ffi::Shader = ws.shader.as_ref();
                                    unsafe {
                                        std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1);
                                    }
                                }
                            } else if let Some(ref fs) = self.fog_shader {
                                let dest = mat.shader_mut();
                                let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                let src_ptr: *const raylib::ffi::Shader = fs.shader.as_ref();
                                unsafe {
                                    std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1);
                                }
                            }
                        }
                    }
                    let atlas = {
                        let nb = structure_neighbor_borders(&light_borders);
                        pack_light_grid_atlas_with_neighbors(&light_grid, &nb)
                    };
                    update_chunk_light_texture(rl, thread, &mut cr, &atlas);
                    self.structure_renders.insert(id, cr);
                }
                self.structure_lights.insert(id, light_grid);
                self.structure_light_borders.insert(id, light_borders);
                if let Some(st) = self.gs.structures.get_mut(&id) {
                    st.built_rev = rev;
                }
            }
            Event::BuildChunkJobCompleted {
                cx,
                cy,
                cz,
                rev,
                occupancy,
                cpu,
                buf,
                light_borders,
                light_grid,
                job_id: _,
                column_profile,
            } => {
                let coord = ChunkCoord::new(cx, cy, cz);
                // Drop if stale
                let cur_rev = self.gs.edits.get_rev(cx, cy, cz);
                if rev < cur_rev {
                    // Only re-enqueue if there isn't already a newer inflight job
                    let inflight = self.gs.inflight_rev.get(&coord).copied().unwrap_or(0);
                    if inflight < cur_rev {
                        let neighbors = self.neighbor_mask(coord);
                        let job_id = Self::job_hash(coord, cur_rev, neighbors);
                        self.queue.emit_now(Event::BuildChunkJobRequested {
                            cx,
                            cy,
                            cz,
                            neighbors,
                            rev: cur_rev,
                            job_id,
                            cause: RebuildCause::Edit,
                        });
                        // Ensure inflight_rev reflects latest
                        self.gs.inflight_rev.insert(coord, cur_rev);
                    }
                    return;
                }
                // Gate completion by desired radius: if chunk is no longer desired, drop
                let center = self.gs.center_chunk;
                let dist_sq = center.distance_sq(coord);
                let keep_r = self.stream_evict_radius();
                let keep_sq = i64::from(keep_r) * i64::from(keep_r);
                if dist_sq > keep_sq {
                    // Not desired anymore: clear inflight and abandon result
                    self.gs.inflight_rev.remove(&coord);
                    // Do not upload or mark built; also avoid lighting border updates
                    return;
                }

                if let Some(profile) = column_profile.as_ref() {
                    self.runtime.column_cache().insert(Arc::clone(profile));
                } else {
                    self.gs.chunks.clear_column_profile(&coord);
                }

                if occupancy.is_empty() {
                    // Remove any previous render/lighting and mark chunk as a sparse placeholder.
                    self.renders.remove(&coord);
                    self.gs.lighting.clear_chunk(coord);
                    let entry = self.gs.chunks.mark_ready(
                        coord,
                        occupancy,
                        None,
                        rev,
                        column_profile.clone(),
                    );
                    entry.lighting_ready = true;
                    entry.mesh_ready = false;
                    self.gs.inflight_rev.remove(&coord);
                    self.gs.edits.mark_built(cx, cy, cz, rev);
                    self.gs.mesh_counts.remove(&coord);
                    self.gs.light_counts.remove(&coord);

                    self.mark_empty_chunk_ready(coord);
                    return;
                }

                let cpu = match cpu {
                    Some(cpu) => cpu,
                    None => {
                        log::warn!(
                            "populated chunk build missing mesh output at ({},{},{}) rev={}",
                            cx,
                            cy,
                            cz,
                            rev
                        );
                        self.gs.inflight_rev.remove(&coord);
                        return;
                    }
                };
                let buf = match buf {
                    Some(buf) => buf,
                    None => {
                        log::warn!(
                            "populated chunk build missing buffer at ({},{},{}) rev={}",
                            cx,
                            cy,
                            cz,
                            rev
                        );
                        self.gs.inflight_rev.remove(&coord);
                        return;
                    }
                };
                // Upload to GPU
                if let Some(mut cr) =
                    upload_chunk_mesh(rl, thread, cpu, &mut self.tex_cache, &self.reg.materials)
                {
                    // Assign biome-based leaf tint for this chunk (center sample)
                    let sx = self.gs.world.chunk_size_x as i32;
                    let sz = self.gs.world.chunk_size_z as i32;
                    let wx = cx * sx + sx / 2;
                    let wz = cz * sz + sz / 2;
                    if let Some(b) = self.gs.world.biome_at(wx, wz) {
                        if let Some(t) = b.leaf_tint {
                            cr.leaf_tint = Some(t);
                        }
                    }
                    // Assign shaders
                    for part in &mut cr.parts {
                        if let Some(mat) = part.model.materials_mut().get_mut(0) {
                            let tag = self
                                .reg
                                .materials
                                .get(part.mid)
                                .and_then(|m| m.render_tag.as_deref());
                            if tag == Some("leaves") {
                                if let Some(ref ls) = self.leaves_shader {
                                    let dest = mat.shader_mut();
                                    let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                    let src_ptr: *const raylib::ffi::Shader = ls.shader.as_ref();
                                    unsafe {
                                        std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1);
                                    }
                                }
                            } else if tag == Some("water") {
                                if let Some(ref ws) = self.water_shader {
                                    let dest = mat.shader_mut();
                                    let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                    let src_ptr: *const raylib::ffi::Shader = ws.shader.as_ref();
                                    unsafe {
                                        std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1);
                                    }
                                }
                            } else if let Some(ref fs) = self.fog_shader {
                                let dest = mat.shader_mut();
                                let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                let src_ptr: *const raylib::ffi::Shader = fs.shader.as_ref();
                                unsafe {
                                    std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1);
                                }
                            }
                        }
                    }
                    self.renders.insert(coord, cr);
                    if let Some(ref lg) = light_grid {
                        let nb = self.gs.lighting.get_neighbor_borders(coord);
                        let atlas = pack_light_grid_atlas_with_neighbors(lg, &nb);
                        self.validate_chunk_light_atlas(coord, &atlas);
                        if let Some(cr) = self.renders.get_mut(&coord) {
                            update_chunk_light_texture(rl, thread, cr, &atlas);
                        }
                    }
                }
                // Update CPU buf & built rev
                let entry = self.gs.chunks.mark_ready(
                    coord,
                    occupancy,
                    Some(buf),
                    rev,
                    column_profile.clone(),
                );
                entry.mesh_ready = true;
                entry.lighting_ready = light_grid.is_some();
                self.gs.inflight_rev.remove(&coord);
                self.gs.edits.mark_built(cx, cy, cz, rev);

                // Track mesh completion count for minimap/debug purposes
                *self.gs.mesh_counts.entry(coord).or_insert(0) += 1;

                // If we have a removal→render timer for this chunk, record latency now.
                if let Some(q) = self.perf_remove_start.get_mut(&coord) {
                    if let Some(t0) = q.pop_front() {
                        let dt_ms_u32 = t0.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
                        Self::perf_push(&mut self.perf_remove_ms, dt_ms_u32);
                        log::info!(
                            target: "perf",
                            "remove_to_render_ms={} cx={} cy={} cz={} rev={}",
                            dt_ms_u32,
                            cx,
                            cy,
                            cz,
                            rev
                        );
                    }
                    if q.is_empty() {
                        self.perf_remove_start.remove(&coord);
                    }
                }

                // Update light borders in main thread; if changed, emit a dedicated event
                let mut notify_mask = geist_lighting::BorderChangeMask::default();
                if let Some(lb) = light_borders {
                    let (changed, mask) = self.gs.lighting.update_borders_mask(coord, lb);
                    if changed {
                        notify_mask = mask;
                    }
                }
                if let Some(ref lg) = light_grid {
                    if lg.micro_change.any() {
                        if !notify_mask.any() {
                            notify_mask = lg.micro_change;
                        } else {
                            notify_mask.or_with(&lg.micro_change);
                        }
                    }
                }
                if notify_mask.any() {
                    self.queue.emit_now(Event::LightBordersUpdated {
                        cx,
                        cy,
                        cz,
                        xn_changed: notify_mask.xn,
                        xp_changed: notify_mask.xp,
                        yn_changed: notify_mask.yn,
                        yp_changed: notify_mask.yp,
                        zn_changed: notify_mask.zn,
                        zp_changed: notify_mask.zp,
                    });
                }
                // If both owners are ready and finalize not yet requested, schedule finalize now
                if let Some(st) = self.gs.finalize.get(&coord).copied() {
                    if st.owner_neg_x_ready
                        && st.owner_neg_y_ready
                        && st.owner_neg_z_ready
                        && !st.finalized
                        && !st.finalize_requested
                    {
                        self.try_schedule_finalize(coord);
                    }
                }
                // If this build was the finalize pass, mark completion
                if let Some(st) = self.gs.finalize.get_mut(&coord) {
                    if st.finalize_requested {
                        st.finalize_requested = false;
                        st.finalized = true;
                    }
                }
            }
            Event::ChunkLightingRecomputed {
                cx,
                cy,
                cz,
                rev,
                light_grid,
                job_id: _,
            } => {
                let coord = ChunkCoord::new(cx, cy, cz);
                // Drop if stale
                let cur_rev = self.gs.edits.get_rev(cx, cy, cz);
                if rev < cur_rev {
                    self.gs.inflight_rev.remove(&coord);
                    return;
                }
                // Gate by desired radius
                let center = self.gs.center_chunk;
                let dist_sq = center.distance_sq(coord);
                let gate = self.stream_evict_radius().saturating_add(1);
                let gate_sq = i64::from(gate) * i64::from(gate);
                if dist_sq > gate_sq {
                    self.gs.inflight_rev.remove(&coord);
                    return;
                }
                let nb = self.gs.lighting.get_neighbor_borders(coord);
                let atlas = pack_light_grid_atlas_with_neighbors(&light_grid, &nb);
                self.validate_chunk_light_atlas(coord, &atlas);
                if let Some(cr) = self.renders.get_mut(&coord) {
                    update_chunk_light_texture(rl, thread, cr, &atlas);
                }
                // Track light-only recompute count for minimap/debug
                *self.gs.light_counts.entry(coord).or_insert(0) += 1;
                if let Some(entry) = self.gs.chunks.get_any_mut(&coord) {
                    entry.lighting_ready = true;
                }
                // If this was a finalize pass scheduled via lighting-only lane, mark completion
                if let Some(st) = self.gs.finalize.get_mut(&coord) {
                    if st.finalize_requested {
                        st.finalize_requested = false;
                        st.finalized = true;
                    }
                }
                // Do not update borders or trigger neighbors on color-only recomputes.
                self.gs.inflight_rev.remove(&coord);
            }
            Event::ChunkRebuildRequested { cx, cy, cz, cause } => {
                let coord = ChunkCoord::new(cx, cy, cz);
                if !self.gs.chunks.mesh_ready(coord) {
                    return;
                }
                // Record rebuild intent; scheduler will cap and prioritize
                let ic = match cause {
                    RebuildCause::Edit => IntentCause::Edit,
                    RebuildCause::LightingBorder => IntentCause::Light,
                    RebuildCause::StreamLoad => IntentCause::StreamLoad,
                    RebuildCause::HotReload => IntentCause::HotReload,
                };
                self.record_intent(coord, ic);
            }
            Event::RaycastEditRequested { place, block } => {
                // Perform world + structure raycast and emit edit events
                let org = self.cam.position;
                let dir = self.cam.forward();
                let sx = self.gs.world.chunk_size_x as i32;
                let sy = self.gs.world.chunk_size_y as i32;
                let sz = self.gs.world.chunk_size_z as i32;
                let reg = self.reg.clone();
                let sampler = |wx: i32, wy: i32, wz: i32| -> Block {
                    if let Some(b) = self.gs.edits.get(wx, wy, wz) {
                        return b;
                    }
                    let cx = wx.div_euclid(sx);
                    let cy = wy.div_euclid(sy);
                    let cz = wz.div_euclid(sz);
                    if let Some(cent) = self.gs.chunks.get(&ChunkCoord::new(cx, cy, cz)) {
                        match (cent.occupancy_or_empty(), cent.buf.as_ref()) {
                            (ChunkOccupancy::Empty, _) => {
                                return Block {
                                    id: reg.id_by_name("air").unwrap_or(0),
                                    state: 0,
                                };
                            }
                            (_, Some(buf)) => {
                                return buf.get_world(wx, wy, wz).unwrap_or(Block {
                                    id: reg.id_by_name("air").unwrap_or(0),
                                    state: 0,
                                });
                            }
                            (_, None) => {}
                        }
                    }
                    // Outside loaded buffers: treat as air
                    Block {
                        id: reg.id_by_name("air").unwrap_or(0),
                        state: 0,
                    }
                };
                let world_hit =
                    raycast::raycast_first_hit_with_face(org, dir, 8.0 * 32.0, |x, y, z| {
                        let b = sampler(x, y, z);
                        self.reg
                            .get(b.id)
                            .map(|ty| ty.is_solid(b.state))
                            .unwrap_or(false)
                    });
                let mut struct_hit: Option<(StructureId, raycast::RayHit, f32)> = None;
                let sun_id = self.sun.as_ref().map(|s| s.id);
                for (id, st) in &self.gs.structures {
                    if Some(*id) == sun_id {
                        continue;
                    }
                    let o = vec3_from_rl(org);
                    let diff = Vec3 {
                        x: o.x - st.pose.pos.x,
                        y: o.y - st.pose.pos.y,
                        z: o.z - st.pose.pos.z,
                    };
                    let local_org = vec3_to_rl(rotate_yaw_inv(diff, st.pose.yaw_deg));
                    let local_dir = vec3_to_rl(rotate_yaw_inv(vec3_from_rl(dir), st.pose.yaw_deg));
                    let is_solid_local = |lx: i32, ly: i32, lz: i32| -> bool {
                        if lx < 0 || ly < 0 || lz < 0 {
                            return false;
                        }
                        let (lxu, lyu, lzu) = (lx as usize, ly as usize, lz as usize);
                        if lxu >= st.sx || lyu >= st.sy || lzu >= st.sz {
                            return false;
                        }
                        if let Some(b) = st.edits.get(lx, ly, lz) {
                            return self
                                .reg
                                .get(b.id)
                                .map(|ty| ty.is_solid(b.state))
                                .unwrap_or(false);
                        }
                        let b = st.blocks[st.idx(lxu, lyu, lzu)];
                        self.reg
                            .get(b.id)
                            .map(|ty| ty.is_solid(b.state))
                            .unwrap_or(false)
                    };
                    if let Some(hit) = raycast::raycast_first_hit_with_face(
                        local_org,
                        local_dir,
                        8.0 * 32.0,
                        is_solid_local,
                    ) {
                        let cc_local = Vector3::new(
                            hit.bx as f32 + 0.5,
                            hit.by as f32 + 0.5,
                            hit.bz as f32 + 0.5,
                        );
                        let wl = rotate_yaw(vec3_from_rl(cc_local), st.pose.yaw_deg);
                        let cc_world = Vec3 {
                            x: wl.x + st.pose.pos.x,
                            y: wl.y + st.pose.pos.y,
                            z: wl.z + st.pose.pos.z,
                        };
                        let cw = vec3_to_rl(cc_world);
                        let d = Vector3::new(cw.x - org.x, cw.y - org.y, cw.z - org.z);
                        let dist2 = d.x * d.x + d.y * d.y + d.z * d.z;
                        struct_hit = Some((*id, hit, dist2));
                        break;
                    }
                }
                let choose_struct = match (world_hit.as_ref(), struct_hit.as_ref()) {
                    (None, Some(_)) => true,
                    (Some(_), None) => false,
                    (Some(wh), Some((_id, _sh, sdist2))) => {
                        let wc = Vector3::new(
                            wh.bx as f32 + 0.5,
                            wh.by as f32 + 0.5,
                            wh.bz as f32 + 0.5,
                        );
                        let dw = wc - org;
                        let wdist2 = dw.x * dw.x + dw.y * dw.y + dw.z * dw.z;
                        *sdist2 < wdist2
                    }
                    _ => false,
                };
                if choose_struct {
                    if let Some((id, hit, _)) = struct_hit {
                        if place {
                            // Place on the adjacent empty cell directly (no extra normal offset)
                            let (lx, ly, lz) = (hit.px, hit.py, hit.pz);
                            self.queue.emit_now(Event::StructureBlockPlaced {
                                id,
                                lx,
                                ly,
                                lz,
                                block,
                            });
                        } else {
                            self.queue.emit_now(Event::StructureBlockRemoved {
                                id,
                                lx: hit.bx,
                                ly: hit.by,
                                lz: hit.bz,
                            });
                        }
                    }
                } else if let Some(hit) = world_hit {
                    if place {
                        let wx = hit.px;
                        let wy = hit.py;
                        let wz = hit.pz;
                        self.queue
                            .emit_now(Event::BlockPlaced { wx, wy, wz, block });
                    } else {
                        let wx = hit.bx;
                        let wy = hit.by;
                        let wz = hit.bz;
                        let prev = sampler(wx, wy, wz);
                        if self
                            .reg
                            .get(prev.id)
                            .map(|t| t.is_solid(prev.state))
                            .unwrap_or(false)
                        {
                            self.queue.emit_now(Event::BlockRemoved { wx, wy, wz });
                        }
                    }
                }
            }
            Event::StructureBlockPlaced {
                id,
                lx,
                ly,
                lz,
                block,
            } => {
                if let Some(st) = self.gs.structures.get_mut(&id) {
                    st.set_local(lx, ly, lz, block);
                    let rev = st.dirty_rev;
                    self.queue
                        .emit_now(Event::StructureBuildRequested { id, rev });
                }
            }
            Event::StructureBlockRemoved { id, lx, ly, lz } => {
                if let Some(st) = self.gs.structures.get_mut(&id) {
                    st.remove_local(lx, ly, lz);
                    let rev = st.dirty_rev;
                    self.queue
                        .emit_now(Event::StructureBuildRequested { id, rev });
                }
            }
            Event::BlockPlaced { wx, wy, wz, block } => {
                self.gs.edits.set(wx, wy, wz, block);
                let em = self
                    .reg
                    .get(block.id)
                    .map(|t| t.light_emission(block.state))
                    .unwrap_or(0);
                if em > 0 {
                    let is_beacon = self
                        .reg
                        .get(block.id)
                        .map(|t| t.light_is_beam())
                        .unwrap_or(false);
                    self.queue.emit_now(Event::LightEmitterAdded {
                        wx,
                        wy,
                        wz,
                        level: em,
                        is_beacon,
                    });
                }
                let _ = self.gs.edits.bump_region_around(wx, wy, wz);
                // Rebuild edited chunk and any boundary-adjacent neighbors that are loaded
                for coord in self.gs.edits.get_affected_chunks(wx, wy, wz) {
                    if self.gs.chunks.mesh_ready(coord) {
                        self.queue.emit_now(Event::ChunkRebuildRequested {
                            cx: coord.cx,
                            cy: coord.cy,
                            cz: coord.cz,
                            cause: RebuildCause::Edit,
                        });
                        // Start removal→render timer for this affected chunk
                        self.perf_remove_start
                            .entry(coord)
                            .or_default()
                            .push_back(Instant::now());
                    } else {
                        self.prepare_chunk_for_edit(coord);
                    }
                }
            }
            Event::BlockRemoved { wx, wy, wz } => {
                // Determine previous block to update lighting
                let sx = self.gs.world.chunk_size_x as i32;
                let sy = self.gs.world.chunk_size_y as i32;
                let sz = self.gs.world.chunk_size_z as i32;
                let reg = &self.reg;
                let sampler = |wx: i32, wy: i32, wz: i32| -> Block {
                    if let Some(b) = self.gs.edits.get(wx, wy, wz) {
                        return b;
                    }
                    let cx = wx.div_euclid(sx);
                    let cy = wy.div_euclid(sy);
                    let cz = wz.div_euclid(sz);
                    if let Some(cent) = self.gs.chunks.get(&ChunkCoord::new(cx, cy, cz)) {
                        match (cent.occupancy_or_empty(), cent.buf.as_ref()) {
                            (ChunkOccupancy::Empty, _) => return Block::AIR,
                            (_, Some(buf)) => {
                                return buf.get_world(wx, wy, wz).unwrap_or(Block::AIR);
                            }
                            (_, None) => {}
                        }
                    }
                    self.gs.world.block_at_runtime(reg, wx, wy, wz)
                };
                let prev = sampler(wx, wy, wz);
                let prev_em = self
                    .reg
                    .get(prev.id)
                    .map(|t| t.light_emission(prev.state))
                    .unwrap_or(0);
                if prev_em > 0 {
                    self.queue
                        .emit_now(Event::LightEmitterRemoved { wx, wy, wz });
                }
                self.gs.edits.set(wx, wy, wz, Block::AIR);
                let _ = self.gs.edits.bump_region_around(wx, wy, wz);
                for coord in self.gs.edits.get_affected_chunks(wx, wy, wz) {
                    if self.gs.chunks.mesh_ready(coord) {
                        self.queue.emit_now(Event::ChunkRebuildRequested {
                            cx: coord.cx,
                            cy: coord.cy,
                            cz: coord.cz,
                            cause: RebuildCause::Edit,
                        });
                    } else {
                        self.prepare_chunk_for_edit(coord);
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
                let sy = self.gs.world.chunk_size_y as i32;
                let sz = self.gs.world.chunk_size_z as i32;
                let cx = wx.div_euclid(sx);
                let cy = wy.div_euclid(sy);
                let cz = wz.div_euclid(sz);
                self.queue.emit_now(Event::ChunkRebuildRequested {
                    cx,
                    cy,
                    cz,
                    cause: RebuildCause::Edit,
                });
            }
            Event::LightEmitterRemoved { wx, wy, wz } => {
                self.gs.lighting.remove_emitter_world(wx, wy, wz);
                let sx = self.gs.world.chunk_size_x as i32;
                let sy = self.gs.world.chunk_size_y as i32;
                let sz = self.gs.world.chunk_size_z as i32;
                let cx = wx.div_euclid(sx);
                let cy = wy.div_euclid(sy);
                let cz = wz.div_euclid(sz);
                self.queue.emit_now(Event::ChunkRebuildRequested {
                    cx,
                    cy,
                    cz,
                    cause: RebuildCause::Edit,
                });
            }
            Event::LightBordersUpdated {
                cx,
                cy,
                cz,
                xn_changed,
                xp_changed,
                yn_changed,
                yp_changed,
                zn_changed,
                zp_changed,
            } => {
                let coord = ChunkCoord::new(cx, cy, cz);
                let center = self.gs.center_chunk;
                let r_gate = self.stream_evict_radius().saturating_add(1);
                let r_gate_sq = i64::from(r_gate) * i64::from(r_gate);

                if xp_changed {
                    let neighbor = coord.offset(1, 0, 0);
                    let st = self
                        .gs
                        .finalize
                        .entry(neighbor)
                        .or_insert(FinalizeState::default());
                    st.owner_neg_x_ready = true;
                    let dist_sq = center.distance_sq(neighbor);
                    if dist_sq <= r_gate_sq
                        && !st.finalized
                        && st.owner_neg_y_ready
                        && st.owner_neg_z_ready
                    {
                        self.try_schedule_finalize(neighbor);
                    } else if st.finalized {
                        if dist_sq <= r_gate_sq && self.gs.chunks.mesh_ready(neighbor) {
                            self.queue.emit_now(Event::ChunkRebuildRequested {
                                cx: neighbor.cx,
                                cy: neighbor.cy,
                                cz: neighbor.cz,
                                cause: RebuildCause::LightingBorder,
                            });
                        }
                    }
                }

                if zp_changed {
                    let neighbor = coord.offset(0, 0, 1);
                    let st = self
                        .gs
                        .finalize
                        .entry(neighbor)
                        .or_insert(FinalizeState::default());
                    st.owner_neg_z_ready = true;
                    let dist_sq = center.distance_sq(neighbor);
                    if dist_sq <= r_gate_sq
                        && !st.finalized
                        && st.owner_neg_x_ready
                        && st.owner_neg_y_ready
                    {
                        self.try_schedule_finalize(neighbor);
                    } else if st.finalized {
                        if dist_sq <= r_gate_sq && self.gs.chunks.mesh_ready(neighbor) {
                            self.queue.emit_now(Event::ChunkRebuildRequested {
                                cx: neighbor.cx,
                                cy: neighbor.cy,
                                cz: neighbor.cz,
                                cause: RebuildCause::LightingBorder,
                            });
                        }
                    }
                }

                if xn_changed {
                    let neighbor = coord.offset(-1, 0, 0);
                    let dist_sq = center.distance_sq(neighbor);
                    if dist_sq <= r_gate_sq && self.gs.chunks.mesh_ready(neighbor) {
                        self.queue.emit_now(Event::ChunkRebuildRequested {
                            cx: neighbor.cx,
                            cy: neighbor.cy,
                            cz: neighbor.cz,
                            cause: RebuildCause::LightingBorder,
                        });
                    }
                }
                if zn_changed {
                    let neighbor = coord.offset(0, 0, -1);
                    let dist_sq = center.distance_sq(neighbor);
                    if dist_sq <= r_gate_sq && self.gs.chunks.mesh_ready(neighbor) {
                        self.queue.emit_now(Event::ChunkRebuildRequested {
                            cx: neighbor.cx,
                            cy: neighbor.cy,
                            cz: neighbor.cz,
                            cause: RebuildCause::LightingBorder,
                        });
                    }
                }
                if yp_changed {
                    let neighbor = coord.offset(0, 1, 0);
                    let st = self
                        .gs
                        .finalize
                        .entry(neighbor)
                        .or_insert(FinalizeState::default());
                    st.owner_neg_y_ready = true;
                    let dist_sq = center.distance_sq(neighbor);
                    if dist_sq <= r_gate_sq
                        && !st.finalized
                        && st.owner_neg_x_ready
                        && st.owner_neg_z_ready
                    {
                        self.try_schedule_finalize(neighbor);
                    } else if st.finalized {
                        if dist_sq <= r_gate_sq && self.gs.chunks.mesh_ready(neighbor) {
                            self.queue.emit_now(Event::ChunkRebuildRequested {
                                cx: neighbor.cx,
                                cy: neighbor.cy,
                                cz: neighbor.cz,
                                cause: RebuildCause::LightingBorder,
                            });
                        }
                    }
                }
                if yn_changed {
                    let neighbor = coord.offset(0, -1, 0);
                    let dist_sq = center.distance_sq(neighbor);
                    if dist_sq <= r_gate_sq && self.gs.chunks.mesh_ready(neighbor) {
                        self.queue.emit_now(Event::ChunkRebuildRequested {
                            cx: neighbor.cx,
                            cy: neighbor.cy,
                            cz: neighbor.cz,
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
            Event::FrustumCullingToggled => {
                self.gs.frustum_culling_enabled = !self.gs.frustum_culling_enabled;
            }
            Event::BiomeLabelToggled => {
                self.gs.show_biome_label = !self.gs.show_biome_label;
            }
            Event::DebugOverlayToggled => {
                self.gs.show_debug_overlay = !self.gs.show_debug_overlay;
            }
            Event::PlaceTypeSelected { block } => {
                self.gs.place_type = block;
            }
        }
    }

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
            E::FrustumCullingToggled => {
                log::info!(target: "events", "[tick {}] FrustumCullingToggled", tick);
            }
            E::BiomeLabelToggled => {
                log::info!(target: "events", "[tick {}] BiomeLabelToggled", tick);
            }
            E::DebugOverlayToggled => {
                log::info!(target: "events", "[tick {}] DebugOverlayToggled", tick);
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
            E::ViewCenterChanged { ccx, ccy, ccz } => {
                log::info!(
                    target: "events",
                    "[tick {}] ViewCenterChanged cc=({}, {}, {})",
                    tick,
                    ccx,
                    ccy,
                    ccz
                );
            }
            E::EnsureChunkLoaded { cx, cy, cz } => {
                log::info!(
                    target: "events",
                    "[tick {}] EnsureChunkLoaded ({}, {}, {})",
                    tick,
                    cx,
                    cy,
                    cz
                );
            }
            E::EnsureChunkUnloaded { cx, cy, cz } => {
                log::info!(
                    target: "events",
                    "[tick {}] EnsureChunkUnloaded ({}, {}, {})",
                    tick,
                    cx,
                    cy,
                    cz
                );
            }
            E::ChunkRebuildRequested { cx, cy, cz, cause } => {
                log::debug!(
                    target: "events",
                    "[tick {}] ChunkRebuildRequested ({}, {}, {}) cause={:?}",
                    tick,
                    cx,
                    cy,
                    cz,
                    cause
                );
            }
            E::BuildChunkJobRequested {
                cx,
                cy,
                cz,
                neighbors,
                rev,
                job_id,
                cause,
            } => {
                let mask = [
                    neighbors.neg_x,
                    neighbors.pos_x,
                    neighbors.neg_y,
                    neighbors.pos_y,
                    neighbors.neg_z,
                    neighbors.pos_z,
                ];
                log::debug!(
                    target: "events",
                    "[tick {}] BuildChunkJobRequested ({}, {}, {}) rev={} cause={:?} nmask={:?} job_id={:#x}",
                    tick,
                    cx,
                    cy,
                    cz,
                    rev,
                    cause,
                    mask,
                    job_id
                );
            }
            E::BuildChunkJobCompleted {
                cx,
                cy,
                cz,
                rev,
                job_id,
                ..
            } => {
                log::debug!(
                    target: "events",
                    "[tick {}] BuildChunkJobCompleted ({}, {}, {}) rev={} job_id={:#x}",
                    tick,
                    cx,
                    cy,
                    cz,
                    rev,
                    job_id
                );
            }
            E::ChunkLightingRecomputed {
                cx,
                cy,
                cz,
                rev,
                job_id,
                ..
            } => {
                log::debug!(
                    target: "events",
                    "[tick {}] ChunkLightingRecomputed ({}, {}, {}) rev={} job_id={:#x}",
                    tick,
                    cx,
                    cy,
                    cz,
                    rev,
                    job_id
                );
            }
            E::StructureBuildRequested { id, rev } => {
                log::info!(target: "events", "[tick {}] StructureBuildRequested id={} rev={}", tick, id, rev);
            }
            E::StructureBuildCompleted { id, rev, .. } => {
                log::info!(target: "events", "[tick {}] StructureBuildCompleted id={} rev={}", tick, id, rev);
            }
            E::StructurePoseUpdated {
                id,
                pos,
                yaw_deg,
                delta,
                velocity,
            } => {
                log::trace!(target: "events", "[tick {}] StructurePoseUpdated id={} pos=({:.2},{:.2},{:.2}) yaw={:.1} delta=({:.2},{:.2},{:.2}) vel=({:.2},{:.2},{:.2})",
                    tick, id, pos.x, pos.y, pos.z, yaw_deg, delta.x, delta.y, delta.z, velocity.x, velocity.y, velocity.z);
            }
            E::StructureBlockPlaced {
                id,
                lx,
                ly,
                lz,
                block,
            } => {
                log::info!(target: "events", "[tick {}] StructureBlockPlaced id={} ({},{},{}) block={:?}", tick, id, lx, ly, lz, block);
            }
            E::StructureBlockRemoved { id, lx, ly, lz } => {
                log::info!(target: "events", "[tick {}] StructureBlockRemoved id={} ({},{},{})", tick, id, lx, ly, lz);
            }
            E::PlayerAttachedToStructure { id, local_offset } => {
                log::info!(target: "events", "[tick {}] PlayerAttachedToStructure id={} local=({:.2},{:.2},{:.2})",
                    tick, id, local_offset.x, local_offset.y, local_offset.z);
            }
            E::PlayerDetachedFromStructure { id } => {
                log::info!(target: "events", "[tick {}] PlayerDetachedFromStructure id={}", tick, id);
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
            E::LightBordersUpdated {
                cx,
                cy,
                cz,
                xn_changed,
                xp_changed,
                yn_changed,
                yp_changed,
                zn_changed,
                zp_changed,
            } => {
                log::debug!(
                    target: "events",
                    "[tick {}] LightBordersUpdated ({}, {}, {}) xn={} xp={} yn={} yp={} zn={} zp={}",
                    tick,
                    cx,
                    cy,
                    cz,
                    xn_changed,
                    xp_changed,
                    yn_changed,
                    yp_changed,
                    zn_changed,
                    zp_changed
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn spherical_range_includes_vertical_diagonals() {
        let center = ChunkCoord::new(5, 4, -2);
        let coords = spherical_chunk_coords(center, 2);
        let set: HashSet<_> = coords.into_iter().collect();
        assert!(
            set.contains(&ChunkCoord::new(6, 5, -1)),
            "missing vertical diagonal"
        );
        assert!(
            set.contains(&ChunkCoord::new(5, 6, -2)),
            "missing +Y neighbor"
        );
        assert!(
            !set.contains(&ChunkCoord::new(7, 5, -2)),
            "included chunk outside radius"
        );
        assert!(
            !set.contains(&ChunkCoord::new(5, 7, -2)),
            "included chunk above radius"
        );
    }

    #[test]
    fn spherical_range_excludes_far_diagonals_at_radius_one() {
        let center = ChunkCoord::new(0, 0, 0);
        let coords = spherical_chunk_coords(center, 1);
        let set: HashSet<_> = coords.into_iter().collect();
        assert!(set.contains(&center));
        assert!(set.contains(&ChunkCoord::new(0, 1, 0)));
        assert!(set.contains(&ChunkCoord::new(1, 0, 0)));
        assert!(
            !set.contains(&ChunkCoord::new(1, 1, 0)),
            "diagonal should be outside radius 1"
        );
        assert!(
            !set.contains(&ChunkCoord::new(0, 0, 2)),
            "distance sqrt(4) > 1 should be excluded"
        );
    }
}

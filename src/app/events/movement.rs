use super::{
    App, anchor_world_position, anchor_world_velocity, structure_local_sampler,
    structure_world_to_local,
};
use crate::event::Event;
use crate::gamestate::{StructureAnchor, WalkerAnchor};
use geist_blocks::Block;
use geist_chunk::ChunkOccupancy;
use geist_geom::Vec3;
use geist_render_raylib::conv::{vec3_from_rl, vec3_to_rl};
use geist_structures::{Structure, StructureId, rotate_yaw_inv};
use raylib::prelude::*;

impl App {
    pub(super) fn handle_structure_pose_updated(
        &mut self,
        id: StructureId,
        pos: Vector3,
        yaw_deg: f32,
        delta: Vector3,
        velocity: Vector3,
    ) {
        if let Some(st) = self.gs.structures.get_mut(&id) {
            st.last_delta = vec3_from_rl(delta);
            st.last_velocity = vec3_from_rl(velocity);
            st.pose.pos = vec3_from_rl(pos);
            st.pose.yaw_deg = yaw_deg;
            if matches!(self.gs.anchor, WalkerAnchor::Structure(anchor) if anchor.id == id) {
                self.sync_anchor_world_pose();
            }
        }
    }

    pub(super) fn handle_movement_requested(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        dt_ms: u32,
        yaw: f32,
        walk_mode: bool,
    ) {
        let _ = (thread, dt_ms, walk_mode);
        if self.gs.walk_mode {
            let sx = self.gs.world.chunk_size_x as i32;
            let sz = self.gs.world.chunk_size_z as i32;

            if matches!(self.gs.anchor, WalkerAnchor::World) {
                let sun_id = self.sun.as_ref().map(|s| s.id);
                for (id, st) in &self.gs.structures {
                    if Some(*id) == sun_id {
                        continue;
                    }
                    if self.is_feet_on_structure(st, self.gs.walker.pos) {
                        let walker_world = vec3_from_rl(self.gs.walker.pos);
                        let local = StructureAnchor::structure_local_from_world(st, walker_world);
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
                if let Some(cent) = self
                    .gs
                    .chunks
                    .get(&geist_world::ChunkCoord::new(cx, cy, cz))
                {
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
                        let local_vel_before = rotate_yaw_inv(relative_vel_world, st.pose.yaw_deg);

                        self.gs.walker.pos = vec3_to_rl(local_before);
                        self.gs.walker.vel = vec3_to_rl(local_vel_before);

                        let structure_sampler =
                            structure_local_sampler(st, |wx, wy, wz| world_sampler(wx, wy, wz));
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
                    self.gs
                        .walker
                        .update_world_space(rl, &world_sampler, &self.reg, dt_sec, yaw);
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
            if let Some(id) = detach_request {
                if let WalkerAnchor::Structure(ref anchor_state) = self.gs.anchor {
                    if let Some(st) = self.gs.structures.get(&id) {
                        let world_vel = anchor_world_velocity(anchor_state, st);
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
            self.emit_view_center_if_changed();
        } else {
            self.emit_view_center_if_changed();
        }
    }

    pub(super) fn handle_player_attached_to_structure(
        &mut self,
        id: StructureId,
        local_offset: Vector3,
    ) {
        if let Some(st) = self.gs.structures.get(&id) {
            let local = vec3_from_rl(local_offset);
            let yaw_offset = self.gs.walker.yaw - st.pose.yaw_deg;
            self.gs.anchor = WalkerAnchor::Structure(StructureAnchor::new(id, local, yaw_offset));
        }
    }

    pub(super) fn handle_player_detached_from_structure(&mut self, id: StructureId) {
        if let WalkerAnchor::Structure(ref anchor) = self.gs.anchor {
            if anchor.id == id {
                self.gs.anchor = WalkerAnchor::World;
            }
        }
    }

    pub(crate) fn is_feet_on_structure(&self, st: &Structure, feet_world: Vector3) -> bool {
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
            if Self::structure_block_solid_at_local(&self.reg, st, lx, ly, lz)
                || Self::structure_block_solid_at_local(&self.reg, st, lx, ly - 1, lz)
            {
                return true;
            }
        }
        false
    }

    fn emit_view_center_if_changed(&mut self) {
        let ccx = (self.cam.position.x / self.gs.world.chunk_size_x as f32).floor() as i32;
        let ccy = (self.cam.position.y / self.gs.world.chunk_size_y as f32).floor() as i32;
        let ccz = (self.cam.position.z / self.gs.world.chunk_size_z as f32).floor() as i32;
        let new_center = geist_world::ChunkCoord::new(ccx, ccy, ccz);
        if new_center != self.gs.center_chunk {
            self.queue
                .emit_now(Event::ViewCenterChanged { ccx, ccy, ccz });
        }
    }

    fn structure_block_solid_at_local(
        reg: &geist_blocks::BlockRegistry,
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
}

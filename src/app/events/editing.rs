use super::App;
use crate::event::{Event, RebuildCause};
use crate::raycast;
use geist_blocks::Block;
use geist_chunk::ChunkOccupancy;
use geist_geom::Vec3;
use geist_render_raylib::conv::{vec3_from_rl, vec3_to_rl};
use geist_structures::{StructureId, rotate_yaw, rotate_yaw_inv};
use geist_world::ChunkCoord;
use raylib::prelude::*;
use std::time::Instant;

impl App {
    pub(super) fn handle_raycast_edit_requested(&mut self, place: bool, block: Block) {
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
            Block {
                id: reg.id_by_name("air").unwrap_or(0),
                state: 0,
            }
        };
        let world_hit = raycast::raycast_first_hit_with_face(org, dir, 8.0 * 32.0, |x, y, z| {
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

    pub(super) fn handle_structure_block_placed(
        &mut self,
        id: StructureId,
        lx: i32,
        ly: i32,
        lz: i32,
        block: Block,
    ) {
        if let Some(st) = self.gs.structures.get_mut(&id) {
            st.set_local(lx, ly, lz, block);
            let rev = st.dirty_rev;
            self.queue
                .emit_now(Event::StructureBuildRequested { id, rev });
        }
    }

    pub(super) fn handle_structure_block_removed(
        &mut self,
        id: StructureId,
        lx: i32,
        ly: i32,
        lz: i32,
    ) {
        if let Some(st) = self.gs.structures.get_mut(&id) {
            st.remove_local(lx, ly, lz);
            let rev = st.dirty_rev;
            self.queue
                .emit_now(Event::StructureBuildRequested { id, rev });
        }
    }

    pub(super) fn handle_block_placed(&mut self, wx: i32, wy: i32, wz: i32, block: Block) {
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
        let sx = self.gs.world.chunk_size_x as i32;
        let sy = self.gs.world.chunk_size_y as i32;
        let sz = self.gs.world.chunk_size_z as i32;
        let origin = ChunkCoord::new(wx.div_euclid(sx), wy.div_euclid(sy), wz.div_euclid(sz));
        for coord in self.gs.edits.get_affected_chunks(wx, wy, wz) {
            let Some(cause) = Self::classify_edit_rebuild_cause(origin, coord) else {
                continue;
            };
            if self.gs.chunks.mesh_ready(coord) {
                self.queue.emit_now(Event::ChunkRebuildRequested {
                    cx: coord.cx,
                    cy: coord.cy,
                    cz: coord.cz,
                    cause,
                });
                if cause == RebuildCause::Edit {
                    self.perf_remove_start
                        .entry(coord)
                        .or_default()
                        .push_back(Instant::now());
                }
            } else if cause == RebuildCause::Edit {
                self.prepare_chunk_for_edit(coord);
            }
        }
    }

    pub(super) fn handle_block_removed(&mut self, wx: i32, wy: i32, wz: i32) {
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
        let origin = ChunkCoord::new(wx.div_euclid(sx), wy.div_euclid(sy), wz.div_euclid(sz));
        for coord in self.gs.edits.get_affected_chunks(wx, wy, wz) {
            let Some(cause) = Self::classify_edit_rebuild_cause(origin, coord) else {
                continue;
            };
            if self.gs.chunks.mesh_ready(coord) {
                self.queue.emit_now(Event::ChunkRebuildRequested {
                    cx: coord.cx,
                    cy: coord.cy,
                    cz: coord.cz,
                    cause,
                });
            } else if cause == RebuildCause::Edit {
                self.prepare_chunk_for_edit(coord);
            }
        }
    }

    pub(super) fn handle_light_emitter_added(
        &mut self,
        wx: i32,
        wy: i32,
        wz: i32,
        level: u8,
        is_beacon: bool,
    ) {
        if is_beacon {
            self.gs.lighting.add_beacon_world(wx, wy, wz, level);
        } else {
            self.gs.lighting.add_emitter_world(wx, wy, wz, level);
        }
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

    pub(super) fn handle_light_emitter_removed(&mut self, wx: i32, wy: i32, wz: i32) {
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
}

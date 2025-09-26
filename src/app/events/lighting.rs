use super::App;
use crate::event::Event;
use crate::gamestate::FinalizeState;
use geist_lighting::{
    LightBorders, LightGrid, NeighborBorders, pack_light_grid_atlas_with_neighbors,
};
use geist_render_raylib::update_chunk_light_texture;
use geist_world::ChunkCoord;
use raylib::prelude::*;

impl App {
    pub(super) fn handle_chunk_lighting_recomputed(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        coord: ChunkCoord,
        rev: u64,
        light_grid: LightGrid,
    ) {
        let cur_rev = self.gs.edits.get_rev(coord.cx, coord.cy, coord.cz);
        if rev < cur_rev {
            self.gs.inflight_rev.remove(&coord);
            return;
        }
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
        *self.gs.light_counts.entry(coord).or_insert(0) += 1;
        if let Some(entry) = self.gs.chunks.get_any_mut(&coord) {
            entry.lighting_ready = true;
        }
        if let Some(st) = self.gs.finalize.get_mut(&coord) {
            if st.finalize_requested {
                st.finalize_requested = false;
                st.finalized = true;
            }
        }
        self.gs.inflight_rev.remove(&coord);
    }

    pub(super) fn handle_light_borders_updated(
        &mut self,
        coord: ChunkCoord,
        xn_changed: bool,
        xp_changed: bool,
        yn_changed: bool,
        yp_changed: bool,
        zn_changed: bool,
        zp_changed: bool,
    ) {
        let center = self.gs.center_chunk;
        let r_gate = self.stream_evict_radius().saturating_add(1);
        let r_gate_sq = i64::from(r_gate) * i64::from(r_gate);

        if xp_changed {
            self.mark_neighbor_finalize(
                coord.offset(1, 0, 0),
                center,
                r_gate_sq,
                true,
                false,
                false,
            );
        }
        if zp_changed {
            self.mark_neighbor_finalize(
                coord.offset(0, 0, 1),
                center,
                r_gate_sq,
                false,
                false,
                true,
            );
        }
        if xn_changed {
            self.schedule_border_rebuild(coord.offset(-1, 0, 0), r_gate_sq, center);
        }
        if zn_changed {
            self.schedule_border_rebuild(coord.offset(0, 0, -1), r_gate_sq, center);
        }
        if yp_changed {
            self.mark_neighbor_finalize(
                coord.offset(0, 1, 0),
                center,
                r_gate_sq,
                false,
                true,
                false,
            );
        }
        if yn_changed {
            self.schedule_border_rebuild(coord.offset(0, -1, 0), r_gate_sq, center);
        }
    }

    fn mark_neighbor_finalize(
        &mut self,
        neighbor: ChunkCoord,
        center: ChunkCoord,
        r_gate_sq: i64,
        x_ready: bool,
        y_ready: bool,
        z_ready: bool,
    ) {
        let st = self
            .gs
            .finalize
            .entry(neighbor)
            .or_insert(FinalizeState::default());
        if x_ready {
            st.owner_neg_x_ready = true;
        }
        if y_ready {
            st.owner_neg_y_ready = true;
        }
        if z_ready {
            st.owner_neg_z_ready = true;
        }
        let dist_sq = center.distance_sq(neighbor);
        if dist_sq <= r_gate_sq
            && !st.finalized
            && st.owner_neg_x_ready
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
                    cause: crate::event::RebuildCause::LightingBorder,
                });
            }
        }
    }

    fn schedule_border_rebuild(
        &mut self,
        neighbor: ChunkCoord,
        r_gate_sq: i64,
        center: ChunkCoord,
    ) {
        let dist_sq = center.distance_sq(neighbor);
        if dist_sq <= r_gate_sq && self.gs.chunks.mesh_ready(neighbor) {
            self.queue.emit_now(Event::ChunkRebuildRequested {
                cx: neighbor.cx,
                cy: neighbor.cy,
                cz: neighbor.cz,
                cause: crate::event::RebuildCause::LightingBorder,
            });
        }
    }
}

pub(crate) fn structure_neighbor_borders(lb: &LightBorders) -> NeighborBorders {
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
        bcn_dir_zn: Some(lb.bcn_zn.clone()),
        bcn_dir_zp: Some(lb.bcn_zp.clone()),
    }
}

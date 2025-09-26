use super::{App, IntentCause};
use crate::gamestate::FinalizeState;
use geist_world::ChunkCoord;

pub(super) fn spherical_chunk_coords(center: ChunkCoord, radius: i32) -> Vec<ChunkCoord> {
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
    #[inline]
    pub(super) fn classify_edit_rebuild_cause(
        origin: ChunkCoord,
        coord: ChunkCoord,
    ) -> Option<crate::event::RebuildCause> {
        use crate::event::RebuildCause;
        if coord == origin {
            Some(RebuildCause::Edit)
        } else if coord.cy == origin.cy {
            Some(RebuildCause::Edit)
        } else {
            Some(RebuildCause::LightingBorder)
        }
    }

    pub(super) fn mark_empty_chunk_ready(&mut self, coord: ChunkCoord) {
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

    pub(super) fn prepare_chunk_for_edit(&mut self, coord: ChunkCoord) {
        self.gs.chunks.mark_loading(coord);

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
}

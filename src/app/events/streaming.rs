use super::{App, IntentCause, helpers::spherical_chunk_coords};
use crate::event::{Event, RebuildCause};
use crate::gamestate::FinalizeState;
use geist_world::ChunkCoord;
use hashbrown::HashSet;

impl App {
    pub(super) fn handle_view_center_changed(&mut self, ccx: i32, ccy: i32, ccz: i32) {
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

    pub(super) fn handle_ensure_chunk_unloaded(&mut self, coord: ChunkCoord) {
        self.renders.remove(&coord);
        self.gs.chunks.mark_missing(coord);
        self.gs.inflight_rev.remove(&coord);
        self.gs.finalize.remove(&coord);
        self.gs.lighting.clear_chunk(coord);
    }

    pub(super) fn handle_ensure_chunk_loaded(&mut self, coord: ChunkCoord) {
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
        self.record_intent(coord, IntentCause::StreamLoad);
    }

    pub(super) fn handle_chunk_rebuild_requested(
        &mut self,
        coord: ChunkCoord,
        cause: RebuildCause,
    ) {
        if !self.gs.chunks.mesh_ready(coord) {
            return;
        }
        let ic = match cause {
            RebuildCause::Edit => IntentCause::Edit,
            RebuildCause::LightingBorder => IntentCause::Light,
            RebuildCause::StreamLoad => IntentCause::StreamLoad,
            RebuildCause::HotReload => IntentCause::HotReload,
        };
        self.record_intent(coord, ic);
    }
}

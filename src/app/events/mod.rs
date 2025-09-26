mod builds;
mod editing;
mod helpers;
mod lighting;
mod logging;
mod movement;
mod streaming;
mod toggles;

pub(super) use super::state::IntentCause;
pub(super) use super::{
    anchor_world_position, anchor_world_velocity, structure_local_sampler, structure_world_to_local,
};

use raylib::prelude::*;

use super::App;
use crate::event::{Event, EventEnvelope, RebuildCause};
use geist_world::ChunkCoord;

impl App {
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
                self.handle_structure_pose_updated(id, pos, yaw_deg, delta, velocity);
            }
            Event::MovementRequested {
                dt_ms,
                yaw,
                walk_mode,
            } => {
                self.handle_movement_requested(rl, thread, dt_ms, yaw, walk_mode);
            }
            Event::PlayerAttachedToStructure { id, local_offset } => {
                self.handle_player_attached_to_structure(id, local_offset);
            }
            Event::PlayerDetachedFromStructure { id } => {
                self.handle_player_detached_from_structure(id);
            }
            Event::ViewCenterChanged { ccx, ccy, ccz } => {
                self.handle_view_center_changed(ccx, ccy, ccz);
            }
            Event::EnsureChunkUnloaded { cx, cy, cz } => {
                let coord = ChunkCoord::new(cx, cy, cz);
                self.handle_ensure_chunk_unloaded(coord);
            }
            Event::EnsureChunkLoaded { cx, cy, cz } => {
                let coord = ChunkCoord::new(cx, cy, cz);
                self.handle_ensure_chunk_loaded(coord);
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
                self.handle_build_chunk_job_requested(coord, neighbors, rev, job_id, cause);
            }
            Event::StructureBuildRequested { id, rev } => {
                self.handle_structure_build_requested(id, rev);
            }
            Event::StructureBuildCompleted {
                id,
                rev,
                cpu,
                light_grid,
                light_borders,
            } => {
                self.handle_structure_build_completed(
                    rl,
                    thread,
                    id,
                    rev,
                    cpu,
                    light_grid,
                    light_borders,
                );
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
                self.handle_build_chunk_job_completed(
                    rl,
                    thread,
                    coord,
                    rev,
                    occupancy,
                    cpu,
                    buf,
                    light_borders,
                    light_grid,
                    column_profile,
                );
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
                self.handle_chunk_lighting_recomputed(rl, thread, coord, rev, light_grid);
            }
            Event::ChunkRebuildRequested { cx, cy, cz, cause } => {
                let coord = ChunkCoord::new(cx, cy, cz);
                self.handle_chunk_rebuild_requested(coord, cause);
            }
            Event::RaycastEditRequested { place, block } => {
                self.handle_raycast_edit_requested(place, block);
            }
            Event::StructureBlockPlaced {
                id,
                lx,
                ly,
                lz,
                block,
            } => {
                self.handle_structure_block_placed(id, lx, ly, lz, block);
            }
            Event::StructureBlockRemoved { id, lx, ly, lz } => {
                self.handle_structure_block_removed(id, lx, ly, lz);
            }
            Event::BlockPlaced { wx, wy, wz, block } => {
                self.handle_block_placed(wx, wy, wz, block);
            }
            Event::BlockRemoved { wx, wy, wz } => {
                self.handle_block_removed(wx, wy, wz);
            }
            Event::LightEmitterAdded {
                wx,
                wy,
                wz,
                level,
                is_beacon,
            } => {
                self.handle_light_emitter_added(wx, wy, wz, level, is_beacon);
            }
            Event::LightEmitterRemoved { wx, wy, wz } => {
                self.handle_light_emitter_removed(wx, wy, wz);
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
                self.handle_light_borders_updated(
                    coord, xn_changed, xp_changed, yn_changed, yp_changed, zn_changed, zp_changed,
                );
            }
            Event::WalkModeToggled => {
                self.handle_walk_mode_toggled();
            }
            Event::GridToggled => {
                self.handle_grid_toggle();
            }
            Event::WireframeToggled => {
                self.handle_wireframe_toggle();
            }
            Event::ChunkBoundsToggled => {
                self.handle_chunk_bounds_toggle();
            }
            Event::FrustumCullingToggled => {
                self.handle_frustum_culling_toggle();
            }
            Event::BiomeLabelToggled => {
                self.handle_biome_label_toggle();
            }
            Event::DebugOverlayToggled => {
                self.handle_debug_overlay_toggle();
            }
            Event::PlaceTypeSelected { block } => {
                self.handle_place_type_selected(block);
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
        let coords = helpers::spherical_chunk_coords(center, 2);
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
        let coords = helpers::spherical_chunk_coords(center, 1);
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

    #[test]
    fn classify_edit_rebuild_cause_marks_vertical_neighbors_for_lighting() {
        let origin = ChunkCoord::new(4, 8, 2);
        assert_eq!(
            App::classify_edit_rebuild_cause(origin, origin),
            Some(RebuildCause::Edit)
        );
        assert_eq!(
            App::classify_edit_rebuild_cause(origin, origin.offset(1, 0, 0)),
            Some(RebuildCause::Edit)
        );
        assert_eq!(
            App::classify_edit_rebuild_cause(origin, origin.offset(0, 0, -1)),
            Some(RebuildCause::Edit)
        );
        assert_eq!(
            App::classify_edit_rebuild_cause(origin, origin.offset(0, 1, 0)),
            Some(RebuildCause::LightingBorder)
        );
        assert_eq!(
            App::classify_edit_rebuild_cause(origin, origin.offset(-1, 1, 0)),
            Some(RebuildCause::LightingBorder)
        );
    }
}

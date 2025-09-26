use super::App;
use crate::event::Event;

impl App {
    pub(super) fn log_event(tick: u64, ev: &Event) {
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
                log::trace!(
                    target: "events",
                    "[tick {}] MovementRequested dt_ms={} yaw={:.1} mode={}",
                    tick,
                    dt_ms,
                    yaw,
                    if *walk_mode { "walk" } else { "fly" }
                );
            }
            E::RaycastEditRequested { place, block } => {
                log::info!(
                    target: "events",
                    "[tick {}] RaycastEditRequested {} block={:?}",
                    tick,
                    if *place { "place" } else { "remove" },
                    block
                );
            }
            E::BlockPlaced { wx, wy, wz, block } => {
                log::info!(
                    target: "events",
                    "[tick {}] BlockPlaced ({},{},{}) block={:?}",
                    tick,
                    wx,
                    wy,
                    wz,
                    block
                );
            }
            E::BlockRemoved { wx, wy, wz } => {
                log::info!(
                    target: "events",
                    "[tick {}] BlockRemoved ({},{},{})",
                    tick,
                    wx,
                    wy,
                    wz
                );
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
                log::trace!(
                    target: "events",
                    "[tick {}] StructurePoseUpdated id={} pos=({:.2},{:.2},{:.2}) yaw={:.1} delta=({:.2},{:.2},{:.2}) vel=({:.2},{:.2},{:.2})",
                    tick,
                    id,
                    pos.x,
                    pos.y,
                    pos.z,
                    yaw_deg,
                    delta.x,
                    delta.y,
                    delta.z,
                    velocity.x,
                    velocity.y,
                    velocity.z
                );
            }
            E::StructureBlockPlaced {
                id,
                lx,
                ly,
                lz,
                block,
            } => {
                log::info!(
                    target: "events",
                    "[tick {}] StructureBlockPlaced id={} ({},{},{}) block={:?}",
                    tick,
                    id,
                    lx,
                    ly,
                    lz,
                    block
                );
            }
            E::StructureBlockRemoved { id, lx, ly, lz } => {
                log::info!(
                    target: "events",
                    "[tick {}] StructureBlockRemoved id={} ({},{},{})",
                    tick,
                    id,
                    lx,
                    ly,
                    lz
                );
            }
            E::PlayerAttachedToStructure { id, local_offset } => {
                log::info!(
                    target: "events",
                    "[tick {}] PlayerAttachedToStructure id={} local=({:.2},{:.2},{:.2})",
                    tick,
                    id,
                    local_offset.x,
                    local_offset.y,
                    local_offset.z
                );
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
                log::info!(
                    target: "events",
                    "[tick {}] LightEmitterAdded ({},{},{}) level={} beacon={}",
                    tick,
                    wx,
                    wy,
                    wz,
                    level,
                    is_beacon
                );
            }
            E::LightEmitterRemoved { wx, wy, wz } => {
                log::info!(
                    target: "events",
                    "[tick {}] LightEmitterRemoved ({},{},{})",
                    tick,
                    wx,
                    wy,
                    wz
                );
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

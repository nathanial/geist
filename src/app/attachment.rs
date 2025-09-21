use geist_geom::Vec3;
use geist_structures::{rotate_yaw, rotate_yaw_inv};

type Degrees = f32;

/// Convert a structure-local position into world space using the provided pose.
#[inline]
pub fn structure_local_to_world(local: Vec3, pose_pos: Vec3, pose_yaw: Degrees) -> Vec3 {
    rotate_yaw(local, pose_yaw) + pose_pos
}

/// Convert a world-space position into structure-local coordinates using the provided pose.
#[inline]
pub fn structure_world_to_local(world: Vec3, pose_pos: Vec3, pose_yaw: Degrees) -> Vec3 {
    let diff = world - pose_pos;
    rotate_yaw_inv(diff, pose_yaw)
}

/// Helper to compute the world position for an attached player using the stored frame data.
#[inline]
pub fn attachment_world_position(att: &crate::gamestate::GroundAttach) -> Vec3 {
    structure_local_to_world(att.local_offset, att.pose_pos, att.pose_yaw_deg)
}

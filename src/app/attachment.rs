use crate::gamestate::GroundAttach;
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
pub fn attachment_world_position(att: &GroundAttach) -> Vec3 {
    structure_local_to_world(att.local_offset, att.pose_pos, att.pose_yaw_deg)
}

/// Rotate a vector from structure-local space into world space.
#[inline]
pub fn structure_local_vec_to_world(local: Vec3, pose_yaw: Degrees) -> Vec3 {
    rotate_yaw(local, pose_yaw)
}

/// Combine the structure's own velocity with the rider's local-relative velocity to get world motion.
#[inline]
pub fn attachment_world_velocity(att: &GroundAttach) -> Vec3 {
    let local = att.local_velocity.unwrap_or(Vec3::ZERO);
    structure_local_vec_to_world(local, att.pose_yaw_deg) + att.structure_velocity
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_local_world_position() {
        let pose_pos = Vec3::new(10.0, 5.0, -2.0);
        let yaw = 90.0;
        let local = Vec3::new(1.0, 2.0, 3.0);
        let world = structure_local_to_world(local, pose_pos, yaw);
        let back = structure_world_to_local(world, pose_pos, yaw);
        assert!((back.x - local.x).abs() < 1e-5);
        assert!((back.y - local.y).abs() < 1e-5);
        assert!((back.z - local.z).abs() < 1e-5);
    }

    #[test]
    fn attachment_velocity_combines_structure_motion() {
        let att = GroundAttach {
            id: 42,
            grace: 8,
            local_offset: Vec3::ZERO,
            pose_pos: Vec3::ZERO,
            pose_yaw_deg: 45.0,
            local_velocity: Some(Vec3::new(1.0, 0.0, 0.0)),
            structure_velocity: Vec3::new(0.0, 2.0, 0.0),
        };
        let vel = attachment_world_velocity(&att);
        assert!((vel.x - 0.70710677).abs() < 1e-5);
        assert!((vel.y - 2.0).abs() < 1e-5);
        assert!((vel.z - 0.70710677).abs() < 1e-5);
    }
}

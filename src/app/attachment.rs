use crate::gamestate::StructureAnchor;
use geist_geom::Vec3;
#[cfg(test)]
use geist_structures::rotate_yaw;
use geist_structures::{Structure, rotate_yaw_inv};

type Degrees = f32;

/// Convert a structure-local position into world space using the provided pose.
#[cfg(test)]
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

/// Compute the world position of an anchor relative to a structure pose.
#[inline]
pub fn anchor_world_position(anchor: &StructureAnchor, structure: &Structure) -> Vec3 {
    anchor.world_position(structure)
}

/// Combine the structure's own velocity with the rider's local-relative velocity to get world motion.
#[inline]
pub fn anchor_world_velocity(anchor: &StructureAnchor, structure: &Structure) -> Vec3 {
    anchor.world_velocity(structure)
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
    fn anchor_velocity_combines_structure_motion() {
        use geist_blocks::BlockRegistry;
        use geist_structures::{Pose, Structure};

        let reg = BlockRegistry::new();
        let mut structure = Structure::new(
            1,
            2,
            2,
            2,
            Pose {
                pos: Vec3::ZERO,
                yaw_deg: 45.0,
            },
            &reg,
        );
        structure.last_velocity = Vec3::new(0.0, 2.0, 0.0);

        let mut anchor = StructureAnchor::new(1, Vec3::ZERO, 0.0);
        anchor.update_local_velocity(Vec3::new(1.0, 0.0, 0.0));

        let vel = anchor_world_velocity(&anchor, &structure);
        assert!((vel.x - 0.70710677).abs() < 1e-5);
        assert!((vel.y - 2.0).abs() < 1e-5);
        assert!((vel.z - 0.70710677).abs() < 1e-5);
    }
}

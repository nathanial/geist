use crate::gamestate::StructureAnchor;
use geist_blocks::Block;
use geist_geom::Vec3;
use geist_structures::{Structure, rotate_yaw, rotate_yaw_inv};

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

/// Build a sampler that prefers structure-local occupancy data and falls back to a world sampler.
pub fn structure_local_sampler<'a, F>(
    structure: &'a Structure,
    fallback: F,
) -> impl Fn(i32, i32, i32) -> Block + 'a
where
    F: Fn(i32, i32, i32) -> Block + 'a,
{
    move |lx: i32, ly: i32, lz: i32| {
        if lx >= 0 && ly >= 0 && lz >= 0 {
            let (ux, uy, uz) = (lx as usize, ly as usize, lz as usize);
            if ux < structure.sx && uy < structure.sy && uz < structure.sz {
                if let Some(edit) = structure.edits.get(lx, ly, lz) {
                    return edit;
                }
                return structure.blocks[structure.idx(ux, uy, uz)];
            }
        }

        // Translate the local cell center back into world space for fallback sampling.
        let local_center = Vec3::new(lx as f32 + 0.5, ly as f32 + 0.5, lz as f32 + 0.5);
        let world_center = rotate_yaw(local_center, structure.pose.yaw_deg) + structure.pose.pos;
        let wx = world_center.x.floor() as i32;
        let wy = world_center.y.floor() as i32;
        let wz = world_center.z.floor() as i32;
        fallback(wx, wy, wz)
    }
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

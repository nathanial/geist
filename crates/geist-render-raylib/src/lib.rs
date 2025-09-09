//! Raylib-based GPU rendering utilities and conversions.
#![forbid(unsafe_code)]

pub mod conv {
    use geist_geom::{Aabb, Vec3};

    pub fn vec3_to_rl(v: Vec3) -> raylib::prelude::Vector3 {
        raylib::prelude::Vector3::new(v.x, v.y, v.z)
    }

    pub fn vec3_from_rl(v: raylib::prelude::Vector3) -> Vec3 {
        Vec3 { x: v.x, y: v.y, z: v.z }
    }

    pub fn aabb_to_rl(bb: Aabb) -> raylib::core::math::BoundingBox {
        raylib::core::math::BoundingBox::new(vec3_to_rl(bb.min), vec3_to_rl(bb.max))
    }

    pub fn aabb_from_rl(bb: raylib::core::math::BoundingBox) -> Aabb {
        Aabb { min: Vec3 { x: bb.min.x, y: bb.min.y, z: bb.min.z }, max: Vec3 { x: bb.max.x, y: bb.max.y, z: bb.max.z } }
    }
}

// Phase 4 will move GPU upload + shaders into this crate.

//! Minimal geometry types for engine crates.
#![forbid(unsafe_code)]

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}


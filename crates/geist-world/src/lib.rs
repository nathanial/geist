//! World sizing, sampling, and worldgen parameters.
#![forbid(unsafe_code)]

pub mod voxel;
pub mod worldgen;

pub use voxel::{CHUNK_SIZE, GenCtx, ShowcaseEntry, ShowcasePlacement, World, WorldGenMode};

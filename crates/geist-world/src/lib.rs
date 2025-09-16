//! World sizing, sampling, and worldgen parameters.
#![forbid(unsafe_code)]

pub mod voxel;
pub mod worldgen;

pub use voxel::{
    CHUNK_SIZE, ChunkCoord, GenCtx, ShowcaseEntry, ShowcasePlacement, World, WorldGenMode,
};

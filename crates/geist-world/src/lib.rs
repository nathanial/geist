//! World sizing, sampling, and worldgen parameters.
#![forbid(unsafe_code)]

pub mod voxel;
pub mod worldgen;

pub use voxel::{
    CHUNK_SIZE, ChunkCoord, GenCtx, HeightTileStats, TERRAIN_STAGE_COUNT, TERRAIN_STAGE_LABELS,
    TerrainMetrics, TerrainStage, TerrainStageSample, World, WorldGenMode,
};

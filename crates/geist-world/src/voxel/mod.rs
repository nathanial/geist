pub const CHUNK_SIZE: usize = 64;

mod chunk_coord;
mod gen_ctx;
mod generation;
mod world;

pub use chunk_coord::ChunkCoord;
pub use gen_ctx::{
    ChunkTiming, GenCtx, HeightTileStats, TERRAIN_STAGE_COUNT, TERRAIN_STAGE_LABELS,
    TerrainMetrics, TerrainProfiler, TerrainStage, TerrainStageSample,
};
pub use world::{World, WorldGenMode};

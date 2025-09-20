use std::sync::Arc;

use fastnoise_lite::FastNoiseLite;

use crate::worldgen::WorldGenParams;

pub struct GenCtx {
    pub terrain: FastNoiseLite,
    pub warp: FastNoiseLite,
    pub tunnel: FastNoiseLite,
    pub params: Arc<WorldGenParams>,
    pub temp2d: Option<FastNoiseLite>,
    pub moist2d: Option<FastNoiseLite>,
    pub height_tile_stats: HeightTileStats,
    pub height_tile: Option<HeightTile>,
    pub terrain_profiler: TerrainProfiler,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct HeightTileStats {
    pub duration_us: u32,
    pub columns: u32,
    pub reused: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TerrainStageSample {
    pub time_us: u32,
    pub calls: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ChunkTiming {
    pub total_us: u32,
    pub height_tile_us: u32,
    pub voxel_fill_us: u32,
    pub feature_us: u32,
}

#[derive(Clone, Debug, Default)]
pub struct TerrainMetrics {
    pub height_tile: HeightTileStats,
    pub stages: [TerrainStageSample; TERRAIN_STAGE_COUNT],
    pub height_cache_hits: u32,
    pub height_cache_misses: u32,
    pub chunk_timing: ChunkTiming,
}

#[derive(Clone, Copy, Debug)]
pub enum TerrainStage {
    Block,
    Tower,
    Height,
    Surface,
    Water,
    Caves,
    Trees,
}

pub const TERRAIN_STAGE_COUNT: usize = TerrainStage::Trees as usize + 1;
pub const TERRAIN_STAGE_LABELS: [&str; TERRAIN_STAGE_COUNT] = [
    "Block", "Tower", "Height", "Surface", "Water", "Caves", "Trees",
];

#[derive(Clone, Debug, Default)]
pub struct TerrainProfiler {
    stage_time_ns: [u128; TERRAIN_STAGE_COUNT],
    stage_calls: [u32; TERRAIN_STAGE_COUNT],
    height_cache_hits: u32,
    height_cache_misses: u32,
}

pub struct HeightTile {
    base_x: i32,
    base_z: i32,
    size_x: usize,
    size_z: usize,
    heights: Vec<i32>,
}

impl HeightTile {
    pub fn new(base_x: i32, base_z: i32, size_x: usize, size_z: usize, heights: Vec<i32>) -> Self {
        debug_assert_eq!(heights.len(), size_x * size_z);
        Self {
            base_x,
            base_z,
            size_x,
            size_z,
            heights,
        }
    }

    #[inline]
    pub fn matches(&self, base_x: i32, base_z: i32, size_x: usize, size_z: usize) -> bool {
        self.base_x == base_x
            && self.base_z == base_z
            && self.size_x == size_x
            && self.size_z == size_z
    }

    #[inline]
    fn index(&self, wx: i32, wz: i32) -> Option<usize> {
        let dx = wx - self.base_x;
        let dz = wz - self.base_z;
        if dx < 0 || dz < 0 {
            return None;
        }
        let (dx, dz) = (dx as usize, dz as usize);
        if dx >= self.size_x || dz >= self.size_z {
            return None;
        }
        Some(dz * self.size_x + dx)
    }

    #[inline]
    pub fn height(&self, wx: i32, wz: i32) -> Option<i32> {
        self.index(wx, wz).map(|idx| self.heights[idx])
    }
}

impl TerrainStage {
    #[inline]
    fn idx(self) -> usize {
        self as usize
    }
}

impl TerrainProfiler {
    #[inline]
    pub fn reset(&mut self) {
        self.stage_time_ns = [0; TERRAIN_STAGE_COUNT];
        self.stage_calls = [0; TERRAIN_STAGE_COUNT];
        self.height_cache_hits = 0;
        self.height_cache_misses = 0;
    }

    #[inline]
    pub fn begin_stage(&mut self, stage: TerrainStage) {
        let idx = stage.idx();
        self.stage_calls[idx] = self.stage_calls[idx].saturating_add(1);
    }

    #[inline]
    pub fn record_stage_duration(&mut self, stage: TerrainStage, elapsed: std::time::Duration) {
        let idx = stage.idx();
        self.stage_time_ns[idx] = self.stage_time_ns[idx].saturating_add(elapsed.as_nanos());
    }

    #[inline]
    pub fn record_height_cache(&mut self, hit: bool) {
        if hit {
            self.height_cache_hits = self.height_cache_hits.saturating_add(1);
        } else {
            self.height_cache_misses = self.height_cache_misses.saturating_add(1);
        }
    }

    pub fn snapshot(&mut self, height_tile: HeightTileStats) -> TerrainMetrics {
        fn to_us(value: u128) -> u32 {
            let micros = value / 1_000; // convert from ns to µs
            if micros > u128::from(u32::MAX) {
                u32::MAX
            } else {
                micros as u32
            }
        }

        let mut stages: [TerrainStageSample; TERRAIN_STAGE_COUNT] =
            [TerrainStageSample::default(); TERRAIN_STAGE_COUNT];
        for idx in 0..TERRAIN_STAGE_COUNT {
            stages[idx] = TerrainStageSample {
                time_us: to_us(self.stage_time_ns[idx]),
                calls: self.stage_calls[idx],
            };
        }
        let metrics = TerrainMetrics {
            height_tile,
            stages,
            height_cache_hits: self.height_cache_hits,
            height_cache_misses: self.height_cache_misses,
            chunk_timing: ChunkTiming::default(),
        };
        self.reset();
        metrics
    }
}

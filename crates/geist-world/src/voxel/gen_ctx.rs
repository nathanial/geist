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
}

#[derive(Clone, Copy, Debug, Default)]
pub struct HeightTileStats {
    pub duration_us: u32,
    pub columns: u32,
    pub reused: bool,
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

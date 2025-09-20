//! Chunk buffer and world generation helpers.
#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

use geist_blocks::BlockRegistry;
use geist_blocks::types::Block;
use geist_world::{ChunkCoord, ChunkTiming, GenCtx, TerrainMetrics, TerrainStage, World};

#[derive(Clone, Debug)]
pub struct ChunkBuf {
    pub coord: ChunkCoord,
    pub sx: usize,
    pub sy: usize,
    pub sz: usize,
    pub blocks: Vec<Block>,
}

impl ChunkBuf {
    #[inline]
    pub fn idx(&self, x: usize, y: usize, z: usize) -> usize {
        (y * self.sz + z) * self.sx + x
    }

    #[inline]
    pub fn get_local(&self, x: usize, y: usize, z: usize) -> Block {
        self.blocks[self.idx(x, y, z)]
    }

    #[inline]
    pub fn contains_world(&self, wx: i32, wy: i32, wz: i32) -> bool {
        let base_x = self.coord.cx * self.sx as i32;
        let base_y = self.coord.cy * self.sy as i32;
        let base_z = self.coord.cz * self.sz as i32;
        if wy < base_y || wy >= base_y + self.sy as i32 {
            return false;
        }
        wx >= base_x && wx < base_x + self.sx as i32 && wz >= base_z && wz < base_z + self.sz as i32
    }

    #[inline]
    pub fn get_world(&self, wx: i32, wy: i32, wz: i32) -> Option<Block> {
        if !self.contains_world(wx, wy, wz) {
            return None;
        }
        let base_x = self.coord.cx * self.sx as i32;
        let base_y = self.coord.cy * self.sy as i32;
        let base_z = self.coord.cz * self.sz as i32;
        let lx = (wx - base_x) as usize;
        let ly = (wy - base_y) as usize;
        let lz = (wz - base_z) as usize;
        Some(self.get_local(lx, ly, lz))
    }

    pub fn from_blocks_local(
        coord: ChunkCoord,
        sx: usize,
        sy: usize,
        sz: usize,
        blocks: Vec<Block>,
    ) -> Self {
        let mut b = blocks;
        let expect = sx * sy * sz;
        if b.len() != expect {
            b.resize(expect, Block { id: 0, state: 0 });
        }
        ChunkBuf {
            coord,
            sx,
            sy,
            sz,
            blocks: b,
        }
    }

    #[inline]
    pub fn has_non_air(&self) -> bool {
        self.blocks.iter().any(|b| *b != Block::AIR)
    }

    #[inline]
    pub fn is_all_air(&self) -> bool {
        !self.has_non_air()
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ChunkOccupancy {
    Empty,
    Populated,
}

impl ChunkOccupancy {
    #[inline]
    pub fn is_empty(self) -> bool {
        matches!(self, ChunkOccupancy::Empty)
    }

    #[inline]
    pub fn has_blocks(self) -> bool {
        matches!(self, ChunkOccupancy::Populated)
    }
}

#[derive(Clone, Debug)]
pub struct ChunkGenerateResult {
    pub buf: ChunkBuf,
    pub occupancy: ChunkOccupancy,
    pub terrain_metrics: TerrainMetrics,
}

#[inline]
fn duration_to_us(duration: Duration) -> u32 {
    duration.as_micros().min(u128::from(u32::MAX)) as u32
}

pub fn generate_chunk_buffer(
    world: &World,
    coord: ChunkCoord,
    reg: &BlockRegistry,
) -> ChunkGenerateResult {
    let mut ctx = world.make_gen_ctx();
    generate_chunk_buffer_with_ctx(world, coord, reg, &mut ctx)
}

pub fn generate_chunk_buffer_with_ctx(
    world: &World,
    coord: ChunkCoord,
    reg: &BlockRegistry,
    ctx: &mut GenCtx,
) -> ChunkGenerateResult {
    ctx.terrain_profiler.reset();

    let total_start = Instant::now();
    let sx = world.chunk_size_x;
    let sy = world.chunk_size_y;
    let sz = world.chunk_size_z;
    let mut blocks = Vec::with_capacity(sx * sy * sz);
    blocks.resize(sx * sy * sz, Block { id: 0, state: 0 });
    let base_x = coord.cx * sx as i32;
    let base_y = coord.cy * sy as i32;
    let base_z = coord.cz * sz as i32;

    let tile_start = Instant::now();
    world.prepare_height_tile(ctx, base_x, base_z, sx, sz);
    let height_tile_us = duration_to_us(tile_start.elapsed());

    let mut has_blocks = false;
    let fill_start = Instant::now();
    for z in 0..sz {
        for y in 0..sy {
            for x in 0..sx {
                let wx = base_x + x as i32;
                let wy = base_y + y as i32;
                let wz = base_z + z as i32;
                let block = world.block_at_runtime_with(reg, ctx, wx, wy, wz);
                if block != Block::AIR {
                    has_blocks = true;
                }
                blocks[(y * sz + z) * sx + x] = block;
            }
        }
    }
    let voxel_fill_us = duration_to_us(fill_start.elapsed());

    let mut chunk_timing = ChunkTiming {
        total_us: duration_to_us(total_start.elapsed()),
        height_tile_us,
        voxel_fill_us,
        feature_us: 0,
    };

    let mut metrics = ctx.terrain_profiler.snapshot(ctx.height_tile_stats);
    let feature_us = metrics.stages[TerrainStage::Caves as usize]
        .time_us
        .saturating_add(metrics.stages[TerrainStage::Trees as usize].time_us);
    chunk_timing.feature_us = feature_us;
    metrics.chunk_timing = chunk_timing;

    ChunkGenerateResult {
        buf: ChunkBuf {
            coord,
            sx,
            sy,
            sz,
            blocks,
        },
        occupancy: if has_blocks {
            ChunkOccupancy::Populated
        } else {
            ChunkOccupancy::Empty
        },
        terrain_metrics: metrics,
    }
}

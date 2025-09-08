use crate::blocks::Block;
use crate::blocks::BlockRegistry;
use crate::voxel::World;

#[derive(Clone, Debug)]
pub struct ChunkBuf {
    pub cx: i32,
    pub cz: i32,
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
        if wy < 0 || wy >= self.sy as i32 {
            return false;
        }
        let x0 = self.cx * self.sx as i32;
        let z0 = self.cz * self.sz as i32;
        wx >= x0 && wx < x0 + self.sx as i32 && wz >= z0 && wz < z0 + self.sz as i32
    }

    #[inline]
    pub fn get_world(&self, wx: i32, wy: i32, wz: i32) -> Option<Block> {
        if !self.contains_world(wx, wy, wz) {
            return None;
        }
        if wy < 0 || wy >= self.sy as i32 {
            return Some(Block::AIR);
        }
        let x0 = self.cx * self.sx as i32;
        let z0 = self.cz * self.sz as i32;
        let lx = (wx - x0) as usize;
        let lz = (wz - z0) as usize;
        Some(self.get_local(lx, wy as usize, lz))
    }

    pub fn from_blocks_local(
        cx: i32,
        cz: i32,
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
            cx,
            cz,
            sx,
            sy,
            sz,
            blocks: b,
        }
    }
}

pub fn generate_chunk_buffer(world: &World, cx: i32, cz: i32, reg: &BlockRegistry) -> ChunkBuf {
    let sx = world.chunk_size_x;
    let sy = world.chunk_size_y;
    let sz = world.chunk_size_z;
    let mut blocks = Vec::with_capacity(sx * sy * sz);
    blocks.resize(sx * sy * sz, Block { id: 0, state: 0 });
    let x0 = cx * sx as i32;
    let z0 = cz * sz as i32;
    // Use reusable worldgen context per chunk to avoid heavy per-voxel allocations
    let mut ctx = world.make_gen_ctx();
    for z in 0..sz {
        for y in 0..sy {
            for x in 0..sx {
                let wx = x0 + x as i32;
                let wy = y as i32;
                let wz = z0 + z as i32;
                blocks[(y * sz + z) * sx + x] =
                    world.block_at_runtime_with(reg, &mut ctx, wx, wy, wz);
            }
        }
    }
    ChunkBuf {
        cx,
        cz,
        sx,
        sy,
        sz,
        blocks,
    }
}

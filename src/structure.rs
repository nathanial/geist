use raylib::prelude::Vector3;
use std::collections::HashMap;

use crate::voxel::Block;

pub type StructureId = u32;

#[derive(Clone)]
pub struct Pose {
    pub pos: Vector3,
    pub yaw_deg: f32,
}

pub struct Structure {
    pub id: StructureId,
    pub sx: usize,
    pub sy: usize,
    pub sz: usize,
    pub blocks: Vec<Block>,
    pub edits: StructureEditStore,
    pub pose: Pose,
    pub last_delta: Vector3,
    pub dirty_rev: u64,
    pub built_rev: u64,
}

impl Structure {
    pub fn new(id: StructureId, sx: usize, sy: usize, sz: usize, pose: Pose) -> Self {
        let mut blocks = vec![Block::Air; sx * sy * sz];
        // Simple starter deck: stone floor slab at 1/3 height, with glow beacons for visibility
        let deck_y = (sy as f32 * 0.33) as usize;
        for z in 0..sz {
            for x in 0..sx {
                // Use proper 3D indexing: (y * sz + z) * sx + x
                let idx = (deck_y * sz + z) * sx + x;
                blocks[idx] = Block::Stone;
            }
        }
        // Place a few beacons at corners of the deck
        for &(x, z) in &[(1usize, 1usize), (sx - 2, 1), (1, sz - 2), (sx - 2, sz - 2)] {
            let idx = (deck_y * sz + z) * sx + x;
            blocks[idx] = Block::Beacon;
        }

        Self {
            id,
            sx,
            sy,
            sz,
            blocks,
            edits: StructureEditStore::new(),
            pose,
            last_delta: Vector3::zero(),
            dirty_rev: 1,
            built_rev: 0,
        }
    }

    #[inline]
    pub fn idx(&self, x: usize, y: usize, z: usize) -> usize {
        (y * self.sz + z) * self.sx + x
    }

    pub fn set_local(&mut self, lx: i32, ly: i32, lz: i32, b: Block) {
        if lx < 0 || ly < 0 || lz < 0 {
            return;
        }
        let (lxu, lyu, lzu) = (lx as usize, ly as usize, lz as usize);
        if lxu >= self.sx || lyu >= self.sy || lzu >= self.sz {
            return;
        }
        self.edits.set(lx, ly, lz, b);
        self.bump_rev();
    }

    pub fn remove_local(&mut self, lx: i32, ly: i32, lz: i32) {
        if lx < 0 || ly < 0 || lz < 0 {
            return;
        }
        let (lxu, lyu, lzu) = (lx as usize, ly as usize, lz as usize);
        if lxu >= self.sx || lyu >= self.sy || lzu >= self.sz {
            return;
        }
        self.edits.set(lx, ly, lz, Block::Air);
        self.bump_rev();
    }

    fn bump_rev(&mut self) {
        self.dirty_rev = self.dirty_rev.wrapping_add(1).max(1);
    }
}

pub struct StructureEditStore {
    inner: HashMap<(i32, i32, i32), Block>,
}

impl StructureEditStore {
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }

    pub fn get(&self, lx: i32, ly: i32, lz: i32) -> Option<Block> {
        self.inner.get(&(lx, ly, lz)).copied()
    }

    pub fn set(&mut self, lx: i32, ly: i32, lz: i32, b: Block) {
        self.inner.insert((lx, ly, lz), b);
    }

    pub fn snapshot_all(&self) -> Vec<((i32, i32, i32), Block)> {
        self.inner.iter().map(|(k, v)| (*k, *v)).collect()
    }
}

// Utility: rotate a vector by yaw degrees (Y axis), preserving Y
#[inline]
pub fn rotate_yaw(v: Vector3, yaw_deg: f32) -> Vector3 {
    let r = yaw_deg.to_radians();
    let (s, c) = r.sin_cos();
    Vector3::new(v.x * c - v.z * s, v.y, v.x * s + v.z * c)
}

#[inline]
pub fn rotate_yaw_inv(v: Vector3, yaw_deg: f32) -> Vector3 {
    rotate_yaw(v, -yaw_deg)
}

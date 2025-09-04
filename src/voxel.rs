use fastnoise_lite::{FastNoiseLite, NoiseType};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Block {
    Air,
    Grass,
    Dirt,
    Stone,
}

impl Block {
    #[inline]
    pub fn is_solid(&self) -> bool {
        !matches!(self, Block::Air)
    }
}

pub struct Chunk {
    pub size_x: usize,
    pub size_y: usize,
    pub size_z: usize,
    pub blocks: Vec<Block>,
}

impl Chunk {
    pub fn new(size_x: usize, size_y: usize, size_z: usize) -> Self {
        Self {
            size_x,
            size_y,
            size_z,
            blocks: vec![Block::Air; size_x * size_y * size_z],
        }
    }

    #[inline]
    pub fn idx(&self, x: usize, y: usize, z: usize) -> usize {
        (y * self.size_z + z) * self.size_x + x
    }

    #[inline]
    pub fn in_bounds(&self, x: i32, y: i32, z: i32) -> bool {
        x >= 0
            && y >= 0
            && z >= 0
            && (x as usize) < self.size_x
            && (y as usize) < self.size_y
            && (z as usize) < self.size_z
    }

    pub fn set(&mut self, x: usize, y: usize, z: usize, b: Block) {
        let i = self.idx(x, y, z);
        self.blocks[i] = b;
    }

    pub fn get(&self, x: usize, y: usize, z: usize) -> Block {
        self.blocks[self.idx(x, y, z)]
    }

    pub fn is_exposed(&self, x: usize, y: usize, z: usize) -> bool {
        // If any neighbor is Air or out-of-bounds, this block is exposed
        let dirs = [
            (1, 0, 0),
            (-1, 0, 0),
            (0, 1, 0),
            (0, -1, 0),
            (0, 0, 1),
            (0, 0, -1),
        ];
        for (dx, dy, dz) in dirs {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            let nz = z as i32 + dz;
            if !self.in_bounds(nx, ny, nz) {
                return true;
            }
            let (nxu, nyu, nzu) = (nx as usize, ny as usize, nz as usize);
            if !self.get(nxu, nyu, nzu).is_solid() {
                return true;
            }
        }
        false
    }
}

pub fn generate_heightmap_chunk(size_x: usize, size_y: usize, size_z: usize, seed: i32) -> Chunk {
    let mut chunk = Chunk::new(size_x, size_y, size_z);

    // 2D noise for heightmap
    let mut noise = FastNoiseLite::with_seed(seed);
    noise.set_noise_type(Some(NoiseType::OpenSimplex2));
    noise.set_frequency(Some(0.02));

    for z in 0..size_z {
        for x in 0..size_x {
            let nx = x as f32;
            let nz = z as f32;
            let h = noise.get_noise_2d(nx, nz);
            // map [-1,1] -> [min_h, max_h]
            let min_h = (size_y as f32 * 0.15) as i32;
            let max_h = (size_y as f32 * 0.7) as i32;
            let hh = ((h + 1.0) * 0.5 * (max_h - min_h) as f32) as i32 + min_h;
            let height = hh.clamp(1, size_y as i32 - 1) as usize;
            for y in 0..height {
                let b = if y == height - 1 {
                    Block::Grass
                } else if y + 3 >= height {
                    Block::Dirt
                } else {
                    Block::Stone
                };
                chunk.set(x, y, z, b);
            }
        }
    }

    chunk
}

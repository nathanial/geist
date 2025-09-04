use fastnoise_lite::{FastNoiseLite, NoiseType};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Block {
    Air,
    Grass,
    Dirt,
    Stone,
    Sand,
    Snow,
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
                    // surface choice: snow at high alt, sand at low alt, else grass
                    if height as f32 >= size_y as f32 * 0.62 {
                        Block::Snow
                    } else if height as f32 <= size_y as f32 * 0.2 {
                        Block::Sand
                    } else {
                        Block::Grass
                    }
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

pub fn generate_heightmap_chunk_at(
    size_x: usize,
    size_y: usize,
    size_z: usize,
    seed: i32,
    world_x0: i32,
    world_z0: i32,
) -> Chunk {
    let mut chunk = Chunk::new(size_x, size_y, size_z);

    let mut noise = FastNoiseLite::with_seed(seed);
    noise.set_noise_type(Some(NoiseType::OpenSimplex2));
    noise.set_frequency(Some(0.02));

    for z in 0..size_z {
        for x in 0..size_x {
            let nx = (world_x0 + x as i32) as f32;
            let nz = (world_z0 + z as i32) as f32;
            let h = noise.get_noise_2d(nx, nz);
            let min_h = (size_y as f32 * 0.15) as i32;
            let max_h = (size_y as f32 * 0.7) as i32;
            let hh = ((h + 1.0) * 0.5 * (max_h - min_h) as f32) as i32 + min_h;
            let height = hh.clamp(1, size_y as i32 - 1) as usize;
            for y in 0..height {
                let b = if y == height - 1 {
                    if height as f32 >= size_y as f32 * 0.62 {
                        Block::Snow
                    } else if height as f32 <= size_y as f32 * 0.2 {
                        Block::Sand
                    } else {
                        Block::Grass
                    }
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

pub struct World {
    pub chunk_size_x: usize,
    pub chunk_size_y: usize,
    pub chunk_size_z: usize,
    pub chunks_x: usize,
    pub chunks_z: usize,
    pub seed: i32,
    pub chunks: Vec<Chunk>,
}

impl World {
    pub fn new(chunks_x: usize, chunks_z: usize, chunk_size_x: usize, chunk_size_y: usize, chunk_size_z: usize, seed: i32) -> Self {
        let mut chunks = Vec::with_capacity(chunks_x * chunks_z);
        for cz in 0..chunks_z as i32 {
            for cx in 0..chunks_x as i32 {
                let x0 = cx * chunk_size_x as i32;
                let z0 = cz * chunk_size_z as i32;
                let ch = generate_heightmap_chunk_at(chunk_size_x, chunk_size_y, chunk_size_z, seed, x0, z0);
                chunks.push(ch);
            }
        }
        Self { chunk_size_x, chunk_size_y, chunk_size_z, chunks_x, chunks_z, seed, chunks }
    }

    #[inline]
    pub fn world_size_x(&self) -> usize { self.chunk_size_x * self.chunks_x }
    #[inline]
    pub fn world_size_y(&self) -> usize { self.chunk_size_y }
    #[inline]
    pub fn world_size_z(&self) -> usize { self.chunk_size_z * self.chunks_z }

    #[inline]
    pub fn in_bounds(&self, x: i32, y: i32, z: i32) -> bool {
        x >= 0 && y >= 0 && z >= 0 && (x as usize) < self.world_size_x() && (y as usize) < self.world_size_y() && (z as usize) < self.world_size_z()
    }

    #[inline]
    fn chunk_indices(&self, x: usize, z: usize) -> (usize, usize, usize, usize) {
        let cx = x / self.chunk_size_x;
        let cz = z / self.chunk_size_z;
        let lx = x % self.chunk_size_x;
        let lz = z % self.chunk_size_z;
        (cx, cz, lx, lz)
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize, z: usize) -> Block {
        let (cx, cz, lx, lz) = self.chunk_indices(x, z);
        let idx = cz * self.chunks_x + cx;
        if idx >= self.chunks.len() || y >= self.chunk_size_y { return Block::Air; }
        self.chunks[idx].get(lx, y, lz)
    }

    pub fn is_exposed(&self, x: usize, y: usize, z: usize) -> bool {
        // neighbors in 6 directions
        let dirs = [(1,0,0),(-1,0,0),(0,1,0),(0,-1,0),(0,0,1),(0,0,-1)];
        for (dx, dy, dz) in dirs {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            let nz = z as i32 + dz;
            if !self.in_bounds(nx, ny, nz) { return true; }
            let (nxu, nyu, nzu) = (nx as usize, ny as usize, nz as usize);
            if !self.get(nxu, nyu, nzu).is_solid() { return true; }
        }
        false
    }
}

impl World {
    // Procedural block at arbitrary world coordinates (x,z in world units, y in [0,chunk_size_y))
    pub fn block_at(&self, x: i32, y: i32, z: i32) -> Block {
        if y < 0 || y >= self.chunk_size_y as i32 { return Block::Air; }
        let mut noise = FastNoiseLite::with_seed(self.seed);
        noise.set_noise_type(Some(NoiseType::OpenSimplex2));
        noise.set_frequency(Some(0.02));
        let nx = x as f32;
        let nz = z as f32;
        let h = noise.get_noise_2d(nx, nz);
        let min_h = (self.chunk_size_y as f32 * 0.15) as i32;
        let max_h = (self.chunk_size_y as f32 * 0.7) as i32;
        let hh = ((h + 1.0) * 0.5 * (max_h - min_h) as f32) as i32 + min_h;
        let height = hh.clamp(1, self.chunk_size_y as i32 - 1) as i32;
        if y >= height { return Block::Air; }
        if y == height - 1 {
            if height as f32 >= self.chunk_size_y as f32 * 0.62 { Block::Snow }
            else if height as f32 <= self.chunk_size_y as f32 * 0.2 { Block::Sand }
            else { Block::Grass }
        } else if y + 3 >= height { Block::Dirt } else { Block::Stone }
    }
}

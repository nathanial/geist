use fastnoise_lite::{FastNoiseLite, NoiseType};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum TreeSpecies {
    Oak,
    Birch,
    Spruce,
    Jungle,
    Acacia,
    DarkOak,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Block {
    Air,
    Grass,
    Dirt,
    Stone,
    Sand,
    Snow,
    Wood(TreeSpecies),
    Leaves(TreeSpecies),
    Glowstone,
}

impl Block {
    #[inline]
    pub fn is_solid(&self) -> bool {
        !matches!(self, Block::Air)
    }

    #[inline]
    pub fn emission(&self) -> u8 {
        match self {
            Block::Glowstone => 255,
            _ => 0,
        }
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
        // Base terrain sampling (shared with trees)
        let mut noise = FastNoiseLite::with_seed(self.seed);
        noise.set_noise_type(Some(NoiseType::OpenSimplex2));
        noise.set_frequency(Some(0.02));
        let height_for = |wx: i32, wz: i32| {
            let h = noise.get_noise_2d(wx as f32, wz as f32);
            let min_h = (self.chunk_size_y as f32 * 0.15) as i32;
            let max_h = (self.chunk_size_y as f32 * 0.7) as i32;
            let hh = ((h + 1.0) * 0.5 * (max_h - min_h) as f32) as i32 + min_h;
            hh.clamp(1, self.chunk_size_y as i32 - 1)
        };
        let height = height_for(x, z);

        // Base terrain block
        let mut base_block = if y >= height {
            Block::Air
        } else if y == height - 1 {
            if height as f32 >= self.chunk_size_y as f32 * 0.62 { Block::Snow }
            else if height as f32 <= self.chunk_size_y as f32 * 0.2 { Block::Sand }
            else { Block::Grass }
        } else if y + 3 >= height { Block::Dirt } else { Block::Stone };

        // Simple static glowstone spawner (underground near air), low probability
        // Avoid recursion: approximate "near air" using only the heightmap.
        if matches!(base_block, Block::Stone) && y > 3 && y < height - 2 {
            // A neighboring cell is air if its ny >= height_for(nx, nz)
            let dirs = [(1,0,0),(-1,0,0),(0,1,0),(0,-1,0),(0,0,1),(0,0,-1)];
            let mut near_air = false;
            for (dx, dy, dz) in dirs {
                let nx = x + dx; let ny = y + dy; let nz = z + dz;
                if ny >= self.chunk_size_y as i32 { near_air = true; break; }
                if ny < 0 { continue; }
                let nh = height_for(nx, nz);
                if ny >= nh { near_air = true; break; }
            }
            if near_air {
                let hash2 = |ix: i32, iz: i32, seed: u32| -> u32 {
                    let mut h = (ix as u32).wrapping_mul(0x85eb_ca6b)
                        ^ (iz as u32).wrapping_mul(0xc2b2_ae35)
                        ^ seed.wrapping_mul(0x27d4_eb2d);
                    h ^= h >> 16; h = h.wrapping_mul(0x7feb_352d); h ^= h >> 15; h = h.wrapping_mul(0x846c_a68b); h ^= h >> 16;
                    h
                };
                let h = hash2(x, z, (self.seed as u32) ^ 0xC0FFEEu32 ^ (y as u32 * 2654435761));
                let r = (h & 0x00ff_ffff) as f32 / 16_777_216.0;
                if r < 0.0015 { base_block = Block::Glowstone; }
            }
        }

        // Deterministic per-column tree spawn (adapted from old code)
        // Multiple species via temperature/moisture fields
        // Only spawn on grass surfaces
        const TREE_PROB: f32 = 0.02; // ~2% chance per column
        const TRUNK_MIN: i32 = 4;
        const TRUNK_MAX: i32 = 6;
        const LEAF_R: i32 = 2;

        // Simple 2D hash -> [0,1)
        let hash2 = |ix: i32, iz: i32, seed: u32| -> u32 {
            let mut h = (ix as u32).wrapping_mul(0x85eb_ca6b)
                ^ (iz as u32).wrapping_mul(0xc2b2_ae35)
                ^ seed.wrapping_mul(0x27d4_eb2d);
            h ^= h >> 16; h = h.wrapping_mul(0x7feb_352d); h ^= h >> 15; h = h.wrapping_mul(0x846c_a68b); h ^= h >> 16;
            h
        };
        let rand01 = |ix: i32, iz: i32, salt: u32| -> f32 {
            let h = hash2(ix, iz, ((self.seed as u32) ^ salt).wrapping_add(0x9E37_79B9));
            ((h & 0x00FF_FFFF) as f32) / 16_777_216.0
        };

        // Species picker based on temp/moist noise like old code
        let mut temp = FastNoiseLite::with_seed(self.seed ^ 100_001);
        temp.set_noise_type(Some(NoiseType::OpenSimplex2));
        temp.set_frequency(Some(0.007));
        let mut mois = FastNoiseLite::with_seed(self.seed ^ 100_007);
        mois.set_noise_type(Some(NoiseType::OpenSimplex2));
        mois.set_frequency(Some(0.005));

        let pick_species = |tx: i32, tz: i32| -> TreeSpecies {
            let mut t = temp.get_noise_2d(tx as f32, tz as f32) * 0.5 + 0.5;
            let mut m = mois.get_noise_2d(tx as f32, tz as f32) * 0.5 + 0.5;
            let h = hash2(tx, tz, (self.seed as u32) ^ 0x5F37_59u32);
            let j = (h & 0xffff) as f32 / 65535.0 * 0.08 - 0.04;
            t = (t + j).clamp(0.0, 1.0);
            m = (m - j).clamp(0.0, 1.0);
            if t < 0.35 { return TreeSpecies::Spruce; }
            if t > 0.7 && m > 0.6 { return TreeSpecies::Jungle; }
            if t > 0.6 && m < 0.45 { return TreeSpecies::Acacia; }
            if m > 0.65 { return TreeSpecies::DarkOak; }
            // tie-breaker between birch and oak
            if ((h >> 20) & 1) == 1 { TreeSpecies::Birch } else { TreeSpecies::Oak }
        };

        // Helper: does a trunk start at (tx,tz)? If yes, return (surf_y, trunk_h, species)
        let trunk_at = |tx: i32, tz: i32| -> Option<(i32, i32, TreeSpecies)> {
            // surf = topmost solid Y index (height_for returns one above surface)
            let surf = height_for(tx, tz) - 1;
            // approximate surface block classification
            let surf_block = if surf as f32 >= self.chunk_size_y as f32 * 0.62 { Block::Snow }
                             else if surf as f32 <= self.chunk_size_y as f32 * 0.2 { Block::Sand }
                             else { Block::Grass };
            if !matches!(surf_block, Block::Grass) { return None; }
            let r = rand01(tx, tz, 0xA53F9);
            if r >= TREE_PROB { return None; }
            let span = (TRUNK_MAX - TRUNK_MIN).max(0) as u32;
            let hsel = hash2(tx, tz, 0x51F0_A7) % (span + 1);
            let th = TRUNK_MIN + hsel as i32;
            if surf <= 2 || surf >= (self.chunk_size_y as i32 - 6) { return None; }
            let sp = pick_species(tx, tz);
            Some((surf, th, sp))
        };

        // Trunk at current column?
        if let Some((surf, th, sp)) = trunk_at(x, z) {
            if y >= surf + 1 && y <= surf + th { return Block::Wood(sp); }
        }

        // Canopy: check nearby trunks and stamp leaves only where base is air
        if matches!(base_block, Block::Air) {
            for tx in (x - LEAF_R)..=(x + LEAF_R) {
                for tz in (z - LEAF_R)..=(z + LEAF_R) {
                    if let Some((surf, th, sp)) = trunk_at(tx, tz) {
                        let top_y = surf + th;
                        // y layer relative to top
                        let dy = y - top_y;
                        if dy < -2 || dy > 2 { continue; }
                        let rad = if dy <= -2 || dy >= 2 { LEAF_R - 1 } else { LEAF_R };
                        let dx = x - tx; let dz = z - tz;
                        if dx == 0 && dz == 0 && dy >= 0 { continue; }
                        let man = dx.abs() + dz.abs();
                        let extra = if dy >= 1 { 0 } else { 1 };
                        if man <= rad + extra { return Block::Leaves(sp); }
                    }
                }
            }
        }

        base_block
    }
}

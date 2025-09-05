use crate::voxel::{World, Block};

pub struct LightGrid {
    sx: usize,
    sy: usize,
    sz: usize,
    // Simple baseline skylight only for Phase 1 (0..255)
    skylight: Vec<u8>,
}

impl LightGrid {
    #[inline]
    fn idx(&self, x: usize, y: usize, z: usize) -> usize { (y * self.sz + z) * self.sx + x }

    pub fn new(sx: usize, sy: usize, sz: usize) -> Self {
        Self { sx, sy, sz, skylight: vec![0; sx*sy*sz] }
    }

    pub fn compute_baseline(world: &World, cx: i32, cz: i32) -> Self {
        let sx = world.chunk_size_x; let sy = world.chunk_size_y; let sz = world.chunk_size_z;
        let base_x = cx * sx as i32; let base_z = cz * sz as i32;
        let mut lg = Self::new(sx, sy, sz);
        // Skylight: for each column, all air cells above the highest solid get full light (255), else 0
        for z in 0..sz {
            for x in 0..sx {
                // find first solid from top
                let mut open_above = true;
                for y in (0..sy).rev() {
                    let b = world.block_at(base_x + x as i32, y as i32, base_z + z as i32);
                    if open_above {
                        let idx = lg.idx(x,y,z);
                        if matches!(b, Block::Air) { lg.skylight[idx] = 255u8; }
                        else { open_above = false; lg.skylight[idx] = 0u8; }
                    } else {
                        let idx = lg.idx(x,y,z);
                        lg.skylight[idx] = 0u8;
                    }
                }
            }
        }
        lg
    }

    // Sample light for the face adjacent to (x,y,z) in local chunk coords
    // face: 0=+Y,1=-Y,2=+X,3=-X,4=+Z,5=-Z
    pub fn sample_face_local(&self, x: usize, y: usize, z: usize, face: usize) -> u8 {
        let (dx,dy,dz) = match face { 0 => (0,1,0), 1 => (0isize,-1,0), 2 => (1,0,0), 3 => (-1,0,0), 4 => (0,0,1), 5 => (0,0,-1), _ => (0,0,0) };
        let nx = x as isize + dx; let ny = y as isize + dy; let nz = z as isize + dz;
        if nx < 0 || ny < 0 || nz < 0 || nx >= self.sx as isize || ny >= self.sy as isize || nz >= self.sz as isize {
            // Outside this chunk: approximate. For top, assume sky; otherwise 0.
            return if face == 0 { 255 } else { 0 };
        }
        self.skylight[self.idx(nx as usize, ny as usize, nz as usize)]
    }
}

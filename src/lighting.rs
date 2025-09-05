use crate::voxel::{World, Block};
use std::collections::HashMap;
use std::sync::Mutex;

pub struct LightGrid {
    sx: usize,
    sy: usize,
    sz: usize,
    // Simple baseline skylight only for Phase 1 (0..255)
    skylight: Vec<u8>,
    // Phase 2: in-chunk block light (0..255)
    block_light: Vec<u8>,
}

impl LightGrid {
    #[inline]
    fn idx(&self, x: usize, y: usize, z: usize) -> usize { (y * self.sz + z) * self.sx + x }

    pub fn new(sx: usize, sy: usize, sz: usize) -> Self {
        Self { sx, sy, sz, skylight: vec![0; sx*sy*sz], block_light: vec![0; sx*sy*sz] }
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
        // Block emissive: seed emitters and BFS within chunk (no cross-chunk yet)
        use std::collections::VecDeque;
        let mut q = VecDeque::new();
        for z in 0..sz { for y in 0..sy { for x in 0..sx {
            let wx = base_x + x as i32; let wy = y as i32; let wz = base_z + z as i32;
            let b = world.block_at(wx, wy, wz);
            let em = b.emission();
            if em > 0 { let idx = lg.idx(x,y,z); lg.block_light[idx] = em; q.push_back((x,y,z,em)); }
        }}}
        // BFS attenuation per step
        let atten: i32 = 32;
        while let Some((x,y,z,v)) = q.pop_front() {
            let vcur = v as i32;
            if vcur <= atten { continue; }
            let vnext = (vcur - atten) as u8;
            let neigh = [ (1,0,0),(-1,0,0),(0,1,0),(0,-1,0),(0,0,1),(0,0,-1) ];
            for (dx,dy,dz) in neigh {
                let nx = x as isize + dx; let ny = y as isize + dy; let nz = z as isize + dz;
                if nx < 0 || ny < 0 || nz < 0 || nx >= sx as isize || ny >= sy as isize || nz >= sz as isize { continue; }
                let nxi = nx as usize; let nyi = ny as usize; let nzi = nz as usize;
                // Only propagate into non-solid (air/leaves)
                let nb = world.block_at(base_x + nxi as i32, nyi as i32, base_z + nzi as i32);
                match nb {
                    Block::Air | Block::Leaves(_) => {
                        let idn = lg.idx(nxi, nyi, nzi);
                        if lg.block_light[idn] < vnext { lg.block_light[idn] = vnext; q.push_back((nxi, nyi, nzi, vnext)); }
                    }
                    _ => {}
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
            let sky = if face == 0 { 255 } else { 0 };
            let blk = 0;
            return sky.max(blk);
        }
        let i = self.idx(nx as usize, ny as usize, nz as usize);
        let sky = self.skylight[i];
        let blk = self.block_light[i];
        sky.max(blk)
    }
}

#[derive(Clone)]
pub struct LightBorders {
    pub sx: usize,
    pub sy: usize,
    pub sz: usize,
    // faces: xn (x-), xp (x+): dims sy*sz
    pub xn: Vec<u8>,
    pub xp: Vec<u8>,
    // zn, zp: dims sy*sx
    pub zn: Vec<u8>,
    pub zp: Vec<u8>,
    // yn, yp: dims sx*sz
    pub yn: Vec<u8>,
    pub yp: Vec<u8>,
}

impl LightBorders {
    pub fn new(sx: usize, sy: usize, sz: usize) -> Self {
        Self {
            sx, sy, sz,
            xn: vec![0; sy*sz], xp: vec![0; sy*sz],
            zn: vec![0; sy*sx], zp: vec![0; sy*sx],
            yn: vec![0; sx*sz], yp: vec![0; sx*sz],
        }
    }

    pub fn from_grid(grid: &LightGrid) -> Self {
        let (sx,sy,sz) = (grid.sx, grid.sy, grid.sz);
        let mut b = Self::new(sx,sy,sz);
        // Block light border from grid.block_light
        let idx3 = |x: usize,y: usize,z: usize| -> usize { (y*sz + z)*sx + x };
        // X- face at x=0
        for z in 0..sz { for y in 0..sy { b.xn[y*sz+z] = grid.block_light[idx3(0,y,z)]; }}
        // X+ face at x=sx-1
        for z in 0..sz { for y in 0..sy { b.xp[y*sz+z] = grid.block_light[idx3(sx-1,y,z)]; }}
        // Z- face at z=0
        for x in 0..sx { for y in 0..sy { b.zn[y*sx+x] = grid.block_light[idx3(x,y,0)]; }}
        // Z+ face at z=sz-1
        for x in 0..sx { for y in 0..sy { b.zp[y*sx+x] = grid.block_light[idx3(x,y,sz-1)]; }}
        // Y- face at y=0
        for z in 0..sz { for x in 0..sx { b.yn[z*sx+x] = grid.block_light[idx3(x,0,z)]; }}
        // Y+ face at y=sy-1
        for z in 0..sz { for x in 0..sx { b.yp[z*sx+x] = grid.block_light[idx3(x,sy-1,z)]; }}
        b
    }
}

pub struct LightingStore {
    sx: usize,
    sy: usize,
    sz: usize,
    borders: Mutex<HashMap<(i32,i32), LightBorders>>, // keyed by (cx,cz)
    emitters: Mutex<HashMap<(i32,i32), Vec<(usize,usize,usize,u8)>>>,
}

impl LightingStore {
    pub fn new(sx: usize, sy: usize, sz: usize) -> Self {
        Self { sx, sy, sz, borders: Mutex::new(HashMap::new()), emitters: Mutex::new(HashMap::new()) }
    }

    pub fn get_neighbor_borders(&self, cx: i32, cz: i32) -> NeighborBorders {
        let map = self.borders.lock().unwrap();
        let mut nb = NeighborBorders::empty(self.sx, self.sy, self.sz);
        if let Some(b) = map.get(&(cx-1, cz)) { nb.xn = Some(b.xp.clone()); }
        if let Some(b) = map.get(&(cx+1, cz)) { nb.xp = Some(b.xn.clone()); }
        if let Some(b) = map.get(&(cx, cz-1)) { nb.zn = Some(b.zp.clone()); }
        if let Some(b) = map.get(&(cx, cz+1)) { nb.zp = Some(b.zn.clone()); }
        // Vertical neighbors (not chunked vertically yet), leave None
        nb
    }

    pub fn update_borders(&self, cx: i32, cz: i32, lb: LightBorders) -> bool {
        let mut map = self.borders.lock().unwrap();
        match map.get_mut(&(cx,cz)) {
            Some(existing) => {
                let changed = !equal_planes(existing, &lb);
                if changed { *existing = lb; }
                changed
            }
            None => { map.insert((cx,cz), lb); true }
        }
    }

    pub fn add_emitter_world(&self, wx: i32, wy: i32, wz: i32, level: u8) {
        if wy < 0 || wy >= self.sy as i32 { return; }
        let sx = self.sx as i32; let sz = self.sz as i32;
        let cx = wx.div_euclid(sx); let cz = wz.div_euclid(sz);
        let lx = wx.rem_euclid(sx) as usize; let lz = wz.rem_euclid(sz) as usize; let ly = wy as usize;
        let mut map = self.emitters.lock().unwrap();
        let v = map.entry((cx,cz)).or_insert_with(Vec::new);
        if !v.iter().any(|&(x,y,z,_): &(usize,usize,usize,u8)| x==lx && y==ly && z==lz) {
            v.push((lx,ly,lz,level));
        }
    }

    pub fn remove_emitter_world(&self, wx: i32, wy: i32, wz: i32) {
        if wy < 0 || wy >= self.sy as i32 { return; }
        let sx = self.sx as i32; let sz = self.sz as i32;
        let cx = wx.div_euclid(sx); let cz = wz.div_euclid(sz);
        let lx = wx.rem_euclid(sx) as usize; let lz = wz.rem_euclid(sz) as usize; let ly = wy as usize;
        let mut map = self.emitters.lock().unwrap();
        if let Some(v) = map.get_mut(&(cx,cz)) {
            v.retain(|&(x,y,z,_): &(usize,usize,usize,u8)| !(x==lx && y==ly && z==lz));
            if v.is_empty() { map.remove(&(cx,cz)); }
        }
    }

    pub fn emitters_for_chunk(&self, cx: i32, cz: i32) -> Vec<(usize,usize,usize,u8)> {
        let map = self.emitters.lock().unwrap();
        map.get(&(cx,cz)).cloned().unwrap_or_default()
    }
}

fn equal_planes(a: &LightBorders, b: &LightBorders) -> bool {
    a.xn == b.xn && a.xp == b.xp && a.zn == b.zn && a.zp == b.zp && a.yn == b.yn && a.yp == b.yp
}

pub struct NeighborBorders {
    pub sx: usize,
    pub sy: usize,
    pub sz: usize,
    pub xn: Option<Vec<u8>>, // neighbor's +X into our -X
    pub xp: Option<Vec<u8>>, // neighbor's -X into our +X
    pub zn: Option<Vec<u8>>, // neighbor's +Z into our -Z
    pub zp: Option<Vec<u8>>, // neighbor's -Z into our +Z
}

impl NeighborBorders {
    pub fn empty(sx: usize, sy: usize, sz: usize) -> Self {
        Self { sx, sy, sz, xn: None, xp: None, zn: None, zp: None }
    }
}

impl LightGrid {
    pub fn compute_with_borders(world: &World, cx: i32, cz: i32, store: &LightingStore) -> Self {
        let sx = world.chunk_size_x; let sy = world.chunk_size_y; let sz = world.chunk_size_z;
        let base_x = cx * sx as i32; let base_z = cz * sz as i32;
        let mut lg = Self::new(sx, sy, sz);
        // Skylight (same as baseline)
        for z in 0..sz { for x in 0..sx {
            let mut open_above = true;
            for y in (0..sy).rev() {
                let b = world.block_at(base_x + x as i32, y as i32, base_z + z as i32);
                let idx = lg.idx(x,y,z);
                if open_above {
                    if matches!(b, Block::Air) { lg.skylight[idx] = 255; } else { open_above = false; lg.skylight[idx] = 0; }
                } else { lg.skylight[idx] = 0; }
            }
        }}
        // Seed emitters
        use std::collections::VecDeque;
        let mut q = VecDeque::new();
        for z in 0..sz { for y in 0..sy { for x in 0..sx {
            let b = world.block_at(base_x + x as i32, y as i32, base_z + z as i32);
            let em = b.emission();
            if em > 0 { let idx = lg.idx(x,y,z); lg.block_light[idx] = em; q.push_back((x,y,z,em)); }
        }}}
        // Dynamic emitters from store
        for (x,y,z,level) in store.emitters_for_chunk(cx, cz) {
            let idx = lg.idx(x,y,z);
            if lg.block_light[idx] < level { lg.block_light[idx] = level; q.push_back((x,y,z,level)); }
        }
        // Seed from neighbor borders (attenuate by 32 across boundary)
        let nb = store.get_neighbor_borders(cx, cz);
        let atten: i32 = 32;
        if let Some(ref plane) = nb.xn { // our x=0 face
            for z in 0..sz { for y in 0..sy {
                let v = plane[y*sz+z] as i32 - atten;
                if v > 0 { let v8 = v as u8; let idx = lg.idx(0,y,z); if lg.block_light[idx] < v8 { lg.block_light[idx] = v8; q.push_back((0,y,z,v8)); } }
            }}
        }
        if let Some(ref plane) = nb.xp { // our x=sx-1 face
            for z in 0..sz { for y in 0..sy {
                let v = plane[y*sz+z] as i32 - atten; if v > 0 { let v8 = v as u8; let xx = sx-1; let idx = lg.idx(xx,y,z); if lg.block_light[idx] < v8 { lg.block_light[idx] = v8; q.push_back((xx,y,z,v8)); } }
            }}
        }
        if let Some(ref plane) = nb.zn { // our z=0 face
            for x in 0..sx { for y in 0..sy {
                let v = plane[y*sx+x] as i32 - atten; if v > 0 { let v8 = v as u8; let idx = lg.idx(x,y,0); if lg.block_light[idx] < v8 { lg.block_light[idx] = v8; q.push_back((x,y,0,v8)); } }
            }}
        }
        if let Some(ref plane) = nb.zp { // our z=sz-1 face
            for x in 0..sx { for y in 0..sy {
                let v = plane[y*sx+x] as i32 - atten; if v > 0 { let v8 = v as u8; let zz = sz-1; let idx = lg.idx(x,y,zz); if lg.block_light[idx] < v8 { lg.block_light[idx] = v8; q.push_back((x,y,zz,v8)); } }
            }}
        }
        // BFS inside chunk
        while let Some((x,y,z,v)) = q.pop_front() {
            let vcur = v as i32; if vcur <= atten { continue; }
            let vnext = (vcur - atten) as u8;
            let neigh = [ (1,0,0),(-1,0,0),(0,1,0),(0,-1,0),(0,0,1),(0,0,-1) ];
            for (dx,dy,dz) in neigh {
                let nx = x as isize + dx; let ny = y as isize + dy; let nz = z as isize + dz;
                if nx < 0 || ny < 0 || nz < 0 || nx >= sx as isize || ny >= sy as isize || nz >= sz as isize { continue; }
                let nxi = nx as usize; let nyi = ny as usize; let nzi = nz as usize;
                let nbk = world.block_at(base_x + nxi as i32, nyi as i32, base_z + nzi as i32);
                match nbk { Block::Air | Block::Leaves(_) => {
                        let idn = lg.idx(nxi, nyi, nzi);
                        if lg.block_light[idn] < vnext { lg.block_light[idn] = vnext; q.push_back((nxi, nyi, nzi, vnext)); }
                    }
                    _ => {}
                }
            }
        }
        lg
    }
}

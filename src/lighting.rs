use crate::chunkbuf::ChunkBuf;
use crate::voxel::Block;
use crate::blocks::BlockRegistry;
use crate::voxel;
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
    // Beacon light channel (0..255) - separate to maintain proper attenuation
    beacon_light: Vec<u8>,
    // Direction of beacon propagation per voxel (0=source, 1=+X, 2=-X, 3=+Z, 4=-Z, 5=non-cardinal)
    beacon_dir: Vec<u8>,
    // Neighbor border planes (optional). Used to sample lighting across chunk seams.
    // Only horizontal neighbors are populated (x-/x+/z-/z+). Dimensions noted per plane.
    nb_xn_blk: Option<Vec<u8>>, // dims sy*sz
    nb_xp_blk: Option<Vec<u8>>, // dims sy*sz
    nb_zn_blk: Option<Vec<u8>>, // dims sy*sx
    nb_zp_blk: Option<Vec<u8>>, // dims sy*sx
    nb_xn_sky: Option<Vec<u8>>, // dims sy*sz
    nb_xp_sky: Option<Vec<u8>>, // dims sy*sz
    nb_zn_sky: Option<Vec<u8>>, // dims sy*sx
    nb_zp_sky: Option<Vec<u8>>, // dims sy*sx
    nb_xn_bcn: Option<Vec<u8>>, // beacon light dims sy*sz
    nb_xp_bcn: Option<Vec<u8>>, // beacon light dims sy*sz
    nb_zn_bcn: Option<Vec<u8>>, // beacon light dims sy*sx
    nb_zp_bcn: Option<Vec<u8>>, // beacon light dims sy*sx
    // Direction planes for beacon light across borders (same indexing as above)
    nb_xn_bcn_dir: Option<Vec<u8>>, // expected direction entering from -X face (codes above)
    nb_xp_bcn_dir: Option<Vec<u8>>, // entering from +X face
    nb_zn_bcn_dir: Option<Vec<u8>>, // entering from -Z face
    nb_zp_bcn_dir: Option<Vec<u8>>, // entering from +Z face
}

impl LightGrid {
    #[inline]
    fn idx(&self, x: usize, y: usize, z: usize) -> usize {
        (y * self.sz + z) * self.sx + x
    }

    pub fn new(sx: usize, sy: usize, sz: usize) -> Self {
        Self {
            sx,
            sy,
            sz,
            skylight: vec![0; sx * sy * sz],
            block_light: vec![0; sx * sy * sz],
            beacon_light: vec![0; sx * sy * sz],
            beacon_dir: vec![0; sx * sy * sz],
            nb_xn_blk: None,
            nb_xp_blk: None,
            nb_zn_blk: None,
            nb_zp_blk: None,
            nb_xn_sky: None,
            nb_xp_sky: None,
            nb_zn_sky: None,
            nb_zp_sky: None,
            nb_xn_bcn: None,
            nb_xp_bcn: None,
            nb_zn_bcn: None,
            nb_zp_bcn: None,
            nb_xn_bcn_dir: None,
            nb_xp_bcn_dir: None,
            nb_zn_bcn_dir: None,
            nb_zp_bcn_dir: None,
        }
    }

    pub fn compute_with_borders_buf(buf: &ChunkBuf, store: &LightingStore, reg: &BlockRegistry) -> Self {
        let sx = buf.sx;
        let sy = buf.sy;
        let sz = buf.sz;
        let mut lg = Self::new(sx, sy, sz);
        // Skylight seeds (Air at top to 255); leaves block skylight
        use std::collections::VecDeque;
        let mut q_sky = VecDeque::new();
        for z in 0..sz {
            for x in 0..sx {
                let mut open_above = true;
                for y in (0..sy).rev() {
                    let b = buf.get_local(x, y, z);
                    let idx = lg.idx(x, y, z);
                    if open_above {
                        if skylight_transparent(b, reg) {
                            lg.skylight[idx] = 255;
                            q_sky.push_back((x, y, z, 255u8));
                        } else {
                            open_above = false;
                            lg.skylight[idx] = 0;
                        }
                    } else {
                        lg.skylight[idx] = 0;
                    }
                }
            }
        }
        // Emitters from blocks in this chunk - track beacon sources separately
        let mut q = VecDeque::new();
        // Beacon queue tracks: (x, y, z, level, direction)
        // Direction: 0=source, 1=+X, 2=-X, 3=+Z, 4=-Z, 5=non-cardinal
        let mut q_beacon: VecDeque<(usize, usize, usize, u8, u8)> = VecDeque::new();
        for z in 0..sz {
            for y in 0..sy {
                for x in 0..sx {
                    let b = buf.get_local(x, y, z);
                    let em = b.emission();
                    if em > 0 {
                        let idx = lg.idx(x, y, z);
                        if matches!(b, Block::Beacon) {
                            lg.beacon_light[idx] = em;
                            lg.beacon_dir[idx] = 0; // source
                            // Initial beacon emission - direction 0 means it's the source
                            q_beacon.push_back((x, y, z, em, 0));
                        } else {
                            lg.block_light[idx] = em;
                            q.push_back((x, y, z, em));
                        }
                    }
                }
            }
        }
        // Dynamic emitters from store (chunk coordinates are same as buf.cx,buf.cz)
        for (x, y, z, level, is_beacon) in store.emitters_for_chunk(buf.cx, buf.cz) {
            if x < sx && y < sy && z < sz {
                let idx = lg.idx(x, y, z);
                if is_beacon {
                    if lg.beacon_light[idx] < level {
                        lg.beacon_light[idx] = level;
                        lg.beacon_dir[idx] = 0; // source
                        q_beacon.push_back((x, y, z, level, 0)); // 0 = source
                    }
                } else {
                    if lg.block_light[idx] < level {
                        lg.block_light[idx] = level;
                        q.push_back((x, y, z, level));
                    }
                }
            }
        }
        // Seed from neighbor borders if any
        let nb = store.get_neighbor_borders(buf.cx, buf.cz);
        lg.nb_xn_blk = nb.xn.clone();
        lg.nb_xp_blk = nb.xp.clone();
        lg.nb_zn_blk = nb.zn.clone();
        lg.nb_zp_blk = nb.zp.clone();
        lg.nb_xn_sky = nb.sk_xn.clone();
        lg.nb_xp_sky = nb.sk_xp.clone();
        lg.nb_zn_sky = nb.sk_zn.clone();
        lg.nb_zp_sky = nb.sk_zp.clone();
        lg.nb_xn_bcn = nb.bcn_xn.clone();
        lg.nb_xp_bcn = nb.bcn_xp.clone();
        lg.nb_zn_bcn = nb.bcn_zn.clone();
        lg.nb_zp_bcn = nb.bcn_zp.clone();
        lg.nb_xn_bcn_dir = nb.bcn_dir_xn.clone();
        lg.nb_xp_bcn_dir = nb.bcn_dir_xp.clone();
        lg.nb_zn_bcn_dir = nb.bcn_dir_zn.clone();
        lg.nb_zp_bcn_dir = nb.bcn_dir_zp.clone();
        // Process neighbor light with appropriate attenuation
        let atten: i32 = 32;
        // Regular block light from neighbors
        if let Some(ref plane) = nb.xn {
            for z in 0..sz {
                for y in 0..sy {
                    let v = plane[y * sz + z] as i32 - atten;
                    if v > 0 {
                        let v8 = v as u8;
                        let idx = lg.idx(0, y, z);
                        if lg.block_light[idx] < v8 {
                            lg.block_light[idx] = v8;
                            q.push_back((0, y, z, v8));
                        }
                    }
                }
            }
        }
        if let Some(ref plane) = nb.xp {
            for z in 0..sz {
                for y in 0..sy {
                    let v = plane[y * sz + z] as i32 - atten;
                    if v > 0 {
                        let v8 = v as u8;
                        let xx = sx - 1;
                        let idx = lg.idx(xx, y, z);
                        if lg.block_light[idx] < v8 {
                            lg.block_light[idx] = v8;
                            q.push_back((xx, y, z, v8));
                        }
                    }
                }
            }
        }
        if let Some(ref plane) = nb.zn {
            for x in 0..sx {
                for y in 0..sy {
                    let v = plane[y * sx + x] as i32 - atten;
                    if v > 0 {
                        let v8 = v as u8;
                        let idx = lg.idx(x, y, 0);
                        if lg.block_light[idx] < v8 {
                            lg.block_light[idx] = v8;
                            q.push_back((x, y, 0, v8));
                        }
                    }
                }
            }
        }
        if let Some(ref plane) = nb.zp {
            for x in 0..sx {
                for y in 0..sy {
                    let v = plane[y * sx + x] as i32 - atten;
                    if v > 0 {
                        let v8 = v as u8;
                        let zz = sz - 1;
                        let idx = lg.idx(x, y, zz);
                        if lg.block_light[idx] < v8 {
                            lg.block_light[idx] = v8;
                            q.push_back((x, y, zz, v8));
                        }
                    }
                }
            }
        }
        // Beacon light from neighbors: Use explicit direction planes (no threshold heuristic)
        if let Some(ref plane) = nb.bcn_xn {
            for z in 0..sz {
                for y in 0..sy {
                    let orig_v = plane[y * sz + z];
                    let dir = lg
                        .nb_xn_bcn_dir
                        .as_ref()
                        .and_then(|p| p.get(y * sz + z).cloned())
                        .unwrap_or(5);
                    let atten = match dir {
                        1 | 2 | 3 | 4 => 1,
                        _ => 32,
                    };
                    let v = orig_v as i32 - atten;
                    if v > 0 {
                        let v8 = v as u8;
                        let idx = lg.idx(0, y, z);
                        if lg.beacon_light[idx] < v8 {
                            lg.beacon_light[idx] = v8;
                            lg.beacon_dir[idx] = dir;
                            q_beacon.push_back((0, y, z, v8, dir));
                        }
                    }
                }
            }
        }
        if let Some(ref plane) = nb.bcn_xp {
            for z in 0..sz {
                for y in 0..sy {
                    let orig_v = plane[y * sz + z];
                    let dir = lg
                        .nb_xp_bcn_dir
                        .as_ref()
                        .and_then(|p| p.get(y * sz + z).cloned())
                        .unwrap_or(5);
                    let atten = match dir {
                        1 | 2 | 3 | 4 => 1,
                        _ => 32,
                    };
                    let v = orig_v as i32 - atten;
                    if v > 0 {
                        let v8 = v as u8;
                        let xx = sx - 1;
                        let idx = lg.idx(xx, y, z);
                        if lg.beacon_light[idx] < v8 {
                            lg.beacon_light[idx] = v8;
                            lg.beacon_dir[idx] = dir;
                            q_beacon.push_back((xx, y, z, v8, dir));
                        }
                    }
                }
            }
        }
        if let Some(ref plane) = nb.bcn_zn {
            for x in 0..sx {
                for y in 0..sy {
                    let orig_v = plane[y * sx + x];
                    let dir = lg
                        .nb_zn_bcn_dir
                        .as_ref()
                        .and_then(|p| p.get(y * sx + x).cloned())
                        .unwrap_or(5);
                    let atten = match dir {
                        1 | 2 | 3 | 4 => 1,
                        _ => 32,
                    };
                    let v = orig_v as i32 - atten;
                    if v > 0 {
                        let v8 = v as u8;
                        let idx = lg.idx(x, y, 0);
                        if lg.beacon_light[idx] < v8 {
                            lg.beacon_light[idx] = v8;
                            lg.beacon_dir[idx] = dir;
                            q_beacon.push_back((x, y, 0, v8, dir));
                        }
                    }
                }
            }
        }
        if let Some(ref plane) = nb.bcn_zp {
            for x in 0..sx {
                for y in 0..sy {
                    let orig_v = plane[y * sx + x];
                    let dir = lg
                        .nb_zp_bcn_dir
                        .as_ref()
                        .and_then(|p| p.get(y * sx + x).cloned())
                        .unwrap_or(5);
                    let atten = match dir {
                        1 | 2 | 3 | 4 => 1,
                        _ => 32,
                    };
                    let v = orig_v as i32 - atten;
                    if v > 0 {
                        let v8 = v as u8;
                        let zz = sz - 1;
                        let idx = lg.idx(x, y, zz);
                        if lg.beacon_light[idx] < v8 {
                            lg.beacon_light[idx] = v8;
                            lg.beacon_dir[idx] = dir;
                            q_beacon.push_back((x, y, zz, v8, dir));
                        }
                    }
                }
            }
        }
        if let Some(ref plane) = nb.sk_xn {
            for z in 0..sz {
                for y in 0..sy {
                    let v = plane[y * sz + z] as i32 - atten;
                    if v > 0 {
                        let v8 = v as u8;
                        let idx = lg.idx(0, y, z);
                        if lg.skylight[idx] < v8 {
                            lg.skylight[idx] = v8;
                            q_sky.push_back((0, y, z, v8));
                        }
                    }
                }
            }
        }
        if let Some(ref plane) = nb.sk_xp {
            for z in 0..sz {
                for y in 0..sy {
                    let v = plane[y * sz + z] as i32 - atten;
                    if v > 0 {
                        let v8 = v as u8;
                        let xx = sx - 1;
                        let idx = lg.idx(xx, y, z);
                        if lg.skylight[idx] < v8 {
                            lg.skylight[idx] = v8;
                            q_sky.push_back((xx, y, z, v8));
                        }
                    }
                }
            }
        }
        if let Some(ref plane) = nb.sk_zn {
            for x in 0..sx {
                for y in 0..sy {
                    let v = plane[y * sx + x] as i32 - atten;
                    if v > 0 {
                        let v8 = v as u8;
                        let idx = lg.idx(x, y, 0);
                        if lg.skylight[idx] < v8 {
                            lg.skylight[idx] = v8;
                            q_sky.push_back((x, y, 0, v8));
                        }
                    }
                }
            }
        }
        if let Some(ref plane) = nb.sk_zp {
            for x in 0..sx {
                for y in 0..sy {
                    let v = plane[y * sx + x] as i32 - atten;
                    if v > 0 {
                        let v8 = v as u8;
                        let zz = sz - 1;
                        let idx = lg.idx(x, y, zz);
                        if lg.skylight[idx] < v8 {
                            lg.skylight[idx] = v8;
                            q_sky.push_back((x, y, zz, v8));
                        }
                    }
                }
            }
        }
        // Skylight BFS within chunk (Air only)
        while let Some((x, y, z, v)) = q_sky.pop_front() {
            let vcur = v as i32;
            if vcur <= atten {
                continue;
            }
            let vnext = (vcur - atten) as u8;
            let neigh = [
                (1, 0, 0),
                (-1, 0, 0),
                (0, 1, 0),
                (0, -1, 0),
                (0, 0, 1),
                (0, 0, -1),
            ];
            for (dx, dy, dz) in neigh {
                let nx = x as isize + dx;
                let ny = y as isize + dy;
                let nz = z as isize + dz;
                if nx < 0
                    || ny < 0
                    || nz < 0
                    || nx >= sx as isize
                    || ny >= sy as isize
                    || nz >= sz as isize
                {
                    continue;
                }
                let nxi = nx as usize;
                let nyi = ny as usize;
                let nzi = nz as usize;
                if matches!(buf.get_local(nxi, nyi, nzi), Block::Air) {
                    let idn = lg.idx(nxi, nyi, nzi);
                    if lg.skylight[idn] < vnext {
                        lg.skylight[idn] = vnext;
                        q_sky.push_back((nxi, nyi, nzi, vnext));
                    }
                }
            }
        }
        // Block-light BFS within chunk (Air only; leaves block)
        // Normal lights with standard attenuation
        while let Some((x, y, z, v)) = q.pop_front() {
            let vcur = v as i32;
            if vcur <= atten {
                continue;
            }
            let vnext = (vcur - atten) as u8;
            let neigh = [
                (1, 0, 0),
                (-1, 0, 0),
                (0, 1, 0),
                (0, -1, 0),
                (0, 0, 1),
                (0, 0, -1),
            ];
            for (dx, dy, dz) in neigh {
                let nx = x as isize + dx;
                let ny = y as isize + dy;
                let nz = z as isize + dz;
                if nx < 0
                    || ny < 0
                    || nz < 0
                    || nx >= sx as isize
                    || ny >= sy as isize
                    || nz >= sz as isize
                {
                    continue;
                }
                let nxi = nx as usize;
                let nyi = ny as usize;
                let nzi = nz as usize;
                if matches!(buf.get_local(nxi, nyi, nzi), Block::Air) {
                    let idn = lg.idx(nxi, nyi, nzi);
                    if lg.block_light[idn] < vnext {
                        lg.block_light[idn] = vnext;
                        q.push_back((nxi, nyi, nzi, vnext));
                    }
                }
            }
        }
        // Beacon lights with directional attenuation
        // Cardinal directions (N/S/E/W): attenuation of 1
        // Diagonals and vertical: attenuation of 32
        while let Some((x, y, z, v, dir)) = q_beacon.pop_front() {
            let neigh = [
                (1, 0, 0),
                (-1, 0, 0),
                (0, 1, 0),
                (0, -1, 0),
                (0, 0, 1),
                (0, 0, -1),
            ];
            for (dx, dy, dz) in neigh.iter() {
                let nx = x as isize + dx;
                let ny = y as isize + dy;
                let nz = z as isize + dz;
                if nx < 0
                    || ny < 0
                    || nz < 0
                    || nx >= sx as isize
                    || ny >= sy as isize
                    || nz >= sz as isize
                {
                    continue;
                }

                // Determine direction and attenuation
                let (new_dir, atten) = match dir {
                    0 => {
                        // Source: can start cardinal in any horizontal direction
                        match (dx, dy, dz) {
                            (1, 0, 0) => (1, 1),  // Start +X cardinal
                            (-1, 0, 0) => (2, 1), // Start -X cardinal
                            (0, 0, 1) => (3, 1),  // Start +Z cardinal
                            (0, 0, -1) => (4, 1), // Start -Z cardinal
                            _ => (5, 32),         // Vertical is non-cardinal
                        }
                    }
                    1 => {
                        // Moving +X: can only continue +X for cardinal
                        if *dx == 1 && *dy == 0 && *dz == 0 {
                            (1, 1)
                        } else {
                            (5, 32)
                        }
                    }
                    2 => {
                        // Moving -X: can only continue -X for cardinal
                        if *dx == -1 && *dy == 0 && *dz == 0 {
                            (2, 1)
                        } else {
                            (5, 32)
                        }
                    }
                    3 => {
                        // Moving +Z: can only continue +Z for cardinal
                        if *dx == 0 && *dy == 0 && *dz == 1 {
                            (3, 1)
                        } else {
                            (5, 32)
                        }
                    }
                    4 => {
                        // Moving -Z: can only continue -Z for cardinal
                        if *dx == 0 && *dy == 0 && *dz == -1 {
                            (4, 1)
                        } else {
                            (5, 32)
                        }
                    }
                    _ => (5, 32), // Non-cardinal always uses high attenuation
                };

                let vcur = v as i32;
                if vcur <= atten {
                    continue;
                }
                let vnext = (vcur - atten) as u8;

                let nxi = nx as usize;
                let nyi = ny as usize;
                let nzi = nz as usize;
                if matches!(buf.get_local(nxi, nyi, nzi), Block::Air) {
                    let idn = lg.idx(nxi, nyi, nzi);
                    if lg.beacon_light[idn] < vnext {
                        lg.beacon_light[idn] = vnext;
                        lg.beacon_dir[idn] = new_dir;
                        q_beacon.push_back((nxi, nyi, nzi, vnext, new_dir));
                    }
                }
            }
        }
        lg
    }

    // Sample light for the face adjacent to (x,y,z) in local chunk coords
    // face: 0=+Y,1=-Y,2=+X,3=-X,4=+Z,5=-Z
    pub fn sample_face_local(&self, x: usize, y: usize, z: usize, face: usize) -> u8 {
        let (dx, dy, dz) = match face {
            0 => (0, 1, 0),
            1 => (0isize, -1, 0),
            2 => (1, 0, 0),
            3 => (-1, 0, 0),
            4 => (0, 0, 1),
            5 => (0, 0, -1),
            _ => (0, 0, 0),
        };
        let nx = x as isize + dx;
        let ny = y as isize + dy;
        let nz = z as isize + dz;
        if nx < 0
            || ny < 0
            || nz < 0
            || nx >= self.sx as isize
            || ny >= self.sy as isize
            || nz >= self.sz as isize
        {
            // Outside this chunk: try neighbor border planes if available.
            // Top/bottom faces have no vertical neighbors yet -> keep simple fallbacks.
            match face {
                0 => return 255, // assume sky above
                1 => return 0,   // assume dark below
                2 => {
                    // +X uses xp planes, index by (y,z) in dims sy*sz
                    let idxp = (y * self.sz + z) as usize;
                    let sky = self
                        .nb_xp_sky
                        .as_ref()
                        .and_then(|p| p.get(idxp).cloned())
                        .unwrap_or(0);
                    let blk = self
                        .nb_xp_blk
                        .as_ref()
                        .and_then(|p| p.get(idxp).cloned())
                        .unwrap_or(0);
                    let bcn = self
                        .nb_xp_bcn
                        .as_ref()
                        .and_then(|p| p.get(idxp).cloned())
                        .unwrap_or(0);
                    let max_neighbor = sky.max(blk).max(bcn);
                    if max_neighbor > 0 {
                        return max_neighbor;
                    }
                    // Fallback: sample our own border cell
                    let i = self.idx(self.sx - 1, y as usize, z as usize);
                    return self.skylight[i]
                        .max(self.block_light[i])
                        .max(self.beacon_light[i]);
                }
                3 => {
                    // -X uses xn planes
                    let idxp = (y * self.sz + z) as usize;
                    let sky = self
                        .nb_xn_sky
                        .as_ref()
                        .and_then(|p| p.get(idxp).cloned())
                        .unwrap_or(0);
                    let blk = self
                        .nb_xn_blk
                        .as_ref()
                        .and_then(|p| p.get(idxp).cloned())
                        .unwrap_or(0);
                    let bcn = self
                        .nb_xn_bcn
                        .as_ref()
                        .and_then(|p| p.get(idxp).cloned())
                        .unwrap_or(0);
                    let max_neighbor = sky.max(blk).max(bcn);
                    if max_neighbor > 0 {
                        return max_neighbor;
                    }
                    let i = self.idx(0, y as usize, z as usize);
                    return self.skylight[i]
                        .max(self.block_light[i])
                        .max(self.beacon_light[i]);
                }
                4 => {
                    // +Z uses zp planes, index by (y,x) in dims sy*sx
                    let idxp = (y * self.sx + x) as usize;
                    let sky = self
                        .nb_zp_sky
                        .as_ref()
                        .and_then(|p| p.get(idxp).cloned())
                        .unwrap_or(0);
                    let blk = self
                        .nb_zp_blk
                        .as_ref()
                        .and_then(|p| p.get(idxp).cloned())
                        .unwrap_or(0);
                    let bcn = self
                        .nb_zp_bcn
                        .as_ref()
                        .and_then(|p| p.get(idxp).cloned())
                        .unwrap_or(0);
                    let max_neighbor = sky.max(blk).max(bcn);
                    if max_neighbor > 0 {
                        return max_neighbor;
                    }
                    let i = self.idx(x as usize, y as usize, self.sz - 1);
                    return self.skylight[i]
                        .max(self.block_light[i])
                        .max(self.beacon_light[i]);
                }
                5 => {
                    // -Z uses zn planes
                    let idxp = (y * self.sx + x) as usize;
                    let sky = self
                        .nb_zn_sky
                        .as_ref()
                        .and_then(|p| p.get(idxp).cloned())
                        .unwrap_or(0);
                    let blk = self
                        .nb_zn_blk
                        .as_ref()
                        .and_then(|p| p.get(idxp).cloned())
                        .unwrap_or(0);
                    let bcn = self
                        .nb_zn_bcn
                        .as_ref()
                        .and_then(|p| p.get(idxp).cloned())
                        .unwrap_or(0);
                    let max_neighbor = sky.max(blk).max(bcn);
                    if max_neighbor > 0 {
                        return max_neighbor;
                    }
                    let i = self.idx(x as usize, y as usize, 0);
                    return self.skylight[i]
                        .max(self.block_light[i])
                        .max(self.beacon_light[i]);
                }
                _ => {}
            }
            // Fallback
            return 0;
        }
        let i = self.idx(nx as usize, ny as usize, nz as usize);
        let sky = self.skylight[i];
        let blk = self.block_light[i];
        let bcn = self.beacon_light[i];
        sky.max(blk).max(bcn)
    }
}

// Helper: determine if a legacy Block is skylight-transparent using the runtime registry.
// Returns true for Air and any block that does not block skylight per registry; false otherwise.
#[inline]
fn skylight_transparent(b: Block, reg: &BlockRegistry) -> bool {
    match b {
        Block::Air => true,
        _ => {
            if let Some(name) = map_legacy_block_to_registry_name(b) {
                if let Some(id) = reg.id_by_name(name) {
                    if let Some(ty) = reg.get(id) {
                        return !ty.blocks_skylight(0);
                    }
                }
            }
            false
        }
    }
}

#[inline]
fn map_legacy_block_to_registry_name(b: Block) -> Option<&'static str> {
    match b {
        Block::Air => Some("air"),
        Block::Stone => Some("stone"),
        Block::Dirt => Some("dirt"),
        Block::Grass => Some("grass"),
        Block::Sand => Some("sand"),
        Block::Snow => Some("snow"),
        Block::Glowstone => Some("glowstone"),
        Block::Beacon => Some("beacon"),
        Block::Wood(voxel::TreeSpecies::Oak) => Some("oak_log"),
        Block::Wood(voxel::TreeSpecies::Birch) => Some("birch_log"),
        Block::Wood(voxel::TreeSpecies::Spruce) => Some("spruce_log"),
        Block::Wood(voxel::TreeSpecies::Jungle) => Some("jungle_log"),
        Block::Wood(voxel::TreeSpecies::Acacia) => Some("acacia_log"),
        Block::Wood(voxel::TreeSpecies::DarkOak) => Some("dark_oak_log"),
        Block::Leaves(voxel::TreeSpecies::Oak) => Some("oak_leaves"),
        Block::Leaves(voxel::TreeSpecies::Birch) => Some("birch_leaves"),
        Block::Leaves(voxel::TreeSpecies::Spruce) => Some("spruce_leaves"),
        Block::Leaves(voxel::TreeSpecies::Jungle) => Some("jungle_leaves"),
        Block::Leaves(voxel::TreeSpecies::Acacia) => Some("acacia_leaves"),
        Block::Leaves(voxel::TreeSpecies::DarkOak) => Some("oak_leaves"),
        _ => None,
    }
}

#[derive(Clone)]
pub struct LightBorders {
    // faces: xn (x-), xp (x+): dims sy*sz
    pub xn: Vec<u8>, // block light
    pub xp: Vec<u8>, // block light
    // zn, zp: dims sy*sx
    pub zn: Vec<u8>, // block light
    pub zp: Vec<u8>, // block light
    // yn, yp: dims sx*sz
    pub yn: Vec<u8>, // block light
    pub yp: Vec<u8>, // block light
    // Skylight border planes, same dimensions as above
    pub sk_xn: Vec<u8>,
    pub sk_xp: Vec<u8>,
    pub sk_zn: Vec<u8>,
    pub sk_zp: Vec<u8>,
    pub sk_yn: Vec<u8>,
    pub sk_yp: Vec<u8>,
    // Beacon light border planes, same dimensions as above
    pub bcn_xn: Vec<u8>,
    pub bcn_xp: Vec<u8>,
    pub bcn_zn: Vec<u8>,
    pub bcn_zp: Vec<u8>,
    pub bcn_yn: Vec<u8>,
    pub bcn_yp: Vec<u8>,
    // Beacon direction planes (codes: 1/2/3/4=cardinal along face normal; 5=non-cardinal)
    pub bcn_dir_xn: Vec<u8>,
    pub bcn_dir_xp: Vec<u8>,
    pub bcn_dir_zn: Vec<u8>,
    pub bcn_dir_zp: Vec<u8>,
}

impl LightBorders {
    pub fn new(sx: usize, sy: usize, sz: usize) -> Self {
        Self {
            xn: vec![0; sy * sz],
            xp: vec![0; sy * sz],
            zn: vec![0; sy * sx],
            zp: vec![0; sy * sx],
            yn: vec![0; sx * sz],
            yp: vec![0; sx * sz],
            sk_xn: vec![0; sy * sz],
            sk_xp: vec![0; sy * sz],
            sk_zn: vec![0; sy * sx],
            sk_zp: vec![0; sy * sx],
            sk_yn: vec![0; sx * sz],
            sk_yp: vec![0; sx * sz],
            bcn_xn: vec![0; sy * sz],
            bcn_xp: vec![0; sy * sz],
            bcn_zn: vec![0; sy * sx],
            bcn_zp: vec![0; sy * sx],
            bcn_yn: vec![0; sx * sz],
            bcn_yp: vec![0; sx * sz],
            bcn_dir_xn: vec![5; sy * sz],
            bcn_dir_xp: vec![5; sy * sz],
            bcn_dir_zn: vec![5; sy * sx],
            bcn_dir_zp: vec![5; sy * sx],
        }
    }

    pub fn from_grid(grid: &LightGrid) -> Self {
        let (sx, sy, sz) = (grid.sx, grid.sy, grid.sz);
        let mut b = Self::new(sx, sy, sz);
        // Block light border from grid.block_light
        let idx3 = |x: usize, y: usize, z: usize| -> usize { (y * sz + z) * sx + x };
        // X- face at x=0
        for z in 0..sz {
            for y in 0..sy {
                b.xn[y * sz + z] = grid.block_light[idx3(0, y, z)];
                b.sk_xn[y * sz + z] = grid.skylight[idx3(0, y, z)];
                b.bcn_xn[y * sz + z] = grid.beacon_light[idx3(0, y, z)];
                let d = grid.beacon_dir[idx3(0, y, z)];
                // For -X face, treat source(0) or -X(2) as cardinal across -X; else non-cardinal
                b.bcn_dir_xn[y * sz + z] = if d == 2 || d == 0 { 2 } else { 5 };
            }
        }
        // X+ face at x=sx-1
        for z in 0..sz {
            for y in 0..sy {
                b.xp[y * sz + z] = grid.block_light[idx3(sx - 1, y, z)];
                b.sk_xp[y * sz + z] = grid.skylight[idx3(sx - 1, y, z)];
                b.bcn_xp[y * sz + z] = grid.beacon_light[idx3(sx - 1, y, z)];
                let d = grid.beacon_dir[idx3(sx - 1, y, z)];
                // For +X face, treat source(0) or +X(1) as cardinal across +X
                b.bcn_dir_xp[y * sz + z] = if d == 1 || d == 0 { 1 } else { 5 };
            }
        }
        // Z- face at z=0
        for x in 0..sx {
            for y in 0..sy {
                b.zn[y * sx + x] = grid.block_light[idx3(x, y, 0)];
                b.sk_zn[y * sx + x] = grid.skylight[idx3(x, y, 0)];
                b.bcn_zn[y * sx + x] = grid.beacon_light[idx3(x, y, 0)];
                let d = grid.beacon_dir[idx3(x, y, 0)];
                // For -Z face, treat source(0) or -Z(4) as cardinal across -Z
                b.bcn_dir_zn[y * sx + x] = if d == 4 || d == 0 { 4 } else { 5 };
            }
        }
        // Z+ face at z=sz-1
        for x in 0..sx {
            for y in 0..sy {
                b.zp[y * sx + x] = grid.block_light[idx3(x, y, sz - 1)];
                b.sk_zp[y * sx + x] = grid.skylight[idx3(x, y, sz - 1)];
                b.bcn_zp[y * sx + x] = grid.beacon_light[idx3(x, y, sz - 1)];
                let d = grid.beacon_dir[idx3(x, y, sz - 1)];
                // For +Z face, treat source(0) or +Z(3) as cardinal across +Z
                b.bcn_dir_zp[y * sx + x] = if d == 3 || d == 0 { 3 } else { 5 };
            }
        }
        // Y- face at y=0
        for z in 0..sz {
            for x in 0..sx {
                b.yn[z * sx + x] = grid.block_light[idx3(x, 0, z)];
                b.sk_yn[z * sx + x] = grid.skylight[idx3(x, 0, z)];
                b.bcn_yn[z * sx + x] = grid.beacon_light[idx3(x, 0, z)];
            }
        }
        // Y+ face at y=sy-1
        for z in 0..sz {
            for x in 0..sx {
                b.yp[z * sx + x] = grid.block_light[idx3(x, sy - 1, z)];
                b.sk_yp[z * sx + x] = grid.skylight[idx3(x, sy - 1, z)];
                b.bcn_yp[z * sx + x] = grid.beacon_light[idx3(x, sy - 1, z)];
            }
        }
        b
    }
}

pub struct LightingStore {
    sx: usize,
    sy: usize,
    sz: usize,
    borders: Mutex<HashMap<(i32, i32), LightBorders>>, // keyed by (cx,cz)
    emitters: Mutex<HashMap<(i32, i32), Vec<(usize, usize, usize, u8, bool)>>>, // (x,y,z,level,is_beacon)
}

impl LightingStore {
    pub fn new(sx: usize, sy: usize, sz: usize) -> Self {
        Self {
            sx,
            sy,
            sz,
            borders: Mutex::new(HashMap::new()),
            emitters: Mutex::new(HashMap::new()),
        }
    }

    // Remove all persisted lighting data associated with a chunk.
    // Called when a chunk is unloaded to prevent unbounded growth.
    pub fn clear_chunk(&self, cx: i32, cz: i32) {
        {
            let mut map = self.borders.lock().unwrap();
            map.remove(&(cx, cz));
        }
        {
            let mut map = self.emitters.lock().unwrap();
            map.remove(&(cx, cz));
        }
    }

    pub fn get_neighbor_borders(&self, cx: i32, cz: i32) -> NeighborBorders {
        let map = self.borders.lock().unwrap();
        let mut nb = NeighborBorders::empty(self.sx, self.sy, self.sz);
        if let Some(b) = map.get(&(cx - 1, cz)) {
            nb.xn = Some(b.xp.clone());
            nb.sk_xn = Some(b.sk_xp.clone());
            nb.bcn_xn = Some(b.bcn_xp.clone());
            nb.bcn_dir_xn = Some(b.bcn_dir_xp.clone());
        }
        if let Some(b) = map.get(&(cx + 1, cz)) {
            nb.xp = Some(b.xn.clone());
            nb.sk_xp = Some(b.sk_xn.clone());
            nb.bcn_xp = Some(b.bcn_xn.clone());
            nb.bcn_dir_xp = Some(b.bcn_dir_xn.clone());
        }
        if let Some(b) = map.get(&(cx, cz - 1)) {
            nb.zn = Some(b.zp.clone());
            nb.sk_zn = Some(b.sk_zp.clone());
            nb.bcn_zn = Some(b.bcn_zp.clone());
            nb.bcn_dir_zn = Some(b.bcn_dir_zp.clone());
        }
        if let Some(b) = map.get(&(cx, cz + 1)) {
            nb.zp = Some(b.zn.clone());
            nb.sk_zp = Some(b.sk_zn.clone());
            nb.bcn_zp = Some(b.bcn_zn.clone());
            nb.bcn_dir_zp = Some(b.bcn_dir_zn.clone());
        }
        // Vertical neighbors (not chunked vertically yet), leave None
        nb
    }

    pub fn update_borders(&self, cx: i32, cz: i32, lb: LightBorders) -> bool {
        let mut map = self.borders.lock().unwrap();
        match map.get_mut(&(cx, cz)) {
            Some(existing) => {
                let changed = !equal_planes(existing, &lb);
                if changed {
                    *existing = lb;
                }
                changed
            }
            None => {
                map.insert((cx, cz), lb);
                true
            }
        }
    }

    pub fn add_emitter_world(&self, wx: i32, wy: i32, wz: i32, level: u8) {
        self.add_emitter_world_typed(wx, wy, wz, level, false);
    }

    pub fn add_beacon_world(&self, wx: i32, wy: i32, wz: i32, level: u8) {
        self.add_emitter_world_typed(wx, wy, wz, level, true);
    }

    fn add_emitter_world_typed(&self, wx: i32, wy: i32, wz: i32, level: u8, is_beacon: bool) {
        if wy < 0 || wy >= self.sy as i32 {
            return;
        }
        let sx = self.sx as i32;
        let sz = self.sz as i32;
        let cx = wx.div_euclid(sx);
        let cz = wz.div_euclid(sz);
        let lx = wx.rem_euclid(sx) as usize;
        let lz = wz.rem_euclid(sz) as usize;
        let ly = wy as usize;
        let mut map = self.emitters.lock().unwrap();
        let v = map.entry((cx, cz)).or_insert_with(Vec::new);
        if !v
            .iter()
            .any(|&(x, y, z, _, _): &(usize, usize, usize, u8, bool)| x == lx && y == ly && z == lz)
        {
            v.push((lx, ly, lz, level, is_beacon));
        }
    }

    pub fn remove_emitter_world(&self, wx: i32, wy: i32, wz: i32) {
        if wy < 0 || wy >= self.sy as i32 {
            return;
        }
        let sx = self.sx as i32;
        let sz = self.sz as i32;
        let cx = wx.div_euclid(sx);
        let cz = wz.div_euclid(sz);
        let lx = wx.rem_euclid(sx) as usize;
        let lz = wz.rem_euclid(sz) as usize;
        let ly = wy as usize;
        let mut map = self.emitters.lock().unwrap();
        if let Some(v) = map.get_mut(&(cx, cz)) {
            v.retain(|&(x, y, z, _, _): &(usize, usize, usize, u8, bool)| {
                !(x == lx && y == ly && z == lz)
            });
            if v.is_empty() {
                map.remove(&(cx, cz));
            }
        }
    }

    pub fn emitters_for_chunk(&self, cx: i32, cz: i32) -> Vec<(usize, usize, usize, u8, bool)> {
        let map = self.emitters.lock().unwrap();
        map.get(&(cx, cz)).cloned().unwrap_or_default()
    }
}

fn equal_planes(a: &LightBorders, b: &LightBorders) -> bool {
    a.xn == b.xn
        && a.xp == b.xp
        && a.zn == b.zn
        && a.zp == b.zp
        && a.yn == b.yn
        && a.yp == b.yp
        && a.sk_xn == b.sk_xn
        && a.sk_xp == b.sk_xp
        && a.sk_zn == b.sk_zn
        && a.sk_zp == b.sk_zp
        && a.sk_yn == b.sk_yn
        && a.sk_yp == b.sk_yp
        && a.bcn_xn == b.bcn_xn
        && a.bcn_xp == b.bcn_xp
        && a.bcn_zn == b.bcn_zn
        && a.bcn_zp == b.bcn_zp
        && a.bcn_yn == b.bcn_yn
        && a.bcn_yp == b.bcn_yp
        && a.bcn_dir_xn == b.bcn_dir_xn
        && a.bcn_dir_xp == b.bcn_dir_xp
        && a.bcn_dir_zn == b.bcn_dir_zn
        && a.bcn_dir_zp == b.bcn_dir_zp
}

pub struct NeighborBorders {
    pub xn: Option<Vec<u8>>, // neighbor's +X into our -X
    pub xp: Option<Vec<u8>>, // neighbor's -X into our +X
    pub zn: Option<Vec<u8>>, // neighbor's +Z into our -Z
    pub zp: Option<Vec<u8>>, // neighbor's -Z into our +Z
    // Skylight planes
    pub sk_xn: Option<Vec<u8>>, // skylight for -X face
    pub sk_xp: Option<Vec<u8>>, // skylight for +X face
    pub sk_zn: Option<Vec<u8>>, // skylight for -Z face
    pub sk_zp: Option<Vec<u8>>, // skylight for +Z face
    // Beacon light planes
    pub bcn_xn: Option<Vec<u8>>, // beacon light for -X face
    pub bcn_xp: Option<Vec<u8>>, // beacon light for +X face
    pub bcn_zn: Option<Vec<u8>>, // beacon light for -Z face
    pub bcn_zp: Option<Vec<u8>>, // beacon light for +Z face
    // Beacon direction planes
    pub bcn_dir_xn: Option<Vec<u8>>, // direction for -X face entries
    pub bcn_dir_xp: Option<Vec<u8>>, // direction for +X face entries
    pub bcn_dir_zn: Option<Vec<u8>>, // direction for -Z face entries
    pub bcn_dir_zp: Option<Vec<u8>>, // direction for +Z face entries
}

impl NeighborBorders {
    pub fn empty(_sx: usize, _sy: usize, _sz: usize) -> Self {
        Self {
            xn: None,
            xp: None,
            zn: None,
            zp: None,
            sk_xn: None,
            sk_xp: None,
            sk_zn: None,
            sk_zp: None,
            bcn_xn: None,
            bcn_xp: None,
            bcn_zn: None,
            bcn_zp: None,
            bcn_dir_xn: None,
            bcn_dir_xp: None,
            bcn_dir_zn: None,
            bcn_dir_zp: None,
        }
    }
}

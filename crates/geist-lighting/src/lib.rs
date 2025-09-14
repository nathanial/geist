//! In-chunk lighting and neighbor border planes.
#![forbid(unsafe_code)]

use geist_blocks::BlockRegistry;
use geist_blocks::micro::micro_face_cell_open_s2;
use geist_blocks::types::Block;
use geist_chunk::ChunkBuf;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU8, Ordering};

mod micro;
mod micro_iter;

// Runtime toggle: allow disabling S=2 micro lighting entirely.
// When disabled, the engine runs a coarse voxel BFS with coarse face gates.

// Micro border planes for S=2 lighting exchange across seams.
// Arrays are stored per-face at micro resolution:
// - X faces: size = Ym * Zm, index = my * Zm + mz
// - Y faces: size = Xm * Zm, index = mz * Xm + mx
// - Z faces: size = Xm * Ym, index = my * Xm + mx
#[derive(Clone)]
pub struct MicroBorders {
    pub xm_sk_neg: Arc<[u8]>,
    pub xm_sk_pos: Arc<[u8]>,
    pub ym_sk_neg: Arc<[u8]>,
    pub ym_sk_pos: Arc<[u8]>,
    pub zm_sk_neg: Arc<[u8]>,
    pub zm_sk_pos: Arc<[u8]>,
    pub xm_bl_neg: Arc<[u8]>,
    pub xm_bl_pos: Arc<[u8]>,
    pub ym_bl_neg: Arc<[u8]>,
    pub ym_bl_pos: Arc<[u8]>,
    pub zm_bl_neg: Arc<[u8]>,
    pub zm_bl_pos: Arc<[u8]>,
    pub xm: usize,
    pub ym: usize,
    pub zm: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BorderChangeMask {
    pub xn: bool,
    pub xp: bool,
    pub zn: bool,
    pub zp: bool,
    pub yn: bool,
    pub yp: bool,
}

pub struct NeighborMicroBorders {
    pub xm_sk_neg: Option<Arc<[u8]>>,
    pub xm_sk_pos: Option<Arc<[u8]>>,
    pub ym_sk_neg: Option<Arc<[u8]>>,
    pub ym_sk_pos: Option<Arc<[u8]>>,
    pub zm_sk_neg: Option<Arc<[u8]>>,
    pub zm_sk_pos: Option<Arc<[u8]>>,
    pub xm_bl_neg: Option<Arc<[u8]>>,
    pub xm_bl_pos: Option<Arc<[u8]>>,
    pub ym_bl_neg: Option<Arc<[u8]>>,
    pub ym_bl_pos: Option<Arc<[u8]>>,
    pub zm_bl_neg: Option<Arc<[u8]>>,
    pub zm_bl_pos: Option<Arc<[u8]>>,
    pub xm: usize,
    pub ym: usize,
    pub zm: usize,
}

#[cfg(test)]
#[inline]
fn occ_bit(occ: u8, x: usize, y: usize, z: usize) -> bool {
    let idx = ((y & 1) << 2) | ((z & 1) << 1) | (x & 1);
    (occ & (1u8 << idx)) != 0
}

#[inline]
fn occ8_for(reg: &BlockRegistry, b: Block) -> Option<u8> {
    reg.get(b.id).and_then(|ty| ty.variant(b.state).occupancy)
}

#[inline]
fn is_full_cube(reg: &BlockRegistry, b: Block) -> bool {
    reg.get(b.id)
        .map(|ty| {
            ty.is_solid(b.state)
                && matches!(
                    ty.shape,
                    geist_blocks::types::Shape::Cube | geist_blocks::types::Shape::AxisCube { .. }
                )
        })
        .unwrap_or(false)
}

// Bit masks for S=2 micro-occupancy faces (2x2x2 bits in layout idx=(y<<2)|(z<<1)|x)
#[inline]
fn occ8_mask_for_face_x0() -> u8 {
    0x55 // bits {0,2,4,6}
}
#[inline]
fn occ8_mask_for_face_x1() -> u8 {
    0xAA // bits {1,3,5,7}
}
#[inline]
fn occ8_mask_for_face_y0() -> u8 {
    0x0F // bits {0..3}
}
#[inline]
fn occ8_mask_for_face_y1() -> u8 {
    0xF0 // bits {4..7}
}
#[inline]
fn occ8_mask_for_face_z0() -> u8 {
    0x33 // bits {0,1,4,5}
}
#[inline]
fn occ8_mask_for_face_z1() -> u8 {
    0xCC // bits {2,3,6,7}
}

// Decide if a face between (x,y,z) and its neighbor in `face` direction is open for light at S=2.
// face indices: 0=+Y,1=-Y,2=+X,3=-X,4=+Z,5=-Z (matches registry/mesher)
#[inline]
fn can_cross_face_s2(
    buf: &ChunkBuf,
    reg: &BlockRegistry,
    x: usize,
    y: usize,
    z: usize,
    face: usize,
) -> bool {
    let (nx, ny, nz) = match face {
        0 => (x as i32, y as i32 + 1, z as i32),
        1 => (x as i32, y as i32 - 1, z as i32),
        2 => (x as i32 + 1, y as i32, z as i32),
        3 => (x as i32 - 1, y as i32, z as i32),
        4 => (x as i32, y as i32, z as i32 + 1),
        5 => (x as i32, y as i32, z as i32 - 1),
        _ => return false,
    };
    if nx < 0
        || ny < 0
        || nz < 0
        || nx >= buf.sx as i32
        || ny >= buf.sy as i32
        || nz >= buf.sz as i32
    {
        return false;
    }
    let here = buf.get_local(x, y, z);
    let there = buf.get_local(nx as usize, ny as usize, nz as usize);
    // Cross if any of the four micro face cells is open
    for i0 in 0..2 {
        for i1 in 0..2 {
            if micro_face_cell_open_s2(reg, here, there, face, i0, i1) {
                return true;
            }
        }
    }
    false
}

pub struct LightGrid {
    pub(crate) sx: usize,
    pub(crate) sy: usize,
    pub(crate) sz: usize,
    pub(crate) skylight: Vec<u8>,
    pub(crate) block_light: Vec<u8>,
    pub(crate) beacon_light: Vec<u8>,
    pub(crate) beacon_dir: Vec<u8>,
    // Optional micro-light fields (present in MicroS2 mode)
    pub(crate) m_sky: Option<Vec<u8>>, // size = (2*sx)*(2*sy)*(2*sz)
    pub(crate) m_blk: Option<Vec<u8>>, // size = (2*sx)*(2*sy)*(2*sz)
    pub(crate) mxs: usize,
    pub(crate) mys: usize,
    pub(crate) mzs: usize,
    // Optional neighbor micro border planes at chunk seams
    // X faces: size = mys * mzs (index = my * mzs + mz)
    pub(crate) mnb_xn_sky: Option<Arc<[u8]>>,
    pub(crate) mnb_xp_sky: Option<Arc<[u8]>>,
    pub(crate) mnb_xn_blk: Option<Arc<[u8]>>,
    pub(crate) mnb_xp_blk: Option<Arc<[u8]>>,
    // Z faces: size = mys * mxs (index = my * mxs + mx)
    pub(crate) mnb_zn_sky: Option<Arc<[u8]>>,
    pub(crate) mnb_zp_sky: Option<Arc<[u8]>>,
    pub(crate) mnb_zn_blk: Option<Arc<[u8]>>,
    pub(crate) mnb_zp_blk: Option<Arc<[u8]>>,
    // Y faces (usually not chunked vertically): size = mzs * mxs (index = mz * mxs + mx)
    pub(crate) mnb_yn_sky: Option<Arc<[u8]>>,
    pub(crate) mnb_yp_sky: Option<Arc<[u8]>>,
    pub(crate) mnb_yn_blk: Option<Arc<[u8]>>,
    pub(crate) mnb_yp_blk: Option<Arc<[u8]>>,
    pub(crate) nb_xn_blk: Option<Arc<[u8]>>,
    pub(crate) nb_xp_blk: Option<Arc<[u8]>>,
    pub(crate) nb_zn_blk: Option<Arc<[u8]>>,
    pub(crate) nb_zp_blk: Option<Arc<[u8]>>,
    pub(crate) nb_xn_sky: Option<Arc<[u8]>>,
    pub(crate) nb_xp_sky: Option<Arc<[u8]>>,
    pub(crate) nb_zn_sky: Option<Arc<[u8]>>,
    pub(crate) nb_zp_sky: Option<Arc<[u8]>>,
    pub(crate) nb_xn_bcn: Option<Arc<[u8]>>,
    pub(crate) nb_xp_bcn: Option<Arc<[u8]>>,
    pub(crate) nb_zn_bcn: Option<Arc<[u8]>>,
    pub(crate) nb_zp_bcn: Option<Arc<[u8]>>,
    pub(crate) nb_xn_bcn_dir: Option<Arc<[u8]>>,
    pub(crate) nb_xp_bcn_dir: Option<Arc<[u8]>>,
    pub(crate) nb_zn_bcn_dir: Option<Arc<[u8]>>,
    pub(crate) nb_zp_bcn_dir: Option<Arc<[u8]>>,
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
            m_sky: None,
            m_blk: None,
            mxs: sx * 2,
            mys: sy * 2,
            mzs: sz * 2,
            mnb_xn_sky: None,
            mnb_xp_sky: None,
            mnb_xn_blk: None,
            mnb_xp_blk: None,
            mnb_zn_sky: None,
            mnb_zp_sky: None,
            mnb_zn_blk: None,
            mnb_zp_blk: None,
            mnb_yn_sky: None,
            mnb_yp_sky: None,
            mnb_yn_blk: None,
            mnb_yp_blk: None,
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

    pub fn compute_with_borders_buf(
        buf: &ChunkBuf,
        store: &LightingStore,
        reg: &BlockRegistry,
    ) -> Self {
        let sx = buf.sx;
        let sy = buf.sy;
        let sz = buf.sz;
        let mut lg = Self::new(sx, sy, sz);
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
        let mut q: VecDeque<(usize, usize, usize, u8, u8)> = VecDeque::new();
        #[allow(clippy::type_complexity)]
        let mut q_beacon: VecDeque<(usize, usize, usize, u8, u8, u8, u8, u8)> = VecDeque::new();
        for z in 0..sz {
            for y in 0..sy {
                for x in 0..sx {
                    let b = buf.get_local(x, y, z);
                    if let Some(ty) = reg.get(b.id) {
                        let em = ty.light_emission(b.state);
                        if em > 0 {
                            let idx = lg.idx(x, y, z);
                            if ty.light_is_beam() {
                                lg.beacon_light[idx] = em;
                                lg.beacon_dir[idx] = 0;
                                let (sc, tc, vc, _sd) = ty.beam_params();
                                q_beacon.push_back((x, y, z, em, 0, sc, tc, vc));
                            } else {
                                lg.block_light[idx] = em;
                                let att = ty.omni_attenuation();
                                q.push_back((x, y, z, em, att));
                            }
                        }
                    }
                }
            }
        }
        // Seed from neighbors
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
        let atten: i32 = 32;
        if let Some(ref plane) = nb.xn {
            for z in 0..sz {
                for y in 0..sy {
                    let v = plane[y * sz + z] as i32 - atten;
                    if v > 0 {
                        let v8 = v as u8;
                        let idx = lg.idx(0, y, z);
                        if lg.block_light[idx] < v8 {
                            lg.block_light[idx] = v8;
                            q.push_back((0, y, z, v8, 32));
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
                            q.push_back((xx, y, z, v8, 32));
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
                            q.push_back((x, y, 0, v8, 32));
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
                            q.push_back((x, y, zz, v8, 32));
                        }
                    }
                }
            }
        }
        // Beacon from neighbors (respect direction planes)
        if let Some(ref plane) = nb.bcn_xn {
            for z in 0..sz {
                for y in 0..sy {
                    let orig_v = plane[y * sz + z];
                    let dir = lg
                        .nb_xn_bcn_dir
                        .as_ref()
                        .and_then(|p| p.get(y * sz + z).cloned())
                        .unwrap_or(5);
                    let atten = if (1..=4).contains(&dir) { 1 } else { 32 };
                    let v = orig_v as i32 - atten;
                    if v > 0 {
                        let v8 = v as u8;
                        let idx = lg.idx(0, y, z);
                        if lg.beacon_light[idx] < v8 {
                            lg.beacon_light[idx] = v8;
                            lg.beacon_dir[idx] = dir;
                            q_beacon.push_back((0, y, z, v8, dir, 1, 32, 32));
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
                    let atten = if (1..=4).contains(&dir) { 1 } else { 32 };
                    let v = orig_v as i32 - atten;
                    if v > 0 {
                        let v8 = v as u8;
                        let xx = sx - 1;
                        let idx = lg.idx(xx, y, z);
                        if lg.beacon_light[idx] < v8 {
                            lg.beacon_light[idx] = v8;
                            lg.beacon_dir[idx] = dir;
                            q_beacon.push_back((xx, y, z, v8, dir, 1, 32, 32));
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
                    let atten = if (1..=4).contains(&dir) { 1 } else { 32 };
                    let v = orig_v as i32 - atten;
                    if v > 0 {
                        let v8 = v as u8;
                        let idx = lg.idx(x, y, 0);
                        if lg.beacon_light[idx] < v8 {
                            lg.beacon_light[idx] = v8;
                            lg.beacon_dir[idx] = dir;
                            q_beacon.push_back((x, y, 0, v8, dir, 1, 32, 32));
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
                    let atten = if (1..=4).contains(&dir) { 1 } else { 32 };
                    let v = orig_v as i32 - atten;
                    if v > 0 {
                        let v8 = v as u8;
                        let zz = sz - 1;
                        let idx = lg.idx(x, y, zz);
                        if lg.beacon_light[idx] < v8 {
                            lg.beacon_light[idx] = v8;
                            lg.beacon_dir[idx] = dir;
                            q_beacon.push_back((x, y, zz, v8, dir, 1, 32, 32));
                        }
                    }
                }
            }
        }
        // Skylight neighbors
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
        // Propagate omni block light
        while let Some((x, y, z, level, atten)) = q.pop_front() {
            let level_i = level as i32;
            if level_i <= 1 {
                continue;
            }
            let mut try_push = |nx: i32, ny: i32, nz: i32, face: usize| {
                if nx < 0
                    || ny < 0
                    || nz < 0
                    || nx >= sx as i32
                    || ny >= sy as i32
                    || nz >= sz as i32
                {
                    return;
                }
                let nb = buf.get_local(nx as usize, ny as usize, nz as usize);
                if !block_light_passable(nb, reg) {
                    return;
                }
                // Require S=2 face-open plane to cross
                if !can_cross_face_s2(buf, reg, x, y, z, face) {
                    return;
                }
                let idx = lg.idx(nx as usize, ny as usize, nz as usize);
                let v = level_i - atten as i32;
                if v > 0 {
                    let v8 = v as u8;
                    if lg.block_light[idx] < v8 {
                        lg.block_light[idx] = v8;
                        q.push_back((nx as usize, ny as usize, nz as usize, v8, atten));
                    }
                }
            };
            try_push(x as i32 + 1, y as i32, z as i32, 2); // +X
            try_push(x as i32 - 1, y as i32, z as i32, 3); // -X
            try_push(x as i32, y as i32 + 1, z as i32, 0); // +Y
            try_push(x as i32, y as i32 - 1, z as i32, 1); // -Y
            try_push(x as i32, y as i32, z as i32 + 1, 4); // +Z
            try_push(x as i32, y as i32, z as i32 - 1, 5); // -Z
        }
        // Propagate beacon light with direction-aware attenuation
        while let Some((x, y, z, level, dir, sc, tc, vc)) = q_beacon.pop_front() {
            let level_i = level as i32;
            if level_i <= 1 {
                continue;
            }
            let mut push_dir = |nx: i32, ny: i32, nz: i32, step_dir: u8| {
                if nx < 0
                    || ny < 0
                    || nz < 0
                    || nx >= sx as i32
                    || ny >= sy as i32
                    || nz >= sz as i32
                {
                    return;
                }
                let nb = buf.get_local(nx as usize, ny as usize, nz as usize);
                // Face-aware crossing at S=2. Use same gating as omni.
                let face = match step_dir {
                    1 => 2,
                    2 => 3,
                    3 => 4,
                    4 => 5,
                    _ => {
                        if ny > y as i32 {
                            0
                        } else {
                            1
                        }
                    }
                };
                if !block_light_passable(nb, reg) {
                    return;
                }
                if !can_cross_face_s2(buf, reg, x, y, z, face) {
                    return;
                }
                let idx = lg.idx(nx as usize, ny as usize, nz as usize);
                // cost: straight vs turn vs vertical
                let cost = if dir == 0 || dir == step_dir {
                    sc as i32
                } else if step_dir == 1 || step_dir == 2 || step_dir == 3 || step_dir == 4 {
                    tc as i32
                } else {
                    vc as i32
                };
                let v = level_i - cost;
                if v > 0 {
                    let v8 = v as u8;
                    if lg.beacon_light[idx] < v8 {
                        lg.beacon_light[idx] = v8;
                        lg.beacon_dir[idx] = step_dir;
                        q_beacon.push_back((
                            nx as usize,
                            ny as usize,
                            nz as usize,
                            v8,
                            step_dir,
                            sc,
                            tc,
                            vc,
                        ));
                    }
                }
            };
            push_dir(x as i32 + 1, y as i32, z as i32, 1); // +X
            push_dir(x as i32 - 1, y as i32, z as i32, 2); // -X
            push_dir(x as i32, y as i32, z as i32 + 1, 3); // +Z
            push_dir(x as i32, y as i32, z as i32 - 1, 4); // -Z
            push_dir(x as i32, y as i32 + 1, z as i32, 5); // vertical/non-cardinal
            push_dir(x as i32, y as i32 - 1, z as i32, 5);
        }
        // Skylight propagation
        while let Some((x, y, z, level)) = q_sky.pop_front() {
            if level <= 1 {
                continue;
            }
            let mut try_push = |nx: i32, ny: i32, nz: i32, face: usize| {
                if nx < 0
                    || ny < 0
                    || nz < 0
                    || nx >= sx as i32
                    || ny >= sy as i32
                    || nz >= sz as i32
                {
                    return;
                }
                // Require S=2 face-open plane and transparent destination
                if !can_cross_face_s2(buf, reg, x, y, z, face) {
                    return;
                }
                let nb = buf.get_local(nx as usize, ny as usize, nz as usize);
                if !skylight_transparent_s2(nb, reg) {
                    return;
                }
                let idx = lg.idx(nx as usize, ny as usize, nz as usize);
                let sky_att: i32 = 32;
                let v = (level as i32) - sky_att;
                if v > 0 {
                    let v8 = v as u8;
                    if lg.skylight[idx] < v8 {
                        lg.skylight[idx] = v8;
                        q_sky.push_back((nx as usize, ny as usize, nz as usize, v8));
                    }
                }
            };
            try_push(x as i32 + 1, y as i32, z as i32, 2); // +X
            try_push(x as i32 - 1, y as i32, z as i32, 3); // -X
            try_push(x as i32, y as i32 + 1, z as i32, 0); // +Y
            try_push(x as i32, y as i32 - 1, z as i32, 1); // -Y
            try_push(x as i32, y as i32, z as i32 + 1, 4); // +Z
            try_push(x as i32, y as i32, z as i32 - 1, 5); // -Z
        }
        lg
    }

    #[inline]
    pub fn neighbor_light_max(&self, x: usize, y: usize, z: usize, face: usize) -> u8 {
        let (nx, ny, nz) = match face {
            0 => (x as i32, y as i32 + 1, z as i32),
            1 => (x as i32, y as i32 - 1, z as i32),
            2 => (x as i32 + 1, y as i32, z as i32),
            3 => (x as i32 - 1, y as i32, z as i32),
            4 => (x as i32, y as i32, z as i32 + 1),
            5 => (x as i32, y as i32, z as i32 - 1),
            _ => return 0,
        };
        if nx < 0
            || ny < 0
            || nz < 0
            || nx >= self.sx as i32
            || ny >= self.sy as i32
            || nz >= self.sz as i32
        {
            match face {
                2 => {
                    let idxp = y * self.sz + z;
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
                    let maxn = sky.max(blk).max(bcn);
                    if maxn > 0 {
                        return maxn;
                    }
                    let i = self.idx(self.sx - 1, y, z);
                    return self.skylight[i]
                        .max(self.block_light[i])
                        .max(self.beacon_light[i]);
                }
                3 => {
                    let idxp = y * self.sz + z;
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
                    let maxn = sky.max(blk).max(bcn);
                    if maxn > 0 {
                        return maxn;
                    }
                    let i = self.idx(0, y, z);
                    return self.skylight[i]
                        .max(self.block_light[i])
                        .max(self.beacon_light[i]);
                }
                4 => {
                    let idxp = y * self.sx + x;
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
                    let maxn = sky.max(blk).max(bcn);
                    if maxn > 0 {
                        return maxn;
                    }
                    let i = self.idx(x, y, self.sz - 1);
                    return self.skylight[i]
                        .max(self.block_light[i])
                        .max(self.beacon_light[i]);
                }
                5 => {
                    let idxp = y * self.sx + x;
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
                    let maxn = sky.max(blk).max(bcn);
                    if maxn > 0 {
                        return maxn;
                    }
                    let i = self.idx(x, y, 0);
                    return self.skylight[i]
                        .max(self.block_light[i])
                        .max(self.beacon_light[i]);
                }
                _ => {}
            }
            return 0;
        }
        let i = self.idx(nx as usize, ny as usize, nz as usize);
        self.skylight[i]
            .max(self.block_light[i])
            .max(self.beacon_light[i])
    }

    #[inline]
    pub fn sample_face_local(&self, x: usize, y: usize, z: usize, face: usize) -> u8 {
        let i = self.idx(x, y, z);
        let local = self.skylight[i]
            .max(self.block_light[i])
            .max(self.beacon_light[i]);
        let nb = self.neighbor_light_max(x, y, z, face);
        local.max(nb)
    }

    // Face-aware light sample that respects S=2 micro openings for neighbor contribution
    pub fn sample_face_local_s2(
        &self,
        buf: &ChunkBuf,
        reg: &BlockRegistry,
        x: usize,
        y: usize,
        z: usize,
        face: usize,
    ) -> u8 {
        // If micro-light is available, compute face light by sampling the two
        // micro voxels across each plane micro cell and taking the maximum.
        if let (Some(ms), Some(mb)) = (&self.m_sky, &self.m_blk) {
            let mxs = self.mxs;
            let mys = self.mys;
            let mzs = self.mzs;
            let mut max_v: u8 = 0;
            let lval = |mx: usize, my: usize, mz: usize| -> u8 {
                if mx < mxs && my < mys && mz < mzs {
                    let i = (my * mzs + mz) * mxs + mx;
                    ms[i].max(mb[i])
                } else {
                    0
                }
            };
            let mut upd = |v: u8| {
                if v > max_v {
                    max_v = v;
                }
            };
            let bx = 2 * x;
            let by = 2 * y;
            let bz = 2 * z;
            match face {
                2 => {
                    // +X
                    let mx_here = bx + 1;
                    let mx_nb = bx + 2;
                    for oy in 0..2 {
                        for oz in 0..2 {
                            let my = by + oy;
                            let mz = bz + oz;
                            let a = lval(mx_here, my, mz);
                            let b = if mx_nb < mxs {
                                lval(mx_nb, my, mz)
                            } else {
                                if let Some(ref nbp) = self.mnb_xp_sky {
                                    let idx = my * mzs + mz;
                                    let sv = *nbp.get(idx).unwrap_or(&0);
                                    sv.max(
                                        *self
                                            .mnb_xp_blk
                                            .as_ref()
                                            .and_then(|p| p.get(idx))
                                            .unwrap_or(&0),
                                    )
                                } else {
                                    0
                                }
                            };
                            upd(a.max(b));
                        }
                    }
                }
                3 => {
                    // -X
                    let mx_here = bx;
                    let mx_nb = if bx > 0 { bx - 1 } else { mxs }; // sentinel
                    for oy in 0..2 {
                        for oz in 0..2 {
                            let my = by + oy;
                            let mz = bz + oz;
                            let a = lval(mx_here, my, mz);
                            let b = if mx_nb < mxs {
                                lval(mx_nb, my, mz)
                            } else {
                                if let Some(ref nbp) = self.mnb_xn_sky {
                                    let idx = my * mzs + mz;
                                    let sv = *nbp.get(idx).unwrap_or(&0);
                                    sv.max(
                                        *self
                                            .mnb_xn_blk
                                            .as_ref()
                                            .and_then(|p| p.get(idx))
                                            .unwrap_or(&0),
                                    )
                                } else {
                                    0
                                }
                            };
                            upd(a.max(b));
                        }
                    }
                }
                4 => {
                    // +Z
                    let mz_here = bz + 1;
                    let mz_nb = bz + 2;
                    for oy in 0..2 {
                        for ox in 0..2 {
                            let my = by + oy;
                            let mx = bx + ox;
                            let a = lval(mx, my, mz_here);
                            let b = if mz_nb < mzs {
                                lval(mx, my, mz_nb)
                            } else {
                                if let Some(ref nbp) = self.mnb_zp_sky {
                                    let idx = my * mxs + mx;
                                    let sv = *nbp.get(idx).unwrap_or(&0);
                                    sv.max(
                                        *self
                                            .mnb_zp_blk
                                            .as_ref()
                                            .and_then(|p| p.get(idx))
                                            .unwrap_or(&0),
                                    )
                                } else {
                                    0
                                }
                            };
                            upd(a.max(b));
                        }
                    }
                }
                5 => {
                    // -Z
                    let mz_here = bz;
                    let mz_nb = if bz > 0 { bz - 1 } else { mzs };
                    for oy in 0..2 {
                        for ox in 0..2 {
                            let my = by + oy;
                            let mx = bx + ox;
                            let a = lval(mx, my, mz_here);
                            let b = if mz_nb < mzs {
                                lval(mx, my, mz_nb)
                            } else {
                                if let Some(ref nbp) = self.mnb_zn_sky {
                                    let idx = my * mxs + mx;
                                    let sv = *nbp.get(idx).unwrap_or(&0);
                                    sv.max(
                                        *self
                                            .mnb_zn_blk
                                            .as_ref()
                                            .and_then(|p| p.get(idx))
                                            .unwrap_or(&0),
                                    )
                                } else {
                                    0
                                }
                            };
                            upd(a.max(b));
                        }
                    }
                }
                0 => {
                    // +Y
                    let my_here = by + 1;
                    let my_nb = by + 2;
                    for oz in 0..2 {
                        for ox in 0..2 {
                            let mz = bz + oz;
                            let mx = bx + ox;
                            let a = lval(mx, my_here, mz);
                            let b = if my_nb < mys {
                                lval(mx, my_nb, mz)
                            } else {
                                if let Some(ref nbp) = self.mnb_yp_sky {
                                    let idx = mz * mxs + mx;
                                    let sv = *nbp.get(idx).unwrap_or(&0);
                                    sv.max(
                                        *self
                                            .mnb_yp_blk
                                            .as_ref()
                                            .and_then(|p| p.get(idx))
                                            .unwrap_or(&0),
                                    )
                                } else {
                                    0
                                }
                            };
                            upd(a.max(b));
                        }
                    }
                }
                1 => {
                    // -Y
                    let my_here = by;
                    let my_nb = if by > 0 { by - 1 } else { mys };
                    for oz in 0..2 {
                        for ox in 0..2 {
                            let mz = bz + oz;
                            let mx = bx + ox;
                            let a = lval(mx, my_here, mz);
                            let b = if my_nb < mys {
                                lval(mx, my_nb, mz)
                            } else {
                                if let Some(ref nbp) = self.mnb_yn_sky {
                                    let idx = mz * mxs + mx;
                                    let sv = *nbp.get(idx).unwrap_or(&0);
                                    sv.max(
                                        *self
                                            .mnb_yn_blk
                                            .as_ref()
                                            .and_then(|p| p.get(idx))
                                            .unwrap_or(&0),
                                    )
                                } else {
                                    0
                                }
                            };
                            upd(a.max(b));
                        }
                    }
                }
                _ => {}
            }
            // Also consider local macro samples as safety nets
            // Include block light (emissive cubes) and beacon macro light.
            let macro_i = self.idx(x, y, z);
            return max_v
                .max(self.block_light[macro_i])
                .max(self.beacon_light[macro_i]);
        }
        let i = self.idx(x, y, z);
        let local = self.skylight[i]
            .max(self.block_light[i])
            .max(self.beacon_light[i]);
        // Compute neighbor coords
        let (nx, ny, nz) = match face {
            0 => (x as i32, y as i32 + 1, z as i32),
            1 => (x as i32, y as i32 - 1, z as i32),
            2 => (x as i32 + 1, y as i32, z as i32),
            3 => (x as i32 - 1, y as i32, z as i32),
            4 => (x as i32, y as i32, z as i32 + 1),
            5 => (x as i32, y as i32, z as i32 - 1),
            _ => return local,
        };
        // Out-of-bounds: fall back to border-aware neighbor max
        if nx < 0
            || ny < 0
            || nz < 0
            || nx >= buf.sx as i32
            || ny >= buf.sy as i32
            || nz >= buf.sz as i32
        {
            let nb = self.neighbor_light_max(x, y, z, face);
            return local.max(nb);
        }
        // Only the neighbor's micro occupancy can seal light reaching the boundary from that side.
        let there = buf.get_local(nx as usize, ny as usize, nz as usize);
        let all_covered = if let Some(o) = occ8_for(reg, there) {
            let mask = match face {
                2 => occ8_mask_for_face_x0(), // neighbor's x=0 plane
                3 => occ8_mask_for_face_x1(), // neighbor's x=1 plane
                0 => occ8_mask_for_face_y0(), // neighbor's y=0 plane
                1 => occ8_mask_for_face_y1(), // neighbor's y=1 plane
                4 => occ8_mask_for_face_z0(), // neighbor's z=0 plane
                5 => occ8_mask_for_face_z1(), // neighbor's z=1 plane
                _ => 0,
            };
            (o & mask) == mask
        } else {
            is_full_cube(reg, there)
        };
        if all_covered {
            return local;
        }
        // Otherwise, approximate the face-neighbor contribution by sampling the best among the micro-adjacent voxels
        let mut nb_max: u8 = 0;
        let mut upd = |sx_i: i32, sy_i: i32, sz_i: i32| {
            if sx_i >= 0
                && sy_i >= 0
                && sz_i >= 0
                && sx_i < buf.sx as i32
                && sy_i < buf.sy as i32
                && sz_i < buf.sz as i32
            {
                let idx = self.idx(sx_i as usize, sy_i as usize, sz_i as usize);
                let v = self.skylight[idx]
                    .max(self.block_light[idx])
                    .max(self.beacon_light[idx]);
                if v > nb_max {
                    nb_max = v;
                }
            }
        };
        match face {
            2 | 3 => {
                // X faces: sample around (nx,ny,nz) over Y/Z micro offsets
                for my in 0..=1 {
                    for mz in 0..=1 {
                        upd(nx, ny + my, nz + mz);
                    }
                }
            }
            0 | 1 => {
                // Y faces: sample around over X/Z
                for mx in 0..=1 {
                    for mz in 0..=1 {
                        upd(nx + mx, ny, nz + mz);
                    }
                }
            }
            4 | 5 => {
                // Z faces: sample around over X/Y
                for mx in 0..=1 {
                    for my in 0..=1 {
                        upd(nx + mx, ny + my, nz);
                    }
                }
            }
            _ => {}
        }
        local.max(nb_max)
    }
}

#[inline]
fn skylight_transparent(b: Block, reg: &BlockRegistry) -> bool {
    if b.id == reg.id_by_name("air").unwrap_or(0) {
        return true;
    }
    reg.get(b.id)
        .map(|ty| !ty.blocks_skylight(b.state))
        .unwrap_or(false)
}

// S=2-aware skylight transparency gate used during BFS propagation.
// It treats micro-occupancy blocks (slab/stairs) as enterable when
// can_cross_face_s2 has already validated the plane is open.
#[inline]
fn skylight_transparent_s2(b: Block, reg: &BlockRegistry) -> bool {
    // Air is transparent
    if b.id == reg.id_by_name("air").unwrap_or(0) {
        return true;
    }
    // Full cubes block skylight
    if is_full_cube(reg, b) {
        return false;
    }
    // Micro occupancy (e.g., slabs/stairs) should not block BFS
    if occ8_for(reg, b).is_some() {
        return true;
    }
    // Fallback to coarse flag for other shapes
    reg.get(b.id)
        .map(|ty| !ty.blocks_skylight(b.state))
        .unwrap_or(false)
}

#[inline]
fn block_light_passable(b: Block, reg: &BlockRegistry) -> bool {
    if b.id == reg.id_by_name("air").unwrap_or(0) {
        return true;
    }
    reg.get(b.id)
        .map(|ty| ty.propagates_light(b.state))
        .unwrap_or(false)
}

#[derive(Clone)]
pub struct LightBorders {
    pub xn: Arc<[u8]>,
    pub xp: Arc<[u8]>,
    pub zn: Arc<[u8]>,
    pub zp: Arc<[u8]>,
    pub yn: Arc<[u8]>,
    pub yp: Arc<[u8]>,
    pub sk_xn: Arc<[u8]>,
    pub sk_xp: Arc<[u8]>,
    pub sk_zn: Arc<[u8]>,
    pub sk_zp: Arc<[u8]>,
    pub sk_yn: Arc<[u8]>,
    pub sk_yp: Arc<[u8]>,
    pub bcn_xn: Arc<[u8]>,
    pub bcn_xp: Arc<[u8]>,
    pub bcn_zn: Arc<[u8]>,
    pub bcn_zp: Arc<[u8]>,
    pub bcn_yn: Arc<[u8]>,
    pub bcn_yp: Arc<[u8]>,
    pub bcn_dir_xn: Arc<[u8]>,
    pub bcn_dir_xp: Arc<[u8]>,
    pub bcn_dir_zn: Arc<[u8]>,
    pub bcn_dir_zp: Arc<[u8]>,
}

impl LightBorders {
    pub fn new(sx: usize, sy: usize, sz: usize) -> Self {
        Self {
            xn: vec![0; sy * sz].into(),
            xp: vec![0; sy * sz].into(),
            zn: vec![0; sy * sx].into(),
            zp: vec![0; sy * sx].into(),
            yn: vec![0; sx * sz].into(),
            yp: vec![0; sx * sz].into(),
            sk_xn: vec![0; sy * sz].into(),
            sk_xp: vec![0; sy * sz].into(),
            sk_zn: vec![0; sy * sx].into(),
            sk_zp: vec![0; sy * sx].into(),
            sk_yn: vec![0; sx * sz].into(),
            sk_yp: vec![0; sx * sz].into(),
            bcn_xn: vec![0; sy * sz].into(),
            bcn_xp: vec![0; sy * sz].into(),
            bcn_zn: vec![0; sy * sx].into(),
            bcn_zp: vec![0; sy * sx].into(),
            bcn_yn: vec![0; sx * sz].into(),
            bcn_yp: vec![0; sx * sz].into(),
            bcn_dir_xn: vec![5; sy * sz].into(),
            bcn_dir_xp: vec![5; sy * sz].into(),
            bcn_dir_zn: vec![5; sy * sx].into(),
            bcn_dir_zp: vec![5; sy * sx].into(),
        }
    }
    pub fn from_grid(grid: &LightGrid) -> Self {
        let (sx, sy, sz) = (grid.sx, grid.sy, grid.sz);
        let mut xn = vec![0u8; sy * sz];
        let mut xp = vec![0u8; sy * sz];
        let mut zn = vec![0u8; sy * sx];
        let mut zp = vec![0u8; sy * sx];
        let mut yn = vec![0u8; sx * sz];
        let mut yp = vec![0u8; sx * sz];
        let mut sk_xn = vec![0u8; sy * sz];
        let mut sk_xp = vec![0u8; sy * sz];
        let mut sk_zn = vec![0u8; sy * sx];
        let mut sk_zp = vec![0u8; sy * sx];
        let mut sk_yn = vec![0u8; sx * sz];
        let mut sk_yp = vec![0u8; sx * sz];
        let mut bcn_xn = vec![0u8; sy * sz];
        let mut bcn_xp = vec![0u8; sy * sz];
        let mut bcn_zn = vec![0u8; sy * sx];
        let mut bcn_zp = vec![0u8; sy * sx];
        let mut bcn_yn = vec![0u8; sx * sz];
        let mut bcn_yp = vec![0u8; sx * sz];
        let mut bcn_dir_xn = vec![5u8; sy * sz];
        let mut bcn_dir_xp = vec![5u8; sy * sz];
        let mut bcn_dir_zn = vec![5u8; sy * sx];
        let mut bcn_dir_zp = vec![5u8; sy * sx];
        let idx3 = |x: usize, y: usize, z: usize| -> usize { (y * sz + z) * sx + x };
        for z in 0..sz {
            for y in 0..sy {
                let ii = y * sz + z;
                xn[ii] = grid.block_light[idx3(0, y, z)];
                sk_xn[ii] = grid.skylight[idx3(0, y, z)];
                bcn_xn[ii] = grid.beacon_light[idx3(0, y, z)];
                let d = grid.beacon_dir[idx3(0, y, z)];
                bcn_dir_xn[ii] = if d == 2 || d == 0 { 2 } else { 5 };
            }
        }
        for z in 0..sz {
            for y in 0..sy {
                let ii = y * sz + z;
                xp[ii] = grid.block_light[idx3(sx - 1, y, z)];
                sk_xp[ii] = grid.skylight[idx3(sx - 1, y, z)];
                bcn_xp[ii] = grid.beacon_light[idx3(sx - 1, y, z)];
                let d = grid.beacon_dir[idx3(sx - 1, y, z)];
                bcn_dir_xp[ii] = if d == 1 || d == 0 { 1 } else { 5 };
            }
        }
        for x in 0..sx {
            for y in 0..sy {
                let ii = y * sx + x;
                zn[ii] = grid.block_light[idx3(x, y, 0)];
                sk_zn[ii] = grid.skylight[idx3(x, y, 0)];
                bcn_zn[ii] = grid.beacon_light[idx3(x, y, 0)];
                let d = grid.beacon_dir[idx3(x, y, 0)];
                bcn_dir_zn[ii] = if d == 4 || d == 0 { 4 } else { 5 };
            }
        }
        for x in 0..sx {
            for y in 0..sy {
                let ii = y * sx + x;
                zp[ii] = grid.block_light[idx3(x, y, sz - 1)];
                sk_zp[ii] = grid.skylight[idx3(x, y, sz - 1)];
                bcn_zp[ii] = grid.beacon_light[idx3(x, y, sz - 1)];
                let d = grid.beacon_dir[idx3(x, y, sz - 1)];
                bcn_dir_zp[ii] = if d == 3 || d == 0 { 3 } else { 5 };
            }
        }
        for z in 0..sz {
            for x in 0..sx {
                let ii = z * sx + x;
                yn[ii] = grid.block_light[idx3(x, 0, z)];
                sk_yn[ii] = grid.skylight[idx3(x, 0, z)];
                bcn_yn[ii] = grid.beacon_light[idx3(x, 0, z)];
            }
        }
        for z in 0..sz {
            for x in 0..sx {
                let ii = z * sx + x;
                yp[ii] = grid.block_light[idx3(x, sy - 1, z)];
                sk_yp[ii] = grid.skylight[idx3(x, sy - 1, z)];
                bcn_yp[ii] = grid.beacon_light[idx3(x, sy - 1, z)];
            }
        }
        Self {
            xn: xn.into(),
            xp: xp.into(),
            zn: zn.into(),
            zp: zp.into(),
            yn: yn.into(),
            yp: yp.into(),
            sk_xn: sk_xn.into(),
            sk_xp: sk_xp.into(),
            sk_zn: sk_zn.into(),
            sk_zp: sk_zp.into(),
            sk_yn: sk_yn.into(),
            sk_yp: sk_yp.into(),
            bcn_xn: bcn_xn.into(),
            bcn_xp: bcn_xp.into(),
            bcn_zn: bcn_zn.into(),
            bcn_zp: bcn_zp.into(),
            bcn_yn: bcn_yn.into(),
            bcn_yp: bcn_yp.into(),
            bcn_dir_xn: bcn_dir_xn.into(),
            bcn_dir_xp: bcn_dir_xp.into(),
            bcn_dir_zn: bcn_dir_zn.into(),
            bcn_dir_zp: bcn_dir_zp.into(),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LightingMode {
    FullMicro = 0,
    CoarseS2 = 1,
    SeamMicro = 2,
    IterativeCPU = 3,
}

pub struct LightingStore {
    sx: usize,
    sy: usize,
    sz: usize,
    borders: Mutex<HashMap<(i32, i32), LightBorders>>,
    emitters: Mutex<HashMap<(i32, i32), Vec<(usize, usize, usize, u8, bool)>>>,
    micro_borders: Mutex<HashMap<(i32, i32), MicroBorders>>,
    // Runtime mode selection
    mode: AtomicU8,
}

impl LightingStore {
    pub fn new(sx: usize, sy: usize, sz: usize) -> Self {
        Self {
            sx,
            sy,
            sz,
            borders: Mutex::new(HashMap::new()),
            emitters: Mutex::new(HashMap::new()),
            micro_borders: Mutex::new(HashMap::new()),
            // Default to coarse BFS with S=2 gating (fast, no dark quads near stairs)
            mode: AtomicU8::new(LightingMode::FullMicro as u8),
        }
    }
    /// Set the global lighting mode.
    pub fn set_mode(&self, m: LightingMode) {
        self.mode.store(m as u8, Ordering::Relaxed);
    }
    /// Read the global lighting mode.
    pub fn mode(&self) -> LightingMode {
        match self.mode.load(Ordering::Relaxed) {
            0 => LightingMode::FullMicro,
            2 => LightingMode::SeamMicro,
            3 => LightingMode::IterativeCPU,
            _ => LightingMode::CoarseS2,
        }
    }
    pub fn clear_chunk(&self, cx: i32, cz: i32) {
        {
            let mut m = self.borders.lock().unwrap();
            m.remove(&(cx, cz));
        }
        {
            let mut m = self.emitters.lock().unwrap();
            m.remove(&(cx, cz));
        }
        {
            let mut m = self.micro_borders.lock().unwrap();
            m.remove(&(cx, cz));
        }
    }
    pub fn clear_all_borders(&self) {
        let mut m = self.borders.lock().unwrap();
        m.clear();
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
        nb
    }
    /// Update stored borders and return whether anything changed, plus a per-face change mask.
    pub fn update_borders_mask(
        &self,
        cx: i32,
        cz: i32,
        lb: LightBorders,
    ) -> (bool, BorderChangeMask) {
        let mut map = self.borders.lock().unwrap();
        match map.get_mut(&(cx, cz)) {
            Some(existing) => {
                let mut mask = BorderChangeMask::default();
                // Per-face change detection (coarse + skylight + beacon planes)
                mask.xn = existing.xn.as_ref() != lb.xn.as_ref()
                    || existing.sk_xn.as_ref() != lb.sk_xn.as_ref()
                    || existing.bcn_xn.as_ref() != lb.bcn_xn.as_ref()
                    || existing.bcn_dir_xn.as_ref() != lb.bcn_dir_xn.as_ref();
                mask.xp = existing.xp.as_ref() != lb.xp.as_ref()
                    || existing.sk_xp.as_ref() != lb.sk_xp.as_ref()
                    || existing.bcn_xp.as_ref() != lb.bcn_xp.as_ref()
                    || existing.bcn_dir_xp.as_ref() != lb.bcn_dir_xp.as_ref();
                mask.zn = existing.zn.as_ref() != lb.zn.as_ref()
                    || existing.sk_zn.as_ref() != lb.sk_zn.as_ref()
                    || existing.bcn_zn.as_ref() != lb.bcn_zn.as_ref()
                    || existing.bcn_dir_zn.as_ref() != lb.bcn_dir_zn.as_ref();
                mask.zp = existing.zp.as_ref() != lb.zp.as_ref()
                    || existing.sk_zp.as_ref() != lb.sk_zp.as_ref()
                    || existing.bcn_zp.as_ref() != lb.bcn_zp.as_ref()
                    || existing.bcn_dir_zp.as_ref() != lb.bcn_dir_zp.as_ref();
                mask.yn = existing.yn.as_ref() != lb.yn.as_ref()
                    || existing.sk_yn.as_ref() != lb.sk_yn.as_ref()
                    || existing.bcn_yn.as_ref() != lb.bcn_yn.as_ref();
                mask.yp = existing.yp.as_ref() != lb.yp.as_ref()
                    || existing.sk_yp.as_ref() != lb.sk_yp.as_ref()
                    || existing.bcn_yp.as_ref() != lb.bcn_yp.as_ref();
                let any = mask.xn || mask.xp || mask.zn || mask.zp || mask.yn || mask.yp;
                if any {
                    *existing = lb;
                }
                (any, mask)
            }
            None => {
                // New entry: treat as a change; mark +X and +Z as changed for owner notification.
                let mut mask = BorderChangeMask::default();
                mask.xp = true;
                mask.zp = true;
                map.insert((cx, cz), lb);
                (true, mask)
            }
        }
    }
    /// Backward-compatible update that only returns 'changed' (any face).
    pub fn update_borders(&self, cx: i32, cz: i32, lb: LightBorders) -> bool {
        let (changed, _mask) = self.update_borders_mask(cx, cz, lb);
        changed
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
        let v = map.entry((cx, cz)).or_default();
        if !v
            .iter()
            .any(|&(x, y, z, _, _)| x == lx && y == ly && z == lz)
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
            v.retain(|&(x, y, z, _, _)| !(x == lx && y == ly && z == lz));
            if v.is_empty() {
                map.remove(&(cx, cz));
            }
        }
    }
    pub fn emitters_for_chunk(&self, cx: i32, cz: i32) -> Vec<(usize, usize, usize, u8, bool)> {
        let map = self.emitters.lock().unwrap();
        map.get(&(cx, cz)).cloned().unwrap_or_default()
    }
    pub fn update_micro_borders(&self, cx: i32, cz: i32, mb: MicroBorders) {
        let mut m = self.micro_borders.lock().unwrap();
        m.insert((cx, cz), mb);
    }
    pub fn get_neighbor_micro_borders(&self, cx: i32, cz: i32) -> NeighborMicroBorders {
        let xm = self.sx * 2;
        let ym = self.sy * 2;
        let zm = self.sz * 2;
        let map = self.micro_borders.lock().unwrap();
        let mut nb = NeighborMicroBorders {
            xm_sk_neg: None,
            xm_sk_pos: None,
            ym_sk_neg: None,
            ym_sk_pos: None,
            zm_sk_neg: None,
            zm_sk_pos: None,
            xm_bl_neg: None,
            xm_bl_pos: None,
            ym_bl_neg: None,
            ym_bl_pos: None,
            zm_bl_neg: None,
            zm_bl_pos: None,
            xm,
            ym,
            zm,
        };
        if let Some(m) = map.get(&(cx - 1, cz)) {
            nb.xm_sk_neg = Some(m.xm_sk_pos.clone());
            nb.xm_bl_neg = Some(m.xm_bl_pos.clone());
        }
        if let Some(m) = map.get(&(cx + 1, cz)) {
            nb.xm_sk_pos = Some(m.xm_sk_neg.clone());
            nb.xm_bl_pos = Some(m.xm_bl_neg.clone());
        }
        if let Some(m) = map.get(&(cx, cz - 1)) {
            nb.zm_sk_neg = Some(m.zm_sk_pos.clone());
            nb.zm_bl_neg = Some(m.zm_bl_pos.clone());
        }
        if let Some(m) = map.get(&(cx, cz + 1)) {
            nb.zm_sk_pos = Some(m.zm_sk_neg.clone());
            nb.zm_bl_pos = Some(m.zm_bl_neg.clone());
        }
        // Vertical neighbors are not chunked here; keep None. If vertically chunked, add mapping like above.
        nb
    }
}

#[cfg(test)]
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
    pub xn: Option<Arc<[u8]>>,
    pub xp: Option<Arc<[u8]>>,
    pub zn: Option<Arc<[u8]>>,
    pub zp: Option<Arc<[u8]>>,
    pub sk_xn: Option<Arc<[u8]>>,
    pub sk_xp: Option<Arc<[u8]>>,
    pub sk_zn: Option<Arc<[u8]>>,
    pub sk_zp: Option<Arc<[u8]>>,
    pub bcn_xn: Option<Arc<[u8]>>,
    pub bcn_xp: Option<Arc<[u8]>>,
    pub bcn_zn: Option<Arc<[u8]>>,
    pub bcn_zp: Option<Arc<[u8]>>,
    pub bcn_dir_xn: Option<Arc<[u8]>>,
    pub bcn_dir_xp: Option<Arc<[u8]>>,
    pub bcn_dir_zn: Option<Arc<[u8]>>,
    pub bcn_dir_zp: Option<Arc<[u8]>>,
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

// Entry point that chooses the lighting algorithm based on LightingStore runtime toggle.
use geist_world::World;

// use crate::micro::MICRO_SKY_ATTENUATION; // unused in this module

pub fn compute_light_with_borders_buf(
    buf: &ChunkBuf,
    store: &LightingStore,
    reg: &BlockRegistry,
    world: &World,
) -> LightGrid {
    match store.mode() {
        LightingMode::FullMicro => micro::compute_light_with_borders_buf_micro(buf, store, reg, world),
        LightingMode::CoarseS2 => LightGrid::compute_with_borders_buf(buf, store, reg),
        LightingMode::SeamMicro => {
            let lg = LightGrid::compute_with_borders_buf(buf, store, reg);
            publish_seam_micro_borders(buf, &lg, store);
            lg
        }
        LightingMode::IterativeCPU => micro_iter::compute_light_with_borders_buf_iterative(buf, store, reg),
    }
}

fn publish_seam_micro_borders(buf: &ChunkBuf, lg: &LightGrid, store: &LightingStore) {
    let mxs = buf.sx * 2;
    let mys = buf.sy * 2;
    let mzs = buf.sz * 2;
    let mut xm_sk_neg = vec![0u8; mys * mzs];
    let mut xm_bl_neg = vec![0u8; mys * mzs];
    let mut xm_sk_pos = vec![0u8; mys * mzs];
    let mut xm_bl_pos = vec![0u8; mys * mzs];
    let mut zm_sk_neg = vec![0u8; mys * mxs];
    let mut zm_bl_neg = vec![0u8; mys * mxs];
    let mut zm_sk_pos = vec![0u8; mys * mxs];
    let mut zm_bl_pos = vec![0u8; mys * mxs];
    let mut ym_sk_neg = vec![0u8; mzs * mxs];
    let mut ym_bl_neg = vec![0u8; mzs * mxs];
    let mut ym_sk_pos = vec![0u8; mzs * mxs];
    let mut ym_bl_pos = vec![0u8; mzs * mxs];
    let idx3 = |x: usize, y: usize, z: usize| -> usize { (y * buf.sz + z) * buf.sx + x };
    // X planes
    for my in 0..mys {
        let y = my >> 1;
        for mz in 0..mzs {
            let z = mz >> 1;
            let ii = my * mzs + mz;
            xm_sk_neg[ii] = lg.skylight[idx3(0, y, z)];
            xm_bl_neg[ii] = lg.block_light[idx3(0, y, z)];
            xm_sk_pos[ii] = lg.skylight[idx3(buf.sx - 1, y, z)];
            xm_bl_pos[ii] = lg.block_light[idx3(buf.sx - 1, y, z)];
        }
    }
    // Z planes
    for my in 0..mys {
        let y = my >> 1;
        for mx in 0..mxs {
            let x = mx >> 1;
            let ii = my * mxs + mx;
            zm_sk_neg[ii] = lg.skylight[idx3(x, y, 0)];
            zm_bl_neg[ii] = lg.block_light[idx3(x, y, 0)];
            zm_sk_pos[ii] = lg.skylight[idx3(x, y, buf.sz - 1)];
            zm_bl_pos[ii] = lg.block_light[idx3(x, y, buf.sz - 1)];
        }
    }
    // Y planes
    for mz in 0..mzs {
        let z = mz >> 1;
        for mx in 0..mxs {
            let x = mx >> 1;
            let ii = mz * mxs + mx;
            ym_sk_neg[ii] = lg.skylight[idx3(x, 0, z)];
            ym_bl_neg[ii] = lg.block_light[idx3(x, 0, z)];
            ym_sk_pos[ii] = lg.skylight[idx3(x, buf.sy - 1, z)];
            ym_bl_pos[ii] = lg.block_light[idx3(x, buf.sy - 1, z)];
        }
    }
    store.update_micro_borders(
        buf.cx,
        buf.cz,
        MicroBorders {
            xm_sk_neg: xm_sk_neg.into(),
            xm_sk_pos: xm_sk_pos.into(),
            ym_sk_neg: ym_sk_neg.into(),
            ym_sk_pos: ym_sk_pos.into(),
            zm_sk_neg: zm_sk_neg.into(),
            zm_sk_pos: zm_sk_pos.into(),
            xm_bl_neg: xm_bl_neg.into(),
            xm_bl_pos: xm_bl_pos.into(),
            ym_bl_neg: ym_bl_neg.into(),
            ym_bl_pos: ym_bl_pos.into(),
            zm_bl_neg: zm_bl_neg.into(),
            zm_bl_pos: zm_bl_pos.into(),
            xm: mxs,
            ym: mys,
            zm: mzs,
        },
    );
}

// --- GPU lightfield (Phase 2) helpers ---

/// Packed 2D atlas representation of a chunk lightfield for shader sampling.
/// Layout: each Y slice [0..sy) is a tile of size ((sx+2) x (sz+2)) arranged in a grid
/// of (grid_cols x grid_rows). The +2 accounts for border rings on both -X/+X and -Z/+Z
/// sides to enable seamless sampling across chunk boundaries in the shader.
/// Pixel format is RGBA8 where:
/// - R = block light (0..255)
/// - G = skylight (0..255)
/// - B = beacon light (0..255)
/// - A = beacon primary direction (0..5) scaled to 0..255 for debug/optional use
#[derive(Clone)]
pub struct LightAtlas {
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub sx: usize,
    pub sy: usize,
    pub sz: usize,
    pub grid_cols: usize,
    pub grid_rows: usize,
}

// Removed: worker-side atlas packing (`pack_light_grid_atlas`). Use
// `pack_light_grid_atlas_with_neighbors` to assemble rings from authoritative borders.

/// Packs a `LightGrid` into a 2D RGBA8 atlas using the provided neighbor borders
/// (fetched live from the `LightingStore` or cached externally). This avoids races
/// where the worker-computed grid's embedded neighbor planes may be stale by the
/// time of upload.
pub fn pack_light_grid_atlas_with_neighbors(light: &LightGrid, nb: &NeighborBorders) -> LightAtlas {
    let sx = light.sx;
    let sy = light.sy;
    let sz = light.sz;
    // Choose grid columns ~ sqrt(sy)
    let mut grid_cols = (sy as f32).sqrt().ceil() as usize;
    if grid_cols == 0 { grid_cols = 1; }
    let grid_rows = ((sy + grid_cols - 1) / grid_cols).max(1);
    // Include -X/+X and -Z/+Z border rings
    let tile_w = sx + 2;
    let tile_h = sz + 2;
    let width = tile_w * grid_cols;
    let height = tile_h * grid_rows;
    let mut data: Vec<u8> = vec![0u8; width * height * 4];
    let idx3 = |x: usize, y: usize, z: usize| -> usize { (y * sz + z) * sx + x };
    for y in 0..sy {
        let tx = y % grid_cols;
        let ty = y / grid_cols;
        let ox = tx * tile_w;
        let oy = ty * tile_h;
        // Interior
        for z in 0..sz {
            for x in 0..sx {
                let src = idx3(x, y, z);
                let dst_x = ox + 1 + x;
                let dst_y = oy + 1 + z;
                let di = (dst_y * width + dst_x) * 4;
                data[di + 0] = light.block_light[src];
                data[di + 1] = light.skylight[src];
                data[di + 2] = light.beacon_light[src];
                data[di + 3] = match light.beacon_dir[src] { v => (v as f32 * (255.0/5.0)).round() as u8 };
            }
        }
        // +X ring (from nb.xp)
        if let (Some(nb_blk), Some(nb_sky), Some(nb_bcn)) = (&nb.xp, &nb.sk_xp, &nb.bcn_xp) {
            for z in 0..sz {
                let dst_x = ox + (sx + 1);
                let dst_y = oy + 1 + z;
                let di = (dst_y * width + dst_x) * 4;
                let ii = y * sz + z;
                data[di + 0] = nb_blk.get(ii).cloned().unwrap_or(0);
                data[di + 1] = nb_sky.get(ii).cloned().unwrap_or(0);
                data[di + 2] = nb_bcn.get(ii).cloned().unwrap_or(0);
                data[di + 3] = 0;
            }
        }
        // -X ring (from nb.xn)
        if let (Some(nb_blk), Some(nb_sky), Some(nb_bcn)) = (&nb.xn, &nb.sk_xn, &nb.bcn_xn) {
            for z in 0..sz {
                let dst_x = ox + 0;
                let dst_y = oy + 1 + z;
                let di = (dst_y * width + dst_x) * 4;
                let ii = y * sz + z;
                data[di + 0] = nb_blk.get(ii).cloned().unwrap_or(0);
                data[di + 1] = nb_sky.get(ii).cloned().unwrap_or(0);
                data[di + 2] = nb_bcn.get(ii).cloned().unwrap_or(0);
                data[di + 3] = 0;
            }
        }
        // +Z ring (from nb.zp)
        if let (Some(nb_blk), Some(nb_sky), Some(nb_bcn)) = (&nb.zp, &nb.sk_zp, &nb.bcn_zp) {
            for x in 0..sx {
                let dst_x = ox + 1 + x;
                let dst_y = oy + (sz + 1);
                let di = (dst_y * width + dst_x) * 4;
                let ii = y * sx + x;
                data[di + 0] = nb_blk.get(ii).cloned().unwrap_or(0);
                data[di + 1] = nb_sky.get(ii).cloned().unwrap_or(0);
                data[di + 2] = nb_bcn.get(ii).cloned().unwrap_or(0);
                data[di + 3] = 0;
            }
        }
        // -Z ring (from nb.zn)
        if let (Some(nb_blk), Some(nb_sky), Some(nb_bcn)) = (&nb.zn, &nb.sk_zn, &nb.bcn_zn) {
            for x in 0..sx {
                let dst_x = ox + 1 + x;
                let dst_y = oy + 0;
                let di = (dst_y * width + dst_x) * 4;
                let ii = y * sx + x;
                data[di + 0] = nb_blk.get(ii).cloned().unwrap_or(0);
                data[di + 1] = nb_sky.get(ii).cloned().unwrap_or(0);
                data[di + 2] = nb_bcn.get(ii).cloned().unwrap_or(0);
                data[di + 3] = 0;
            }
        }
    }
    LightAtlas { data, width, height, sx: sx + 2, sy, sz: sz + 2, grid_cols, grid_rows }
}

#[cfg(test)]
mod tests;

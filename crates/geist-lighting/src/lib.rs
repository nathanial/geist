//! In-chunk lighting and neighbor border planes.
#![forbid(unsafe_code)]

use geist_blocks::BlockRegistry;
use geist_blocks::micro::{micro_cell_solid_s2, micro_face_cell_open_s2};
use geist_blocks::types::Block;
use geist_chunk::ChunkBuf;
use std::collections::HashMap;
use std::sync::Mutex;

mod micro;

// Lighting mode toggle removed; Micro S=2 is always used.

// Micro border planes for S=2 lighting exchange across seams.
// Arrays are stored per-face at micro resolution:
// - X faces: size = Ym * Zm, index = my * Zm + mz
// - Y faces: size = Xm * Zm, index = mz * Xm + mx
// - Z faces: size = Xm * Ym, index = my * Xm + mx
#[derive(Clone)]
pub struct MicroBorders {
    pub xm_sk_neg: Vec<u8>,
    pub xm_sk_pos: Vec<u8>,
    pub ym_sk_neg: Vec<u8>,
    pub ym_sk_pos: Vec<u8>,
    pub zm_sk_neg: Vec<u8>,
    pub zm_sk_pos: Vec<u8>,
    pub xm_bl_neg: Vec<u8>,
    pub xm_bl_pos: Vec<u8>,
    pub ym_bl_neg: Vec<u8>,
    pub ym_bl_pos: Vec<u8>,
    pub zm_bl_neg: Vec<u8>,
    pub zm_bl_pos: Vec<u8>,
    pub xm: usize,
    pub ym: usize,
    pub zm: usize,
}

pub struct NeighborMicroBorders {
    pub xm_sk_neg: Option<Vec<u8>>,
    pub xm_sk_pos: Option<Vec<u8>>,
    pub ym_sk_neg: Option<Vec<u8>>,
    pub ym_sk_pos: Option<Vec<u8>>,
    pub zm_sk_neg: Option<Vec<u8>>,
    pub zm_sk_pos: Option<Vec<u8>>,
    pub xm_bl_neg: Option<Vec<u8>>,
    pub xm_bl_pos: Option<Vec<u8>>,
    pub ym_bl_neg: Option<Vec<u8>>,
    pub ym_bl_pos: Option<Vec<u8>>,
    pub zm_bl_neg: Option<Vec<u8>>,
    pub zm_bl_pos: Option<Vec<u8>>,
    pub xm: usize,
    pub ym: usize,
    pub zm: usize,
}

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
    pub(crate) mnb_xn_sky: Option<Vec<u8>>,
    pub(crate) mnb_xp_sky: Option<Vec<u8>>,
    pub(crate) mnb_xn_blk: Option<Vec<u8>>,
    pub(crate) mnb_xp_blk: Option<Vec<u8>>,
    // Z faces: size = mys * mxs (index = my * mxs + mx)
    pub(crate) mnb_zn_sky: Option<Vec<u8>>,
    pub(crate) mnb_zp_sky: Option<Vec<u8>>,
    pub(crate) mnb_zn_blk: Option<Vec<u8>>,
    pub(crate) mnb_zp_blk: Option<Vec<u8>>,
    // Y faces (usually not chunked vertically): size = mzs * mxs (index = mz * mxs + mx)
    pub(crate) mnb_yn_sky: Option<Vec<u8>>,
    pub(crate) mnb_yp_sky: Option<Vec<u8>>,
    pub(crate) mnb_yn_blk: Option<Vec<u8>>,
    pub(crate) mnb_yp_blk: Option<Vec<u8>>,
    pub(crate) nb_xn_blk: Option<Vec<u8>>,
    pub(crate) nb_xp_blk: Option<Vec<u8>>,
    pub(crate) nb_zn_blk: Option<Vec<u8>>,
    pub(crate) nb_zp_blk: Option<Vec<u8>>,
    pub(crate) nb_xn_sky: Option<Vec<u8>>,
    pub(crate) nb_xp_sky: Option<Vec<u8>>,
    pub(crate) nb_zn_sky: Option<Vec<u8>>,
    pub(crate) nb_zp_sky: Option<Vec<u8>>,
    pub(crate) nb_xn_bcn: Option<Vec<u8>>,
    pub(crate) nb_xp_bcn: Option<Vec<u8>>,
    pub(crate) nb_zn_bcn: Option<Vec<u8>>,
    pub(crate) nb_zp_bcn: Option<Vec<u8>>,
    pub(crate) nb_xn_bcn_dir: Option<Vec<u8>>,
    pub(crate) nb_xp_bcn_dir: Option<Vec<u8>>,
    pub(crate) nb_zn_bcn_dir: Option<Vec<u8>>,
    pub(crate) nb_zp_bcn_dir: Option<Vec<u8>>,
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
        // Propagate omni block light (face-aware at S=2 for micro occupancy)
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
                // Face-aware crossing: allow step only if crossing plane is open at S=2, or
                // fall back to legacy passability when neither side uses micro occupancy.
                let nb = buf.get_local(nx as usize, ny as usize, nz as usize);
                if !block_light_passable(nb, reg) {
                    return;
                }
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
        // Skylight propagation (face-aware at S=2)
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
                // Require the crossing plane to be open at S=2, and the target voxel to be skylight transparent
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
            // Also consider local beacon light at the macro sample as a safety net (micro beacons unsupported)
            let macro_i = self.idx(x, y, z);
            return max_v.max(self.beacon_light[macro_i]);
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
        let mut all_covered = true;
        match face {
            2 => {
                for my in 0..2 {
                    for mz in 0..2 {
                        if !micro_cell_solid_s2(reg, there, 0, my, mz) {
                            all_covered = false;
                            break;
                        }
                    }
                    if !all_covered {
                        break;
                    }
                }
            }
            3 => {
                for my in 0..2 {
                    for mz in 0..2 {
                        if !micro_cell_solid_s2(reg, there, 1, my, mz) {
                            all_covered = false;
                            break;
                        }
                    }
                    if !all_covered {
                        break;
                    }
                }
            }
            0 => {
                for mx in 0..2 {
                    for mz in 0..2 {
                        if !micro_cell_solid_s2(reg, there, mx, 0, mz) {
                            all_covered = false;
                            break;
                        }
                    }
                    if !all_covered {
                        break;
                    }
                }
            }
            1 => {
                for mx in 0..2 {
                    for mz in 0..2 {
                        if !micro_cell_solid_s2(reg, there, mx, 1, mz) {
                            all_covered = false;
                            break;
                        }
                    }
                    if !all_covered {
                        break;
                    }
                }
            }
            4 => {
                for mx in 0..2 {
                    for my in 0..2 {
                        if !micro_cell_solid_s2(reg, there, mx, my, 0) {
                            all_covered = false;
                            break;
                        }
                    }
                    if !all_covered {
                        break;
                    }
                }
            }
            5 => {
                for mx in 0..2 {
                    for my in 0..2 {
                        if !micro_cell_solid_s2(reg, there, mx, my, 1) {
                            all_covered = false;
                            break;
                        }
                    }
                    if !all_covered {
                        break;
                    }
                }
            }
            _ => {}
        }
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
    pub xn: Vec<u8>,
    pub xp: Vec<u8>,
    pub zn: Vec<u8>,
    pub zp: Vec<u8>,
    pub yn: Vec<u8>,
    pub yp: Vec<u8>,
    pub sk_xn: Vec<u8>,
    pub sk_xp: Vec<u8>,
    pub sk_zn: Vec<u8>,
    pub sk_zp: Vec<u8>,
    pub sk_yn: Vec<u8>,
    pub sk_yp: Vec<u8>,
    pub bcn_xn: Vec<u8>,
    pub bcn_xp: Vec<u8>,
    pub bcn_zn: Vec<u8>,
    pub bcn_zp: Vec<u8>,
    pub bcn_yn: Vec<u8>,
    pub bcn_yp: Vec<u8>,
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
        let idx3 = |x: usize, y: usize, z: usize| -> usize { (y * sz + z) * sx + x };
        for z in 0..sz {
            for y in 0..sy {
                b.xn[y * sz + z] = grid.block_light[idx3(0, y, z)];
                b.sk_xn[y * sz + z] = grid.skylight[idx3(0, y, z)];
                b.bcn_xn[y * sz + z] = grid.beacon_light[idx3(0, y, z)];
                let d = grid.beacon_dir[idx3(0, y, z)];
                b.bcn_dir_xn[y * sz + z] = if d == 2 || d == 0 { 2 } else { 5 };
            }
        }
        for z in 0..sz {
            for y in 0..sy {
                b.xp[y * sz + z] = grid.block_light[idx3(sx - 1, y, z)];
                b.sk_xp[y * sz + z] = grid.skylight[idx3(sx - 1, y, z)];
                b.bcn_xp[y * sz + z] = grid.beacon_light[idx3(sx - 1, y, z)];
                let d = grid.beacon_dir[idx3(sx - 1, y, z)];
                b.bcn_dir_xp[y * sz + z] = if d == 1 || d == 0 { 1 } else { 5 };
            }
        }
        for x in 0..sx {
            for y in 0..sy {
                b.zn[y * sx + x] = grid.block_light[idx3(x, y, 0)];
                b.sk_zn[y * sx + x] = grid.skylight[idx3(x, y, 0)];
                b.bcn_zn[y * sx + x] = grid.beacon_light[idx3(x, y, 0)];
                let d = grid.beacon_dir[idx3(x, y, 0)];
                b.bcn_dir_zn[y * sx + x] = if d == 4 || d == 0 { 4 } else { 5 };
            }
        }
        for x in 0..sx {
            for y in 0..sy {
                b.zp[y * sx + x] = grid.block_light[idx3(x, y, sz - 1)];
                b.sk_zp[y * sx + x] = grid.skylight[idx3(x, y, sz - 1)];
                b.bcn_zp[y * sx + x] = grid.beacon_light[idx3(x, y, sz - 1)];
                let d = grid.beacon_dir[idx3(x, y, sz - 1)];
                b.bcn_dir_zp[y * sx + x] = if d == 3 || d == 0 { 3 } else { 5 };
            }
        }
        for z in 0..sz {
            for x in 0..sx {
                b.yn[z * sx + x] = grid.block_light[idx3(x, 0, z)];
                b.sk_yn[z * sx + x] = grid.skylight[idx3(x, 0, z)];
                b.bcn_yn[z * sx + x] = grid.beacon_light[idx3(x, 0, z)];
            }
        }
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
    borders: Mutex<HashMap<(i32, i32), LightBorders>>,
    emitters: Mutex<HashMap<(i32, i32), Vec<(usize, usize, usize, u8, bool)>>>,
    micro_borders: Mutex<HashMap<(i32, i32), MicroBorders>>,
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
    pub xn: Option<Vec<u8>>,
    pub xp: Option<Vec<u8>>,
    pub zn: Option<Vec<u8>>,
    pub zp: Option<Vec<u8>>,
    pub sk_xn: Option<Vec<u8>>,
    pub sk_xp: Option<Vec<u8>>,
    pub sk_zn: Option<Vec<u8>>,
    pub sk_zp: Option<Vec<u8>>,
    pub bcn_xn: Option<Vec<u8>>,
    pub bcn_xp: Option<Vec<u8>>,
    pub bcn_zn: Option<Vec<u8>>,
    pub bcn_zp: Option<Vec<u8>>,
    pub bcn_dir_xn: Option<Vec<u8>>,
    pub bcn_dir_xp: Option<Vec<u8>>,
    pub bcn_dir_zn: Option<Vec<u8>>,
    pub bcn_dir_zp: Option<Vec<u8>>,
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

// Entry point that chooses the lighting algorithm based on LightingStore mode.
use geist_world::World;

use crate::micro::MICRO_SKY_ATTENUATION;

pub fn compute_light_with_borders_buf(
    buf: &ChunkBuf,
    store: &LightingStore,
    reg: &BlockRegistry,
    world: &World,
) -> LightGrid {
    micro::compute_light_with_borders_buf_micro(buf, store, reg, world)
}

#[cfg(test)]
mod tests {
    use super::*;
    use geist_blocks::config::{BlockDef, BlocksConfig, ShapeConfig};
    use geist_blocks::material::MaterialCatalog;
    use geist_blocks::types::Block;

    fn make_test_registry() -> BlockRegistry {
        let materials = MaterialCatalog::new();
        let blocks = vec![
            BlockDef {
                name: "air".into(),
                id: Some(0),
                solid: Some(false),
                blocks_skylight: Some(false),
                propagates_light: Some(true),
                emission: Some(0),
                light_profile: None,
                light: None,
                shape: Some(ShapeConfig::Simple("cube".into())),
                materials: None,
                state_schema: None,
                seam: None,
            },
            BlockDef {
                name: "stone".into(),
                id: Some(1),
                solid: Some(true),
                blocks_skylight: Some(true),
                propagates_light: Some(false),
                emission: Some(0),
                light_profile: None,
                light: None,
                shape: Some(ShapeConfig::Simple("cube".into())),
                materials: None,
                state_schema: None,
                seam: None,
            },
            BlockDef {
                name: "fence".into(),
                id: Some(2),
                solid: Some(false),
                blocks_skylight: Some(false),
                propagates_light: Some(true),
                emission: Some(0),
                light_profile: None,
                light: None,
                shape: Some(ShapeConfig::Simple("fence".into())),
                materials: None,
                state_schema: None,
                seam: None,
            },
        ];
        BlockRegistry::from_configs(materials, BlocksConfig { blocks, lighting: None, unknown_block: Some("unknown".into()) }).unwrap()
    }

    fn make_chunk_buf_with(
        reg: &BlockRegistry,
        cx: i32,
        cz: i32,
        sx: usize,
        sy: usize,
        sz: usize,
        fill: &dyn Fn(usize, usize, usize) -> Block,
    ) -> ChunkBuf {
        let mut blocks = Vec::with_capacity(sx * sy * sz);
        for y in 0..sy {
            for z in 0..sz {
                for x in 0..sx {
                    blocks.push(fill(x, y, z));
                }
            }
        }
        ChunkBuf::from_blocks_local(cx, cz, sx, sy, sz, blocks)
    }

    #[test]
    fn occ_bit_indexing() {
        // Each bit should map to (x,y,z) in S=2 micro grid
        for x in 0..2 {
            for y in 0..2 {
                for z in 0..2 {
                    let idx = ((y & 1) << 2) | ((z & 1) << 1) | (x & 1);
                    let mask = 1u8 << idx;
                    assert!(super::occ_bit(mask, x, y, z));
                    // Neighbor bit should be false
                    let other = (idx + 1) & 7;
                    let other_mask = 1u8 << other;
                    let ox = other & 1;
                    let oy = (other >> 2) & 1;
                    let oz = (other >> 1) & 1;
                    assert!(!super::occ_bit(other_mask, x, y, z) || (x == ox && y == oy && z == oz));
                }
            }
        }
    }

    #[test]
    fn skylight_and_block_passable_gates() {
        let reg = make_test_registry();
        let air = Block { id: reg.id_by_name("air").unwrap(), state: 0 };
        let stone = Block { id: reg.id_by_name("stone").unwrap(), state: 0 };
        let fence = Block { id: reg.id_by_name("fence").unwrap(), state: 0 };

        // skylight_transparent: air and fence (blocks_skylight=false) are transparent; stone is not
        assert!(super::skylight_transparent(air, &reg));
        assert!(super::skylight_transparent(fence, &reg));
        assert!(!super::skylight_transparent(stone, &reg));

        // block_light_passable: air and fence propagate; stone does not
        assert!(super::block_light_passable(air, &reg));
        assert!(super::block_light_passable(fence, &reg));
        assert!(!super::block_light_passable(stone, &reg));
    }

    #[test]
    fn lightborders_from_grid_and_equal() {
        // Build a small grid and verify planes extracted correctly; test equal_planes too
        let sx = 3usize; let sy = 2usize; let sz = 2usize;
        let mut lg = LightGrid::new(sx, sy, sz);
        // Fill distinct values
        for y in 0..sy {
            for z in 0..sz {
                for x in 0..sx {
                    let v = (x as u8) + 10 * (y as u8) + 40 * (z as u8);
                    let i = lg.idx(x, y, z);
                    lg.block_light[i] = v;
                    lg.skylight[i] = v.saturating_add(1);
                    lg.beacon_light[i] = v.saturating_add(2);
                    lg.beacon_dir[i] = 0; // neutral -> maps to face-specific dir in borders
                }
            }
        }
        let b = LightBorders::from_grid(&lg);
        // Check -X plane
        for y in 0..sy { for z in 0..sz {
            let ii = y * sz + z;
            assert_eq!(b.xn[ii], lg.block_light[lg.idx(0, y, z)]);
            assert_eq!(b.sk_xn[ii], lg.skylight[lg.idx(0, y, z)]);
            assert_eq!(b.bcn_xn[ii], lg.beacon_light[lg.idx(0, y, z)]);
            // With beacon_dir=0, -X dir plane encodes 2 (PosX) per impl
            assert_eq!(b.bcn_dir_xn[ii], 2);
        }}
        // Check +Z plane
        for x in 0..sx { for y in 0..sy {
            let ii = y * sx + x;
            assert_eq!(b.zp[ii], lg.block_light[lg.idx(x, y, sz - 1)]);
            assert_eq!(b.sk_zp[ii], lg.skylight[lg.idx(x, y, sz - 1)]);
            assert_eq!(b.bcn_zp[ii], lg.beacon_light[lg.idx(x, y, sz - 1)]);
            // With beacon_dir=0, +Z dir plane encodes 3 (NegZ) per impl
            assert_eq!(b.bcn_dir_zp[ii], 3);
        }}

        // equal_planes detects equality and inequality
        let mut b2 = LightBorders::from_grid(&lg);
        assert!(super::equal_planes(&b, &b2));
        b2.xn[0] ^= 1;
        assert!(!super::equal_planes(&b, &b2));
    }

    #[test]
    fn neighbor_light_max_uses_neighbor_planes_on_bounds() {
        let sx = 2; let sy = 1; let sz = 1;
        let mut lg = LightGrid::new(sx, sy, sz);
        // No local light
        lg.block_light.fill(0);
        lg.skylight.fill(0);
        lg.beacon_light.fill(0);
        // Provide +X neighbor planes
        lg.nb_xp_blk = Some(vec![77]); // index y*sz+z = 0
        lg.nb_xp_sky = Some(vec![10]);
        lg.nb_xp_bcn = Some(vec![5]);
        assert_eq!(lg.neighbor_light_max(sx - 1, 0, 0, 2), 77);

        // -X neighbor via xn
        lg.nb_xn_blk = Some(vec![66]);
        lg.nb_xn_sky = Some(vec![3]);
        lg.nb_xn_bcn = Some(vec![9]);
        assert_eq!(lg.neighbor_light_max(0, 0, 0, 3), 66);

        // When neighbor plane is None, falls back to boundary cell value
        lg.nb_zp_blk = None; lg.nb_zp_sky = None; lg.nb_zp_bcn = None;
        let edge_i = lg.idx(0, 0, sz - 1);
        lg.block_light[edge_i] = 65;
        assert_eq!(lg.neighbor_light_max(0, 0, sz - 1, 4), 65);
    }

    #[test]
    fn lightingstore_borders_and_micro_neighbors() {
        let store = LightingStore::new(2, 1, 2);
        // Insert neighbor at (-1,0) so current (0,0) sees xn from its xp
        let mut b = LightBorders::new(2, 1, 2);
        b.xp = vec![11; 1 * 2];
        b.sk_xp = vec![22; 1 * 2];
        b.bcn_xp = vec![33; 1 * 2];
        b.bcn_dir_xp = vec![1; 1 * 2];
        store.update_borders(-1, 0, b.clone());
        let nb = store.get_neighbor_borders(0, 0);
        assert_eq!(nb.xn.as_ref().unwrap(), &b.xp);
        assert_eq!(nb.sk_xn.as_ref().unwrap(), &b.sk_xp);
        assert_eq!(nb.bcn_xn.as_ref().unwrap(), &b.bcn_xp);
        assert_eq!(nb.bcn_dir_xn.as_ref().unwrap(), &b.bcn_dir_xp);

        // Update borders returns false when unchanged
        assert!(!store.update_borders(-1, 0, b.clone()));
        // And true when changed
        let mut b_changed = b.clone();
        b_changed.xp[0] = 99;
        assert!(store.update_borders(-1, 0, b_changed));

        // Micro neighbor mapping
        let mb = MicroBorders {
            xm_sk_neg: vec![1; 2 * 4],
            xm_sk_pos: vec![2; 2 * 4],
            ym_sk_neg: vec![3; 4 * 4],
            ym_sk_pos: vec![4; 4 * 4],
            zm_sk_neg: vec![5; 2 * 4],
            zm_sk_pos: vec![6; 2 * 4],
            xm_bl_neg: vec![7; 2 * 4],
            xm_bl_pos: vec![8; 2 * 4],
            ym_bl_neg: vec![9; 4 * 4],
            ym_bl_pos: vec![10; 4 * 4],
            zm_bl_neg: vec![11; 2 * 4],
            zm_bl_pos: vec![12; 2 * 4],
            xm: 4, ym: 2, zm: 4,
        };
        store.update_micro_borders(-1, 0, mb.clone());
        let nbm = store.get_neighbor_micro_borders(0, 0);
        // -X neighbor provides xm_*_neg/pos to our neg
        assert_eq!(nbm.xm_sk_neg.as_ref().unwrap(), &mb.xm_sk_pos);
        assert_eq!(nbm.xm_bl_neg.as_ref().unwrap(), &mb.xm_bl_pos);
    }

    #[test]
    fn sample_face_local_s2_fallback_respects_neighbor_coverage() {
        let reg = make_test_registry();
        // 2x2x1 chunk: left column air, right column stone
        let air_id = reg.id_by_name("air").unwrap();
        let stone_id = reg.id_by_name("stone").unwrap();
        let buf = make_chunk_buf_with(&reg, 0, 0, 2, 2, 1, &|x, _, _| Block { id: if x == 0 { air_id } else { stone_id }, state: 0 });

        let mut lg = LightGrid::new(2, 2, 1);
        // Set local at (0,0,0) to 10, and its +X neighbor (1,0,0) to 0 initially
        let i000 = lg.idx(0, 0, 0);
        lg.block_light[i000] = 10;
        let i100 = lg.idx(1, 0, 0);
        lg.block_light[i100] = 0;
        // Also set (0,1,0) to 60 to test fallback sampling for open neighbor
        let i010 = lg.idx(0, 1, 0);
        lg.block_light[i010] = 60;

        // From (0,0,0) towards +X where neighbor is stone: fully covered -> return local only
        let v_solid = lg.sample_face_local_s2(&buf, &reg, 0, 0, 0, 2 /* +X into stone */);
        assert_eq!(v_solid, 10);

        // From (1,0,0) towards -X where neighbor is air: fallback samples (0,0,0) and (0,1,0) -> max=60
        let v_open = lg.sample_face_local_s2(&buf, &reg, 1, 0, 0, 3 /* -X into air */);
        assert_eq!(v_open, 60);
    }

    use geist_world::WorldGenMode;

    #[test]
    fn compute_with_borders_buf_seeds_from_coarse_neighbors() {
        let reg = make_test_registry();
        let sx = 2; let sy = 2; let sz = 2;
        let world = geist_world::World::new(1, 1, sx, sy, sz, 42, WorldGenMode::Flat { thickness: 0 });
        let air_id = reg.id_by_name("air").unwrap();
        // All air chunk at (0,0)
        let buf = make_chunk_buf_with(&reg, 0, 0, sx, sy, sz, &|_, _, _| Block { id: air_id, state: 0 });

        // Seed coarse neighbor on -X via neighbor chunk (-1,0)'s +X plane
        let store = LightingStore::new(sx, sy, sz);
        let mut nb = LightBorders::new(sx, sy, sz);
        nb.xp = vec![200; sy * sz];
        store.update_borders(-1, 0, nb);

        let lg = super::compute_light_with_borders_buf(&buf, &store, &reg, &world);
        // Expect V-atten on x=0 edge where V=200 atten=32
        for y in 0..sy { for z in 0..sz {
            assert_eq!(lg.block_light[lg.idx(0, y, z)], 168);
        }}
        // Interior spreads by micro BFS: next macro cell gets one extra micro step attenuation (168-16=152) on micro x=1, and another step to reach macro x=1 (152-16=136)
        for y in 0..sy { for z in 0..sz {
            assert_eq!(lg.block_light[lg.idx(sx-1, y, z)], 136);
        }}

        // Borders from grid reflect edge values
        let b = LightBorders::from_grid(&lg);
        for y in 0..sy { for z in 0..sz {
            assert_eq!(b.xn[y * sz + z], 168);
        }}
    }

    #[test]
    fn compute_with_borders_buf_micro_neighbors_take_precedence() {
        let reg = make_test_registry();
        let sx = 2; let sy = 2; let sz = 2;
        let world = geist_world::World::new(1, 1, sx, sy, sz, 7, WorldGenMode::Flat { thickness: 0 });
        let air_id = reg.id_by_name("air").unwrap();
        let buf = make_chunk_buf_with(&reg, 0, 0, sx, sy, sz, &|_, _, _| Block { id: air_id, state: 0 });

        let store = LightingStore::new(sx, sy, sz);
        // Provide both coarse and micro neighbors on -X; micro should win
        let mut coarse = LightBorders::new(sx, sy, sz);
        coarse.xp = vec![200; sy * sz];
        store.update_borders(-1, 0, coarse);

        // Neighbor micro planes for chunk (-1,0): we need xm_bl_pos to be present (maps to our xm_bl_neg)
        let (mxs, mys, mzs) = (sx * 2, sy * 2, sz * 2);
        let mut mb = MicroBorders {
            xm_sk_neg: vec![0; mys * mzs],
            xm_sk_pos: vec![0; mys * mzs],
            ym_sk_neg: vec![0; mzs * mxs],
            ym_sk_pos: vec![0; mzs * mxs],
            zm_sk_neg: vec![0; mys * mxs],
            zm_sk_pos: vec![0; mys * mxs],
            xm_bl_neg: vec![0; mys * mzs],
            xm_bl_pos: vec![200; mys * mzs],
            ym_bl_neg: vec![0; mzs * mxs],
            ym_bl_pos: vec![0; mzs * mxs],
            zm_bl_neg: vec![0; mys * mxs],
            zm_bl_pos: vec![0; mys * mxs],
            xm: mxs,
            ym: mys,
            zm: mzs,
        };
        // Publish neighbor micro borders for (-1,0)
        store.update_micro_borders(-1, 0, mb.clone());

        let lg = super::compute_light_with_borders_buf(&buf, &store, &reg, &world);
        // With MICRO_BLOCK_ATTENUATION=16, expect 200-16=184 on x=0 edge
        for y in 0..sy { for z in 0..sz {
            assert_eq!(lg.block_light[lg.idx(0, y, z)], 184);
        }}
    }
}

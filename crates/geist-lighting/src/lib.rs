//! In-chunk lighting and neighbor border planes.
#![forbid(unsafe_code)]

use geist_blocks::types::Block;
use geist_blocks::BlockRegistry;
use geist_blocks::micro::{micro_cell_solid_s2, micro_face_cell_open_s2};
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
    pub xm_sk_neg: Vec<u8>, pub xm_sk_pos: Vec<u8>,
    pub ym_sk_neg: Vec<u8>, pub ym_sk_pos: Vec<u8>,
    pub zm_sk_neg: Vec<u8>, pub zm_sk_pos: Vec<u8>,
    pub xm_bl_neg: Vec<u8>, pub xm_bl_pos: Vec<u8>,
    pub ym_bl_neg: Vec<u8>, pub ym_bl_pos: Vec<u8>,
    pub zm_bl_neg: Vec<u8>, pub zm_bl_pos: Vec<u8>,
    pub xm: usize, pub ym: usize, pub zm: usize,
}

pub struct NeighborMicroBorders {
    pub xm_sk_neg: Option<Vec<u8>>, pub xm_sk_pos: Option<Vec<u8>>,
    pub ym_sk_neg: Option<Vec<u8>>, pub ym_sk_pos: Option<Vec<u8>>,
    pub zm_sk_neg: Option<Vec<u8>>, pub zm_sk_pos: Option<Vec<u8>>,
    pub xm_bl_neg: Option<Vec<u8>>, pub xm_bl_pos: Option<Vec<u8>>,
    pub ym_bl_neg: Option<Vec<u8>>, pub ym_bl_pos: Option<Vec<u8>>,
    pub zm_bl_neg: Option<Vec<u8>>, pub zm_bl_pos: Option<Vec<u8>>,
    pub xm: usize, pub ym: usize, pub zm: usize,
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
        .map(|ty| ty.is_solid(b.state) && matches!(ty.shape, geist_blocks::types::Shape::Cube | geist_blocks::types::Shape::AxisCube { .. }))
        .unwrap_or(false)
}

// Decide if a face between (x,y,z) and its neighbor in `face` direction is open for light at S=2.
// face indices: 0=+Y,1=-Y,2=+X,3=-X,4=+Z,5=-Z (matches registry/mesher)
#[inline]
fn can_cross_face_s2(buf: &ChunkBuf, reg: &BlockRegistry, x: usize, y: usize, z: usize, face: usize) -> bool {
    let (nx, ny, nz) = match face {
        0 => (x as i32, y as i32 + 1, z as i32),
        1 => (x as i32, y as i32 - 1, z as i32),
        2 => (x as i32 + 1, y as i32, z as i32),
        3 => (x as i32 - 1, y as i32, z as i32),
        4 => (x as i32, y as i32, z as i32 + 1),
        5 => (x as i32, y as i32, z as i32 - 1),
        _ => return false,
    };
    if nx < 0 || ny < 0 || nz < 0 || nx >= buf.sx as i32 || ny >= buf.sy as i32 || nz >= buf.sz as i32 {
        return false;
    }
    let here = buf.get_local(x, y, z);
    let there = buf.get_local(nx as usize, ny as usize, nz as usize);
    // Cross if any of the four micro face cells is open
    for i0 in 0..2 { for i1 in 0..2 {
        if micro_face_cell_open_s2(reg, here, there, face, i0, i1) { return true; }
    }}
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
    pub(crate) mnb_xn_sky: Option<Vec<u8>>, pub(crate) mnb_xp_sky: Option<Vec<u8>>,
    pub(crate) mnb_xn_blk: Option<Vec<u8>>, pub(crate) mnb_xp_blk: Option<Vec<u8>>,
    // Z faces: size = mys * mxs (index = my * mxs + mx)
    pub(crate) mnb_zn_sky: Option<Vec<u8>>, pub(crate) mnb_zp_sky: Option<Vec<u8>>,
    pub(crate) mnb_zn_blk: Option<Vec<u8>>, pub(crate) mnb_zp_blk: Option<Vec<u8>>,
    // Y faces (usually not chunked vertically): size = mzs * mxs (index = mz * mxs + mx)
    pub(crate) mnb_yn_sky: Option<Vec<u8>>, pub(crate) mnb_yp_sky: Option<Vec<u8>>,
    pub(crate) mnb_yn_blk: Option<Vec<u8>>, pub(crate) mnb_yp_blk: Option<Vec<u8>>,
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
    fn idx(&self, x: usize, y: usize, z: usize) -> usize { (y * self.sz + z) * self.sx + x }

    pub fn new(sx: usize, sy: usize, sz: usize) -> Self {
        Self {
            sx, sy, sz,
            skylight: vec![0; sx * sy * sz],
            block_light: vec![0; sx * sy * sz],
            beacon_light: vec![0; sx * sy * sz],
            beacon_dir: vec![0; sx * sy * sz],
            m_sky: None,
            m_blk: None,
            mxs: sx * 2,
            mys: sy * 2,
            mzs: sz * 2,
            mnb_xn_sky: None, mnb_xp_sky: None, mnb_xn_blk: None, mnb_xp_blk: None,
            mnb_zn_sky: None, mnb_zp_sky: None, mnb_zn_blk: None, mnb_zp_blk: None,
            mnb_yn_sky: None, mnb_yp_sky: None, mnb_yn_blk: None, mnb_yp_blk: None,
            nb_xn_blk: None, nb_xp_blk: None, nb_zn_blk: None, nb_zp_blk: None,
            nb_xn_sky: None, nb_xp_sky: None, nb_zn_sky: None, nb_zp_sky: None,
            nb_xn_bcn: None, nb_xp_bcn: None, nb_zn_bcn: None, nb_zp_bcn: None,
            nb_xn_bcn_dir: None, nb_xp_bcn_dir: None, nb_zn_bcn_dir: None, nb_zp_bcn_dir: None,
        }
    }

    pub fn compute_with_borders_buf(buf: &ChunkBuf, store: &LightingStore, reg: &BlockRegistry) -> Self {
        let sx = buf.sx; let sy = buf.sy; let sz = buf.sz; let mut lg = Self::new(sx, sy, sz);
        use std::collections::VecDeque;
        let mut q_sky = VecDeque::new();
        for z in 0..sz { for x in 0..sx { let mut open_above = true; for y in (0..sy).rev() {
            let b = buf.get_local(x,y,z); let idx = lg.idx(x,y,z);
            if open_above { if skylight_transparent(b, reg) { lg.skylight[idx] = 255; q_sky.push_back((x,y,z,255u8)); } else { open_above = false; lg.skylight[idx] = 0; } } else { lg.skylight[idx] = 0; }
        }}}
        let mut q: VecDeque<(usize, usize, usize, u8, u8)> = VecDeque::new();
        #[allow(clippy::type_complexity)]
        let mut q_beacon: VecDeque<(usize, usize, usize, u8, u8, u8, u8, u8)> = VecDeque::new();
        for z in 0..sz { for y in 0..sy { for x in 0..sx {
            let b = buf.get_local(x,y,z);
            if let Some(ty) = reg.get(b.id) { let em = ty.light_emission(b.state);
                if em > 0 { let idx = lg.idx(x,y,z);
                    if ty.light_is_beam() { lg.beacon_light[idx] = em; lg.beacon_dir[idx] = 0; let (sc,tc,vc,_sd) = ty.beam_params(); q_beacon.push_back((x,y,z,em,0,sc,tc,vc)); }
                    else { lg.block_light[idx] = em; let att = ty.omni_attenuation(); q.push_back((x,y,z,em,att)); }
                }
            }
        }}}
        // Seed from neighbors
        let nb = store.get_neighbor_borders(buf.cx, buf.cz);
        lg.nb_xn_blk = nb.xn.clone(); lg.nb_xp_blk = nb.xp.clone(); lg.nb_zn_blk = nb.zn.clone(); lg.nb_zp_blk = nb.zp.clone();
        lg.nb_xn_sky = nb.sk_xn.clone(); lg.nb_xp_sky = nb.sk_xp.clone(); lg.nb_zn_sky = nb.sk_zn.clone(); lg.nb_zp_sky = nb.sk_zp.clone();
        lg.nb_xn_bcn = nb.bcn_xn.clone(); lg.nb_xp_bcn = nb.bcn_xp.clone(); lg.nb_zn_bcn = nb.bcn_zn.clone(); lg.nb_zp_bcn = nb.bcn_zp.clone();
        lg.nb_xn_bcn_dir = nb.bcn_dir_xn.clone(); lg.nb_xp_bcn_dir = nb.bcn_dir_xp.clone(); lg.nb_zn_bcn_dir = nb.bcn_dir_zn.clone(); lg.nb_zp_bcn_dir = nb.bcn_dir_zp.clone();
        let atten: i32 = 32;
        if let Some(ref plane) = nb.xn { for z in 0..sz { for y in 0..sy { let v = plane[y*sz+z] as i32 - atten; if v>0 { let v8=v as u8; let idx=lg.idx(0,y,z); if lg.block_light[idx] < v8 { lg.block_light[idx]=v8; q.push_back((0,y,z,v8,32)); }}}}}
        if let Some(ref plane) = nb.xp { for z in 0..sz { for y in 0..sy { let v = plane[y*sz+z] as i32 - atten; if v>0 { let v8=v as u8; let xx=sx-1; let idx=lg.idx(xx,y,z); if lg.block_light[idx] < v8 { lg.block_light[idx]=v8; q.push_back((xx,y,z,v8,32)); }}}}}
        if let Some(ref plane) = nb.zn { for x in 0..sx { for y in 0..sy { let v = plane[y*sx+x] as i32 - atten; if v>0 { let v8=v as u8; let idx=lg.idx(x,y,0); if lg.block_light[idx] < v8 { lg.block_light[idx]=v8; q.push_back((x,y,0,v8,32)); }}}}}
        if let Some(ref plane) = nb.zp { for x in 0..sx { for y in 0..sy { let v = plane[y*sx+x] as i32 - atten; if v>0 { let v8=v as u8; let zz=sz-1; let idx=lg.idx(x,y,zz); if lg.block_light[idx] < v8 { lg.block_light[idx]=v8; q.push_back((x,y,zz,v8,32)); }}}}}
        // Beacon from neighbors (respect direction planes)
        if let Some(ref plane) = nb.bcn_xn { for z in 0..sz { for y in 0..sy { let orig_v = plane[y*sz+z]; let dir = lg.nb_xn_bcn_dir.as_ref().and_then(|p| p.get(y*sz+z).cloned()).unwrap_or(5); let atten = if (1..=4).contains(&dir) {1} else {32}; let v = orig_v as i32 - atten; if v>0 { let v8=v as u8; let idx = lg.idx(0,y,z); if lg.beacon_light[idx] < v8 { lg.beacon_light[idx]=v8; lg.beacon_dir[idx]=dir; q_beacon.push_back((0,y,z,v8,dir,1,32,32)); }}}}}
        if let Some(ref plane) = nb.bcn_xp { for z in 0..sz { for y in 0..sy { let orig_v = plane[y*sz+z]; let dir = lg.nb_xp_bcn_dir.as_ref().and_then(|p| p.get(y*sz+z).cloned()).unwrap_or(5); let atten = if (1..=4).contains(&dir) {1} else {32}; let v = orig_v as i32 - atten; if v>0 { let v8=v as u8; let xx=sx-1; let idx=lg.idx(xx,y,z); if lg.beacon_light[idx] < v8 { lg.beacon_light[idx]=v8; lg.beacon_dir[idx]=dir; q_beacon.push_back((xx,y,z,v8,dir,1,32,32)); }}}}}
        if let Some(ref plane) = nb.bcn_zn { for x in 0..sx { for y in 0..sy { let orig_v = plane[y*sx+x]; let dir = lg.nb_zn_bcn_dir.as_ref().and_then(|p| p.get(y*sx+x).cloned()).unwrap_or(5); let atten = if (1..=4).contains(&dir) {1} else {32}; let v = orig_v as i32 - atten; if v>0 { let v8=v as u8; let idx=lg.idx(x,y,0); if lg.beacon_light[idx] < v8 { lg.beacon_light[idx]=v8; lg.beacon_dir[idx]=dir; q_beacon.push_back((x,y,0,v8,dir,1,32,32)); }}}}}
        if let Some(ref plane) = nb.bcn_zp { for x in 0..sx { for y in 0..sy { let orig_v = plane[y*sx+x]; let dir = lg.nb_zp_bcn_dir.as_ref().and_then(|p| p.get(y*sx+x).cloned()).unwrap_or(5); let atten = if (1..=4).contains(&dir) {1} else {32}; let v = orig_v as i32 - atten; if v>0 { let v8=v as u8; let zz=sz-1; let idx=lg.idx(x,y,zz); if lg.beacon_light[idx] < v8 { lg.beacon_light[idx]=v8; lg.beacon_dir[idx]=dir; q_beacon.push_back((x,y,zz,v8,dir,1,32,32)); }}}}}
        // Skylight neighbors
        if let Some(ref plane) = nb.sk_xn { for z in 0..sz { for y in 0..sy { let v = plane[y*sz+z] as i32 - atten; if v>0 { let v8=v as u8; let idx=lg.idx(0,y,z); if lg.skylight[idx] < v8 { lg.skylight[idx]=v8; q_sky.push_back((0,y,z,v8)); }}}}}
        if let Some(ref plane) = nb.sk_xp { for z in 0..sz { for y in 0..sy { let v = plane[y*sz+z] as i32 - atten; if v>0 { let v8=v as u8; let xx=sx-1; let idx=lg.idx(xx,y,z); if lg.skylight[idx] < v8 { lg.skylight[idx]=v8; q_sky.push_back((xx,y,z,v8)); }}}}}
        if let Some(ref plane) = nb.sk_zn { for x in 0..sx { for y in 0..sy { let v = plane[y*sx+x] as i32 - atten; if v>0 { let v8=v as u8; let idx=lg.idx(x,y,0); if lg.skylight[idx] < v8 { lg.skylight[idx]=v8; q_sky.push_back((x,y,0,v8)); }}}}}
        if let Some(ref plane) = nb.sk_zp { for x in 0..sx { for y in 0..sy { let v = plane[y*sx+x] as i32 - atten; if v>0 { let v8=v as u8; let zz=sz-1; let idx=lg.idx(x,y,zz); if lg.skylight[idx] < v8 { lg.skylight[idx]=v8; q_sky.push_back((x,y,zz,v8)); }}}}}
        // Propagate omni block light (face-aware at S=2 for micro occupancy)
        while let Some((x,y,z,level,atten)) = q.pop_front() {
            let level_i = level as i32; if level_i <= 1 { continue; }
            let mut try_push = |nx: i32, ny: i32, nz: i32, face: usize| {
                if nx<0 || ny<0 || nz<0 || nx>=sx as i32 || ny>=sy as i32 || nz>=sz as i32 { return; }
                // Face-aware crossing: allow step only if crossing plane is open at S=2, or
                // fall back to legacy passability when neither side uses micro occupancy.
                let nb = buf.get_local(nx as usize, ny as usize, nz as usize);
                if !block_light_passable(nb, reg) { return; }
                if !can_cross_face_s2(buf, reg, x, y, z, face) { return; }
                let idx = lg.idx(nx as usize, ny as usize, nz as usize);
                let v = level_i - atten as i32;
                if v > 0 { let v8=v as u8; if lg.block_light[idx] < v8 { lg.block_light[idx]=v8; q.push_back((nx as usize, ny as usize, nz as usize, v8, atten)); }}
            };
            try_push(x as i32 + 1, y as i32, z as i32, 2); // +X
            try_push(x as i32 - 1, y as i32, z as i32, 3); // -X
            try_push(x as i32, y as i32 + 1, z as i32, 0); // +Y
            try_push(x as i32, y as i32 - 1, z as i32, 1); // -Y
            try_push(x as i32, y as i32, z as i32 + 1, 4); // +Z
            try_push(x as i32, y as i32, z as i32 - 1, 5); // -Z
        }
        // Propagate beacon light with direction-aware attenuation
        while let Some((x,y,z,level,dir,sc,tc,vc)) = q_beacon.pop_front() {
            let level_i = level as i32; if level_i <= 1 { continue; }
            let mut push_dir = |nx:i32,ny:i32,nz:i32, step_dir:u8| {
                if nx<0 || ny<0 || nz<0 || nx>=sx as i32 || ny>=sy as i32 || nz>=sz as i32 { return; }
                let nb = buf.get_local(nx as usize, ny as usize, nz as usize);
                // Face-aware crossing at S=2. Use same gating as omni.
                let face = match step_dir { 1=>2, 2=>3, 3=>4, 4=>5, _=> if ny>y as i32 {0} else {1} };
                if !block_light_passable(nb, reg) { return; }
                if !can_cross_face_s2(buf, reg, x, y, z, face) { return; }
                let idx = lg.idx(nx as usize, ny as usize, nz as usize);
                // cost: straight vs turn vs vertical
                let cost = if dir == 0 || dir == step_dir { sc as i32 } else if step_dir == 1 || step_dir == 2 || step_dir == 3 || step_dir == 4 { tc as i32 } else { vc as i32 };
                let v = level_i - cost; if v > 0 { let v8=v as u8; if lg.beacon_light[idx] < v8 { lg.beacon_light[idx]=v8; lg.beacon_dir[idx]=step_dir; q_beacon.push_back((nx as usize, ny as usize, nz as usize, v8, step_dir, sc, tc, vc)); }}
            };
            push_dir(x as i32 + 1, y as i32, z as i32, 1); // +X
            push_dir(x as i32 - 1, y as i32, z as i32, 2); // -X
            push_dir(x as i32, y as i32, z as i32 + 1, 3); // +Z
            push_dir(x as i32, y as i32, z as i32 - 1, 4); // -Z
            push_dir(x as i32, y as i32 + 1, z as i32, 5); // vertical/non-cardinal
            push_dir(x as i32, y as i32 - 1, z as i32, 5);
        }
        // Skylight propagation (face-aware at S=2)
        while let Some((x,y,z,level)) = q_sky.pop_front() {
            if level <= 1 { continue; }
            let mut try_push = |nx:i32,ny:i32,nz:i32, face: usize| {
                if nx<0 || ny<0 || nz<0 || nx>=sx as i32 || ny>=sy as i32 || nz>=sz as i32 { return; }
                // Require the crossing plane to be open at S=2, and the target voxel to be skylight transparent
                if !can_cross_face_s2(buf, reg, x, y, z, face) { return; }
                let nb = buf.get_local(nx as usize, ny as usize, nz as usize);
                if !skylight_transparent_s2(nb, reg) { return; }
                let idx = lg.idx(nx as usize, ny as usize, nz as usize);
                let sky_att: i32 = 32;
                let v = (level as i32) - sky_att; if v > 0 { let v8 = v as u8; if lg.skylight[idx] < v8 { lg.skylight[idx] = v8; q_sky.push_back((nx as usize, ny as usize, nz as usize, v8)); }}
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
        let (nx, ny, nz) = match face { 0=> (x as i32, y as i32+1, z as i32), 1=> (x as i32, y as i32-1, z as i32), 2=> (x as i32+1, y as i32, z as i32), 3=> (x as i32-1, y as i32, z as i32), 4=> (x as i32, y as i32, z as i32+1), 5=> (x as i32, y as i32, z as i32-1), _=> return 0 };
        if nx < 0 || ny < 0 || nz < 0 || nx >= self.sx as i32 || ny >= self.sy as i32 || nz >= self.sz as i32 {
            match face { 2 => { let idxp = y * self.sz + z; let sky=self.nb_xp_sky.as_ref().and_then(|p| p.get(idxp).cloned()).unwrap_or(0); let blk=self.nb_xp_blk.as_ref().and_then(|p| p.get(idxp).cloned()).unwrap_or(0); let bcn=self.nb_xp_bcn.as_ref().and_then(|p| p.get(idxp).cloned()).unwrap_or(0); let maxn=sky.max(blk).max(bcn); if maxn>0 { return maxn; } let i=self.idx(self.sx-1,y,z); return self.skylight[i].max(self.block_light[i]).max(self.beacon_light[i]); },
                   3 => { let idxp = y * self.sz + z; let sky=self.nb_xn_sky.as_ref().and_then(|p| p.get(idxp).cloned()).unwrap_or(0); let blk=self.nb_xn_blk.as_ref().and_then(|p| p.get(idxp).cloned()).unwrap_or(0); let bcn=self.nb_xn_bcn.as_ref().and_then(|p| p.get(idxp).cloned()).unwrap_or(0); let maxn=sky.max(blk).max(bcn); if maxn>0 { return maxn; } let i=self.idx(0,y,z); return self.skylight[i].max(self.block_light[i]).max(self.beacon_light[i]); },
                   4 => { let idxp = y * self.sx + x; let sky=self.nb_zp_sky.as_ref().and_then(|p| p.get(idxp).cloned()).unwrap_or(0); let blk=self.nb_zp_blk.as_ref().and_then(|p| p.get(idxp).cloned()).unwrap_or(0); let bcn=self.nb_zp_bcn.as_ref().and_then(|p| p.get(idxp).cloned()).unwrap_or(0); let maxn=sky.max(blk).max(bcn); if maxn>0 { return maxn; } let i=self.idx(x,y,self.sz-1); return self.skylight[i].max(self.block_light[i]).max(self.beacon_light[i]); },
                   5 => { let idxp = y * self.sx + x; let sky=self.nb_zn_sky.as_ref().and_then(|p| p.get(idxp).cloned()).unwrap_or(0); let blk=self.nb_zn_blk.as_ref().and_then(|p| p.get(idxp).cloned()).unwrap_or(0); let bcn=self.nb_zn_bcn.as_ref().and_then(|p| p.get(idxp).cloned()).unwrap_or(0); let maxn=sky.max(blk).max(bcn); if maxn>0 { return maxn; } let i=self.idx(x,y,0); return self.skylight[i].max(self.block_light[i]).max(self.beacon_light[i]); }, _=>{} }
            return 0;
        }
        let i = self.idx(nx as usize, ny as usize, nz as usize); self.skylight[i].max(self.block_light[i]).max(self.beacon_light[i])
    }

    #[inline]
    pub fn sample_face_local(&self, x: usize, y: usize, z: usize, face: usize) -> u8 {
        let i = self.idx(x, y, z);
        let local = self.skylight[i].max(self.block_light[i]).max(self.beacon_light[i]);
        let nb = self.neighbor_light_max(x, y, z, face);
        local.max(nb)
    }

    // Face-aware light sample that respects S=2 micro openings for neighbor contribution
    pub fn sample_face_local_s2(&self, buf: &ChunkBuf, reg: &BlockRegistry, x: usize, y: usize, z: usize, face: usize) -> u8 {
        // If micro-light is available, compute face light by sampling the two
        // micro voxels across each plane micro cell and taking the maximum.
        if let (Some(ms), Some(mb)) = (&self.m_sky, &self.m_blk) {
            let mxs = self.mxs; let mys = self.mys; let mzs = self.mzs;
            let mut max_v: u8 = 0;
            let lval = |mx: usize, my: usize, mz: usize| -> u8 {
                if mx < mxs && my < mys && mz < mzs { let i = (my * mzs + mz) * mxs + mx; ms[i].max(mb[i]) } else { 0 }
            };
            let mut upd = |v: u8| { if v > max_v { max_v = v; } };
            let bx = 2 * x; let by = 2 * y; let bz = 2 * z;
            match face {
                2 => { // +X
                    let mx_here = bx + 1; let mx_nb = bx + 2;
                    for oy in 0..2 { for oz in 0..2 {
                        let my = by + oy; let mz = bz + oz;
                        let a = lval(mx_here, my, mz);
                        let b = if mx_nb < mxs { lval(mx_nb, my, mz) } else {
                            if let Some(ref nbp) = self.mnb_xp_sky {
                                let idx = my * mzs + mz; let sv = *nbp.get(idx).unwrap_or(&0);
                                sv.max(*self.mnb_xp_blk.as_ref().and_then(|p| p.get(idx)).unwrap_or(&0))
                            } else { 0 }
                        };
                        upd(a.max(b));
                    }}
                }
                3 => { // -X
                    let mx_here = bx; let mx_nb = if bx > 0 { bx - 1 } else { mxs }; // sentinel
                    for oy in 0..2 { for oz in 0..2 {
                        let my = by + oy; let mz = bz + oz;
                        let a = lval(mx_here, my, mz);
                        let b = if mx_nb < mxs { lval(mx_nb, my, mz) } else {
                            if let Some(ref nbp) = self.mnb_xn_sky {
                                let idx = my * mzs + mz; let sv = *nbp.get(idx).unwrap_or(&0);
                                sv.max(*self.mnb_xn_blk.as_ref().and_then(|p| p.get(idx)).unwrap_or(&0))
                            } else { 0 }
                        };
                        upd(a.max(b));
                    }}
                }
                4 => { // +Z
                    let mz_here = bz + 1; let mz_nb = bz + 2;
                    for oy in 0..2 { for ox in 0..2 {
                        let my = by + oy; let mx = bx + ox;
                        let a = lval(mx, my, mz_here);
                        let b = if mz_nb < mzs { lval(mx, my, mz_nb) } else {
                            if let Some(ref nbp) = self.mnb_zp_sky {
                                let idx = my * mxs + mx; let sv = *nbp.get(idx).unwrap_or(&0);
                                sv.max(*self.mnb_zp_blk.as_ref().and_then(|p| p.get(idx)).unwrap_or(&0))
                            } else { 0 }
                        };
                        upd(a.max(b));
                    }}
                }
                5 => { // -Z
                    let mz_here = bz; let mz_nb = if bz > 0 { bz - 1 } else { mzs };
                    for oy in 0..2 { for ox in 0..2 {
                        let my = by + oy; let mx = bx + ox;
                        let a = lval(mx, my, mz_here);
                        let b = if mz_nb < mzs { lval(mx, my, mz_nb) } else {
                            if let Some(ref nbp) = self.mnb_zn_sky {
                                let idx = my * mxs + mx; let sv = *nbp.get(idx).unwrap_or(&0);
                                sv.max(*self.mnb_zn_blk.as_ref().and_then(|p| p.get(idx)).unwrap_or(&0))
                            } else { 0 }
                        };
                        upd(a.max(b));
                    }}
                }
                0 => { // +Y
                    let my_here = by + 1; let my_nb = by + 2;
                    for oz in 0..2 { for ox in 0..2 {
                        let mz = bz + oz; let mx = bx + ox;
                        let a = lval(mx, my_here, mz);
                        let b = if my_nb < mys { lval(mx, my_nb, mz) }
                                else { if let Some(ref nbp) = self.mnb_yp_sky { let idx = mz * mxs + mx; let sv = *nbp.get(idx).unwrap_or(&0); sv.max(*self.mnb_yp_blk.as_ref().and_then(|p| p.get(idx)).unwrap_or(&0)) } else { 0 } };
                        upd(a.max(b));
                    }}
                }
                1 => { // -Y
                    let my_here = by; let my_nb = if by > 0 { by - 1 } else { mys };
                    for oz in 0..2 { for ox in 0..2 {
                        let mz = bz + oz; let mx = bx + ox;
                        let a = lval(mx, my_here, mz);
                        let b = if my_nb < mys { lval(mx, my_nb, mz) }
                                else { if let Some(ref nbp) = self.mnb_yn_sky { let idx = mz * mxs + mx; let sv = *nbp.get(idx).unwrap_or(&0); sv.max(*self.mnb_yn_blk.as_ref().and_then(|p| p.get(idx)).unwrap_or(&0)) } else { 0 } };
                        upd(a.max(b));
                    }}
                }
                _ => {}
            }
            // Also consider local beacon light at the macro sample as a safety net (micro beacons unsupported)
            let macro_i = self.idx(x, y, z);
            return max_v.max(self.beacon_light[macro_i]);
        }
        let i = self.idx(x, y, z);
        let local = self.skylight[i].max(self.block_light[i]).max(self.beacon_light[i]);
        // Compute neighbor coords
        let (nx, ny, nz) = match face { 0=> (x as i32, y as i32+1, z as i32), 1=> (x as i32, y as i32-1, z as i32), 2=> (x as i32+1, y as i32, z as i32), 3=> (x as i32-1, y as i32, z as i32), 4=> (x as i32, y as i32, z as i32+1), 5=> (x as i32, y as i32, z as i32-1), _=> return local };
        // Out-of-bounds: fall back to border-aware neighbor max
        if nx < 0 || ny < 0 || nz < 0 || nx >= buf.sx as i32 || ny >= buf.sy as i32 || nz >= buf.sz as i32 {
            let nb = self.neighbor_light_max(x, y, z, face);
            return local.max(nb);
        }
        // Only the neighbor's micro occupancy can seal light reaching the boundary from that side.
        let there = buf.get_local(nx as usize, ny as usize, nz as usize);
        let mut all_covered = true;
        match face {
            2 => { for my in 0..2 { for mz in 0..2 { if !micro_cell_solid_s2(reg, there, 0, my, mz) { all_covered=false; break; } } if !all_covered { break; } } }
            3 => { for my in 0..2 { for mz in 0..2 { if !micro_cell_solid_s2(reg, there, 1, my, mz) { all_covered=false; break; } } if !all_covered { break; } } }
            0 => { for mx in 0..2 { for mz in 0..2 { if !micro_cell_solid_s2(reg, there, mx, 0, mz) { all_covered=false; break; } } if !all_covered { break; } } }
            1 => { for mx in 0..2 { for mz in 0..2 { if !micro_cell_solid_s2(reg, there, mx, 1, mz) { all_covered=false; break; } } if !all_covered { break; } } }
            4 => { for mx in 0..2 { for my in 0..2 { if !micro_cell_solid_s2(reg, there, mx, my, 0) { all_covered=false; break; } } if !all_covered { break; } } }
            5 => { for mx in 0..2 { for my in 0..2 { if !micro_cell_solid_s2(reg, there, mx, my, 1) { all_covered=false; break; } } if !all_covered { break; } } }
            _ => {}
        }
        if all_covered { return local; }
        // Otherwise, approximate the face-neighbor contribution by sampling the best among the micro-adjacent voxels
        let mut nb_max: u8 = 0;
        let mut upd = |sx_i: i32, sy_i: i32, sz_i: i32| {
            if sx_i>=0 && sy_i>=0 && sz_i>=0 && sx_i < buf.sx as i32 && sy_i < buf.sy as i32 && sz_i < buf.sz as i32 {
                let idx = self.idx(sx_i as usize, sy_i as usize, sz_i as usize);
                let v = self.skylight[idx].max(self.block_light[idx]).max(self.beacon_light[idx]);
                if v > nb_max { nb_max = v; }
            }
        };
        match face {
            2 | 3 => { // X faces: sample around (nx,ny,nz) over Y/Z micro offsets
                for my in 0..=1 { for mz in 0..=1 { upd(nx, ny + my, nz + mz); }}
            }
            0 | 1 => { // Y faces: sample around over X/Z
                for mx in 0..=1 { for mz in 0..=1 { upd(nx + mx, ny, nz + mz); }}
            }
            4 | 5 => { // Z faces: sample around over X/Y
                for mx in 0..=1 { for my in 0..=1 { upd(nx + mx, ny + my, nz); }}
            }
            _ => {}
        }
        local.max(nb_max)
    }
}

#[inline]
fn skylight_transparent(b: Block, reg: &BlockRegistry) -> bool {
    if b.id == reg.id_by_name("air").unwrap_or(0) { return true; }
    reg.get(b.id).map(|ty| !ty.blocks_skylight(b.state)).unwrap_or(false)
}

// S=2-aware skylight transparency gate used during BFS propagation.
// It treats micro-occupancy blocks (slab/stairs) as enterable when
// can_cross_face_s2 has already validated the plane is open.
#[inline]
fn skylight_transparent_s2(b: Block, reg: &BlockRegistry) -> bool {
    // Air is transparent
    if b.id == reg.id_by_name("air").unwrap_or(0) { return true; }
    // Full cubes block skylight
    if is_full_cube(reg, b) { return false; }
    // Micro occupancy (e.g., slabs/stairs) should not block BFS
    if occ8_for(reg, b).is_some() { return true; }
    // Fallback to coarse flag for other shapes
    reg.get(b.id).map(|ty| !ty.blocks_skylight(b.state)).unwrap_or(false)
}

#[inline]
fn block_light_passable(b: Block, reg: &BlockRegistry) -> bool {
    if b.id == reg.id_by_name("air").unwrap_or(0) { return true; }
    reg.get(b.id).map(|ty| ty.propagates_light(b.state)).unwrap_or(false)
}

#[derive(Clone)]
pub struct LightBorders {
    pub xn: Vec<u8>, pub xp: Vec<u8>, pub zn: Vec<u8>, pub zp: Vec<u8>, pub yn: Vec<u8>, pub yp: Vec<u8>,
    pub sk_xn: Vec<u8>, pub sk_xp: Vec<u8>, pub sk_zn: Vec<u8>, pub sk_zp: Vec<u8>, pub sk_yn: Vec<u8>, pub sk_yp: Vec<u8>,
    pub bcn_xn: Vec<u8>, pub bcn_xp: Vec<u8>, pub bcn_zn: Vec<u8>, pub bcn_zp: Vec<u8>, pub bcn_yn: Vec<u8>, pub bcn_yp: Vec<u8>,
    pub bcn_dir_xn: Vec<u8>, pub bcn_dir_xp: Vec<u8>, pub bcn_dir_zn: Vec<u8>, pub bcn_dir_zp: Vec<u8>,
}

impl LightBorders {
    pub fn new(sx: usize, sy: usize, sz: usize) -> Self {
        Self { xn: vec![0; sy*sz], xp: vec![0; sy*sz], zn: vec![0; sy*sx], zp: vec![0; sy*sx], yn: vec![0; sx*sz], yp: vec![0; sx*sz],
               sk_xn: vec![0; sy*sz], sk_xp: vec![0; sy*sz], sk_zn: vec![0; sy*sx], sk_zp: vec![0; sy*sx], sk_yn: vec![0; sx*sz], sk_yp: vec![0; sx*sz],
               bcn_xn: vec![0; sy*sz], bcn_xp: vec![0; sy*sz], bcn_zn: vec![0; sy*sx], bcn_zp: vec![0; sy*sx], bcn_yn: vec![0; sx*sz], bcn_yp: vec![0; sx*sz],
               bcn_dir_xn: vec![5; sy*sz], bcn_dir_xp: vec![5; sy*sz], bcn_dir_zn: vec![5; sy*sx], bcn_dir_zp: vec![5; sy*sx] }
    }
    pub fn from_grid(grid: &LightGrid) -> Self {
        let (sx, sy, sz) = (grid.sx, grid.sy, grid.sz); let mut b = Self::new(sx, sy, sz);
        let idx3 = |x:usize,y:usize,z:usize| -> usize { (y*sz+z)*sx + x };
        for z in 0..sz { for y in 0..sy { b.xn[y*sz+z] = grid.block_light[idx3(0,y,z)]; b.sk_xn[y*sz+z] = grid.skylight[idx3(0,y,z)]; b.bcn_xn[y*sz+z] = grid.beacon_light[idx3(0,y,z)]; let d=grid.beacon_dir[idx3(0,y,z)]; b.bcn_dir_xn[y*sz+z] = if d==2 || d==0 {2} else {5}; }}
        for z in 0..sz { for y in 0..sy { b.xp[y*sz+z] = grid.block_light[idx3(sx-1,y,z)]; b.sk_xp[y*sz+z] = grid.skylight[idx3(sx-1,y,z)]; b.bcn_xp[y*sz+z] = grid.beacon_light[idx3(sx-1,y,z)]; let d=grid.beacon_dir[idx3(sx-1,y,z)]; b.bcn_dir_xp[y*sz+z] = if d==1 || d==0 {1} else {5}; }}
        for x in 0..sx { for y in 0..sy { b.zn[y*sx+x] = grid.block_light[idx3(x,y,0)]; b.sk_zn[y*sx+x] = grid.skylight[idx3(x,y,0)]; b.bcn_zn[y*sx+x] = grid.beacon_light[idx3(x,y,0)]; let d=grid.beacon_dir[idx3(x,y,0)]; b.bcn_dir_zn[y*sx+x] = if d==4 || d==0 {4} else {5}; }}
        for x in 0..sx { for y in 0..sy { b.zp[y*sx+x] = grid.block_light[idx3(x,y,sz-1)]; b.sk_zp[y*sx+x] = grid.skylight[idx3(x,y,sz-1)]; b.bcn_zp[y*sx+x] = grid.beacon_light[idx3(x,y,sz-1)]; let d=grid.beacon_dir[idx3(x,y,sz-1)]; b.bcn_dir_zp[y*sx+x] = if d==3 || d==0 {3} else {5}; }}
        for z in 0..sz { for x in 0..sx { b.yn[z*sx+x] = grid.block_light[idx3(x,0,z)]; b.sk_yn[z*sx+x] = grid.skylight[idx3(x,0,z)]; b.bcn_yn[z*sx+x] = grid.beacon_light[idx3(x,0,z)]; }}
        for z in 0..sz { for x in 0..sx { b.yp[z*sx+x] = grid.block_light[idx3(x,sy-1,z)]; b.sk_yp[z*sx+x] = grid.skylight[idx3(x,sy-1,z)]; b.bcn_yp[z*sx+x] = grid.beacon_light[idx3(x,sy-1,z)]; }}
        b
    }
}

pub struct LightingStore {
    sx: usize, sy: usize, sz: usize,
    borders: Mutex<HashMap<(i32, i32), LightBorders>>,
    emitters: Mutex<HashMap<(i32, i32), Vec<(usize, usize, usize, u8, bool)>>>,
    micro_borders: Mutex<HashMap<(i32, i32), MicroBorders>>,
}

impl LightingStore {
    pub fn new(sx: usize, sy: usize, sz: usize) -> Self { Self { sx, sy, sz, borders: Mutex::new(HashMap::new()), emitters: Mutex::new(HashMap::new()), micro_borders: Mutex::new(HashMap::new()) } }
    pub fn clear_chunk(&self, cx: i32, cz: i32) { { let mut m = self.borders.lock().unwrap(); m.remove(&(cx,cz)); } { let mut m = self.emitters.lock().unwrap(); m.remove(&(cx,cz)); } { let mut m = self.micro_borders.lock().unwrap(); m.remove(&(cx,cz)); } }
    pub fn clear_all_borders(&self) { let mut m = self.borders.lock().unwrap(); m.clear(); }
    pub fn get_neighbor_borders(&self, cx: i32, cz: i32) -> NeighborBorders {
        let map = self.borders.lock().unwrap(); let mut nb = NeighborBorders::empty(self.sx, self.sy, self.sz);
        if let Some(b)=map.get(&(cx-1,cz)) { nb.xn=Some(b.xp.clone()); nb.sk_xn=Some(b.sk_xp.clone()); nb.bcn_xn=Some(b.bcn_xp.clone()); nb.bcn_dir_xn=Some(b.bcn_dir_xp.clone()); }
        if let Some(b)=map.get(&(cx+1,cz)) { nb.xp=Some(b.xn.clone()); nb.sk_xp=Some(b.sk_xn.clone()); nb.bcn_xp=Some(b.bcn_xn.clone()); nb.bcn_dir_xp=Some(b.bcn_dir_xn.clone()); }
        if let Some(b)=map.get(&(cx,cz-1)) { nb.zn=Some(b.zp.clone()); nb.sk_zn=Some(b.sk_zp.clone()); nb.bcn_zn=Some(b.bcn_zp.clone()); nb.bcn_dir_zn=Some(b.bcn_dir_zp.clone()); }
        if let Some(b)=map.get(&(cx,cz+1)) { nb.zp=Some(b.zn.clone()); nb.sk_zp=Some(b.sk_zn.clone()); nb.bcn_zp=Some(b.bcn_zn.clone()); nb.bcn_dir_zp=Some(b.bcn_dir_zn.clone()); }
        nb
    }
    pub fn update_borders(&self, cx: i32, cz: i32, lb: LightBorders) -> bool {
        let mut map = self.borders.lock().unwrap(); match map.get_mut(&(cx,cz)) { Some(existing)=>{ let changed = !equal_planes(existing,&lb); if changed { *existing = lb; } changed }, None=>{ map.insert((cx,cz), lb); true } }
    }
    pub fn add_emitter_world(&self, wx: i32, wy: i32, wz: i32, level: u8) { self.add_emitter_world_typed(wx, wy, wz, level, false); }
    pub fn add_beacon_world(&self, wx: i32, wy: i32, wz: i32, level: u8) { self.add_emitter_world_typed(wx, wy, wz, level, true); }
    fn add_emitter_world_typed(&self, wx: i32, wy: i32, wz: i32, level: u8, is_beacon: bool) {
        if wy < 0 || wy >= self.sy as i32 { return; }
        let sx = self.sx as i32; let sz = self.sz as i32; let cx = wx.div_euclid(sx); let cz = wz.div_euclid(sz);
        let lx = wx.rem_euclid(sx) as usize; let lz = wz.rem_euclid(sz) as usize; let ly = wy as usize;
        let mut map = self.emitters.lock().unwrap(); let v = map.entry((cx,cz)).or_default();
        if !v.iter().any(|&(x,y,z,_, _)| x==lx && y==ly && z==lz) { v.push((lx,ly,lz,level,is_beacon)); }
    }
    pub fn remove_emitter_world(&self, wx: i32, wy: i32, wz: i32) {
        if wy < 0 || wy >= self.sy as i32 { return; }
        let sx = self.sx as i32; let sz = self.sz as i32; let cx = wx.div_euclid(sx); let cz = wz.div_euclid(sz);
        let lx = wx.rem_euclid(sx) as usize; let lz = wz.rem_euclid(sz) as usize; let ly = wy as usize;
        let mut map = self.emitters.lock().unwrap(); if let Some(v)=map.get_mut(&(cx,cz)) { v.retain(|&(x,y,z,_,_)| !(x==lx && y==ly && z==lz)); if v.is_empty() { map.remove(&(cx,cz)); } }
    }
    pub fn emitters_for_chunk(&self, cx: i32, cz: i32) -> Vec<(usize, usize, usize, u8, bool)> { let map = self.emitters.lock().unwrap(); map.get(&(cx,cz)).cloned().unwrap_or_default() }
    pub fn update_micro_borders(&self, cx: i32, cz: i32, mb: MicroBorders) { let mut m = self.micro_borders.lock().unwrap(); m.insert((cx,cz), mb); }
    pub fn get_neighbor_micro_borders(&self, cx: i32, cz: i32) -> NeighborMicroBorders {
        let xm = self.sx * 2; let ym = self.sy * 2; let zm = self.sz * 2;
        let map = self.micro_borders.lock().unwrap();
        let mut nb = NeighborMicroBorders {
            xm_sk_neg: None, xm_sk_pos: None, ym_sk_neg: None, ym_sk_pos: None, zm_sk_neg: None, zm_sk_pos: None,
            xm_bl_neg: None, xm_bl_pos: None, ym_bl_neg: None, ym_bl_pos: None, zm_bl_neg: None, zm_bl_pos: None,
            xm, ym, zm,
        };
        if let Some(m)=map.get(&(cx-1,cz)) { nb.xm_sk_neg=Some(m.xm_sk_pos.clone()); nb.xm_bl_neg=Some(m.xm_bl_pos.clone()); }
        if let Some(m)=map.get(&(cx+1,cz)) { nb.xm_sk_pos=Some(m.xm_sk_neg.clone()); nb.xm_bl_pos=Some(m.xm_bl_neg.clone()); }
        if let Some(m)=map.get(&(cx,cz-1)) { nb.zm_sk_neg=Some(m.zm_sk_pos.clone()); nb.zm_bl_neg=Some(m.zm_bl_pos.clone()); }
        if let Some(m)=map.get(&(cx,cz+1)) { nb.zm_sk_pos=Some(m.zm_sk_neg.clone()); nb.zm_bl_pos=Some(m.zm_bl_neg.clone()); }
        // Vertical neighbors are not chunked here; keep None. If vertically chunked, add mapping like above.
        nb
    }
}

fn equal_planes(a: &LightBorders, b: &LightBorders) -> bool {
    a.xn==b.xn && a.xp==b.xp && a.zn==b.zn && a.zp==b.zp && a.yn==b.yn && a.yp==b.yp && a.sk_xn==b.sk_xn && a.sk_xp==b.sk_xp && a.sk_zn==b.sk_zn && a.sk_zp==b.sk_zp && a.sk_yn==b.sk_yn && a.sk_yp==b.sk_yp && a.bcn_xn==b.bcn_xn && a.bcn_xp==b.bcn_xp && a.bcn_zn==b.bcn_zn && a.bcn_zp==b.bcn_zp && a.bcn_yn==b.bcn_yn && a.bcn_yp==b.bcn_yp && a.bcn_dir_xn==b.bcn_dir_xn && a.bcn_dir_xp==b.bcn_dir_xp && a.bcn_dir_zn==b.bcn_dir_zn && a.bcn_dir_zp==b.bcn_dir_zp
}

pub struct NeighborBorders {
    pub xn: Option<Vec<u8>>, pub xp: Option<Vec<u8>>, pub zn: Option<Vec<u8>>, pub zp: Option<Vec<u8>>,
    pub sk_xn: Option<Vec<u8>>, pub sk_xp: Option<Vec<u8>>, pub sk_zn: Option<Vec<u8>>, pub sk_zp: Option<Vec<u8>>,
    pub bcn_xn: Option<Vec<u8>>, pub bcn_xp: Option<Vec<u8>>, pub bcn_zn: Option<Vec<u8>>, pub bcn_zp: Option<Vec<u8>>,
    pub bcn_dir_xn: Option<Vec<u8>>, pub bcn_dir_xp: Option<Vec<u8>>, pub bcn_dir_zn: Option<Vec<u8>>, pub bcn_dir_zp: Option<Vec<u8>>,
}

impl NeighborBorders { pub fn empty(_sx: usize,_sy: usize,_sz: usize) -> Self { Self { xn:None,xp:None,zn:None,zp:None, sk_xn:None,sk_xp:None,sk_zn:None,sk_zp:None, bcn_xn:None,bcn_xp:None,bcn_zn:None,bcn_zp:None, bcn_dir_xn:None,bcn_dir_xp:None,bcn_dir_zn:None,bcn_dir_zp:None } } }

// Entry point that chooses the lighting algorithm based on LightingStore mode.
use geist_world::World;

pub fn compute_light_with_borders_buf(buf: &ChunkBuf, store: &LightingStore, reg: &BlockRegistry, world: &World) -> LightGrid {
    micro::compute_light_with_borders_buf_micro(buf, store, reg, world)
}

use crate::{LightGrid, LightingStore, MicroBorders};
use geist_blocks::{types::Block, BlockRegistry};
use geist_chunk::ChunkBuf;

// Micro-voxel scale factor (2x resolution in each dimension)
const MICRO_SCALE: usize = 2;

// Light attenuation values
const MICRO_BLOCK_ATTENUATION: u8 = 16;  // Per-micro-step block light attenuation
const MICRO_SKY_ATTENUATION: u8 = 16;    // Per-micro-step skylight attenuation
const COARSE_SEAM_ATTENUATION: u8 = 32;  // Attenuation when falling back to coarse neighbors

// Maximum light values
const MAX_LIGHT: u8 = 255;

#[inline]
fn micro_dims(buf: &ChunkBuf) -> (usize, usize, usize) {
    (buf.sx * MICRO_SCALE, buf.sy * MICRO_SCALE, buf.sz * MICRO_SCALE)
}

#[inline]
fn midx(mx: usize, my: usize, mz: usize, mxs: usize, mzs: usize) -> usize {
    (my * mzs + mz) * mxs + mx
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

#[inline]
fn micro_solid_at(buf: &ChunkBuf, reg: &BlockRegistry, mx: usize, my: usize, mz: usize) -> bool {
    let x = mx >> 1; let y = my >> 1; let z = mz >> 1;
    if x >= buf.sx || y >= buf.sy || z >= buf.sz { return true; }
    let b = buf.get_local(x, y, z);
    if is_full_cube(reg, b) { return true; }
    if let Some(occ) = occ8_for(reg, b) {
        let lx = mx & 1; let ly = my & 1; let lz = mz & 1;
        let idx = ((ly & 1) << 2) | ((lz & 1) << 1) | (lx & 1);
        return (occ & (1u8 << idx)) != 0;
    }
    false
}

#[inline]
fn clamp_sub_u8(v: u8, d: u8) -> u8 { v.saturating_sub(d) }

pub fn compute_light_with_borders_buf_micro(buf: &ChunkBuf, store: &LightingStore, reg: &BlockRegistry) -> LightGrid {
    let (mxs, mys, mzs) = micro_dims(buf);
    let mut micro_sky = vec![0u8; mxs * mys * mzs];
    let mut micro_blk = vec![0u8; mxs * mys * mzs];

    // Seed skylight from open-above micro columns (world-local within chunk)
    for mz in 0..mzs { for mx in 0..mxs { let mut open_above = true; for my in (0..mys).rev() {
        if open_above {
            if !micro_solid_at(buf, reg, mx, my, mz) {
                let i = midx(mx, my, mz, mxs, mzs);
                micro_sky[i] = MAX_LIGHT;
            } else {
                open_above = false;
            }
        }
    }}}

    // Seed from neighbor micro border planes when available; fall back to coarse upsample
    let nbm = store.get_neighbor_micro_borders(buf.cx, buf.cz);
    let nb = store.get_neighbor_borders(buf.cx, buf.cz);
    let atten: u8 = COARSE_SEAM_ATTENUATION;
    // Block light neighbors (-X and +X)
    if let Some(ref plane) = nbm.xm_bl_neg { for my in 0..mys { for mz in 0..mzs {
        let v = clamp_sub_u8(plane[my*mzs + mz], MICRO_BLOCK_ATTENUATION);
        if v > 0 { let i = midx(0, my, mz, mxs, mzs); if !micro_solid_at(buf, reg, 0, my, mz) && micro_blk[i] < v { micro_blk[i] = v; } }
    }}}
    else if let Some(ref plane) = nb.xn { for z in 0..buf.sz { for y in 0..buf.sy {
        let v = clamp_sub_u8(plane[y*buf.sz+z], atten);
        if v > 0 { for mz in (z*MICRO_SCALE)..(z*MICRO_SCALE+MICRO_SCALE) { for my in (y*MICRO_SCALE)..(y*MICRO_SCALE+MICRO_SCALE) {
            let i = midx(0, my, mz, mxs, mzs); if !micro_solid_at(buf, reg, 0, my, mz) && micro_blk[i] < v { micro_blk[i] = v; }
        }}}
    }}}
    if let Some(ref plane) = nbm.xm_bl_pos { for my in 0..mys { for mz in 0..mzs {
        let v = clamp_sub_u8(plane[my*mzs + mz], MICRO_BLOCK_ATTENUATION);
        if v > 0 { let i = midx(mxs-1, my, mz, mxs, mzs); if !micro_solid_at(buf, reg, mxs-1, my, mz) && micro_blk[i] < v { micro_blk[i] = v; } }
    }}}
    else if let Some(ref plane) = nb.xp { for z in 0..buf.sz { for y in 0..buf.sy {
        let v = clamp_sub_u8(plane[y*buf.sz+z], atten);
        if v > 0 { for mz in (z*MICRO_SCALE)..(z*MICRO_SCALE+MICRO_SCALE) { for my in (y*MICRO_SCALE)..(y*MICRO_SCALE+MICRO_SCALE) {
            let i = midx(mxs-1, my, mz, mxs, mzs); if !micro_solid_at(buf, reg, mxs-1, my, mz) && micro_blk[i] < v { micro_blk[i] = v; }
        }}}
    }}}
    if let Some(ref plane) = nbm.zm_bl_neg { for my in 0..mys { for mx in 0..mxs {
        let v = clamp_sub_u8(plane[my*mxs + mx], MICRO_BLOCK_ATTENUATION);
        if v > 0 { let i = midx(mx, my, 0, mxs, mzs); if !micro_solid_at(buf, reg, mx, my, 0) && micro_blk[i] < v { micro_blk[i] = v; } }
    }}}
    else if let Some(ref plane) = nb.zn { for x in 0..buf.sx { for y in 0..buf.sy {
        let v = clamp_sub_u8(plane[y*buf.sx+x], atten);
        if v > 0 { for mx in (x*MICRO_SCALE)..(x*MICRO_SCALE+MICRO_SCALE) { for my in (y*MICRO_SCALE)..(y*MICRO_SCALE+MICRO_SCALE) {
            let i = midx(mx, my, 0, mxs, mzs); if !micro_solid_at(buf, reg, mx, my, 0) && micro_blk[i] < v { micro_blk[i] = v; }
        }}}
    }}}
    if let Some(ref plane) = nbm.zm_bl_pos { for my in 0..mys { for mx in 0..mxs {
        let v = clamp_sub_u8(plane[my*mxs + mx], MICRO_BLOCK_ATTENUATION);
        if v > 0 { let i = midx(mx, my, mzs-1, mxs, mzs); if !micro_solid_at(buf, reg, mx, my, mzs-1) && micro_blk[i] < v { micro_blk[i] = v; } }
    }}}
    else if let Some(ref plane) = nb.zp { for x in 0..buf.sx { for y in 0..buf.sy {
        let v = clamp_sub_u8(plane[y*buf.sx+x], atten);
        if v > 0 { for mx in (x*MICRO_SCALE)..(x*MICRO_SCALE+MICRO_SCALE) { for my in (y*MICRO_SCALE)..(y*MICRO_SCALE+MICRO_SCALE) {
            let i = midx(mx, my, mzs-1, mxs, mzs); if !micro_solid_at(buf, reg, mx, my, mzs-1) && micro_blk[i] < v { micro_blk[i] = v; }
        }}}
    }}}
    // Skylight neighbors
    if let Some(ref plane) = nbm.xm_sk_neg { for my in 0..mys { for mz in 0..mzs {
        let v = clamp_sub_u8(plane[my*mzs + mz], MICRO_SKY_ATTENUATION);
        if v > 0 { let i = midx(0, my, mz, mxs, mzs); if !micro_solid_at(buf, reg, 0, my, mz) && micro_sky[i] < v { micro_sky[i] = v; } }
    }}}
    else if let Some(ref plane) = nb.sk_xn { for z in 0..buf.sz { for y in 0..buf.sy {
        let v = clamp_sub_u8(plane[y*buf.sz+z], atten);
        if v > 0 { for mz in (z*MICRO_SCALE)..(z*MICRO_SCALE+MICRO_SCALE) { for my in (y*MICRO_SCALE)..(y*MICRO_SCALE+MICRO_SCALE) {
            let i = midx(0, my, mz, mxs, mzs); if !micro_solid_at(buf, reg, 0, my, mz) && micro_sky[i] < v { micro_sky[i] = v; }
        }}}
    }}}
    if let Some(ref plane) = nbm.xm_sk_pos { for my in 0..mys { for mz in 0..mzs {
        let v = clamp_sub_u8(plane[my*mzs + mz], MICRO_SKY_ATTENUATION);
        if v > 0 { let i = midx(mxs-1, my, mz, mxs, mzs); if !micro_solid_at(buf, reg, mxs-1, my, mz) && micro_sky[i] < v { micro_sky[i] = v; } }
    }}}
    else if let Some(ref plane) = nb.sk_xp { for z in 0..buf.sz { for y in 0..buf.sy {
        let v = clamp_sub_u8(plane[y*buf.sz+z], atten);
        if v > 0 { for mz in (z*MICRO_SCALE)..(z*MICRO_SCALE+MICRO_SCALE) { for my in (y*MICRO_SCALE)..(y*MICRO_SCALE+MICRO_SCALE) {
            let i = midx(mxs-1, my, mz, mxs, mzs); if !micro_solid_at(buf, reg, mxs-1, my, mz) && micro_sky[i] < v { micro_sky[i] = v; }
        }}}
    }}}
    // (removed duplicate X skylight seeding)
    if let Some(ref plane) = nbm.zm_sk_neg { for my in 0..mys { for mx in 0..mxs {
        let v = clamp_sub_u8(plane[my*mxs + mx], MICRO_SKY_ATTENUATION);
        if v > 0 { let i = midx(mx, my, 0, mxs, mzs); if !micro_solid_at(buf, reg, mx, my, 0) && micro_sky[i] < v { micro_sky[i] = v; } }
    }}}
    else if let Some(ref plane) = nb.sk_zn { for x in 0..buf.sx { for y in 0..buf.sy {
        let v = clamp_sub_u8(plane[y*buf.sx+x], atten);
        if v > 0 { for mx in (x*MICRO_SCALE)..(x*MICRO_SCALE+MICRO_SCALE) { for my in (y*MICRO_SCALE)..(y*MICRO_SCALE+MICRO_SCALE) {
            let i = midx(mx, my, 0, mxs, mzs); if !micro_solid_at(buf, reg, mx, my, 0) && micro_sky[i] < v { micro_sky[i] = v; }
        }}}
    }}}
    if let Some(ref plane) = nbm.zm_sk_pos { for my in 0..mys { for mx in 0..mxs {
        let v = clamp_sub_u8(plane[my*mxs + mx], MICRO_SKY_ATTENUATION);
        if v > 0 { let i = midx(mx, my, mzs-1, mxs, mzs); if !micro_solid_at(buf, reg, mx, my, mzs-1) && micro_sky[i] < v { micro_sky[i] = v; } }
    }}}
    else if let Some(ref plane) = nb.sk_zp { for x in 0..buf.sx { for y in 0..buf.sy {
        let v = clamp_sub_u8(plane[y*buf.sx+x], atten);
        if v > 0 { for mx in (x*MICRO_SCALE)..(x*MICRO_SCALE+MICRO_SCALE) { for my in (y*MICRO_SCALE)..(y*MICRO_SCALE+MICRO_SCALE) {
            let i = midx(mx, my, mzs-1, mxs, mzs); if !micro_solid_at(buf, reg, mx, my, mzs-1) && micro_sky[i] < v { micro_sky[i] = v; }
        }}}
    }}}

    // Seed emissive blocks at micro resolution (fill interior air micro voxels of the macro cell)
    for (lx, ly, lz, level, is_beacon) in store.emitters_for_chunk(buf.cx, buf.cz) {
        if is_beacon { continue; } // beacons not supported in Micro S=2 path yet
        let mx0 = lx * MICRO_SCALE; let my0 = ly * MICRO_SCALE; let mz0 = lz * MICRO_SCALE;
        for mx in mx0..(mx0+MICRO_SCALE) { for my in my0..(my0+MICRO_SCALE) { for mz in mz0..(mz0+MICRO_SCALE) {
            if !micro_solid_at(buf, reg, mx, my, mz) {
                let i = midx(mx, my, mz, mxs, mzs);
                if micro_blk[i] < level { micro_blk[i] = level; }
            }
        }}}
    }

    // Propagate block light (omni) and skylight with per-micro step attenuation
    use std::collections::VecDeque;
    let mut q_blk: VecDeque<(usize, usize, usize, u8)> = VecDeque::new();
    let mut q_sky: VecDeque<(usize, usize, usize, u8)> = VecDeque::new();
    for mz in 0..mzs { for my in 0..mys { for mx in 0..mxs {
        let i = midx(mx,my,mz,mxs,mzs);
        if micro_blk[i] > 0 { q_blk.push_back((mx,my,mz,micro_blk[i])); }
        if micro_sky[i] > 0 { q_sky.push_back((mx,my,mz,micro_sky[i])); }
    }}}

    // Use per-micro step attenuation constants
    let att_blk: u8 = MICRO_BLOCK_ATTENUATION;
    let att_sky: u8 = MICRO_SKY_ATTENUATION;
    let mut push = |mx: i32, my: i32, mz: i32, mxs: usize, mys: usize, mzs: usize, arr: &mut [u8], lvl: u8, att: u8| {
        if mx < 0 || my < 0 || mz < 0 { return; }
        let (mxu, myu, mzu) = (mx as usize, my as usize, mz as usize);
        if mxu >= mxs || myu >= mys || mzu >= mzs { return; }
        if micro_solid_at(buf, reg, mxu, myu, mzu) { return; }
        let v = clamp_sub_u8(lvl, att);
        if v == 0 { return; }
        let i = midx(mxu, myu, mzu, mxs, mzs);
        if arr[i] < v { arr[i] = v; }
    };

    while let Some((mx,my,mz,level)) = q_blk.pop_front() {
        if level <= 1 { continue; }
        let lvl = level;
        let (mx_i, my_i, mz_i) = (mx as i32, my as i32, mz as i32);
        let before = micro_blk[midx(mx, my, mz, mxs, mzs)];
        if before != lvl { continue; }
        push(mx_i+1, my_i, mz_i, mxs, mys, mzs, &mut micro_blk, lvl, att_blk);
        push(mx_i-1, my_i, mz_i, mxs, mys, mzs, &mut micro_blk, lvl, att_blk);
        push(mx_i, my_i+1, mz_i, mxs, mys, mzs, &mut micro_blk, lvl, att_blk);
        push(mx_i, my_i-1, mz_i, mxs, mys, mzs, &mut micro_blk, lvl, att_blk);
        push(mx_i, my_i, mz_i+1, mxs, mys, mzs, &mut micro_blk, lvl, att_blk);
        push(mx_i, my_i, mz_i-1, mxs, mys, mzs, &mut micro_blk, lvl, att_blk);
        // enqueue neighbors that we updated
        let neigh = [
            (mx_i+1,my_i,mz_i), (mx_i-1,my_i,mz_i), (mx_i,my_i+1,mz_i), (mx_i,my_i-1,mz_i), (mx_i,my_i,mz_i+1), (mx_i,my_i,mz_i-1)
        ];
        for &(nx,ny,nz) in &neigh {
            if nx>=0 && ny>=0 && nz>=0 { let (nxu,nyu,nzu)=(nx as usize,ny as usize,nz as usize); if nxu<mxs && nyu<mys && nzu<mzs {
                let ii = midx(nxu,nyu,nzu,mxs,mzs);
                if micro_blk[ii] + att_blk == lvl { q_blk.push_back((nxu,nyu,nzu,micro_blk[ii])); }
            }}
        }
    }

    while let Some((mx,my,mz,level)) = q_sky.pop_front() {
        if level <= 1 { continue; }
        let lvl = level;
        let (mx_i, my_i, mz_i) = (mx as i32, my as i32, mz as i32);
        let before = micro_sky[midx(mx, my, mz, mxs, mzs)];
        if before != lvl { continue; }
        push(mx_i+1, my_i, mz_i, mxs, mys, mzs, &mut micro_sky, lvl, att_sky);
        push(mx_i-1, my_i, mz_i, mxs, mys, mzs, &mut micro_sky, lvl, att_sky);
        push(mx_i, my_i+1, mz_i, mxs, mys, mzs, &mut micro_sky, lvl, att_sky);
        push(mx_i, my_i-1, mz_i, mxs, mys, mzs, &mut micro_sky, lvl, att_sky);
        push(mx_i, my_i, mz_i+1, mxs, mys, mzs, &mut micro_sky, lvl, att_sky);
        push(mx_i, my_i, mz_i-1, mxs, mys, mzs, &mut micro_sky, lvl, att_sky);
        // enqueue neighbors that we updated
        let neigh = [
            (mx_i+1,my_i,mz_i), (mx_i-1,my_i,mz_i), (mx_i,my_i+1,mz_i), (mx_i,my_i-1,mz_i), (mx_i,my_i,mz_i+1), (mx_i,my_i,mz_i-1)
        ];
        for &(nx,ny,nz) in &neigh {
            if nx>=0 && ny>=0 && nz>=0 { let (nxu,nyu,nzu)=(nx as usize,ny as usize,nz as usize); if nxu<mxs && nyu<mys && nzu<mzs {
                let ii = midx(nxu,nyu,nzu,mxs,mzs);
                if micro_sky[ii] + att_sky == lvl { q_sky.push_back((nxu,nyu,nzu,micro_sky[ii])); }
            }}
        }
    }

    // Downsample micro -> macro (max over the MICRO_SCALE^3 block) and retain micro arrays + neighbor planes
    let mut lg = LightGrid::new(buf.sx, buf.sy, buf.sz);
    for z in 0..buf.sz { for y in 0..buf.sy { for x in 0..buf.sx {
        let mut smax = 0u8; let mut bmax = 0u8;
        for dz in 0..MICRO_SCALE { for dy in 0..MICRO_SCALE { for dx in 0..MICRO_SCALE {
            let i = midx(x*MICRO_SCALE+dx, y*MICRO_SCALE+dy, z*MICRO_SCALE+dz, mxs, mzs);
            smax = smax.max(micro_sky[i]);
            bmax = bmax.max(micro_blk[i]);
        }}}
        let ii = ((y * buf.sz) + z) * buf.sx + x;
        lg.skylight[ii] = smax;
        lg.block_light[ii] = bmax;
    }}}

    // Compute and publish micro border planes for this chunk (we own -X/-Y/-Z planes for stitching)
    let mut xm_sk_neg = vec![0u8; mys*mzs]; let mut xm_bl_neg = vec![0u8; mys*mzs];
    let mut xm_sk_pos = vec![0u8; mys*mzs]; let mut xm_bl_pos = vec![0u8; mys*mzs];
    let mut zm_sk_neg = vec![0u8; mys*mxs]; let mut zm_bl_neg = vec![0u8; mys*mxs];
    let mut zm_sk_pos = vec![0u8; mys*mxs]; let mut zm_bl_pos = vec![0u8; mys*mxs];
    let mut ym_sk_neg = vec![0u8; mzs*mxs]; let mut ym_bl_neg = vec![0u8; mzs*mxs];
    let mut ym_sk_pos = vec![0u8; mzs*mxs]; let mut ym_bl_pos = vec![0u8; mzs*mxs];
    // X planes
    for my in 0..mys { for mz in 0..mzs { let idx=my*mzs+mz; xm_sk_neg[idx]=micro_sky[midx(0,my,mz,mxs,mzs)]; xm_bl_neg[idx]=micro_blk[midx(0,my,mz,mxs,mzs)]; xm_sk_pos[idx]=micro_sky[midx(mxs-1,my,mz,mxs,mzs)]; xm_bl_pos[idx]=micro_blk[midx(mxs-1,my,mz,mxs,mzs)]; }}
    // Z planes
    for my in 0..mys { for mx in 0..mxs { let idx=my*mxs+mx; zm_sk_neg[idx]=micro_sky[midx(mx,my,0,mxs,mzs)]; zm_bl_neg[idx]=micro_blk[midx(mx,my,0,mxs,mzs)]; zm_sk_pos[idx]=micro_sky[midx(mx,my,mzs-1,mxs,mzs)]; zm_bl_pos[idx]=micro_blk[midx(mx,my,mzs-1,mxs,mzs)]; }}
    // Y planes
    for mz in 0..mzs { for mx in 0..mxs { let idx=mz*mxs+mx; ym_sk_neg[idx]=micro_sky[midx(mx,0,mz,mxs,mzs)]; ym_bl_neg[idx]=micro_blk[midx(mx,0,mz,mxs,mzs)]; ym_sk_pos[idx]=micro_sky[midx(mx,mys-1,mz,mxs,mzs)]; ym_bl_pos[idx]=micro_blk[midx(mx,mys-1,mz,mxs,mzs)]; }}
    store.update_micro_borders(buf.cx, buf.cz, MicroBorders { xm_sk_neg, xm_sk_pos, ym_sk_neg, ym_sk_pos, zm_sk_neg, zm_sk_pos, xm_bl_neg, xm_bl_pos, ym_bl_neg, ym_bl_pos, zm_bl_neg, zm_bl_pos, xm: mxs, ym: mys, zm: mzs });
    // Attach micro arrays and neighbor planes to LightGrid for micro face sampling
    lg.m_sky = Some(micro_sky);
    lg.m_blk = Some(micro_blk);
    // Add neighbor micro planes for sampling across seams
    lg.mnb_xn_sky = nbm.xm_sk_neg; lg.mnb_xp_sky = nbm.xm_sk_pos;
    lg.mnb_zn_sky = nbm.zm_sk_neg; lg.mnb_zp_sky = nbm.zm_sk_pos;
    lg.mnb_yn_sky = nbm.ym_sk_neg; lg.mnb_yp_sky = nbm.ym_sk_pos;
    lg.mnb_xn_blk = nbm.xm_bl_neg; lg.mnb_xp_blk = nbm.xm_bl_pos;
    lg.mnb_zn_blk = nbm.zm_bl_neg; lg.mnb_zp_blk = nbm.zm_bl_pos;
    lg.mnb_yn_blk = nbm.ym_bl_neg; lg.mnb_yp_blk = nbm.ym_bl_pos;
    // Coarse planes are still derived by LightBorders::from_grid upstream.
    lg
}

// Scaffold for S=2 micro-voxel lighting engine.
// For now, this delegates to the legacy voxel light grid to keep behavior unchanged
// while wiring up mode toggling and rebuild plumbing. The full implementation will
// allocate a micro grid, run bucketed BFS at S=2, and produce border planes.

// (old scaffold removed; Micro S=2 implementation is above)

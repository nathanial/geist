use crate::{LightGrid, LightingStore};
use geist_blocks::{types::Block, BlockRegistry};
use geist_chunk::ChunkBuf;

#[inline]
fn micro_dims(buf: &ChunkBuf) -> (usize, usize, usize) {
    (buf.sx * 2, buf.sy * 2, buf.sz * 2)
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
                micro_sky[i] = 255;
            } else {
                open_above = false;
            }
        }
    }}}

    // Seed from neighbor coarse border planes (upsampled 2x per axis)
    let nb = store.get_neighbor_borders(buf.cx, buf.cz);
    let atten: u8 = 32; // coarse seam attenuation baseline
    // Block light neighbors
    if let Some(ref plane) = nb.xn { for z in 0..buf.sz { for y in 0..buf.sy {
        let v = clamp_sub_u8(plane[y*buf.sz+z], atten);
        if v > 0 { for mz in (z*2)..(z*2+2) { for my in (y*2)..(y*2+2) {
            let i = midx(0, my, mz, mxs, mzs); if micro_blk[i] < v { micro_blk[i] = v; }
        }}}
    }}}
    if let Some(ref plane) = nb.xp { for z in 0..buf.sz { for y in 0..buf.sy {
        let v = clamp_sub_u8(plane[y*buf.sz+z], atten);
        if v > 0 { for mz in (z*2)..(z*2+2) { for my in (y*2)..(y*2+2) {
            let i = midx(mxs-1, my, mz, mxs, mzs); if micro_blk[i] < v { micro_blk[i] = v; }
        }}}
    }}}
    if let Some(ref plane) = nb.zn { for x in 0..buf.sx { for y in 0..buf.sy {
        let v = clamp_sub_u8(plane[y*buf.sx+x], atten);
        if v > 0 { for mx in (x*2)..(x*2+2) { for my in (y*2)..(y*2+2) {
            let i = midx(mx, my, 0, mxs, mzs); if micro_blk[i] < v { micro_blk[i] = v; }
        }}}
    }}}
    if let Some(ref plane) = nb.zp { for x in 0..buf.sx { for y in 0..buf.sy {
        let v = clamp_sub_u8(plane[y*buf.sx+x], atten);
        if v > 0 { for mx in (x*2)..(x*2+2) { for my in (y*2)..(y*2+2) {
            let i = midx(mx, my, mzs-1, mxs, mzs); if micro_blk[i] < v { micro_blk[i] = v; }
        }}}
    }}}
    // Skylight neighbors
    if let Some(ref plane) = nb.sk_xn { for z in 0..buf.sz { for y in 0..buf.sy {
        let v = clamp_sub_u8(plane[y*buf.sz+z], atten);
        if v > 0 { for mz in (z*2)..(z*2+2) { for my in (y*2)..(y*2+2) {
            let i = midx(0, my, mz, mxs, mzs); if micro_sky[i] < v { micro_sky[i] = v; }
        }}}
    }}}
    if let Some(ref plane) = nb.sk_xp { for z in 0..buf.sz { for y in 0..buf.sy {
        let v = clamp_sub_u8(plane[y*buf.sz+z], atten);
        if v > 0 { for mz in (z*2)..(z*2+2) { for my in (y*2)..(y*2+2) {
            let i = midx(mxs-1, my, mz, mxs, mzs); if micro_sky[i] < v { micro_sky[i] = v; }
        }}}
    }}}
    if let Some(ref plane) = nb.sk_zn { for x in 0..buf.sx { for y in 0..buf.sy {
        let v = clamp_sub_u8(plane[y*buf.sx+x], atten);
        if v > 0 { for mx in (x*2)..(x*2+2) { for my in (y*2)..(y*2+2) {
            let i = midx(mx, my, 0, mxs, mzs); if micro_sky[i] < v { micro_sky[i] = v; }
        }}}
    }}}
    if let Some(ref plane) = nb.sk_zp { for x in 0..buf.sx { for y in 0..buf.sy {
        let v = clamp_sub_u8(plane[y*buf.sx+x], atten);
        if v > 0 { for mx in (x*2)..(x*2+2) { for my in (y*2)..(y*2+2) {
            let i = midx(mx, my, mzs-1, mxs, mzs); if micro_sky[i] < v { micro_sky[i] = v; }
        }}}
    }}}

    // Seed emissive blocks at micro resolution (fill interior air micro voxels of the macro cell)
    for (lx, ly, lz, level, is_beacon) in store.emitters_for_chunk(buf.cx, buf.cz) {
        if is_beacon { continue; } // beacons not supported in Micro S=2 path yet
        let mx0 = lx * 2; let my0 = ly * 2; let mz0 = lz * 2;
        for mx in mx0..(mx0+2) { for my in my0..(my0+2) { for mz in mz0..(mz0+2) {
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

    // Choose per-micro step cost (approx half macro step)
    let att_blk: u8 = 16;
    let att_sky: u8 = 16;
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

    // Downsample micro -> macro (max over the 2x2x2 block)
    let mut lg = LightGrid::new(buf.sx, buf.sy, buf.sz);
    for z in 0..buf.sz { for y in 0..buf.sy { for x in 0..buf.sx {
        let mut smax = 0u8; let mut bmax = 0u8;
        for dz in 0..2 { for dy in 0..2 { for dx in 0..2 {
            let i = midx(x*2+dx, y*2+dy, z*2+dz, mxs, mzs);
            smax = smax.max(micro_sky[i]);
            bmax = bmax.max(micro_blk[i]);
        }}}
        let ii = ((y * buf.sz) + z) * buf.sx + x;
        lg.skylight[ii] = smax;
        lg.block_light[ii] = bmax;
    }}}

    // Borders: leave beacon arrays at default (0). Neighbor planes are derived lazily by LightBorders::from_grid upstream.
    lg
}

// Scaffold for S=2 micro-voxel lighting engine.
// For now, this delegates to the legacy voxel light grid to keep behavior unchanged
// while wiring up mode toggling and rebuild plumbing. The full implementation will
// allocate a micro grid, run bucketed BFS at S=2, and produce border planes.

// (old scaffold removed; Micro S=2 implementation is above)

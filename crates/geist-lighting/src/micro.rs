use crate::{LightGrid, LightingStore, MicroBorders};
// (Arc used via .into() conversions when publishing planes)
use geist_blocks::micro::{micro_cell_solid_s2, micro_face_cell_open_s2};
use geist_blocks::{BlockRegistry, types::Block};
use geist_chunk::ChunkBuf;
use geist_world::World;

// Micro-voxel scale factor (2x resolution in each dimension)
const MICRO_SCALE: usize = 2;

// Light attenuation values
const MICRO_BLOCK_ATTENUATION: u8 = 16; // Per-micro-step block light attenuation
pub const MICRO_SKY_ATTENUATION: u8 = 16; // Per-micro-step skylight attenuation
const COARSE_SEAM_ATTENUATION: u8 = 32; // Attenuation when falling back to coarse neighbors

// Maximum light values
const MAX_LIGHT: u8 = 255;

#[inline]
fn micro_dims(buf: &ChunkBuf) -> (usize, usize, usize) {
    (
        buf.sx * MICRO_SCALE,
        buf.sy * MICRO_SCALE,
        buf.sz * MICRO_SCALE,
    )
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
        .map(|ty| {
            ty.is_solid(b.state)
                && matches!(
                    ty.shape,
                    geist_blocks::types::Shape::Cube | geist_blocks::types::Shape::AxisCube { .. }
                )
        })
        .unwrap_or(false)
}

#[inline]
fn micro_solid_at(buf: &ChunkBuf, reg: &BlockRegistry, mx: usize, my: usize, mz: usize) -> bool {
    let x = mx >> 1;
    let y = my >> 1;
    let z = mz >> 1;
    if x >= buf.sx || y >= buf.sy || z >= buf.sz {
        return true;
    }
    let b = buf.get_local(x, y, z);
    let lx = mx & 1;
    let ly = my & 1;
    let lz = mz & 1;
    micro_cell_solid_s2(reg, b, lx, ly, lz)
}

#[inline]
fn clamp_sub_u8(v: u8, d: u8) -> u8 {
    v.saturating_sub(d)
}

pub fn compute_light_with_borders_buf_micro(
    buf: &ChunkBuf,
    store: &LightingStore,
    reg: &BlockRegistry,
    world: &World,
) -> LightGrid {
    let (mxs, mys, mzs) = micro_dims(buf);
    let mut micro_sky = vec![0u8; mxs * mys * mzs];
    let mut micro_blk = vec![0u8; mxs * mys * mzs];

    // Precompute per-macro-cell micro occupancy to accelerate micro solid checks
    let mut occ8 = vec![0u8; buf.sx * buf.sy * buf.sz];
    let mut full = vec![0u8; buf.sx * buf.sy * buf.sz];
    let idx3 = |x: usize, y: usize, z: usize| (y * buf.sz + z) * buf.sx + x;
    for z in 0..buf.sz {
        for y in 0..buf.sy {
            for x in 0..buf.sx {
                let b = buf.get_local(x, y, z);
                full[idx3(x, y, z)] = is_full_cube(reg, b) as u8;
                if let Some(o) = occ8_for(reg, b) {
                    occ8[idx3(x, y, z)] = o;
                }
            }
        }
    }
    #[inline]
    fn occ_bit8(o: u8, x: usize, y: usize, z: usize) -> bool {
        ((o >> (((y & 1) << 2) | ((z & 1) << 1) | (x & 1))) & 1) != 0
    }
    #[inline]
    fn micro_solid_at_fast(
        mx: usize,
        my: usize,
        mz: usize,
        buf: &ChunkBuf,
        occ8: &[u8],
        full: &[u8],
    ) -> bool {
        let x = mx >> 1;
        let y = my >> 1;
        let z = mz >> 1;
        if x >= buf.sx || y >= buf.sy || z >= buf.sz {
            return true;
        }
        let i = (y * buf.sz + z) * buf.sx + x;
        let o = occ8[i];
        if o != 0 {
            return occ_bit8(o, mx & 1, my & 1, mz & 1);
        }
        full[i] != 0
    }

    // BFS queues (stable order). We seed as we write, so no full-volume scan needed.
    use std::collections::VecDeque;
    let mut q_blk: VecDeque<(usize, usize, usize, u8)> = VecDeque::with_capacity(mxs * mzs);
    let mut q_sky: VecDeque<(usize, usize, usize, u8)> = VecDeque::with_capacity(mxs * mzs);

    // Seed skylight from open-above micro columns (world-local within chunk)
    for mz in 0..mzs {
        for mx in 0..mxs {
            let mut open_above = true;
            for my in (0..mys).rev() {
                if open_above {
                    if !micro_solid_at_fast(mx, my, mz, buf, &occ8, &full) {
                        let i = midx(mx, my, mz, mxs, mzs);
                        if micro_sky[i] == 0 {
                            micro_sky[i] = MAX_LIGHT;
                            q_sky.push_back((mx, my, mz, MAX_LIGHT));
                        }
                    } else {
                        open_above = false;
                    }
                }
            }
        }
    }

    // Seed from neighbor micro border planes with S=2 ghost halo; fall back to coarse upsample with proper seam gating
    let nbm = store.get_neighbor_micro_borders(buf.cx, buf.cz);
    let nb = store.get_neighbor_borders(buf.cx, buf.cz);
    let atten: u8 = COARSE_SEAM_ATTENUATION;
    let base_x = buf.cx * buf.sx as i32;
    let base_z = buf.cz * buf.sz as i32;
    // Block light neighbors
    // Skylight neighbors: handled together with block after the coarse fallback expansion

    // Expanded implementation: X seams (block + sky) with macro-first loops and cached 2x2 gates
    for lz in 0..buf.sz {
        for ly in 0..buf.sy {
            let here_nx = buf.get_local(0, ly, lz);
            let there_nx = world.block_at_runtime(reg, base_x - 1, ly as i32, base_z + lz as i32);
            let here_px = buf.get_local(buf.sx - 1, ly, lz);
            let there_px = world.block_at_runtime(
                reg,
                base_x + buf.sx as i32,
                ly as i32,
                base_z + lz as i32,
            );
            // Precompute gate masks for -X (face=3) and +X (face=2)
            let mut gate_nx = [[false; 2]; 2];
            let mut gate_px = [[false; 2]; 2];
            for iym in 0..2 {
                for izm in 0..2 {
                    gate_nx[iym][izm] = micro_face_cell_open_s2(reg, here_nx, there_nx, 3, iym, izm);
                    gate_px[iym][izm] = micro_face_cell_open_s2(reg, here_px, there_px, 2, iym, izm);
                }
            }
            // Process the four micro offsets within this macro pair
            for iym in 0..2 {
                for izm in 0..2 {
                    let my = (ly << 1) | iym;
                    let mz = (lz << 1) | izm;
                    // -X
                    let seed_blk_nx = nbm
                        .xm_bl_neg
                        .as_ref()
                        .map(|p| clamp_sub_u8(p[my * mzs + mz], MICRO_BLOCK_ATTENUATION))
                        .unwrap_or_else(|| {
                            nb.xn
                                .as_ref()
                                .map(|p| clamp_sub_u8(p[ly * buf.sz + lz], atten))
                                .unwrap_or(0)
                        });
                    let seed_sky_nx = nbm
                        .xm_sk_neg
                        .as_ref()
                        .map(|p| clamp_sub_u8(p[my * mzs + mz], MICRO_SKY_ATTENUATION))
                        .unwrap_or_else(|| {
                            nb.sk_xn
                                .as_ref()
                                .map(|p| clamp_sub_u8(p[ly * buf.sz + lz], atten))
                                .unwrap_or(0)
                        });
                    if (seed_blk_nx > 0 || seed_sky_nx > 0) && gate_nx[iym][izm] {
                        let i = midx(0, my, mz, mxs, mzs);
                        if seed_blk_nx > 0
                            && !micro_solid_at_fast(0, my, mz, buf, &occ8, &full)
                            && micro_blk[i] < seed_blk_nx
                        {
                            micro_blk[i] = seed_blk_nx;
                            q_blk.push_back((0, my, mz, seed_blk_nx));
                        }
                        if seed_sky_nx > 0
                            && !micro_solid_at_fast(0, my, mz, buf, &occ8, &full)
                            && micro_sky[i] < seed_sky_nx
                        {
                            micro_sky[i] = seed_sky_nx;
                            q_sky.push_back((0, my, mz, seed_sky_nx));
                        }
                    }
                    // +X
                    let seed_blk_px = nbm
                        .xm_bl_pos
                        .as_ref()
                        .map(|p| clamp_sub_u8(p[my * mzs + mz], MICRO_BLOCK_ATTENUATION))
                        .unwrap_or_else(|| {
                            nb.xp
                                .as_ref()
                                .map(|p| clamp_sub_u8(p[ly * buf.sz + lz], atten))
                                .unwrap_or(0)
                        });
                    let seed_sky_px = nbm
                        .xm_sk_pos
                        .as_ref()
                        .map(|p| clamp_sub_u8(p[my * mzs + mz], MICRO_SKY_ATTENUATION))
                        .unwrap_or_else(|| {
                            nb.sk_xp
                                .as_ref()
                                .map(|p| clamp_sub_u8(p[ly * buf.sz + lz], atten))
                                .unwrap_or(0)
                        });
                    if (seed_blk_px > 0 || seed_sky_px > 0) && gate_px[iym][izm] {
                        let i = midx(mxs - 1, my, mz, mxs, mzs);
                        if seed_blk_px > 0
                            && !micro_solid_at_fast(mxs - 1, my, mz, buf, &occ8, &full)
                            && micro_blk[i] < seed_blk_px
                        {
                            micro_blk[i] = seed_blk_px;
                            q_blk.push_back((mxs - 1, my, mz, seed_blk_px));
                        }
                        if seed_sky_px > 0
                            && !micro_solid_at_fast(mxs - 1, my, mz, buf, &occ8, &full)
                            && micro_sky[i] < seed_sky_px
                        {
                            micro_sky[i] = seed_sky_px;
                            q_sky.push_back((mxs - 1, my, mz, seed_sky_px));
                        }
                    }
                }
            }
        }
    }
    // Z seams (block + sky) with macro-first loops and cached 2x2 gates
    for ly in 0..buf.sy {
        for lx in 0..buf.sx {
            let here_nz = buf.get_local(lx, ly, 0);
            let there_nz = world.block_at_runtime(reg, base_x + lx as i32, ly as i32, base_z - 1);
            let here_pz = buf.get_local(lx, ly, buf.sz - 1);
            let there_pz = world.block_at_runtime(
                reg,
                base_x + lx as i32,
                ly as i32,
                base_z + buf.sz as i32,
            );
            let mut gate_nz = [[false; 2]; 2];
            let mut gate_pz = [[false; 2]; 2];
            for ixm in 0..2 {
                for iym in 0..2 {
                    gate_nz[iym][ixm] = micro_face_cell_open_s2(reg, here_nz, there_nz, 5, ixm, iym);
                    gate_pz[iym][ixm] = micro_face_cell_open_s2(reg, here_pz, there_pz, 4, ixm, iym);
                }
            }
            for ixm in 0..2 {
                for iym in 0..2 {
                    let mx = (lx << 1) | ixm;
                    let my = (ly << 1) | iym;
                    // -Z
                    let seed_blk_nz = nbm
                        .zm_bl_neg
                        .as_ref()
                        .map(|p| clamp_sub_u8(p[my * mxs + mx], MICRO_BLOCK_ATTENUATION))
                        .unwrap_or_else(|| {
                            nb.zn
                                .as_ref()
                                .map(|p| clamp_sub_u8(p[ly * buf.sx + lx], atten))
                                .unwrap_or(0)
                        });
                    let seed_sky_nz = nbm
                        .zm_sk_neg
                        .as_ref()
                        .map(|p| clamp_sub_u8(p[my * mxs + mx], MICRO_SKY_ATTENUATION))
                        .unwrap_or_else(|| {
                            nb.sk_zn
                                .as_ref()
                                .map(|p| clamp_sub_u8(p[ly * buf.sx + lx], atten))
                                .unwrap_or(0)
                        });
                    if (seed_blk_nz > 0 || seed_sky_nz > 0) && gate_nz[iym][ixm] {
                        let i = midx(mx, my, 0, mxs, mzs);
                        if seed_blk_nz > 0
                            && !micro_solid_at_fast(mx, my, 0, buf, &occ8, &full)
                            && micro_blk[i] < seed_blk_nz
                        {
                            micro_blk[i] = seed_blk_nz;
                            q_blk.push_back((mx, my, 0, seed_blk_nz));
                        }
                        if seed_sky_nz > 0
                            && !micro_solid_at_fast(mx, my, 0, buf, &occ8, &full)
                            && micro_sky[i] < seed_sky_nz
                        {
                            micro_sky[i] = seed_sky_nz;
                            q_sky.push_back((mx, my, 0, seed_sky_nz));
                        }
                    }
                    // +Z
                    let seed_blk_pz = nbm
                        .zm_bl_pos
                        .as_ref()
                        .map(|p| clamp_sub_u8(p[my * mxs + mx], MICRO_BLOCK_ATTENUATION))
                        .unwrap_or_else(|| {
                            nb.zp
                                .as_ref()
                                .map(|p| clamp_sub_u8(p[ly * buf.sx + lx], atten))
                                .unwrap_or(0)
                        });
                    let seed_sky_pz = nbm
                        .zm_sk_pos
                        .as_ref()
                        .map(|p| clamp_sub_u8(p[my * mxs + mx], MICRO_SKY_ATTENUATION))
                        .unwrap_or_else(|| {
                            nb.sk_zp
                                .as_ref()
                                .map(|p| clamp_sub_u8(p[ly * buf.sx + lx], atten))
                                .unwrap_or(0)
                        });
                    if (seed_blk_pz > 0 || seed_sky_pz > 0) && gate_pz[iym][ixm] {
                        let i = midx(mx, my, mzs - 1, mxs, mzs);
                        if seed_blk_pz > 0
                            && !micro_solid_at_fast(mx, my, mzs - 1, buf, &occ8, &full)
                            && micro_blk[i] < seed_blk_pz
                        {
                            micro_blk[i] = seed_blk_pz;
                            q_blk.push_back((mx, my, mzs - 1, seed_blk_pz));
                        }
                        if seed_sky_pz > 0
                            && !micro_solid_at_fast(mx, my, mzs - 1, buf, &occ8, &full)
                            && micro_sky[i] < seed_sky_pz
                        {
                            micro_sky[i] = seed_sky_pz;
                            q_sky.push_back((mx, my, mzs - 1, seed_sky_pz));
                        }
                    }
                }
            }
        }
    }

    // Seed emissive blocks at micro resolution (fill interior air micro voxels of the macro cell)
    for (lx, ly, lz, level, is_beacon) in store.emitters_for_chunk(buf.cx, buf.cz) {
        if is_beacon {
            continue;
        } // beacons not supported in Micro S=2 path yet
        let mx0 = lx * MICRO_SCALE;
        let my0 = ly * MICRO_SCALE;
        let mz0 = lz * MICRO_SCALE;
        for mx in mx0..(mx0 + MICRO_SCALE) {
            for my in my0..(my0 + MICRO_SCALE) {
                for mz in mz0..(mz0 + MICRO_SCALE) {
                    if !micro_solid_at_fast(mx, my, mz, buf, &occ8, &full) {
                        let i = midx(mx, my, mz, mxs, mzs);
                        if micro_blk[i] < level {
                            micro_blk[i] = level;
                            q_blk.push_back((mx, my, mz, level));
                        }
                    }
                }
            }
        }
    }

    // Propagate block light (omni) and skylight with per-micro step attenuation

    // Use per-micro step attenuation constants
    let att_blk: u8 = MICRO_BLOCK_ATTENUATION;
    let att_sky: u8 = MICRO_SKY_ATTENUATION;
    let mut push = |mx: i32,
                    my: i32,
                    mz: i32,
                    mxs: usize,
                    mys: usize,
                    mzs: usize,
                    arr: &mut [u8],
                    lvl: u8,
                    att: u8| {
        if mx < 0 || my < 0 || mz < 0 {
            return;
        }
        let (mxu, myu, mzu) = (mx as usize, my as usize, mz as usize);
        if mxu >= mxs || myu >= mys || mzu >= mzs {
            return;
        }
        if micro_solid_at_fast(mxu, myu, mzu, buf, &occ8, &full) {
            return;
        }
        let v = clamp_sub_u8(lvl, att);
        if v == 0 {
            return;
        }
        let i = midx(mxu, myu, mzu, mxs, mzs);
        if arr[i] < v {
            arr[i] = v;
        }
    };

    // BFS over block-light queue
    while let Some((mx, my, mz, level)) = q_blk.pop_front() {
        if level <= 1 {
            continue;
        }
        let lvl = level;
        let (mx_i, my_i, mz_i) = (mx as i32, my as i32, mz as i32);
        let before = micro_blk[midx(mx, my, mz, mxs, mzs)];
        if before != lvl {
            continue;
        }
        push(mx_i + 1, my_i, mz_i, mxs, mys, mzs, &mut micro_blk, lvl, att_blk);
        push(mx_i - 1, my_i, mz_i, mxs, mys, mzs, &mut micro_blk, lvl, att_blk);
        push(mx_i, my_i + 1, mz_i, mxs, mys, mzs, &mut micro_blk, lvl, att_blk);
        push(mx_i, my_i - 1, mz_i, mxs, mys, mzs, &mut micro_blk, lvl, att_blk);
        push(mx_i, my_i, mz_i + 1, mxs, mys, mzs, &mut micro_blk, lvl, att_blk);
        push(mx_i, my_i, mz_i - 1, mxs, mys, mzs, &mut micro_blk, lvl, att_blk);
        // enqueue neighbors that we updated
        let neigh = [
            (mx_i + 1, my_i, mz_i),
            (mx_i - 1, my_i, mz_i),
            (mx_i, my_i + 1, mz_i),
            (mx_i, my_i - 1, mz_i),
            (mx_i, my_i, mz_i + 1),
            (mx_i, my_i, mz_i - 1),
        ];
        for &(nx, ny, nz) in &neigh {
            if nx >= 0 && ny >= 0 && nz >= 0 {
                let (nxu, nyu, nzu) = (nx as usize, ny as usize, nz as usize);
                if nxu < mxs && nyu < mys && nzu < mzs {
                    let ii = midx(nxu, nyu, nzu, mxs, mzs);
                    if (micro_blk[ii] as u16 + att_blk as u16) == (lvl as u16) {
                        q_blk.push_back((nxu, nyu, nzu, micro_blk[ii]));
                    }
                }
            }
        }
    }

    // BFS over skylight queue
    while let Some((mx, my, mz, level)) = q_sky.pop_front() {
        if level <= 1 {
            continue;
        }
        let lvl = level;
        let (mx_i, my_i, mz_i) = (mx as i32, my as i32, mz as i32);
        let before = micro_sky[midx(mx, my, mz, mxs, mzs)];
        if before != lvl {
            continue;
        }
        push(mx_i + 1, my_i, mz_i, mxs, mys, mzs, &mut micro_sky, lvl, att_sky);
        push(mx_i - 1, my_i, mz_i, mxs, mys, mzs, &mut micro_sky, lvl, att_sky);
        push(mx_i, my_i + 1, mz_i, mxs, mys, mzs, &mut micro_sky, lvl, att_sky);
        push(mx_i, my_i - 1, mz_i, mxs, mys, mzs, &mut micro_sky, lvl, att_sky);
        push(mx_i, my_i, mz_i + 1, mxs, mys, mzs, &mut micro_sky, lvl, att_sky);
        push(mx_i, my_i, mz_i - 1, mxs, mys, mzs, &mut micro_sky, lvl, att_sky);
        // enqueue neighbors that we updated
        let neigh = [
            (mx_i + 1, my_i, mz_i),
            (mx_i - 1, my_i, mz_i),
            (mx_i, my_i + 1, mz_i),
            (mx_i, my_i - 1, mz_i),
            (mx_i, my_i, mz_i + 1),
            (mx_i, my_i, mz_i - 1),
        ];
        for &(nx, ny, nz) in &neigh {
            if nx >= 0 && ny >= 0 && nz >= 0 {
                let (nxu, nyu, nzu) = (nx as usize, ny as usize, nz as usize);
                if nxu < mxs && nyu < mys && nzu < mzs {
                    let ii = midx(nxu, nyu, nzu, mxs, mzs);
                    if (micro_sky[ii] as u16 + att_sky as u16) == (lvl as u16) {
                        q_sky.push_back((nxu, nyu, nzu, micro_sky[ii]));
                    }
                }
            }
        }
    }

    // Downsample micro -> macro (max over the 2x2x2 block) and retain micro arrays + neighbor planes
    let mut lg = LightGrid::new(buf.sx, buf.sy, buf.sz);
    let stride_z = mxs; // +1 micro Z
    let stride_y = mxs * mzs; // +1 micro Y
    for z in 0..buf.sz {
        for y in 0..buf.sy {
            for x in 0..buf.sx {
                let mx0 = x << 1;
                let my0 = y << 1;
                let mz0 = z << 1;
                let i000 = midx(mx0, my0, mz0, mxs, mzs);
                let i001 = i000 + stride_z;
                let i010 = i000 + stride_y;
                let i011 = i010 + stride_z;
                let i100 = i000 + 1;
                let i101 = i100 + stride_z;
                let i110 = i100 + stride_y;
                let i111 = i110 + stride_z;
                let smax = *[
                    micro_sky[i000],
                    micro_sky[i001],
                    micro_sky[i010],
                    micro_sky[i011],
                    micro_sky[i100],
                    micro_sky[i101],
                    micro_sky[i110],
                    micro_sky[i111],
                ]
                .iter()
                .max()
                .unwrap();
                let bmax = *[
                    micro_blk[i000],
                    micro_blk[i001],
                    micro_blk[i010],
                    micro_blk[i011],
                    micro_blk[i100],
                    micro_blk[i101],
                    micro_blk[i110],
                    micro_blk[i111],
                ]
                .iter()
                .max()
                .unwrap();
                let ii = ((y * buf.sz) + z) * buf.sx + x;
                lg.skylight[ii] = smax;
                lg.block_light[ii] = bmax;
            }
        }
    }

    // Compute and publish micro border planes for this chunk (we own -X/-Y/-Z planes for stitching)
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
    // X planes
    for my in 0..mys {
        for mz in 0..mzs {
            let idx = my * mzs + mz;
            xm_sk_neg[idx] = micro_sky[midx(0, my, mz, mxs, mzs)];
            xm_bl_neg[idx] = micro_blk[midx(0, my, mz, mxs, mzs)];
            xm_sk_pos[idx] = micro_sky[midx(mxs - 1, my, mz, mxs, mzs)];
            xm_bl_pos[idx] = micro_blk[midx(mxs - 1, my, mz, mxs, mzs)];
        }
    }
    // Z planes
    for my in 0..mys {
        for mx in 0..mxs {
            let idx = my * mxs + mx;
            zm_sk_neg[idx] = micro_sky[midx(mx, my, 0, mxs, mzs)];
            zm_bl_neg[idx] = micro_blk[midx(mx, my, 0, mxs, mzs)];
            zm_sk_pos[idx] = micro_sky[midx(mx, my, mzs - 1, mxs, mzs)];
            zm_bl_pos[idx] = micro_blk[midx(mx, my, mzs - 1, mxs, mzs)];
        }
    }
    // Y planes
    for mz in 0..mzs {
        for mx in 0..mxs {
            let idx = mz * mxs + mx;
            ym_sk_neg[idx] = micro_sky[midx(mx, 0, mz, mxs, mzs)];
            ym_bl_neg[idx] = micro_blk[midx(mx, 0, mz, mxs, mzs)];
            ym_sk_pos[idx] = micro_sky[midx(mx, mys - 1, mz, mxs, mzs)];
            ym_bl_pos[idx] = micro_blk[midx(mx, mys - 1, mz, mxs, mzs)];
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
    // Attach micro arrays and neighbor planes to LightGrid for micro face sampling
    lg.m_sky = Some(micro_sky);
    lg.m_blk = Some(micro_blk);
    // Add neighbor micro planes for sampling across seams
    lg.mnb_xn_sky = nbm.xm_sk_neg;
    lg.mnb_xp_sky = nbm.xm_sk_pos;
    lg.mnb_zn_sky = nbm.zm_sk_neg;
    lg.mnb_zp_sky = nbm.zm_sk_pos;
    lg.mnb_yn_sky = nbm.ym_sk_neg;
    lg.mnb_yp_sky = nbm.ym_sk_pos;
    lg.mnb_xn_blk = nbm.xm_bl_neg;
    lg.mnb_xp_blk = nbm.xm_bl_pos;
    lg.mnb_zn_blk = nbm.zm_bl_neg;
    lg.mnb_zp_blk = nbm.zm_bl_pos;
    lg.mnb_yn_blk = nbm.ym_bl_neg;
    lg.mnb_yp_blk = nbm.ym_bl_pos;
    // Coarse planes are still derived by LightBorders::from_grid upstream.
    lg
}

// Scaffold for S=2 micro-voxel lighting engine.
// For now, this delegates to the legacy voxel light grid to keep behavior unchanged
// while wiring up mode toggling and rebuild plumbing. The full implementation will
// allocate a micro grid, run bucketed BFS at S=2, and produce border planes.

// (old scaffold removed; Micro S=2 implementation is above)

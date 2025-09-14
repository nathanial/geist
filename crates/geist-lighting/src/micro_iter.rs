use crate::{LightGrid, LightingStore, MicroBorders};
use geist_blocks::{types::Block, BlockRegistry};
use geist_chunk::ChunkBuf;

// Shared constants with micro BFS implementation
const MICRO_SCALE: usize = 2;
const MICRO_BLOCK_ATTENUATION: u8 = 16;
const MICRO_SKY_ATTENUATION: u8 = 16;
const COARSE_SEAM_ATTENUATION: u8 = 32;
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
fn clamp_sub_u8(v: u8, d: u8) -> u8 {
    v.saturating_sub(d)
}

#[inline]
fn bs_set(bits: &mut [u64], idx: usize) {
    let w = idx >> 6;
    let b = idx & 63;
    bits[w] |= 1u64 << b;
}

#[inline]
fn bs_get(bits: &[u64], idx: usize) -> bool {
    let w = idx >> 6;
    let b = idx & 63;
    ((bits[w] >> b) & 1) != 0
}

pub fn compute_light_with_borders_buf_iterative(
    buf: &ChunkBuf,
    store: &LightingStore,
    reg: &BlockRegistry,
) -> LightGrid {
    let (mxs, mys, mzs) = micro_dims(buf);
    let micro_len = mxs * mys * mzs;
    let mut micro_sky = vec![0u8; micro_len];
    let mut micro_blk = vec![0u8; micro_len];
    let mut seed_sky = vec![0u8; micro_len];
    let mut seed_blk = vec![0u8; micro_len];

    // Occupancy
    let mut micro_solid_bits = vec![0u64; (micro_len + 63) / 64];
    let stride_z_m = mxs;
    let stride_y_m = mxs * mzs;
    let idx3 = |x: usize, y: usize, z: usize| (y * buf.sz + z) * buf.sx + x;
    for z in 0..buf.sz {
        for y in 0..buf.sy {
            for x in 0..buf.sx {
                let b = buf.get_local(x, y, z);
                let mx0 = x * 2;
                let my0 = y * 2;
                let mz0 = z * 2;
                let base = (my0 * mzs + mz0) * mxs + mx0;
                if is_full_cube(reg, b) {
                    bs_set(&mut micro_solid_bits, base);
                    bs_set(&mut micro_solid_bits, base + 1);
                    bs_set(&mut micro_solid_bits, base + stride_z_m);
                    bs_set(&mut micro_solid_bits, base + stride_z_m + 1);
                    bs_set(&mut micro_solid_bits, base + stride_y_m);
                    bs_set(&mut micro_solid_bits, base + stride_y_m + 1);
                    bs_set(&mut micro_solid_bits, base + stride_y_m + stride_z_m);
                    bs_set(&mut micro_solid_bits, base + stride_y_m + stride_z_m + 1);
                } else if let Some(o) = occ8_for(reg, b) {
                    if (o & (1 << 0)) != 0 { bs_set(&mut micro_solid_bits, base); }
                    if (o & (1 << 1)) != 0 { bs_set(&mut micro_solid_bits, base + 1); }
                    if (o & (1 << 2)) != 0 { bs_set(&mut micro_solid_bits, base + stride_z_m); }
                    if (o & (1 << 3)) != 0 { bs_set(&mut micro_solid_bits, base + stride_z_m + 1); }
                    if (o & (1 << 4)) != 0 { bs_set(&mut micro_solid_bits, base + stride_y_m); }
                    if (o & (1 << 5)) != 0 { bs_set(&mut micro_solid_bits, base + stride_y_m + 1); }
                    if (o & (1 << 6)) != 0 { bs_set(&mut micro_solid_bits, base + stride_y_m + stride_z_m); }
                    if (o & (1 << 7)) != 0 { bs_set(&mut micro_solid_bits, base + stride_y_m + stride_z_m + 1); }
                }
            }
        }
    }

    // Skylight open-above seeds
    let mut open_start = vec![mys; mxs * mzs];
    for mz in 0..mzs {
        for mx in 0..mxs {
            let mut start = 0usize;
            let mut y = mys as i32 - 1;
            while y >= 0 {
                let ii = midx(mx, y as usize, mz, mxs, mzs);
                if bs_get(&micro_solid_bits, ii) { start = (y as usize).saturating_add(1); break; }
                y -= 1;
            }
            open_start[mz * mxs + mx] = start;
            for my in start..mys {
                let ii = midx(mx, my, mz, mxs, mzs);
                seed_sky[ii] = MAX_LIGHT; // 255
            }
        }
    }

    // Neighbor micro planes (preferred) and coarse fallback seeds
    let nbm = store.get_neighbor_micro_borders(buf.cx, buf.cz);
    let nb = store.get_neighbor_borders(buf.cx, buf.cz);
    // X seams
    for ly in 0..buf.sy { for lz in 0..buf.sz {
        for iym in 0..2 { for izm in 0..2 {
            let my = (ly << 1) | iym; let mz = (lz << 1) | izm;
            let off_m = my * mzs + mz;
            // -X
            let vblk_xn = nbm.xm_bl_neg.as_ref().map(|p| clamp_sub_u8(p[off_m], MICRO_BLOCK_ATTENUATION))
                .or_else(|| nb.xn.as_ref().map(|p| clamp_sub_u8(p[ly * buf.sz + lz], COARSE_SEAM_ATTENUATION))).unwrap_or(0);
            let vsky_xn = nbm.xm_sk_neg.as_ref().map(|p| clamp_sub_u8(p[off_m], MICRO_SKY_ATTENUATION))
                .or_else(|| nb.sk_xn.as_ref().map(|p| clamp_sub_u8(p[ly * buf.sz + lz], COARSE_SEAM_ATTENUATION))).unwrap_or(0);
            if vblk_xn > 0 { seed_blk[midx(0, my, mz, mxs, mzs)] = seed_blk[midx(0, my, mz, mxs, mzs)].max(vblk_xn); }
            if vsky_xn > 0 { seed_sky[midx(0, my, mz, mxs, mzs)] = seed_sky[midx(0, my, mz, mxs, mzs)].max(vsky_xn); }
            // +X
            let vblk_xp = nbm.xm_bl_pos.as_ref().map(|p| clamp_sub_u8(p[off_m], MICRO_BLOCK_ATTENUATION))
                .or_else(|| nb.xp.as_ref().map(|p| clamp_sub_u8(p[ly * buf.sz + lz], COARSE_SEAM_ATTENUATION))).unwrap_or(0);
            let vsky_xp = nbm.xm_sk_pos.as_ref().map(|p| clamp_sub_u8(p[off_m], MICRO_SKY_ATTENUATION))
                .or_else(|| nb.sk_xp.as_ref().map(|p| clamp_sub_u8(p[ly * buf.sz + lz], COARSE_SEAM_ATTENUATION))).unwrap_or(0);
            let ii_xp = midx(mxs - 1, my, mz, mxs, mzs);
            if vblk_xp > 0 { seed_blk[ii_xp] = seed_blk[ii_xp].max(vblk_xp); }
            if vsky_xp > 0 { seed_sky[ii_xp] = seed_sky[ii_xp].max(vsky_xp); }
        }}
    }}
    // Z seams
    for ly in 0..buf.sy { for lx in 0..buf.sx {
        for iym in 0..2 { for ixm in 0..2 {
            let my = (ly << 1) | iym; let mx = (lx << 1) | ixm;
            let off_m = my * mxs + mx;
            // -Z
            let vblk_zn = nbm.zm_bl_neg.as_ref().map(|p| clamp_sub_u8(p[off_m], MICRO_BLOCK_ATTENUATION))
                .or_else(|| nb.zn.as_ref().map(|p| clamp_sub_u8(p[ly * buf.sx + lx], COARSE_SEAM_ATTENUATION))).unwrap_or(0);
            let vsky_zn = nbm.zm_sk_neg.as_ref().map(|p| clamp_sub_u8(p[off_m], MICRO_SKY_ATTENUATION))
                .or_else(|| nb.sk_zn.as_ref().map(|p| clamp_sub_u8(p[ly * buf.sx + lx], COARSE_SEAM_ATTENUATION))).unwrap_or(0);
            if vblk_zn > 0 { seed_blk[midx(mx, my, 0, mxs, mzs)] = seed_blk[midx(mx, my, 0, mxs, mzs)].max(vblk_zn); }
            if vsky_zn > 0 { seed_sky[midx(mx, my, 0, mxs, mzs)] = seed_sky[midx(mx, my, 0, mxs, mzs)].max(vsky_zn); }
            // +Z
            let vblk_zp = nbm.zm_bl_pos.as_ref().map(|p| clamp_sub_u8(p[off_m], MICRO_BLOCK_ATTENUATION))
                .or_else(|| nb.zp.as_ref().map(|p| clamp_sub_u8(p[ly * buf.sx + lx], COARSE_SEAM_ATTENUATION))).unwrap_or(0);
            let vsky_zp = nbm.zm_sk_pos.as_ref().map(|p| clamp_sub_u8(p[off_m], MICRO_SKY_ATTENUATION))
                .or_else(|| nb.sk_zp.as_ref().map(|p| clamp_sub_u8(p[ly * buf.sx + lx], COARSE_SEAM_ATTENUATION))).unwrap_or(0);
            let ii_zp = midx(mx, my, mzs - 1, mxs, mzs);
            if vblk_zp > 0 { seed_blk[ii_zp] = seed_blk[ii_zp].max(vblk_zp); }
            if vsky_zp > 0 { seed_sky[ii_zp] = seed_sky[ii_zp].max(vsky_zp); }
        }}
    }}

    // Emissive block seeds (faces + adjacent outside if air)
    for z in 0..buf.sz {
        for y in 0..buf.sy {
            for x in 0..buf.sx {
                let b = buf.get_local(x, y, z);
                if let Some(ty) = reg.get(b.id) {
                    let level = ty.light_emission(b.state);
                    if level == 0 { continue; }
                    let bx = x * 2; let by = y * 2; let bz = z * 2;
                    let mut seed_idx = |ii: usize| { if seed_blk[ii] < level { seed_blk[ii] = level; } };
                    // +X face and outside
                    for oy in 0..2 { for oz in 0..2 {
                        let ii_in = midx(bx + 1, by + oy, bz + oz, mxs, mzs); seed_idx(ii_in);
                        let mx_out = bx + 2; if mx_out < mxs {
                            let ii_out = midx(mx_out, by + oy, bz + oz, mxs, mzs);
                            if !bs_get(&micro_solid_bits, ii_out) { seed_idx(ii_out); }
                        }
                    }}
                    // -X
                    for oy in 0..2 { for oz in 0..2 {
                        let ii_in = midx(bx + 0, by + oy, bz + oz, mxs, mzs); seed_idx(ii_in);
                        if bx > 0 {
                            let ii_out = midx(bx - 1, by + oy, bz + oz, mxs, mzs);
                            if !bs_get(&micro_solid_bits, ii_out) { seed_idx(ii_out); }
                        }
                    }}
                    // +Y
                    for oz in 0..2 { for ox in 0..2 {
                        let ii_in = midx(bx + ox, by + 1, bz + oz, mxs, mzs); seed_idx(ii_in);
                        let my_out = by + 2; if my_out < mys {
                            let ii_out = midx(bx + ox, my_out, bz + oz, mxs, mzs);
                            if !bs_get(&micro_solid_bits, ii_out) { seed_idx(ii_out); }
                        }
                    }}
                    // -Y
                    for oz in 0..2 { for ox in 0..2 {
                        let ii_in = midx(bx + ox, by + 0, bz + oz, mxs, mzs); seed_idx(ii_in);
                        if by > 0 {
                            let ii_out = midx(bx + ox, by - 1, bz + oz, mxs, mzs);
                            if !bs_get(&micro_solid_bits, ii_out) { seed_idx(ii_out); }
                        }
                    }}
                    // +Z
                    for oy in 0..2 { for ox in 0..2 {
                        let ii_in = midx(bx + ox, by + oy, bz + 1, mxs, mzs); seed_idx(ii_in);
                        let mz_out = bz + 2; if mz_out < mzs {
                            let ii_out = midx(bx + ox, by + oy, mz_out, mxs, mzs);
                            if !bs_get(&micro_solid_bits, ii_out) { seed_idx(ii_out); }
                        }
                    }}
                    // -Z
                    for oy in 0..2 { for ox in 0..2 {
                        let ii_in = midx(bx + ox, by + oy, bz + 0, mxs, mzs); seed_idx(ii_in);
                        if bz > 0 {
                            let ii_out = midx(bx + ox, by + oy, bz - 1, mxs, mzs);
                            if !bs_get(&micro_solid_bits, ii_out) { seed_idx(ii_out); }
                        }
                    }}
                }
            }
        }
    }
    // Runtime emitter overlay
    for (lx, ly, lz, level, _is_beacon) in store.emitters_for_chunk(buf.cx, buf.cz) {
        if level == 0 { continue; }
        let mx0 = lx * 2; let my0 = ly * 2; let mz0 = lz * 2;
        for mx in mx0..(mx0 + 2) { for my in my0..(my0 + 2) { for mz in mz0..(mz0 + 2) {
            let ii = midx(mx, my, mz, mxs, mzs);
            if !bs_get(&micro_solid_bits, ii) { if seed_blk[ii] < level { seed_blk[ii] = level; } }
        }}} 
    }

    // Iterative relaxation (double-buffer ping-pong)
    let iters_blk = (MAX_LIGHT as u16 / MICRO_BLOCK_ATTENUATION as u16) as usize;
    let iters_sky = (MAX_LIGHT as u16 / MICRO_SKY_ATTENUATION as u16) as usize;

    let mut cur_blk = micro_blk;
    let mut nxt_blk = vec![0u8; micro_len];
    for _ in 0..iters_blk {
        let mut any = false;
        for my in 0..mys { for mz in 0..mzs { for mx in 0..mxs {
            let i = midx(mx, my, mz, mxs, mzs);
            let mut v = seed_blk[i].max(cur_blk[i]);
            if !bs_get(&micro_solid_bits, i) {
                let att = MICRO_BLOCK_ATTENUATION;
                // +X
                if mx + 1 < mxs { let n = i + 1; v = v.max(cur_blk[n].saturating_sub(att)); }
                // -X
                if mx > 0 { let n = i - 1; v = v.max(cur_blk[n].saturating_sub(att)); }
                // +Y
                if my + 1 < mys { let n = i + stride_y_m; v = v.max(cur_blk[n].saturating_sub(att)); }
                // -Y
                if my > 0 { let n = i - stride_y_m; v = v.max(cur_blk[n].saturating_sub(att)); }
                // +Z
                if mz + 1 < mzs { let n = i + stride_z_m; v = v.max(cur_blk[n].saturating_sub(att)); }
                // -Z
                if mz > 0 { let n = i - stride_z_m; v = v.max(cur_blk[n].saturating_sub(att)); }
            }
            nxt_blk[i] = v;
            any |= v != cur_blk[i];
        }}}
        if !any { break; }
        std::mem::swap(&mut cur_blk, &mut nxt_blk);
    }

    let mut cur_sky = micro_sky;
    let mut nxt_sky = vec![0u8; micro_len];
    for _ in 0..iters_sky {
        let mut any = false;
        for my in 0..mys { for mz in 0..mzs { for mx in 0..mxs {
            let i = midx(mx, my, mz, mxs, mzs);
            let mut v = seed_sky[i].max(cur_sky[i]);
            if !bs_get(&micro_solid_bits, i) {
                let att = MICRO_SKY_ATTENUATION;
                if mx + 1 < mxs { let n = i + 1; v = v.max(cur_sky[n].saturating_sub(att)); }
                if mx > 0 { let n = i - 1; v = v.max(cur_sky[n].saturating_sub(att)); }
                if my + 1 < mys { let n = i + stride_y_m; v = v.max(cur_sky[n].saturating_sub(att)); }
                if my > 0 { let n = i - stride_y_m; v = v.max(cur_sky[n].saturating_sub(att)); }
                if mz + 1 < mzs { let n = i + stride_z_m; v = v.max(cur_sky[n].saturating_sub(att)); }
                if mz > 0 { let n = i - stride_z_m; v = v.max(cur_sky[n].saturating_sub(att)); }
            }
            nxt_sky[i] = v;
            any |= v != cur_sky[i];
        }}}
        if !any { break; }
        std::mem::swap(&mut cur_sky, &mut nxt_sky);
    }

    // Downsample micro -> macro (max over 2x2x2)
    let mut lg = LightGrid::new(buf.sx, buf.sy, buf.sz);
    let stride_z = mxs;
    let stride_y = mxs * mzs;
    for z in 0..buf.sz { for y in 0..buf.sy { for x in 0..buf.sx {
        let ii = idx3(x, y, z);
        let mx0 = x << 1; let my0 = y << 1; let mz0 = z << 1;
        let i000 = midx(mx0, my0, mz0, mxs, mzs);
        let i001 = i000 + stride_z;
        let i010 = i000 + stride_y;
        let i011 = i010 + stride_z;
        let i100 = i000 + 1;
        let i101 = i100 + stride_z;
        let i110 = i100 + stride_y;
        let i111 = i110 + stride_z;
        let smax = *[
            cur_sky[i000], cur_sky[i001], cur_sky[i010], cur_sky[i011],
            cur_sky[i100], cur_sky[i101], cur_sky[i110], cur_sky[i111],
        ].iter().max().unwrap();
        let bmax = *[
            cur_blk[i000], cur_blk[i001], cur_blk[i010], cur_blk[i011],
            cur_blk[i100], cur_blk[i101], cur_blk[i110], cur_blk[i111],
        ].iter().max().unwrap();
        lg.skylight[ii] = smax;
        lg.block_light[ii] = bmax;
    }}}

    // Publish micro border planes for neighbors
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
    for my in 0..mys { for mz in 0..mzs {
        let idx = my * mzs + mz;
        xm_sk_neg[idx] = cur_sky[midx(0, my, mz, mxs, mzs)];
        xm_bl_neg[idx] = cur_blk[midx(0, my, mz, mxs, mzs)];
        xm_sk_pos[idx] = cur_sky[midx(mxs - 1, my, mz, mxs, mzs)];
        xm_bl_pos[idx] = cur_blk[midx(mxs - 1, my, mz, mxs, mzs)];
    }}
    for my in 0..mys { for mx in 0..mxs {
        let idx = my * mxs + mx;
        zm_sk_neg[idx] = cur_sky[midx(mx, my, 0, mxs, mzs)];
        zm_bl_neg[idx] = cur_blk[midx(mx, my, 0, mxs, mzs)];
        zm_sk_pos[idx] = cur_sky[midx(mx, my, mzs - 1, mxs, mzs)];
        zm_bl_pos[idx] = cur_blk[midx(mx, my, mzs - 1, mxs, mzs)];
    }}
    for mz in 0..mzs { for mx in 0..mxs {
        let idx = mz * mxs + mx;
        ym_sk_neg[idx] = cur_sky[midx(mx, 0, mz, mxs, mzs)];
        ym_bl_neg[idx] = cur_blk[midx(mx, 0, mz, mxs, mzs)];
        ym_sk_pos[idx] = cur_sky[midx(mx, mys - 1, mz, mxs, mzs)];
        ym_bl_pos[idx] = cur_blk[midx(mx, mys - 1, mz, mxs, mzs)];
    }}
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

    // Attach micro arrays and neighbor planes to LightGrid for S=2 sampling
    lg.m_sky = Some(cur_sky);
    lg.m_blk = Some(cur_blk);
    let nbm2 = store.get_neighbor_micro_borders(buf.cx, buf.cz);
    lg.mnb_xn_sky = nbm2.xm_sk_neg;
    lg.mnb_xp_sky = nbm2.xm_sk_pos;
    lg.mnb_zn_sky = nbm2.zm_sk_neg;
    lg.mnb_zp_sky = nbm2.zm_sk_pos;
    lg.mnb_yn_sky = nbm2.ym_sk_neg;
    lg.mnb_yp_sky = nbm2.ym_sk_pos;
    lg.mnb_xn_blk = nbm2.xm_bl_neg;
    lg.mnb_xp_blk = nbm2.xm_bl_pos;
    lg.mnb_zn_blk = nbm2.zm_bl_neg;
    lg.mnb_zp_blk = nbm2.zm_bl_pos;
    lg.mnb_yn_blk = nbm2.ym_bl_neg;
    lg.mnb_yp_blk = nbm2.ym_bl_pos;
    lg
}


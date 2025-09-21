use crate::{LightGrid, LightingStore, MicroBorders};
use rayon::prelude::*;
// (Arc used via .into() conversions when publishing planes)
use geist_blocks::micro::micro_face_cell_open_s2;
use geist_blocks::{BlockRegistry, types::Block};
use geist_chunk::ChunkBuf;
use geist_world::{World, WorldGenMode};

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
    let base_x = buf.coord.cx * buf.sx as i32;
    let base_y = buf.coord.cy * buf.sy as i32;
    let base_z = buf.coord.cz * buf.sz as i32;
    let chunk_cap_y = base_y + buf.sy as i32; // exclusive upper bound for this chunk
    let world_height = world.world_height_hint() as i32;
    let mut reuse_ctx = world.make_gen_ctx();
    // Height tile lets us query column surface once instead of sampling every Y upwards.
    world.prepare_height_tile(&mut reuse_ctx, base_x, base_z, buf.sx, buf.sz);
    let height_tile = reuse_ctx.height_tile.clone();
    let mut micro_sky = vec![0u8; mxs * mys * mzs];
    let mut micro_blk = vec![0u8; mxs * mys * mzs];
    let stride_z_m = mxs; // +1 micro Z
    let stride_y_m = mxs * mzs; // +1 micro Y
    // Macro touched bitset (one bit per macro voxel)
    let macro_voxels = buf.sx * buf.sy * buf.sz;
    let mut macro_touched = vec![0u64; (macro_voxels + 63) / 64];

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

    // Build a 1-bit-per-micro-cell occupancy bitset
    let micro_bit_count = mxs * mys * mzs;
    let mut micro_solid_bits = vec![0u64; (micro_bit_count + 63) / 64];
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
    // Fill bitset using macro occupancy
    for z in 0..buf.sz {
        for y in 0..buf.sy {
            for x in 0..buf.sx {
                let i3 = (y * buf.sz + z) * buf.sx + x;
                let o = occ8[i3];
                let f = full[i3] != 0;
                let mx0 = x * 2;
                let my0 = y * 2;
                let mz0 = z * 2;
                let base = (my0 * mzs + mz0) * mxs + mx0;
                if f {
                    bs_set(&mut micro_solid_bits, base);
                    bs_set(&mut micro_solid_bits, base + 1);
                    bs_set(&mut micro_solid_bits, base + stride_z_m);
                    bs_set(&mut micro_solid_bits, base + stride_z_m + 1);
                    bs_set(&mut micro_solid_bits, base + stride_y_m);
                    bs_set(&mut micro_solid_bits, base + stride_y_m + 1);
                    bs_set(&mut micro_solid_bits, base + stride_y_m + stride_z_m);
                    bs_set(&mut micro_solid_bits, base + stride_y_m + stride_z_m + 1);
                } else if o != 0 {
                    if (o & (1 << 0)) != 0 {
                        bs_set(&mut micro_solid_bits, base);
                    }
                    if (o & (1 << 1)) != 0 {
                        bs_set(&mut micro_solid_bits, base + 1);
                    }
                    if (o & (1 << 2)) != 0 {
                        bs_set(&mut micro_solid_bits, base + stride_z_m);
                    }
                    if (o & (1 << 3)) != 0 {
                        bs_set(&mut micro_solid_bits, base + stride_z_m + 1);
                    }
                    if (o & (1 << 4)) != 0 {
                        bs_set(&mut micro_solid_bits, base + stride_y_m);
                    }
                    if (o & (1 << 5)) != 0 {
                        bs_set(&mut micro_solid_bits, base + stride_y_m + 1);
                    }
                    if (o & (1 << 6)) != 0 {
                        bs_set(&mut micro_solid_bits, base + stride_y_m + stride_z_m);
                    }
                    if (o & (1 << 7)) != 0 {
                        bs_set(&mut micro_solid_bits, base + stride_y_m + stride_z_m + 1);
                    }
                }
            }
        }
    }

    // BFS queues (stable order). We seed as we write, so no full-volume scan needed.
    struct Bucket {
        data: Vec<(usize, u8)>,
        read: usize,
    }
    impl Bucket {
        #[inline]
        fn new() -> Self {
            Self {
                data: Vec::new(),
                read: 0,
            }
        }
        #[inline]
        fn push(&mut self, idx: usize, level: u8) {
            self.data.push((idx, level));
        }
        #[inline]
        fn reset_if_empty(&mut self) {
            if self.read >= self.data.len() {
                self.data.clear();
                self.read = 0;
            }
        }
    }
    struct DialQ {
        buckets: [Bucket; 16],
        cur_d: u16,
        pending: usize,
    }
    impl DialQ {
        fn new() -> Self {
            Self {
                buckets: std::array::from_fn(|_| Bucket::new()),
                cur_d: 0,
                pending: 0,
            }
        }
        #[inline]
        fn push_idx(&mut self, idx: usize, level: u8) {
            let d = (MAX_LIGHT as u16).wrapping_sub(level as u16);
            let bi = (d & 15) as usize;
            self.buckets[bi].push(idx, level);
            if self.pending == 0 || d < self.cur_d {
                self.cur_d = d;
            }
            self.pending += 1;
        }
    }

    let mut q_blk: DialQ = DialQ::new();
    let mut q_sky: DialQ = DialQ::new();

    // Seed skylight from open-above micro columns (world-local within chunk)
    // Phase 1: compute open-above start Y for each (mx, mz) column: the first Y such that all cells above are air.
    let mut open_start = vec![mys; mxs * mzs];
    for mz in 0..mzs {
        for mx in 0..mxs {
            let mut found_solid = false;
            let mut start = 0usize;
            let mut y = mys as i32 - 1;
            while y >= 0 {
                let ii = midx(mx, y as usize, mz, mxs, mzs);
                if bs_get(&micro_solid_bits, ii) {
                    start = (y as usize).saturating_add(1);
                    found_solid = true;
                    break;
                }
                y -= 1;
            }
            if !found_solid {
                start = 0;
            }
            open_start[mz * mxs + mx] = start;
        }
    }
    // Determine which macro columns genuinely reach the sky when accounting for neighbours above.
    let mut column_open_to_sky = vec![true; buf.sx * buf.sz];
    if let Some(tile) = height_tile.as_ref() {
        for lz in 0..buf.sz {
            let wz = base_z + lz as i32;
            for lx in 0..buf.sx {
                let wx = base_x + lx as i32;
                let height = tile
                    .height(wx, wz)
                    .unwrap_or(world_height.saturating_sub(1));
                column_open_to_sky[lz * buf.sx + lx] = height < chunk_cap_y;
            }
        }
    } else if let WorldGenMode::Flat { thickness } = world.mode {
        let height = (thickness - 1).max(-1);
        for lz in 0..buf.sz {
            for lx in 0..buf.sx {
                column_open_to_sky[lz * buf.sx + lx] = height < chunk_cap_y;
            }
        }
    } else {
        // Fallback: sample upwards but bail once we find a blocking block.
        for lz in 0..buf.sz {
            let wz = base_z + lz as i32;
            for lx in 0..buf.sx {
                let wx = base_x + lx as i32;
                let mut open = true;
                let mut wy = chunk_cap_y;
                while wy < world_height {
                    let block = world.block_at_runtime_with(reg, &mut reuse_ctx, wx, wy, wz);
                    if !super::skylight_transparent_s2(block, reg) {
                        open = false;
                        break;
                    }
                    wy += 1;
                }
                column_open_to_sky[lz * buf.sx + lx] = open;
            }
        }
    }
    // Columns sealed above should not receive skylight seeding even if this chunk's top slice is air.
    for mz in 0..mzs {
        for mx in 0..mxs {
            let lx = mx >> 1;
            let lz = mz >> 1;
            if !column_open_to_sky[lz * buf.sx + lx] {
                open_start[mz * mxs + mx] = mys;
            }
        }
    }
    // Phase 2: fill all open-above cells to 255
    for mz in 0..mzs {
        for mx in 0..mxs {
            let start = open_start[mz * mxs + mx];
            if start >= mys {
                continue;
            }
            for my in start..mys {
                let i = midx(mx, my, mz, mxs, mzs);
                micro_sky[i] = MAX_LIGHT;
                // Mark macro cell as touched for downsample tightening
                let lx = mx >> 1;
                let ly = my >> 1;
                let lz = mz >> 1;
                let mii = (ly * buf.sz + lz) * buf.sx + lx;
                bs_set(&mut macro_touched, mii);
            }
        }
    }
    // Phase 3: enqueue only boundary cells (open-above cells adjacent to a lateral neighbor that is NOT open-above at same Y)
    let neighbor_start = |mx: isize, mz: isize| -> usize {
        if mx < 0 || mz < 0 || mx >= mxs as isize || mz >= mzs as isize {
            // Out of bounds: treat as same start to avoid wasting seeds on chunk edges; neighbor planes handle seams
            return mys;
        }
        open_start[(mz as usize) * mxs + (mx as usize)]
    };
    for mz in 0..mzs {
        for mx in 0..mxs {
            let start = open_start[mz * mxs + mx];
            if start >= mys {
                continue;
            }
            let nxp = neighbor_start(mx as isize + 1, mz as isize);
            let nxn = neighbor_start(mx as isize - 1, mz as isize);
            let nzp = neighbor_start(mx as isize, mz as isize + 1);
            let nzn = neighbor_start(mx as isize, mz as isize - 1);
            let max_n_start = nxp.max(nxn).max(nzp).max(nzn);
            let end_y = max_n_start.min(mys);
            for my in start..end_y {
                let i = midx(mx, my, mz, mxs, mzs);
                // Already set to 255 in Phase 2
                q_sky.push_idx(i, MAX_LIGHT);
            }
        }
    }

    // Seed from neighbor micro border planes with S=2 ghost halo; fall back to coarse upsample with proper seam gating
    let nbm = store.get_neighbor_micro_borders(buf.coord);
    let nb = store.get_neighbor_borders(buf.coord);
    let plane_nonzero = |p: &Option<std::sync::Arc<[u8]>>| -> bool {
        if let Some(a) = p {
            a.iter().any(|&v| v != 0)
        } else {
            false
        }
    };
    let use_xn = nbm.xm_bl_neg.is_some()
        || nbm.xm_sk_neg.is_some()
        || plane_nonzero(&nb.xn)
        || plane_nonzero(&nb.sk_xn);
    let use_xp = nbm.xm_bl_pos.is_some()
        || nbm.xm_sk_pos.is_some()
        || plane_nonzero(&nb.xp)
        || plane_nonzero(&nb.sk_xp);
    let use_zn = nbm.zm_bl_neg.is_some()
        || nbm.zm_sk_neg.is_some()
        || plane_nonzero(&nb.zn)
        || plane_nonzero(&nb.sk_zn);
    let use_zp = nbm.zm_bl_pos.is_some()
        || nbm.zm_sk_pos.is_some()
        || plane_nonzero(&nb.zp)
        || plane_nonzero(&nb.sk_zp);
    let use_yn = nbm.ym_bl_neg.is_some()
        || nbm.ym_sk_neg.is_some()
        || plane_nonzero(&nb.yn)
        || plane_nonzero(&nb.sk_yn);
    let use_yp = nbm.ym_bl_pos.is_some()
        || nbm.ym_sk_pos.is_some()
        || plane_nonzero(&nb.yp)
        || plane_nonzero(&nb.sk_yp);
    let atten: u8 = COARSE_SEAM_ATTENUATION;
    // Block light neighbors
    // Skylight neighbors: handled together with block after the coarse fallback expansion

    // Expanded implementation: X seams (block + sky) with macro-first loops and cached 2x2 gates
    // Avoid expensive world noise sampling when micro neighbor planes exist by gating using our
    // own micro occupancy only. When falling back to coarse neighbors, reuse a single GenCtx.
    for lz in 0..buf.sz {
        for ly in 0..buf.sy {
            if !(use_xn || use_xp) {
                continue;
            }
            let here_nx = buf.get_local(0, ly, lz);
            let here_px = buf.get_local(buf.sx - 1, ly, lz);
            let have_micro_nx = nbm.xm_bl_neg.is_some() || nbm.xm_sk_neg.is_some();
            let have_micro_px = nbm.xm_bl_pos.is_some() || nbm.xm_sk_pos.is_some();
            // Per-line micro seed precheck (2x2 micro offsets)
            let mut mic_line_nx = false;
            if have_micro_nx {
                for iym in 0..2 {
                    for izm in 0..2 {
                        let my = (ly << 1) | iym;
                        let mz = (lz << 1) | izm;
                        let off = my * mzs + mz;
                        let sblk = nbm
                            .xm_bl_neg
                            .as_ref()
                            .map(|p| clamp_sub_u8(p[off], MICRO_BLOCK_ATTENUATION))
                            .unwrap_or(0);
                        let ssky = nbm
                            .xm_sk_neg
                            .as_ref()
                            .map(|p| clamp_sub_u8(p[off], MICRO_SKY_ATTENUATION))
                            .unwrap_or(0);
                        if sblk > 0 || ssky > 0 {
                            mic_line_nx = true;
                            break;
                        }
                    }
                    if mic_line_nx {
                        break;
                    }
                }
            }
            let mut mic_line_px = false;
            if have_micro_px {
                for iym in 0..2 {
                    for izm in 0..2 {
                        let my = (ly << 1) | iym;
                        let mz = (lz << 1) | izm;
                        let off = my * mzs + mz;
                        let sblk = nbm
                            .xm_bl_pos
                            .as_ref()
                            .map(|p| clamp_sub_u8(p[off], MICRO_BLOCK_ATTENUATION))
                            .unwrap_or(0);
                        let ssky = nbm
                            .xm_sk_pos
                            .as_ref()
                            .map(|p| clamp_sub_u8(p[off], MICRO_SKY_ATTENUATION))
                            .unwrap_or(0);
                        if sblk > 0 || ssky > 0 {
                            mic_line_px = true;
                            break;
                        }
                    }
                    if mic_line_px {
                        break;
                    }
                }
            }
            // Line-level coarse seeds (skip side if no micro and no coarse seeds on this line)
            let mut coarse_xn = false;
            if let Some(ref p) = nb.xn {
                coarse_xn |= clamp_sub_u8(p[ly * buf.sz + lz], atten) > 0;
            }
            if let Some(ref p) = nb.sk_xn {
                coarse_xn |= clamp_sub_u8(p[ly * buf.sz + lz], atten) > 0;
            }
            let mut coarse_xp = false;
            if let Some(ref p) = nb.xp {
                coarse_xp |= clamp_sub_u8(p[ly * buf.sz + lz], atten) > 0;
            }
            if let Some(ref p) = nb.sk_xp {
                coarse_xp |= clamp_sub_u8(p[ly * buf.sz + lz], atten) > 0;
            }
            let mut do_xn = mic_line_nx || coarse_xn;
            let mut do_xp = mic_line_px || coarse_xp;
            if !do_xn && !do_xp {
                continue;
            }
            // Only fetch neighbor blocks when we need coarse fallback gating
            let (there_nx, there_px) = if (!have_micro_nx && do_xn) || (!have_micro_px && do_xp) {
                (
                    world.block_at_runtime_with(
                        reg,
                        &mut reuse_ctx,
                        base_x - 1,
                        base_y + ly as i32,
                        base_z + lz as i32,
                    ),
                    world.block_at_runtime_with(
                        reg,
                        &mut reuse_ctx,
                        base_x + buf.sx as i32,
                        base_y + ly as i32,
                        base_z + lz as i32,
                    ),
                )
            } else {
                // Dummy values; will not be used
                (here_nx, here_px)
            };
            // Precompute gate masks for -X (face=3) and +X (face=2)
            let mut gate_nx = [[false; 2]; 2];
            let mut gate_px = [[false; 2]; 2];
            for iym in 0..2 {
                for izm in 0..2 {
                    gate_nx[iym][izm] = if do_xn && have_micro_nx {
                        // Gate based only on our micro occupancy
                        let my = (ly << 1) | iym;
                        let mz = (lz << 1) | izm;
                        !bs_get(&micro_solid_bits, midx(0, my, mz, mxs, mzs))
                    } else if do_xn {
                        micro_face_cell_open_s2(reg, here_nx, there_nx, 3, iym, izm)
                    } else {
                        false
                    };
                    gate_px[iym][izm] = if do_xp && have_micro_px {
                        let my = (ly << 1) | iym;
                        let mz = (lz << 1) | izm;
                        !bs_get(&micro_solid_bits, midx(mxs - 1, my, mz, mxs, mzs))
                    } else if do_xp {
                        micro_face_cell_open_s2(reg, here_px, there_px, 2, iym, izm)
                    } else {
                        false
                    };
                }
            }
            // Extra pruning: if all gates closed for a side, skip it
            if do_xn {
                let mut any = false;
                for iym in 0..2 {
                    for izm in 0..2 {
                        any |= gate_nx[iym][izm];
                    }
                }
                if !any {
                    do_xn = false;
                }
            }
            if do_xp {
                let mut any = false;
                for iym in 0..2 {
                    for izm in 0..2 {
                        any |= gate_px[iym][izm];
                    }
                }
                if !any {
                    do_xp = false;
                }
            }
            if !do_xn && !do_xp {
                continue;
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
                    if do_xn && (seed_blk_nx > 0 || seed_sky_nx > 0) && gate_nx[iym][izm] {
                        let i = midx(0, my, mz, mxs, mzs);
                        if seed_blk_nx > 0
                            && !bs_get(&micro_solid_bits, midx(0, my, mz, mxs, mzs))
                            && micro_blk[i] < seed_blk_nx
                        {
                            micro_blk[i] = seed_blk_nx;
                            q_blk.push_idx(i, seed_blk_nx);
                            let mi = (ly * buf.sz + lz) * buf.sx + 0;
                            bs_set(&mut macro_touched, mi);
                        }
                        if seed_sky_nx > 0
                            && !bs_get(&micro_solid_bits, midx(0, my, mz, mxs, mzs))
                            && micro_sky[i] < seed_sky_nx
                        {
                            micro_sky[i] = seed_sky_nx;
                            q_sky.push_idx(i, seed_sky_nx);
                            let mi = (ly * buf.sz + lz) * buf.sx + 0;
                            bs_set(&mut macro_touched, mi);
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
                    if do_xp && (seed_blk_px > 0 || seed_sky_px > 0) && gate_px[iym][izm] {
                        let i = midx(mxs - 1, my, mz, mxs, mzs);
                        if seed_blk_px > 0
                            && !bs_get(&micro_solid_bits, midx(mxs - 1, my, mz, mxs, mzs))
                            && micro_blk[i] < seed_blk_px
                        {
                            micro_blk[i] = seed_blk_px;
                            q_blk.push_idx(i, seed_blk_px);
                            let mi = (ly * buf.sz + lz) * buf.sx + (buf.sx - 1);
                            bs_set(&mut macro_touched, mi);
                        }
                        if seed_sky_px > 0
                            && !bs_get(&micro_solid_bits, midx(mxs - 1, my, mz, mxs, mzs))
                            && micro_sky[i] < seed_sky_px
                        {
                            micro_sky[i] = seed_sky_px;
                            q_sky.push_idx(i, seed_sky_px);
                            let mi = (ly * buf.sz + lz) * buf.sx + (buf.sx - 1);
                            bs_set(&mut macro_touched, mi);
                        }
                    }
                }
            }
        }
    }
    // Z seams (block + sky) with macro-first loops and cached 2x2 gates
    for ly in 0..buf.sy {
        for lx in 0..buf.sx {
            if !(use_zn || use_zp) {
                continue;
            }
            let here_nz = buf.get_local(lx, ly, 0);
            let here_pz = buf.get_local(lx, ly, buf.sz - 1);
            let have_micro_nz = nbm.zm_bl_neg.is_some() || nbm.zm_sk_neg.is_some();
            let have_micro_pz = nbm.zm_bl_pos.is_some() || nbm.zm_sk_pos.is_some();
            // Per-line micro seed precheck (2x2 micro offsets)
            let mut mic_line_zn = false;
            if have_micro_nz {
                for ixm in 0..2 {
                    for iym in 0..2 {
                        let mx = (lx << 1) | ixm;
                        let my = (ly << 1) | iym;
                        let off = my * mxs + mx;
                        let sblk = nbm
                            .zm_bl_neg
                            .as_ref()
                            .map(|p| clamp_sub_u8(p[off], MICRO_BLOCK_ATTENUATION))
                            .unwrap_or(0);
                        let ssky = nbm
                            .zm_sk_neg
                            .as_ref()
                            .map(|p| clamp_sub_u8(p[off], MICRO_SKY_ATTENUATION))
                            .unwrap_or(0);
                        if sblk > 0 || ssky > 0 {
                            mic_line_zn = true;
                            break;
                        }
                    }
                    if mic_line_zn {
                        break;
                    }
                }
            }
            let mut mic_line_zp = false;
            if have_micro_pz {
                for ixm in 0..2 {
                    for iym in 0..2 {
                        let mx = (lx << 1) | ixm;
                        let my = (ly << 1) | iym;
                        let off = my * mxs + mx;
                        let sblk = nbm
                            .zm_bl_pos
                            .as_ref()
                            .map(|p| clamp_sub_u8(p[off], MICRO_BLOCK_ATTENUATION))
                            .unwrap_or(0);
                        let ssky = nbm
                            .zm_sk_pos
                            .as_ref()
                            .map(|p| clamp_sub_u8(p[off], MICRO_SKY_ATTENUATION))
                            .unwrap_or(0);
                        if sblk > 0 || ssky > 0 {
                            mic_line_zp = true;
                            break;
                        }
                    }
                    if mic_line_zp {
                        break;
                    }
                }
            }
            // Line-level coarse seeds for this line
            let mut coarse_zn = false;
            if let Some(ref p) = nb.zn {
                coarse_zn |= clamp_sub_u8(p[ly * buf.sx + lx], atten) > 0;
            }
            if let Some(ref p) = nb.sk_zn {
                coarse_zn |= clamp_sub_u8(p[ly * buf.sx + lx], atten) > 0;
            }
            let mut coarse_zp = false;
            if let Some(ref p) = nb.zp {
                coarse_zp |= clamp_sub_u8(p[ly * buf.sx + lx], atten) > 0;
            }
            if let Some(ref p) = nb.sk_zp {
                coarse_zp |= clamp_sub_u8(p[ly * buf.sx + lx], atten) > 0;
            }
            let mut do_zn = mic_line_zn || coarse_zn;
            let mut do_zp = mic_line_zp || coarse_zp;
            if !do_zn && !do_zp {
                continue;
            }
            // Only fetch neighbor blocks for coarse fallback
            let (there_nz, there_pz) = if (!have_micro_nz && do_zn) || (!have_micro_pz && do_zp) {
                (
                    world.block_at_runtime_with(
                        reg,
                        &mut reuse_ctx,
                        base_x + lx as i32,
                        base_y + ly as i32,
                        base_z - 1,
                    ),
                    world.block_at_runtime_with(
                        reg,
                        &mut reuse_ctx,
                        base_x + lx as i32,
                        base_y + ly as i32,
                        base_z + buf.sz as i32,
                    ),
                )
            } else {
                (here_nz, here_pz)
            };
            let mut gate_nz = [[false; 2]; 2];
            let mut gate_pz = [[false; 2]; 2];
            for ixm in 0..2 {
                for iym in 0..2 {
                    gate_nz[iym][ixm] = if do_zn && have_micro_nz {
                        let mx = (lx << 1) | ixm;
                        let my = (ly << 1) | iym;
                        !bs_get(&micro_solid_bits, midx(mx, my, 0, mxs, mzs))
                    } else if do_zn {
                        micro_face_cell_open_s2(reg, here_nz, there_nz, 5, ixm, iym)
                    } else {
                        false
                    };
                    gate_pz[iym][ixm] = if do_zp && have_micro_pz {
                        let mx = (lx << 1) | ixm;
                        let my = (ly << 1) | iym;
                        !bs_get(&micro_solid_bits, midx(mx, my, mzs - 1, mxs, mzs))
                    } else if do_zp {
                        micro_face_cell_open_s2(reg, here_pz, there_pz, 4, ixm, iym)
                    } else {
                        false
                    };
                }
            }
            // Extra pruning: if all gates closed, skip side
            if do_zn {
                let mut any = false;
                for iym in 0..2 {
                    for ixm in 0..2 {
                        any |= gate_nz[iym][ixm];
                    }
                }
                if !any {
                    do_zn = false;
                }
            }
            if do_zp {
                let mut any = false;
                for iym in 0..2 {
                    for ixm in 0..2 {
                        any |= gate_pz[iym][ixm];
                    }
                }
                if !any {
                    do_zp = false;
                }
            }
            if !do_zn && !do_zp {
                continue;
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
                    if do_zn && (seed_blk_nz > 0 || seed_sky_nz > 0) && gate_nz[iym][ixm] {
                        let i = midx(mx, my, 0, mxs, mzs);
                        if seed_blk_nz > 0
                            && !bs_get(&micro_solid_bits, midx(mx, my, 0, mxs, mzs))
                            && micro_blk[i] < seed_blk_nz
                        {
                            micro_blk[i] = seed_blk_nz;
                            q_blk.push_idx(i, seed_blk_nz);
                            let mi = (ly * buf.sz + 0) * buf.sx + lx;
                            bs_set(&mut macro_touched, mi);
                        }
                        if seed_sky_nz > 0
                            && !bs_get(&micro_solid_bits, midx(mx, my, 0, mxs, mzs))
                            && micro_sky[i] < seed_sky_nz
                        {
                            micro_sky[i] = seed_sky_nz;
                            q_sky.push_idx(i, seed_sky_nz);
                            let mi = (ly * buf.sz + 0) * buf.sx + lx;
                            bs_set(&mut macro_touched, mi);
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
                    if do_zp && (seed_blk_pz > 0 || seed_sky_pz > 0) && gate_pz[iym][ixm] {
                        let i = midx(mx, my, mzs - 1, mxs, mzs);
                        if seed_blk_pz > 0
                            && !bs_get(&micro_solid_bits, midx(mx, my, mzs - 1, mxs, mzs))
                            && micro_blk[i] < seed_blk_pz
                        {
                            micro_blk[i] = seed_blk_pz;
                            q_blk.push_idx(i, seed_blk_pz);
                            let mi = (ly * buf.sz + (buf.sz - 1)) * buf.sx + lx;
                            bs_set(&mut macro_touched, mi);
                        }
                        if seed_sky_pz > 0
                            && !bs_get(&micro_solid_bits, midx(mx, my, mzs - 1, mxs, mzs))
                            && micro_sky[i] < seed_sky_pz
                        {
                            micro_sky[i] = seed_sky_pz;
                            q_sky.push_idx(i, seed_sky_pz);
                            let mi = (ly * buf.sz + (buf.sz - 1)) * buf.sx + lx;
                            bs_set(&mut macro_touched, mi);
                        }
                    }
                }
            }
        }
    }

    // Y seams (block + sky)
    for lx in 0..buf.sx {
        for lz in 0..buf.sz {
            if !(use_yn || use_yp) {
                continue;
            }
            let here_ny = buf.get_local(lx, 0, lz);
            let here_py = buf.get_local(lx, buf.sy - 1, lz);
            let have_micro_ny = nbm.ym_bl_neg.is_some() || nbm.ym_sk_neg.is_some();
            let have_micro_py = nbm.ym_bl_pos.is_some() || nbm.ym_sk_pos.is_some();
            let mut mic_column_ny = false;
            if have_micro_ny {
                for ixm in 0..2 {
                    for izm in 0..2 {
                        let mx = (lx << 1) | ixm;
                        let mz = (lz << 1) | izm;
                        let off = mz * mxs + mx;
                        let sblk = nbm
                            .ym_bl_neg
                            .as_ref()
                            .map(|p| clamp_sub_u8(p[off], MICRO_BLOCK_ATTENUATION))
                            .unwrap_or(0);
                        let ssky = nbm
                            .ym_sk_neg
                            .as_ref()
                            .map(|p| clamp_sub_u8(p[off], MICRO_SKY_ATTENUATION))
                            .unwrap_or(0);
                        if sblk > 0 || ssky > 0 {
                            mic_column_ny = true;
                            break;
                        }
                    }
                    if mic_column_ny {
                        break;
                    }
                }
            }
            let mut mic_column_py = false;
            if have_micro_py {
                for ixm in 0..2 {
                    for izm in 0..2 {
                        let mx = (lx << 1) | ixm;
                        let mz = (lz << 1) | izm;
                        let off = mz * mxs + mx;
                        let sblk = nbm
                            .ym_bl_pos
                            .as_ref()
                            .map(|p| clamp_sub_u8(p[off], MICRO_BLOCK_ATTENUATION))
                            .unwrap_or(0);
                        let ssky = nbm
                            .ym_sk_pos
                            .as_ref()
                            .map(|p| clamp_sub_u8(p[off], MICRO_SKY_ATTENUATION))
                            .unwrap_or(0);
                        if sblk > 0 || ssky > 0 {
                            mic_column_py = true;
                            break;
                        }
                    }
                    if mic_column_py {
                        break;
                    }
                }
            }
            let mut coarse_yn = false;
            if let Some(ref p) = nb.yn {
                coarse_yn |= clamp_sub_u8(p[lz * buf.sx + lx], atten) > 0;
            }
            if let Some(ref p) = nb.sk_yn {
                coarse_yn |= clamp_sub_u8(p[lz * buf.sx + lx], atten) > 0;
            }
            let mut coarse_yp = false;
            if let Some(ref p) = nb.yp {
                coarse_yp |= clamp_sub_u8(p[lz * buf.sx + lx], atten) > 0;
            }
            if let Some(ref p) = nb.sk_yp {
                coarse_yp |= clamp_sub_u8(p[lz * buf.sx + lx], atten) > 0;
            }
            let mut do_yn = mic_column_ny || coarse_yn;
            let mut do_yp = mic_column_py || coarse_yp;
            if !do_yn && !do_yp {
                continue;
            }
            let (there_ny, there_py) = if (!have_micro_ny && do_yn) || (!have_micro_py && do_yp) {
                (
                    world.block_at_runtime_with(
                        reg,
                        &mut reuse_ctx,
                        base_x + lx as i32,
                        base_y - 1,
                        base_z + lz as i32,
                    ),
                    world.block_at_runtime_with(
                        reg,
                        &mut reuse_ctx,
                        base_x + lx as i32,
                        base_y + buf.sy as i32,
                        base_z + lz as i32,
                    ),
                )
            } else {
                (here_ny, here_py)
            };
            let mut gate_ny = [[false; 2]; 2];
            let mut gate_py = [[false; 2]; 2];
            for ixm in 0..2 {
                for izm in 0..2 {
                    gate_ny[ixm][izm] = if do_yn && have_micro_ny {
                        let mx = (lx << 1) | ixm;
                        let mz = (lz << 1) | izm;
                        !bs_get(&micro_solid_bits, midx(mx, 0, mz, mxs, mzs))
                    } else if do_yn {
                        micro_face_cell_open_s2(reg, here_ny, there_ny, 1, ixm, izm)
                    } else {
                        false
                    };
                    gate_py[ixm][izm] = if do_yp && have_micro_py {
                        let mx = (lx << 1) | ixm;
                        let mz = (lz << 1) | izm;
                        !bs_get(&micro_solid_bits, midx(mx, mys - 1, mz, mxs, mzs))
                    } else if do_yp {
                        micro_face_cell_open_s2(reg, here_py, there_py, 0, ixm, izm)
                    } else {
                        false
                    };
                }
            }
            if do_yn {
                let mut any = false;
                for ixm in 0..2 {
                    for izm in 0..2 {
                        any |= gate_ny[ixm][izm];
                    }
                }
                if !any {
                    do_yn = false;
                }
            }
            if do_yp {
                let mut any = false;
                for ixm in 0..2 {
                    for izm in 0..2 {
                        any |= gate_py[ixm][izm];
                    }
                }
                if !any {
                    do_yp = false;
                }
            }
            if !do_yn && !do_yp {
                continue;
            }
            for ixm in 0..2 {
                for izm in 0..2 {
                    let mx = (lx << 1) | ixm;
                    let mz = (lz << 1) | izm;
                    // -Y
                    let seed_blk_ny = nbm
                        .ym_bl_neg
                        .as_ref()
                        .map(|p| clamp_sub_u8(p[mz * mxs + mx], MICRO_BLOCK_ATTENUATION))
                        .unwrap_or_else(|| {
                            nb.yn
                                .as_ref()
                                .map(|p| clamp_sub_u8(p[lz * buf.sx + lx], atten))
                                .unwrap_or(0)
                        });
                    let seed_sky_ny = nbm
                        .ym_sk_neg
                        .as_ref()
                        .map(|p| clamp_sub_u8(p[mz * mxs + mx], MICRO_SKY_ATTENUATION))
                        .unwrap_or_else(|| {
                            nb.sk_yn
                                .as_ref()
                                .map(|p| clamp_sub_u8(p[lz * buf.sx + lx], atten))
                                .unwrap_or(0)
                        });
                    if do_yn && (seed_blk_ny > 0 || seed_sky_ny > 0) && gate_ny[ixm][izm] {
                        let i = midx(mx, 0, mz, mxs, mzs);
                        if seed_blk_ny > 0
                            && !bs_get(&micro_solid_bits, midx(mx, 0, mz, mxs, mzs))
                            && micro_blk[i] < seed_blk_ny
                        {
                            micro_blk[i] = seed_blk_ny;
                            q_blk.push_idx(i, seed_blk_ny);
                            let mi = (0 * buf.sz + lz) * buf.sx + lx;
                            bs_set(&mut macro_touched, mi);
                        }
                        if seed_sky_ny > 0
                            && !bs_get(&micro_solid_bits, midx(mx, 0, mz, mxs, mzs))
                            && micro_sky[i] < seed_sky_ny
                        {
                            micro_sky[i] = seed_sky_ny;
                            q_sky.push_idx(i, seed_sky_ny);
                            let mi = (0 * buf.sz + lz) * buf.sx + lx;
                            bs_set(&mut macro_touched, mi);
                        }
                    }
                    // +Y
                    let seed_blk_py = nbm
                        .ym_bl_pos
                        .as_ref()
                        .map(|p| clamp_sub_u8(p[mz * mxs + mx], MICRO_BLOCK_ATTENUATION))
                        .unwrap_or_else(|| {
                            nb.yp
                                .as_ref()
                                .map(|p| clamp_sub_u8(p[lz * buf.sx + lx], atten))
                                .unwrap_or(0)
                        });
                    let seed_sky_py = nbm
                        .ym_sk_pos
                        .as_ref()
                        .map(|p| clamp_sub_u8(p[mz * mxs + mx], MICRO_SKY_ATTENUATION))
                        .unwrap_or_else(|| {
                            nb.sk_yp
                                .as_ref()
                                .map(|p| clamp_sub_u8(p[lz * buf.sx + lx], atten))
                                .unwrap_or(0)
                        });
                    if do_yp && (seed_blk_py > 0 || seed_sky_py > 0) && gate_py[ixm][izm] {
                        let i = midx(mx, mys - 1, mz, mxs, mzs);
                        if seed_blk_py > 0
                            && !bs_get(&micro_solid_bits, midx(mx, mys - 1, mz, mxs, mzs))
                            && micro_blk[i] < seed_blk_py
                        {
                            micro_blk[i] = seed_blk_py;
                            q_blk.push_idx(i, seed_blk_py);
                            let mi = ((buf.sy - 1) * buf.sz + lz) * buf.sx + lx;
                            bs_set(&mut macro_touched, mi);
                        }
                        if seed_sky_py > 0
                            && !bs_get(&micro_solid_bits, midx(mx, mys - 1, mz, mxs, mzs))
                            && micro_sky[i] < seed_sky_py
                        {
                            micro_sky[i] = seed_sky_py;
                            q_sky.push_idx(i, seed_sky_py);
                            let mi = ((buf.sy - 1) * buf.sz + lz) * buf.sx + lx;
                            bs_set(&mut macro_touched, mi);
                        }
                    }
                }
            }
        }
    }

    // Seed emissive blocks at micro resolution (fill interior air micro voxels of the macro cell)
    // A) Scan the chunk buffer for emissive blocks (covers worldgen + edits folded into buf)
    //    Seed at face boundaries so full-cube emitters (e.g., glowstone) light adjacent air.
    for z in 0..buf.sz {
        for y in 0..buf.sy {
            for x in 0..buf.sx {
                let b = buf.get_local(x, y, z);
                if let Some(ty) = reg.get(b.id) {
                    let level = ty.light_emission(b.state);
                    if level == 0 {
                        continue;
                    }
                    let bx = x * MICRO_SCALE;
                    let by = y * MICRO_SCALE;
                    let bz = z * MICRO_SCALE;
                    // Helper to write and enqueue a micro cell
                    let mut seed_idx = |ii: usize| {
                        if micro_blk[ii] < level {
                            micro_blk[ii] = level;
                            q_blk.push_idx(ii, level);
                            let mii = (y * buf.sz + z) * buf.sx + x;
                            bs_set(&mut macro_touched, mii);
                        }
                    };
                    // For each face, set the 2x2 micro cells on the emitter's face to `level` (even if solid),
                    // and set the adjacent outside micro cells when they exist and are not solid.
                    // +X face
                    for oy in 0..MICRO_SCALE {
                        for oz in 0..MICRO_SCALE {
                            let ii_in = midx(bx + 1, by + oy, bz + oz, mxs, mzs);
                            seed_idx(ii_in);
                            let mx_out = bx + 2;
                            if mx_out < mxs {
                                let ii_out = midx(mx_out, by + oy, bz + oz, mxs, mzs);
                                if !bs_get(&micro_solid_bits, ii_out) {
                                    seed_idx(ii_out);
                                }
                            }
                        }
                    }
                    // -X face
                    for oy in 0..MICRO_SCALE {
                        for oz in 0..MICRO_SCALE {
                            let ii_in = midx(bx + 0, by + oy, bz + oz, mxs, mzs);
                            seed_idx(ii_in);
                            if bx > 0 {
                                let ii_out = midx(bx - 1, by + oy, bz + oz, mxs, mzs);
                                if !bs_get(&micro_solid_bits, ii_out) {
                                    seed_idx(ii_out);
                                }
                            }
                        }
                    }
                    // +Y face
                    for oz in 0..MICRO_SCALE {
                        for ox in 0..MICRO_SCALE {
                            let ii_in = midx(bx + ox, by + 1, bz + oz, mxs, mzs);
                            seed_idx(ii_in);
                            let my_out = by + 2;
                            if my_out < mys {
                                let ii_out = midx(bx + ox, my_out, bz + oz, mxs, mzs);
                                if !bs_get(&micro_solid_bits, ii_out) {
                                    seed_idx(ii_out);
                                }
                            }
                        }
                    }
                    // -Y face
                    for oz in 0..MICRO_SCALE {
                        for ox in 0..MICRO_SCALE {
                            let ii_in = midx(bx + ox, by + 0, bz + oz, mxs, mzs);
                            seed_idx(ii_in);
                            if by > 0 {
                                let ii_out = midx(bx + ox, by - 1, bz + oz, mxs, mzs);
                                if !bs_get(&micro_solid_bits, ii_out) {
                                    seed_idx(ii_out);
                                }
                            }
                        }
                    }
                    // +Z face
                    for oy in 0..MICRO_SCALE {
                        for ox in 0..MICRO_SCALE {
                            let ii_in = midx(bx + ox, by + oy, bz + 1, mxs, mzs);
                            seed_idx(ii_in);
                            let mz_out = bz + 2;
                            if mz_out < mzs {
                                let ii_out = midx(bx + ox, by + oy, mz_out, mxs, mzs);
                                if !bs_get(&micro_solid_bits, ii_out) {
                                    seed_idx(ii_out);
                                }
                            }
                        }
                    }
                    // -Z face
                    for oy in 0..MICRO_SCALE {
                        for ox in 0..MICRO_SCALE {
                            let ii_in = midx(bx + ox, by + oy, bz + 0, mxs, mzs);
                            seed_idx(ii_in);
                            if bz > 0 {
                                let ii_out = midx(bx + ox, by + oy, bz - 1, mxs, mzs);
                                if !bs_get(&micro_solid_bits, ii_out) {
                                    seed_idx(ii_out);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // B) Overlay: also seed explicit runtime emitters from the store (e.g., interactive placements)
    // Treat beacons as omni seeds for now to ensure visible emission in Micro S=2.
    for (lx, ly, lz, level, _is_beacon) in store.emitters_for_chunk(buf.coord) {
        if level == 0 {
            continue;
        }
        let mx0 = lx * MICRO_SCALE;
        let my0 = ly * MICRO_SCALE;
        let mz0 = lz * MICRO_SCALE;
        for mx in mx0..(mx0 + MICRO_SCALE) {
            for my in my0..(my0 + MICRO_SCALE) {
                for mz in mz0..(mz0 + MICRO_SCALE) {
                    let ii = midx(mx, my, mz, mxs, mzs);
                    if !bs_get(&micro_solid_bits, ii) {
                        if micro_blk[ii] < level {
                            micro_blk[ii] = level;
                            q_blk.push_idx(ii, level);
                            let mii = (ly * buf.sz + lz) * buf.sx + lx;
                            bs_set(&mut macro_touched, mii);
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
    // (push helper removed in favor of parallel per-bucket processing)

    // BFS over block-light queue (parallel per-bucket)
    while q_blk.pending > 0 {
        let bi = (q_blk.cur_d & 15) as usize;
        let bucket = &mut q_blk.buckets[bi];
        // If current bucket empty, advance
        if bucket.read >= bucket.data.len() {
            bucket.reset_if_empty();
            q_blk.cur_d = q_blk.cur_d.wrapping_add(1);
            continue;
        }
        // Drain current frontier
        let frontier: Vec<(usize, u8)> = bucket.data[bucket.read..].to_vec();
        q_blk.pending -= frontier.len();
        bucket.read = bucket.data.len();
        bucket.reset_if_empty();

        // Parallel neighbor proposals
        let proposals: Vec<(usize, u8)> = frontier
            .par_iter()
            .fold(
                || Vec::new(),
                |mut out, &(idx0, level)| {
                    if level <= 1 {
                        return out;
                    }
                    // Skip stale entries
                    if micro_blk[idx0] != level {
                        return out;
                    }
                    let lvl = level;
                    let v = clamp_sub_u8(lvl, att_blk);
                    if v == 0 {
                        return out;
                    }
                    let my = idx0 / (mzs * mxs);
                    let rem = idx0 - my * (mzs * mxs);
                    let mz = rem / mxs;
                    let mx = rem - mz * mxs;
                    // +X
                    if mx + 1 < mxs {
                        let ii = idx0 + 1;
                        if micro_blk[ii] < v && !bs_get(&micro_solid_bits, ii) {
                            out.push((ii, v));
                        }
                    }
                    // -X
                    if mx > 0 {
                        let ii = idx0 - 1;
                        if micro_blk[ii] < v && !bs_get(&micro_solid_bits, ii) {
                            out.push((ii, v));
                        }
                    }
                    // +Y
                    if my + 1 < mys {
                        let ii = idx0 + stride_y_m;
                        if micro_blk[ii] < v && !bs_get(&micro_solid_bits, ii) {
                            out.push((ii, v));
                        }
                    }
                    // -Y
                    if my > 0 {
                        let ii = idx0 - stride_y_m;
                        if micro_blk[ii] < v && !bs_get(&micro_solid_bits, ii) {
                            out.push((ii, v));
                        }
                    }
                    // +Z
                    if mz + 1 < mzs {
                        let ii = idx0 + stride_z_m;
                        if micro_blk[ii] < v && !bs_get(&micro_solid_bits, ii) {
                            out.push((ii, v));
                        }
                    }
                    // -Z
                    if mz > 0 {
                        let ii = idx0 - stride_z_m;
                        if micro_blk[ii] < v && !bs_get(&micro_solid_bits, ii) {
                            out.push((ii, v));
                        }
                    }
                    out
                },
            )
            .reduce(
                || Vec::new(),
                |mut a, mut b| {
                    a.append(&mut b);
                    a
                },
            );

        // Merge proposals sequentially: apply updates and enqueue
        for (ii, v) in proposals {
            if v == 0 {
                continue;
            }
            if micro_blk[ii] < v && !bs_get(&micro_solid_bits, ii) {
                micro_blk[ii] = v;
                q_blk.push_idx(ii, v);
                // Mark macro cell as touched
                let my = ii / (mzs * mxs);
                let rem = ii - my * (mzs * mxs);
                let mz = rem / mxs;
                let mx = rem - mz * mxs;
                let mii = ((my >> 1) * buf.sz + (mz >> 1)) * buf.sx + (mx >> 1);
                bs_set(&mut macro_touched, mii);
            }
        }
    }

    // BFS over skylight queue (parallel per-bucket)
    while q_sky.pending > 0 {
        let bi = (q_sky.cur_d & 15) as usize;
        let bucket = &mut q_sky.buckets[bi];
        // If current bucket empty, advance
        if bucket.read >= bucket.data.len() {
            bucket.reset_if_empty();
            q_sky.cur_d = q_sky.cur_d.wrapping_add(1);
            continue;
        }
        // Drain current frontier
        let frontier: Vec<(usize, u8)> = bucket.data[bucket.read..].to_vec();
        q_sky.pending -= frontier.len();
        bucket.read = bucket.data.len();
        bucket.reset_if_empty();

        // Parallel neighbor proposals
        let proposals: Vec<(usize, u8)> = frontier
            .par_iter()
            .fold(
                || Vec::new(),
                |mut out, &(idx0, level)| {
                    if level <= 1 {
                        return out;
                    }
                    // Skip stale entries
                    if micro_sky[idx0] != level {
                        return out;
                    }
                    let lvl = level;
                    let v = clamp_sub_u8(lvl, att_sky);
                    if v == 0 {
                        return out;
                    }
                    let my = idx0 / (mzs * mxs);
                    let rem = idx0 - my * (mzs * mxs);
                    let mz = rem / mxs;
                    let mx = rem - mz * mxs;
                    // +X
                    if mx + 1 < mxs {
                        let ii = idx0 + 1;
                        if micro_sky[ii] < v && !bs_get(&micro_solid_bits, ii) {
                            out.push((ii, v));
                        }
                    }
                    // -X
                    if mx > 0 {
                        let ii = idx0 - 1;
                        if micro_sky[ii] < v && !bs_get(&micro_solid_bits, ii) {
                            out.push((ii, v));
                        }
                    }
                    // +Y
                    if my + 1 < mys {
                        let ii = idx0 + stride_y_m;
                        if micro_sky[ii] < v && !bs_get(&micro_solid_bits, ii) {
                            out.push((ii, v));
                        }
                    }
                    // -Y
                    if my > 0 {
                        let ii = idx0 - stride_y_m;
                        if micro_sky[ii] < v && !bs_get(&micro_solid_bits, ii) {
                            out.push((ii, v));
                        }
                    }
                    // +Z
                    if mz + 1 < mzs {
                        let ii = idx0 + stride_z_m;
                        if micro_sky[ii] < v && !bs_get(&micro_solid_bits, ii) {
                            out.push((ii, v));
                        }
                    }
                    // -Z
                    if mz > 0 {
                        let ii = idx0 - stride_z_m;
                        if micro_sky[ii] < v && !bs_get(&micro_solid_bits, ii) {
                            out.push((ii, v));
                        }
                    }
                    out
                },
            )
            .reduce(
                || Vec::new(),
                |mut a, mut b| {
                    a.append(&mut b);
                    a
                },
            );

        // Merge proposals sequentially: apply updates and enqueue
        for (ii, v) in proposals {
            if v == 0 {
                continue;
            }
            if micro_sky[ii] < v && !bs_get(&micro_solid_bits, ii) {
                micro_sky[ii] = v;
                q_sky.push_idx(ii, v);
                // Mark macro cell as touched
                let my = ii / (mzs * mxs);
                let rem = ii - my * (mzs * mxs);
                let mz = rem / mxs;
                let mx = rem - mz * mxs;
                let mii = ((my >> 1) * buf.sz + (mz >> 1)) * buf.sx + (mx >> 1);
                bs_set(&mut macro_touched, mii);
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
                let ii = ((y * buf.sz) + z) * buf.sx + x;
                // Downsample tightening: skip if macro cell never touched
                if !bs_get(&macro_touched, ii) {
                    lg.skylight[ii] = 0;
                    lg.block_light[ii] = 0;
                    continue;
                }
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
    let micro_mask = store.update_micro_borders(
        buf.coord,
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
    lg.micro_change = micro_mask;
    // Coarse planes are still derived by LightBorders::from_grid upstream.
    lg
}

// Scaffold for S=2 micro-voxel lighting engine.
// For now, this delegates to the legacy voxel light grid to keep behavior unchanged
// while wiring up mode toggling and rebuild plumbing. The full implementation will
// allocate a micro grid, run bucketed BFS at S=2, and produce border planes.

// (old scaffold removed; Micro S=2 implementation is above)

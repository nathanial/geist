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
    let stride_z_m = mxs; // +1 micro Z
    let stride_y_m = mxs * mzs; // +1 micro Y

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
    struct Bucket {
        data: Vec<(usize, u8)>,
        read: usize,
    }
    impl Bucket {
        #[inline]
        fn new() -> Self {
            Self { data: Vec::new(), read: 0 }
        }
        #[inline]
        fn push(&mut self, idx: usize, level: u8) {
            self.data.push((idx, level));
        }
        #[inline]
        fn pop_front(&mut self) -> Option<(usize, u8)> {
            if self.read < self.data.len() {
                let v = self.data[self.read];
                self.read += 1;
                Some(v)
            } else {
                None
            }
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
        #[inline]
        fn pop_idx(&mut self) -> Option<(usize, u8)> {
            if self.pending == 0 {
                return None;
            }
            loop {
                let bi = (self.cur_d & 15) as usize;
                if let Some(v) = self.buckets[bi].pop_front() {
                    self.pending -= 1;
                    return Some(v);
                }
                // advance to next bucket and reset the now-empty bucket for reuse
                self.buckets[bi].reset_if_empty();
                self.cur_d = self.cur_d.wrapping_add(1);
            }
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
            }
        }
    }
    // Phase 3: enqueue only boundary cells (open-above cells adjacent to a lateral neighbor that is NOT open-above at same Y)
    let mut neighbor_start = |mx: isize, mz: isize| -> usize {
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
    let nbm = store.get_neighbor_micro_borders(buf.cx, buf.cz);
    let nb = store.get_neighbor_borders(buf.cx, buf.cz);
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
    let atten: u8 = COARSE_SEAM_ATTENUATION;
    let base_x = buf.cx * buf.sx as i32;
    let base_z = buf.cz * buf.sz as i32;
    // Block light neighbors
    // Skylight neighbors: handled together with block after the coarse fallback expansion

    // Expanded implementation: X seams (block + sky) with macro-first loops and cached 2x2 gates
    // Avoid expensive world noise sampling when micro neighbor planes exist by gating using our
    // own micro occupancy only. When falling back to coarse neighbors, reuse a single GenCtx.
    let mut reuse_ctx = world.make_gen_ctx();
    for lz in 0..buf.sz {
        for ly in 0..buf.sy {
            if !(use_xn || use_xp) {
                continue;
            }
            let here_nx = buf.get_local(0, ly, lz);
            let here_px = buf.get_local(buf.sx - 1, ly, lz);
            let have_micro_nx = nbm.xm_bl_neg.is_some() || nbm.xm_sk_neg.is_some();
            let have_micro_px = nbm.xm_bl_pos.is_some() || nbm.xm_sk_pos.is_some();
            // Line-level coarse seeds (skip side if no micro and no coarse seeds on this line)
            let mut coarse_xn = false;
            if let Some(ref p) = nb.xn { coarse_xn |= clamp_sub_u8(p[ly * buf.sz + lz], atten) > 0; }
            if let Some(ref p) = nb.sk_xn { coarse_xn |= clamp_sub_u8(p[ly * buf.sz + lz], atten) > 0; }
            let mut coarse_xp = false;
            if let Some(ref p) = nb.xp { coarse_xp |= clamp_sub_u8(p[ly * buf.sz + lz], atten) > 0; }
            if let Some(ref p) = nb.sk_xp { coarse_xp |= clamp_sub_u8(p[ly * buf.sz + lz], atten) > 0; }
            let do_xn = have_micro_nx || coarse_xn;
            let do_xp = have_micro_px || coarse_xp;
            if !do_xn && !do_xp { continue; }
            // Only fetch neighbor blocks when we need coarse fallback gating
            let (there_nx, there_px) = if (!have_micro_nx && do_xn) || (!have_micro_px && do_xp) {
                (
                    world.block_at_runtime_with(reg, &mut reuse_ctx, base_x - 1, ly as i32, base_z + lz as i32),
                    world.block_at_runtime_with(
                        reg,
                        &mut reuse_ctx,
                        base_x + buf.sx as i32,
                        ly as i32,
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
                        }
                        if seed_sky_nx > 0
                            && !bs_get(&micro_solid_bits, midx(0, my, mz, mxs, mzs))
                            && micro_sky[i] < seed_sky_nx
                        {
                            micro_sky[i] = seed_sky_nx;
                            q_sky.push_idx(i, seed_sky_nx);
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
                        }
                        if seed_sky_px > 0
                            && !bs_get(&micro_solid_bits, midx(mxs - 1, my, mz, mxs, mzs))
                            && micro_sky[i] < seed_sky_px
                        {
                            micro_sky[i] = seed_sky_px;
                            q_sky.push_idx(i, seed_sky_px);
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
            // Line-level coarse seeds for this line
            let mut coarse_zn = false;
            if let Some(ref p) = nb.zn { coarse_zn |= clamp_sub_u8(p[ly * buf.sx + lx], atten) > 0; }
            if let Some(ref p) = nb.sk_zn { coarse_zn |= clamp_sub_u8(p[ly * buf.sx + lx], atten) > 0; }
            let mut coarse_zp = false;
            if let Some(ref p) = nb.zp { coarse_zp |= clamp_sub_u8(p[ly * buf.sx + lx], atten) > 0; }
            if let Some(ref p) = nb.sk_zp { coarse_zp |= clamp_sub_u8(p[ly * buf.sx + lx], atten) > 0; }
            let do_zn = have_micro_nz || coarse_zn;
            let do_zp = have_micro_pz || coarse_zp;
            if !do_zn && !do_zp { continue; }
            // Only fetch neighbor blocks for coarse fallback
            let (there_nz, there_pz) = if (!have_micro_nz && do_zn) || (!have_micro_pz && do_zp) {
                (
                    world.block_at_runtime_with(reg, &mut reuse_ctx, base_x + lx as i32, ly as i32, base_z - 1),
                    world.block_at_runtime_with(
                        reg,
                        &mut reuse_ctx,
                        base_x + lx as i32,
                        ly as i32,
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
                        }
                        if seed_sky_nz > 0
                            && !bs_get(&micro_solid_bits, midx(mx, my, 0, mxs, mzs))
                            && micro_sky[i] < seed_sky_nz
                        {
                            micro_sky[i] = seed_sky_nz;
                            q_sky.push_idx(i, seed_sky_nz);
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
                        }
                        if seed_sky_pz > 0
                            && !bs_get(&micro_solid_bits, midx(mx, my, mzs - 1, mxs, mzs))
                            && micro_sky[i] < seed_sky_pz
                        {
                            micro_sky[i] = seed_sky_pz;
                            q_sky.push_idx(i, seed_sky_pz);
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
                    if !bs_get(&micro_solid_bits, midx(mx, my, mz, mxs, mzs)) {
                        let i = midx(mx, my, mz, mxs, mzs);
                        if micro_blk[i] < level {
                            micro_blk[i] = level;
                            q_blk.push_idx(i, level);
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
        let v = clamp_sub_u8(lvl, att);
        if v == 0 {
            return;
        }
        let i = midx(mxu, myu, mzu, mxs, mzs);
        if arr[i] >= v {
            return;
        }
        if bs_get(&micro_solid_bits, i) {
            return;
        }
        if arr[i] < v {
            arr[i] = v;
        }
    };

    // BFS over block-light queue
    while let Some((idx0, level)) = q_blk.pop_idx() {
        if level <= 1 {
            continue;
        }
        let lvl = level;
        let my = idx0 / (mzs * mxs);
        let rem = idx0 - my * (mzs * mxs);
        let mz = rem / mxs;
        let mx = rem - mz * mxs;
        let (mx_i, my_i, mz_i) = (mx as i32, my as i32, mz as i32);
        let before = micro_blk[idx0];
        if before != lvl {
            continue;
        }
        push(mx_i + 1, my_i, mz_i, mxs, mys, mzs, &mut micro_blk, lvl, att_blk);
        push(mx_i - 1, my_i, mz_i, mxs, mys, mzs, &mut micro_blk, lvl, att_blk);
        push(mx_i, my_i + 1, mz_i, mxs, mys, mzs, &mut micro_blk, lvl, att_blk);
        push(mx_i, my_i - 1, mz_i, mxs, mys, mzs, &mut micro_blk, lvl, att_blk);
        push(mx_i, my_i, mz_i + 1, mxs, mys, mzs, &mut micro_blk, lvl, att_blk);
        push(mx_i, my_i, mz_i - 1, mxs, mys, mzs, &mut micro_blk, lvl, att_blk);
        // enqueue neighbors that we updated (index-based to avoid midx)
        if mx + 1 < mxs {
            let ii = idx0 + 1;
            if (micro_blk[ii] as u16 + att_blk as u16) == (lvl as u16) {
                q_blk.push_idx(ii, micro_blk[ii]);
            }
        }
        if mx > 0 {
            let ii = idx0 - 1;
            if (micro_blk[ii] as u16 + att_blk as u16) == (lvl as u16) {
                q_blk.push_idx(ii, micro_blk[ii]);
            }
        }
        if my + 1 < mys {
            let ii = idx0 + stride_y_m;
            if (micro_blk[ii] as u16 + att_blk as u16) == (lvl as u16) {
                q_blk.push_idx(ii, micro_blk[ii]);
            }
        }
        if my > 0 {
            let ii = idx0 - stride_y_m;
            if (micro_blk[ii] as u16 + att_blk as u16) == (lvl as u16) {
                q_blk.push_idx(ii, micro_blk[ii]);
            }
        }
        if mz + 1 < mzs {
            let ii = idx0 + stride_z_m;
            if (micro_blk[ii] as u16 + att_blk as u16) == (lvl as u16) {
                q_blk.push_idx(ii, micro_blk[ii]);
            }
        }
        if mz > 0 {
            let ii = idx0 - stride_z_m;
            if (micro_blk[ii] as u16 + att_blk as u16) == (lvl as u16) {
                q_blk.push_idx(ii, micro_blk[ii]);
            }
        }
    }

    // BFS over skylight queue
    while let Some((idx0, level)) = q_sky.pop_idx() {
        if level <= 1 {
            continue;
        }
        let lvl = level;
        let my = idx0 / (mzs * mxs);
        let rem = idx0 - my * (mzs * mxs);
        let mz = rem / mxs;
        let mx = rem - mz * mxs;
        let (mx_i, my_i, mz_i) = (mx as i32, my as i32, mz as i32);
        let before = micro_sky[idx0];
        if before != lvl {
            continue;
        }
        push(mx_i + 1, my_i, mz_i, mxs, mys, mzs, &mut micro_sky, lvl, att_sky);
        push(mx_i - 1, my_i, mz_i, mxs, mys, mzs, &mut micro_sky, lvl, att_sky);
        push(mx_i, my_i + 1, mz_i, mxs, mys, mzs, &mut micro_sky, lvl, att_sky);
        push(mx_i, my_i - 1, mz_i, mxs, mys, mzs, &mut micro_sky, lvl, att_sky);
        push(mx_i, my_i, mz_i + 1, mxs, mys, mzs, &mut micro_sky, lvl, att_sky);
        push(mx_i, my_i, mz_i - 1, mxs, mys, mzs, &mut micro_sky, lvl, att_sky);
        // enqueue neighbors that we updated (index-based)
        if mx + 1 < mxs {
            let ii = idx0 + 1;
            if (micro_sky[ii] as u16 + att_sky as u16) == (lvl as u16) {
                q_sky.push_idx(ii, micro_sky[ii]);
            }
        }
        if mx > 0 {
            let ii = idx0 - 1;
            if (micro_sky[ii] as u16 + att_sky as u16) == (lvl as u16) {
                q_sky.push_idx(ii, micro_sky[ii]);
            }
        }
        if my + 1 < mys {
            let ii = idx0 + stride_y_m;
            if (micro_sky[ii] as u16 + att_sky as u16) == (lvl as u16) {
                q_sky.push_idx(ii, micro_sky[ii]);
            }
        }
        if my > 0 {
            let ii = idx0 - stride_y_m;
            if (micro_sky[ii] as u16 + att_sky as u16) == (lvl as u16) {
                q_sky.push_idx(ii, micro_sky[ii]);
            }
        }
        if mz + 1 < mzs {
            let ii = idx0 + stride_z_m;
            if (micro_sky[ii] as u16 + att_sky as u16) == (lvl as u16) {
                q_sky.push_idx(ii, micro_sky[ii]);
            }
        }
        if mz > 0 {
            let ii = idx0 - stride_z_m;
            if (micro_sky[ii] as u16 + att_sky as u16) == (lvl as u16) {
                q_sky.push_idx(ii, micro_sky[ii]);
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

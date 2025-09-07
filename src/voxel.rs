use fastnoise_lite::{FastNoiseLite, NoiseType};
use crate::blocks::{Block as RtBlock, BlockRegistry};

pub struct World {
    pub chunk_size_x: usize,
    pub chunk_size_y: usize,
    pub chunk_size_z: usize,
    pub chunks_x: usize,
    pub chunks_z: usize,
    pub seed: i32,
    pub mode: WorldGenMode,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorldGenMode {
    Normal,
    // An infinite flat slab of stone of given thickness from y=0 upwards
    Flat { thickness: i32 },
}

// Legacy enums removed; all worldgen is runtime-config driven.

impl World {
    pub fn new(
        chunks_x: usize,
        chunks_z: usize,
        chunk_size_x: usize,
        chunk_size_y: usize,
        chunk_size_z: usize,
        seed: i32,
        mode: WorldGenMode,
    ) -> Self {
        Self {
            chunk_size_x,
            chunk_size_y,
            chunk_size_z,
            chunks_x,
            chunks_z,
            seed,
            mode,
        }
    }

    #[inline]
    pub fn world_size_x(&self) -> usize {
        self.chunk_size_x * self.chunks_x
    }
    #[inline]
    pub fn world_size_y(&self) -> usize {
        self.chunk_size_y
    }
    #[inline]
    pub fn world_size_z(&self) -> usize {
        self.chunk_size_z * self.chunks_z
    }
}


// Optional reusable noise context for batch generation to avoid re-allocating per voxel
pub struct GenCtx {
    pub terrain: FastNoiseLite,
    pub warp: FastNoiseLite,
    pub tunnel: FastNoiseLite,
}

impl World {
    pub fn make_gen_ctx(&self) -> GenCtx {
        let mut terrain = FastNoiseLite::with_seed(self.seed);
        terrain.set_noise_type(Some(NoiseType::OpenSimplex2));
        terrain.set_frequency(Some(0.02));
        let mut warp = FastNoiseLite::with_seed((self.seed as i32 ^ 991_73) as i32);
        warp.set_noise_type(Some(NoiseType::OpenSimplex2));
        warp.set_frequency(Some(0.012));
        let mut tunnel = FastNoiseLite::with_seed((self.seed as i32 ^ 41_337) as i32);
        tunnel.set_noise_type(Some(NoiseType::OpenSimplex2));
        tunnel.set_frequency(Some(0.017));
        GenCtx { terrain, warp, tunnel }
    }

    // Runtime worldgen: generate blocks directly via registry (no legacy enums)
    pub fn block_at_runtime(&self, reg: &BlockRegistry, x: i32, y: i32, z: i32) -> RtBlock {
        // Fallback wrapper for occasional sampling; batch paths should use block_at_runtime_with
        let mut ctx = self.make_gen_ctx();
        self.block_at_runtime_with(reg, &mut ctx, x, y, z)
    }

    pub fn block_at_runtime_with(
        &self,
        reg: &BlockRegistry,
        ctx: &mut GenCtx,
        x: i32,
        y: i32,
        z: i32,
    ) -> RtBlock {
        // Out-of-bounds in Y -> air
        if y < 0 || y >= self.chunk_size_y as i32 {
            let id = reg.id_by_name("air").unwrap_or(0);
            return RtBlock { id, state: 0 };
        }
        // Flat world shortcut
        if let WorldGenMode::Flat { thickness } = self.mode {
            let name = if y < thickness { "stone" } else { "air" };
            let id = reg.id_by_name(name).unwrap_or(0);
            return RtBlock { id, state: 0 };
        }

        // Base terrain sampling using reusable noise
        let height_for = |wx: i32, wz: i32| {
            let h = ctx.terrain.get_noise_2d(wx as f32, wz as f32);
            let min_h = (self.chunk_size_y as f32 * 0.15) as i32;
            let max_h = (self.chunk_size_y as f32 * 0.7) as i32;
            let hh = ((h + 1.0) * 0.5 * (max_h - min_h) as f32) as i32 + min_h;
            hh.clamp(1, self.chunk_size_y as i32 - 1)
        };
        let height = height_for(x, z);

        // Choose surface/underground base block name
        let mut base = if y >= height {
            "air"
        } else if y == height - 1 {
            if height as f32 >= self.chunk_size_y as f32 * 0.62 {
                "snow"
            } else if height as f32 <= self.chunk_size_y as f32 * 0.2 {
                "sand"
            } else {
                "grass"
            }
        } else if y + 3 >= height {
            "dirt"
        } else {
            "stone"
        };

        // --- Cave carving with occasional glowstone ---
        if matches!(base, "stone" | "dirt" | "sand" | "snow" | "glowstone") {
            // Params
            const Y_SCALE: f32 = 1.6;
            const EPS_BASE: f32 = 0.04;
            const EPS_ADD: f32 = 0.08;
            const WARP_XY: f32 = 5.0;
            const WARP_Y: f32 = 2.5;
            const ROOM_CELL: f32 = 120.0;
            const ROOM_THR_BASE: f32 = 0.12;
            const ROOM_THR_ADD: f32 = 0.12;
            const SOIL_MIN: f32 = 3.5;
            const MIN_Y: f32 = 2.0;
            const GLOW_PROB: f32 = 0.0009;

            let h = height as f32;
            let wy = y as f32;
            let soil = h - wy;
            if soil > SOIL_MIN && wy > MIN_Y {
                let fractal3 = |n: &FastNoiseLite,
                                x: f32,
                                y: f32,
                                z: f32,
                                oct: i32,
                                persistence: f32,
                                lacunarity: f32,
                                scale: f32| {
                    let mut amp = 1.0_f32;
                    let mut freq = 1.0_f32 / scale.max(0.0001);
                    let mut sum = 0.0_f32;
                    let mut max_amp = 0.0_f32;
                    for _ in 0..oct.max(1) {
                        sum += n.get_noise_3d(x * freq, y * freq, z * freq) * amp;
                        max_amp += amp;
                        amp *= persistence;
                        freq *= lacunarity;
                    }
                    if max_amp > 0.0 { sum / max_amp } else { sum }
                };

                let uhash32 = |mut a: u32| -> u32 {
                    a ^= a >> 16;
                    a = a.wrapping_mul(0x7feb_352d);
                    a ^= a >> 15;
                    a = a.wrapping_mul(0x846c_a68b);
                    a ^= a >> 16;
                    a
                };
                let hash3 = |x: i32, y: i32, z: i32, seed: u32| -> u32 {
                    let ux = x as u32;
                    let uy = y as u32;
                    let uz = z as u32;
                    let mut h = seed ^ 0x9e37_79b9;
                    h ^= uhash32(ux.wrapping_add(0x85eb_ca6b));
                    h ^= uhash32(uy.wrapping_add(0xc2b2_ae35));
                    h ^= uhash32(uz.wrapping_add(0x27d4_eb2f));
                    uhash32(h)
                };
                let rand01_cell = |cx: i32, cy: i32, cz: i32, salt: u32| -> f32 {
                    let h = hash3(cx, cy, cz, salt);
                    (h & 0x00FF_FFFF) as f32 / 16_777_216.0
                };
                let worley3_f1_norm = |x: f32, y: f32, z: f32, cell: f32| -> f32 {
                    let cell = if cell <= 0.0001 { 1.0 } else { cell };
                    let px = x / cell;
                    let py = y / cell;
                    let pz = z / cell;
                    let ix = px.floor() as i32;
                    let iy = py.floor() as i32;
                    let iz = pz.floor() as i32;
                    let fx = px - ix as f32;
                    let fy = py - iy as f32;
                    let fz = pz - iz as f32;
                    let mut min_d2 = f32::INFINITY;
                    for dz in -1..=1 {
                        for dy in -1..=1 {
                            for dx in -1..=1 {
                                let cx = ix + dx;
                                let cy = iy + dy;
                                let cz = iz + dz;
                                let jx = rand01_cell(cx, cy, cz, (self.seed as u32) ^ 0x068b_c021);
                                let jy = rand01_cell(cx, cy, cz, (self.seed as u32) ^ 0x02e1_b213);
                                let jz = rand01_cell(cx, cy, cz, (self.seed as u32) ^ 0x0f1a_1234);
                                let dx = (dx as f32 + jx) - fx;
                                let dy = (dy as f32 + jy) - fy;
                                let dz = (dz as f32 + jz) - fz;
                                let d2 = dx * dx + dy * dy + dz * dz;
                                if d2 < min_d2 { min_d2 = d2; }
                            }
                        }
                    }
                    (min_d2.sqrt()).min(1.0)
                };

                let wx = x as f32;
                let wy = y as f32;
                let wz = z as f32;
                let wxw = fractal3(&ctx.warp, wx, wy, wz, 3, 0.6, 2.0, 220.0);
                let wyw = fractal3(&ctx.warp, wx + 133.7, wy + 71.3, wz - 19.1, 3, 0.6, 2.0, 220.0);
                let wzw = fractal3(&ctx.warp, wx - 54.2,  wy + 29.7, wz + 88.8, 3, 0.6, 2.0, 220.0);
                let xp = wx + wxw * WARP_XY;
                let yp = wy + wyw * WARP_Y;
                let zp = wz + wzw * WARP_XY;

                let tn = fractal3(&ctx.tunnel, xp, yp * Y_SCALE, zp, 4, 0.55, 2.0, 140.0);
                let mut depth = soil / (self.chunk_size_y as f32);
                if depth < 0.0 { depth = 0.0; }
                if depth > 1.0 { depth = 1.0; }
                let eps = EPS_BASE + EPS_ADD * depth;
                let carve_tn = tn.abs() < eps;
                let wn = worley3_f1_norm(xp, yp, zp, ROOM_CELL);
                let room_thr = ROOM_THR_BASE + ROOM_THR_ADD * depth;
                let carve_rm = wn < room_thr;

                if carve_tn || carve_rm {
                    // Check if near solid for glowstone sprinkle
                    let mut near_solid = false;
                    for (dx, dy, dz) in [(-1,0,0),(1,0,0),(0,-1,0),(0,1,0),(0,0,-1),(0,0,1)].iter() {
                        let nx = x + dx; let ny = y + dy; let nz = z + dz;
                        if ny < 0 || ny >= self.chunk_size_y as i32 { continue; }
                        let nh = height_for(nx, nz);
                        if ny >= nh { continue; }
                        // Re-evaluate carve at neighbor
                        let wxn = nx as f32;
                        let wyn = ny as f32;
                        let wzn = nz as f32;
                        let wxw_n = fractal3(&ctx.warp, wxn, wyn, wzn, 3, 0.6, 2.0, 220.0);
                        let wyw_n = fractal3(&ctx.warp, wxn + 133.7, wyn + 71.3, wzn - 19.1, 3, 0.6, 2.0, 220.0);
                        let wzw_n = fractal3(&ctx.warp, wxn - 54.2,  wyn + 29.7, wzn + 88.8, 3, 0.6, 2.0, 220.0);
                        let nxp = wxn + wxw_n * WARP_XY;
                        let nyp = wyn + wyw_n * WARP_Y;
                        let nzp = wzn + wzw_n * WARP_XY;
                        let tn_n = fractal3(&ctx.tunnel, nxp, nyp * Y_SCALE, nzp, 4, 0.55, 2.0, 140.0);
                        let nsoil = nh as f32 - wyn;
                        let mut n_depth = nsoil / (self.chunk_size_y as f32);
                        if n_depth < 0.0 { n_depth = 0.0; }
                        if n_depth > 1.0 { n_depth = 1.0; }
                        let eps_n = EPS_BASE + EPS_ADD * n_depth;
                        let wn_n = worley3_f1_norm(nxp, nyp, nzp, ROOM_CELL);
                        let room_thr_n = ROOM_THR_BASE + ROOM_THR_ADD * n_depth;
                        let neighbor_carved_air = (nsoil > SOIL_MIN && wyn > MIN_Y) && (tn_n.abs() < eps_n || wn_n < room_thr_n);
                        if !neighbor_carved_air { near_solid = true; break; }
                    }
                    let h3 = {
                        let mut a = ((self.seed as u32) ^ 0xC0FF_EE15) ^ 0x9e37_79b9;
                        let uhash32 = |mut v: u32| { v ^= v >> 16; v = v.wrapping_mul(0x7feb_352d); v ^= v >> 15; v = v.wrapping_mul(0x846c_a68b); v ^= v >> 16; v };
                        a ^= uhash32(x as u32);
                        a ^= uhash32(y as u32);
                        a ^= uhash32(z as u32);
                        a
                    };
                    let r = (h3 & 0x00FF_FFFF) as f32 / 16_777_216.0;
                    if near_solid && y < height - 2 && r < GLOW_PROB {
                        base = "glowstone";
                    } else {
                        base = "air";
                    }
                }
            }
        }

        // Tree placement
        const TREE_PROB: f32 = 0.02;
        const TRUNK_MIN: i32 = 4;
        const TRUNK_MAX: i32 = 6;
        const LEAF_R: i32 = 2;
        let hash2 = |ix: i32, iz: i32, seed: u32| -> u32 {
            let mut h = (ix as u32).wrapping_mul(0x85eb_ca6b)
                ^ (iz as u32).wrapping_mul(0xc2b2_ae35)
                ^ seed.wrapping_mul(0x27d4_eb2d);
            h ^= h >> 16;
            h = h.wrapping_mul(0x7feb_352d);
            h ^= h >> 15;
            h = h.wrapping_mul(0x846c_a68b);
            h ^= h >> 16;
            h
        };
        let rand01 = |ix: i32, iz: i32, salt: u32| -> f32 {
            let h = hash2(ix, iz, ((self.seed as u32) ^ salt).wrapping_add(0x9E37_79B9));
            ((h & 0x00FF_FFFF) as f32) / 16_777_216.0
        };
        // species selection via simple climate fields
        let pick_species = |tx: i32, tz: i32| -> &'static str {
            let t = rand01(tx, tz, 0xBEEF01);
            let m = rand01(tx, tz, 0xC0FFEE);
            if t < 0.22 && m > 0.65 { return "spruce"; }
            if t > 0.78 && m > 0.45 { return "jungle"; }
            if t > 0.75 && m < 0.32 { return "acacia"; }
            if t > 0.65 && m < 0.25 { return "dark_oak"; }
            if ((hash2(tx, tz, 0xDEAD_BEEF) >> 20) & 1) == 1 { "birch" } else { "oak" }
        };
        let trunk_at = |tx: i32, tz: i32| -> Option<(i32, i32, &'static str)> {
            let surf = height_for(tx, tz) - 1;
            let surf_block = if surf as f32 >= self.chunk_size_y as f32 * 0.62 {
                "snow"
            } else if surf as f32 <= self.chunk_size_y as f32 * 0.2 {
                "sand"
            } else {
                "grass"
            };
            if surf_block != "grass" { return None; }
            if rand01(tx, tz, 0xA53F9) >= TREE_PROB { return None; }
            let span = (TRUNK_MAX - TRUNK_MIN).max(0) as u32;
            let hsel = hash2(tx, tz, 0x51F0_A7) % (span + 1);
            let th = TRUNK_MIN + hsel as i32;
            if surf <= 2 || surf >= (self.chunk_size_y as i32 - 6) { return None; }
            let sp = pick_species(tx, tz);
            Some((surf, th, sp))
        };

        // Trunk or leaves overrides base at this column
        if let Some((surf, th, sp)) = trunk_at(x, z) {
            if y >= surf + 1 && y <= surf + th {
                base = match sp {
                    "oak" => "oak_log",
                    "birch" => "birch_log",
                    "spruce" => "spruce_log",
                    "jungle" => "jungle_log",
                    "acacia" => "acacia_log",
                    "dark_oak" => "dark_oak_log",
                    _ => "oak_log",
                };
            }
        }
        if base == "air" {
            for tx in (x - LEAF_R)..=(x + LEAF_R) {
                for tz in (z - LEAF_R)..=(z + LEAF_R) {
                    if let Some((surf, th, sp)) = trunk_at(tx, tz) {
                        let top_y = surf + th;
                        let dy = y - top_y;
                        if dy < -2 || dy > 2 { continue; }
                        let rad = if dy <= -2 || dy >= 2 { LEAF_R - 1 } else { LEAF_R };
                        let dx = x - tx; let dz = z - tz;
                        if dx == 0 && dz == 0 && dy >= 0 { continue; }
                        let man = dx.abs() + dz.abs();
                        let extra = if dy >= 1 { 0 } else { 1 };
                        if man <= rad + extra {
                            base = match sp {
                                "oak" => "oak_leaves",
                                "birch" => "birch_leaves",
                                "spruce" => "spruce_leaves",
                                "jungle" => "jungle_leaves",
                                "acacia" => "acacia_leaves",
                                "dark_oak" => "oak_leaves",
                                _ => "oak_leaves",
                            };
                            break;
                        }
                    }
                }
                if base.ends_with("_leaves") { break; }
            }
        }

        let id = reg.id_by_name(base).unwrap_or_else(|| reg.id_by_name("air").unwrap_or(0));
        RtBlock { id, state: 0 }
    }
}

impl World {
    #[inline]
    pub fn is_flat(&self) -> bool {
        matches!(self.mode, WorldGenMode::Flat { .. })
    }
}

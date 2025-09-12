use fastnoise_lite::{FastNoiseLite, NoiseType};
use std::sync::{Arc, RwLock};

use geist_blocks::registry::BlockRegistry;
use geist_blocks::types::Block as RtBlock;

use crate::worldgen::WorldGenParams;

#[derive(Clone, Debug)]
pub struct ShowcaseEntry {
    pub block: RtBlock,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct ShowcasePlacement {
    pub dx: i32,
    pub dz: i32,
    pub block: RtBlock,
    pub label: String,
}

// Build the list of showcase entries (blocks to place and their labels).
pub fn build_showcase_entries(reg: &BlockRegistry) -> Vec<ShowcaseEntry> {
    let mut out: Vec<ShowcaseEntry> = Vec::new();
    let air_id = reg.id_by_name("air").unwrap_or(0);
    for ty in &reg.blocks {
        if ty.id == air_id {
            continue;
        }
        if ty.name == "slab" {
            if let Some(mats) = ty.state_schema.get("material") {
                for m in mats {
                    let mut props = std::collections::HashMap::new();
                    props.insert("half".to_string(), "bottom".to_string());
                    props.insert("material".to_string(), m.clone());
                    let state = ty.pack_state(&props);
                    out.push(ShowcaseEntry {
                        block: RtBlock { id: ty.id, state },
                        label: format!("slab({},bottom)", m),
                    });
                    let mut props_top = std::collections::HashMap::new();
                    props_top.insert("half".to_string(), "top".to_string());
                    props_top.insert("material".to_string(), m.clone());
                    let state_top = ty.pack_state(&props_top);
                    out.push(ShowcaseEntry {
                        block: RtBlock {
                            id: ty.id,
                            state: state_top,
                        },
                        label: format!("slab({},top)", m),
                    });
                }
                continue;
            }
        } else if ty.name == "stairs" {
            if let Some(mats) = ty.state_schema.get("material") {
                for m in mats {
                    let mut props = std::collections::HashMap::new();
                    props.insert("half".to_string(), "bottom".to_string());
                    props.insert("facing".to_string(), "north".to_string());
                    props.insert("material".to_string(), m.clone());
                    let state = ty.pack_state(&props);
                    out.push(ShowcaseEntry {
                        block: RtBlock { id: ty.id, state },
                        label: format!("stairs({})", m),
                    });
                }
                continue;
            }
        }
        out.push(ShowcaseEntry {
            block: RtBlock {
                id: ty.id,
                state: 0,
            },
            label: ty.name.clone(),
        });
    }
    out
}

pub fn build_showcase_stairs_cluster(reg: &BlockRegistry) -> Vec<ShowcasePlacement> {
    let mut out: Vec<ShowcasePlacement> = Vec::new();
    let stairs = match reg.blocks.iter().find(|t| t.name == "stairs") {
        Some(t) => t,
        None => return out,
    };
    let mats = stairs
        .state_schema
        .get("material")
        .cloned()
        .unwrap_or_default();
    let material = if mats.iter().any(|m| m == "stone_bricks") {
        "stone_bricks"
    } else {
        mats.first().map(|s| s.as_str()).unwrap_or("smooth_stone")
    };
    let mut make = |half: &str, facing: &str| -> RtBlock {
        let mut props = std::collections::HashMap::new();
        props.insert("half".to_string(), half.to_string());
        props.insert("facing".to_string(), facing.to_string());
        props.insert("material".to_string(), material.to_string());
        let state = stairs.pack_state(&props);
        RtBlock {
            id: stairs.id,
            state,
        }
    };

    let mut x = 0i32;
    let dz0 = 0i32;
    for (f, lbl) in [("north", "N"), ("east", "E"), ("south", "S"), ("west", "W")] {
        out.push(ShowcasePlacement {
            dx: x,
            dz: dz0,
            block: make("bottom", f),
            label: format!("stairs({},bottom,{})", material, lbl),
        });
        x += 2;
    }
    x += 1;
    out.push(ShowcasePlacement {
        dx: x,
        dz: dz0,
        block: make("bottom", "east"),
        label: format!("stairs({},bottom,E)", material),
    });
    out.push(ShowcasePlacement {
        dx: x + 1,
        dz: dz0,
        block: make("bottom", "east"),
        label: format!("stairs({},bottom,E)", material),
    });
    x += 3;
    out.push(ShowcasePlacement {
        dx: x,
        dz: dz0,
        block: make("bottom", "east"),
        label: format!("stairs({},bottom,E)", material),
    });
    out.push(ShowcasePlacement {
        dx: x + 1,
        dz: dz0,
        block: make("top", "east"),
        label: format!("stairs({},top,E)", material),
    });
    x += 3;
    out.push(ShowcasePlacement {
        dx: x,
        dz: dz0,
        block: make("bottom", "east"),
        label: format!("stairs({},bottom,E)", material),
    });
    out.push(ShowcasePlacement {
        dx: x + 1,
        dz: dz0,
        block: make("bottom", "west"),
        label: format!("stairs({},bottom,W)", material),
    });
    x += 3;

    let dz1 = 1i32;
    out.push(ShowcasePlacement {
        dx: 0,
        dz: dz1,
        block: make("bottom", "south"),
        label: format!("stairs({},bottom,S)", material),
    });
    out.push(ShowcasePlacement {
        dx: 0,
        dz: dz1 + 1,
        block: make("bottom", "south"),
        label: format!("stairs({},bottom,S)", material),
    });
    out.push(ShowcasePlacement {
        dx: 2,
        dz: dz1,
        block: make("bottom", "south"),
        label: format!("stairs({},bottom,S)", material),
    });
    out.push(ShowcasePlacement {
        dx: 2,
        dz: dz1 + 1,
        block: make("top", "south"),
        label: format!("stairs({},top,S)", material),
    });
    out
}

pub struct World {
    pub chunk_size_x: usize,
    pub chunk_size_y: usize,
    pub chunk_size_z: usize,
    pub chunks_x: usize,
    pub chunks_z: usize,
    pub seed: i32,
    pub mode: WorldGenMode,
    pub gen_params: Arc<RwLock<WorldGenParams>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorldGenMode {
    Normal,
    Flat { thickness: i32 },
    Showcase,
}

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
            gen_params: Arc::new(RwLock::new(WorldGenParams::default())),
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

pub struct GenCtx {
    pub terrain: FastNoiseLite,
    pub warp: FastNoiseLite,
    pub tunnel: FastNoiseLite,
    pub params: WorldGenParams,
    pub temp2d: Option<FastNoiseLite>,
    pub moist2d: Option<FastNoiseLite>,
}

impl World {
    pub fn make_gen_ctx(&self) -> GenCtx {
        let params = { self.gen_params.read().unwrap().clone() };
        let mut terrain = FastNoiseLite::with_seed(self.seed);
        terrain.set_noise_type(Some(NoiseType::OpenSimplex2));
        terrain.set_frequency(Some(params.height_frequency));
        let mut warp = FastNoiseLite::with_seed(self.seed ^ 99_173);
        warp.set_noise_type(Some(NoiseType::OpenSimplex2));
        warp.set_frequency(Some(0.012));
        let mut tunnel = FastNoiseLite::with_seed(self.seed ^ 41_337);
        tunnel.set_noise_type(Some(NoiseType::OpenSimplex2));
        tunnel.set_frequency(Some(0.017));
        let (temp2d, moist2d) = if let Some(ref b) = params.biomes {
            let mut t = FastNoiseLite::with_seed(self.seed ^ 0x1203_5F31);
            t.set_noise_type(Some(NoiseType::OpenSimplex2));
            t.set_frequency(Some(b.temp_freq));
            let mut m = FastNoiseLite::with_seed(((self.seed as u32) ^ 0x92E3_A1B2u32) as i32);
            m.set_noise_type(Some(NoiseType::OpenSimplex2));
            m.set_frequency(Some(b.moisture_freq));
            (Some(t), Some(m))
        } else {
            (None, None)
        };
        GenCtx {
            terrain,
            warp,
            tunnel,
            params,
            temp2d,
            moist2d,
        }
    }

    pub fn block_at_runtime(&self, reg: &BlockRegistry, x: i32, y: i32, z: i32) -> RtBlock {
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
        if y < 0 || y >= self.chunk_size_y as i32 {
            let id = reg.id_by_name("air").unwrap_or(0);
            return RtBlock { id, state: 0 };
        }
        if let WorldGenMode::Showcase = self.mode {
            let air = RtBlock {
                id: reg.id_by_name("air").unwrap_or(0),
                state: 0,
            };
            let mut row_y = (self.chunk_size_y as f32 * ctx.params.platform_y_ratio
                + ctx.params.platform_y_offset)
                .round() as i32;
            row_y = row_y.clamp(1, self.chunk_size_y as i32 - 2);
            if y != row_y {
                return air;
            }
            let cz = (self.world_size_z() as i32) / 2;
            if z == cz {
                let entries = build_showcase_entries(reg);
                if entries.is_empty() {
                    return air;
                }
                let spacing = 2;
                let row_len = (entries.len() as i32) * spacing - 1;
                let cx = (self.world_size_x() as i32) / 2;
                let start_x = cx - row_len / 2;
                if x < start_x || x >= start_x + row_len {
                    return air;
                }
                let dx = x - start_x;
                if dx % spacing != 0 {
                    return air;
                }
                let idx = (dx / spacing) as usize;
                return entries.get(idx).map(|e| e.block).unwrap_or(air);
            }
            let stair_base_z = cz + 3;
            if z >= stair_base_z && z <= stair_base_z + 4 {
                let placements = build_showcase_stairs_cluster(reg);
                if placements.is_empty() {
                    return air;
                }
                let max_dx = placements.iter().map(|p| p.dx).max().unwrap_or(0);
                let cluster_w = max_dx + 1;
                let cx = (self.world_size_x() as i32) / 2;
                let start_x = cx - cluster_w / 2;
                let dx = x - start_x;
                if dx < 0 || dx >= cluster_w {
                    return air;
                }
                let dz = z - stair_base_z;
                for p in &placements {
                    if p.dx == dx && p.dz == dz {
                        return p.block;
                    }
                }
                return air;
            }
            return air;
        }
        if let WorldGenMode::Flat { thickness } = self.mode {
            let name = if y < thickness { "stone" } else { "air" };
            let id = reg.id_by_name(name).unwrap_or(0);
            return RtBlock { id, state: 0 };
        }

        // Base terrain sampling using reusable noise
        let height_for = |wx: i32, wz: i32| {
            let h = ctx.terrain.get_noise_2d(wx as f32, wz as f32);
            let min_h = (self.chunk_size_y as f32 * ctx.params.min_y_ratio) as i32;
            let max_h = (self.chunk_size_y as f32 * ctx.params.max_y_ratio) as i32;
            let hh = ((h + 1.0) * 0.5 * (max_h - min_h) as f32) as i32 + min_h;
            hh.clamp(1, self.chunk_size_y as i32 - 1)
        };
        let height = height_for(x, z);
        let water_level_y = if ctx.params.water_enable {
            (self.chunk_size_y as f32 * ctx.params.water_level_ratio).round() as i32
        } else {
            -1
        };

        // Biomes helpers
        let climate_for = |wx: i32, wz: i32| -> Option<(f32, f32)> {
            match (&ctx.temp2d, &ctx.moist2d) {
                (Some(t), Some(m)) => {
                    let b = ctx.params.biomes.as_ref().unwrap();
                    let sx = if b.scale_x == 0.0 { 1.0 } else { b.scale_x };
                    let sz = if b.scale_z == 0.0 { 1.0 } else { b.scale_z };
                    let x = wx as f32 * sx;
                    let z = wz as f32 * sz;
                    let tt = ((t.get_noise_2d(x, z) + 1.0) * 0.5).clamp(0.0, 1.0);
                    let mm = ((m.get_noise_2d(x, z) + 1.0) * 0.5).clamp(0.0, 1.0);
                    Some((tt, mm))
                }
                _ => None,
            }
        };
        let biome_for = |wx: i32, wz: i32| -> Option<&crate::worldgen::BiomeDefParam> {
            let b = ctx.params.biomes.as_ref()?;
            if b.debug_pack_all && !b.defs.is_empty() {
                let cell = b.debug_cell_size.max(1);
                let cx = (wx.div_euclid(cell)) as i64;
                let cz = (wz.div_euclid(cell)) as i64;
                let idx = ((cx * 31 + cz * 17).rem_euclid(b.defs.len() as i64)) as usize;
                if let Some(def) = b.defs.get(idx) {
                    return Some(def);
                }
            }
            let (t, m) = climate_for(wx, wz)?;
            for def in &b.defs {
                if t >= def.temp_min
                    && t < def.temp_max
                    && m >= def.moisture_min
                    && m < def.moisture_max
                {
                    return Some(def);
                }
            }
            None
        };
        let top_block_for_column = |wx: i32, wz: i32, hh: i32| -> &str {
            if hh as f32 >= self.chunk_size_y as f32 * ctx.params.snow_threshold {
                return &ctx.params.top_high;
            }
            if hh as f32 <= self.chunk_size_y as f32 * ctx.params.sand_threshold {
                return &ctx.params.top_low;
            }
            if let Some(def) = biome_for(wx, wz) {
                if let Some(ref tb) = def.top_block {
                    return tb.as_str();
                }
            }
            &ctx.params.top_mid
        };

        // Choose surface/underground base block name
        let mut base: &str = if y >= height {
            "air"
        } else if y == height - 1 {
            top_block_for_column(x, z, height)
        } else if y + ctx.params.topsoil_thickness >= height {
            &ctx.params.sub_near
        } else {
            &ctx.params.sub_deep
        };

        // Water fill: above terrain and below/at water level becomes water
        if base == "air" && ctx.params.water_enable && y <= water_level_y {
            base = "water";
        }

        // --- Cave carving ---
        let mut carved_here = false;
        if matches!(base, "stone" | "dirt" | "sand" | "snow" | "glowstone") {
            let y_scale: f32 = ctx.params.y_scale;
            let eps_base: f32 = ctx.params.eps_base;
            let eps_add: f32 = ctx.params.eps_add;
            let warp_xy: f32 = ctx.params.warp_xy;
            let warp_y: f32 = ctx.params.warp_y;
            let room_cell: f32 = ctx.params.room_cell;
            let room_thr_base: f32 = ctx.params.room_thr_base;
            let room_thr_add: f32 = ctx.params.room_thr_add;
            let soil_min: f32 = ctx.params.soil_min;
            let min_y: f32 = ctx.params.min_y;

            let h = height as f32;
            let wy = y as f32;
            let soil = h - wy;
            if ctx.params.carvers_enable && soil > soil_min && wy > min_y {
                let fractal3 =
                    |n: &FastNoiseLite, x: f32, y: f32, z: f32, f: &crate::worldgen::Fractal| {
                        let mut amp = 1.0_f32;
                        let mut freq = 1.0_f32 / f.scale.max(0.0001);
                        let mut sum = 0.0_f32;
                        let mut max_amp = 0.0_f32;
                        for _ in 0..f.octaves.max(1) {
                            sum += n.get_noise_3d(x * freq, y * freq, z * freq) * amp;
                            max_amp += amp;
                            amp *= f.persistence;
                            freq *= f.lacunarity;
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
                                if d2 < min_d2 {
                                    min_d2 = d2;
                                }
                            }
                        }
                    }
                    (min_d2.sqrt()).min(1.0)
                };
                // Warp space and evaluate tunnel + rooms for this voxel
                let wx = x as f32;
                let wyf = y as f32;
                let wz = z as f32;
                let wxw = fractal3(&ctx.warp, wx, wyf, wz, &ctx.params.warp);
                let wyw = fractal3(
                    &ctx.warp,
                    wx + 133.7,
                    wyf + 71.3,
                    wz - 19.1,
                    &ctx.params.warp,
                );
                let wzw = fractal3(
                    &ctx.warp,
                    wx - 54.2,
                    wyf + 29.7,
                    wz + 88.8,
                    &ctx.params.warp,
                );
                let xp = wx + wxw * warp_xy;
                let yp = wyf + wyw * warp_y;
                let zp = wz + wzw * warp_xy;
                let tn = fractal3(&ctx.tunnel, xp, yp * y_scale, zp, &ctx.params.tunnel);
                let mut depth01 = (soil / (self.chunk_size_y as f32)).clamp(0.0, 1.0);
                let eps = eps_base + eps_add * depth01;
                let wn = worley3_f1_norm(xp, yp, zp, room_cell);
                let room_thr = room_thr_base + room_thr_add * depth01;
                let carved_air = (tn.abs() < eps) || (wn < room_thr);
                if carved_air {
                    base = "air";
                    carved_here = true;
                }
                // Water fill for carved interior handled by global check above

                // Neighbor-solid check used by features
                let mut near_solid_cache: Option<bool> = None;
                let mut compute_near_solid = || -> bool {
                    if let Some(v) = near_solid_cache {
                        return v;
                    }
                    let mut near_solid = false;
                    for (dx, dy, dz) in [
                        (-1, 0, 0),
                        (1, 0, 0),
                        (0, -1, 0),
                        (0, 1, 0),
                        (0, 0, -1),
                        (0, 0, 1),
                    ]
                    .iter()
                    {
                        let nx = x + dx;
                        let ny = y + dy;
                        let nz = z + dz;
                        if ny < 0 || ny >= self.chunk_size_y as i32 {
                            continue;
                        }
                        let nh = height_for(nx, nz);
                        if ny >= nh {
                            continue;
                        }
                        let wxn = nx as f32;
                        let wyn = ny as f32;
                        let wzn = nz as f32;
                        let wxw_n = fractal3(&ctx.warp, wxn, wyn, wzn, &ctx.params.warp);
                        let wyw_n = fractal3(
                            &ctx.warp,
                            wxn + 133.7,
                            wyn + 71.3,
                            wzn - 19.1,
                            &ctx.params.warp,
                        );
                        let wzw_n = fractal3(
                            &ctx.warp,
                            wxn - 54.2,
                            wyn + 29.7,
                            wzn + 88.8,
                            &ctx.params.warp,
                        );
                        let nxp = wxn + wxw_n * warp_xy;
                        let nyp = wyn + wyw_n * warp_y;
                        let nzp = wzn + wzw_n * warp_xy;
                        let tn_n =
                            fractal3(&ctx.tunnel, nxp, nyp * y_scale, nzp, &ctx.params.tunnel);
                        let nsoil = nh as f32 - wyn;
                        let mut n_depth = (nsoil / (self.chunk_size_y as f32)).clamp(0.0, 1.0);
                        let eps_n = eps_base + eps_add * n_depth;
                        let wn_n = worley3_f1_norm(nxp, nyp, nzp, room_cell);
                        let room_thr_n = room_thr_base + room_thr_add * n_depth;
                        let neighbor_carved_air = (nsoil > soil_min && wyn > min_y)
                            && (tn_n.abs() < eps_n || wn_n < room_thr_n);
                        if !neighbor_carved_air {
                            near_solid = true;
                            break;
                        }
                    }
                    near_solid_cache = Some(near_solid);
                    near_solid
                };

                // Feature placement
                if !ctx.params.features.is_empty() {
                    let hash3 = |x: i32, y: i32, z: i32, seed: u32| -> u32 {
                        let mut a = seed ^ 0x9e37_79b9;
                        let mix = |mut v: u32| {
                            v ^= v >> 16;
                            v = v.wrapping_mul(0x7feb_352d);
                            v ^= v >> 15;
                            v = v.wrapping_mul(0x846c_a68b);
                            v ^= v >> 16;
                            v
                        };
                        a ^= mix(x as u32);
                        a ^= mix(y as u32);
                        a ^= mix(z as u32);
                        a
                    };
                    for (ri, rule) in ctx.params.features.iter().enumerate() {
                        let w = &rule.when;
                        if !w.base_in.is_empty() && !w.base_in.iter().any(|s| s.as_str() == base) {
                            continue;
                        }
                        if !w.base_not_in.is_empty()
                            && w.base_not_in.iter().any(|s| s.as_str() == base)
                        {
                            continue;
                        }
                        if let Some(ymin) = w.y_min {
                            if y < ymin {
                                continue;
                            }
                        }
                        if let Some(ymax) = w.y_max {
                            if y > ymax {
                                continue;
                            }
                        }
                        if let Some(off) = w.below_height_offset {
                            if y >= height - off {
                                continue;
                            }
                        }
                        if let Some(req) = w.in_carved {
                            if req != carved_here {
                                continue;
                            }
                        }
                        if let Some(req) = w.near_solid {
                            if req != compute_near_solid() {
                                continue;
                            }
                        }
                        if let Some(p) = w.chance {
                            if p < 1.0 {
                                let salt = ((self.seed as u32).wrapping_add(0xC0FF_EE15))
                                    .wrapping_add(ri as u32 * 0x9E37_79B9);
                                let h = hash3(x, y, z, salt) & 0x00FF_FFFF;
                                let r = (h as f32) / 16_777_216.0;
                                if r >= p {
                                    continue;
                                }
                            }
                        }
                        base = &rule.place.block;
                        break;
                    }
                }
            }
        }

        // Tree placement (uses biome densities when available)
        let tree_prob_for = |wx: i32, wz: i32| -> f32 {
            if let Some(ref b) = ctx.params.biomes {
                if b.debug_pack_all && !b.defs.is_empty() {
                    let cell = b.debug_cell_size.max(1);
                    let cx = (wx.div_euclid(cell)) as i64;
                    let cz = (wz.div_euclid(cell)) as i64;
                    let idx = ((cx * 31 + cz * 17).rem_euclid(b.defs.len() as i64)) as usize;
                    if let Some(def) = b.defs.get(idx) {
                        if let Some(d) = def.tree_density {
                            return d.clamp(0.0, 1.0);
                        }
                    }
                } else if let (Some(tn), Some(mn)) = (&ctx.temp2d, &ctx.moist2d) {
                    let sx = if b.scale_x == 0.0 { 1.0 } else { b.scale_x };
                    let sz = if b.scale_z == 0.0 { 1.0 } else { b.scale_z };
                    let x = wx as f32 * sx;
                    let z = wz as f32 * sz;
                    let tt = ((tn.get_noise_2d(x, z) + 1.0) * 0.5).clamp(0.0, 1.0);
                    let mm = ((mn.get_noise_2d(x, z) + 1.0) * 0.5).clamp(0.0, 1.0);
                    for def in &b.defs {
                        if tt >= def.temp_min
                            && tt < def.temp_max
                            && mm >= def.moisture_min
                            && mm < def.moisture_max
                        {
                            if let Some(d) = def.tree_density {
                                return d.clamp(0.0, 1.0);
                            }
                            break;
                        }
                    }
                }
            }
            ctx.params.tree_probability
        };
        let tree_prob: f32 = tree_prob_for(x, z);
        let trunk_min: i32 = ctx.params.trunk_min;
        let trunk_max: i32 = ctx.params.trunk_max;
        let leaf_r: i32 = ctx.params.leaf_radius;
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
            let h = hash2(
                ix,
                iz,
                ((self.seed as u32) ^ salt).wrapping_add(0x9E37_79B9),
            );
            ((h & 0x00FF_FFFF) as f32) / 16_777_216.0
        };
        let pick_species = |tx: i32, tz: i32| -> &'static str {
            if let Some(def) = biome_for(tx, tz) {
                if !def.species_weights.is_empty() {
                    let mut total = 0.0_f32;
                    for w in def.species_weights.values() {
                        total += *w;
                    }
                    if total > 0.0 {
                        let r = rand01(tx, tz, 0xA11CE) * total;
                        let mut acc = 0.0_f32;
                        for (k, w) in &def.species_weights {
                            acc += *w;
                            if r <= acc {
                                let ks = k.as_str();
                                return match ks {
                                    "oak" => "oak",
                                    "birch" => "birch",
                                    "spruce" => "spruce",
                                    "jungle" => "jungle",
                                    "acacia" => "acacia",
                                    "dark_oak" => "dark_oak",
                                    _ => "oak",
                                };
                            }
                        }
                    }
                }
            }
            let t = rand01(tx, tz, 0xBEEF01);
            let m = rand01(tx, tz, 0xC0FFEE);
            if t < 0.22 && m > 0.65 {
                return "spruce";
            }
            if t > 0.78 && m > 0.45 {
                return "jungle";
            }
            if t > 0.75 && m < 0.32 {
                return "acacia";
            }
            if t > 0.65 && m < 0.25 {
                return "dark_oak";
            }
            if ((hash2(tx, tz, 0xDEAD_BEEF) >> 20) & 1) == 1 {
                "birch"
            } else {
                "oak"
            }
        };
        let trunk_at = |tx: i32, tz: i32| -> Option<(i32, i32, &'static str)> {
            let surf = height_for(tx, tz) - 1;
            let surf_block = top_block_for_column(tx, tz, surf + 1);
            if surf_block != "grass" {
                return None;
            }
            if rand01(tx, tz, 0xA53F9) >= tree_prob {
                return None;
            }
            let span = (trunk_max - trunk_min).max(0) as u32;
            let hsel = hash2(tx, tz, 0x0051_F0A7) % (span + 1);
            let th = trunk_min + hsel as i32;
            if surf <= 2 || surf >= (self.chunk_size_y as i32 - 6) {
                return None;
            }
            let sp = pick_species(tx, tz);
            Some((surf, th, sp))
        };

        if let Some((surf, th, sp)) = trunk_at(x, z) {
            if y > surf && y <= surf + th {
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
            for tx in (x - leaf_r)..=(x + leaf_r) {
                for tz in (z - leaf_r)..=(z + leaf_r) {
                    if let Some((surf, th, sp)) = trunk_at(tx, tz) {
                        let top_y = surf + th;
                        let dy = y - top_y;
                        if !(-2..=2).contains(&dy) {
                            continue;
                        }
                        let rad = if dy <= -2 || dy >= 2 {
                            leaf_r - 1
                        } else {
                            leaf_r
                        };
                        let dx = x - tx;
                        let dz = z - tz;
                        if dx == 0 && dz == 0 && dy >= 0 {
                            continue;
                        }
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
                if base.ends_with("_leaves") {
                    break;
                }
            }
        }

        let id = reg
            .id_by_name(base)
            .unwrap_or_else(|| reg.id_by_name("air").unwrap_or(0));
        RtBlock { id, state: 0 }
    }
}

impl World {
    pub fn update_worldgen_params(&self, params: WorldGenParams) {
        *self.gen_params.write().expect("worldgen params lock") = params;
    }

    #[inline]
    pub fn is_flat(&self) -> bool {
        matches!(self.mode, WorldGenMode::Flat { .. })
    }

    pub fn biome_at(&self, wx: i32, wz: i32) -> Option<crate::worldgen::BiomeDefParam> {
        let params = self.gen_params.read().ok()?.clone();
        let b = params.biomes?;
        // Recreate noise for temperature and moisture using the same seed scheme.
        let mut t = FastNoiseLite::with_seed(self.seed ^ 0x1203_5F31);
        t.set_noise_type(Some(NoiseType::OpenSimplex2));
        t.set_frequency(Some(b.temp_freq));
        let mut m = FastNoiseLite::with_seed(((self.seed as u32) ^ 0x92E3_A1B2u32) as i32);
        m.set_noise_type(Some(NoiseType::OpenSimplex2));
        m.set_frequency(Some(b.moisture_freq));
        let x = (wx as f32) / b.scale_x.max(1e-6);
        let z = (wz as f32) / b.scale_z.max(1e-6);
        let temp = (t.get_noise_2d(x, z) * 0.5 + 0.5).clamp(0.0, 1.0);
        let moist = (m.get_noise_2d(x, z) * 0.5 + 0.5).clamp(0.0, 1.0);
        for def in b.defs {
            if temp >= def.temp_min
                && temp <= def.temp_max
                && moist >= def.moisture_min
                && moist <= def.moisture_max
            {
                return Some(def);
            }
        }
        None
    }
}

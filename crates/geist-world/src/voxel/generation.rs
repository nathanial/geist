use std::sync::Arc;
use std::time::Instant;

use fastnoise_lite::FastNoiseLite;
use geist_blocks::registry::BlockRegistry;
use geist_blocks::types::Block as RtBlock;

use crate::worldgen::{Fractal, WorldGenParams};

use super::gen_ctx::{HeightTile, HeightTileStats, TerrainProfiler, TerrainStage};
use super::{GenCtx, World, WorldGenMode};

fn remap_noise_to_height(
    noise: f32,
    params: &WorldGenParams,
    world_height: i32,
    world_height_f: f32,
) -> i32 {
    let min_h = (world_height_f * params.min_y_ratio) as i32;
    let max_h = (world_height_f * params.max_y_ratio) as i32;
    let span = (max_h - min_h) as f32;
    let hh = ((noise + 1.0) * 0.5 * span) as i32 + min_h;
    hh.clamp(1, world_height - 1)
}

impl World {
    pub fn block_at_runtime(&self, reg: &BlockRegistry, x: i32, y: i32, z: i32) -> RtBlock {
        // PERF: This path constructs fresh noise generators; reuse `GenCtx` when sampling many voxels.
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
        ctx.terrain_profiler.begin_stage(TerrainStage::Block);
        let block_start = Instant::now();
        let air = self.air_block(reg);
        if y < 0 {
            ctx.terrain_profiler
                .record_stage_duration(TerrainStage::Block, block_start.elapsed());
            return air;
        }

        if let WorldGenMode::Flat { thickness } = self.mode {
            let name = if y < thickness { "stone" } else { "air" };
            let id = self.resolve_block_id(reg, name);
            ctx.terrain_profiler
                .record_stage_duration(TerrainStage::Block, block_start.elapsed());
            return RtBlock { id, state: 0 };
        }

        if let Some(block) = evaluate_tower(self, reg, &mut ctx.terrain_profiler, x, y, z, air) {
            ctx.terrain_profiler
                .record_stage_duration(TerrainStage::Block, block_start.elapsed());
            return block;
        }

        let params_guard: Arc<WorldGenParams> = Arc::clone(&ctx.params);
        let mut sampler = ColumnSampler::new(self, ctx, &params_guard);

        let height = sampler.height_for(x, z);
        let water_level = sampler.water_level();
        let mut base = select_surface_block(&mut sampler, x, y, z, height);
        apply_water_fill(&mut sampler, y, water_level, &mut base);
        let _ = apply_caves_and_features(self, &mut sampler, x, y, z, height, &mut base);
        apply_tree_blocks(self, &mut sampler, x, y, z, &mut base);

        let id = self.resolve_block_id(reg, base);
        ctx.terrain_profiler
            .record_stage_duration(TerrainStage::Block, block_start.elapsed());
        RtBlock { id, state: 0 }
    }

    pub fn prepare_height_tile(
        &self,
        ctx: &mut GenCtx,
        base_x: i32,
        base_z: i32,
        size_x: usize,
        size_z: usize,
    ) {
        if matches!(self.mode, WorldGenMode::Flat { .. }) {
            ctx.height_tile = None;
            ctx.height_tile_stats = HeightTileStats {
                duration_us: 0,
                columns: 0,
                reused: true,
            };
            return;
        }

        let total_columns = (size_x * size_z) as u32;
        if let Some(tile) = ctx.height_tile.as_ref() {
            if tile.matches(base_x, base_z, size_x, size_z) {
                ctx.height_tile_stats = HeightTileStats {
                    duration_us: 0,
                    columns: total_columns,
                    reused: true,
                };
                return;
            }
        }

        let params_guard = Arc::clone(&ctx.params);
        let params = &*params_guard;
        let world_height = self.world_height_hint() as i32;
        let world_height_f = world_height as f32;
        let mut heights = Vec::with_capacity(size_x * size_z);
        let t0 = Instant::now();
        for dz in 0..size_z {
            let wz = base_z + dz as i32;
            for dx in 0..size_x {
                let wx = base_x + dx as i32;
                let noise = ctx.terrain.get_noise_2d(wx as f32, wz as f32);
                let height = remap_noise_to_height(noise, params, world_height, world_height_f);
                heights.push(height);
            }
        }
        let elapsed_us = t0.elapsed().as_micros().min(u128::from(u32::MAX)) as u32;
        ctx.height_tile_stats = HeightTileStats {
            duration_us: elapsed_us,
            columns: total_columns,
            reused: false,
        };

        ctx.height_tile = Some(HeightTile::new(base_x, base_z, size_x, size_z, heights));
    }
}

struct ColumnSampler<'ctx, 'p> {
    ctx: &'ctx mut GenCtx,
    params: &'p WorldGenParams,
    world_height: i32,
    world_height_f: f32,
}

impl<'ctx, 'p> ColumnSampler<'ctx, 'p> {
    fn new(world: &World, ctx: &'ctx mut GenCtx, params: &'p WorldGenParams) -> Self {
        let world_height = world.world_height_hint() as i32;
        let world_height_f = world_height as f32;
        Self {
            ctx,
            params,
            world_height,
            world_height_f,
        }
    }

    #[inline]
    fn profiler_mut(&mut self) -> &mut TerrainProfiler {
        &mut self.ctx.terrain_profiler
    }

    fn world_height(&self) -> i32 {
        self.world_height
    }

    fn world_height_f(&self) -> f32 {
        self.world_height_f
    }

    fn height_for(&mut self, wx: i32, wz: i32) -> i32 {
        self.profiler_mut().begin_stage(TerrainStage::Height);
        let stage_start = Instant::now();
        if let Some(tile) = self.ctx.height_tile.as_ref() {
            if let Some(height) = tile.height(wx, wz) {
                self.profiler_mut().record_height_cache(true);
                self.profiler_mut()
                    .record_stage_duration(TerrainStage::Height, stage_start.elapsed());
                return height;
            }
        }
        self.profiler_mut().record_height_cache(false);
        let noise = self.ctx.terrain.get_noise_2d(wx as f32, wz as f32);
        let height =
            remap_noise_to_height(noise, self.params, self.world_height, self.world_height_f);
        self.profiler_mut()
            .record_stage_duration(TerrainStage::Height, stage_start.elapsed());
        height
    }

    fn water_level(&self) -> i32 {
        if self.params.water_enable {
            (self.world_height_f * self.params.water_level_ratio).round() as i32
        } else {
            -1
        }
    }

    fn climate_for(&mut self, wx: i32, wz: i32) -> Option<(f32, f32)> {
        // PERF: Each lookup re-samples 2D noise; cache by (wx, wz) when iterating broad areas.
        let biomes = self.params.biomes.as_ref()?;
        let temp = self.ctx.temp2d.as_ref()?;
        let moist = self.ctx.moist2d.as_ref()?;
        let pack = &**biomes;
        let sx = if pack.scale_x == 0.0 {
            1.0
        } else {
            pack.scale_x
        };
        let sz = if pack.scale_z == 0.0 {
            1.0
        } else {
            pack.scale_z
        };
        let x = wx as f32 * sx;
        let z = wz as f32 * sz;
        let tt = ((temp.get_noise_2d(x, z) + 1.0) * 0.5).clamp(0.0, 1.0);
        let mm = ((moist.get_noise_2d(x, z) + 1.0) * 0.5).clamp(0.0, 1.0);
        Some((tt, mm))
    }

    fn biome_for(&mut self, wx: i32, wz: i32) -> Option<&'p crate::worldgen::BiomeDefParam> {
        let biomes = self.params.biomes.as_ref()?;
        let pack = &**biomes;
        if pack.debug_pack_all && !pack.defs.is_empty() {
            let cell = pack.debug_cell_size.max(1);
            let cx = (wx.div_euclid(cell)) as i64;
            let cz = (wz.div_euclid(cell)) as i64;
            let idx = ((cx * 31 + cz * 17).rem_euclid(pack.defs.len() as i64)) as usize;
            if let Some(def) = pack.defs.get(idx) {
                return Some(def);
            }
        }
        let (t, m) = self.climate_for(wx, wz)?;
        for def in &pack.defs {
            if t >= def.temp_min
                && t < def.temp_max
                && m >= def.moisture_min
                && m < def.moisture_max
            {
                return Some(def);
            }
        }
        None
    }

    fn top_block_for_column(&mut self, wx: i32, wz: i32, hh: i32) -> &'p str {
        let params = self.params;
        if hh as f32 >= self.world_height_f * params.snow_threshold {
            return params.top_high.as_str();
        }
        if hh as f32 <= self.world_height_f * params.sand_threshold {
            return params.top_low.as_str();
        }
        if let Some(def) = self.biome_for(wx, wz) {
            if let Some(ref tb) = def.top_block {
                return tb.as_str();
            }
        }
        params.top_mid.as_str()
    }

    fn tree_probability(&mut self, wx: i32, wz: i32) -> f32 {
        if let Some(def) = self.biome_for(wx, wz) {
            if let Some(density) = def.tree_density {
                return density.clamp(0.0, 1.0);
            }
        }
        self.params.tree_probability
    }
}

fn evaluate_tower(
    world: &World,
    reg: &BlockRegistry,
    profiler: &mut TerrainProfiler,
    x: i32,
    y: i32,
    z: i32,
    air: RtBlock,
) -> Option<RtBlock> {
    profiler.begin_stage(TerrainStage::Tower);
    let stage_start = Instant::now();
    let mut result = None;
    let tower_center_x = (world.world_size_x() as i32) / 2;
    let tower_center_z = (world.world_size_z() as i32) / 2;
    let dx = x - tower_center_x;
    let dz = z - tower_center_z;
    let dist2 = (dx as i64).pow(2) + (dz as i64).pow(2);
    const TOWER_OUTER_RADIUS: i32 = 12;
    const TOWER_INNER_RADIUS: i32 = 7;
    let outer_sq = (TOWER_OUTER_RADIUS as i64).pow(2);
    let inner_sq = (TOWER_INNER_RADIUS as i64).pow(2);
    if dist2 <= outer_sq {
        let tower_top = 4096;
        if y < tower_top {
            if dist2 <= inner_sq {
                if y % 32 == 0 {
                    let id = world.resolve_block_id(reg, "stone");
                    result = Some(RtBlock { id, state: 0 });
                }
                if result.is_none() {
                    result = Some(air);
                }
            }
            if result.is_none() {
                let band = y.rem_euclid(128);
                let block_name = if band < 6 {
                    "glowstone"
                } else if band < 24 {
                    "glass"
                } else {
                    "stone"
                };
                let id = world.resolve_block_id(reg, block_name);
                result = Some(RtBlock { id, state: 0 });
            }
        }
        if result.is_none() {
            result = Some(air);
        }
    }
    profiler.record_stage_duration(TerrainStage::Tower, stage_start.elapsed());
    result
}

fn select_surface_block<'p>(
    sampler: &mut ColumnSampler<'_, 'p>,
    x: i32,
    y: i32,
    z: i32,
    height: i32,
) -> &'p str {
    sampler.profiler_mut().begin_stage(TerrainStage::Surface);
    let stage_start = Instant::now();
    let block = if y >= height {
        "air"
    } else if y == height - 1 {
        sampler.top_block_for_column(x, z, height)
    } else if y + sampler.params.topsoil_thickness >= height {
        sampler.params.sub_near.as_str()
    } else {
        sampler.params.sub_deep.as_str()
    };
    sampler
        .profiler_mut()
        .record_stage_duration(TerrainStage::Surface, stage_start.elapsed());
    block
}

fn apply_water_fill<'p>(
    sampler: &mut ColumnSampler<'_, 'p>,
    y: i32,
    water_level: i32,
    base: &mut &'p str,
) {
    sampler.profiler_mut().begin_stage(TerrainStage::Water);
    let stage_start = Instant::now();
    if *base == "air" && sampler.params.water_enable && y <= water_level {
        *base = "water";
    }
    sampler
        .profiler_mut()
        .record_stage_duration(TerrainStage::Water, stage_start.elapsed());
}

fn apply_caves_and_features<'p>(
    world: &World,
    sampler: &mut ColumnSampler<'_, 'p>,
    x: i32,
    y: i32,
    z: i32,
    height: i32,
    base: &mut &'p str,
) -> bool {
    sampler.profiler_mut().begin_stage(TerrainStage::Caves);
    let stage_start = Instant::now();
    let params = sampler.params;
    let world_height = sampler.world_height();
    let world_height_f = sampler.world_height_f();
    let mut carved_here = false;

    if matches!(*base, "stone" | "dirt" | "sand" | "snow" | "glowstone") {
        let y_scale = params.y_scale;
        let eps_base = params.eps_base;
        let eps_add = params.eps_add;
        let warp_xy = params.warp_xy;
        let warp_y = params.warp_y;
        let room_cell = params.room_cell;
        let room_thr_base = params.room_thr_base;
        let room_thr_add = params.room_thr_add;
        let soil_min = params.soil_min;
        let min_y = params.min_y;

        let h = height as f32;
        let wy = y as f32;
        let soil = h - wy;
        if params.carvers_enable && soil > soil_min && wy > min_y {
            let wx = x as f32;
            let wyf = y as f32;
            let wz = z as f32;
            let wxw = fractal3(&sampler.ctx.warp, wx, wyf, wz, &params.warp);
            let wyw = fractal3(
                &sampler.ctx.warp,
                wx + 133.7,
                wyf + 71.3,
                wz - 19.1,
                &params.warp,
            );
            let wzw = fractal3(
                &sampler.ctx.warp,
                wx - 54.2,
                wyf + 29.7,
                wz + 88.8,
                &params.warp,
            );
            let xp = wx + wxw * warp_xy;
            let yp = wyf + wyw * warp_y;
            let zp = wz + wzw * warp_xy;
            let tn = fractal3(&sampler.ctx.tunnel, xp, yp * y_scale, zp, &params.tunnel);
            let depth01 = (soil / world_height_f).clamp(0.0, 1.0);
            let eps = eps_base + eps_add * depth01;
            let wn = worley3_f1_norm(world.seed as u32, xp, yp, zp, room_cell);
            let room_thr = room_thr_base + room_thr_add * depth01;
            let carved_air = (tn.abs() < eps) || (wn < room_thr);
            if carved_air {
                *base = "air";
                carved_here = true;
            }

            let mut near_solid_cache: Option<bool> = None;
            let features = &params.features;
            if !features.is_empty() {
                for (ri, rule) in features.iter().enumerate() {
                    let w = &rule.when;
                    if !w.base_in.is_empty() && !w.base_in.iter().any(|s| s.as_str() == *base) {
                        continue;
                    }
                    if !w.base_not_in.is_empty()
                        && w.base_not_in.iter().any(|s| s.as_str() == *base)
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
                        if req
                            != compute_near_solid(
                                sampler,
                                &mut near_solid_cache,
                                world.seed as u32,
                                x,
                                y,
                                z,
                                params,
                                world_height,
                                world_height_f,
                                y_scale,
                                warp_xy,
                                warp_y,
                                eps_base,
                                eps_add,
                                room_cell,
                                room_thr_base,
                                room_thr_add,
                                soil_min,
                                min_y,
                            )
                        {
                            continue;
                        }
                    }
                    if let Some(p) = w.chance {
                        if p < 1.0 {
                            let salt = ((world.seed as u32).wrapping_add(0xC0FF_EE15))
                                .wrapping_add(ri as u32 * 0x9E37_79B9);
                            let h = hash3_feature(x, y, z, salt) & 0x00FF_FFFF;
                            let r = (h as f32) / 16_777_216.0;
                            if r >= p {
                                continue;
                            }
                        }
                    }
                    *base = rule.place.block.as_str();
                    break;
                }
            }
        }
    }

    let carved = carved_here;
    sampler
        .profiler_mut()
        .record_stage_duration(TerrainStage::Caves, stage_start.elapsed());
    carved
}

fn compute_near_solid<'p>(
    sampler: &mut ColumnSampler<'_, 'p>,
    cache: &mut Option<bool>,
    world_seed: u32,
    x: i32,
    y: i32,
    z: i32,
    params: &WorldGenParams,
    world_height: i32,
    world_height_f: f32,
    y_scale: f32,
    warp_xy: f32,
    warp_y: f32,
    eps_base: f32,
    eps_add: f32,
    room_cell: f32,
    room_thr_base: f32,
    room_thr_add: f32,
    soil_min: f32,
    min_y: f32,
) -> bool {
    if let Some(value) = *cache {
        return value;
    }
    let mut near_solid = false;
    for (dx, dy, dz) in [
        (-1, 0, 0),
        (1, 0, 0),
        (0, -1, 0),
        (0, 1, 0),
        (0, 0, -1),
        (0, 0, 1),
    ] {
        let nx = x + dx;
        let ny = y + dy;
        let nz = z + dz;
        if ny < 0 || ny >= world_height {
            continue;
        }
        let nh = sampler.height_for(nx, nz);
        if ny >= nh {
            continue;
        }
        let wxn = nx as f32;
        let wyn = ny as f32;
        let wzn = nz as f32;
        // PERF: Neighbor checks rerun the same noise stack as the main voxel; cache if profiling shows spikes.
        let wxw_n = fractal3(&sampler.ctx.warp, wxn, wyn, wzn, &params.warp);
        let wyw_n = fractal3(
            &sampler.ctx.warp,
            wxn + 133.7,
            wyn + 71.3,
            wzn - 19.1,
            &params.warp,
        );
        let wzw_n = fractal3(
            &sampler.ctx.warp,
            wxn - 54.2,
            wyn + 29.7,
            wzn + 88.8,
            &params.warp,
        );
        let nxp = wxn + wxw_n * warp_xy;
        let nyp = wyn + wyw_n * warp_y;
        let nzp = wzn + wzw_n * warp_xy;
        let tn_n = fractal3(&sampler.ctx.tunnel, nxp, nyp * y_scale, nzp, &params.tunnel);
        let nsoil = nh as f32 - wyn;
        let n_depth = (nsoil / world_height_f).clamp(0.0, 1.0);
        let eps_n = eps_base + eps_add * n_depth;
        let wn_n = worley3_f1_norm(world_seed, nxp, nyp, nzp, room_cell);
        let room_thr_n = room_thr_base + room_thr_add * n_depth;
        let neighbor_carved_air =
            (nsoil > soil_min && wyn > min_y) && (tn_n.abs() < eps_n || wn_n < room_thr_n);
        if !neighbor_carved_air {
            near_solid = true;
            break;
        }
    }
    *cache = Some(near_solid);
    near_solid
}

fn apply_tree_blocks<'p>(
    world: &World,
    sampler: &mut ColumnSampler<'_, 'p>,
    x: i32,
    y: i32,
    z: i32,
    base: &mut &'p str,
) {
    sampler.profiler_mut().begin_stage(TerrainStage::Trees);
    let stage_start = Instant::now();
    let params = sampler.params;
    let tree_prob = sampler.tree_probability(x, z);
    let trunk_min = params.trunk_min;
    let trunk_max = params.trunk_max;
    let leaf_r = params.leaf_radius;
    let world_height = sampler.world_height();
    let seed = world.seed as u32;

    if let Some((surf, th, sp)) = trunk_info(
        sampler,
        x,
        z,
        tree_prob,
        trunk_min,
        trunk_max,
        world_height,
        seed,
    ) {
        if y > surf && y <= surf + th {
            *base = match sp {
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

    if *base == "air" {
        for tx in (x - leaf_r)..=(x + leaf_r) {
            for tz in (z - leaf_r)..=(z + leaf_r) {
                // PERF: O(r^2) scan per voxel; guard with cheaper early-outs if foliage turns hot in profiles.
                if let Some((surf, th, sp)) = trunk_info(
                    sampler,
                    tx,
                    tz,
                    tree_prob,
                    trunk_min,
                    trunk_max,
                    world_height,
                    seed,
                ) {
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
                        *base = match sp {
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
    sampler
        .profiler_mut()
        .record_stage_duration(TerrainStage::Trees, stage_start.elapsed());
}

fn hash2_tree(ix: i32, iz: i32, seed: u32) -> u32 {
    let mut h = (ix as u32).wrapping_mul(0x85eb_ca6b)
        ^ (iz as u32).wrapping_mul(0xc2b2_ae35)
        ^ seed.wrapping_mul(0x27d4_eb2d);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h
}

fn rand01_tree(world_seed: u32, ix: i32, iz: i32, salt: u32) -> f32 {
    let h = hash2_tree(ix, iz, (world_seed ^ salt).wrapping_add(0x9E37_79B9));
    ((h & 0x00FF_FFFF) as f32) / 16_777_216.0
}

fn pick_species_for_column<'p>(
    sampler: &mut ColumnSampler<'_, 'p>,
    tx: i32,
    tz: i32,
    seed: u32,
) -> &'static str {
    // PERF: Species selection can bounce through biome tables and random generators per column.
    if let Some(def) = sampler.biome_for(tx, tz) {
        if !def.species_weights.is_empty() {
            let mut total = 0.0_f32;
            for w in def.species_weights.values() {
                total += *w;
            }
            if total > 0.0 {
                let r = rand01_tree(seed, tx, tz, 0xA11CE) * total;
                let mut acc = 0.0_f32;
                for (key, weight) in &def.species_weights {
                    acc += *weight;
                    if r <= acc {
                        return match key.as_str() {
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
    let t = rand01_tree(seed, tx, tz, 0xBEEF01);
    let m = rand01_tree(seed, tx, tz, 0xC0FFEE);
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
    if ((hash2_tree(tx, tz, 0xDEAD_BEEF) >> 20) & 1) == 1 {
        "birch"
    } else {
        "oak"
    }
}

fn trunk_info<'p>(
    sampler: &mut ColumnSampler<'_, 'p>,
    tx: i32,
    tz: i32,
    tree_prob: f32,
    trunk_min: i32,
    trunk_max: i32,
    world_height: i32,
    seed: u32,
) -> Option<(i32, i32, &'static str)> {
    let surf = sampler.height_for(tx, tz) - 1;
    let surf_block = sampler.top_block_for_column(tx, tz, surf + 1);
    if surf_block != "grass" {
        return None;
    }
    if rand01_tree(seed, tx, tz, 0xA53F9) >= tree_prob {
        return None;
    }
    let span = (trunk_max - trunk_min).max(0) as u32;
    let hsel = hash2_tree(tx, tz, 0x0051_F0A7) % (span + 1);
    let th = trunk_min + hsel as i32;
    if surf <= 2 || surf >= (world_height - 6) {
        return None;
    }
    let sp = pick_species_for_column(sampler, tx, tz, seed);
    Some((surf, th, sp))
}

fn fractal3(noise: &FastNoiseLite, x: f32, y: f32, z: f32, fractal: &Fractal) -> f32 {
    // PERF: Each call runs multiple octaves for the same coordinates; reuse when stepping coherently.
    let mut amp = 1.0_f32;
    let mut freq = 1.0_f32 / fractal.scale.max(0.0001);
    let mut sum = 0.0_f32;
    let mut max_amp = 0.0_f32;
    for _ in 0..fractal.octaves.max(1) {
        sum += noise.get_noise_3d(x * freq, y * freq, z * freq) * amp;
        max_amp += amp;
        amp *= fractal.persistence;
        freq *= fractal.lacunarity;
    }
    if max_amp > 0.0 { sum / max_amp } else { sum }
}

fn worley3_f1_norm(seed: u32, x: f32, y: f32, z: f32, cell: f32) -> f32 {
    // PERF: Worley lookup scans 27 pseudo-random cells; avoid in tight loops if possible.
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
                let jx = rand01_cell(seed, cx, cy, cz, 0x068b_c021);
                let jy = rand01_cell(seed, cx, cy, cz, 0x02e1_b213);
                let jz = rand01_cell(seed, cx, cy, cz, 0x0f1a_1234);
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
}

fn rand01_cell(seed: u32, cx: i32, cy: i32, cz: i32, salt: u32) -> f32 {
    let h = hash3_carver(cx, cy, cz, seed ^ salt);
    (h & 0x00FF_FFFF) as f32 / 16_777_216.0
}

fn hash3_carver(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    fn uhash32(mut a: u32) -> u32 {
        a ^= a >> 16;
        a = a.wrapping_mul(0x7feb_352d);
        a ^= a >> 15;
        a = a.wrapping_mul(0x846c_a68b);
        a ^= a >> 16;
        a
    }
    let ux = x as u32;
    let uy = y as u32;
    let uz = z as u32;
    let mut h = seed ^ 0x9e37_79b9;
    h ^= uhash32(ux.wrapping_add(0x85eb_ca6b));
    h ^= uhash32(uy.wrapping_add(0xc2b2_ae35));
    h ^= uhash32(uz.wrapping_add(0x27d4_eb2f));
    uhash32(h)
}

fn hash3_feature(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    let mix = |mut v: u32| {
        v ^= v >> 16;
        v = v.wrapping_mul(0x7feb_352d);
        v ^= v >> 15;
        v = v.wrapping_mul(0x846c_a68b);
        v ^= v >> 16;
        v
    };
    let mut a = seed ^ 0x9e37_79b9;
    a ^= mix(x as u32);
    a ^= mix(y as u32);
    a ^= mix(z as u32);
    a
}

use std::time::Instant;

use crate::worldgen::WorldGenParams;

use super::super::gen_ctx::{TerrainProfiler, TerrainStage};
use super::super::{GenCtx, World};

pub(super) fn remap_noise_to_height(
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

pub struct ColumnSampler<'ctx, 'p> {
    pub(super) ctx: &'ctx mut GenCtx,
    pub(super) params: &'p WorldGenParams,
    world_height: i32,
    world_height_f: f32,
}

impl<'ctx, 'p> ColumnSampler<'ctx, 'p> {
    pub fn new(world: &World, ctx: &'ctx mut GenCtx, params: &'p WorldGenParams) -> Self {
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
    pub(super) fn profiler_mut(&mut self) -> &mut TerrainProfiler {
        &mut self.ctx.terrain_profiler
    }

    pub(super) fn world_height(&self) -> i32 {
        self.world_height
    }

    pub(super) fn world_height_f(&self) -> f32 {
        self.world_height_f
    }

    pub(super) fn height_for(&mut self, wx: i32, wz: i32) -> i32 {
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

    pub(super) fn water_level(&self) -> i32 {
        if self.params.water_enable {
            (self.world_height_f * self.params.water_level_ratio).round() as i32
        } else {
            -1
        }
    }

    pub(super) fn biome_for(
        &mut self,
        wx: i32,
        wz: i32,
    ) -> Option<&'p crate::worldgen::BiomeDefParam> {
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

    pub(super) fn top_block_for_column(&mut self, wx: i32, wz: i32, hh: i32) -> &'p str {
        if hh as f32 >= self.world_height_f * self.params.snow_threshold {
            return self.params.top_high.as_str();
        }
        if hh as f32 <= self.world_height_f * self.params.sand_threshold {
            return self.params.top_low.as_str();
        }
        if let Some(def) = self.biome_for(wx, wz) {
            if let Some(ref tb) = def.top_block {
                return tb.as_str();
            }
        }
        self.params.top_mid.as_str()
    }

    pub(super) fn tree_probability(&mut self, wx: i32, wz: i32) -> f32 {
        if let Some(def) = self.biome_for(wx, wz) {
            if let Some(density) = def.tree_density {
                return density.clamp(0.0, 1.0);
            }
        }
        self.params.tree_probability
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
}

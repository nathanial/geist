mod caves;
mod column_plan;
mod column_sampler;
mod surface;
mod tower;
mod trees;
mod water;

use std::sync::Arc;
use std::time::Instant;

use geist_blocks::registry::BlockRegistry;
use geist_blocks::types::Block as RtBlock;

use crate::worldgen::WorldGenParams;

use super::gen_ctx::{HeightTileStats, TerrainStage};
use super::tile_cache::{TerrainTile, TileKey};
use super::{GenCtx, World, WorldGenMode};

use self::caves::apply_caves_and_features;
pub use self::caves::{BlockLookup, apply_caves_and_features_blocks};
pub use self::column_plan::{
    ChunkColumnPlan, ColumnInfo, ColumnMaterials, build_chunk_column_plan,
};
pub use self::column_sampler::ColumnSampler;
use self::column_sampler::remap_noise_to_height;
use self::surface::select_surface_block;
use self::tower::evaluate_tower;
use self::trees::apply_tree_blocks;
use self::water::apply_water_fill;

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
            ctx.tile_cache_stats = self.terrain_tile_cache_stats();
            return;
        }

        let total_columns = (size_x * size_z) as u32;
        let key = TileKey::new(base_x, base_z, size_x, size_z);
        if ctx.height_tile.as_ref().is_some_and(|tile| {
            tile.matches(&key) && tile.worldgen_rev == self.current_worldgen_rev()
        }) {
            ctx.height_tile_stats = HeightTileStats {
                duration_us: 0,
                columns: total_columns,
                reused: true,
            };
            ctx.tile_cache_stats = self.terrain_tile_cache_stats();
            return;
        }

        let rev = self.current_worldgen_rev();
        if let Some(tile) = self.tile_cache().get(&key, rev) {
            ctx.height_tile = Some(tile);
            ctx.height_tile_stats = HeightTileStats {
                duration_us: 0,
                columns: total_columns,
                reused: true,
            };
            ctx.tile_cache_stats = self.terrain_tile_cache_stats();
            return;
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
        let tile = TerrainTile::new(key, rev, heights, elapsed_us, total_columns);
        self.tile_cache().insert(tile.clone());
        ctx.height_tile = Some(tile);
        ctx.tile_cache_stats = self.terrain_tile_cache_stats();
    }
}

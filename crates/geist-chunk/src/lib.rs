//! Chunk buffer and world generation helpers.
#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use geist_blocks::BlockRegistry;
use geist_blocks::types::Block;
use geist_world::{
    ChunkCoord, ChunkTiming, GenCtx, HeightTileStats, TerrainMetrics, TerrainStage,
    TerrainTileCacheStats, World,
    voxel::generation::{
        BlockLookup, ChunkColumnPlan, ChunkColumnProfile, ColumnMaterials, ColumnSampler,
        TOWER_OUTER_RADIUS, TowerMaterial, TreePlan, apply_caves_and_features_blocks,
        build_chunk_column_plan, tower_material,
    },
};

#[derive(Clone, Debug)]
pub struct ChunkBuf {
    pub coord: ChunkCoord,
    pub sx: usize,
    pub sy: usize,
    pub sz: usize,
    pub blocks: Vec<Block>,
}

impl ChunkBuf {
    #[inline]
    pub fn idx(&self, x: usize, y: usize, z: usize) -> usize {
        (y * self.sz + z) * self.sx + x
    }

    #[inline]
    pub fn get_local(&self, x: usize, y: usize, z: usize) -> Block {
        self.blocks[self.idx(x, y, z)]
    }

    #[inline]
    pub fn contains_world(&self, wx: i32, wy: i32, wz: i32) -> bool {
        let base_x = self.coord.cx * self.sx as i32;
        let base_y = self.coord.cy * self.sy as i32;
        let base_z = self.coord.cz * self.sz as i32;
        if wy < base_y || wy >= base_y + self.sy as i32 {
            return false;
        }
        wx >= base_x && wx < base_x + self.sx as i32 && wz >= base_z && wz < base_z + self.sz as i32
    }

    #[inline]
    pub fn get_world(&self, wx: i32, wy: i32, wz: i32) -> Option<Block> {
        if !self.contains_world(wx, wy, wz) {
            return None;
        }
        let base_x = self.coord.cx * self.sx as i32;
        let base_y = self.coord.cy * self.sy as i32;
        let base_z = self.coord.cz * self.sz as i32;
        let lx = (wx - base_x) as usize;
        let ly = (wy - base_y) as usize;
        let lz = (wz - base_z) as usize;
        Some(self.get_local(lx, ly, lz))
    }

    pub fn from_blocks_local(
        coord: ChunkCoord,
        sx: usize,
        sy: usize,
        sz: usize,
        blocks: Vec<Block>,
    ) -> Self {
        let mut b = blocks;
        let expect = sx * sy * sz;
        if b.len() != expect {
            b.resize(expect, Block { id: 0, state: 0 });
        }
        ChunkBuf {
            coord,
            sx,
            sy,
            sz,
            blocks: b,
        }
    }

    #[inline]
    pub fn has_non_air(&self) -> bool {
        self.blocks.iter().any(|b| *b != Block::AIR)
    }

    #[inline]
    pub fn is_all_air(&self) -> bool {
        !self.has_non_air()
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ChunkOccupancy {
    Empty,
    Populated,
}

impl ChunkOccupancy {
    #[inline]
    pub fn is_empty(self) -> bool {
        matches!(self, ChunkOccupancy::Empty)
    }

    #[inline]
    pub fn has_blocks(self) -> bool {
        matches!(self, ChunkOccupancy::Populated)
    }
}

#[derive(Debug)]
pub struct ChunkGenerateResult {
    pub buf: ChunkBuf,
    pub occupancy: ChunkOccupancy,
    pub terrain_metrics: TerrainMetrics,
    pub column_profile: Option<ChunkColumnProfile>,
}

struct MaterializeOutput {
    blocks: Vec<Block>,
    occupancy: ChunkOccupancy,
    metrics: TerrainMetrics,
}

fn collect_tree_plans(
    world: &World,
    ctx: &mut GenCtx,
    reg: &BlockRegistry,
    plan: &ChunkColumnPlan,
) -> Vec<TreePlan> {
    let mut tree_plans = Vec::new();
    let mut seen_trees = HashSet::new();
    for column in &plan.columns {
        if let Some(tree) = &column.tree {
            if seen_trees.insert((tree.base_x, tree.base_z)) {
                tree_plans.push(tree.clone());
            }
        }
    }
    let leaf_radius = plan.materials.leaf_radius;
    if leaf_radius > 0 && !world.is_flat() {
        let radius = leaf_radius as usize;
        if radius > 0 {
            let sx = plan.width;
            let sz = plan.depth;
            let base_x = plan.columns.first().map(|c| c.wx).unwrap_or_default();
            let base_z = plan.columns.first().map(|c| c.wz).unwrap_or_default();
            let extra_base_x = base_x - leaf_radius;
            let extra_base_z = base_z - leaf_radius;
            let extra_sx = sx + radius * 2;
            let extra_sz = sz + radius * 2;
            let extra_plan = build_chunk_column_plan(
                world,
                ctx,
                reg,
                extra_base_x,
                extra_base_z,
                extra_sx,
                extra_sz,
            );
            for column in &extra_plan.columns {
                if let Some(tree) = &column.tree {
                    if seen_trees.insert((tree.base_x, tree.base_z)) {
                        tree_plans.push(tree.clone());
                    }
                }
            }
        }
    }
    tree_plans
}

fn materialize_chunk(
    world: &World,
    coord: ChunkCoord,
    reg: &BlockRegistry,
    ctx: &mut GenCtx,
    plan: &ChunkColumnPlan,
    tree_plans: &[TreePlan],
    base_y: i32,
    height_tile_us: u32,
    total_start: Instant,
) -> MaterializeOutput {
    let sx = plan.width;
    let sy = world.chunk_size_y;
    let sz = plan.depth;
    let mut blocks = Vec::with_capacity(sx * sy * sz);
    blocks.resize(sx * sy * sz, Block { id: 0, state: 0 });
    let base_x = coord.cx * sx as i32;
    let base_z = coord.cz * sz as i32;
    let chunk_min_y = base_y;
    let chunk_max_y = base_y + sy as i32;

    let mut block_lookup = BlockLookup::default();

    let fill_start = Instant::now();

    let materials: &ColumnMaterials = &plan.materials;
    let topsoil = materials.topsoil_thickness.max(0) as i32;
    for lz in 0..sz {
        let column = |lx: usize| plan.column(lx, lz);
        for lx in 0..sx {
            let column = column(lx);
            let height = column.height;
            let surface_y = height - 1;
            let soil_start = height - topsoil;

            let deep_end = soil_start.min(surface_y + 1).min(chunk_max_y);
            if deep_end > chunk_min_y {
                for wy in chunk_min_y..deep_end {
                    let ly = (wy - chunk_min_y) as usize;
                    let idx = (ly * sz + lz) * sx + lx;
                    blocks[idx] = materials.sub_deep_block;
                }
            }

            let near_start = soil_start.max(chunk_min_y);
            let near_end = surface_y.min(chunk_max_y - 1) + 1;
            if near_end > near_start {
                for wy in near_start..near_end {
                    let ly = (wy - chunk_min_y) as usize;
                    let idx = (ly * sz + lz) * sx + lx;
                    blocks[idx] = materials.sub_near_block;
                }
            }

            if surface_y >= chunk_min_y && surface_y < chunk_max_y {
                let ly = (surface_y - chunk_min_y) as usize;
                let idx = (ly * sz + lz) * sx + lx;
                blocks[idx] = column.surface_block;
            }

            if let Some(water_block) = materials.water_block {
                let water_start = height.max(chunk_min_y);
                let water_end = (column.water_level + 1).min(chunk_max_y);
                if water_end > water_start {
                    for wy in water_start..water_end {
                        let ly = (wy - chunk_min_y) as usize;
                        let idx = (ly * sz + lz) * sx + lx;
                        if blocks[idx] == materials.air_block {
                            blocks[idx] = water_block;
                        }
                    }
                }
            }
        }
    }

    if !world.is_flat() {
        let params_guard = Arc::clone(&ctx.params);
        let params = &*params_guard;
        let mut sampler = ColumnSampler::new(world, ctx, params);
        for lz in 0..sz {
            let wz = base_z + lz as i32;
            for lx in 0..sx {
                let wx = base_x + lx as i32;
                let column = plan.column(lx, lz);
                let height = column.height;
                let carve_top = height.min(chunk_max_y);
                if carve_top <= chunk_min_y {
                    continue;
                }
                for wy in chunk_min_y..carve_top {
                    let ly = (wy - chunk_min_y) as usize;
                    let idx = (ly * sz + lz) * sx + lx;
                    if blocks[idx] == materials.air_block {
                        continue;
                    }
                    let mut block = blocks[idx];
                    let _ = apply_caves_and_features_blocks(
                        world,
                        &mut sampler,
                        reg,
                        &mut block_lookup,
                        wx,
                        wy,
                        wz,
                        height,
                        &mut block,
                    );
                    blocks[idx] = block;
                }
            }
        }
    }

    for tree in tree_plans {
        let trunk_x = tree.base_x - base_x;
        let trunk_z = tree.base_z - base_z;
        if trunk_x >= 0 && trunk_z >= 0 && trunk_x < sx as i32 && trunk_z < sz as i32 {
            let lx = trunk_x as usize;
            let lz = trunk_z as usize;
            let trunk_start = tree.surface_y + 1;
            let trunk_end = tree.surface_y + tree.trunk_height;
            for wy in trunk_start..=trunk_end {
                if wy < chunk_min_y || wy >= chunk_max_y {
                    continue;
                }
                let ly = (wy - chunk_min_y) as usize;
                let idx = (ly * sz + lz) * sx + lx;
                blocks[idx] = tree.trunk_block;
            }
        }
    }

    let leaf_radius = materials.leaf_radius;
    if leaf_radius > 0 {
        for tree in tree_plans {
            let top_y = tree.surface_y + tree.trunk_height;
            for dy in -2..=2 {
                let wy = top_y + dy;
                if wy < chunk_min_y || wy >= chunk_max_y {
                    continue;
                }
                let radius = if dy <= -2 || dy >= 2 {
                    leaf_radius - 1
                } else {
                    leaf_radius
                };
                if radius < 0 {
                    continue;
                }
                for dx in -leaf_radius..=leaf_radius {
                    for dz in -leaf_radius..=leaf_radius {
                        let man = dx.abs() + dz.abs();
                        let extra = if dy >= 1 { 0 } else { 1 };
                        if man > radius + extra {
                            continue;
                        }
                        if dx == 0 && dz == 0 && dy >= 0 {
                            continue;
                        }
                        let wx = tree.base_x + dx;
                        let wz = tree.base_z + dz;
                        if wx < base_x
                            || wz < base_z
                            || wx >= base_x + sx as i32
                            || wz >= base_z + sz as i32
                        {
                            continue;
                        }
                        let lx = (wx - base_x) as usize;
                        let lz = (wz - base_z) as usize;
                        let ly = (wy - chunk_min_y) as usize;
                        let idx = (ly * sz + lz) * sx + lx;
                        if blocks[idx] == materials.air_block {
                            blocks[idx] = tree.leaves_block;
                        }
                    }
                }
            }
        }
    }

    {
        let tower_center_x = (world.world_size_x() as i32) / 2;
        let tower_center_z = (world.world_size_z() as i32) / 2;
        let chunk_min_x = base_x;
        let chunk_max_x = base_x + sx as i32;
        let chunk_min_z = base_z;
        let chunk_max_z = base_z + sz as i32;
        let tower_min_x = tower_center_x - TOWER_OUTER_RADIUS;
        let tower_max_x = tower_center_x + TOWER_OUTER_RADIUS;
        let tower_min_z = tower_center_z - TOWER_OUTER_RADIUS;
        let tower_max_z = tower_center_z + TOWER_OUTER_RADIUS;

        if chunk_max_x > tower_min_x
            && chunk_min_x < tower_max_x
            && chunk_max_z > tower_min_z
            && chunk_min_z < tower_max_z
        {
            ctx.terrain_profiler.begin_stage(TerrainStage::Tower);
            let tower_stage_start = Instant::now();

            let air_block = materials.air_block;
            let air_id = air_block.id;
            let lookup_block = |name: &str| Block {
                id: reg.id_by_name(name).unwrap_or(air_id),
                state: 0,
            };
            let stone_block = lookup_block("stone");
            let glass_block = lookup_block("glass");
            let glowstone_block = lookup_block("glowstone");
            let outer_sq = (TOWER_OUTER_RADIUS as i64).pow(2);

            for lz in 0..sz {
                let wz = base_z + lz as i32;
                let dz = (wz - tower_center_z) as i64;
                for lx in 0..sx {
                    let wx = base_x + lx as i32;
                    let dx = (wx - tower_center_x) as i64;
                    let dist2 = dx * dx + dz * dz;
                    if dist2 > outer_sq {
                        continue;
                    }
                    for wy in chunk_min_y..chunk_max_y {
                        let material = tower_material(dist2, wy);
                        if material == TowerMaterial::None {
                            continue;
                        }
                        let new_block = match material {
                            TowerMaterial::None => continue,
                            TowerMaterial::Air => air_block,
                            TowerMaterial::Stone => stone_block,
                            TowerMaterial::Glass => glass_block,
                            TowerMaterial::Glowstone => glowstone_block,
                        };
                        let ly = (wy - chunk_min_y) as usize;
                        let idx = (ly * sz + lz) * sx + lx;
                        if blocks[idx] != new_block {
                            blocks[idx] = new_block;
                        }
                    }
                }
            }

            ctx.terrain_profiler
                .record_stage_duration(TerrainStage::Tower, tower_stage_start.elapsed());
        }
    }

    let voxel_fill_us = duration_to_us(fill_start.elapsed());
    let has_blocks = blocks.iter().any(|b| *b != Block::AIR);

    let mut chunk_timing = ChunkTiming {
        total_us: duration_to_us(total_start.elapsed()),
        height_tile_us,
        voxel_fill_us,
        feature_us: 0,
    };

    let mut metrics = ctx
        .terrain_profiler
        .snapshot(ctx.height_tile_stats, ctx.tile_cache_stats);
    ctx.height_tile_stats = HeightTileStats::default();
    ctx.tile_cache_stats = TerrainTileCacheStats::default();
    let feature_us = metrics.stages[TerrainStage::Caves as usize]
        .time_us
        .saturating_add(metrics.stages[TerrainStage::Trees as usize].time_us);
    chunk_timing.feature_us = feature_us;
    metrics.chunk_timing = chunk_timing;

    MaterializeOutput {
        blocks,
        occupancy: if has_blocks {
            ChunkOccupancy::Populated
        } else {
            ChunkOccupancy::Empty
        },
        metrics,
    }
}

#[inline]
fn duration_to_us(duration: Duration) -> u32 {
    duration.as_micros().min(u128::from(u32::MAX)) as u32
}

pub fn generate_chunk_buffer(
    world: &World,
    coord: ChunkCoord,
    reg: &BlockRegistry,
) -> ChunkGenerateResult {
    let mut ctx = world.make_gen_ctx();
    generate_chunk_buffer_with_ctx(world, coord, reg, &mut ctx)
}

pub fn generate_chunk_buffer_with_ctx(
    world: &World,
    coord: ChunkCoord,
    reg: &BlockRegistry,
    ctx: &mut GenCtx,
) -> ChunkGenerateResult {
    ctx.terrain_profiler.reset();

    let total_start = Instant::now();
    let sx = world.chunk_size_x;
    let sy = world.chunk_size_y;
    let sz = world.chunk_size_z;
    let base_x = coord.cx * sx as i32;
    let base_y = coord.cy * sy as i32;
    let base_z = coord.cz * sz as i32;

    let tile_start = Instant::now();
    world.prepare_height_tile(ctx, base_x, base_z, sx, sz);
    let height_tile_us = duration_to_us(tile_start.elapsed());

    let column_plan = build_chunk_column_plan(world, ctx, reg, base_x, base_z, sx, sz);
    let tree_plans = collect_tree_plans(world, ctx, reg, &column_plan);
    let materialized = materialize_chunk(
        world,
        coord,
        reg,
        ctx,
        &column_plan,
        &tree_plans,
        base_y,
        height_tile_us,
        total_start,
    );

    let buf = ChunkBuf::from_blocks_local(coord, sx, sy, sz, materialized.blocks);
    let profile =
        ChunkColumnProfile::new(coord, world.current_worldgen_rev(), column_plan, tree_plans);

    ChunkGenerateResult {
        buf,
        occupancy: materialized.occupancy,
        terrain_metrics: materialized.metrics,
        column_profile: Some(profile),
    }
}

pub fn generate_chunk_buffer_from_profile(
    world: &World,
    coord: ChunkCoord,
    reg: &BlockRegistry,
    ctx: &mut GenCtx,
    profile: &ChunkColumnProfile,
) -> ChunkGenerateResult {
    ctx.terrain_profiler.reset();
    profile.bump_reuse();

    let total_start = Instant::now();
    let sx = world.chunk_size_x;
    let sy = world.chunk_size_y;
    let sz = world.chunk_size_z;
    let base_y = coord.cy * sy as i32;

    ctx.height_tile_stats = HeightTileStats {
        duration_us: 0,
        columns: (sx * sz) as u32,
        reused: true,
    };
    ctx.tile_cache_stats = world.terrain_tile_cache_stats();

    let materialized = materialize_chunk(
        world,
        coord,
        reg,
        ctx,
        &profile.plan,
        &profile.trees,
        base_y,
        0,
        total_start,
    );

    let buf = ChunkBuf::from_blocks_local(coord, sx, sy, sz, materialized.blocks);

    ChunkGenerateResult {
        buf,
        occupancy: materialized.occupancy,
        terrain_metrics: materialized.metrics,
        column_profile: None,
    }
}

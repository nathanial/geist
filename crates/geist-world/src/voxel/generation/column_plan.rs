use std::sync::Arc;

use geist_blocks::registry::BlockRegistry;
use geist_blocks::types::Block;

use crate::worldgen::WorldGenParams;

use super::super::{GenCtx, World};
use super::column_sampler::ColumnSampler;
use super::trees::{TreePlan, plan_tree_for_column};

#[derive(Clone, Debug)]
pub struct ColumnInfo {
    pub wx: i32,
    pub wz: i32,
    pub height: i32,
    pub water_level: i32,
    pub surface_block: Block,
    pub column_seed: u32,
    pub tree: Option<TreePlan>,
}

#[derive(Clone, Debug)]
pub struct ColumnMaterials {
    pub sub_near_block: Block,
    pub sub_deep_block: Block,
    pub water_block: Option<Block>,
    pub air_block: Block,
    pub topsoil_thickness: i32,
    pub leaf_radius: i32,
}

#[derive(Clone, Debug)]
pub struct ChunkColumnPlan {
    pub columns: Vec<ColumnInfo>,
    pub materials: ColumnMaterials,
    pub width: usize,
    pub depth: usize,
}

impl ChunkColumnPlan {
    #[inline]
    pub fn index(&self, lx: usize, lz: usize) -> usize {
        lz * self.width + lx
    }

    #[inline]
    pub fn column(&self, lx: usize, lz: usize) -> &ColumnInfo {
        let idx = self.index(lx, lz);
        &self.columns[idx]
    }
}

pub fn build_chunk_column_plan(
    world: &World,
    ctx: &mut GenCtx,
    reg: &BlockRegistry,
    base_x: i32,
    base_z: i32,
    size_x: usize,
    size_z: usize,
) -> ChunkColumnPlan {
    let params_guard: Arc<WorldGenParams> = Arc::clone(&ctx.params);
    let params: &WorldGenParams = &params_guard;
    let sub_near_block = Block {
        id: world.resolve_block_id(reg, params.sub_near.as_str()),
        state: 0,
    };
    let sub_deep_block = Block {
        id: world.resolve_block_id(reg, params.sub_deep.as_str()),
        state: 0,
    };
    let water_block = if params.water_enable {
        Some(Block {
            id: world.resolve_block_id(reg, "water"),
            state: 0,
        })
    } else {
        None
    };
    let air_block = world.air_block(reg);
    let topsoil_thickness = params.topsoil_thickness;

    let mut columns = Vec::with_capacity(size_x * size_z);
    let mut sampler = ColumnSampler::new(world, ctx, params);
    let water_level = sampler.water_level();

    for lz in 0..size_z {
        let wz = base_z + lz as i32;
        for lx in 0..size_x {
            let wx = base_x + lx as i32;
            let height = sampler.height_for(wx, wz);
            let surface_name = sampler.top_block_for_column(wx, wz, height);
            let surface_block = Block {
                id: world.resolve_block_id(reg, surface_name),
                state: 0,
            };
            let column_seed = column_seed(world.seed as u32, wx, wz);
            let tree = plan_tree_for_column(world, &mut sampler, reg, wx, wz, height);

            columns.push(ColumnInfo {
                wx,
                wz,
                height,
                water_level,
                surface_block,
                column_seed,
                tree,
            });
        }
    }

    ChunkColumnPlan {
        columns,
        materials: ColumnMaterials {
            sub_near_block,
            sub_deep_block,
            water_block,
            air_block,
            topsoil_thickness,
            leaf_radius: params.leaf_radius,
        },
        width: size_x,
        depth: size_z,
    }
}

#[inline]
fn column_seed(world_seed: u32, wx: i32, wz: i32) -> u32 {
    // Simple mix inspired by PCG hashing to avoid collisions across nearby columns.
    let mut seed = world_seed ^ 0x9e37_79b9;
    seed = seed.wrapping_add((wx as u32).wrapping_mul(0x85eb_ca6b));
    seed ^= seed >> 16;
    seed = seed.wrapping_mul(0xc2b2_ae35);
    seed ^= (wz as u32).wrapping_mul(0x27d4_eb2f);
    seed ^= seed >> 15;
    seed = seed.wrapping_mul(0x1656_7b7f);
    seed ^ (seed >> 16)
}

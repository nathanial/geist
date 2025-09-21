use std::io::{Cursor, Read};
use std::sync::Arc;

use geist_blocks::registry::BlockRegistry;
use geist_blocks::types::Block;
use serde::{Deserialize, Serialize};

use crate::worldgen::WorldGenParams;

use super::super::{ChunkCoord, GenCtx, World};
use super::column_sampler::ColumnSampler;
use super::trees::{TreePlan, TreeSpecies, plan_tree_for_column};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub wx: i32,
    pub wz: i32,
    pub height: i32,
    pub water_level: i32,
    pub surface_block: Block,
    pub column_seed: u32,
    pub tree: Option<TreePlan>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ColumnMaterials {
    pub sub_near_block: Block,
    pub sub_deep_block: Block,
    pub water_block: Option<Block>,
    pub air_block: Block,
    pub topsoil_thickness: i32,
    pub leaf_radius: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Debug)]
pub struct ChunkColumnProfile {
    pub coord: ChunkCoord,
    pub worldgen_rev: u32,
    pub plan: ChunkColumnPlan,
    pub trees: Vec<TreePlan>,
    pub reuse_count: std::sync::atomic::AtomicU64,
}

impl ChunkColumnProfile {
    pub fn new(
        coord: ChunkCoord,
        worldgen_rev: u32,
        plan: ChunkColumnPlan,
        trees: Vec<TreePlan>,
    ) -> Self {
        Self {
            coord,
            worldgen_rev,
            plan,
            trees,
            reuse_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    #[inline]
    pub fn bump_reuse(&self) {
        self.reuse_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn reuse_count(&self) -> u64 {
        self.reuse_count.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(1); // version
        write_i32(&mut buf, self.coord.cx);
        write_i32(&mut buf, self.coord.cy);
        write_i32(&mut buf, self.coord.cz);
        write_u32(&mut buf, self.worldgen_rev);
        write_u16(&mut buf, self.plan.width as u16);
        write_u16(&mut buf, self.plan.depth as u16);
        write_u32(&mut buf, self.plan.columns.len() as u32);
        for column in &self.plan.columns {
            write_i32(&mut buf, column.wx);
            write_i32(&mut buf, column.wz);
            write_i32(&mut buf, column.height);
            write_i32(&mut buf, column.water_level);
            write_block(&mut buf, column.surface_block);
            write_u32(&mut buf, column.column_seed);
            match &column.tree {
                Some(tree) => {
                    buf.push(1);
                    write_tree(&mut buf, tree);
                }
                None => buf.push(0),
            }
        }
        write_block(&mut buf, self.plan.materials.sub_near_block);
        write_block(&mut buf, self.plan.materials.sub_deep_block);
        match self.plan.materials.water_block {
            Some(block) => {
                buf.push(1);
                write_block(&mut buf, block);
            }
            None => buf.push(0),
        }
        write_block(&mut buf, self.plan.materials.air_block);
        write_i32(&mut buf, self.plan.materials.topsoil_thickness);
        write_i32(&mut buf, self.plan.materials.leaf_radius);

        write_u32(&mut buf, self.trees.len() as u32);
        for tree in &self.trees {
            write_tree(&mut buf, tree);
        }

        buf
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut cursor = Cursor::new(bytes);
        let version = read_u8(&mut cursor)?;
        if version != 1 {
            return Err(format!("unsupported column profile version {}", version));
        }
        let cx = read_i32(&mut cursor)?;
        let cy = read_i32(&mut cursor)?;
        let cz = read_i32(&mut cursor)?;
        let worldgen_rev = read_u32(&mut cursor)?;
        let width = read_u16(&mut cursor)? as usize;
        let depth = read_u16(&mut cursor)? as usize;
        let column_count = read_u32(&mut cursor)? as usize;
        let mut columns = Vec::with_capacity(column_count);
        for _ in 0..column_count {
            let wx = read_i32(&mut cursor)?;
            let wz = read_i32(&mut cursor)?;
            let height = read_i32(&mut cursor)?;
            let water_level = read_i32(&mut cursor)?;
            let surface_block = read_block(&mut cursor)?;
            let column_seed = read_u32(&mut cursor)?;
            let tree_flag = read_u8(&mut cursor)?;
            let tree = if tree_flag != 0 {
                Some(read_tree(&mut cursor)?)
            } else {
                None
            };
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

        let sub_near_block = read_block(&mut cursor)?;
        let sub_deep_block = read_block(&mut cursor)?;
        let water_flag = read_u8(&mut cursor)?;
        let water_block = if water_flag != 0 {
            Some(read_block(&mut cursor)?)
        } else {
            None
        };
        let air_block = read_block(&mut cursor)?;
        let topsoil_thickness = read_i32(&mut cursor)?;
        let leaf_radius = read_i32(&mut cursor)?;

        let tree_count = read_u32(&mut cursor)? as usize;
        let mut trees = Vec::with_capacity(tree_count);
        for _ in 0..tree_count {
            trees.push(read_tree(&mut cursor)?);
        }

        let plan = ChunkColumnPlan {
            columns,
            materials: ColumnMaterials {
                sub_near_block,
                sub_deep_block,
                water_block,
                air_block,
                topsoil_thickness,
                leaf_radius,
            },
            width,
            depth,
        };

        Ok(ChunkColumnProfile::new(
            ChunkCoord::new(cx, cy, cz),
            worldgen_rev,
            plan,
            trees,
        ))
    }
}

fn write_i32(buf: &mut Vec<u8>, value: i32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn write_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn write_u16(buf: &mut Vec<u8>, value: u16) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn write_block(buf: &mut Vec<u8>, block: Block) {
    write_u16(buf, block.id);
    write_u16(buf, block.state);
}

fn write_tree(buf: &mut Vec<u8>, tree: &TreePlan) {
    write_i32(buf, tree.base_x);
    write_i32(buf, tree.base_z);
    write_i32(buf, tree.surface_y);
    write_i32(buf, tree.trunk_height);
    buf.push(tree.species.to_u8());
    write_block(buf, tree.trunk_block);
    write_block(buf, tree.leaves_block);
}

fn read_u8(cursor: &mut Cursor<&[u8]>) -> Result<u8, String> {
    let mut byte = [0u8; 1];
    cursor
        .read_exact(&mut byte)
        .map_err(|e| format!("failed to read u8: {}", e))?;
    Ok(byte[0])
}

fn read_i32(cursor: &mut Cursor<&[u8]>) -> Result<i32, String> {
    let mut bytes = [0u8; 4];
    cursor
        .read_exact(&mut bytes)
        .map_err(|e| format!("failed to read i32: {}", e))?;
    Ok(i32::from_le_bytes(bytes))
}

fn read_u32(cursor: &mut Cursor<&[u8]>) -> Result<u32, String> {
    let mut bytes = [0u8; 4];
    cursor
        .read_exact(&mut bytes)
        .map_err(|e| format!("failed to read u32: {}", e))?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u16(cursor: &mut Cursor<&[u8]>) -> Result<u16, String> {
    let mut bytes = [0u8; 2];
    cursor
        .read_exact(&mut bytes)
        .map_err(|e| format!("failed to read u16: {}", e))?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_block(cursor: &mut Cursor<&[u8]>) -> Result<Block, String> {
    let id = read_u16(cursor)?;
    let state = read_u16(cursor)?;
    Ok(Block { id, state })
}

fn read_tree(cursor: &mut Cursor<&[u8]>) -> Result<TreePlan, String> {
    let base_x = read_i32(cursor)?;
    let base_z = read_i32(cursor)?;
    let surface_y = read_i32(cursor)?;
    let trunk_height = read_i32(cursor)?;
    let species_id = read_u8(cursor)?;
    let species = TreeSpecies::from_u8(species_id)
        .ok_or_else(|| format!("invalid tree species id {}", species_id))?;
    let trunk_block = read_block(cursor)?;
    let leaves_block = read_block(cursor)?;
    Ok(TreePlan {
        base_x,
        base_z,
        surface_y,
        trunk_height,
        species,
        trunk_block,
        leaves_block,
    })
}

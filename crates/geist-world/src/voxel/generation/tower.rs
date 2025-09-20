use std::time::Instant;

use geist_blocks::registry::BlockRegistry;
use geist_blocks::types::Block as RtBlock;

use super::super::World;
use super::super::gen_ctx::{TerrainProfiler, TerrainStage};

pub const TOWER_OUTER_RADIUS: i32 = 12;
pub const TOWER_INNER_RADIUS: i32 = 7;
pub const TOWER_TOP: i32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TowerMaterial {
    None,
    Air,
    Stone,
    Glass,
    Glowstone,
}

#[inline]
pub fn tower_material(dist2: i64, y: i32) -> TowerMaterial {
    let outer_sq = (TOWER_OUTER_RADIUS as i64).pow(2);
    if dist2 > outer_sq {
        return TowerMaterial::None;
    }
    if y >= TOWER_TOP {
        return TowerMaterial::Air;
    }
    let inner_sq = (TOWER_INNER_RADIUS as i64).pow(2);
    if dist2 <= inner_sq {
        if y % 32 == 0 {
            TowerMaterial::Stone
        } else {
            TowerMaterial::Air
        }
    } else {
        let band = y.rem_euclid(128);
        if band < 6 {
            TowerMaterial::Glowstone
        } else if band < 24 {
            TowerMaterial::Glass
        } else {
            TowerMaterial::Stone
        }
    }
}

pub fn evaluate_tower(
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
    let tower_center_x = (world.world_size_x() as i32) / 2;
    let tower_center_z = (world.world_size_z() as i32) / 2;
    let dx = x - tower_center_x;
    let dz = z - tower_center_z;
    let dist2 = (dx as i64).pow(2) + (dz as i64).pow(2);
    let result = match tower_material(dist2, y) {
        TowerMaterial::None => None,
        TowerMaterial::Air => Some(air),
        TowerMaterial::Stone => {
            let id = world.resolve_block_id(reg, "stone");
            Some(RtBlock { id, state: 0 })
        }
        TowerMaterial::Glass => {
            let id = world.resolve_block_id(reg, "glass");
            Some(RtBlock { id, state: 0 })
        }
        TowerMaterial::Glowstone => {
            let id = world.resolve_block_id(reg, "glowstone");
            Some(RtBlock { id, state: 0 })
        }
    };
    profiler.record_stage_duration(TerrainStage::Tower, stage_start.elapsed());
    result
}

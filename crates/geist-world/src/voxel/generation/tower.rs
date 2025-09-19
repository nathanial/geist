use std::time::Instant;

use geist_blocks::registry::BlockRegistry;
use geist_blocks::types::Block as RtBlock;

use super::super::World;
use super::super::gen_ctx::{TerrainProfiler, TerrainStage};

pub(super) fn evaluate_tower(
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

use std::sync::Arc;

use geist_blocks::Block;
use geist_blocks::BlockRegistry;
use geist_geom::Vec3;
use geist_render_raylib::conv::vec3_to_rl;
use geist_structures::{Pose, Structure, StructureEditStore, StructureId};

use crate::event::{Event, EventQueue};

use super::DayLightSample;

pub const SUN_STRUCTURE_ID: StructureId = 1;

const SUN_DISTANCE: f32 = 260.0;
const SUN_DIAMETER_BLOCKS: usize = 48;
const SUN_SHELL_THICKNESS: f32 = 2.5;

pub struct SunBody {
    pub id: StructureId,
    distance: f32,
    last_pos: Vec3,
}

impl SunBody {
    pub fn new(
        id: StructureId,
        reg: &BlockRegistry,
        cam_pos: Vec3,
        sample: &DayLightSample,
    ) -> Option<(Self, Structure)> {
        let sun_block_id = reg.id_by_name("sun_core")?;
        let sun_block = Block {
            id: sun_block_id,
            state: 0,
        };
        let blocks = build_sun_shell(sun_block);
        let initial_pos = cam_pos + sample.sun_dir * SUN_DISTANCE;
        let structure = Structure {
            id,
            sx: SUN_DIAMETER_BLOCKS,
            sy: SUN_DIAMETER_BLOCKS,
            sz: SUN_DIAMETER_BLOCKS,
            blocks: Arc::from(blocks.into_boxed_slice()),
            edits: StructureEditStore::new(),
            pose: Pose {
                pos: initial_pos,
                yaw_deg: 0.0,
            },
            last_delta: Vec3::ZERO,
            dirty_rev: 1,
            built_rev: 0,
        };
        let body = Self {
            id,
            distance: SUN_DISTANCE,
            last_pos: initial_pos,
        };
        Some((body, structure))
    }

    #[inline]
    pub fn target_position(&self, cam_pos: Vec3, sample: &DayLightSample) -> Vec3 {
        cam_pos + sample.sun_dir * self.distance
    }

    pub fn update_pose(&mut self, queue: &mut EventQueue, target: Vec3) {
        let delta = target - self.last_pos;
        if delta.length() < 0.05 {
            return;
        }
        self.last_pos = target;
        queue.emit_now(Event::StructurePoseUpdated {
            id: self.id,
            pos: vec3_to_rl(target),
            yaw_deg: 0.0,
            delta: vec3_to_rl(delta),
        });
    }
}

fn build_sun_shell(fill: Block) -> Vec<Block> {
    let side = SUN_DIAMETER_BLOCKS;
    let mut blocks = vec![Block::AIR; side * side * side];
    let center = (side as f32 - 1.0) * 0.5;
    let radius = center;
    let inner = (radius - SUN_SHELL_THICKNESS).max(0.0);
    let radius_sq = radius * radius;
    let inner_sq = inner * inner;
    for y in 0..side {
        for z in 0..side {
            for x in 0..side {
                let dx = x as f32 - center;
                let dy = y as f32 - center;
                let dz = z as f32 - center;
                let d2 = dx * dx + dy * dy + dz * dz;
                if d2 <= radius_sq && d2 >= inner_sq {
                    let idx = (y * side + z) * side + x;
                    blocks[idx] = fill;
                }
            }
        }
    }
    blocks
}

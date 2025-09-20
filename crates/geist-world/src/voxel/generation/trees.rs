use std::time::Instant;

use geist_blocks::registry::BlockRegistry;
use geist_blocks::types::Block;

use super::super::World;
use super::super::gen_ctx::TerrainStage;
use super::column_sampler::ColumnSampler;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeSpecies {
    Oak,
    Birch,
    Spruce,
    Jungle,
    Acacia,
    DarkOak,
}

impl TreeSpecies {
    #[inline]
    fn trunk_block_name(self) -> &'static str {
        match self {
            TreeSpecies::Oak => "oak_log",
            TreeSpecies::Birch => "birch_log",
            TreeSpecies::Spruce => "spruce_log",
            TreeSpecies::Jungle => "jungle_log",
            TreeSpecies::Acacia => "acacia_log",
            TreeSpecies::DarkOak => "dark_oak_log",
        }
    }

    #[inline]
    fn leaves_block_name(self) -> &'static str {
        match self {
            TreeSpecies::Oak => "oak_leaves",
            TreeSpecies::Birch => "birch_leaves",
            TreeSpecies::Spruce => "spruce_leaves",
            TreeSpecies::Jungle => "jungle_leaves",
            TreeSpecies::Acacia => "acacia_leaves",
            TreeSpecies::DarkOak => "oak_leaves",
        }
    }
}

#[derive(Clone, Debug)]
pub struct TreePlan {
    pub base_x: i32,
    pub base_z: i32,
    pub surface_y: i32,
    pub trunk_height: i32,
    pub species: TreeSpecies,
    pub trunk_block: Block,
    pub leaves_block: Block,
}

pub(super) fn plan_tree_for_column<'p>(
    world: &World,
    sampler: &mut ColumnSampler<'_, 'p>,
    reg: &BlockRegistry,
    x: i32,
    z: i32,
    column_height: i32,
) -> Option<TreePlan> {
    let params = sampler.params;
    let tree_prob = sampler.tree_probability(x, z);
    let world_height = sampler.world_height();
    let seed = world.seed as u32;
    let (surface_y, trunk_height, species) = trunk_info(
        sampler,
        x,
        z,
        tree_prob,
        params.trunk_min,
        params.trunk_max,
        world_height,
        seed,
        Some(column_height),
    )?;

    let trunk_block = Block {
        id: world.resolve_block_id(reg, species.trunk_block_name()),
        state: 0,
    };
    let leaves_block = Block {
        id: world.resolve_block_id(reg, species.leaves_block_name()),
        state: 0,
    };

    Some(TreePlan {
        base_x: x,
        base_z: z,
        surface_y,
        trunk_height,
        species,
        trunk_block,
        leaves_block,
    })
}

pub(super) fn apply_tree_blocks<'p>(
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
        None,
    ) {
        if y > surf && y <= surf + th {
            *base = match sp {
                TreeSpecies::Oak => "oak_log",
                TreeSpecies::Birch => "birch_log",
                TreeSpecies::Spruce => "spruce_log",
                TreeSpecies::Jungle => "jungle_log",
                TreeSpecies::Acacia => "acacia_log",
                TreeSpecies::DarkOak => "dark_oak_log",
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
                    None,
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
                            TreeSpecies::Oak => "oak_leaves",
                            TreeSpecies::Birch => "birch_leaves",
                            TreeSpecies::Spruce => "spruce_leaves",
                            TreeSpecies::Jungle => "jungle_leaves",
                            TreeSpecies::Acacia => "acacia_leaves",
                            TreeSpecies::DarkOak => "oak_leaves",
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
) -> TreeSpecies {
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
                            "oak" => TreeSpecies::Oak,
                            "birch" => TreeSpecies::Birch,
                            "spruce" => TreeSpecies::Spruce,
                            "jungle" => TreeSpecies::Jungle,
                            "acacia" => TreeSpecies::Acacia,
                            "dark_oak" => TreeSpecies::DarkOak,
                            _ => TreeSpecies::Oak,
                        };
                    }
                }
            }
        }
    }
    let t = rand01_tree(seed, tx, tz, 0xBEEF01);
    let m = rand01_tree(seed, tx, tz, 0xC0FFEE);
    if t < 0.22 && m > 0.65 {
        return TreeSpecies::Spruce;
    }
    if t > 0.78 && m > 0.45 {
        return TreeSpecies::Jungle;
    }
    if t > 0.75 && m < 0.32 {
        return TreeSpecies::Acacia;
    }
    if t > 0.65 && m < 0.25 {
        return TreeSpecies::DarkOak;
    }
    if ((hash2_tree(tx, tz, 0xDEAD_BEEF) >> 20) & 1) == 1 {
        TreeSpecies::Birch
    } else {
        TreeSpecies::Oak
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
    height_override: Option<i32>,
) -> Option<(i32, i32, TreeSpecies)> {
    let height = height_override.unwrap_or_else(|| sampler.height_for(tx, tz));
    let surf = height - 1;
    let surf_block = sampler.top_block_for_column(tx, tz, height);
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

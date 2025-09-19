use std::time::Instant;

use super::super::World;
use super::super::gen_ctx::TerrainStage;
use super::column_sampler::ColumnSampler;

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

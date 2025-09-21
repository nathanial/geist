use std::collections::HashMap;
use std::time::Instant;

use fastnoise_lite::FastNoiseLite;
use geist_blocks::registry::BlockRegistry;
use geist_blocks::types::Block;

use crate::worldgen::{Fractal, WorldGenParams};

use super::super::World;
use super::super::gen_ctx::TerrainStage;
use super::column_sampler::ColumnSampler;

#[derive(Default)]
pub struct BlockLookup {
    cache: HashMap<String, Block>,
}

impl BlockLookup {
    #[inline]
    pub fn resolve(&mut self, world: &World, reg: &BlockRegistry, name: &str) -> Block {
        if let Some(block) = self.cache.get(name) {
            *block
        } else {
            let block = Block {
                id: world.resolve_block_id(reg, name),
                state: 0,
            };
            self.cache.insert(name.to_string(), block);
            block
        }
    }
}

pub(crate) fn apply_caves_and_features<'p>(
    world: &World,
    sampler: &mut ColumnSampler<'_, 'p>,
    x: i32,
    y: i32,
    z: i32,
    height: i32,
    base: &mut &'p str,
) -> bool {
    sampler.profiler_mut().begin_stage(TerrainStage::Caves);
    let stage_start = Instant::now();
    let params = sampler.params;
    let world_height = sampler.world_height();
    let world_height_f = sampler.world_height_f();
    let mut carved_here = false;

    if matches!(*base, "stone" | "dirt" | "sand" | "snow" | "glowstone") {
        let y_scale = params.y_scale;
        let eps_base = params.eps_base;
        let eps_add = params.eps_add;
        let warp_xy = params.warp_xy;
        let warp_y = params.warp_y;
        let room_cell = params.room_cell;
        let room_thr_base = params.room_thr_base;
        let room_thr_add = params.room_thr_add;
        let soil_min = params.soil_min;
        let min_y = params.min_y;

        let h = height as f32;
        let wy = y as f32;
        let soil = h - wy;
        if params.carvers_enable && soil > soil_min && wy > min_y {
            let wx = x as f32;
            let wyf = y as f32;
            let wz = z as f32;
            let wxw = fractal3(&sampler.ctx.warp, wx, wyf, wz, &params.warp);
            let wyw = fractal3(
                &sampler.ctx.warp,
                wx + 133.7,
                wyf + 71.3,
                wz - 19.1,
                &params.warp,
            );
            let wzw = fractal3(
                &sampler.ctx.warp,
                wx - 54.2,
                wyf + 29.7,
                wz + 88.8,
                &params.warp,
            );
            let xp = wx + wxw * warp_xy;
            let yp = wyf + wyw * warp_y;
            let zp = wz + wzw * warp_xy;
            let tn = fractal3(&sampler.ctx.tunnel, xp, yp * y_scale, zp, &params.tunnel);
            let depth01 = (soil / world_height_f).clamp(0.0, 1.0);
            let eps = eps_base + eps_add * depth01;
            let wn = worley3_f1_norm(world.seed as u32, xp, yp, zp, room_cell);
            let room_thr = room_thr_base + room_thr_add * depth01;
            let carved_air = (tn.abs() < eps) || (wn < room_thr);
            if carved_air {
                *base = "air";
                carved_here = true;
            }

            let mut near_solid_cache: Option<bool> = None;
            let features = &params.features;
            if !features.is_empty() {
                for (ri, rule) in features.iter().enumerate() {
                    let w = &rule.when;
                    if !w.base_in.is_empty() && !w.base_in.iter().any(|s| s.as_str() == *base) {
                        continue;
                    }
                    if !w.base_not_in.is_empty()
                        && w.base_not_in.iter().any(|s| s.as_str() == *base)
                    {
                        continue;
                    }
                    if let Some(ymin) = w.y_min {
                        if y < ymin {
                            continue;
                        }
                    }
                    if let Some(ymax) = w.y_max {
                        if y > ymax {
                            continue;
                        }
                    }
                    if let Some(off) = w.below_height_offset {
                        if y >= height - off {
                            continue;
                        }
                    }
                    if let Some(req) = w.in_carved {
                        if req != carved_here {
                            continue;
                        }
                    }
                    if let Some(req) = w.near_solid {
                        if req
                            != compute_near_solid(
                                sampler,
                                &mut near_solid_cache,
                                world.seed as u32,
                                x,
                                y,
                                z,
                                params,
                                world_height,
                                world_height_f,
                                y_scale,
                                warp_xy,
                                warp_y,
                                eps_base,
                                eps_add,
                                room_cell,
                                room_thr_base,
                                room_thr_add,
                                soil_min,
                                min_y,
                            )
                        {
                            continue;
                        }
                    }
                    if let Some(p) = w.chance {
                        if p < 1.0 {
                            let salt = ((world.seed as u32).wrapping_add(0xC0FF_EE15))
                                .wrapping_add(ri as u32 * 0x9E37_79B9);
                            let h = hash3_feature(x, y, z, salt) & 0x00FF_FFFF;
                            let r = (h as f32) / 16_777_216.0;
                            if r >= p {
                                continue;
                            }
                        }
                    }
                    *base = rule.place.block.as_str();
                    break;
                }
            }
        }
    }

    let carved = carved_here;
    sampler
        .profiler_mut()
        .record_stage_duration(TerrainStage::Caves, stage_start.elapsed());
    carved
}

pub fn apply_caves_and_features_blocks<'p>(
    world: &World,
    sampler: &mut ColumnSampler<'_, 'p>,
    reg: &BlockRegistry,
    lookup: &mut BlockLookup,
    x: i32,
    y: i32,
    z: i32,
    height: i32,
    base: &mut Block,
) -> bool {
    sampler.profiler_mut().begin_stage(TerrainStage::Caves);
    let stage_start = Instant::now();
    let params = sampler.params;
    let world_height = sampler.world_height();
    let world_height_f = sampler.world_height_f();
    let mut carved_here = false;
    let mut base_block = *base;

    if is_carvable_block(reg, base_block) {
        let y_scale = params.y_scale;
        let eps_base = params.eps_base;
        let eps_add = params.eps_add;
        let warp_xy = params.warp_xy;
        let warp_y = params.warp_y;
        let room_cell = params.room_cell;
        let room_thr_base = params.room_thr_base;
        let room_thr_add = params.room_thr_add;
        let soil_min = params.soil_min;
        let min_y = params.min_y;

        let h = height as f32;
        let wy = y as f32;
        let soil = h - wy;
        if params.carvers_enable && soil > soil_min && wy > min_y {
            let wx = x as f32;
            let wyf = y as f32;
            let wz = z as f32;
            let wxw = fractal3(&sampler.ctx.warp, wx, wyf, wz, &params.warp);
            let wyw = fractal3(
                &sampler.ctx.warp,
                wx + 133.7,
                wyf + 71.3,
                wz - 19.1,
                &params.warp,
            );
            let wzw = fractal3(
                &sampler.ctx.warp,
                wx - 54.2,
                wyf + 29.7,
                wz + 88.8,
                &params.warp,
            );
            let xp = wx + wxw * warp_xy;
            let yp = wyf + wyw * warp_y;
            let zp = wz + wzw * warp_xy;
            let tn = fractal3(&sampler.ctx.tunnel, xp, yp * y_scale, zp, &params.tunnel);
            let depth01 = (soil / world_height_f).clamp(0.0, 1.0);
            let eps = eps_base + eps_add * depth01;
            let wn = worley3_f1_norm(world.seed as u32, xp, yp, zp, room_cell);
            let room_thr = room_thr_base + room_thr_add * depth01;
            let carved_air = (tn.abs() < eps) || (wn < room_thr);
            if carved_air {
                base_block = lookup.resolve(world, reg, "air");
                carved_here = true;
            }

            let mut near_solid_cache: Option<bool> = None;
            let features = &params.features;
            if !features.is_empty() {
                for (ri, rule) in features.iter().enumerate() {
                    let w = &rule.when;
                    let current_name = block_name(reg, base_block);
                    if !w.base_in.is_empty()
                        && !w.base_in.iter().any(|s| s.as_str() == current_name)
                    {
                        continue;
                    }
                    if !w.base_not_in.is_empty()
                        && w.base_not_in.iter().any(|s| s.as_str() == current_name)
                    {
                        continue;
                    }
                    if let Some(ymin) = w.y_min {
                        if y < ymin {
                            continue;
                        }
                    }
                    if let Some(ymax) = w.y_max {
                        if y > ymax {
                            continue;
                        }
                    }
                    if let Some(off) = w.below_height_offset {
                        if y >= height - off {
                            continue;
                        }
                    }
                    if let Some(req) = w.in_carved {
                        if req != carved_here {
                            continue;
                        }
                    }
                    if let Some(req) = w.near_solid {
                        if req
                            != compute_near_solid(
                                sampler,
                                &mut near_solid_cache,
                                world.seed as u32,
                                x,
                                y,
                                z,
                                params,
                                world_height,
                                world_height_f,
                                y_scale,
                                warp_xy,
                                warp_y,
                                eps_base,
                                eps_add,
                                room_cell,
                                room_thr_base,
                                room_thr_add,
                                soil_min,
                                min_y,
                            )
                        {
                            continue;
                        }
                    }
                    if let Some(p) = w.chance {
                        if p < 1.0 {
                            let salt = ((world.seed as u32).wrapping_add(0xC0FF_EE15))
                                .wrapping_add(ri as u32 * 0x9E37_79B9);
                            let h = hash3_feature(x, y, z, salt) & 0x00FF_FFFF;
                            let r = (h as f32) / 16_777_216.0;
                            if r >= p {
                                continue;
                            }
                        }
                    }
                    base_block = lookup.resolve(world, reg, rule.place.block.as_str());
                    break;
                }
            }
        }
    }

    let carved = carved_here;
    sampler
        .profiler_mut()
        .record_stage_duration(TerrainStage::Caves, stage_start.elapsed());
    *base = base_block;
    carved
}

fn compute_near_solid<'p>(
    sampler: &mut ColumnSampler<'_, 'p>,
    cache: &mut Option<bool>,
    world_seed: u32,
    x: i32,
    y: i32,
    z: i32,
    params: &WorldGenParams,
    world_height: i32,
    world_height_f: f32,
    y_scale: f32,
    warp_xy: f32,
    warp_y: f32,
    eps_base: f32,
    eps_add: f32,
    room_cell: f32,
    room_thr_base: f32,
    room_thr_add: f32,
    soil_min: f32,
    min_y: f32,
) -> bool {
    if let Some(value) = *cache {
        return value;
    }
    let mut near_solid = false;
    for (dx, dy, dz) in [
        (-1, 0, 0),
        (1, 0, 0),
        (0, -1, 0),
        (0, 1, 0),
        (0, 0, -1),
        (0, 0, 1),
    ] {
        let nx = x + dx;
        let ny = y + dy;
        let nz = z + dz;
        if ny < 0 || ny >= world_height {
            continue;
        }
        let nh = sampler.height_for(nx, nz);
        if ny >= nh {
            continue;
        }
        let wxn = nx as f32;
        let wyn = ny as f32;
        let wzn = nz as f32;
        // PERF: Neighbor checks rerun the same noise stack as the main voxel; cache if profiling shows spikes.
        let wxw_n = fractal3(&sampler.ctx.warp, wxn, wyn, wzn, &params.warp);
        let wyw_n = fractal3(
            &sampler.ctx.warp,
            wxn + 133.7,
            wyn + 71.3,
            wzn - 19.1,
            &params.warp,
        );
        let wzw_n = fractal3(
            &sampler.ctx.warp,
            wxn - 54.2,
            wyn + 29.7,
            wzn + 88.8,
            &params.warp,
        );
        let nxp = wxn + wxw_n * warp_xy;
        let nyp = wyn + wyw_n * warp_y;
        let nzp = wzn + wzw_n * warp_xy;
        let tn_n = fractal3(&sampler.ctx.tunnel, nxp, nyp * y_scale, nzp, &params.tunnel);
        let nsoil = nh as f32 - wyn;
        let n_depth = (nsoil / world_height_f).clamp(0.0, 1.0);
        let eps_n = eps_base + eps_add * n_depth;
        let wn_n = worley3_f1_norm(world_seed, nxp, nyp, nzp, room_cell);
        let room_thr_n = room_thr_base + room_thr_add * n_depth;
        let neighbor_carved_air =
            (nsoil > soil_min && wyn > min_y) && (tn_n.abs() < eps_n || wn_n < room_thr_n);
        if !neighbor_carved_air {
            near_solid = true;
            break;
        }
    }
    *cache = Some(near_solid);
    near_solid
}

#[inline]
fn block_name<'a>(reg: &'a BlockRegistry, block: Block) -> &'a str {
    reg.get(block.id)
        .map(|ty| ty.name.as_str())
        .unwrap_or("unknown")
}

#[inline]
fn is_carvable_block(reg: &BlockRegistry, block: Block) -> bool {
    matches!(
        block_name(reg, block),
        "stone" | "dirt" | "sand" | "snow" | "glowstone"
    )
}

fn fractal3(noise: &FastNoiseLite, x: f32, y: f32, z: f32, fractal: &Fractal) -> f32 {
    // PERF: Each call runs multiple octaves for the same coordinates; reuse when stepping coherently.
    let mut amp = 1.0_f32;
    let mut freq = 1.0_f32 / fractal.scale.max(0.0001);
    let mut sum = 0.0_f32;
    let mut max_amp = 0.0_f32;
    for _ in 0..fractal.octaves.max(1) {
        sum += noise.get_noise_3d(x * freq, y * freq, z * freq) * amp;
        max_amp += amp;
        amp *= fractal.persistence;
        freq *= fractal.lacunarity;
    }
    if max_amp > 0.0 { sum / max_amp } else { sum }
}

fn worley3_f1_norm(seed: u32, x: f32, y: f32, z: f32, cell: f32) -> f32 {
    // PERF: Worley lookup scans 27 pseudo-random cells; avoid in tight loops if possible.
    let cell = if cell <= 0.0001 { 1.0 } else { cell };
    let px = x / cell;
    let py = y / cell;
    let pz = z / cell;
    let ix = px.floor() as i32;
    let iy = py.floor() as i32;
    let iz = pz.floor() as i32;
    let fx = px - ix as f32;
    let fy = py - iy as f32;
    let fz = pz - iz as f32;
    let mut min_d2 = f32::INFINITY;
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let cx = ix + dx;
                let cy = iy + dy;
                let cz = iz + dz;
                let jx = rand01_cell(seed, cx, cy, cz, 0x068b_c021);
                let jy = rand01_cell(seed, cx, cy, cz, 0x02e1_b213);
                let jz = rand01_cell(seed, cx, cy, cz, 0x0f1a_1234);
                let dx = (dx as f32 + jx) - fx;
                let dy = (dy as f32 + jy) - fy;
                let dz = (dz as f32 + jz) - fz;
                let d2 = dx * dx + dy * dy + dz * dz;
                if d2 < min_d2 {
                    min_d2 = d2;
                }
            }
        }
    }
    (min_d2.sqrt()).min(1.0)
}

fn rand01_cell(seed: u32, cx: i32, cy: i32, cz: i32, salt: u32) -> f32 {
    let h = hash3_carver(cx, cy, cz, seed ^ salt);
    (h & 0x00FF_FFFF) as f32 / 16_777_216.0
}

fn hash3_carver(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    fn uhash32(mut a: u32) -> u32 {
        a ^= a >> 16;
        a = a.wrapping_mul(0x7feb_352d);
        a ^= a >> 15;
        a = a.wrapping_mul(0x846c_a68b);
        a ^= a >> 16;
        a
    }
    let ux = x as u32;
    let uy = y as u32;
    let uz = z as u32;
    let mut h = seed ^ 0x9e37_79b9;
    h ^= uhash32(ux.wrapping_add(0x85eb_ca6b));
    h ^= uhash32(uy.wrapping_add(0xc2b2_ae35));
    h ^= uhash32(uz.wrapping_add(0x27d4_eb2f));
    uhash32(h)
}

fn hash3_feature(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    let mix = |mut v: u32| {
        v ^= v >> 16;
        v = v.wrapping_mul(0x7feb_352d);
        v ^= v >> 15;
        v = v.wrapping_mul(0x846c_a68b);
        v ^= v >> 16;
        v
    };
    let mut a = seed ^ 0x9e37_79b9;
    a ^= mix(x as u32);
    a ^= mix(y as u32);
    a ^= mix(z as u32);
    a
}

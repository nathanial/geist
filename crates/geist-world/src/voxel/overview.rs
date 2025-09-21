use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crate::voxel::generation::{ColumnSampler, caves::apply_caves_and_features};
use crate::voxel::{GenCtx, World};
use crate::worldgen::WorldGenParams;

#[derive(Clone, Copy, Debug)]
pub struct OverviewRegion {
    pub min_x: i32,
    pub min_z: i32,
    pub max_x: i32,
    pub max_z: i32,
}

impl OverviewRegion {
    pub fn new(min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> Result<Self, OverviewError> {
        if min_x >= max_x || min_z >= max_z {
            return Err(OverviewError::InvalidRegion(
                "region max must be greater than min",
            ));
        }
        Ok(Self {
            min_x,
            min_z,
            max_x,
            max_z,
        })
    }

    #[inline]
    pub fn width(&self) -> usize {
        (self.max_x - self.min_x) as usize
    }

    #[inline]
    pub fn height(&self) -> usize {
        (self.max_z - self.min_z) as usize
    }
}

#[derive(Clone, Copy, Debug)]
pub enum OverviewMode {
    HeightMap,
    BiomeMap,
    CavePreview,
}

#[derive(Debug)]
pub enum OverviewError {
    InvalidRegion(&'static str),
    ThreadPanicked,
}

pub struct WorldOverview {
    world: Arc<World>,
}

#[derive(Clone, Debug)]
pub struct WorldOverviewImage {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl WorldOverviewImage {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            data: vec![0; width * height * 3],
        }
    }

    #[inline]
    pub fn put_pixel(&mut self, x: usize, y: usize, rgb: [u8; 3]) {
        let idx = (y * self.width + x) * 3;
        self.data[idx..idx + 3].copy_from_slice(&rgb);
    }
}

pub struct WorldOverviewJob {
    handle: JoinHandle<Result<WorldOverviewImage, OverviewError>>,
}

impl WorldOverviewJob {
    pub fn join(self) -> Result<WorldOverviewImage, OverviewError> {
        match self.handle.join() {
            Ok(res) => res,
            Err(_) => Err(OverviewError::ThreadPanicked),
        }
    }
}

impl WorldOverview {
    pub fn new(world: Arc<World>) -> Self {
        Self { world }
    }

    pub fn spawn_region(&self, region: OverviewRegion, mode: OverviewMode) -> WorldOverviewJob {
        let world = Arc::clone(&self.world);
        let handle = thread::spawn(move || {
            let overview = WorldOverview { world };
            overview.generate_region(region, mode)
        });
        WorldOverviewJob { handle }
    }

    pub fn generate_region(
        &self,
        region: OverviewRegion,
        mode: OverviewMode,
    ) -> Result<WorldOverviewImage, OverviewError> {
        if region.min_x >= region.max_x || region.min_z >= region.max_z {
            return Err(OverviewError::InvalidRegion(
                "region max must be greater than min",
            ));
        }
        let width = region.width();
        let height = region.height();
        let mut image = WorldOverviewImage::new(width, height);
        let mut ctx = self.world.make_gen_ctx();
        let params_guard: Arc<WorldGenParams> = Arc::clone(&ctx.params);
        let params = &*params_guard;
        let world_height = self.world.world_height_hint() as i32;
        let water_level = if params.water_enable {
            (world_height as f32 * params.water_level_ratio).round() as i32
        } else {
            -1
        };
        match mode {
            OverviewMode::HeightMap => {
                self.render_height_map(region, water_level, world_height, &mut ctx, &mut image)?;
            }
            OverviewMode::BiomeMap => {
                self.render_biome_map(region, &mut ctx, &mut image)?;
            }
            OverviewMode::CavePreview => {
                self.render_cave_preview(region, params, &mut ctx, &mut image)?;
            }
        }
        Ok(image)
    }

    fn render_height_map(
        &self,
        region: OverviewRegion,
        water_level: i32,
        world_height: i32,
        ctx: &mut GenCtx,
        image: &mut WorldOverviewImage,
    ) -> Result<(), OverviewError> {
        let chunk_sx = self.world.chunk_size_x as i32;
        let chunk_sz = self.world.chunk_size_z as i32;
        let min_tile_x = region.min_x.div_euclid(chunk_sx) * chunk_sx;
        let min_tile_z = region.min_z.div_euclid(chunk_sz) * chunk_sz;
        let max_tile_x = (region.max_x - 1).div_euclid(chunk_sx) * chunk_sx;
        let max_tile_z = (region.max_z - 1).div_euclid(chunk_sz) * chunk_sz;
        let mut tile_z = min_tile_z;
        while tile_z <= max_tile_z {
            let mut tile_x = min_tile_x;
            while tile_x <= max_tile_x {
                self.world.prepare_height_tile(
                    ctx,
                    tile_x,
                    tile_z,
                    chunk_sx as usize,
                    chunk_sz as usize,
                );
                if let Some(tile) = ctx.height_tile.as_ref() {
                    for dz in 0..chunk_sz {
                        let world_z = tile_z + dz;
                        if world_z < region.min_z || world_z >= region.max_z {
                            continue;
                        }
                        for dx in 0..chunk_sx {
                            let world_x = tile_x + dx;
                            if world_x < region.min_x || world_x >= region.max_x {
                                continue;
                            }
                            if let Some(height) = tile.height(world_x, world_z) {
                                let color = height_color(height, water_level, world_height);
                                let px = (world_x - region.min_x) as usize;
                                let py = (world_z - region.min_z) as usize;
                                image.put_pixel(px, py, color);
                            }
                        }
                    }
                }
                tile_x += chunk_sx;
            }
            tile_z += chunk_sz;
        }
        Ok(())
    }

    fn render_biome_map(
        &self,
        region: OverviewRegion,
        ctx: &mut GenCtx,
        image: &mut WorldOverviewImage,
    ) -> Result<(), OverviewError> {
        let chunk_sx = self.world.chunk_size_x as i32;
        let chunk_sz = self.world.chunk_size_z as i32;
        let min_tile_x = region.min_x.div_euclid(chunk_sx) * chunk_sx;
        let min_tile_z = region.min_z.div_euclid(chunk_sz) * chunk_sz;
        let max_tile_x = (region.max_x - 1).div_euclid(chunk_sx) * chunk_sx;
        let max_tile_z = (region.max_z - 1).div_euclid(chunk_sz) * chunk_sz;
        let mut tile_z = min_tile_z;
        while tile_z <= max_tile_z {
            let mut tile_x = min_tile_x;
            while tile_x <= max_tile_x {
                self.world.prepare_height_tile(
                    ctx,
                    tile_x,
                    tile_z,
                    chunk_sx as usize,
                    chunk_sz as usize,
                );
                for dz in 0..chunk_sz {
                    let world_z = tile_z + dz;
                    if world_z < region.min_z || world_z >= region.max_z {
                        continue;
                    }
                    for dx in 0..chunk_sx {
                        let world_x = tile_x + dx;
                        if world_x < region.min_x || world_x >= region.max_x {
                            continue;
                        }
                        let color = biome_color(self.world.as_ref(), world_x, world_z);
                        let px = (world_x - region.min_x) as usize;
                        let py = (world_z - region.min_z) as usize;
                        image.put_pixel(px, py, color);
                    }
                }
                tile_x += chunk_sx;
            }
            tile_z += chunk_sz;
        }
        Ok(())
    }

    fn render_cave_preview(
        &self,
        region: OverviewRegion,
        params: &WorldGenParams,
        ctx: &mut GenCtx,
        image: &mut WorldOverviewImage,
    ) -> Result<(), OverviewError> {
        let chunk_sx = self.world.chunk_size_x as i32;
        let chunk_sz = self.world.chunk_size_z as i32;
        let min_tile_x = region.min_x.div_euclid(chunk_sx) * chunk_sx;
        let min_tile_z = region.min_z.div_euclid(chunk_sz) * chunk_sz;
        let max_tile_x = (region.max_x - 1).div_euclid(chunk_sx) * chunk_sx;
        let max_tile_z = (region.max_z - 1).div_euclid(chunk_sz) * chunk_sz;
        let mut tile_z = min_tile_z;
        while tile_z <= max_tile_z {
            let mut tile_x = min_tile_x;
            while tile_x <= max_tile_x {
                self.world.prepare_height_tile(
                    ctx,
                    tile_x,
                    tile_z,
                    chunk_sx as usize,
                    chunk_sz as usize,
                );
                let tile = ctx.height_tile.clone();
                if let Some(tile) = tile {
                    for dz in 0..chunk_sz {
                        let world_z = tile_z + dz;
                        if world_z < region.min_z || world_z >= region.max_z {
                            continue;
                        }
                        for dx in 0..chunk_sx {
                            let world_x = tile_x + dx;
                            if world_x < region.min_x || world_x >= region.max_x {
                                continue;
                            }
                            let px = (world_x - region.min_x) as usize;
                            let py = (world_z - region.min_z) as usize;
                            let Some(column_height) = tile.height(world_x, world_z) else {
                                continue;
                            };
                            let mut carved_levels = 0;
                            for depth in [column_height - 10, column_height - 30] {
                                if depth <= 0 {
                                    continue;
                                }
                                let mut sampler =
                                    ColumnSampler::new(self.world.as_ref(), ctx, params);
                                let mut base = "stone";
                                if apply_caves_and_features(
                                    self.world.as_ref(),
                                    &mut sampler,
                                    world_x,
                                    depth,
                                    world_z,
                                    column_height,
                                    &mut base,
                                ) {
                                    carved_levels += 1;
                                } else if base == "air" {
                                    carved_levels += 1;
                                }
                            }
                            let color = cave_color(carved_levels);
                            image.put_pixel(px, py, color);
                        }
                    }
                }
                tile_x += chunk_sx;
            }
            tile_z += chunk_sz;
        }
        Ok(())
    }
}

fn height_color(height: i32, water_level: i32, world_height: i32) -> [u8; 3] {
    if water_level >= 0 && height <= water_level {
        let depth = (water_level - height).max(0) as f32;
        let denom = water_level.max(1) as f32;
        let d_norm = (depth / denom).clamp(0.0, 1.0);
        let blue = lerp(110, 200, d_norm);
        let green = lerp(30, 90, d_norm * 0.6);
        [0, green, blue]
    } else {
        let above = (height - water_level) as f32;
        let span = (world_height - water_level).max(1) as f32;
        let t = (above / span).clamp(0.0, 1.0);
        if t < 0.33 {
            let local = t / 0.33;
            lerp_color([34, 139, 34], [107, 142, 35], local)
        } else if t < 0.66 {
            let local = (t - 0.33) / 0.33;
            lerp_color([107, 142, 35], [139, 69, 19], local)
        } else {
            let local = (t - 0.66) / 0.34;
            lerp_color([139, 69, 19], [245, 245, 245], local)
        }
    }
}

fn biome_color(world: &World, wx: i32, wz: i32) -> [u8; 3] {
    if let Some(biome) = world.biome_at(wx, wz) {
        if let Some(tint) = biome.leaf_tint {
            return [
                (tint[0] * 255.0).clamp(0.0, 255.0) as u8,
                (tint[1] * 255.0).clamp(0.0, 255.0) as u8,
                (tint[2] * 255.0).clamp(0.0, 255.0) as u8,
            ];
        }
        hash_color(biome.name.as_str())
    } else {
        [80, 80, 80]
    }
}

fn cave_color(carved_levels: i32) -> [u8; 3] {
    match carved_levels {
        0 => [60, 60, 65],
        1 => [0, 170, 200],
        _ => [200, 80, 220],
    }
}

#[inline]
fn lerp(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn lerp_color(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    [
        lerp(a[0], b[0], t),
        lerp(a[1], b[1], t),
        lerp(a[2], b[2], t),
    ]
}

fn hash_color(name: &str) -> [u8; 3] {
    let mut hash = 0u32;
    for b in name.as_bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(u32::from(*b));
    }
    [
        (hash & 0xFF) as u8,
        ((hash >> 8) & 0xFF) as u8,
        ((hash >> 16) & 0xFF) as u8,
    ]
}

impl std::fmt::Display for OverviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OverviewError::InvalidRegion(msg) => write!(f, "invalid region: {}", msg),
            OverviewError::ThreadPanicked => write!(f, "overview job panicked"),
        }
    }
}

impl std::error::Error for OverviewError {}

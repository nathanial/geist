use fastnoise_lite::{FastNoiseLite, NoiseType};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum TreeSpecies {
    Oak,
    Birch,
    Spruce,
    Jungle,
    Acacia,
    DarkOak,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum TerracottaColor {
    White,
    Orange,
    Magenta,
    LightBlue,
    Yellow,
    Lime,
    Pink,
    Gray,
    LightGray,
    Cyan,
    Purple,
    Blue,
    Brown,
    Green,
    Red,
    Black,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Block {
    Air,
    // Placeholder for unrecognized/unsupported imported blocks
    Unknown,
    Grass,
    Dirt,
    Stone,
    Sand,
    Snow,
    Bookshelf,
    CoarseDirt,
    Podzol,
    Cobblestone,
    MossyCobblestone,
    StoneBricks,
    MossyStoneBricks,
    Brick,
    Granite,
    Diorite,
    Andesite,
    PolishedGranite,
    PolishedDiorite,
    PolishedAndesite,
    Gravel,
    SmoothStone,
    Sandstone,
    SmoothSandstone,
    RedSandstone,
    SmoothRedSandstone,
    QuartzBlock,
    LapisBlock,
    CoalBlock,
    PrismarineBricks,
    NetherBricks,
    EndStone,
    EndStoneBricks,
    Planks(TreeSpecies),
    Wood(TreeSpecies),
    LogAxis(TreeSpecies, Axis),
    Leaves(TreeSpecies),
    TerracottaPlain,
    Terracotta(TerracottaColor),
    QuartzPillar(Axis),
    // Special shapes (non-cubic)
    Slab {
        half: SlabHalf,
        key: MaterialKey,
    },
    Stairs {
        dir: Dir4,
        half: SlabHalf,
        key: MaterialKey,
    },
    Glowstone,
    Beacon,
}

impl Block {
    #[inline]
    pub fn is_solid(&self) -> bool {
        !matches!(self, Block::Air)
    }

    #[inline]
    pub fn emission(&self) -> u8 {
        match self {
            Block::Glowstone => 255,
            Block::Beacon => 255, // Beacon emits at level 255
            _ => 0,
        }
    }
}

pub struct World {
    pub chunk_size_x: usize,
    pub chunk_size_y: usize,
    pub chunk_size_z: usize,
    pub chunks_x: usize,
    pub chunks_z: usize,
    pub seed: i32,
    pub mode: WorldGenMode,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorldGenMode {
    Normal,
    // An infinite flat slab of stone of given thickness from y=0 upwards
    Flat { thickness: i32 },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Axis {
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum SlabHalf {
    Bottom,
    Top,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Dir4 {
    North,
    South,
    West,
    East,
}

// Material family for non-cubic shapes
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum MaterialKey {
    SmoothStone,
    Sandstone,
    RedSandstone,
    Cobblestone,
    MossyCobblestone,
    StoneBricks,
    MossyStoneBricks,
    QuartzBlock,
    Planks(TreeSpecies),
    PrismarineBricks,
    EndStone,
    EndStoneBricks,
    Granite,
    Diorite,
    Andesite,
    PolishedGranite,
    PolishedDiorite,
    PolishedAndesite,
}

impl World {
    pub fn new(
        chunks_x: usize,
        chunks_z: usize,
        chunk_size_x: usize,
        chunk_size_y: usize,
        chunk_size_z: usize,
        seed: i32,
        mode: WorldGenMode,
    ) -> Self {
        Self {
            chunk_size_x,
            chunk_size_y,
            chunk_size_z,
            chunks_x,
            chunks_z,
            seed,
            mode,
        }
    }

    #[inline]
    pub fn world_size_x(&self) -> usize {
        self.chunk_size_x * self.chunks_x
    }
    #[inline]
    pub fn world_size_y(&self) -> usize {
        self.chunk_size_y
    }
    #[inline]
    pub fn world_size_z(&self) -> usize {
        self.chunk_size_z * self.chunks_z
    }
}

impl World {
    // Procedural block at arbitrary world coordinates (x,z in world units, y in [0,chunk_size_y))
    pub fn block_at(&self, x: i32, y: i32, z: i32) -> Block {
        if y < 0 || y >= self.chunk_size_y as i32 {
            return Block::Air;
        }
        // Flat world shortcut: stone slab of configured thickness at base, otherwise air
        if let WorldGenMode::Flat { thickness } = self.mode {
            return if y < thickness {
                Block::Stone
            } else {
                Block::Air
            };
        }
        // Base terrain sampling (shared with trees)
        let mut noise = FastNoiseLite::with_seed(self.seed);
        noise.set_noise_type(Some(NoiseType::OpenSimplex2));
        noise.set_frequency(Some(0.02));
        let height_for = |wx: i32, wz: i32| {
            let h = noise.get_noise_2d(wx as f32, wz as f32);
            let min_h = (self.chunk_size_y as f32 * 0.15) as i32;
            let max_h = (self.chunk_size_y as f32 * 0.7) as i32;
            let hh = ((h + 1.0) * 0.5 * (max_h - min_h) as f32) as i32 + min_h;
            hh.clamp(1, self.chunk_size_y as i32 - 1)
        };
        let height = height_for(x, z);

        // Base terrain block
        let mut base_block = if y >= height {
            Block::Air
        } else if y == height - 1 {
            if height as f32 >= self.chunk_size_y as f32 * 0.62 {
                Block::Snow
            } else if height as f32 <= self.chunk_size_y as f32 * 0.2 {
                Block::Sand
            } else {
                Block::Grass
            }
        } else if y + 3 >= height {
            Block::Dirt
        } else {
            Block::Stone
        };

        // --- Cave carving (deterministic, noise-derived, no cross-chunk coordination) ---
        // Ported from old rastergeist-c (wg_should_carve): tunnels from warped 3D noise;
        // rooms from Worley F1. Carving only affects solid blocks sufficiently below surface.
        if matches!(
            base_block,
            Block::Stone | Block::Dirt | Block::Sand | Block::Snow | Block::Glowstone
        ) {
            // Parameters mirrored from old codegen defaults
            const Y_SCALE: f32 = 1.6; // vertical scaling for tunnel field
            const EPS_BASE: f32 = 0.04; // base tunnel band half-width
            const EPS_ADD: f32 = 0.08; // additional width with depth
            const WARP_XY: f32 = 5.0; // warp strength in X/Z
            const WARP_Y: f32 = 2.5; // warp strength in Y
            const ROOM_CELL: f32 = 120.0; // worley cell size for rooms
            const ROOM_THR_BASE: f32 = 0.12; // base room threshold
            const ROOM_THR_ADD: f32 = 0.12; // additional with depth
            const SOIL_MIN: f32 = 3.5; // don't carve within ~3.5 blocks of surface
            const MIN_Y: f32 = 2.0; // avoid near bedrock
            const GLOW_PROB: f32 = 0.0009; // low probability cave glowstones

            // Depth from surface (float)
            let h = height as f32; // our height is already clamped to world ceiling
            let wy = y as f32;
            let soil = h - wy;

            if soil > SOIL_MIN && wy > MIN_Y {
                // Normalized depth factor in [0,1]
                let mut depth_factor = soil / (self.chunk_size_y as f32);
                if depth_factor < 0.0 {
                    depth_factor = 0.0;
                }
                if depth_factor > 1.0 {
                    depth_factor = 1.0;
                }

                // Local helpers (manual fBm normalized to [-1,1])
                let fractal3 = |n: &FastNoiseLite,
                                x: f32,
                                y: f32,
                                z: f32,
                                oct: i32,
                                persistence: f32,
                                lacunarity: f32,
                                scale: f32|
                 -> f32 {
                    let mut amp = 1.0_f32;
                    let mut freq = 1.0_f32 / scale.max(0.0001);
                    let mut sum = 0.0_f32;
                    let mut max_amp = 0.0_f32;
                    for _ in 0..oct.max(1) {
                        sum += n.get_noise_3d(x * freq, y * freq, z * freq) * amp;
                        max_amp += amp;
                        amp *= persistence;
                        freq *= lacunarity;
                    }
                    if max_amp > 0.0 { sum / max_amp } else { sum }
                };

                // Hash utilities for Worley
                let uhash32 = |mut a: u32| -> u32 {
                    a ^= a >> 16;
                    a = a.wrapping_mul(0x7feb_352d);
                    a ^= a >> 15;
                    a = a.wrapping_mul(0x846c_a68b);
                    a ^= a >> 16;
                    a
                };
                let hash3 = |x: i32, y: i32, z: i32, seed: u32| -> u32 {
                    let ux = x as u32;
                    let uy = y as u32;
                    let uz = z as u32;
                    let mut h = seed ^ 0x9e37_79b9;
                    h ^= uhash32(ux.wrapping_add(0x85eb_ca6b));
                    h ^= uhash32(uy.wrapping_add(0xc2b2_ae35));
                    h ^= uhash32(uz.wrapping_add(0x27d4_eb2f));
                    uhash32(h)
                };
                let rand01_cell = |cx: i32, cy: i32, cz: i32, salt: u32| -> f32 {
                    let h = hash3(cx, cy, cz, salt);
                    (h & 0x00FF_FFFF) as f32 / 16_777_216.0
                };
                let worley3_f1_norm = |x: f32, y: f32, z: f32, cell: f32, seed: u32| -> f32 {
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
                                let jx = rand01_cell(cx, cy, cz, seed ^ 0x068b_c021);
                                let jy = rand01_cell(cx, cy, cz, seed ^ 0x02e1_b213);
                                let jz = rand01_cell(cx, cy, cz, seed ^ 0x097c_29f7);
                                let vx = dx as f32 + jx - fx;
                                let vy = dy as f32 + jy - fy;
                                let vz = dz as f32 + jz - fz;
                                let d2 = vx * vx + vy * vy + vz * vz;
                                if d2 < min_d2 {
                                    min_d2 = d2;
                                }
                            }
                        }
                    }
                    let d = min_d2.sqrt();
                    let half_diag = 0.866_025_4_f32; // sqrt(3)/2
                    let mut norm = d / half_diag;
                    if norm < 0.0 {
                        norm = 0.0;
                    }
                    if norm > 1.0 {
                        norm = 1.0;
                    }
                    norm
                };

                // Noise fields (seeded deterministically from world seed)
                let mut n_warp = FastNoiseLite::with_seed(self.seed ^ 2100);
                n_warp.set_noise_type(Some(NoiseType::OpenSimplex2));
                let mut n_tun = FastNoiseLite::with_seed(self.seed ^ 2101);
                n_tun.set_noise_type(Some(NoiseType::OpenSimplex2));

                // Warped sample positions
                let wx = x as f32;
                let wyf = wy;
                let wz = z as f32;
                let wxw = fractal3(&n_warp, wx, wyf, wz, 3, 0.6, 2.0, 220.0);
                let wyw = fractal3(
                    &n_warp,
                    wx + 133.7,
                    wyf + 71.3,
                    wz - 19.1,
                    3,
                    0.6,
                    2.0,
                    220.0,
                );
                let wzw = fractal3(
                    &n_warp,
                    wx - 54.2,
                    wyf + 29.7,
                    wz + 88.8,
                    3,
                    0.6,
                    2.0,
                    220.0,
                );
                let wxp = wx + wxw * WARP_XY;
                let wyp = wyf + wyw * WARP_Y;
                let wzp = wz + wzw * WARP_XY;

                // Tunnels: |tn| < eps (eps increases with depth)
                let tn = fractal3(&n_tun, wxp, (wyp) * Y_SCALE, wzp, 4, 0.55, 2.0, 140.0);
                let eps = EPS_BASE + EPS_ADD * depth_factor;
                let carve_tunnel = tn.abs() < eps;

                // Rooms: Worley F1 below threshold that increases with depth
                let worley_seed = ((self.seed as u32) ^ 2100u32).wrapping_add(1337);
                let wn = worley3_f1_norm(wxp, wyp, wzp, ROOM_CELL, worley_seed);
                let room_thr = ROOM_THR_BASE + ROOM_THR_ADD * depth_factor;
                let carve_room = wn < room_thr;

                if carve_tunnel || carve_room {
                    // Decide attachment-based glowstone placement in carved air.
                    // Check if any 6-neighbor remains solid after its own carve.
                    let neigh = [
                        (1, 0, 0),
                        (-1, 0, 0),
                        (0, 1, 0),
                        (0, -1, 0),
                        (0, 0, 1),
                        (0, 0, -1),
                    ];
                    let mut near_solid = false;
                    for (dx, dy, dz) in neigh {
                        let nx = x + dx;
                        let ny = y + dy;
                        let nz = z + dz;
                        if ny < 0 || ny >= self.chunk_size_y as i32 {
                            continue;
                        }
                        // Neighbor surface and base solidity
                        let nh = height_for(nx, nz) as f32;
                        if (ny as f32) >= nh {
                            continue;
                        } // neighbor is above its surface -> air
                        // Apply same carve test to neighbor
                        let nwy = ny as f32;
                        let nsoil = nh - nwy;
                        let mut n_depth = nsoil / (self.chunk_size_y as f32);
                        if n_depth < 0.0 {
                            n_depth = 0.0;
                        }
                        if n_depth > 1.0 {
                            n_depth = 1.0;
                        }
                        let wxn = nx as f32;
                        let wyn = nwy;
                        let wzn = nz as f32;
                        let wxw_n = fractal3(&n_warp, wxn, wyn, wzn, 3, 0.6, 2.0, 220.0);
                        let wyw_n = fractal3(
                            &n_warp,
                            wxn + 133.7,
                            wyn + 71.3,
                            wzn - 19.1,
                            3,
                            0.6,
                            2.0,
                            220.0,
                        );
                        let wzw_n = fractal3(
                            &n_warp,
                            wxn - 54.2,
                            wyn + 29.7,
                            wzn + 88.8,
                            3,
                            0.6,
                            2.0,
                            220.0,
                        );
                        let nxp = wxn + wxw_n * WARP_XY;
                        let nyp = wyn + wyw_n * WARP_Y;
                        let nzp = wzn + wzw_n * WARP_XY;
                        let tn_n = fractal3(&n_tun, nxp, nyp * Y_SCALE, nzp, 4, 0.55, 2.0, 140.0);
                        let eps_n = EPS_BASE + EPS_ADD * n_depth;
                        let carve_tn = tn_n.abs() < eps_n;
                        let wn_n = worley3_f1_norm(
                            nxp,
                            nyp,
                            nzp,
                            ROOM_CELL,
                            ((self.seed as u32) ^ 2100u32).wrapping_add(1337),
                        );
                        let room_thr_n = ROOM_THR_BASE + ROOM_THR_ADD * n_depth;
                        let carve_rm = wn_n < room_thr_n;
                        let neighbor_carved_air =
                            (nsoil > SOIL_MIN && nwy > MIN_Y) && (carve_tn || carve_rm);
                        if !neighbor_carved_air {
                            near_solid = true;
                            break;
                        }
                    }
                    // Random selection per voxel, seeded deterministically
                    let h3 = hash3(x, y, z, (self.seed as u32) ^ 0xC0FF_EE15);
                    let r = (h3 & 0x00FF_FFFF) as f32 / 16_777_216.0;
                    if near_solid && y < height - 2 && r < GLOW_PROB {
                        base_block = Block::Glowstone;
                    } else {
                        base_block = Block::Air;
                    }
                }
            }
        }

        // Glowstone is now handled during carving above.

        // Deterministic per-column tree spawn (adapted from old code)
        // Multiple species via temperature/moisture fields
        // Only spawn on grass surfaces
        const TREE_PROB: f32 = 0.02; // ~2% chance per column
        const TRUNK_MIN: i32 = 4;
        const TRUNK_MAX: i32 = 6;
        const LEAF_R: i32 = 2;

        // Simple 2D hash -> [0,1)
        let hash2 = |ix: i32, iz: i32, seed: u32| -> u32 {
            let mut h = (ix as u32).wrapping_mul(0x85eb_ca6b)
                ^ (iz as u32).wrapping_mul(0xc2b2_ae35)
                ^ seed.wrapping_mul(0x27d4_eb2d);
            h ^= h >> 16;
            h = h.wrapping_mul(0x7feb_352d);
            h ^= h >> 15;
            h = h.wrapping_mul(0x846c_a68b);
            h ^= h >> 16;
            h
        };
        let rand01 = |ix: i32, iz: i32, salt: u32| -> f32 {
            let h = hash2(
                ix,
                iz,
                ((self.seed as u32) ^ salt).wrapping_add(0x9E37_79B9),
            );
            ((h & 0x00FF_FFFF) as f32) / 16_777_216.0
        };

        // Species picker based on temp/moist noise like old code
        let mut temp = FastNoiseLite::with_seed(self.seed ^ 100_001);
        temp.set_noise_type(Some(NoiseType::OpenSimplex2));
        temp.set_frequency(Some(0.007));
        let mut mois = FastNoiseLite::with_seed(self.seed ^ 100_007);
        mois.set_noise_type(Some(NoiseType::OpenSimplex2));
        mois.set_frequency(Some(0.005));

        let pick_species = |tx: i32, tz: i32| -> TreeSpecies {
            let mut t = temp.get_noise_2d(tx as f32, tz as f32) * 0.5 + 0.5;
            let mut m = mois.get_noise_2d(tx as f32, tz as f32) * 0.5 + 0.5;
            let h = hash2(tx, tz, (self.seed as u32) ^ 0x5F37_59u32);
            let j = (h & 0xffff) as f32 / 65535.0 * 0.08 - 0.04;
            t = (t + j).clamp(0.0, 1.0);
            m = (m - j).clamp(0.0, 1.0);
            if t < 0.35 {
                return TreeSpecies::Spruce;
            }
            if t > 0.7 && m > 0.6 {
                return TreeSpecies::Jungle;
            }
            if t > 0.6 && m < 0.45 {
                return TreeSpecies::Acacia;
            }
            if m > 0.65 {
                return TreeSpecies::DarkOak;
            }
            // tie-breaker between birch and oak
            if ((h >> 20) & 1) == 1 {
                TreeSpecies::Birch
            } else {
                TreeSpecies::Oak
            }
        };

        // Helper: does a trunk start at (tx,tz)? If yes, return (surf_y, trunk_h, species)
        let trunk_at = |tx: i32, tz: i32| -> Option<(i32, i32, TreeSpecies)> {
            // surf = topmost solid Y index (height_for returns one above surface)
            let surf = height_for(tx, tz) - 1;
            // approximate surface block classification
            let surf_block = if surf as f32 >= self.chunk_size_y as f32 * 0.62 {
                Block::Snow
            } else if surf as f32 <= self.chunk_size_y as f32 * 0.2 {
                Block::Sand
            } else {
                Block::Grass
            };
            if !matches!(surf_block, Block::Grass) {
                return None;
            }
            let r = rand01(tx, tz, 0xA53F9);
            if r >= TREE_PROB {
                return None;
            }
            let span = (TRUNK_MAX - TRUNK_MIN).max(0) as u32;
            let hsel = hash2(tx, tz, 0x51F0_A7) % (span + 1);
            let th = TRUNK_MIN + hsel as i32;
            if surf <= 2 || surf >= (self.chunk_size_y as i32 - 6) {
                return None;
            }
            let sp = pick_species(tx, tz);
            Some((surf, th, sp))
        };

        // Trunk at current column?
        if let Some((surf, th, sp)) = trunk_at(x, z) {
            if y >= surf + 1 && y <= surf + th {
                return Block::Wood(sp);
            }
        }

        // Canopy: check nearby trunks and stamp leaves only where base is air
        if matches!(base_block, Block::Air) {
            for tx in (x - LEAF_R)..=(x + LEAF_R) {
                for tz in (z - LEAF_R)..=(z + LEAF_R) {
                    if let Some((surf, th, sp)) = trunk_at(tx, tz) {
                        let top_y = surf + th;
                        // y layer relative to top
                        let dy = y - top_y;
                        if dy < -2 || dy > 2 {
                            continue;
                        }
                        let rad = if dy <= -2 || dy >= 2 {
                            LEAF_R - 1
                        } else {
                            LEAF_R
                        };
                        let dx = x - tx;
                        let dz = z - tz;
                        if dx == 0 && dz == 0 && dy >= 0 {
                            continue;
                        }
                        let man = dx.abs() + dz.abs();
                        let extra = if dy >= 1 { 0 } else { 1 };
                        if man <= rad + extra {
                            return Block::Leaves(sp);
                        }
                    }
                }
            }
        }

        base_block
    }
}

impl World {
    #[inline]
    pub fn is_flat(&self) -> bool {
        matches!(self.mode, WorldGenMode::Flat { .. })
    }
}

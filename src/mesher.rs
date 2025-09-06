use crate::chunkbuf::ChunkBuf;
use crate::lighting::{LightBorders, LightGrid, LightingStore};
use crate::voxel::{Axis, Block, Dir4, MaterialKey, SlabHalf, TerracottaColor, TreeSpecies, World};
use raylib::core::math::BoundingBox;
use raylib::prelude::*;
use std::collections::HashMap as StdHashMap;
use std::collections::HashMap;

// Visual-only lighting floor to avoid pitch-black faces in darkness.
// Does not affect logical light propagation.
const VISUAL_LIGHT_MIN: u8 = 18; // ~7% brightness floor

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FaceMaterial {
    GrassTop,
    GrassSide, // needs V-flip
    Dirt,
    Stone,
    Sand,
    Snow,
    Bookshelf,
    CoarseDirt,
    PodzolTop,
    PodzolSide,
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
    SandstoneTop,
    SandstoneBottom,
    SandstoneSide,
    RedSandstoneTop,
    RedSandstoneBottom,
    RedSandstoneSide,
    QuartzBlockTop,
    QuartzBlockSide,
    LapisBlock,
    CoalBlock,
    PrismarineBricks,
    NetherBricks,
    EndStone,
    EndStoneBricks,
    Planks(TreeSpecies),
    WoodTop(TreeSpecies),
    WoodSide(TreeSpecies),
    Leaves(TreeSpecies),
    TerracottaPlain,
    Terracotta(TerracottaColor),
    QuartzPillarTop,
    QuartzPillarSide,
    Glowstone,
    Beacon,
}

impl FaceMaterial {
    pub fn texture_candidates(&self) -> Vec<&'static str> {
        match self {
            FaceMaterial::GrassTop => vec!["assets/blocks/grass_top.png"],
            FaceMaterial::GrassSide => vec!["assets/blocks/grass_side.png"],
            FaceMaterial::Dirt => vec!["assets/blocks/dirt.png"],
            FaceMaterial::Stone => vec!["assets/blocks/stone.png"],
            FaceMaterial::Sand => vec!["assets/blocks/sand.png"],
            FaceMaterial::Snow => vec!["assets/blocks/snow.png"],
            FaceMaterial::Bookshelf => vec!["assets/blocks/bookshelf.png"],
            FaceMaterial::CoarseDirt => vec!["assets/blocks/coarse_dirt.png"],
            FaceMaterial::PodzolTop => vec!["assets/blocks/dirt_podzol_top.png"],
            FaceMaterial::PodzolSide => vec!["assets/blocks/dirt_podzol_side.png"],
            FaceMaterial::Cobblestone => vec!["assets/blocks/cobblestone.png"],
            FaceMaterial::MossyCobblestone => vec!["assets/blocks/cobblestone_mossy.png"],
            FaceMaterial::StoneBricks => vec!["assets/blocks/stonebrick.png"],
            FaceMaterial::MossyStoneBricks => vec!["assets/blocks/stonebrick_mossy.png"],
            FaceMaterial::Brick => vec!["assets/blocks/brick.png"],
            FaceMaterial::Granite => vec!["assets/blocks/stone_granite.png"],
            FaceMaterial::Diorite => vec!["assets/blocks/stone_diorite.png"],
            FaceMaterial::Andesite => vec!["assets/blocks/stone_andesite.png"],
            FaceMaterial::PolishedGranite => vec!["assets/blocks/stone_granite_smooth.png"],
            FaceMaterial::PolishedDiorite => vec!["assets/blocks/stone_diorite_smooth.png"],
            FaceMaterial::PolishedAndesite => vec!["assets/blocks/stone_andesite_smooth.png"],
            FaceMaterial::Gravel => vec!["assets/blocks/gravel.png"],
            FaceMaterial::SmoothStone => vec![
                "assets/blocks/stone_slab_top.png",
                "assets/blocks/stone.png",
            ],
            FaceMaterial::SandstoneTop => vec!["assets/blocks/sandstone_top.png"],
            FaceMaterial::SandstoneBottom => vec!["assets/blocks/sandstone_bottom.png"],
            FaceMaterial::SandstoneSide => vec!["assets/blocks/sandstone_normal.png"],
            FaceMaterial::RedSandstoneTop => vec!["assets/blocks/red_sandstone_top.png"],
            FaceMaterial::RedSandstoneBottom => vec!["assets/blocks/red_sandstone_bottom.png"],
            FaceMaterial::RedSandstoneSide => vec!["assets/blocks/red_sandstone_normal.png"],
            FaceMaterial::QuartzBlockTop => vec!["assets/blocks/quartz_block_top.png"],
            FaceMaterial::QuartzBlockSide => vec!["assets/blocks/quartz_block_side.png"],
            FaceMaterial::LapisBlock => vec!["assets/blocks/lapis_block.png"],
            FaceMaterial::CoalBlock => vec!["assets/blocks/coal_block.png"],
            FaceMaterial::PrismarineBricks => vec!["assets/blocks/prismarine_bricks.png"],
            FaceMaterial::NetherBricks => vec!["assets/blocks/nether_brick.png"],
            FaceMaterial::EndStone => vec!["assets/blocks/end_stone.png"],
            // Temporary: use end_stone texture for bricks too until a brick texture is provided
            FaceMaterial::EndStoneBricks => vec!["assets/blocks/end_stone.png"],
            FaceMaterial::QuartzPillarTop => vec!["assets/blocks/quartz_block_lines_top.png"],
            FaceMaterial::QuartzPillarSide => vec!["assets/blocks/quartz_block_lines.png"],
            FaceMaterial::Planks(sp) => match sp {
                TreeSpecies::Oak => vec!["assets/blocks/planks_oak.png"],
                TreeSpecies::Birch => vec!["assets/blocks/planks_birch.png"],
                TreeSpecies::Spruce => vec!["assets/blocks/planks_spruce.png"],
                TreeSpecies::Jungle => vec!["assets/blocks/planks_jungle.png"],
                TreeSpecies::Acacia => vec!["assets/blocks/planks_acacia.png"],
                TreeSpecies::DarkOak => vec!["assets/blocks/planks_big_oak.png"],
            },
            FaceMaterial::TerracottaPlain => vec!["assets/blocks/hardened_clay.png"],
            FaceMaterial::Terracotta(color) => match color {
                TerracottaColor::White => vec!["assets/blocks/hardened_clay_stained_white.png"],
                TerracottaColor::Orange => vec!["assets/blocks/hardened_clay_stained_orange.png"],
                TerracottaColor::Magenta => vec!["assets/blocks/hardened_clay_stained_magenta.png"],
                TerracottaColor::LightBlue => vec!["assets/blocks/hardened_clay_stained_light_blue.png"],
                TerracottaColor::Yellow => vec!["assets/blocks/hardened_clay_stained_yellow.png"],
                TerracottaColor::Lime => vec!["assets/blocks/hardened_clay_stained_lime.png"],
                TerracottaColor::Pink => vec!["assets/blocks/hardened_clay_stained_pink.png"],
                TerracottaColor::Gray => vec!["assets/blocks/hardened_clay_stained_gray.png"],
                TerracottaColor::LightGray => vec!["assets/blocks/hardened_clay_stained_silver.png"],
                TerracottaColor::Cyan => vec!["assets/blocks/hardened_clay_stained_cyan.png"],
                TerracottaColor::Purple => vec!["assets/blocks/hardened_clay_stained_purple.png"],
                TerracottaColor::Blue => vec!["assets/blocks/hardened_clay_stained_blue.png"],
                TerracottaColor::Brown => vec!["assets/blocks/hardened_clay_stained_brown.png"],
                TerracottaColor::Green => vec!["assets/blocks/hardened_clay_stained_green.png"],
                TerracottaColor::Red => vec!["assets/blocks/hardened_clay_stained_red.png"],
                TerracottaColor::Black => vec!["assets/blocks/hardened_clay_stained_black.png"],
            },
            FaceMaterial::Glowstone => vec![
                "assets/blocks/glowstone.png",
                "assets/blocks/sea_lantern.png",
            ],
            FaceMaterial::Beacon => vec![
                "assets/blocks/beacon.png",
                "assets/blocks/sea_lantern.png",
                "assets/blocks/glowstone.png",
            ],
            FaceMaterial::WoodTop(sp) => match sp {
                TreeSpecies::Oak => vec![
                    "assets/blocks/log_oak_top.png",
                    "assets/blocks/log_big_oak_top.png",
                ],
                TreeSpecies::DarkOak => vec![
                    "assets/blocks/log_big_oak_top.png",
                    "assets/blocks/log_oak_top.png",
                ],
                TreeSpecies::Birch => vec!["assets/blocks/log_birch_top.png"],
                TreeSpecies::Spruce => vec!["assets/blocks/log_spruce_top.png"],
                TreeSpecies::Jungle => vec!["assets/blocks/log_jungle_top.png"],
                TreeSpecies::Acacia => vec!["assets/blocks/log_acacia_top.png"],
            },
            FaceMaterial::WoodSide(sp) => match sp {
                TreeSpecies::Oak => {
                    vec!["assets/blocks/log_oak.png", "assets/blocks/log_big_oak.png"]
                }
                TreeSpecies::DarkOak => {
                    vec!["assets/blocks/log_big_oak.png", "assets/blocks/log_oak.png"]
                }
                TreeSpecies::Birch => vec!["assets/blocks/log_birch.png"],
                TreeSpecies::Spruce => vec!["assets/blocks/log_spruce.png"],
                TreeSpecies::Jungle => vec!["assets/blocks/log_jungle.png"],
                TreeSpecies::Acacia => vec!["assets/blocks/log_acacia.png"],
            },
            FaceMaterial::Leaves(sp) => match sp {
                // Prefer opaque variants first to avoid alpha
                TreeSpecies::Oak => vec![
                    "assets/blocks/leaves_oak_opaque.png",
                    "assets/blocks/leaves_oak.png",
                ],
                TreeSpecies::DarkOak => vec![
                    "assets/blocks/leaves_big_oak_opaque.png",
                    "assets/blocks/leaves_big_oak.png",
                ],
                TreeSpecies::Birch => vec![
                    "assets/blocks/leaves_birch_opaque.png",
                    "assets/blocks/leaves_birch.png",
                ],
                TreeSpecies::Spruce => vec![
                    "assets/blocks/leaves_spruce_opaque.png",
                    "assets/blocks/leaves_spruce.png",
                ],
                TreeSpecies::Jungle => vec![
                    "assets/blocks/leaves_jungle_opaque.png",
                    "assets/blocks/leaves_jungle.png",
                ],
                TreeSpecies::Acacia => vec![
                    "assets/blocks/leaves_acacia_opaque.png",
                    "assets/blocks/leaves_acacia.png",
                ],
            },
        }
        .to_vec()
    }
}

#[derive(Default, Clone)]
pub struct MeshBuild {
    pos: Vec<f32>,
    norm: Vec<f32>,
    uv: Vec<f32>,
    idx: Vec<u16>,
    col: Vec<u8>,
}

impl MeshBuild {
    pub(crate) fn add_quad(
        &mut self,
        a: Vector3,
        b: Vector3,
        c: Vector3,
        d: Vector3,
        n: Vector3,
        u1: f32,
        v1: f32,
        flip_v: bool,
        rgba: [u8; 4],
    ) {
        let base = self.pos.len() as u32 / 3;
        // Start with the same order the old code used (a,d,c,b)
        let mut vs = [a, d, c, b];
        // UVs: (0,0) (0,v1) (u1,v1) (u1,0)
        let mut uvs = [(0.0, 0.0), (0.0, v1), (u1, v1), (u1, 0.0)];

        // Ensure winding faces outward: ((vs1-vs0) x (vs2-vs0)) · n should be > 0 for CCW
        let e1 = vs[1] - vs[0];
        let e2 = vs[2] - vs[0];
        let cross = e1.cross(e2);
        if cross.dot(n) < 0.0 {
            // Swap 1 <-> 3 to flip winding while keeping rectangle
            vs.swap(1, 3);
            uvs.swap(1, 3);
        }

        if flip_v {
            for uv in &mut uvs {
                uv.1 = v1 - uv.1;
            }
        }

        for i in 0..4 {
            self.pos.extend_from_slice(&[vs[i].x, vs[i].y, vs[i].z]);
            self.norm.extend_from_slice(&[n.x, n.y, n.z]);
            self.uv.extend_from_slice(&[uvs[i].0, uvs[i].1]);
            self.col
                .extend_from_slice(&[rgba[0], rgba[1], rgba[2], rgba[3]]);
        }
        // Two triangles: (0,1,2) and (0,2,3)
        self.idx.extend_from_slice(&[
            (base + 0) as u16,
            (base + 1) as u16,
            (base + 2) as u16,
            (base + 0) as u16,
            (base + 2) as u16,
            (base + 3) as u16,
        ]);
    }
}

// (greedy_rects moved to meshing_core)

fn face_material_for(block: Block, face: usize) -> Option<FaceMaterial> {
    // face: 0=+Y(top), 1=-Y(bottom), 2=+X, 3=-X, 4=+Z, 5=-Z
    match block {
        Block::Air => None,
        Block::Grass => match face {
            0 => Some(FaceMaterial::GrassTop),
            1 => Some(FaceMaterial::Dirt), // bottom is dirt
            2 | 3 | 4 | 5 => Some(FaceMaterial::GrassSide),
            _ => None,
        },
        Block::Dirt => Some(FaceMaterial::Dirt),
        Block::Stone => Some(FaceMaterial::Stone),
        Block::Sand => Some(FaceMaterial::Sand),
        Block::Snow => Some(FaceMaterial::Snow),
        Block::Bookshelf => Some(FaceMaterial::Bookshelf),
        Block::CoarseDirt => Some(FaceMaterial::CoarseDirt),
        Block::Podzol => match face {
            0 => Some(FaceMaterial::PodzolTop),
            1 => Some(FaceMaterial::Dirt),
            _ => Some(FaceMaterial::PodzolSide),
        },
        Block::Cobblestone => Some(FaceMaterial::Cobblestone),
        Block::MossyCobblestone => Some(FaceMaterial::MossyCobblestone),
        Block::StoneBricks => Some(FaceMaterial::StoneBricks),
        Block::MossyStoneBricks => Some(FaceMaterial::MossyStoneBricks),
        Block::Brick => Some(FaceMaterial::Brick),
        Block::Granite => Some(FaceMaterial::Granite),
        Block::Diorite => Some(FaceMaterial::Diorite),
        Block::Andesite => Some(FaceMaterial::Andesite),
        Block::PolishedGranite => Some(FaceMaterial::PolishedGranite),
        Block::PolishedDiorite => Some(FaceMaterial::PolishedDiorite),
        Block::PolishedAndesite => Some(FaceMaterial::PolishedAndesite),
        Block::Gravel => Some(FaceMaterial::Gravel),
        Block::SmoothStone => Some(FaceMaterial::SmoothStone),
        Block::Sandstone => match face {
            0 => Some(FaceMaterial::SandstoneTop),
            1 => Some(FaceMaterial::SandstoneBottom),
            _ => Some(FaceMaterial::SandstoneSide),
        },
        Block::SmoothSandstone => match face {
            0 => Some(FaceMaterial::SandstoneTop),
            1 => Some(FaceMaterial::SandstoneBottom),
            _ => Some(FaceMaterial::SandstoneSide),
        },
        Block::RedSandstone => match face {
            0 => Some(FaceMaterial::RedSandstoneTop),
            1 => Some(FaceMaterial::RedSandstoneBottom),
            _ => Some(FaceMaterial::RedSandstoneSide),
        },
        Block::SmoothRedSandstone => match face {
            0 => Some(FaceMaterial::RedSandstoneTop),
            1 => Some(FaceMaterial::RedSandstoneBottom),
            _ => Some(FaceMaterial::RedSandstoneSide),
        },
        Block::QuartzBlock => match face {
            0 | 1 => Some(FaceMaterial::QuartzBlockTop),
            _ => Some(FaceMaterial::QuartzBlockSide),
        },
        Block::LapisBlock => Some(FaceMaterial::LapisBlock),
        Block::CoalBlock => Some(FaceMaterial::CoalBlock),
        Block::PrismarineBricks => Some(FaceMaterial::PrismarineBricks),
        Block::NetherBricks => Some(FaceMaterial::NetherBricks),
        Block::EndStone => Some(FaceMaterial::EndStone),
        Block::EndStoneBricks => Some(FaceMaterial::EndStoneBricks),
        Block::Planks(sp) => Some(FaceMaterial::Planks(sp)),
        Block::Wood(sp) => match face {
            0 | 1 => Some(FaceMaterial::WoodTop(sp)),
            2 | 3 | 4 | 5 => Some(FaceMaterial::WoodSide(sp)),
            _ => None,
        },
        Block::LogAxis(sp, axis) => {
            // face: 0=+Y,1=-Y,2=+X,3=-X,4=+Z,5=-Z
            let face_axis = match face { 0 | 1 => Axis::Y, 2 | 3 => Axis::X, 4 | 5 => Axis::Z, _ => Axis::Y };
            if face_axis == axis { Some(FaceMaterial::WoodTop(sp)) } else { Some(FaceMaterial::WoodSide(sp)) }
        }
        Block::QuartzPillar(axis) => {
            let face_axis = match face { 0 | 1 => Axis::Y, 2 | 3 => Axis::X, 4 | 5 => Axis::Z, _ => Axis::Y };
            if face_axis == axis { Some(FaceMaterial::QuartzPillarTop) } else { Some(FaceMaterial::QuartzPillarSide) }
        }
        Block::Leaves(sp) => Some(FaceMaterial::Leaves(sp)),
        // Special shapes handled in the special mesher pass
        Block::Slab { .. } => None,
        Block::Stairs { .. } => None,
        Block::TerracottaPlain => Some(FaceMaterial::TerracottaPlain),
        Block::Terracotta(c) => Some(FaceMaterial::Terracotta(c)),
        Block::Glowstone => Some(FaceMaterial::Glowstone),
        Block::Beacon => Some(FaceMaterial::Beacon),
    }
}

#[inline]
fn face_material_for_key(key: MaterialKey, face: usize) -> FaceMaterial {
    match key {
        MaterialKey::SmoothStone => FaceMaterial::SmoothStone,
        MaterialKey::Sandstone => match face { 0 => FaceMaterial::SandstoneTop, 1 => FaceMaterial::SandstoneBottom, _ => FaceMaterial::SandstoneSide },
        MaterialKey::RedSandstone => match face { 0 => FaceMaterial::RedSandstoneTop, 1 => FaceMaterial::RedSandstoneBottom, _ => FaceMaterial::RedSandstoneSide },
        MaterialKey::Cobblestone => FaceMaterial::Cobblestone,
        MaterialKey::MossyCobblestone => FaceMaterial::MossyCobblestone,
        MaterialKey::StoneBricks => FaceMaterial::StoneBricks,
        MaterialKey::MossyStoneBricks => FaceMaterial::MossyStoneBricks,
        MaterialKey::QuartzBlock => match face { 0 | 1 => FaceMaterial::QuartzBlockTop, _ => FaceMaterial::QuartzBlockSide },
        MaterialKey::Planks(sp) => FaceMaterial::Planks(sp),
        MaterialKey::PrismarineBricks => FaceMaterial::PrismarineBricks,
        MaterialKey::EndStone => FaceMaterial::EndStone,
        MaterialKey::EndStoneBricks => FaceMaterial::EndStoneBricks,
        MaterialKey::Granite => FaceMaterial::Granite,
        MaterialKey::Diorite => FaceMaterial::Diorite,
        MaterialKey::Andesite => FaceMaterial::Andesite,
        MaterialKey::PolishedGranite => FaceMaterial::PolishedGranite,
        MaterialKey::PolishedDiorite => FaceMaterial::PolishedDiorite,
        MaterialKey::PolishedAndesite => FaceMaterial::PolishedAndesite,
    }
}

#[inline]
fn emit_box(
    builds: &mut std::collections::HashMap<FaceMaterial, MeshBuild>,
    buf: &ChunkBuf,
    world: &World,
    edits: Option<&StdHashMap<(i32, i32, i32), Block>>,
    neighbors: NeighborsLoaded,
    light: &LightGrid,
    x: usize,
    y: usize,
    z: usize,
    base_x: i32,
    base_z: i32,
    fm_for_face: &dyn Fn(usize) -> FaceMaterial,
    min: Vector3,
    max: Vector3,
) {
    // Faces: 0=+Y,1=-Y,2=+X,3=-X,4=+Z,5=-Z
    let gx = base_x + x as i32;
    let gy = y as i32;
    let gz = base_z + z as i32;
    let here = buf.get_local(x, y, z);
    // +Y top
    {
        let nx = gx;
        let ny = gy + 1;
        let nz = gz;
        if !is_occluder(buf, world, edits, neighbors, here, 0, nx, ny, nz) {
            let l = light.sample_face_local(x, y, z, 0);
            let lv = l.max(VISUAL_LIGHT_MIN);
            let rgba = [lv, lv, lv, 255];
            let fm = fm_for_face(0);
            let mb = builds.entry(fm).or_default();
            mb.add_quad(
                Vector3::new(min.x, max.y, min.z),
                Vector3::new(max.x, max.y, min.z),
                Vector3::new(max.x, max.y, max.z),
                Vector3::new(min.x, max.y, max.z),
                Vector3::new(0.0, 1.0, 0.0),
                (max.x - min.x),
                (max.z - min.z),
                false,
                rgba,
            );
        }
    }
    // -Y bottom
    {
        let nx = gx;
        let ny = gy - 1;
        let nz = gz;
        if !is_occluder(buf, world, edits, neighbors, here, 1, nx, ny, nz) {
            let l = light.sample_face_local(x, y, z, 1);
            let lv = l.max(VISUAL_LIGHT_MIN);
            let rgba = [lv, lv, lv, 255];
            let fm = fm_for_face(1);
            let mb = builds.entry(fm).or_default();
            mb.add_quad(
                Vector3::new(min.x, min.y, max.z),
                Vector3::new(max.x, min.y, max.z),
                Vector3::new(max.x, min.y, min.z),
                Vector3::new(min.x, min.y, min.z),
                Vector3::new(0.0, -1.0, 0.0),
                (max.x - min.x),
                (max.z - min.z),
                false,
                rgba,
            );
        }
    }
    // +X face
    {
        let nx = gx + 1;
        let ny = gy;
        let nz = gz;
        if !is_occluder(buf, world, edits, neighbors, here, 2, nx, ny, nz) {
            let l = light.sample_face_local(x, y, z, 2);
            let lv = l.max(VISUAL_LIGHT_MIN);
            let rgba = [lv, lv, lv, 255];
            let fm = fm_for_face(2);
            let mb = builds.entry(fm).or_default();
            mb.add_quad(
                Vector3::new(max.x, max.y, max.z),
                Vector3::new(max.x, max.y, min.z),
                Vector3::new(max.x, min.y, min.z),
                Vector3::new(max.x, min.y, max.z),
                Vector3::new(1.0, 0.0, 0.0),
                (max.z - min.z),
                (max.y - min.y),
                false,
                rgba,
            );
        }
    }
    // -X face
    {
        let nx = gx - 1;
        let ny = gy;
        let nz = gz;
        if !is_occluder(buf, world, edits, neighbors, here, 3, nx, ny, nz) {
            let l = light.sample_face_local(x, y, z, 3);
            let lv = l.max(VISUAL_LIGHT_MIN);
            let rgba = [lv, lv, lv, 255];
            let fm = fm_for_face(3);
            let mb = builds.entry(fm).or_default();
            mb.add_quad(
                Vector3::new(min.x, max.y, min.z),
                Vector3::new(min.x, max.y, max.z),
                Vector3::new(min.x, min.y, max.z),
                Vector3::new(min.x, min.y, min.z),
                Vector3::new(-1.0, 0.0, 0.0),
                (max.z - min.z),
                (max.y - min.y),
                false,
                rgba,
            );
        }
    }
    // +Z face
    {
        let nx = gx;
        let ny = gy;
        let nz = gz + 1;
        if !is_occluder(buf, world, edits, neighbors, here, 4, nx, ny, nz) {
            let l = light.sample_face_local(x, y, z, 4);
            let lv = l.max(VISUAL_LIGHT_MIN);
            let rgba = [lv, lv, lv, 255];
            let fm = fm_for_face(4);
            let mb = builds.entry(fm).or_default();
            mb.add_quad(
                Vector3::new(min.x, max.y, max.z),
                Vector3::new(max.x, max.y, max.z),
                Vector3::new(max.x, min.y, max.z),
                Vector3::new(min.x, min.y, max.z),
                Vector3::new(0.0, 0.0, 1.0),
                (max.x - min.x),
                (max.y - min.y),
                false,
                rgba,
            );
        }
    }
    // -Z face
    {
        let nx = gx;
        let ny = gy;
        let nz = gz - 1;
        if !is_occluder(buf, world, edits, neighbors, here, 5, nx, ny, nz) {
            let l = light.sample_face_local(x, y, z, 5);
            let lv = l.max(VISUAL_LIGHT_MIN);
            let rgba = [lv, lv, lv, 255];
            let fm = fm_for_face(5);
            let mb = builds.entry(fm).or_default();
            mb.add_quad(
                Vector3::new(max.x, max.y, min.z),
                Vector3::new(min.x, max.y, min.z),
                Vector3::new(min.x, min.y, min.z),
                Vector3::new(max.x, min.y, min.z),
                Vector3::new(0.0, 0.0, -1.0),
                (max.x - min.x),
                (max.y - min.y),
                false,
                rgba,
            );
        }
    }
}

// world-based occluder test removed; occlusion uses only local chunk buffers.

#[derive(Clone, Copy, Debug, Default)]
pub struct NeighborsLoaded {
    pub neg_x: bool, // west  (cx-1, cz)
    pub pos_x: bool, // east  (cx+1, cz)
    pub neg_z: bool, // north (cx, cz-1)
    pub pos_z: bool, // south (cx, cz+1)
}

#[inline]
fn is_occluder(
    buf: &ChunkBuf,
    world: &World,
    edits: Option<&StdHashMap<(i32, i32, i32), Block>>,
    nmask: NeighborsLoaded,
    here: Block,
    face: usize,
    nx: i32,
    ny: i32,
    nz: i32,
) -> bool {
    if !here.is_solid() {
        return false;
    }
    // Check inside this chunk first
    if buf.contains_world(nx, ny, nz) {
        let x0 = buf.cx * buf.sx as i32;
        let z0 = buf.cz * buf.sz as i32;
        if ny < 0 || ny >= buf.sy as i32 {
            return false;
        }
        let lx = (nx - x0) as usize;
        let ly = ny as usize;
        let lz = (nz - z0) as usize;
        let nb = buf.get_local(lx, ly, lz);
        return occludes_face(nb, face);
    }
    // Outside current chunk: only occlude if the corresponding neighbor chunk is loaded; otherwise treat as air
    let x0 = buf.cx * buf.sx as i32;
    let z0 = buf.cz * buf.sz as i32;
    let x1 = x0 + buf.sx as i32;
    let z1 = z0 + buf.sz as i32;
    let mut neighbor_loaded = false;
    if nx < x0 {
        neighbor_loaded = nmask.neg_x;
    } else if nx >= x1 {
        neighbor_loaded = nmask.pos_x;
    } else if nz < z0 {
        neighbor_loaded = nmask.neg_z;
    } else if nz >= z1 {
        neighbor_loaded = nmask.pos_z;
    }
    // Y outside world or not strictly an adjacent chunk border: treat as air
    if !neighbor_loaded {
        return false;
    }
    // Query edits overlay first, falling back to world generation
    let nb = if let Some(es) = edits {
        es.get(&(nx, ny, nz))
            .copied()
            .unwrap_or_else(|| world.block_at(nx, ny, nz))
    } else {
        world.block_at(nx, ny, nz)
    };
    occludes_face(nb, face)
}

#[inline]
fn occludes_face(nb: Block, face: usize) -> bool {
    match nb {
        Block::Slab { half, .. } => match face {
            0 => matches!(half, SlabHalf::Bottom), // neighbor above occludes only if bottom slab
            1 => matches!(half, SlabHalf::Top),    // neighbor below occludes only if top slab
            _ => nb.is_solid(), // sides treated as full for greedy; partial handled separately
        },
        Block::Stairs { half, .. } => match face {
            0 => matches!(half, SlabHalf::Bottom),
            1 => matches!(half, SlabHalf::Top),
            _ => nb.is_solid(),
        },
        _ => nb.is_solid(),
    }
}

pub struct ChunkRender {
    pub cx: i32,
    pub cz: i32,
    pub bbox: BoundingBox,
    pub parts: Vec<(FaceMaterial, raylib::core::models::Model)>,
}

pub struct ChunkMeshCPU {
    pub cx: i32,
    pub cz: i32,
    pub bbox: BoundingBox,
    pub parts: std::collections::HashMap<FaceMaterial, MeshBuild>,
}

pub fn build_chunk_greedy_cpu_buf(
    buf: &ChunkBuf,
    lighting: Option<&LightingStore>,
    world: &World,
    edits: Option<&StdHashMap<(i32, i32, i32), Block>>,
    neighbors: NeighborsLoaded,
    cx: i32,
    cz: i32,
) -> Option<(ChunkMeshCPU, Option<LightBorders>)> {
    let sx = buf.sx;
    let sy = buf.sy;
    let sz = buf.sz;
    let base_x = buf.cx * sx as i32;
    let base_z = buf.cz * sz as i32;

    // Unified path via meshing_core
    let light = match lighting {
        Some(store) => LightGrid::compute_with_borders_buf(buf, store),
        None => return None,
    };
        let flip_v = [false, false, false, false, false, false];
        let mut builds = crate::meshing_core::build_mesh_core(
            buf,
            base_x,
            base_z,
            flip_v,
            Some(VISUAL_LIGHT_MIN),
            |x, y, z, face, here| {
                if !here.is_solid() {
                    return None;
                }
                let gx = base_x + x as i32;
                let gy = y as i32;
                let gz = base_z + z as i32;
                let (nx, ny, nz) = match face {
                    0 => (gx, gy + 1, gz),
                    1 => (gx, gy - 1, gz),
                    2 => (gx + 1, gy, gz),
                    3 => (gx - 1, gy, gz),
                    4 => (gx, gy, gz + 1),
                    5 => (gx, gy, gz - 1),
                    _ => unreachable!(),
                };
                if is_occluder(buf, world, edits, neighbors, here, face, nx, ny, nz) {
                    return None;
                }
                if let Some(fm) = face_material_for(here, face) {
                    let mut l = light.sample_face_local(x, y, z, face);
                    // If top face and neighbor above is a top-half shape, sample one more cell above for light
                    if face == 0 {
                        // Convert world neighbor coords back to local if within this chunk
                        if buf.contains_world(nx, ny, nz) && ny >= 0 && (ny as usize) < sy {
                            let lx = (nx - base_x) as usize;
                            let ly = ny as usize;
                            let lz = (nz - base_z) as usize;
                            match buf.get_local(lx, ly, lz) {
                                Block::Slab { half: SlabHalf::Top, .. } | Block::Stairs { half: SlabHalf::Top, .. } => {
                                    if y + 1 < sy {
                                        let l2 = light.sample_face_local(x, y + 1, z, 0);
                                        l = l.max(l2);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Some((fm, l))
                } else {
                    None
                }
            },
        );
        // Special-shapes pass: mesh slabs and stairs
        for z in 0..sz {
            for y in 0..sy {
                for x in 0..sx {
                    match buf.get_local(x, y, z) {
                        Block::Slab { half, key } => {
                            let fx = base_x as f32 + x as f32;
                            let fy = y as f32;
                            let fz = base_z as f32 + z as f32;
                            let (y0, y1) = match half { SlabHalf::Bottom => (fy, fy + 0.5), SlabHalf::Top => (fy + 0.5, fy + 1.0) };
                            let min = Vector3::new(fx, y0, fz);
                            let max = Vector3::new(fx + 1.0, y1, fz + 1.0);
                            let keyc = key; // copy
                            emit_box(
                                &mut builds,
                                buf,
                                world,
                                edits,
                                neighbors,
                                &light,
                                x,
                                y,
                                z,
                                base_x,
                                base_z,
                                &|face| face_material_for_key(keyc, face),
                                min,
                                max,
                            );

                            // Restore partial neighbor faces that greedy culled fully
                            // Visible portion is opposite half along Y
                            let (vis_y0, vis_y1) = match half { SlabHalf::Bottom => (fy + 0.5, fy + 1.0), SlabHalf::Top => (fy, fy + 0.5) };
                            // Helper to decide if neighbor is a full cube (not special)
                            let is_full_cube = |b: Block| -> bool {
                                match b {
                                    Block::Slab { .. } | Block::Stairs { .. } => false,
                                    Block::Air => false,
                                    _ => true,
                                }
                            };
                            // West neighbor (+X face on neighbor)
                            if x > 0 {
                                let nb = buf.get_local(x - 1, y, z);
                                if is_full_cube(nb) {
                                    if let Some(fm) = face_material_for(nb, 2) {
                                        let l0 = light.sample_face_local(x - 1, y, z, 2);
                                        let lv = match half {
                                            SlabHalf::Bottom => {
                                                let la = if y + 1 < sy { light.sample_face_local(x - 1, y + 1, z, 2) } else { l0 };
                                                l0.max(la).max(VISUAL_LIGHT_MIN)
                                            }
                                            SlabHalf::Top => {
                                                let lb = if y > 0 { light.sample_face_local(x - 1, y - 1, z, 2) } else { l0 };
                                                l0.max(lb).max(VISUAL_LIGHT_MIN)
                                            }
                                        };
                                        let rgba = [lv, lv, lv, 255];
                                        let mb = builds.entry(fm).or_default();
                                        let px = fx; // plane at x
                                        // +X face orientation (normal +X)
                                        mb.add_quad(
                                            Vector3::new(px, vis_y1, fz + 1.0),
                                            Vector3::new(px, vis_y1, fz),
                                            Vector3::new(px, vis_y0, fz),
                                            Vector3::new(px, vis_y0, fz + 1.0),
                                            Vector3::new(1.0, 0.0, 0.0),
                                            1.0,
                                            vis_y1 - vis_y0,
                                            false,
                                            rgba,
                                        );
                                    }
                                }
                            }
                            // East neighbor (-X face on neighbor)
                            if x + 1 < sx {
                                let nb = buf.get_local(x + 1, y, z);
                                if is_full_cube(nb) {
                                    if let Some(fm) = face_material_for(nb, 3) {
                                        let l0 = light.sample_face_local(x + 1, y, z, 3);
                                        let lv = match half {
                                            SlabHalf::Bottom => {
                                                let la = if y + 1 < sy { light.sample_face_local(x + 1, y + 1, z, 3) } else { l0 };
                                                l0.max(la).max(VISUAL_LIGHT_MIN)
                                            }
                                            SlabHalf::Top => {
                                                let lb = if y > 0 { light.sample_face_local(x + 1, y - 1, z, 3) } else { l0 };
                                                l0.max(lb).max(VISUAL_LIGHT_MIN)
                                            }
                                        };
                                        let rgba = [lv, lv, lv, 255];
                                        let mb = builds.entry(fm).or_default();
                                        let px = fx + 1.0; // plane at x+1
                                        // -X face orientation (normal -X)
                                        mb.add_quad(
                                            Vector3::new(px, vis_y1, fz),
                                            Vector3::new(px, vis_y1, fz + 1.0),
                                            Vector3::new(px, vis_y0, fz + 1.0),
                                            Vector3::new(px, vis_y0, fz),
                                            Vector3::new(-1.0, 0.0, 0.0),
                                            1.0,
                                            vis_y1 - vis_y0,
                                            false,
                                            rgba,
                                        );
                                    }
                                }
                            }
                            // North neighbor (+Z face on neighbor)
                            if z > 0 {
                                let nb = buf.get_local(x, y, z - 1);
                                if is_full_cube(nb) {
                                    if let Some(fm) = face_material_for(nb, 4) {
                                        let l0 = light.sample_face_local(x, y, z - 1, 4);
                                        let lv = match half {
                                            SlabHalf::Bottom => {
                                                let la = if y + 1 < sy { light.sample_face_local(x, y + 1, z - 1, 4) } else { l0 };
                                                l0.max(la).max(VISUAL_LIGHT_MIN)
                                            }
                                            SlabHalf::Top => {
                                                let lb = if y > 0 { light.sample_face_local(x, y - 1, z - 1, 4) } else { l0 };
                                                l0.max(lb).max(VISUAL_LIGHT_MIN)
                                            }
                                        };
                                        let rgba = [lv, lv, lv, 255];
                                        let mb = builds.entry(fm).or_default();
                                        let pz = fz; // plane at z
                                        // +Z face orientation (normal +Z)
                                        mb.add_quad(
                                            Vector3::new(fx + 1.0, vis_y1, pz),
                                            Vector3::new(fx, vis_y1, pz),
                                            Vector3::new(fx, vis_y0, pz),
                                            Vector3::new(fx + 1.0, vis_y0, pz),
                                            Vector3::new(0.0, 0.0, 1.0),
                                            1.0,
                                            vis_y1 - vis_y0,
                                            false,
                                            rgba,
                                        );
                                    }
                                }
                            }
                            // South neighbor (-Z face on neighbor)
                            if z + 1 < sz {
                                let nb = buf.get_local(x, y, z + 1);
                                if is_full_cube(nb) {
                                    if let Some(fm) = face_material_for(nb, 5) {
                                        let l0 = light.sample_face_local(x, y, z + 1, 5);
                                        let lv = match half {
                                            SlabHalf::Bottom => {
                                                let la = if y + 1 < sy { light.sample_face_local(x, y + 1, z + 1, 5) } else { l0 };
                                                l0.max(la).max(VISUAL_LIGHT_MIN)
                                            }
                                            SlabHalf::Top => {
                                                let lb = if y > 0 { light.sample_face_local(x, y - 1, z + 1, 5) } else { l0 };
                                                l0.max(lb).max(VISUAL_LIGHT_MIN)
                                            }
                                        };
                                        let rgba = [lv, lv, lv, 255];
                                        let mb = builds.entry(fm).or_default();
                                        let pz = fz + 1.0; // plane at z+1
                                        // -Z face orientation (normal -Z)
                                        mb.add_quad(
                                            Vector3::new(fx, vis_y1, pz),
                                            Vector3::new(fx + 1.0, vis_y1, pz),
                                            Vector3::new(fx + 1.0, vis_y0, pz),
                                            Vector3::new(fx, vis_y0, pz),
                                            Vector3::new(0.0, 0.0, -1.0),
                                            1.0,
                                            vis_y1 - vis_y0,
                                            false,
                                            rgba,
                                        );
                                    }
                                }
                            }
                        }
                        Block::Stairs { dir, half, key } => {
                            let fx = base_x as f32 + x as f32;
                            let fy = y as f32;
                            let fz = base_z as f32 + z as f32;
                            // Big slab across half-height depending on half
                            let (min_a, max_a) = match half {
                                SlabHalf::Bottom => (Vector3::new(fx, fy, fz), Vector3::new(fx + 1.0, fy + 0.5, fz + 1.0)),
                                SlabHalf::Top => (Vector3::new(fx, fy + 0.5, fz), Vector3::new(fx + 1.0, fy + 1.0, fz + 1.0)),
                            };
                            let keyc = key;
                            emit_box(
                                &mut builds,
                                buf,
                                world,
                                edits,
                                neighbors,
                                &light,
                                x,
                                y,
                                z,
                                base_x,
                                base_z,
                                &|face| face_material_for_key(keyc, face),
                                min_a,
                                max_a,
                            );
                            // Secondary half-depth slab on the back half toward facing
                            let (min_b, max_b) = match (dir, half) {
                                (Dir4::North, SlabHalf::Bottom) => (Vector3::new(fx, fy + 0.5, fz), Vector3::new(fx + 1.0, fy + 1.0, fz + 0.5)),
                                (Dir4::South, SlabHalf::Bottom) => (Vector3::new(fx, fy + 0.5, fz + 0.5), Vector3::new(fx + 1.0, fy + 1.0, fz + 1.0)),
                                (Dir4::West, SlabHalf::Bottom) => (Vector3::new(fx, fy + 0.5, fz), Vector3::new(fx + 0.5, fy + 1.0, fz + 1.0)),
                                (Dir4::East, SlabHalf::Bottom) => (Vector3::new(fx + 0.5, fy + 0.5, fz), Vector3::new(fx + 1.0, fy + 1.0, fz + 1.0)),
                                (Dir4::North, SlabHalf::Top) => (Vector3::new(fx, fy, fz), Vector3::new(fx + 1.0, fy + 0.5, fz + 0.5)),
                                (Dir4::South, SlabHalf::Top) => (Vector3::new(fx, fy, fz + 0.5), Vector3::new(fx + 1.0, fy + 0.5, fz + 1.0)),
                                (Dir4::West, SlabHalf::Top) => (Vector3::new(fx, fy, fz), Vector3::new(fx + 0.5, fy + 0.5, fz + 1.0)),
                                (Dir4::East, SlabHalf::Top) => (Vector3::new(fx + 0.5, fy, fz), Vector3::new(fx + 1.0, fy + 0.5, fz + 1.0)),
                            };
                            emit_box(
                                &mut builds,
                                buf,
                                world,
                                edits,
                                neighbors,
                                &light,
                                x,
                                y,
                                z,
                                base_x,
                                base_z,
                                &|face| face_material_for_key(keyc, face),
                                min_b,
                                max_b,
                            );

                            // Restore partial neighbor faces occluded by stair shape.
                            // Helper for light: combine face sample with one voxel above/below depending on which y-half we draw
                            let sample_lv = |nx: usize, ny: usize, nz: usize, face: usize, draw_top_half: bool| -> u8 {
                                let l0 = light.sample_face_local(nx, ny, nz, face);
                                let ladd = if draw_top_half {
                                    if ny + 1 < sy { light.sample_face_local(nx, ny + 1, nz, face) } else { l0 }
                                } else {
                                    if ny > 0 { light.sample_face_local(nx, ny - 1, nz, face) } else { l0 }
                                };
                                l0.max(ladd).max(VISUAL_LIGHT_MIN)
                            };

                            // WEST neighbor (+X face on neighbor at (x-1,y,z))
                            if x > 0 {
                                let nb = buf.get_local(x - 1, y, z);
                                // Only for full cubes
                                if let Some(fm) = if matches!(nb, Block::Slab{..} | Block::Stairs{..} | Block::Air) { None } else { face_material_for(nb, 2) } {
                                    let draw_top = matches!(half, SlabHalf::Bottom);
                                    let y0 = if draw_top { fy + 0.5 } else { fy };
                                    let y1 = if draw_top { fy + 1.0 } else { fy + 0.5 };
                                    let px = fx; // plane at x
                                    let rgba = {
                                        let lv = sample_lv(x - 1, y, z, 2, draw_top);
                                        [lv, lv, lv, 255]
                                    };
                                    let mb = builds.entry(fm).or_default();
                                    // Compute z segments visible
                                    let segs: &[(f32, f32)] = match (dir, half) {
                                        // Bottom: base occludes bottom half; riser occludes top half as:
                                        (Dir4::North, SlabHalf::Bottom) => &[(fz + 0.5, fz + 1.0)],
                                        (Dir4::South, SlabHalf::Bottom) => &[(fz, fz + 0.5)],
                                        (Dir4::West,  SlabHalf::Bottom) => &[],
                                        (Dir4::East,  SlabHalf::Bottom) => &[(fz, fz + 1.0)],
                                        // Top: base occludes top half; riser occludes bottom half as:
                                        (Dir4::North, SlabHalf::Top) => &[(fz + 0.5, fz + 1.0)],
                                        (Dir4::South, SlabHalf::Top) => &[(fz, fz + 0.5)],
                                        (Dir4::West,  SlabHalf::Top) => &[],
                                        (Dir4::East,  SlabHalf::Top) => &[(fz, fz + 1.0)],
                                    };
                                    for &(z0, z1) in segs.iter() {
                                        if z1 <= z0 { continue; }
                                        // +X face
                                        mb.add_quad(
                                            Vector3::new(px, y1, z1),
                                            Vector3::new(px, y1, z0),
                                            Vector3::new(px, y0, z0),
                                            Vector3::new(px, y0, z1),
                                            Vector3::new(1.0, 0.0, 0.0),
                                            z1 - z0,
                                            y1 - y0,
                                            false,
                                            rgba,
                                        );
                                    }
                                }
                            }
                            // EAST neighbor (-X face on neighbor at (x+1,y,z))
                            if x + 1 < sx {
                                let nb = buf.get_local(x + 1, y, z);
                                if let Some(fm) = if matches!(nb, Block::Slab{..} | Block::Stairs{..} | Block::Air) { None } else { face_material_for(nb, 3) } {
                                    let draw_top = matches!(half, SlabHalf::Bottom);
                                    let y0 = if draw_top { fy + 0.5 } else { fy };
                                    let y1 = if draw_top { fy + 1.0 } else { fy + 0.5 };
                                    let px = fx + 1.0;
                                    let rgba = {
                                        let lv = sample_lv(x + 1, y, z, 3, draw_top);
                                        [lv, lv, lv, 255]
                                    };
                                    let mb = builds.entry(fm).or_default();
                                    let segs: &[(f32, f32)] = match (dir, half) {
                                        (Dir4::North, SlabHalf::Bottom) => &[(fz + 0.5, fz + 1.0)],
                                        (Dir4::South, SlabHalf::Bottom) => &[(fz, fz + 0.5)],
                                        (Dir4::West,  SlabHalf::Bottom) => &[(fz, fz + 1.0)], // B at x half does not hit plane x+1 when West? Actually West B x in [fx,fx+0.5], plane at x+1 not intersected, so visible all top half
                                        (Dir4::East,  SlabHalf::Bottom) => &[],
                                        (Dir4::North, SlabHalf::Top) => &[(fz + 0.5, fz + 1.0)],
                                        (Dir4::South, SlabHalf::Top) => &[(fz, fz + 0.5)],
                                        (Dir4::West,  SlabHalf::Top) => &[(fz, fz + 1.0)],
                                        (Dir4::East,  SlabHalf::Top) => &[],
                                    };
                                    for &(z0, z1) in segs.iter() {
                                        if z1 <= z0 { continue; }
                                        // -X face
                                        mb.add_quad(
                                            Vector3::new(px, y1, z0),
                                            Vector3::new(px, y1, z1),
                                            Vector3::new(px, y0, z1),
                                            Vector3::new(px, y0, z0),
                                            Vector3::new(-1.0, 0.0, 0.0),
                                            z1 - z0,
                                            y1 - y0,
                                            false,
                                            rgba,
                                        );
                                    }
                                }
                            }
                            // NORTH neighbor (+Z face on neighbor at (x,y,z-1))
                            if z > 0 {
                                let nb = buf.get_local(x, y, z - 1);
                                if let Some(fm) = if matches!(nb, Block::Slab{..} | Block::Stairs{..} | Block::Air) { None } else { face_material_for(nb, 4) } {
                                    let draw_top = matches!(half, SlabHalf::Bottom);
                                    let y0 = if draw_top { fy + 0.5 } else { fy };
                                    let y1 = if draw_top { fy + 1.0 } else { fy + 0.5 };
                                    let pz = fz;
                                    let rgba = {
                                        let lv = sample_lv(x, y, z - 1, 4, draw_top);
                                        [lv, lv, lv, 255]
                                    };
                                    let mb = builds.entry(fm).or_default();
                                    // X segments visible (since plane is Z)
                                    let segs: &[(f32, f32)] = match (dir, half) {
                                        (Dir4::East,  SlabHalf::Bottom) => &[(fx + 0.5, fx + 1.0)],
                                        (Dir4::West,  SlabHalf::Bottom) => &[(fx, fx + 0.5)],
                                        (Dir4::North, SlabHalf::Bottom) => &[(fx, fx + 1.0)],
                                        (Dir4::South, SlabHalf::Bottom) => &[],
                                        (Dir4::East,  SlabHalf::Top) => &[(fx + 0.5, fx + 1.0)],
                                        (Dir4::West,  SlabHalf::Top) => &[(fx, fx + 0.5)],
                                        (Dir4::North, SlabHalf::Top) => &[(fx, fx + 1.0)],
                                        (Dir4::South, SlabHalf::Top) => &[],
                                    };
                                    for &(x0f, x1f) in segs.iter() {
                                        if x1f <= x0f { continue; }
                                        // +Z face
                                        mb.add_quad(
                                            Vector3::new(x1f, y1, pz),
                                            Vector3::new(x0f, y1, pz),
                                            Vector3::new(x0f, y0, pz),
                                            Vector3::new(x1f, y0, pz),
                                            Vector3::new(0.0, 0.0, 1.0),
                                            x1f - x0f,
                                            y1 - y0,
                                            false,
                                            rgba,
                                        );
                                    }
                                }
                            }
                            // SOUTH neighbor (-Z face on neighbor at (x,y,z+1))
                            if z + 1 < sz {
                                let nb = buf.get_local(x, y, z + 1);
                                if let Some(fm) = if matches!(nb, Block::Slab{..} | Block::Stairs{..} | Block::Air) { None } else { face_material_for(nb, 5) } {
                                    let draw_top = matches!(half, SlabHalf::Bottom);
                                    let y0 = if draw_top { fy + 0.5 } else { fy };
                                    let y1 = if draw_top { fy + 1.0 } else { fy + 0.5 };
                                    let pz = fz + 1.0;
                                    let rgba = {
                                        let lv = sample_lv(x, y, z + 1, 5, draw_top);
                                        [lv, lv, lv, 255]
                                    };
                                    let mb = builds.entry(fm).or_default();
                                    let segs: &[(f32, f32)] = match (dir, half) {
                                        (Dir4::East,  SlabHalf::Bottom) => &[(fx + 0.5, fx + 1.0)],
                                        (Dir4::West,  SlabHalf::Bottom) => &[(fx, fx + 0.5)],
                                        (Dir4::North, SlabHalf::Bottom) => &[],
                                        (Dir4::South, SlabHalf::Bottom) => &[(fx, fx + 1.0)],
                                        (Dir4::East,  SlabHalf::Top) => &[(fx + 0.5, fx + 1.0)],
                                        (Dir4::West,  SlabHalf::Top) => &[(fx, fx + 0.5)],
                                        (Dir4::North, SlabHalf::Top) => &[],
                                        (Dir4::South, SlabHalf::Top) => &[(fx, fx + 1.0)],
                                    };
                                    for &(x0f, x1f) in segs.iter() {
                                        if x1f <= x0f { continue; }
                                        // -Z face
                                        mb.add_quad(
                                            Vector3::new(x0f, y1, pz),
                                            Vector3::new(x1f, y1, pz),
                                            Vector3::new(x1f, y0, pz),
                                            Vector3::new(x0f, y0, pz),
                                            Vector3::new(0.0, 0.0, -1.0),
                                            x1f - x0f,
                                            y1 - y0,
                                            false,
                                            rgba,
                                        );
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        let bbox = BoundingBox::new(
            Vector3::new(base_x as f32, 0.0, base_z as f32),
            Vector3::new(
                base_x as f32 + sx as f32,
                sy as f32,
                base_z as f32 + sz as f32,
            ),
        );
    let light_borders = Some(LightBorders::from_grid(&light));
    return Some((
        ChunkMeshCPU {
            cx,
            cz,
            bbox,
            parts: builds,
        },
        light_borders,
    ));
}

pub fn upload_chunk_mesh(
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    cpu: ChunkMeshCPU,
    tex_cache: &mut TextureCache,
) -> Option<ChunkRender> {
    let mut parts_gpu = Vec::new();
    for (fm, mb) in cpu.parts.into_iter() {
        if mb.idx.is_empty() {
            continue;
        }
        // allocate mesh
        let mut raw: raylib::ffi::Mesh = unsafe { std::mem::zeroed() };
        raw.vertexCount = (mb.pos.len() / 3) as i32;
        raw.triangleCount = (mb.idx.len() / 3) as i32;
        unsafe {
            let vbytes = (mb.pos.len() * std::mem::size_of::<f32>()) as u32;
            let nbytes = (mb.norm.len() * std::mem::size_of::<f32>()) as u32;
            let tbytes = (mb.uv.len() * std::mem::size_of::<f32>()) as u32;
            let ibytes = (mb.idx.len() * std::mem::size_of::<u16>()) as u32;
            let cbytes = (mb.col.len() * std::mem::size_of::<u8>()) as u32;
            raw.vertices = raylib::ffi::MemAlloc(vbytes) as *mut f32;
            raw.normals = raylib::ffi::MemAlloc(nbytes) as *mut f32;
            raw.texcoords = raylib::ffi::MemAlloc(tbytes) as *mut f32;
            raw.indices = raylib::ffi::MemAlloc(ibytes) as *mut u16;
            raw.colors = raylib::ffi::MemAlloc(cbytes) as *mut u8;
            std::ptr::copy_nonoverlapping(mb.pos.as_ptr(), raw.vertices, mb.pos.len());
            std::ptr::copy_nonoverlapping(mb.norm.as_ptr(), raw.normals, mb.norm.len());
            std::ptr::copy_nonoverlapping(mb.uv.as_ptr(), raw.texcoords, mb.uv.len());
            std::ptr::copy_nonoverlapping(mb.idx.as_ptr(), raw.indices, mb.idx.len());
            std::ptr::copy_nonoverlapping(mb.col.as_ptr(), raw.colors, mb.col.len());
        }
        let mut mesh = unsafe { raylib::core::models::Mesh::from_raw(raw) };
        unsafe {
            mesh.upload(false);
        }
        let model = rl
            .load_model_from_mesh(thread, unsafe { mesh.make_weak() })
            .ok()?;
        // Get cached texture and assign
        let mut model = model;
        if let Some(mat) = model.materials_mut().get_mut(0) {
            if let Some(tex) = tex_cache.get_or_load(rl, thread, &fm.texture_candidates()) {
                mat.set_material_texture(
                    raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO,
                    tex,
                );
            } else {
                // No texture available; leave material as-is
            }
        }
        parts_gpu.push((fm, model));
    }
    Some(ChunkRender {
        cx: cpu.cx,
        cz: cpu.cz,
        bbox: cpu.bbox,
        parts: parts_gpu,
    })
}

// Purged world-based synchronous build path; buffer-based pipeline is authoritative.

// Simple per-app texture cache keyed by file path; loads each texture once and reuses it across chunks.
pub struct TextureCache {
    map: HashMap<&'static str, raylib::core::texture::Texture2D>,
}

// Local-body mesher: emits vertices in local-space [0..sx, 0..sz], no world/lighting deps.
pub fn build_voxel_body_cpu_buf(buf: &ChunkBuf, ambient: u8) -> ChunkMeshCPU {
    let sx = buf.sx;
    let sy = buf.sy;
    let sz = buf.sz;

    // Unified path via meshing_core
    {
        #[inline]
        fn solid_local(buf: &ChunkBuf, x: i32, y: i32, z: i32) -> bool {
            if x < 0 || y < 0 || z < 0 {
                return false;
            }
            let (xu, yu, zu) = (x as usize, y as usize, z as usize);
            if xu >= buf.sx || yu >= buf.sy || zu >= buf.sz {
                return false;
            }
            buf.get_local(xu, yu, zu).is_solid()
        }

        #[inline]
        fn face_light(face: usize, ambient: u8) -> u8 {
            match face {
                0 => ambient.saturating_add(40).min(255),
                1 => ambient.saturating_sub(60),
                _ => ambient,
            }
        }

        let flip_v = [false, true, false, true, false, true];
        let builds = crate::meshing_core::build_mesh_core(
            buf,
            0,
            0,
            flip_v,
            None,
            |x, y, z, face, here| {
                if !here.is_solid() {
                    return None;
                }
                let (nx, ny, nz) = match face {
                    0 => (x as i32, y as i32 + 1, z as i32),
                    1 => (x as i32, y as i32 - 1, z as i32),
                    2 => (x as i32 + 1, y as i32, z as i32),
                    3 => (x as i32 - 1, y as i32, z as i32),
                    4 => (x as i32, y as i32, z as i32 + 1),
                    5 => (x as i32, y as i32, z as i32 - 1),
                    _ => unreachable!(),
                };
                if solid_local(buf, nx, ny, nz) {
                    return None;
                }
                if let Some(fm) = face_material_for(here, face) {
                    let l = face_light(face, ambient);
                    Some((fm, l))
                } else {
                    None
                }
            },
        );
        let bbox = BoundingBox::new(
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(sx as f32, sy as f32, sz as f32),
        );
        return ChunkMeshCPU {
            cx: 0,
            cz: 0,
            bbox,
            parts: builds,
        };
    }
}

impl TextureCache {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn get_or_load<'a>(
        &'a mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        candidates: &[&'static str],
    ) -> Option<&'a raylib::core::texture::Texture2D> {
        // Pick first candidate that either exists in cache or loads successfully
        for &p in candidates {
            if self.map.contains_key(p) {
                return self.map.get(p);
            }
            if let Ok(t) = rl.load_texture(thread, p) {
                t.set_texture_filter(thread, raylib::consts::TextureFilter::TEXTURE_FILTER_POINT);
                t.set_texture_wrap(thread, raylib::consts::TextureWrap::TEXTURE_WRAP_REPEAT);
                self.map.insert(p, t);
                return self.map.get(p);
            }
        }
        None
    }
}

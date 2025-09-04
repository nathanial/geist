use crate::voxel::{Block, World, TreeSpecies};
use raylib::prelude::*;
use raylib::core::math::BoundingBox;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FaceMaterial {
    GrassTop,
    GrassSide, // needs V-flip
    Dirt,
    Stone,
    Sand,
    Snow,
    WoodTop(TreeSpecies),
    WoodSide(TreeSpecies),
    Leaves(TreeSpecies),
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
            FaceMaterial::WoodTop(sp) => match sp {
                TreeSpecies::Oak => vec!["assets/blocks/log_oak_top.png", "assets/blocks/log_big_oak_top.png"],
                TreeSpecies::DarkOak => vec!["assets/blocks/log_big_oak_top.png", "assets/blocks/log_oak_top.png"],
                TreeSpecies::Birch => vec!["assets/blocks/log_birch_top.png"],
                TreeSpecies::Spruce => vec!["assets/blocks/log_spruce_top.png"],
                TreeSpecies::Jungle => vec!["assets/blocks/log_jungle_top.png"],
                TreeSpecies::Acacia => vec!["assets/blocks/log_acacia_top.png"],
            },
            FaceMaterial::WoodSide(sp) => match sp {
                TreeSpecies::Oak => vec!["assets/blocks/log_oak.png", "assets/blocks/log_big_oak.png"],
                TreeSpecies::DarkOak => vec!["assets/blocks/log_big_oak.png", "assets/blocks/log_oak.png"],
                TreeSpecies::Birch => vec!["assets/blocks/log_birch.png"],
                TreeSpecies::Spruce => vec!["assets/blocks/log_spruce.png"],
                TreeSpecies::Jungle => vec!["assets/blocks/log_jungle.png"],
                TreeSpecies::Acacia => vec!["assets/blocks/log_acacia.png"],
            },
            FaceMaterial::Leaves(sp) => match sp {
                // Prefer opaque variants first to avoid alpha
                TreeSpecies::Oak => vec!["assets/blocks/leaves_oak_opaque.png", "assets/blocks/leaves_oak.png"],
                TreeSpecies::DarkOak => vec!["assets/blocks/leaves_big_oak_opaque.png", "assets/blocks/leaves_big_oak.png"],
                TreeSpecies::Birch => vec!["assets/blocks/leaves_birch_opaque.png", "assets/blocks/leaves_birch.png"],
                TreeSpecies::Spruce => vec!["assets/blocks/leaves_spruce_opaque.png", "assets/blocks/leaves_spruce.png"],
                TreeSpecies::Jungle => vec!["assets/blocks/leaves_jungle_opaque.png", "assets/blocks/leaves_jungle.png"],
                TreeSpecies::Acacia => vec!["assets/blocks/leaves_acacia_opaque.png", "assets/blocks/leaves_acacia.png"],
            },
        }.to_vec()
    }
}

#[derive(Default)]
struct MeshBuild {
    pos: Vec<f32>,
    norm: Vec<f32>,
    uv: Vec<f32>,
    idx: Vec<u16>,
}

impl MeshBuild {
    fn add_quad(&mut self, a: Vector3, b: Vector3, c: Vector3, d: Vector3, n: Vector3, u1: f32, v1: f32, flip_v: bool) {
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
            for uv in &mut uvs { uv.1 = v1 - uv.1; }
        }

        for i in 0..4 {
            self.pos.extend_from_slice(&[vs[i].x, vs[i].y, vs[i].z]);
            self.norm.extend_from_slice(&[n.x, n.y, n.z]);
            self.uv.extend_from_slice(&[uvs[i].0, uvs[i].1]);
        }
        // Two triangles: (0,1,2) and (0,2,3)
        self.idx.extend_from_slice(&[
            (base + 0) as u16, (base + 1) as u16, (base + 2) as u16,
            (base + 0) as u16, (base + 2) as u16, (base + 3) as u16,
        ]);
    }
}

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
        Block::Wood(sp) => match face {
            0 | 1 => Some(FaceMaterial::WoodTop(sp)),
            2 | 3 | 4 | 5 => Some(FaceMaterial::WoodSide(sp)),
            _ => None,
        },
        Block::Leaves(sp) => Some(FaceMaterial::Leaves(sp)),
    }
}

#[inline]
#[inline]
fn is_occluder_for(world: &World, here: Block, nx: i32, ny: i32, nz: i32) -> bool {
    if !here.is_solid() { return false; }
    let nb = world.block_at(nx, ny, nz);
    nb.is_solid()
}

pub struct ChunkRender {
    pub cx: i32,
    pub cz: i32,
    pub bbox: BoundingBox,
    pub parts: Vec<(FaceMaterial, raylib::core::models::Model, raylib::core::texture::Texture2D)>,
}

pub fn build_chunk_greedy(
    world: &World,
    cx: i32,
    cz: i32,
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
) -> Option<ChunkRender> {
    let sx = world.chunk_size_x;
    let sy = world.chunk_size_y;
    let sz = world.chunk_size_z;
    let base_x = cx * sx as i32;
    let base_z = cz * sz as i32;

    use std::collections::HashMap;
    let mut builds: HashMap<FaceMaterial, MeshBuild> = HashMap::new();

    // Y layers: top (+Y) and bottom (-Y)
    for y in 0..sy {
        // top faces at y+1 plane
        {
            let mut mask: Vec<Option<FaceMaterial>> = vec![None; sx * sz];
            for z in 0..sz { for x in 0..sx {
                let gx = base_x + x as i32; let gz = base_z + z as i32;
                let here = world.block_at(gx, y as i32, gz);
                if here.is_solid() {
                    let neigh = is_occluder_for(world, here, gx as i32, (y as i32) + 1, gz as i32);
                    if !neigh { mask[z * sx + x] = face_material_for(here, 0); }
                }
            }}
            // greedy merge on mask (x,z)
            let mut used = vec![false; sx * sz];
            for z in 0..sz { for x in 0..sx {
                let code = mask[z * sx + x]; if code.is_none() || used[z * sx + x] { continue; }
                let codev = code.unwrap();
                let mut w = 1; while x + w < sx && mask[z * sx + x + w] == code && !used[z * sx + x + w] { w += 1; }
                let mut h = 1; 'expand: while z + h < sz {
                    for i in 0..w { if mask[(z + h) * sx + (x + i)] != code || used[(z + h) * sx + (x + i)] { break 'expand; } }
                    h += 1;
                }
                let fx = (base_x + x as i32) as f32; let fz = (base_z + z as i32) as f32; let fy = (y as f32) + 1.0;
                let u1 = w as f32; let v1 = h as f32;
                let mb = builds.entry(codev).or_default();
                mb.add_quad(
                    Vector3::new(fx, fy, fz),
                    Vector3::new(fx + u1, fy, fz),
                    Vector3::new(fx + u1, fy, fz + v1),
                    Vector3::new(fx, fy, fz + v1),
                    Vector3::new(0.0, 1.0, 0.0),
                    u1, v1,
                    false,
                );
                for zz in 0..h { for xx in 0..w { used[(z + zz) * sx + (x + xx)] = true; } }
            }}
        }
        // bottom faces at y plane
        {
            let mut mask: Vec<Option<FaceMaterial>> = vec![None; sx * sz];
            for z in 0..sz { for x in 0..sx {
                let gx = base_x + x as i32; let gz = base_z + z as i32;
                let here = world.block_at(gx, y as i32, gz);
                if here.is_solid() {
                    let neigh = is_occluder_for(world, here, gx as i32, (y as i32) - 1, gz as i32);
                    if !neigh { mask[z * sx + x] = face_material_for(here, 1); }
                }
            }}
            let mut used = vec![false; sx * sz];
            for z in 0..sz { for x in 0..sx {
                let code = mask[z * sx + x]; if code.is_none() || used[z * sx + x] { continue; }
                let codev = code.unwrap();
                let mut w = 1; while x + w < sx && mask[z * sx + x + w] == code && !used[z * sx + x + w] { w += 1; }
                let mut h = 1; 'expand: while z + h < sz {
                    for i in 0..w { if mask[(z + h) * sx + (x + i)] != code || used[(z + h) * sx + (x + i)] { break 'expand; } }
                    h += 1;
                }
                let fx = (base_x + x as i32) as f32; let fz = (base_z + z as i32) as f32; let fy = y as f32;
                let u1 = w as f32; let v1 = h as f32;
                let mb = builds.entry(codev).or_default();
                mb.add_quad(
                    Vector3::new(fx, fy, fz + v1),
                    Vector3::new(fx + u1, fy, fz + v1),
                    Vector3::new(fx + u1, fy, fz),
                    Vector3::new(fx, fy, fz),
                    Vector3::new(0.0, -1.0, 0.0),
                    u1, v1,
                    false,
                );
                for zz in 0..h { for xx in 0..w { used[(z + zz) * sx + (x + xx)] = true; } }
            }}
        }
    }

    // X planes: negative and positive
    for x in 0..sx {
        for &pos in &[false, true] {
            let mut mask: Vec<Option<FaceMaterial>> = vec![None; sz * sy];
            for z in 0..sz { for y in 0..sy {
                let gx = base_x + x as i32; let gz = base_z + z as i32; let gy = y as i32;
                let here = world.block_at(gx, gy, gz);
                if here.is_solid() {
                    let neigh = if pos {
                        is_occluder_for(world, here, (gx as i32) + 1, gy as i32, gz as i32)
                    } else {
                        is_occluder_for(world, here, (gx as i32) - 1, gy as i32, gz as i32)
                    };
                    if !neigh { mask[y * sz + z] = face_material_for(here, if pos { 2 } else { 3 }); }
                }
            }}
            let mut used = vec![false; sz * sy];
            for y in 0..sy { for z in 0..sz {
                let code = mask[y * sz + z]; if code.is_none() || used[y * sz + z] { continue; }
                let codev = code.unwrap();
                let mut h = 1; while y + h < sy && mask[(y + h) * sz + z] == code && !used[(y + h) * sz + z] { h += 1; }
                let mut w = 1; 'expand: while z + w < sz {
                    for i in 0..h { if mask[(y + i) * sz + (z + w)] != code || used[(y + i) * sz + (z + w)] { break 'expand; } }
                    w += 1;
                }
                let fx = (base_x + x as i32) as f32 + if pos { 1.0 } else { 0.0 };
                let fy = y as f32; let fz = (base_z + z as i32) as f32;
                let u1 = w as f32; let v1 = h as f32;
                let mb = builds.entry(codev).or_default();
                if !pos {
                    mb.add_quad(
                        Vector3::new(fx, fy + v1, fz),
                        Vector3::new(fx, fy + v1, fz + u1),
                        Vector3::new(fx, fy, fz + u1),
                        Vector3::new(fx, fy, fz),
                        Vector3::new(-1.0, 0.0, 0.0),
                        u1, v1,
                        false,
                    );
                } else {
                    mb.add_quad(
                        Vector3::new(fx, fy + v1, fz + u1),
                        Vector3::new(fx, fy + v1, fz),
                        Vector3::new(fx, fy, fz),
                        Vector3::new(fx, fy, fz + u1),
                        Vector3::new(1.0, 0.0, 0.0),
                        u1, v1,
                        false,
                    );
                }
                for yy in 0..v1 as usize { for zz in 0..u1 as usize { used[(y + yy) * sz + (z + zz)] = true; } }
            }}
        }
    }

    // Z planes: negative and positive
    for z in 0..sz {
        for &pos in &[false, true] {
            let mut mask: Vec<Option<FaceMaterial>> = vec![None; sx * sy];
            for x in 0..sx { for y in 0..sy {
                let gx = base_x + x as i32; let gz = base_z + z as i32; let gy = y as i32;
                let here = world.block_at(gx, gy, gz);
                if here.is_solid() {
                    let neigh = if pos {
                        is_occluder_for(world, here, gx as i32, gy as i32, (gz as i32) + 1)
                    } else {
                        is_occluder_for(world, here, gx as i32, gy as i32, (gz as i32) - 1)
                    };
                    if !neigh { mask[y * sx + x] = face_material_for(here, if pos { 4 } else { 5 }); }
                }
            }}
            let mut used = vec![false; sx * sy];
            for y in 0..sy { for x in 0..sx {
                let code = mask[y * sx + x]; if code.is_none() || used[y * sx + x] { continue; }
                let codev = code.unwrap();
                let mut h = 1; while y + h < sy && mask[(y + h) * sx + x] == code && !used[(y + h) * sx + x] { h += 1; }
                let mut w = 1; 'expand: while x + w < sx {
                    for i in 0..h { if mask[(y + i) * sx + (x + w)] != code || used[(y + i) * sx + (x + w)] { break 'expand; } }
                    w += 1;
                }
                let fx = (base_x + x as i32) as f32; let fy = y as f32; let fz = (base_z + z as i32) as f32 + if pos { 1.0 } else { 0.0 };
                let u1 = w as f32; let v1 = h as f32;
                let mb = builds.entry(codev).or_default();
                if !pos {
                    mb.add_quad(
                        Vector3::new(fx, fy + v1, fz),
                        Vector3::new(fx + u1, fy + v1, fz),
                        Vector3::new(fx + u1, fy, fz),
                        Vector3::new(fx, fy, fz),
                        Vector3::new(0.0, 0.0, -1.0),
                        u1, v1,
                        false,
                    );
                } else {
                    mb.add_quad(
                        Vector3::new(fx + u1, fy + v1, fz),
                        Vector3::new(fx, fy + v1, fz),
                        Vector3::new(fx, fy, fz),
                        Vector3::new(fx + u1, fy, fz),
                        Vector3::new(0.0, 0.0, 1.0),
                        u1, v1,
                        false,
                    );
                }
                for yy in 0..v1 as usize { for xx in 0..u1 as usize { used[(y + yy) * sx + (x + xx)] = true; } }
            }}
        }
    }

    // Convert MeshBuilds to Models
    let mut parts = Vec::new();
    for (fm, mb) in builds.into_iter() {
        if mb.idx.is_empty() { continue; }
        // allocate mesh
        let mut raw: raylib::ffi::Mesh = unsafe { std::mem::zeroed() };
        raw.vertexCount = (mb.pos.len() / 3) as i32;
        raw.triangleCount = (mb.idx.len() / 3) as i32;
        unsafe {
            use std::ffi::c_void;
            let vbytes = (mb.pos.len() * std::mem::size_of::<f32>()) as u32;
            let nbytes = (mb.norm.len() * std::mem::size_of::<f32>()) as u32;
            let tbytes = (mb.uv.len() * std::mem::size_of::<f32>()) as u32;
            let ibytes = (mb.idx.len() * std::mem::size_of::<u16>()) as u32;
            raw.vertices = raylib::ffi::MemAlloc(vbytes) as *mut f32;
            raw.normals = raylib::ffi::MemAlloc(nbytes) as *mut f32;
            raw.texcoords = raylib::ffi::MemAlloc(tbytes) as *mut f32;
            raw.indices = raylib::ffi::MemAlloc(ibytes) as *mut u16;
            std::ptr::copy_nonoverlapping(mb.pos.as_ptr(), raw.vertices, mb.pos.len());
            std::ptr::copy_nonoverlapping(mb.norm.as_ptr(), raw.normals, mb.norm.len());
            std::ptr::copy_nonoverlapping(mb.uv.as_ptr(), raw.texcoords, mb.uv.len());
            std::ptr::copy_nonoverlapping(mb.idx.as_ptr(), raw.indices, mb.idx.len());
        }

        let mut mesh = unsafe { raylib::core::models::Mesh::from_raw(raw) };
        unsafe { mesh.upload(false); } // static

        let model = rl.load_model_from_mesh(thread, unsafe { mesh.make_weak() }).ok()?;
        // Load texture
        let mut tex_opt = None;
        for p in fm.texture_candidates() {
            if let Ok(t) = rl.load_texture(thread, p) { tex_opt = Some(t); break; }
        }
        let tex = tex_opt?;
        tex.set_texture_filter(thread, raylib::consts::TextureFilter::TEXTURE_FILTER_POINT);
        tex.set_texture_wrap(thread, raylib::consts::TextureWrap::TEXTURE_WRAP_REPEAT);

        let mut model = model;
        if let Some(mat) = model.materials_mut().get_mut(0) {
            mat.set_material_texture(raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO, &tex);
        }
        parts.push((fm, model, tex));
    }

    let bbox = BoundingBox::new(
        Vector3::new(base_x as f32, 0.0, base_z as f32),
        Vector3::new(base_x as f32 + sx as f32, sy as f32, base_z as f32 + sz as f32),
    );
    Some(ChunkRender { cx, cz, bbox, parts })
}

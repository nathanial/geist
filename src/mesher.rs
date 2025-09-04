use crate::voxel::{Block, World};
use raylib::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FaceMaterial {
    GrassTop,
    GrassSide, // needs V-flip
    Dirt,
    Stone,
    Sand,
    Snow,
}

impl FaceMaterial {
    pub fn texture_candidates(&self) -> &'static [&'static str] {
        match self {
            FaceMaterial::GrassTop => &["assets/blocks/grass_top.png"],
            FaceMaterial::GrassSide => &["assets/blocks/grass_side.png"],
            FaceMaterial::Dirt => &["assets/blocks/dirt.png"],
            FaceMaterial::Stone => &["assets/blocks/stone.png"],
            FaceMaterial::Sand => &["assets/blocks/sand.png"],
            FaceMaterial::Snow => &["assets/blocks/snow.png"],
        }
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
        let vs = [a, d, c, b];
        // uvs: (0,0) (0,v1) (u1,v1) (u1,0)
        let mut uvs = [(0.0, 0.0), (0.0, v1), (u1, v1), (u1, 0.0)];
        if flip_v {
            for uv in &mut uvs { uv.1 = v1 - uv.1; }
        }
        for i in 0..4 {
            self.pos.extend_from_slice(&[vs[i].x, vs[i].y, vs[i].z]);
            self.norm.extend_from_slice(&[n.x, n.y, n.z]);
            self.uv.extend_from_slice(&[uvs[i].0, uvs[i].1]);
        }
        // two triangles: (0,1,2) and (0,2,3) in our vs ordering
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
    }
}

pub struct ChunkRender {
    pub cx: usize,
    pub cz: usize,
    pub parts: Vec<(FaceMaterial, raylib::core::models::Model, raylib::core::texture::Texture2D)>,
}

pub fn build_chunk_greedy(
    world: &World,
    cx: usize,
    cz: usize,
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
) -> Option<ChunkRender> {
    let sx = world.chunk_size_x;
    let sy = world.chunk_size_y;
    let sz = world.chunk_size_z;
    let base_x = cx * sx;
    let base_z = cz * sz;

    use std::collections::HashMap;
    let mut builds: HashMap<FaceMaterial, MeshBuild> = HashMap::new();

    // Y layers: top (+Y) and bottom (-Y)
    for y in 0..sy {
        // top faces at y+1 plane
        {
            let mut mask: Vec<Option<FaceMaterial>> = vec![None; sx * sz];
            for z in 0..sz { for x in 0..sx {
                let gx = base_x + x; let gz = base_z + z;
                let here = world.get(gx, y, gz);
                if here.is_solid() {
                    let neigh = if y + 1 < sy { world.get(gx, y + 1, gz).is_solid() } else { false };
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
                let fx = (base_x + x) as f32; let fz = (base_z + z) as f32; let fy = (y as f32) + 1.0;
                let u1 = w as f32; let v1 = h as f32;
                let mb = builds.entry(codev).or_default();
                mb.add_quad(
                    Vector3::new(fx, fy, fz),
                    Vector3::new(fx + u1, fy, fz),
                    Vector3::new(fx + u1, fy, fz + v1),
                    Vector3::new(fx, fy, fz + v1),
                    Vector3::new(0.0, 1.0, 0.0),
                    u1, v1,
                    matches!(codev, FaceMaterial::GrassSide),
                );
                for zz in 0..h { for xx in 0..w { used[(z + zz) * sx + (x + xx)] = true; } }
            }}
        }
        // bottom faces at y plane
        {
            let mut mask: Vec<Option<FaceMaterial>> = vec![None; sx * sz];
            for z in 0..sz { for x in 0..sx {
                let gx = base_x + x; let gz = base_z + z;
                let here = world.get(gx, y, gz);
                if here.is_solid() {
                    let neigh = if y > 0 { world.get(gx, y - 1, gz).is_solid() } else { false };
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
                let fx = (base_x + x) as f32; let fz = (base_z + z) as f32; let fy = y as f32;
                let u1 = w as f32; let v1 = h as f32;
                let mb = builds.entry(codev).or_default();
                mb.add_quad(
                    Vector3::new(fx, fy, fz + v1),
                    Vector3::new(fx + u1, fy, fz + v1),
                    Vector3::new(fx + u1, fy, fz),
                    Vector3::new(fx, fy, fz),
                    Vector3::new(0.0, -1.0, 0.0),
                    u1, v1,
                    matches!(codev, FaceMaterial::GrassSide),
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
                let gx = base_x + x; let gz = base_z + z; let gy = y;
                let here = world.get(gx, gy, gz);
                if here.is_solid() {
                    let neigh = if pos { world.get(gx + 1, gy, gz).is_solid() } else { gx > 0 && world.get(gx - 1, gy, gz).is_solid() };
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
                let fx = (base_x + x) as f32 + if pos { 1.0 } else { 0.0 };
                let fy = y as f32; let fz = (base_z + z) as f32;
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
                        matches!(codev, FaceMaterial::GrassSide),
                    );
                } else {
                    mb.add_quad(
                        Vector3::new(fx, fy + v1, fz + u1),
                        Vector3::new(fx, fy + v1, fz),
                        Vector3::new(fx, fy, fz),
                        Vector3::new(fx, fy, fz + u1),
                        Vector3::new(1.0, 0.0, 0.0),
                        u1, v1,
                        matches!(codev, FaceMaterial::GrassSide),
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
                let gx = base_x + x; let gz = base_z + z; let gy = y;
                let here = world.get(gx, gy, gz);
                if here.is_solid() {
                    let neigh = if pos { world.get(gx, gy, gz + 1).is_solid() } else { gz > 0 && world.get(gx, gy, gz - 1).is_solid() };
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
                let fx = (base_x + x) as f32; let fy = y as f32; let fz = (base_z + z) as f32 + if pos { 1.0 } else { 0.0 };
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
                        matches!(codev, FaceMaterial::GrassSide),
                    );
                } else {
                    mb.add_quad(
                        Vector3::new(fx + u1, fy + v1, fz),
                        Vector3::new(fx, fy + v1, fz),
                        Vector3::new(fx, fy, fz),
                        Vector3::new(fx + u1, fy, fz),
                        Vector3::new(0.0, 0.0, 1.0),
                        u1, v1,
                        matches!(codev, FaceMaterial::GrassSide),
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

    Some(ChunkRender { cx, cz, parts })
}

use crate::chunkbuf::ChunkBuf;
use crate::lighting::{LightBorders, LightGrid, LightingStore};
use crate::voxel::{Block, TreeSpecies, World};
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
    WoodTop(TreeSpecies),
    WoodSide(TreeSpecies),
    Leaves(TreeSpecies),
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
    fn add_quad(
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

// Generic greedy-rectangle sweep over a 2D mask. The mask is width*height laid out row-major.
// For each maximal rectangle of identical Some(code), calls `emit(x, y, w, h, code)` once.
#[inline]
fn greedy_rects(
    width: usize,
    height: usize,
    mask: &mut [Option<(FaceMaterial, u8)>],
    mut emit: impl FnMut(usize, usize, usize, usize, (FaceMaterial, u8)),
) {
    let mut used = vec![false; width * height];
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let code = mask[idx];
            if code.is_none() || used[idx] {
                continue;
            }
            let mut w = 1;
            while x + w < width
                && mask[y * width + (x + w)] == code
                && !used[y * width + (x + w)]
            {
                w += 1;
            }
            let mut h = 1;
            'expand: while y + h < height {
                for i in 0..w {
                    let j = (y + h) * width + (x + i);
                    if mask[j] != code || used[j] {
                        break 'expand;
                    }
                }
                h += 1;
            }
            // Safe to unwrap because we checked is_some above.
            emit(x, y, w, h, code.unwrap());
            for yy in 0..h {
                for xx in 0..w {
                    used[(y + yy) * width + (x + xx)] = true;
                }
            }
        }
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
        Block::Glowstone => Some(FaceMaterial::Glowstone),
        Block::Beacon => Some(FaceMaterial::Beacon),
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
        return buf.get_local(lx, ly, lz).is_solid();
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
    nb.is_solid()
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

    use std::collections::HashMap;
    let mut builds: HashMap<FaceMaterial, MeshBuild> = HashMap::new();
    let light = match lighting {
        Some(store) => LightGrid::compute_with_borders_buf(buf, store),
        None => return None,
    };

    // Y layers: top and bottom
    for y in 0..sy {
        // top faces
        {
            let mut mask: Vec<Option<(FaceMaterial, u8)>> = vec![None; sx * sz];
            for z in 0..sz {
                for x in 0..sx {
                    let here = buf.get_local(x, y, z);
                    if here.is_solid() {
                        let gx = base_x + x as i32;
                        let gz = base_z + z as i32;
                        let neigh =
                            is_occluder(buf, world, edits, neighbors, here, gx, (y as i32) + 1, gz);
                        if !neigh {
                            if let Some(fm) = face_material_for(here, 0) {
                                let l = light.sample_face_local(x, y, z, 0);
                                mask[z * sx + x] = Some((fm, l));
                            }
                        }
                    }
                }
            }
            greedy_rects(sx, sz, &mut mask, |x, z, w, h, codev| {
                let fx = (base_x + x as i32) as f32;
                let fz = (base_z + z as i32) as f32;
                let fy = (y as f32) + 1.0;
                let u1 = w as f32;
                let v1 = h as f32;
                let mb = builds.entry(codev.0).or_default();
                let lv = codev.1.max(VISUAL_LIGHT_MIN);
                let rgba = [lv, lv, lv, 255];
                mb.add_quad(
                    Vector3::new(fx, fy, fz),
                    Vector3::new(fx + u1, fy, fz),
                    Vector3::new(fx + u1, fy, fz + v1),
                    Vector3::new(fx, fy, fz + v1),
                    Vector3::new(0.0, 1.0, 0.0),
                    u1,
                    v1,
                    false,
                    rgba,
                );
            });
        }
        // bottom faces
        {
            let mut mask: Vec<Option<(FaceMaterial, u8)>> = vec![None; sx * sz];
            for z in 0..sz {
                for x in 0..sx {
                    let here = buf.get_local(x, y, z);
                    if here.is_solid() {
                        let gx = base_x + x as i32;
                        let gz = base_z + z as i32;
                        let neigh =
                            is_occluder(buf, world, edits, neighbors, here, gx, (y as i32) - 1, gz);
                        if !neigh {
                            if let Some(fm) = face_material_for(here, 1) {
                                let l = light.sample_face_local(x, y, z, 1);
                                mask[z * sx + x] = Some((fm, l));
                            }
                        }
                    }
                }
            }
            greedy_rects(sx, sz, &mut mask, |x, z, w, h, codev| {
                let fx = (base_x + x as i32) as f32;
                let fz = (base_z + z as i32) as f32;
                let fy = y as f32;
                let u1 = w as f32;
                let v1 = h as f32;
                let mb = builds.entry(codev.0).or_default();
                let lv = codev.1.max(VISUAL_LIGHT_MIN);
                let rgba = [lv, lv, lv, 255];
                mb.add_quad(
                    Vector3::new(fx, fy, fz + v1),
                    Vector3::new(fx + u1, fy, fz + v1),
                    Vector3::new(fx + u1, fy, fz),
                    Vector3::new(fx, fy, fz),
                    Vector3::new(0.0, -1.0, 0.0),
                    u1,
                    v1,
                    false,
                    rgba,
                );
            });
        }
    }

    // X planes
    for x in 0..sx {
        for &pos in &[false, true] {
            let mut mask: Vec<Option<(FaceMaterial, u8)>> = vec![None; sz * sy];
            for z in 0..sz {
                for y in 0..sy {
                    let here = buf.get_local(x, y, z);
                    if here.is_solid() {
                        let gx = base_x + x as i32;
                        let gz = base_z + z as i32;
                        let gy = y as i32;
                        let neigh = if pos {
                            is_occluder(buf, world, edits, neighbors, here, gx + 1, gy, gz)
                        } else {
                            is_occluder(buf, world, edits, neighbors, here, gx - 1, gy, gz)
                        };
                        if !neigh {
                            if let Some(fm) = face_material_for(here, if pos { 2 } else { 3 }) {
                                let l = light.sample_face_local(x, y, z, if pos { 2 } else { 3 });
                                mask[y * sz + z] = Some((fm, l));
                            }
                        }
                    }
                }
            }
            greedy_rects(sz, sy, &mut mask, |z, y, w, h, codev| {
                let fx = (base_x + x as i32) as f32 + if pos { 1.0 } else { 0.0 };
                let fy = y as f32;
                let fz = (base_z + z as i32) as f32;
                let u1 = w as f32;
                let v1 = h as f32;
                let mb = builds.entry(codev.0).or_default();
                let lv = codev.1.max(VISUAL_LIGHT_MIN);
                let rgba = [lv, lv, lv, 255];
                if !pos {
                    mb.add_quad(
                        Vector3::new(fx, fy + v1, fz),
                        Vector3::new(fx, fy + v1, fz + u1),
                        Vector3::new(fx, fy, fz + u1),
                        Vector3::new(fx, fy, fz),
                        Vector3::new(-1.0, 0.0, 0.0),
                        u1,
                        v1,
                        false,
                        rgba,
                    );
                } else {
                    mb.add_quad(
                        Vector3::new(fx, fy + v1, fz + u1),
                        Vector3::new(fx, fy + v1, fz),
                        Vector3::new(fx, fy, fz),
                        Vector3::new(fx, fy, fz + u1),
                        Vector3::new(1.0, 0.0, 0.0),
                        u1,
                        v1,
                        false,
                        rgba,
                    );
                }
            });
        }
    }

    // Z planes
    for z in 0..sz {
        for &pos in &[false, true] {
            let mut mask: Vec<Option<(FaceMaterial, u8)>> = vec![None; sx * sy];
            for x in 0..sx {
                for y in 0..sy {
                    let here = buf.get_local(x, y, z);
                    if here.is_solid() {
                        let gx = base_x + x as i32;
                        let gz = base_z + z as i32;
                        let gy = y as i32;
                        let neigh = if pos {
                            is_occluder(buf, world, edits, neighbors, here, gx, gy, gz + 1)
                        } else {
                            is_occluder(buf, world, edits, neighbors, here, gx, gy, gz - 1)
                        };
                        if !neigh {
                            if let Some(fm) = face_material_for(here, if pos { 4 } else { 5 }) {
                                let l = light.sample_face_local(x, y, z, if pos { 4 } else { 5 });
                                mask[y * sx + x] = Some((fm, l));
                            }
                        }
                    }
                }
            }
            greedy_rects(sx, sy, &mut mask, |x, y, w, h, codev| {
                let fx = (base_x + x as i32) as f32;
                let fy = y as f32;
                let fz = (base_z + z as i32) as f32 + if pos { 1.0 } else { 0.0 };
                let u1 = w as f32;
                let v1 = h as f32;
                let mb = builds.entry(codev.0).or_default();
                let lv = codev.1.max(VISUAL_LIGHT_MIN);
                let rgba = [lv, lv, lv, 255];
                if !pos {
                    mb.add_quad(
                        Vector3::new(fx, fy + v1, fz),
                        Vector3::new(fx + u1, fy + v1, fz),
                        Vector3::new(fx + u1, fy, fz),
                        Vector3::new(fx, fy, fz),
                        Vector3::new(0.0, 0.0, -1.0),
                        u1,
                        v1,
                        false,
                        rgba,
                    );
                } else {
                    mb.add_quad(
                        Vector3::new(fx + u1, fy + v1, fz),
                        Vector3::new(fx, fy + v1, fz),
                        Vector3::new(fx, fy, fz),
                        Vector3::new(fx + u1, fy, fz),
                        Vector3::new(0.0, 0.0, 1.0),
                        u1,
                        v1,
                        false,
                        rgba,
                    );
                }
            });
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
    // Instead of mutating the store directly, return the light borders
    let light_borders = if lighting.is_some() {
        Some(LightBorders::from_grid(&light))
    } else {
        None
    };
    Some((
        ChunkMeshCPU {
            cx,
            cz,
            bbox,
            parts: builds,
        },
        light_borders,
    ))
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

    use std::collections::HashMap;
    let mut builds: HashMap<FaceMaterial, MeshBuild> = HashMap::new();

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

    for y in 0..sy {
        // +Y faces
        {
            let mut mask: Vec<Option<(FaceMaterial, u8)>> = vec![None; sx * sz];
            for z in 0..sz {
                for x in 0..sx {
                    let here = buf.get_local(x, y, z);
                    if here.is_solid() {
                        let neigh = solid_local(buf, x as i32, y as i32 + 1, z as i32);
                        if !neigh {
                            if let Some(fm) = face_material_for(here, 0) {
                                let l = face_light(0, ambient);
                                mask[z * sx + x] = Some((fm, l));
                            }
                        }
                    }
                }
            }
            let mut used = vec![false; sx * sz];
            for z in 0..sz {
                for x in 0..sx {
                    let code = mask[z * sx + x];
                    if code.is_none() || used[z * sx + x] {
                        continue;
                    }
                    let codev = code.unwrap();
                    let mut w = 1;
                    while x + w < sx && mask[z * sx + x + w] == code && !used[z * sx + x + w] {
                        w += 1;
                    }
                    let mut h = 1;
                    'expand: while z + h < sz {
                        for i in 0..w {
                            if mask[(z + h) * sx + (x + i)] != code || used[(z + h) * sx + (x + i)]
                            {
                                break 'expand;
                            }
                        }
                        h += 1;
                    }
                    let fx = x as f32;
                    let fz = z as f32;
                    let fy = (y as f32) + 1.0;
                    let u1 = w as f32;
                    let v1 = h as f32;
                    let mb = builds.entry(codev.0).or_default();
                    let lv = codev.1;
                    let rgba = [lv, lv, lv, 255];
                    mb.add_quad(
                        Vector3::new(fx, fy, fz),
                        Vector3::new(fx + u1, fy, fz),
                        Vector3::new(fx + u1, fy, fz + v1),
                        Vector3::new(fx, fy, fz + v1),
                        Vector3::new(0.0, 1.0, 0.0),
                        u1,
                        v1,
                        false,
                        rgba,
                    );
                    for zz in 0..h {
                        for xx in 0..w {
                            used[(z + zz) * sx + (x + xx)] = true;
                        }
                    }
                }
            }
        }
        // -Y faces
        {
            let mut mask: Vec<Option<(FaceMaterial, u8)>> = vec![None; sx * sz];
            for z in 0..sz {
                for x in 0..sx {
                    let here = buf.get_local(x, y, z);
                    if here.is_solid() {
                        let neigh = solid_local(buf, x as i32, y as i32 - 1, z as i32);
                        if !neigh {
                            if let Some(fm) = face_material_for(here, 1) {
                                let l = face_light(1, ambient);
                                mask[z * sx + x] = Some((fm, l));
                            }
                        }
                    }
                }
            }
            let mut used = vec![false; sx * sz];
            for z in 0..sz {
                for x in 0..sx {
                    let code = mask[z * sx + x];
                    if code.is_none() || used[z * sx + x] {
                        continue;
                    }
                    let codev = code.unwrap();
                    let mut w = 1;
                    while x + w < sx && mask[z * sx + x + w] == code && !used[z * sx + x + w] {
                        w += 1;
                    }
                    let mut h = 1;
                    'expand: while z + h < sz {
                        for i in 0..w {
                            if mask[(z + h) * sx + (x + i)] != code || used[(z + h) * sx + (x + i)]
                            {
                                break 'expand;
                            }
                        }
                        h += 1;
                    }
                    let fx = x as f32;
                    let fz = z as f32;
                    let fy = y as f32;
                    let u1 = w as f32;
                    let v1 = h as f32;
                    let mb = builds.entry(codev.0).or_default();
                    let lv = codev.1;
                    let rgba = [lv, lv, lv, 255];
                    mb.add_quad(
                        Vector3::new(fx, fy, fz),
                        Vector3::new(fx, fy, fz + v1),
                        Vector3::new(fx + u1, fy, fz + v1),
                        Vector3::new(fx + u1, fy, fz),
                        Vector3::new(0.0, -1.0, 0.0),
                        u1,
                        v1,
                        true,
                        rgba,
                    );
                    for zz in 0..h {
                        for xx in 0..w {
                            used[(z + zz) * sx + (x + xx)] = true;
                        }
                    }
                }
            }
        }
    }

    // +X faces
    for x in 0..sx {
        let mut mask: Vec<Option<(FaceMaterial, u8)>> = vec![None; sy * sz];
        for z in 0..sz {
            for y in 0..sy {
                let here = buf.get_local(x, y, z);
                if here.is_solid() {
                    let neigh = solid_local(buf, x as i32 + 1, y as i32, z as i32);
                    if !neigh {
                        if let Some(fm) = face_material_for(here, 2) {
                            let l = face_light(2, ambient);
                            mask[y * sz + z] = Some((fm, l));
                        }
                    }
                }
            }
        }
        greedy_rects(sz, sy, &mut mask, |z, y, w, h, codev| {
            let fx = (x as f32) + 1.0;
            let fy = y as f32;
            let fz = z as f32;
            let u1 = w as f32;
            let v1 = h as f32;
            let mb = builds.entry(codev.0).or_default();
            let lv = codev.1;
            let rgba = [lv, lv, lv, 255];
            mb.add_quad(
                Vector3::new(fx, fy, fz),
                Vector3::new(fx, fy + v1, fz),
                Vector3::new(fx, fy + v1, fz + u1),
                Vector3::new(fx, fy, fz + u1),
                Vector3::new(1.0, 0.0, 0.0),
                u1,
                v1,
                false,
                rgba,
            );
        });
    }

    // -X faces
    for x in 0..sx {
        let mut mask: Vec<Option<(FaceMaterial, u8)>> = vec![None; sy * sz];
        for z in 0..sz {
            for y in 0..sy {
                let here = buf.get_local(x, y, z);
                if here.is_solid() {
                    let neigh = solid_local(buf, x as i32 - 1, y as i32, z as i32);
                    if !neigh {
                        if let Some(fm) = face_material_for(here, 3) {
                            let l = face_light(3, ambient);
                            mask[y * sz + z] = Some((fm, l));
                        }
                    }
                }
            }
        }
        greedy_rects(sz, sy, &mut mask, |z, y, w, h, codev| {
            let fx = x as f32;
            let fy = y as f32;
            let fz = z as f32;
            let u1 = w as f32;
            let v1 = h as f32;
            let mb = builds.entry(codev.0).or_default();
            let lv = codev.1;
            let rgba = [lv, lv, lv, 255];
            mb.add_quad(
                Vector3::new(fx, fy, fz),
                Vector3::new(fx, fy, fz + u1),
                Vector3::new(fx, fy + v1, fz + u1),
                Vector3::new(fx, fy + v1, fz),
                Vector3::new(-1.0, 0.0, 0.0),
                u1,
                v1,
                true,
                rgba,
            );
        });
    }

    // +Z faces
    for z in 0..sz {
        let mut mask: Vec<Option<(FaceMaterial, u8)>> = vec![None; sy * sx];
        for x in 0..sx {
            for y in 0..sy {
                let here = buf.get_local(x, y, z);
                if here.is_solid() {
                    let neigh = solid_local(buf, x as i32, y as i32, z as i32 + 1);
                    if !neigh {
                        if let Some(fm) = face_material_for(here, 4) {
                            let l = face_light(4, ambient);
                            mask[y * sx + x] = Some((fm, l));
                        }
                    }
                }
            }
        }
        greedy_rects(sx, sy, &mut mask, |x, y, w, h, codev| {
            let fx = x as f32;
            let fy = y as f32;
            let fz = (z as f32) + 1.0;
            let u1 = w as f32;
            let v1 = h as f32;
            let mb = builds.entry(codev.0).or_default();
            let lv = codev.1;
            let rgba = [lv, lv, lv, 255];
            mb.add_quad(
                Vector3::new(fx, fy, fz),
                Vector3::new(fx + u1, fy, fz),
                Vector3::new(fx + u1, fy + v1, fz),
                Vector3::new(fx, fy + v1, fz),
                Vector3::new(0.0, 0.0, 1.0),
                u1,
                v1,
                false,
                rgba,
            );
        });
    }

    // -Z faces
    for z in 0..sz {
        let mut mask: Vec<Option<(FaceMaterial, u8)>> = vec![None; sy * sx];
        for x in 0..sx {
            for y in 0..sy {
                let here = buf.get_local(x, y, z);
                if here.is_solid() {
                    let neigh = solid_local(buf, x as i32, y as i32, z as i32 - 1);
                    if !neigh {
                        if let Some(fm) = face_material_for(here, 5) {
                            let l = face_light(5, ambient);
                            mask[y * sx + x] = Some((fm, l));
                        }
                    }
                }
            }
        }
        greedy_rects(sx, sy, &mut mask, |x, y, w, h, codev| {
            let fx = x as f32;
            let fy = y as f32;
            let fz = z as f32;
            let u1 = w as f32;
            let v1 = h as f32;
            let mb = builds.entry(codev.0).or_default();
            let lv = codev.1;
            let rgba = [lv, lv, lv, 255];
            mb.add_quad(
                Vector3::new(fx, fy, fz),
                Vector3::new(fx, fy + v1, fz),
                Vector3::new(fx + u1, fy + v1, fz),
                Vector3::new(fx + u1, fy, fz),
                Vector3::new(0.0, 0.0, -1.0),
                u1,
                v1,
                true,
                rgba,
            );
        });
    }

    let bbox = BoundingBox::new(
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(sx as f32, sy as f32, sz as f32),
    );
    ChunkMeshCPU {
        cx: 0,
        cz: 0,
        bbox,
        parts: builds,
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

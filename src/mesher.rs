use crate::chunkbuf::ChunkBuf;
use crate::lighting::{LightBorders, LightGrid, LightingStore};
use crate::voxel::World;
use raylib::core::math::BoundingBox;
use raylib::prelude::*;
use std::collections::HashMap as StdHashMap;
use std::collections::HashMap;
use crate::blocks::{BlockRegistry, FaceRole, MaterialCatalog, MaterialId, Block};

// Visual-only lighting floor to avoid pitch-black faces in darkness.
// Does not affect logical light propagation.
const VISUAL_LIGHT_MIN: u8 = 18; // ~7% brightness floor


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

#[inline]
fn unknown_material_id(reg: &BlockRegistry) -> MaterialId {
    reg.materials.get_id("unknown").unwrap_or(MaterialId(0))
}

#[inline]
fn registry_material_for(block: Block, face: usize, reg: &BlockRegistry) -> Option<MaterialId> {
    let ty = reg.get(block.id)?;
    let role = match face {
        0 => FaceRole::Top,
        1 => FaceRole::Bottom,
        _ => FaceRole::Side,
    };
    ty.materials.material_for(role, block.state, ty)
}

#[inline]
fn registry_material_for_or_unknown(block: Block, face: usize, reg: &BlockRegistry) -> MaterialId {
    registry_material_for(block, face, reg).unwrap_or_else(|| unknown_material_id(reg))
}

#[inline]
// Legacy helpers removed; materials are resolved via registry CompiledMaterials

#[inline]
fn is_solid_runtime(b: Block, reg: &BlockRegistry) -> bool {
    reg.get(b.id).map(|ty| ty.is_solid(b.state)).unwrap_or(false)
}

// Property decoding now lives on BlockType via registry (see state_prop_value/state_prop_is_value)

#[inline]
fn is_top_half_shape(b: Block, reg: &BlockRegistry) -> bool {
    if let Some(ty) = reg.get(b.id) {
        match &ty.shape {
            crate::blocks::Shape::Slab { half_from } | crate::blocks::Shape::Stairs { half_from, .. } => {
                return ty.state_prop_is_value(b.state, half_from, "top");
            }
            _ => {}
        }
    }
    false
}

// Legacy world mapping removed; mesher queries runtime worldgen directly when needed.

#[inline]
fn emit_box_faces(
    builds: &mut std::collections::HashMap<MaterialId, MeshBuild>,
    min: Vector3,
    max: Vector3,
    mut choose: impl FnMut(usize) -> Option<(MaterialId, [u8; 4])>,
)
{
    // 0 = +Y
    if let Some((mid, rgba)) = choose(0) {
        let mb = builds.entry(mid).or_default();
        mb.add_quad(
            Vector3::new(min.x, max.y, min.z),
            Vector3::new(max.x, max.y, min.z),
            Vector3::new(max.x, max.y, max.z),
            Vector3::new(min.x, max.y, max.z),
            Vector3::new(0.0, 1.0, 0.0),
            max.x - min.x,
            max.z - min.z,
            false,
            rgba,
        );
    }
    // 1 = -Y
    if let Some((mid, rgba)) = choose(1) {
        let mb = builds.entry(mid).or_default();
        mb.add_quad(
            Vector3::new(min.x, min.y, max.z),
            Vector3::new(max.x, min.y, max.z),
            Vector3::new(max.x, min.y, min.z),
            Vector3::new(min.x, min.y, min.z),
            Vector3::new(0.0, -1.0, 0.0),
            max.x - min.x,
            max.z - min.z,
            false,
            rgba,
        );
    }
    // 2 = +X
    if let Some((mid, rgba)) = choose(2) {
        let mb = builds.entry(mid).or_default();
        mb.add_quad(
            Vector3::new(max.x, max.y, max.z),
            Vector3::new(max.x, max.y, min.z),
            Vector3::new(max.x, min.y, min.z),
            Vector3::new(max.x, min.y, max.z),
            Vector3::new(1.0, 0.0, 0.0),
            max.z - min.z,
            max.y - min.y,
            false,
            rgba,
        );
    }
    // 3 = -X
    if let Some((mid, rgba)) = choose(3) {
        let mb = builds.entry(mid).or_default();
        mb.add_quad(
            Vector3::new(min.x, max.y, min.z),
            Vector3::new(min.x, max.y, max.z),
            Vector3::new(min.x, min.y, max.z),
            Vector3::new(min.x, min.y, min.z),
            Vector3::new(-1.0, 0.0, 0.0),
            max.z - min.z,
            max.y - min.y,
            false,
            rgba,
        );
    }
    // 4 = +Z
    if let Some((mid, rgba)) = choose(4) {
        let mb = builds.entry(mid).or_default();
        mb.add_quad(
            Vector3::new(min.x, max.y, max.z),
            Vector3::new(max.x, max.y, max.z),
            Vector3::new(max.x, min.y, max.z),
            Vector3::new(min.x, min.y, max.z),
            Vector3::new(0.0, 0.0, 1.0),
            max.x - min.x,
            max.y - min.y,
            false,
            rgba,
        );
    }
    // 5 = -Z
    if let Some((mid, rgba)) = choose(5) {
        let mb = builds.entry(mid).or_default();
        mb.add_quad(
            Vector3::new(max.x, max.y, min.z),
            Vector3::new(min.x, max.y, min.z),
            Vector3::new(min.x, min.y, min.z),
            Vector3::new(max.x, min.y, min.z),
            Vector3::new(0.0, 0.0, -1.0),
            max.x - min.x,
            max.y - min.y,
            false,
            rgba,
        );
    }
}

#[inline]
fn emit_box(
    builds: &mut std::collections::HashMap<MaterialId, MeshBuild>,
    buf: &ChunkBuf,
    world: &World,
    edits: Option<&StdHashMap<(i32, i32, i32), Block>>,
    neighbors: NeighborsLoaded,
    reg: &BlockRegistry,
    light: &LightGrid,
    x: usize,
    y: usize,
    z: usize,
    base_x: i32,
    base_z: i32,
    fm_for_face: &dyn Fn(usize) -> MaterialId,
    min: Vector3,
    max: Vector3,
) {
    let gx = base_x + x as i32;
    let gy = y as i32;
    let gz = base_z + z as i32;
    let here = buf.get_local(x, y, z);
    emit_box_faces(builds, min, max, |face| {
        let (nx, ny, nz) = match face {
            0 => (gx, gy + 1, gz),
            1 => (gx, gy - 1, gz),
            2 => (gx + 1, gy, gz),
            3 => (gx - 1, gy, gz),
            4 => (gx, gy, gz + 1),
            5 => (gx, gy, gz - 1),
            _ => unreachable!(),
        };
        if is_occluder(buf, world, edits, neighbors, reg, here, face, nx, ny, nz) {
            return None;
        }
        let mut lv = light.sample_face_local(x, y, z, face);
        lv = lv.max(VISUAL_LIGHT_MIN);
        let rgba = [lv, lv, lv, 255];
        let mid = fm_for_face(face);
        Some((mid, rgba))
    });
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
    reg: &BlockRegistry,
    here: Block,
    face: usize,
    nx: i32,
    ny: i32,
    nz: i32,
) -> bool {
    if !is_solid_runtime(here, reg) {
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
        return occludes_face(nb, face, reg);
    }
    // Outside current chunk: only occlude if the corresponding neighbor chunk is loaded.
    // Without cross-chunk access in worker, do not cull across seam to avoid holes.
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
            .unwrap_or_else(|| world.block_at_runtime(reg, nx, ny, nz))
    } else {
        world.block_at_runtime(reg, nx, ny, nz)
    };
    occludes_face(nb, face, reg)
}

#[inline]
fn occludes_face(nb: Block, face: usize, reg: &BlockRegistry) -> bool {
    // Slab/stairs: occlusion based on half
    if let Some(ty) = reg.get(nb.id) {
        match &ty.shape {
            crate::blocks::Shape::Slab { half_from } => {
                let is_top = ty.state_prop_is_value(nb.state, half_from, "top");
                return match face {
                    0 => !is_top, // above occluded by bottom slab
                    1 => is_top,  // below occluded by top slab
                    _ => true,
                };
            }
            crate::blocks::Shape::Stairs { half_from, .. } => {
                let is_top = ty.state_prop_is_value(nb.state, half_from, "top");
                return match face {
                    0 => !is_top,
                    1 => is_top,
                    _ => true,
                };
            }
            _ => {}
        }
        return ty.is_solid(nb.state);
    }
    false
}

// No legacy mapping helpers; all block resolution is via registry-backed runtime Block.

pub struct ChunkRender {
    pub cx: i32,
    pub cz: i32,
    pub bbox: BoundingBox,
    pub parts: Vec<(MaterialId, raylib::core::models::Model)>,
    pub leaf_tint: Option<[f32; 3]>,
}

pub struct ChunkMeshCPU {
    pub cx: i32,
    pub cz: i32,
    pub bbox: BoundingBox,
    pub parts: std::collections::HashMap<MaterialId, MeshBuild>,
}

pub fn build_chunk_greedy_cpu_buf(
    buf: &ChunkBuf,
    lighting: Option<&LightingStore>,
    world: &World,
    edits: Option<&StdHashMap<(i32, i32, i32), Block>>,
    neighbors: NeighborsLoaded,
    cx: i32,
    cz: i32,
    reg: &BlockRegistry,
) -> Option<(ChunkMeshCPU, Option<LightBorders>)> {
    let sx = buf.sx;
    let sy = buf.sy;
    let sz = buf.sz;
    let base_x = buf.cx * sx as i32;
    let base_z = buf.cz * sz as i32;

    // Unified path via meshing_core
    let light = match lighting {
        Some(store) => LightGrid::compute_with_borders_buf(buf, store, reg),
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
            if !is_solid_runtime(here, reg) {
                return None;
            }
            // Skip non-cubic special shapes here; they are handled in a dedicated pass below.
            if let Some(ty) = reg.get(here.id) {
                match ty.shape {
                    crate::blocks::Shape::Slab { .. } | crate::blocks::Shape::Stairs { .. } => {
                        return None;
                    }
                    _ => {}
                }
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
            if is_occluder(buf, world, edits, neighbors, reg, here, face, nx, ny, nz) {
                return None;
            }
            // Resolve material via registry; fallback to unknown when unmapped
            let mut l = light.sample_face_local(x, y, z, face);
            if face == 0 {
                if buf.contains_world(nx, ny, nz) && ny >= 0 && (ny as usize) < sy {
                    let lx = (nx - base_x) as usize;
                    let ly = ny as usize;
                    let lz = (nz - base_z) as usize;
            let nb = buf.get_local(lx, ly, lz);
            if is_top_half_shape(nb, reg) {
                    let l2 = light
                        .sample_face_local(x, y, z, 2)
                        .max(light.sample_face_local(x, y, z, 3))
                        .max(light.sample_face_local(x, y, z, 4))
                        .max(light.sample_face_local(x, y, z, 5));
                    l = l.max(l2);
            }
                }
            }
            let mid = registry_material_for_or_unknown(here, face, reg);
            Some((mid, l))
        },
    );
    // Special-shapes pass: mesh slabs (runtime registry-driven)
    for z in 0..sz {
        for y in 0..sy {
            for x in 0..sx {
                let b = buf.get_local(x, y, z);
                if let Some(ty) = reg.get(b.id) {
                    match &ty.shape {
                        crate::blocks::Shape::Slab { half_from } => {
                        let fx = base_x as f32 + x as f32;
                        let fy = y as f32;
                        let fz = base_z as f32 + z as f32;
                        let is_top = ty.state_prop_is_value(b.state, half_from, "top");
                        let (y0, y1) = if is_top { (fy + 0.5, fy + 1.0) } else { (fy, fy + 0.5) };
                        let min = Vector3::new(fx, y0, fz);
                        let max = Vector3::new(fx + 1.0, y1, fz + 1.0);
                        emit_box(
                            &mut builds,
                            buf,
                            world,
                            edits,
                            neighbors,
                            reg,
                            &light,
                            x,
                            y,
                            z,
                            base_x,
                            base_z,
                            &|face| {
                                let role = match face {
                                    0 => FaceRole::Top,
                                    1 => FaceRole::Bottom,
                                    _ => FaceRole::Side,
                                };
                                ty.materials
                                    .material_for(role, b.state, ty)
                                    .unwrap_or_else(|| unknown_material_id(reg))
                            },
                            min,
                            max,
                        );

                        // Restore partial neighbor faces that greedy culled fully
                        // Visible portion is opposite half along Y
                        let (vis_y0, vis_y1) = if is_top { (fy, fy + 0.5) } else { (fy + 0.5, fy + 1.0) };
                        // Helper to decide if neighbor is a full cube (not special)
                        let is_full_cube = |nb: Block| -> bool {
                            reg.get(nb.id)
                                .map(|t| matches!(t.shape, crate::blocks::Shape::Cube | crate::blocks::Shape::AxisCube { .. }))
                                .unwrap_or(false)
                        };
                        // West neighbor (+X face on neighbor)
                        if x > 0 {
                            let nb = buf.get_local(x - 1, y, z);
                            if is_full_cube(nb) {
                                let l0 = light.sample_face_local(x - 1, y, z, 2);
                                let lv = if !is_top {
                                    let la = if y + 1 < sy { light.sample_face_local(x - 1, y + 1, z, 2) } else { l0 };
                                    l0.max(la).max(VISUAL_LIGHT_MIN)
                                } else {
                                    let lb = if y > 0 { light.sample_face_local(x - 1, y - 1, z, 2) } else { l0 };
                                    l0.max(lb).max(VISUAL_LIGHT_MIN)
                                };
                                let rgba = [lv, lv, lv, 255];
                                let mid = registry_material_for_or_unknown(nb, 2, reg);
                                let mb = builds.entry(mid).or_default();
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
                        // East neighbor (-X face on neighbor)
                        if x + 1 < sx {
                            let nb = buf.get_local(x + 1, y, z);
                            if is_full_cube(nb) {
                                let l0 = light.sample_face_local(x + 1, y, z, 3);
                                let lv = if !is_top {
                                    let la = if y + 1 < sy { light.sample_face_local(x + 1, y + 1, z, 3) } else { l0 };
                                    l0.max(la).max(VISUAL_LIGHT_MIN)
                                } else {
                                    let lb = if y > 0 { light.sample_face_local(x + 1, y - 1, z, 3) } else { l0 };
                                    l0.max(lb).max(VISUAL_LIGHT_MIN)
                                };
                                let rgba = [lv, lv, lv, 255];
                                let mid = registry_material_for_or_unknown(nb, 3, reg);
                                let mb = builds.entry(mid).or_default();
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
                        // North neighbor (+Z face on neighbor)
                        if z > 0 {
                            let nb = buf.get_local(x, y, z - 1);
                            if is_full_cube(nb) {
                                let l0 = light.sample_face_local(x, y, z - 1, 4);
                                let lv = if !is_top {
                                    let la = if y + 1 < sy { light.sample_face_local(x, y + 1, z - 1, 4) } else { l0 };
                                    l0.max(la).max(VISUAL_LIGHT_MIN)
                                } else {
                                    let lb = if y > 0 { light.sample_face_local(x, y - 1, z - 1, 4) } else { l0 };
                                    l0.max(lb).max(VISUAL_LIGHT_MIN)
                                };
                                let rgba = [lv, lv, lv, 255];
                                let mid = registry_material_for_or_unknown(nb, 4, reg);
                                let mb = builds.entry(mid).or_default();
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
                        // South neighbor (-Z face on neighbor)
                        if z + 1 < sz {
                            let nb = buf.get_local(x, y, z + 1);
                            if is_full_cube(nb) {
                                let l0 = light.sample_face_local(x, y, z + 1, 5);
                                let lv = if !is_top {
                                    let la = if y + 1 < sy { light.sample_face_local(x, y + 1, z + 1, 5) } else { l0 };
                                    l0.max(la).max(VISUAL_LIGHT_MIN)
                                } else {
                                    let lb = if y > 0 { light.sample_face_local(x, y - 1, z + 1, 5) } else { l0 };
                                    l0.max(lb).max(VISUAL_LIGHT_MIN)
                                };
                                let rgba = [lv, lv, lv, 255];
                                let mid = registry_material_for_or_unknown(nb, 5, reg);
                                let mb = builds.entry(mid).or_default();
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
                        crate::blocks::Shape::Stairs { facing_from, half_from } => {
                            let fx = base_x as f32 + x as f32;
                            let fy = y as f32;
                            let fz = base_z as f32 + z as f32;
                            let is_top = ty.state_prop_is_value(b.state, half_from, "top");
                            let facing = ty.state_prop_value(b.state, facing_from).unwrap_or("north");
                            // Big half-height slab depending on half
                            let (min_a, max_a) = if is_top {
                                (Vector3::new(fx, fy + 0.5, fz), Vector3::new(fx + 1.0, fy + 1.0, fz + 1.0))
                            } else {
                                (Vector3::new(fx, fy, fz), Vector3::new(fx + 1.0, fy + 0.5, fz + 1.0))
                            };
                            let face_material = |face: usize| {
                                let role = match face {
                                    0 => FaceRole::Top,
                                    1 => FaceRole::Bottom,
                                    _ => FaceRole::Side,
                                };
                                ty.materials
                                    .material_for(role, b.state, ty)
                                    .unwrap_or_else(|| unknown_material_id(reg))
                            };
                            emit_box(
                                &mut builds,
                                buf,
                                world,
                                edits,
                                neighbors,
                                reg,
                                &light,
                                x,
                                y,
                                z,
                                base_x,
                                base_z,
                                &face_material,
                                min_a,
                                max_a,
                            );
                            // Secondary half-depth/width slab toward facing
                            let (min_b, max_b) = match (facing, is_top) {
                                ("north", false) => (Vector3::new(fx, fy + 0.5, fz), Vector3::new(fx + 1.0, fy + 1.0, fz + 0.5)),
                                ("south", false) => (Vector3::new(fx, fy + 0.5, fz + 0.5), Vector3::new(fx + 1.0, fy + 1.0, fz + 1.0)),
                                ("west", false) => (Vector3::new(fx, fy + 0.5, fz), Vector3::new(fx + 0.5, fy + 1.0, fz + 1.0)),
                                ("east", false) => (Vector3::new(fx + 0.5, fy + 0.5, fz), Vector3::new(fx + 1.0, fy + 1.0, fz + 1.0)),
                                ("north", true) => (Vector3::new(fx, fy, fz), Vector3::new(fx + 1.0, fy + 0.5, fz + 0.5)),
                                ("south", true) => (Vector3::new(fx, fy, fz + 0.5), Vector3::new(fx + 1.0, fy + 0.5, fz + 1.0)),
                                ("west", true) => (Vector3::new(fx, fy, fz), Vector3::new(fx + 0.5, fy + 0.5, fz + 1.0)),
                                ("east", true) => (Vector3::new(fx + 0.5, fy, fz), Vector3::new(fx + 1.0, fy + 0.5, fz + 1.0)),
                                _ => (Vector3::new(fx, fy, fz), Vector3::new(fx, fy, fz)),
                            };
                            if max_b.x > min_b.x || max_b.y > min_b.y || max_b.z > min_b.z {
                                emit_box(
                                    &mut builds,
                                    buf,
                                    world,
                                    edits,
                                    neighbors,
                                    reg,
                                    &light,
                                    x,
                                    y,
                                    z,
                                    base_x,
                                    base_z,
                                    &face_material,
                                    min_b,
                                    max_b,
                                );
                            }

                            // Neighbor face restoration for full-cube neighbors occluded by stair shape
                            let draw_top = !is_top; // visible portion of neighbor face is opposite half
                            let y0 = if draw_top { fy + 0.5 } else { fy };
                            let y1 = if draw_top { fy + 1.0 } else { fy + 0.5 };
                            #[inline]
                            fn sample_lv(light: &LightGrid, x: usize, y: usize, z: usize, face: usize, draw_top_half: bool, sy: usize) -> u8 {
                                let l0 = light.sample_face_local(x, y, z, face);
                                let ladd = if draw_top_half {
                                    if y + 1 < sy { light.sample_face_local(x, y + 1, z, face) } else { l0 }
                                } else {
                                    if y > 0 { light.sample_face_local(x, y - 1, z, face) } else { l0 }
                                };
                                l0.max(ladd).max(VISUAL_LIGHT_MIN)
                            }
                            let is_full_cube = |nb: Block| -> bool {
                                reg.get(nb.id)
                                    .map(|t| matches!(t.shape, crate::blocks::Shape::Cube | crate::blocks::Shape::AxisCube { .. }))
                                    .unwrap_or(false)
                            };
                            // West neighbor (+X face on neighbor)
                            if x > 0 {
                                let nb = buf.get_local(x - 1, y, z);
                                if is_full_cube(nb) {
                                    let rgba = {
                                        let lv = sample_lv(&light, x - 1, y, z, 2, draw_top, sy);
                                        [lv, lv, lv, 255]
                                    };
                                    let mid = registry_material_for_or_unknown(nb, 2, reg);
                                    let mb = builds.entry(mid).or_default();
                                    let px = fx;
                                    let segs: &[(f32, f32)] = match (facing, is_top) {
                                        ("north", false) => &[(fz + 0.5, fz + 1.0)],
                                        ("south", false) => &[(fz, fz + 0.5)],
                                        ("west", false) => &[],
                                        ("east", false) => &[(fz, fz + 1.0)],
                                        ("north", true) => &[(fz + 0.5, fz + 1.0)],
                                        ("south", true) => &[(fz, fz + 0.5)],
                                        ("west", true) => &[],
                                        ("east", true) => &[(fz, fz + 1.0)],
                                        _ => &[],
                                    };
                                    for &(z0, z1) in segs.iter() {
                                        if z1 <= z0 { continue; }
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
                            // East neighbor (-X face on neighbor)
                            if x + 1 < sx {
                                let nb = buf.get_local(x + 1, y, z);
                                if is_full_cube(nb) {
                                    let rgba = {
                                        let lv = sample_lv(&light, x + 1, y, z, 3, draw_top, sy);
                                        [lv, lv, lv, 255]
                                    };
                                    let mid = registry_material_for_or_unknown(nb, 3, reg);
                                    let mb = builds.entry(mid).or_default();
                                    let px = fx + 1.0;
                                    let segs: &[(f32, f32)] = match (facing, is_top) {
                                        ("north", false) => &[(fz + 0.5, fz + 1.0)],
                                        ("south", false) => &[(fz, fz + 0.5)],
                                        ("west", false) => &[(fz, fz + 1.0)],
                                        ("east", false) => &[],
                                        ("north", true) => &[(fz + 0.5, fz + 1.0)],
                                        ("south", true) => &[(fz, fz + 0.5)],
                                        ("west", true) => &[(fz, fz + 1.0)],
                                        ("east", true) => &[],
                                        _ => &[],
                                    };
                                    for &(z0, z1) in segs.iter() {
                                        if z1 <= z0 { continue; }
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
                            // North neighbor (+Z face on neighbor)
                            if z > 0 {
                                let nb = buf.get_local(x, y, z - 1);
                                if is_full_cube(nb) {
                                    let rgba = {
                                        let lv = sample_lv(&light, x, y, z - 1, 4, draw_top, sy);
                                        [lv, lv, lv, 255]
                                    };
                                    let mid = registry_material_for_or_unknown(nb, 4, reg);
                                    let mb = builds.entry(mid).or_default();
                                    let pz = fz;
                                    let segs: &[(f32, f32)] = match (facing, is_top) {
                                        ("east", false) => &[(fx + 0.5, fx + 1.0)],
                                        ("west", false) => &[(fx, fx + 0.5)],
                                        ("north", false) => &[(fx, fx + 1.0)],
                                        ("south", false) => &[],
                                        ("east", true) => &[(fx + 0.5, fx + 1.0)],
                                        ("west", true) => &[(fx, fx + 0.5)],
                                        ("north", true) => &[(fx, fx + 1.0)],
                                        ("south", true) => &[],
                                        _ => &[],
                                    };
                                    for &(x0f, x1f) in segs.iter() {
                                        if x1f <= x0f { continue; }
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
                            // South neighbor (-Z face on neighbor)
                            if z + 1 < sz {
                                let nb = buf.get_local(x, y, z + 1);
                                if is_full_cube(nb) {
                                    let rgba = {
                                        let lv = sample_lv(&light, x, y, z + 1, 5, draw_top, sy);
                                        [lv, lv, lv, 255]
                                    };
                                    let mid = registry_material_for_or_unknown(nb, 5, reg);
                                    let mb = builds.entry(mid).or_default();
                                    let pz = fz + 1.0;
                                    let segs: &[(f32, f32)] = match (facing, is_top) {
                                        ("east", false) => &[(fx + 0.5, fx + 1.0)],
                                        ("west", false) => &[(fx, fx + 0.5)],
                                        ("north", false) => &[],
                                        ("south", false) => &[(fx, fx + 1.0)],
                                        ("east", true) => &[(fx + 0.5, fx + 1.0)],
                                        ("west", true) => &[(fx, fx + 0.5)],
                                        ("north", true) => &[],
                                        ("south", true) => &[(fx, fx + 1.0)],
                                        _ => &[],
                                    };
                                    for &(x0f, x1f) in segs.iter() {
                                        if x1f <= x0f { continue; }
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
    mats: &MaterialCatalog,
) -> Option<ChunkRender> {
    let mut parts_gpu = Vec::new();
    for (mid, mb) in cpu.parts.into_iter() {
        let total_verts = mb.pos.len() / 3;
        if total_verts == 0 { continue; }
        // Split into safe submeshes to avoid 16-bit index overflow (raylib uses u16 indices)
        let max_verts: usize = 65000; // keep under u16::MAX and divisible by quads nicely
        let total_quads = total_verts / 4; // each quad adds 4 verts
        let max_quads = max_verts / 4;
        let mut q = 0usize;
        while q < total_quads {
            let take_q = (total_quads - q).min(max_quads);
            let v_start = q * 4;
            let v_count = take_q * 4;
            // Prepare raw mesh
            let mut raw: raylib::ffi::Mesh = unsafe { std::mem::zeroed() };
            raw.vertexCount = v_count as i32;
            raw.triangleCount = (take_q * 2) as i32;
            unsafe {
                let pos_start = v_start * 3;
                let pos_end = pos_start + v_count * 3;
                let norm_start = v_start * 3;
                let norm_end = norm_start + v_count * 3;
                let uv_start = v_start * 2;
                let uv_end = uv_start + v_count * 2;
                let col_start = v_start * 4;
                let col_end = col_start + v_count * 4;

                // Allocate buffers
                let vbytes = (v_count * 3 * std::mem::size_of::<f32>()) as u32;
                let nbytes = (v_count * 3 * std::mem::size_of::<f32>()) as u32;
                let tbytes = (v_count * 2 * std::mem::size_of::<f32>()) as u32;
                let cbytes = (v_count * 4 * std::mem::size_of::<u8>()) as u32;
                let ibytes = (take_q * 6 * std::mem::size_of::<u16>()) as u32;
                raw.vertices = raylib::ffi::MemAlloc(vbytes) as *mut f32;
                raw.normals = raylib::ffi::MemAlloc(nbytes) as *mut f32;
                raw.texcoords = raylib::ffi::MemAlloc(tbytes) as *mut f32;
                raw.colors = raylib::ffi::MemAlloc(cbytes) as *mut u8;
                raw.indices = raylib::ffi::MemAlloc(ibytes) as *mut u16;

                // Copy vertex attributes slice
                std::ptr::copy_nonoverlapping(mb.pos[pos_start..pos_end].as_ptr(), raw.vertices, v_count * 3);
                std::ptr::copy_nonoverlapping(mb.norm[norm_start..norm_end].as_ptr(), raw.normals, v_count * 3);
                std::ptr::copy_nonoverlapping(mb.uv[uv_start..uv_end].as_ptr(), raw.texcoords, v_count * 2);
                std::ptr::copy_nonoverlapping(mb.col[col_start..col_end].as_ptr(), raw.colors, v_count * 4);

                // Rebuild indices per submesh with local bases (0..v_count)
                let idx_ptr = raw.indices;
                let mut write = 0usize;
                for i in 0..take_q {
                    let base = (i * 4) as u16;
                    let tri = [base, base + 1, base + 2, base, base + 2, base + 3];
                    let dst = idx_ptr.add(write);
                    std::ptr::copy_nonoverlapping(tri.as_ptr(), dst, 6);
                    write += 6;
                }
            }
            let mut mesh = unsafe { raylib::core::models::Mesh::from_raw(raw) };
            unsafe { mesh.upload(false); }
            let model = rl
                .load_model_from_mesh(thread, unsafe { mesh.make_weak() })
                .ok()?;
            // Assign texture
            let mut model = model;
            if let Some(mat) = model.materials_mut().get_mut(0) {
                if let Some(mdef) = mats.get(mid) {
                    let candidates: Vec<String> = mdef
                        .texture_candidates
                        .iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect();
                    let chosen: Option<String> = candidates
                        .iter()
                        .find(|p| std::path::Path::new(p.as_str()).exists())
                        .cloned()
                        .or_else(|| candidates.first().cloned());
                    if let Some(path) = chosen {
                        let key = std::fs::canonicalize(&path)
                            .ok()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or(path);
                        use std::collections::hash_map::Entry;
                        match tex_cache.map.entry(key.clone()) {
                            Entry::Occupied(e) => {
                                let tex = e.into_mut();
                                mat.set_material_texture(
                                    raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO,
                                    tex,
                                );
                            }
                            Entry::Vacant(v) => {
                                if let Ok(t) = rl.load_texture(thread, &key) {
                                    t.set_texture_filter(
                                        thread,
                                        raylib::consts::TextureFilter::TEXTURE_FILTER_POINT,
                                    );
                                    t.set_texture_wrap(
                                        thread,
                                        raylib::consts::TextureWrap::TEXTURE_WRAP_REPEAT,
                                    );
                                    let tex = v.insert(t);
                                    mat.set_material_texture(
                                        raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO,
                                        tex,
                                    );
                                }
                            }
                        }
                    }
                }
            }
            parts_gpu.push((mid, model));
            q += take_q;
        }
    }
    Some(ChunkRender {
        cx: cpu.cx,
        cz: cpu.cz,
        bbox: cpu.bbox,
        parts: parts_gpu,
        leaf_tint: None,
    })
}

// Purged world-based synchronous build path; buffer-based pipeline is authoritative.

// Simple per-app texture cache keyed by file path; loads each texture once and reuses it across chunks.
pub struct TextureCache {
    map: HashMap<String, raylib::core::texture::Texture2D>,
}

// Local-body mesher: emits vertices in local-space [0..sx, 0..sz], no world/lighting deps.
pub fn build_voxel_body_cpu_buf(buf: &ChunkBuf, ambient: u8, reg: &BlockRegistry) -> ChunkMeshCPU {
    let sx = buf.sx;
    let sy = buf.sy;
    let sz = buf.sz;

    // Unified path via meshing_core + special-shapes pass to match world mesher
    #[inline]
    fn solid_local(buf: &ChunkBuf, x: i32, y: i32, z: i32, reg: &BlockRegistry) -> bool {
        if x < 0 || y < 0 || z < 0 {
            return false;
        }
        let (xu, yu, zu) = (x as usize, y as usize, z as usize);
        if xu >= buf.sx || yu >= buf.sy || zu >= buf.sz {
            return false;
        }
        let b = buf.get_local(xu, yu, zu);
        reg.get(b.id).map(|ty| ty.is_solid(b.state)).unwrap_or(false)
    }

    #[inline]
    fn face_light(face: usize, ambient: u8) -> u8 {
        match face {
            0 => ambient.saturating_add(40).min(255),
            1 => ambient.saturating_sub(60),
            _ => ambient,
        }
    }

    // Match world mesher V orientation for all faces
    let flip_v = [false, false, false, false, false, false];

    // Skip non-cubic shapes in greedy pass; they are emitted below
    let mut builds = crate::meshing_core::build_mesh_core(buf, 0, 0, flip_v, None, |x, y, z, face, here| {
        if !is_solid_runtime(here, reg) {
            return None;
        }
        if let Some(ty) = reg.get(here.id) {
            match ty.shape {
                crate::blocks::Shape::Slab { .. } | crate::blocks::Shape::Stairs { .. } => {
                    return None;
                }
                _ => {}
            }
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
        // Local occlusion: only cull when an in-bounds neighbor truly occludes this face.
        if nx >= 0 && ny >= 0 && nz >= 0 {
            let (xu, yu, zu) = (nx as usize, ny as usize, nz as usize);
            if xu < buf.sx && yu < buf.sy && zu < buf.sz {
                let nb = buf.get_local(xu, yu, zu);
                if occludes_face(nb, face, reg) {
                    return None;
                }
            }
        }
        let mid = registry_material_for_or_unknown(here, face, reg);
        let l = face_light(face, ambient);
        Some((mid, l))
    });

    // Helpers for special-shapes pass
    #[inline]
    fn occludes_local(buf: &ChunkBuf, x: i32, y: i32, z: i32, face: usize, reg: &BlockRegistry) -> bool {
        if x < 0 || y < 0 || z < 0 {
            return false;
        }
        let (xu, yu, zu) = (x as usize, y as usize, z as usize);
        if xu >= buf.sx || yu >= buf.sy || zu >= buf.sz {
            return false;
        }
        let nb = buf.get_local(xu, yu, zu);
        occludes_face(nb, face, reg)
    }

    #[inline]
    fn emit_box_local(
        builds: &mut std::collections::HashMap<MaterialId, MeshBuild>,
        buf: &ChunkBuf,
        reg: &BlockRegistry,
        x: usize,
        y: usize,
        z: usize,
        face_material: &dyn Fn(usize) -> MaterialId,
        min: Vector3,
        max: Vector3,
        ambient: u8,
    ) {
        let gx = x as i32;
        let gy = y as i32;
        let gz = z as i32;
        emit_box_faces(builds, min, max, |face| {
            let (nx, ny, nz) = match face {
                0 => (gx, gy + 1, gz),
                1 => (gx, gy - 1, gz),
                2 => (gx + 1, gy, gz),
                3 => (gx - 1, gy, gz),
                4 => (gx, gy, gz + 1),
                5 => (gx, gy, gz - 1),
                _ => unreachable!(),
            };
            if occludes_local(buf, nx, ny, nz, face, reg) {
                return None;
            }
            let lv = face_light(face, ambient);
            let rgba = [lv, lv, lv, 255];
            let mid = face_material(face);
            Some((mid, rgba))
        });
    }

    // Special-shapes pass: slabs and stairs
    let sx = buf.sx;
    let sy = buf.sy;
    let sz = buf.sz;
    for z in 0..sz {
        for y in 0..sy {
            for x in 0..sx {
                let b = buf.get_local(x, y, z);
                if let Some(ty) = reg.get(b.id) {
                    match &ty.shape {
                        crate::blocks::Shape::Slab { half_from } => {
                            let fx = x as f32;
                            let fy = y as f32;
                            let fz = z as f32;
                            let is_top = ty.state_prop_is_value(b.state, half_from, "top");
                            let (y0, y1) = if is_top { (fy + 0.5, fy + 1.0) } else { (fy, fy + 0.5) };
                            let min = Vector3::new(fx, y0, fz);
                            let max = Vector3::new(fx + 1.0, y1, fz + 1.0);
                            let face_material = |face: usize| {
                                let role = match face {
                                    0 => FaceRole::Top,
                                    1 => FaceRole::Bottom,
                                    _ => FaceRole::Side,
                                };
                                ty.materials
                                    .material_for(role, b.state, ty)
                                    .unwrap_or_else(|| unknown_material_id(reg))
                            };
                            emit_box_local(&mut builds, buf, reg, x, y, z, &face_material, min, max, ambient);

                            // Restore partial neighbor faces for full-cube neighbors
                            let (vis_y0, vis_y1) = if is_top { (fy, fy + 0.5) } else { (fy + 0.5, fy + 1.0) };
                            let is_full_cube = |nb: Block| -> bool {
                                reg.get(nb.id)
                                    .map(|t| matches!(t.shape, crate::blocks::Shape::Cube | crate::blocks::Shape::AxisCube { .. }))
                                    .unwrap_or(false)
                            };
                            // West neighbor (+X face on neighbor)
                            if x > 0 {
                                let nb = buf.get_local(x - 1, y, z);
                                if is_full_cube(nb) {
                                    let rgba = {
                                        let lv = face_light(2, ambient);
                                        [lv, lv, lv, 255]
                                    };
                                    let mid = registry_material_for_or_unknown(nb, 2, reg);
                                    let mb = builds.entry(mid).or_default();
                                    let px = fx; // plane at x
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
                            // East neighbor (-X face on neighbor)
                            if x + 1 < sx {
                                let nb = buf.get_local(x + 1, y, z);
                                if is_full_cube(nb) {
                                    let rgba = {
                                        let lv = face_light(3, ambient);
                                        [lv, lv, lv, 255]
                                    };
                                    let mid = registry_material_for_or_unknown(nb, 3, reg);
                                    let mb = builds.entry(mid).or_default();
                                    let px = fx + 1.0; // plane at x+1
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
                            // North neighbor (+Z face on neighbor)
                            if z > 0 {
                                let nb = buf.get_local(x, y, z - 1);
                                if is_full_cube(nb) {
                                    let rgba = {
                                        let lv = face_light(4, ambient);
                                        [lv, lv, lv, 255]
                                    };
                                    let mid = registry_material_for_or_unknown(nb, 4, reg);
                                    let mb = builds.entry(mid).or_default();
                                    let pz = fz; // plane at z
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
                            // South neighbor (-Z face on neighbor)
                            if z + 1 < sz {
                                let nb = buf.get_local(x, y, z + 1);
                                if is_full_cube(nb) {
                                    let rgba = {
                                        let lv = face_light(5, ambient);
                                        [lv, lv, lv, 255]
                                    };
                                    let mid = registry_material_for_or_unknown(nb, 5, reg);
                                    let mb = builds.entry(mid).or_default();
                                    let pz = fz + 1.0; // plane at z+1
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
                        crate::blocks::Shape::Stairs { facing_from, half_from } => {
                            let fx = x as f32;
                            let fy = y as f32;
                            let fz = z as f32;
                            let is_top = ty.state_prop_is_value(b.state, half_from, "top");
                            let facing = ty.state_prop_value(b.state, facing_from).unwrap_or("north");
                            // Big half-height slab
                            let (min_a, max_a) = if is_top {
                                (Vector3::new(fx, fy + 0.5, fz), Vector3::new(fx + 1.0, fy + 1.0, fz + 1.0))
                            } else {
                                (Vector3::new(fx, fy, fz), Vector3::new(fx + 1.0, fy + 0.5, fz + 1.0))
                            };
                            let face_material = |face: usize| {
                                let role = match face {
                                    0 => FaceRole::Top,
                                    1 => FaceRole::Bottom,
                                    _ => FaceRole::Side,
                                };
                                ty.materials
                                    .material_for(role, b.state, ty)
                                    .unwrap_or_else(|| unknown_material_id(reg))
                            };
                            emit_box_local(&mut builds, buf, reg, x, y, z, &face_material, min_a, max_a, ambient);
                            // Secondary half slab toward facing
                            let (min_b, max_b) = match (facing, is_top) {
                                ("north", false) => (Vector3::new(fx, fy + 0.5, fz), Vector3::new(fx + 1.0, fy + 1.0, fz + 0.5)),
                                ("south", false) => (Vector3::new(fx, fy + 0.5, fz + 0.5), Vector3::new(fx + 1.0, fy + 1.0, fz + 1.0)),
                                ("west",  false) => (Vector3::new(fx, fy + 0.5, fz), Vector3::new(fx + 0.5, fy + 1.0, fz + 1.0)),
                                ("east",  false) => (Vector3::new(fx + 0.5, fy + 0.5, fz), Vector3::new(fx + 1.0, fy + 1.0, fz + 1.0)),
                                ("north", true)  => (Vector3::new(fx, fy, fz + 0.5), Vector3::new(fx + 1.0, fy + 0.5, fz + 1.0)),
                                ("south", true)  => (Vector3::new(fx, fy, fz),       Vector3::new(fx + 1.0, fy + 0.5, fz + 0.5)),
                                ("west",  true)  => (Vector3::new(fx + 0.5, fy, fz), Vector3::new(fx + 1.0, fy + 0.5, fz + 1.0)),
                                ("east",  true)  => (Vector3::new(fx, fy, fz),       Vector3::new(fx + 0.5, fy + 0.5, fz + 1.0)),
                                _ => (Vector3::new(fx, fy, fz), Vector3::new(fx, fy, fz)),
                            };
                            if max_b.x > min_b.x || max_b.y > min_b.y || max_b.z > min_b.z {
                                emit_box_local(&mut builds, buf, reg, x, y, z, &face_material, min_b, max_b, ambient);
                            }

                            // Neighbor face restoration for full cubes with oriented segments
                            let draw_top = !is_top;
                            let y0 = if is_top { fy + 0.5 } else { fy };
                            let y1 = if is_top { fy + 1.0 } else { fy + 0.5 };
                            let is_full_cube = |nb: Block| -> bool {
                                reg.get(nb.id)
                                    .map(|t| matches!(t.shape, crate::blocks::Shape::Cube | crate::blocks::Shape::AxisCube { .. }))
                                    .unwrap_or(false)
                            };
                            // West neighbor (+X on neighbor)
                            if x > 0 {
                                let nb = buf.get_local(x - 1, y, z);
                                if is_full_cube(nb) {
                                    let rgba = { let lv = face_light(2, ambient); [lv, lv, lv, 255] };
                                    let mid = registry_material_for_or_unknown(nb, 2, reg);
                                    let mb = builds.entry(mid).or_default();
                                    let px = fx;
                                    let segs: &[(f32, f32)] = match (facing, is_top) {
                                        ("north", false) => &[(fz + 0.5, fz + 1.0)],
                                        ("south", false) => &[(fz, fz + 0.5)],
                                        ("west",  false) => &[],
                                        ("east",  false) => &[(fz, fz + 1.0)],
                                        ("north", true)  => &[(fz + 0.5, fz + 1.0)],
                                        ("south", true)  => &[(fz, fz + 0.5)],
                                        ("west",  true)  => &[],
                                        ("east",  true)  => &[(fz, fz + 1.0)],
                                        _ => &[],
                                    };
                                    for &(z0, z1) in segs.iter() {
                                        if z1 <= z0 { continue; }
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
                            // East neighbor (-X on neighbor)
                            if x + 1 < sx {
                                let nb = buf.get_local(x + 1, y, z);
                                if is_full_cube(nb) {
                                    let rgba = { let lv = face_light(3, ambient); [lv, lv, lv, 255] };
                                    let mid = registry_material_for_or_unknown(nb, 3, reg);
                                    let mb = builds.entry(mid).or_default();
                                    let px = fx + 1.0;
                                    let segs: &[(f32, f32)] = match (facing, is_top) {
                                        ("north", false) => &[(fz + 0.5, fz + 1.0)],
                                        ("south", false) => &[(fz, fz + 0.5)],
                                        ("west",  false) => &[(fz, fz + 1.0)],
                                        ("east",  false) => &[],
                                        ("north", true)  => &[(fz + 0.5, fz + 1.0)],
                                        ("south", true)  => &[(fz, fz + 0.5)],
                                        ("west",  true)  => &[(fz, fz + 1.0)],
                                        ("east",  true)  => &[],
                                        _ => &[],
                                    };
                                    for &(z0, z1) in segs.iter() {
                                        if z1 <= z0 { continue; }
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
                            // North neighbor (+Z on neighbor)
                            if z > 0 {
                                let nb = buf.get_local(x, y, z - 1);
                                if is_full_cube(nb) {
                                    let rgba = { let lv = face_light(4, ambient); [lv, lv, lv, 255] };
                                    let mid = registry_material_for_or_unknown(nb, 4, reg);
                                    let mb = builds.entry(mid).or_default();
                                    let pz = fz;
                                    let segs: &[(f32, f32)] = match (facing, is_top) {
                                        ("east",  false) => &[(fx + 0.5, fx + 1.0)],
                                        ("west",  false) => &[(fx, fx + 0.5)],
                                        ("north", false) => &[(fx, fx + 1.0)],
                                        ("south", false) => &[],
                                        ("east",  true)  => &[(fx + 0.5, fx + 1.0)],
                                        ("west",  true)  => &[(fx, fx + 0.5)],
                                        ("north", true)  => &[(fx, fx + 1.0)],
                                        ("south", true)  => &[],
                                        _ => &[],
                                    };
                                    for &(x0f, x1f) in segs.iter() {
                                        if x1f <= x0f { continue; }
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
                            // South neighbor (-Z on neighbor)
                            if z + 1 < sz {
                                let nb = buf.get_local(x, y, z + 1);
                                if is_full_cube(nb) {
                                    let rgba = { let lv = face_light(5, ambient); [lv, lv, lv, 255] };
                                    let mid = registry_material_for_or_unknown(nb, 5, reg);
                                    let mb = builds.entry(mid).or_default();
                                    let pz = fz + 1.0;
                                    let segs: &[(f32, f32)] = match (facing, is_top) {
                                        ("east",  false) => &[(fx + 0.5, fx + 1.0)],
                                        ("west",  false) => &[(fx, fx + 0.5)],
                                        ("north", false) => &[],
                                        ("south", false) => &[(fx, fx + 1.0)],
                                        ("east",  true)  => &[(fx + 0.5, fx + 1.0)],
                                        ("west",  true)  => &[(fx, fx + 0.5)],
                                        ("north", true)  => &[],
                                        ("south", true)  => &[(fx, fx + 1.0)],
                                        _ => &[],
                                    };
                                    for &(x0f, x1f) in segs.iter() {
                                        if x1f <= x0f { continue; }
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

    // Legacy API removed; prefer get_ref + insert_from_path

    pub fn get_ref(&self, key: &str) -> Option<&raylib::core::texture::Texture2D> {
        self.map.get(key)
    }

    pub fn insert_from_path<'a>(
        &'a mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        key: &str,
    ) -> Option<&'a raylib::core::texture::Texture2D> {
        if let Ok(t) = rl.load_texture(thread, key) {
            t.set_texture_filter(
                thread,
                raylib::consts::TextureFilter::TEXTURE_FILTER_POINT,
            );
            t.set_texture_wrap(
                thread,
                raylib::consts::TextureWrap::TEXTURE_WRAP_REPEAT,
            );
            self.map.insert(key.to_string(), t);
            return self.map.get(key);
        }
        None
    }

    pub fn replace_loaded(
        &mut self,
        key: String,
        tex: raylib::core::texture::Texture2D,
    ) {
        self.map.insert(key, tex);
    }

    // Note: higher-level helpers operate on a single chosen path to avoid borrow issues
}

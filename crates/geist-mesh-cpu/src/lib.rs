//! CPU meshing crate: greedy mesher and helpers (engine-only).
#![forbid(unsafe_code)]

use geist_blocks::BlockRegistry;
use geist_blocks::types::{Block, FaceRole, MaterialId};
use geist_chunk::ChunkBuf;
use geist_geom::{Aabb, Vec3};
use geist_lighting::{LightBorders, LightGrid, LightingStore};
use geist_world::World;
use std::collections::HashMap;
use std::hash::Hash;

pub mod microgrid_tables;

// Visual-only lighting floor to avoid pitch-black faces in darkness.
// Does not affect logical light propagation.
const VISUAL_LIGHT_MIN: u8 = 18; // ~7% brightness floor

#[derive(Default, Clone)]
pub struct MeshBuild {
    pub pos: Vec<f32>,
    pub norm: Vec<f32>,
    pub uv: Vec<f32>,
    pub idx: Vec<u16>,
    pub col: Vec<u8>,
}

impl MeshBuild {
    pub fn add_quad(
        &mut self,
        a: Vec3,
        b: Vec3,
        c: Vec3,
        d: Vec3,
        n: Vec3,
        u1: f32,
        v1: f32,
        flip_v: bool,
        rgba: [u8; 4],
    ) {
        let base = self.pos.len() as u32 / 3;
        let mut vs = [a, d, c, b];
        let mut uvs = [(0.0, 0.0), (0.0, v1), (u1, v1), (u1, 0.0)];
        let e1 = Vec3 { x: vs[1].x - vs[0].x, y: vs[1].y - vs[0].y, z: vs[1].z - vs[0].z };
        let e2 = Vec3 { x: vs[2].x - vs[0].x, y: vs[2].y - vs[0].y, z: vs[2].z - vs[0].z };
        let cross = Vec3 { x: e1.y * e2.z - e1.z * e2.y, y: e1.z * e2.x - e1.x * e2.z, z: e1.x * e2.y - e1.y * e2.x };
        if (cross.x * n.x + cross.y * n.y + cross.z * n.z) < 0.0 {
            vs.swap(1, 3);
            uvs.swap(1, 3);
        }
        if flip_v {
            for uv in &mut uvs {
                uv.1 = v1 - uv.1;
            }
        }
        for i in 0..4 {
            self.pos
                .extend_from_slice(&[vs[i].x, vs[i].y, vs[i].z]);
            self.norm.extend_from_slice(&[n.x, n.y, n.z]);
            self.uv.extend_from_slice(&[uvs[i].0, uvs[i].1]);
            self.col
                .extend_from_slice(&[rgba[0], rgba[1], rgba[2], rgba[3]]);
        }
        self.idx.extend_from_slice(&[
            base as u16,
            (base + 1) as u16,
            (base + 2) as u16,
            base as u16,
            (base + 2) as u16,
            (base + 3) as u16,
        ]);
    }

    pub fn add_face_rect(
        &mut self,
        face: Face,
        origin: Vec3,
        u1: f32,
        v1: f32,
        flip_v: bool,
        rgba: [u8; 4],
    ) {
        let n = face.normal();
        let (a, b, c, d) = match face {
            Face::PosY => (
                origin,
                Vec3 { x: origin.x + u1, y: origin.y, z: origin.z },
                Vec3 { x: origin.x + u1, y: origin.y, z: origin.z + v1 },
                Vec3 { x: origin.x, y: origin.y, z: origin.z + v1 },
            ),
            Face::NegY => (
                Vec3 { x: origin.x, y: origin.y, z: origin.z + v1 },
                Vec3 { x: origin.x + u1, y: origin.y, z: origin.z + v1 },
                Vec3 { x: origin.x + u1, y: origin.y, z: origin.z },
                origin,
            ),
            Face::PosX => (
                Vec3 { x: origin.x, y: origin.y + v1, z: origin.z + u1 },
                Vec3 { x: origin.x, y: origin.y + v1, z: origin.z },
                origin,
                Vec3 { x: origin.x, y: origin.y, z: origin.z + u1 },
            ),
            Face::NegX => (
                Vec3 { x: origin.x, y: origin.y + v1, z: origin.z },
                Vec3 { x: origin.x, y: origin.y + v1, z: origin.z + u1 },
                Vec3 { x: origin.x, y: origin.y, z: origin.z + u1 },
                origin,
            ),
            Face::PosZ => (
                Vec3 { x: origin.x + u1, y: origin.y + v1, z: origin.z },
                Vec3 { x: origin.x, y: origin.y + v1, z: origin.z },
                origin,
                Vec3 { x: origin.x + u1, y: origin.y, z: origin.z },
            ),
            Face::NegZ => (
                Vec3 { x: origin.x, y: origin.y + v1, z: origin.z },
                Vec3 { x: origin.x + u1, y: origin.y + v1, z: origin.z },
                Vec3 { x: origin.x + u1, y: origin.y, z: origin.z },
                origin,
            ),
        };
        self.add_quad(a, b, c, d, n, u1, v1, flip_v, rgba);
    }

    // Accessors used by debug/diagnostic paths
    pub fn positions(&self) -> &[f32] { &self.pos }
    pub fn normals(&self) -> &[f32] { &self.norm }
}

#[inline]
fn unknown_material_id(reg: &BlockRegistry) -> MaterialId {
    reg.materials.get_id("unknown").unwrap_or(MaterialId(0))
}

#[inline]
fn registry_material_for_or_unknown(block: Block, face: Face, reg: &BlockRegistry) -> MaterialId {
    reg.get(block.id)
        .map(|ty| ty.material_for_cached(face.role(), block.state))
        .unwrap_or_else(|| unknown_material_id(reg))
}

#[inline]
fn emit_face_rect_for(
    builds: &mut std::collections::HashMap<MaterialId, MeshBuild>,
    mid: MaterialId,
    face: Face,
    origin: Vec3,
    u1: f32,
    v1: f32,
    rgba: [u8; 4],
) {
    let mb = builds.entry(mid).or_default();
    mb.add_face_rect(face, origin, u1, v1, false, rgba);
}

#[inline]
fn sample_neighbor_half_light(
    light: &LightGrid,
    x: usize,
    y: usize,
    z: usize,
    face: Face,
    draw_top_half: bool,
    sy: usize,
) -> u8 {
    let l0 = light.sample_face_local(x, y, z, face.index());
    let ladd = if draw_top_half {
        if y + 1 < sy {
            light.sample_face_local(x, y + 1, z, face.index())
        } else {
            l0
        }
    } else if y > 0 {
        light.sample_face_local(x, y - 1, z, face.index())
    } else {
        l0
    };
    l0.max(ladd).max(VISUAL_LIGHT_MIN)
}

#[inline]
fn emit_neighbor_fixups_micro_generic(
    builds: &mut std::collections::HashMap<MaterialId, MeshBuild>,
    buf: &ChunkBuf,
    reg: &BlockRegistry,
    x: usize,
    y: usize,
    z: usize,
    fx: f32,
    fy: f32,
    fz: f32,
    occ: u8,
    project_fixups: bool,
    mut light_for_neighbor: impl FnMut(usize, usize, usize, Face, bool) -> u8,
) {
    use microgrid_tables::{empty4_to_rects, occ8_to_boxes};
    if !project_fixups {
        return;
    }
    let sx = buf.sx as i32;
    let sz = buf.sz as i32;
    let cell = 0.5f32;

    for &(dx, dz, face, x_off, z_off) in &SIDE_NEIGHBORS {
        let nx = x as i32 + dx;
        let nz = z as i32 + dz;
        if nx < 0 || nx >= sx || nz < 0 || nz >= sz {
            continue;
        }
        let nb = buf.get_local(nx as usize, y, nz as usize);
        let (neighbor_ok, nb_occ) = if let Some(ty) = reg.get(nb.id) {
            let var = ty.variant(nb.state);
            if var.occupancy.is_some() {
                (true, var.occupancy.unwrap())
            } else if is_full_cube(reg, nb) {
                (true, 0xFFu8)
            } else {
                (false, 0)
            }
        } else {
            (false, 0)
        };
        if !neighbor_ok {
            continue;
        }
        let mid = registry_material_for_or_unknown(nb, face, reg);
        match face {
            Face::PosX | Face::NegX => {
                let bx = if dx < 0 { 0 } else { 1 };
                let nb_bx = match face {
                    Face::PosX => 1,
                    Face::NegX => 0,
                    _ => bx,
                };
                let mut mask: u8 = 0;
                for ly in 0..2 {
                    for lz in 0..2 {
                        let empty_here = (occ & micro_bit(bx, ly, lz)) == 0;
                        let nb_has = (nb_occ & micro_bit(nb_bx, ly, lz)) != 0;
                        if empty_here && nb_has {
                            let bit = ((ly << 1) | lz) as u8;
                            mask |= 1u8 << bit;
                        }
                    }
                }
                for r in empty4_to_rects(mask) {
                    let u0 = r[0] as f32 * cell; // along Z
                    let v0 = r[1] as f32 * cell; // along Y
                    let du = r[2] as f32 * cell;
                    let dv = r[3] as f32 * cell;
                    let lv = if r[3] == 2 {
                        let l0 = light_for_neighbor(nx as usize, y, nz as usize, face, false);
                        let l1 = light_for_neighbor(nx as usize, y, nz as usize, face, true);
                        l0.max(l1)
                    } else {
                        let draw_top = r[1] == 1; // v0==1 -> top half
                        light_for_neighbor(nx as usize, y, nz as usize, face, draw_top)
                    };
                    let rgba = [lv, lv, lv, 255];
                    let origin = Vec3 { x: fx + x_off, y: fy + v0, z: fz + u0 };
                    emit_face_rect_for(builds, mid, face, origin, du, dv, rgba);
                }
            }
            Face::PosZ | Face::NegZ => {
                let bz = if dz < 0 { 0 } else { 1 };
                let nb_bz = match face {
                    Face::PosZ => 1,
                    Face::NegZ => 0,
                    _ => bz,
                };
                let mut mask: u8 = 0;
                for ly in 0..2 {
                    for lx in 0..2 {
                        let empty_here = (occ & micro_bit(lx, ly, bz)) == 0;
                        let nb_has = (nb_occ & micro_bit(lx, ly, nb_bz)) != 0;
                        if empty_here && nb_has {
                            let bit = ((ly << 1) | lx) as u8;
                            mask |= 1u8 << bit;
                        }
                    }
                }
                for r in empty4_to_rects(mask) {
                    let u0 = r[0] as f32 * cell; // along X
                    let v0 = r[1] as f32 * cell; // along Y
                    let du = r[2] as f32 * cell;
                    let dv = r[3] as f32 * cell;
                    let lv = if r[3] == 2 {
                        let l0 = light_for_neighbor(nx as usize, y, nz as usize, face, false);
                        let l1 = light_for_neighbor(nx as usize, y, nz as usize, face, true);
                        l0.max(l1)
                    } else {
                        let draw_top = r[1] == 1;
                        light_for_neighbor(nx as usize, y, nz as usize, face, draw_top)
                    };
                    let rgba = [lv, lv, lv, 255];
                    let origin = Vec3 { x: fx + u0, y: fy + v0, z: fz + z_off };
                    emit_face_rect_for(builds, mid, face, origin, du, dv, rgba);
                }
            }
            _ => {}
        }
    }

    let sy = buf.sy as i32;
    for &(dy, face) in &[(-1, Face::PosY), (1, Face::NegY)] {
        let ny = y as i32 + dy;
        if ny < 0 || ny >= sy {
            continue;
        }
        let nb = buf.get_local(x, ny as usize, z);
        let (neighbor_ok, nb_occ) = if let Some(ty) = reg.get(nb.id) {
            let var = ty.variant(nb.state);
            if var.occupancy.is_some() {
                (true, var.occupancy.unwrap())
            } else if is_full_cube(reg, nb) {
                (true, 0xFFu8)
            } else {
                (false, 0)
            }
        } else {
            (false, 0)
        };
        if !neighbor_ok {
            continue;
        }
        let mid = registry_material_for_or_unknown(nb, face, reg);
        let by = if dy < 0 { 0 } else { 1 };
        let nb_by = match face {
            Face::PosY => 1,
            Face::NegY => 0,
            _ => by,
        };
        let mut mask: u8 = 0;
        for lz in 0..2 {
            for lx in 0..2 {
                let empty_here = (occ & micro_bit(lx, by, lz)) == 0;
                let nb_has = (nb_occ & micro_bit(lx, nb_by, lz)) != 0;
                if empty_here && nb_has {
                    let bit = ((lz << 1) | lx) as u8;
                    mask |= 1u8 << bit;
                }
            }
        }
        for r in microgrid_tables::empty4_to_rects(mask) {
            let u0 = r[0] as f32 * cell; // along X
            let v0 = r[1] as f32 * cell; // along Z
            let du = r[2] as f32 * cell;
            let dv = r[3] as f32 * cell;
            let y0 = if dy < 0 { fy } else { fy + 1.0 };
            let lv = if r[3] == 2 {
                let l0 = light_for_neighbor(x, ny as usize, z, face, false);
                let l1 = light_for_neighbor(x, ny as usize, z, face, true);
                l0.max(l1)
            } else {
                let draw_top = r[1] == 1;
                light_for_neighbor(x, ny as usize, z, face, draw_top)
            };
            let rgba = [lv, lv, lv, 255];
            let origin = Vec3 { x: fx + u0, y: y0, z: fz + v0 };
            emit_face_rect_for(builds, mid, face, origin, du, dv, rgba);
        }
    }
}

#[inline]
fn is_solid_runtime(b: Block, reg: &BlockRegistry) -> bool {
    reg.get(b.id)
        .map(|ty| ty.is_solid(b.state))
        .unwrap_or(false)
}

#[inline]
fn is_top_half_shape(b: Block, reg: &BlockRegistry) -> bool {
    reg.get(b.id).map_or(false, |ty| match &ty.shape {
        geist_blocks::types::Shape::Slab { half_from }
        | geist_blocks::types::Shape::Stairs { half_from, .. } => {
            ty.state_prop_is_value(b.state, half_from, "top")
        }
        _ => false,
    })
}

const LOCAL_FACE_LIGHT: [i16; 6] = [40, -60, 0, 0, 0, 0];

fn face_light(face: Face, ambient: u8) -> u8 {
    let bias = LOCAL_FACE_LIGHT[face.index()];
    if bias >= 0 {
        ambient.saturating_add(bias as u8)
    } else {
        ambient.saturating_sub((-bias) as u8)
    }
}

#[inline]
fn emit_box_faces(
    builds: &mut std::collections::HashMap<MaterialId, MeshBuild>,
    min: Vec3,
    max: Vec3,
    mut choose: impl FnMut(Face) -> Option<(MaterialId, [u8; 4])>,
) {
    const FACE_DATA: [(Face, [usize; 4], (f32, f32, f32)); 6] = [
        (Face::PosY, [0, 2, 6, 4], (0.0, 1.0, 0.0)),
        (Face::NegY, [5, 7, 3, 1], (0.0, -1.0, 0.0)),
        (Face::PosX, [6, 2, 3, 7], (1.0, 0.0, 0.0)),
        (Face::NegX, [0, 4, 5, 1], (-1.0, 0.0, 0.0)),
        (Face::PosZ, [4, 6, 7, 5], (0.0, 0.0, 1.0)),
        (Face::NegZ, [2, 0, 1, 3], (0.0, 0.0, -1.0)),
    ];

    let corners = [
        Vec3 { x: min.x, y: max.y, z: min.z },
        Vec3 { x: min.x, y: min.y, z: min.z },
        Vec3 { x: max.x, y: max.y, z: min.z },
        Vec3 { x: max.x, y: min.y, z: min.z },
        Vec3 { x: min.x, y: max.y, z: max.z },
        Vec3 { x: min.x, y: min.y, z: max.z },
        Vec3 { x: max.x, y: max.y, z: max.z },
        Vec3 { x: max.x, y: min.y, z: max.z },
    ];

    for &(face, indices, normal) in &FACE_DATA {
        if let Some((mid, rgba)) = choose(face) {
            let (u1, v1) = match face {
                Face::PosY | Face::NegY => (max.x - min.x, max.z - min.z),
                Face::PosX | Face::NegX => (max.z - min.z, max.y - min.y),
                Face::PosZ | Face::NegZ => (max.x - min.x, max.y - min.y),
            };
            let n = Vec3 { x: normal.0, y: normal.1, z: normal.2 };
            builds
                .entry(mid)
                .or_default()
                .add_quad(corners[indices[0]], corners[indices[1]], corners[indices[2]], corners[indices[3]], n, u1, v1, false, rgba);
        }
    }
}

#[inline]
fn emit_box_generic(
    builds: &mut std::collections::HashMap<MaterialId, MeshBuild>,
    min: Vec3,
    max: Vec3,
    fm_for_face: &dyn Fn(Face) -> MaterialId,
    mut occludes: impl FnMut(Face) -> bool,
    mut sample_light: impl FnMut(Face) -> u8,
) {
    emit_box_faces(builds, min, max, |face| {
        if occludes(face) { return None; }
        let lv = sample_light(face);
        let rgba = [lv, lv, lv, 255];
        let mid = fm_for_face(face);
        Some((mid, rgba))
    });
}

#[inline]
fn micro_bit(x: usize, y: usize, z: usize) -> u8 {
    1u8 << (((y & 1) << 2) | ((z & 1) << 1) | (x & 1))
}

#[inline]
fn microgrid_boxes(fx: f32, fy: f32, fz: f32, occ: u8) -> Vec<(Vec3, Vec3)> {
    use microgrid_tables::occ8_to_boxes;
    let cell = 0.5f32;
    let mut out = Vec::new();
    for b in occ8_to_boxes(occ) {
        let min = Vec3 { x: fx + (b[0] as f32) * cell, y: fy + (b[1] as f32) * cell, z: fz + (b[2] as f32) * cell };
        let max = Vec3 { x: fx + (b[3] as f32) * cell, y: fy + (b[4] as f32) * cell, z: fz + (b[5] as f32) * cell };
        out.push((min, max));
    }
    out
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NeighborsLoaded {
    pub neg_x: bool,
    pub pos_x: bool,
    pub neg_z: bool,
    pub pos_z: bool,
}

#[inline]
fn is_occluder(
    buf: &ChunkBuf,
    world: &World,
    edits: Option<&HashMap<(i32, i32, i32), Block>>,
    nmask: NeighborsLoaded,
    reg: &BlockRegistry,
    here: Block,
    face: Face,
    nx: i32,
    ny: i32,
    nz: i32,
) -> bool {
    if !is_solid_runtime(here, reg) {
        return false;
    }
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
        if let (Some(h), Some(_n)) = (reg.get(here.id), reg.get(nb.id)) {
            if h.seam.dont_occlude_same && here.id == nb.id {
                return false;
            }
        }
        return occludes_face(nb, face, reg);
    }
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
    if !neighbor_loaded {
        return false;
    }
    let nb = if let Some(es) = edits {
        es.get(&(nx, ny, nz))
            .copied()
            .unwrap_or_else(|| world.block_at_runtime(reg, nx, ny, nz))
    } else {
        world.block_at_runtime(reg, nx, ny, nz)
    };
    if let (Some(h), Some(_n)) = (reg.get(here.id), reg.get(nb.id)) {
        if h.seam.dont_occlude_same && here.id == nb.id {
            return false;
        }
    }
    occludes_face(nb, face, reg)
}

fn occlusion_mask_for(reg: &BlockRegistry, nb: Block) -> u8 {
    reg.get(nb.id)
        .map(|ty| ty.occlusion_mask_cached(nb.state))
        .unwrap_or(0)
}

#[inline]
fn occludes_face(nb: Block, face: Face, reg: &BlockRegistry) -> bool {
    let mask = occlusion_mask_for(reg, nb);
    (mask >> face.index()) & 1 == 1
}

pub struct ChunkMeshCPU {
    pub cx: i32,
    pub cz: i32,
    pub bbox: Aabb,
    pub parts: std::collections::HashMap<MaterialId, MeshBuild>,
}

pub fn build_chunk_greedy_cpu_buf(
    buf: &ChunkBuf,
    lighting: Option<&LightingStore>,
    world: &World,
    edits: Option<&HashMap<(i32, i32, i32), Block>>,
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

    let light = match lighting {
        Some(store) => LightGrid::compute_with_borders_buf(buf, store, reg),
        None => return None,
    };
    let flip_v = [false, false, false, false, false, false];
    let mut builds = build_mesh_core(
        buf,
        base_x,
        base_z,
        flip_v,
        Some(VISUAL_LIGHT_MIN),
        |x, y, z, face: Face, here| {
            if !is_solid_runtime(here, reg) {
                return None;
            }
            if let Some(ty) = reg.get(here.id) {
                if ty.variant(here.state).occupancy.is_some() {
                    return None;
                }
            }
            let gx = base_x + x as i32;
            let gy = y as i32;
            let gz = base_z + z as i32;
            let (dx, dy, dz) = face.delta();
            let (nx, ny, nz) = (gx + dx, gy + dy, gz + dz);
            if is_occluder(buf, world, edits, neighbors, reg, here, face, nx, ny, nz) {
                return None;
            }
            let mut l = light.sample_face_local(x, y, z, face.index());
            if matches!(face, Face::PosY)
                && buf.contains_world(nx, ny, nz)
                && ny >= 0
                && (ny as usize) < sy
            {
                let lx = (nx - base_x) as usize;
                let ly = ny as usize;
                let lz = (nz - base_z) as usize;
                let nb = buf.get_local(lx, ly, lz);
                if is_top_half_shape(nb, reg) {
                    let l2 = light
                        .sample_face_local(x, y, z, Face::PosX.index())
                        .max(light.sample_face_local(x, y, z, Face::NegX.index()))
                        .max(light.sample_face_local(x, y, z, Face::PosZ.index()))
                        .max(light.sample_face_local(x, y, z, Face::NegZ.index()));
                    l = l.max(l2);
                }
            }
            let mid = registry_material_for_or_unknown(here, face, reg);
            Some((mid, l))
        },
    );

    for z in 0..sz {
        for y in 0..sy {
            for x in 0..sx {
                let here = buf.get_local(x, y, z);
                let fx = (base_x + x as i32) as f32;
                let fy = y as f32;
                let fz = (base_z + z as i32) as f32;
                if let Some(ty) = reg.get(here.id) {
                    let var = ty.variant(here.state);
                    if let Some(occ) = var.occupancy {
                        let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
                        for (min, max) in microgrid_boxes(fx, fy, fz, occ) {
                            emit_box_generic(
                                &mut builds,
                                min,
                                max,
                                &face_material,
                                |face| {
                                    let (dx, dy, dz) = face.delta();
                                    let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                    is_occluder(
                                        buf, world, edits, neighbors, reg, here, face, nx, ny, nz,
                                    )
                                },
                                |face| {
                                    let lv = light.sample_face_local(x, y, z, face.index());
                                    lv.max(VISUAL_LIGHT_MIN)
                                },
                            );
                        }
                        emit_neighbor_fixups_micro_generic(
                            &mut builds,
                            buf,
                            reg,
                            x,
                            y,
                            z,
                            fx,
                            fy,
                            fz,
                            occ,
                            !ty.seam.dont_project_fixups,
                            |nx, ny, nz, face, draw_top_half| {
                                sample_neighbor_half_light(
                                    &light, nx, ny, nz, face, draw_top_half, sy,
                                )
                            },
                        );
                    } else {
                        match &ty.shape {
                            geist_blocks::types::Shape::Pane => {
                                let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
                                let t = 0.0625f32;
                                let min = Vec3 { x: fx + 0.5 - t, y: fy, z: fz };
                                let max = Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 1.0 };
                                emit_box_generic(
                                    &mut builds,
                                    min,
                                    max,
                                    &face_material,
                                    |face| {
                                        let (dx, dy, dz) = face.delta();
                                        let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                        is_occluder(
                                            buf, world, edits, neighbors, reg, here, face, nx, ny, nz,
                                        )
                                    },
                                    |face| {
                                        let lv = light.sample_face_local(x, y, z, face.index());
                                        lv.max(VISUAL_LIGHT_MIN)
                                    },
                                );
                                let reg_face = |role: FaceRole| match role {
                                    FaceRole::Top => Face::PosY,
                                    FaceRole::Bottom => Face::NegY,
                                    _ => Face::PosZ,
                                };
                                let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
                                let mut connect_xn = false;
                                let mut connect_xp = false;
                                let mut connect_zn = false;
                                let mut connect_zp = false;
                                {
                                    let f = reg_face(FaceRole::Side);
                                    let (dx, dy, dz) = f.delta();
                                    let nb = buf.get_world((fx as i32) + dx, (fy as i32) + dy, (fz as i32) + dz);
                                    if let Some(nb) = nb {
                                        if let Some(nb_ty) = reg.get(nb.id) {
                                            if nb_ty.shape == geist_blocks::types::Shape::Pane { connect_zp = true; }
                                        }
                                    }
                                }
                                {
                                    let f = reg_face(FaceRole::Side);
                                    let (dx, dy, dz) = Face::NegZ.delta();
                                    let nb = buf.get_world((fx as i32) + dx, (fy as i32) + dy, (fz as i32) + dz);
                                    if let Some(nb) = nb {
                                        if let Some(nb_ty) = reg.get(nb.id) {
                                            if nb_ty.shape == geist_blocks::types::Shape::Pane { connect_zn = true; }
                                        }
                                    }
                                }
                                {
                                    let (dx, dy, dz) = Face::PosX.delta();
                                    let nb = buf.get_world((fx as i32) + dx, (fy as i32) + dy, (fz as i32) + dz);
                                    if let Some(nb) = nb {
                                        if let Some(nb_ty) = reg.get(nb.id) {
                                            if nb_ty.shape == geist_blocks::types::Shape::Pane { connect_xp = true; }
                                        }
                                    }
                                }
                                {
                                    let (dx, dy, dz) = Face::NegX.delta();
                                    let nb = buf.get_world((fx as i32) + dx, (fy as i32) + dy, (fz as i32) + dz);
                                    if let Some(nb) = nb {
                                        if let Some(nb_ty) = reg.get(nb.id) {
                                            if nb_ty.shape == geist_blocks::types::Shape::Pane { connect_xn = true; }
                                        }
                                    }
                                }
                                if connect_xn {
                                    let min = Vec3 { x: fx, y: fy, z: fz + 0.5 - t };
                                    let max = Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 0.5 + t };
                                    emit_box_generic(
                                        &mut builds,
                                        min,
                                        max,
                                        &face_material,
                                        |face| {
                                            let (dx, dy, dz) = face.delta();
                                            let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                            is_occluder(
                                                buf, world, edits, neighbors, reg, here, face, nx, ny, nz,
                                            )
                                        },
                                        |face| {
                                            let lv = light.sample_face_local(x, y, z, face.index());
                                            lv.max(VISUAL_LIGHT_MIN)
                                        },
                                    );
                                }
                                if connect_xp {
                                    let min = Vec3 { x: fx + 0.5 - t, y: fy, z: fz + 0.5 - t };
                                    let max = Vec3 { x: fx + 1.0, y: fy + 1.0, z: fz + 0.5 + t };
                                    emit_box_generic(
                                        &mut builds,
                                        min,
                                        max,
                                        &face_material,
                                        |face| {
                                            let (dx, dy, dz) = face.delta();
                                            let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                            is_occluder(
                                                buf, world, edits, neighbors, reg, here, face, nx, ny, nz,
                                            )
                                        },
                                        |face| {
                                            let lv = light.sample_face_local(x, y, z, face.index());
                                            lv.max(VISUAL_LIGHT_MIN)
                                        },
                                    );
                                }
                                if connect_zn {
                                    let min = Vec3 { x: fx + 0.5 - t, y: fy, z: fz };
                                    let max = Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 0.5 + t };
                                    emit_box_generic(
                                        &mut builds,
                                        min,
                                        max,
                                        &face_material,
                                        |face| {
                                            let (dx, dy, dz) = face.delta();
                                            let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                            is_occluder(
                                                buf, world, edits, neighbors, reg, here, face, nx, ny, nz,
                                            )
                                        },
                                        |face| {
                                            let lv = light.sample_face_local(x, y, z, face.index());
                                            lv.max(VISUAL_LIGHT_MIN)
                                        },
                                    );
                                }
                                if connect_zp {
                                    let min = Vec3 { x: fx + 0.5 - t, y: fy, z: fz + 0.5 - t };
                                    let max = Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 1.0 };
                                    emit_box_generic(
                                        &mut builds,
                                        min,
                                        max,
                                        &face_material,
                                        |face| {
                                            let (dx, dy, dz) = face.delta();
                                            let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                            is_occluder(
                                                buf, world, edits, neighbors, reg, here, face, nx, ny, nz,
                                            )
                                        },
                                        |face| {
                                            let lv = light.sample_face_local(x, y, z, face.index());
                                            lv.max(VISUAL_LIGHT_MIN)
                                        },
                                    );
                                }
                            }
                            geist_blocks::types::Shape::Fence => {
                                let t = 0.125f32;
                                let p = 0.375f32;
                                let mut boxes: Vec<(Vec3, Vec3)> = Vec::new();
                                    boxes.push((Vec3 { x: fx + 0.5 - t, y: fy, z: fz + 0.5 - t }, Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 0.5 + t }));
                                let mut connect = [false; 4]; // X-, X+, Z-, Z+
                                let dirs = [Face::NegX, Face::PosX, Face::NegZ, Face::PosZ];
                                for (i, face) in dirs.iter().enumerate() {
                                    let (dx, dy, dz) = face.delta();
                                    let wx = (fx as i32) + dx;
                                    let wy = (fy as i32) + dy;
                                    let wz = (fz as i32) + dz;
                                    if let Some(nb) = buf.get_world(wx, wy, wz) {
                                        if let Some(nb_ty) = reg.get(nb.id) {
                                            connect[i] = nb_ty.shape == geist_blocks::types::Shape::Fence
                                                || nb_ty.shape == geist_blocks::types::Shape::Pane
                                                || matches!(nb_ty.shape, geist_blocks::types::Shape::Gate { .. })
                                                || nb_ty.shape == geist_blocks::types::Shape::Slab { half_from: String::new() };
                                        }
                                    }
                                }
                                if connect[0] {
                                    boxes.push((Vec3 { x: fx, y: fy + p, z: fz + 0.5 - t }, Vec3 { x: fx + 0.5 + t, y: fy + 1.0 - p, z: fz + 0.5 + t }));
                                }
                                if connect[1] {
                                    boxes.push((
                                        Vec3 { x: fx + 0.5 - t, y: fy + p, z: fz + 0.5 - t },
                                        Vec3 { x: fx + 1.0, y: fy + 1.0 - p, z: fz + 0.5 + t },
                                    ));
                                }
                                if connect[2] {
                                    boxes.push((Vec3 { x: fx + 0.5 - t, y: fy + p, z: fz }, Vec3 { x: fx + 0.5 + t, y: fy + 1.0 - p, z: fz + 0.5 + t }));
                                }
                                if connect[3] {
                                    boxes.push((Vec3 { x: fx + 0.5 - t, y: fy + p, z: fz + 0.5 - t }, Vec3 { x: fx + 0.5 + t, y: fy + 1.0 - p, z: fz + 1.0 }));
                                }
                                let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
                                for (min, max) in boxes {
                                    emit_box_generic(
                                        &mut builds,
                                        min,
                                        max,
                                        &face_material,
                                        |face| {
                                            let (dx, dy, dz) = face.delta();
                                            let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                            is_occluder(
                                                buf, world, edits, neighbors, reg, here, face, nx, ny, nz,
                                            )
                                        },
                                        |face| {
                                            let lv = light.sample_face_local(x, y, z, face.index());
                                            lv.max(VISUAL_LIGHT_MIN)
                                        },
                                    );
                                }
                            }
                            geist_blocks::types::Shape::Gate { .. } => {
                                let mut along_x = true;
                                if let geist_blocks::types::Shape::Gate { facing_from, open_from } = &ty.shape {
                                    let facing = ty.state_prop_value(here.state, facing_from).unwrap_or("north");
                                    along_x = matches!(facing, "north" | "south");
                                    if ty.state_prop_is_value(here.state, open_from, "true") {
                                        along_x = !along_x;
                                    }
                                }
                                let t = 0.125f32;
                                let y0 = 0.375f32;
                                let y1 = 0.625f32;
                                let mut boxes: Vec<(Vec3, Vec3)> = Vec::new();
                                if along_x {
                                    boxes.push((Vec3 { x: fx + 0.0, y: fy + y0, z: fz + 0.5 - t }, Vec3 { x: fx + 1.0, y: fy + y0 + t, z: fz + 0.5 + t }));
                                    boxes.push((Vec3 { x: fx + 0.0, y: fy + y1, z: fz + 0.5 - t }, Vec3 { x: fx + 1.0, y: fy + y1 + t, z: fz + 0.5 + t }));
                                } else {
                                    boxes.push((Vec3 { x: fx + 0.5 - t, y: fy + y0, z: fz + 0.0 }, Vec3 { x: fx + 0.5 + t, y: fy + y0 + t, z: fz + 1.0 }));
                                    boxes.push((Vec3 { x: fx + 0.5 - t, y: fy + y1, z: fz + 0.0 }, Vec3 { x: fx + 0.5 + t, y: fy + y1 + t, z: fz + 1.0 }));
                                }
                                let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
                                for (min, max) in boxes {
                                    emit_box_generic(
                                        &mut builds,
                                        min,
                                        max,
                                        &face_material,
                                        |face| {
                                            let (dx, dy, dz) = face.delta();
                                            let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                            is_occluder(
                                                buf, world, edits, neighbors, reg, here, face, nx, ny, nz,
                                            )
                                        },
                                        |face| {
                                            let lv = light.sample_face_local(x, y, z, face.index());
                                            lv.max(VISUAL_LIGHT_MIN)
                                        },
                                    );
                                }
                            }
                            geist_blocks::types::Shape::Carpet => {
                                let h = 0.0625f32;
                                let min = Vec3 { x: fx, y: fy, z: fz };
                                let max = Vec3 { x: fx + 1.0, y: fy + h, z: fz + 1.0 };
                                let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
                                emit_box_generic(
                                    &mut builds,
                                    min,
                                    max,
                                    &face_material,
                                    |face| {
                                        let (dx, dy, dz) = face.delta();
                                        let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                        is_occluder(
                                            buf, world, edits, neighbors, reg, here, face, nx, ny, nz,
                                        )
                                    },
                                    |face| {
                                        let lv = light.sample_face_local(x, y, z, face.index());
                                        lv.max(VISUAL_LIGHT_MIN)
                                    },
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    let bbox = Aabb { min: Vec3 { x: base_x as f32, y: 0.0, z: base_z as f32 }, max: Vec3 { x: base_x as f32 + sx as f32, y: sy as f32, z: base_z as f32 + sz as f32 } };
    let light_borders = Some(LightBorders::from_grid(&light));
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

/// Build a chunk mesh using Watertight Cubical Complex (WCC) at S=1 (full cubes only).
/// Phase 1: Only full cubes contribute; micro/dynamic shapes are ignored here.
pub fn build_chunk_wcc_cpu_buf(
    buf: &ChunkBuf,
    lighting: Option<&LightingStore>,
    world: &World,
    edits: Option<&HashMap<(i32, i32, i32), Block>>,
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

    let light = match lighting {
        Some(store) => LightGrid::compute_with_borders_buf(buf, store, reg),
        None => return None,
    };

    // Helper to sample a neighbor block at world coords, gated by neighbor-loaded policy.
    let mut get_world_block = |wx: i32, wy: i32, wz: i32| -> Option<Block> {
        if buf.contains_world(wx, wy, wz) {
            if wy < 0 || wy >= sy as i32 { return None; }
            let lx = (wx - base_x) as usize;
            let ly = wy as usize;
            let lz = (wz - base_z) as usize;
            return Some(buf.get_local(lx, ly, lz));
        }
        // Determine neighbor direction to apply loaded gating
        let x0 = base_x;
        let z0 = base_z;
        let x1 = x0 + sx as i32;
        let z1 = z0 + sz as i32;
        let mut neighbor_loaded = false;
        if wx < x0 {
            neighbor_loaded = neighbors.neg_x;
        } else if wx >= x1 {
            neighbor_loaded = neighbors.pos_x;
        } else if wz < z0 {
            neighbor_loaded = neighbors.neg_z;
        } else if wz >= z1 {
            neighbor_loaded = neighbors.pos_z;
        }
        if !neighbor_loaded { return None; }
        let nb = if let Some(es) = edits {
            es.get(&(wx, wy, wz))
                .copied()
                .unwrap_or_else(|| world.block_at_runtime(reg, wx, wy, wz))
        } else {
            world.block_at_runtime(reg, wx, wy, wz)
        };
        Some(nb)
    };

    // face_info closure: returns Some((MaterialId, light)) if a face exists per WCC (S=1 full cubes). 
    let mut face_info = |x: usize, y: usize, z: usize, face: Face, here: Block| -> Option<(MaterialId, u8)> {
        // Only full cubes participate in Phase 1.
        if !is_full_cube(reg, here) { return None; }
        let (dx, dy, dz) = face.delta();
        let gx = base_x + x as i32;
        let gy = y as i32;
        let gz = base_z + z as i32;
        let (nx, ny, nz) = (gx + dx, gy + dy, gz + dz);

        // Ownership rule: do not emit on +X/+Y/+Z boundary planes.
        if !buf.contains_world(nx, ny, nz) {
            // Determine if this is a disallowed + plane; if so, skip.
            if dx > 0 && (gx + 1) == base_x + sx as i32 { return None; }
            if dy > 0 && (gy + 1) == sy as i32 { return None; }
            if dz > 0 && (gz + 1) == base_z + sz as i32 { return None; }
        }

        // Determine neighbor solidity (full cube only). If neighbor outside and not loaded, treat as empty (so faces appear until neighbor arrives).
        let nb_opt = get_world_block(nx, ny, nz);
        let nb_solid = nb_opt.map(|b| is_full_cube(reg, b)).unwrap_or(false);

        // Emit this oriented face only when 'here' is solid and neighbor is not.
        if nb_solid { return None; }
        let mid = registry_material_for_or_unknown(here, face, reg);
        let l = light.sample_face_local(x, y, z, face.index());
        Some((mid, l))
    };

    let flip_v = [false, false, false, false, false, false];
    let mut builds = build_mesh_core(
        buf,
        base_x,
        base_z,
        flip_v,
        Some(VISUAL_LIGHT_MIN),
        |x, y, z, face, here| face_info(x, y, z, face, here),
    );

    let bbox = Aabb { min: Vec3 { x: base_x as f32, y: 0.0, z: base_z as f32 }, max: Vec3 { x: base_x as f32 + sx as f32, y: sy as f32, z: base_z as f32 + sz as f32 } };
    let light_borders = Some(LightBorders::from_grid(&light));
    Some((
        ChunkMeshCPU { cx, cz, bbox, parts: builds },
        light_borders,
    ))
}

pub fn build_voxel_body_cpu_buf(buf: &ChunkBuf, ambient: u8, reg: &BlockRegistry) -> ChunkMeshCPU {
    let base_x = buf.cx * buf.sx as i32;
    let base_z = buf.cz * buf.sz as i32;
    let mut builds: HashMap<MaterialId, MeshBuild> = HashMap::new();
    for z in 0..buf.sz {
        for y in 0..buf.sy {
            for x in 0..buf.sx {
                let here = buf.get_local(x, y, z);
                if !is_solid_runtime(here, reg) {
                    continue;
                }
                let fx = (base_x + x as i32) as f32;
                let fy = y as f32;
                let fz = (base_z + z as i32) as f32;
                let face_material = |face: Face| reg
                    .get(here.id)
                    .map(|ty| ty.material_for_cached(face.role(), here.state))
                    .unwrap_or_else(|| unknown_material_id(reg));
                // Prefer microgrid occupancy for shapes like slabs/stairs
                if let Some(ty) = reg.get(here.id) {
                    let var = ty.variant(here.state);
                    if let Some(occ) = var.occupancy {
                        let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
                        for (min, max) in microgrid_boxes(fx, fy, fz, occ) {
                            emit_box_generic(&mut builds, min, max, &face_material, |_face| false, |_face| ambient);
                        }
                        continue;
                    }
                }
                match reg.get(here.id).map(|t| &t.shape) {
                    Some(geist_blocks::types::Shape::Cube) | Some(geist_blocks::types::Shape::AxisCube { .. }) => {
                        let min = Vec3 { x: fx, y: fy, z: fz };
                        let max = Vec3 { x: fx + 1.0, y: fy + 1.0, z: fz + 1.0 };
                        emit_box_generic(&mut builds, min, max, &face_material, |_face| false, |_face| ambient);
                    }
                    Some(geist_blocks::types::Shape::Slab { .. }) => {
                        let top = is_top_half_shape(here, reg);
                        let h = 0.5f32;
                        let min = Vec3 { x: fx, y: if top { fy + 0.5 } else { fy }, z: fz };
                        let max = Vec3 { x: fx + 1.0, y: if top { fy + 1.0 } else { fy + 0.5 }, z: fz + 1.0 };
                        emit_box_generic(&mut builds, min, max, &face_material, |_face| false, |_face| ambient);
                    }
                    Some(geist_blocks::types::Shape::Pane) => {
                        let t = 0.0625f32;
                        let min = Vec3 { x: fx + 0.5 - t, y: fy, z: fz };
                        let max = Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 1.0 };
                        emit_box_generic(&mut builds, min, max, &face_material, |_face| false, |_face| ambient);
                    }
                    Some(geist_blocks::types::Shape::Fence) => {
                        let t = 0.125f32;
                        let p = 0.375f32;
                        let mut boxes: Vec<(Vec3, Vec3)> = Vec::new();
                        boxes.push((Vec3 { x: fx + 0.5 - t, y: fy, z: fz + 0.5 - t }, Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 0.5 + t }));
                        boxes.push((Vec3 { x: fx, y: fy + p, z: fz + 0.5 - t }, Vec3 { x: fx + 1.0, y: fy + 1.0 - p, z: fz + 0.5 + t }));
                        boxes.push((Vec3 { x: fx + 0.5 - t, y: fy + p, z: fz }, Vec3 { x: fx + 0.5 + t, y: fy + 1.0 - p, z: fz + 1.0 }));
                        for (min, max) in boxes {
                            emit_box_generic(
                                &mut builds,
                                min,
                                max,
                                &face_material,
                                |_face| false,
                                |_face| ambient,
                            );
                        }
                    }
                    Some(geist_blocks::types::Shape::Gate { .. }) => {
                        let t = 0.125f32;
                        let y0 = 0.375f32;
                        let y1 = 0.625f32;
                        let mut boxes: Vec<(Vec3, Vec3)> = Vec::new();
                        boxes.push((Vec3 { x: fx + 0.0, y: fy + y0, z: fz + 0.5 - t }, Vec3 { x: fx + 1.0, y: fy + y0 + t, z: fz + 0.5 + t }));
                        boxes.push((Vec3 { x: fx + 0.0, y: fy + y1, z: fz + 0.5 - t }, Vec3 { x: fx + 1.0, y: fy + y1 + t, z: fz + 0.5 + t }));
                        for (min, max) in boxes {
                            emit_box_generic(&mut builds, min, max, &face_material, |_face| false, |_face| ambient);
                        }
                    }
                    Some(geist_blocks::types::Shape::Carpet) => {
                        let h = 0.0625f32;
                        let min = Vec3 { x: fx, y: fy, z: fz };
                        let max = Vec3 { x: fx + 1.0, y: fy + h, z: fz + 1.0 };
                        emit_box_generic(&mut builds, min, max, &face_material, |_face| false, |_face| ambient);
                    }
                    _ => {}
                }
            }
        }
    }
    ChunkMeshCPU {
        cx: buf.cx,
        cz: buf.cz,
        bbox: Aabb { min: Vec3 { x: base_x as f32, y: 0.0, z: base_z as f32 }, max: Vec3 { x: base_x as f32 + buf.sx as f32, y: buf.sy as f32, z: base_z as f32 + buf.sz as f32 } },
        parts: builds,
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Face {
    PosY = 0,
    NegY = 1,
    PosX = 2,
    NegX = 3,
    PosZ = 4,
    NegZ = 5,
}

impl Face {
    #[inline]
    pub fn index(self) -> usize { self as usize }
    #[inline]
    pub fn from_index(i: usize) -> Face {
        match i { 0 => Face::PosY, 1 => Face::NegY, 2 => Face::PosX, 3 => Face::NegX, 4 => Face::PosZ, 5 => Face::NegZ, _ => Face::PosY }
    }
    #[inline]
    pub fn normal(self) -> Vec3 {
        match self {
            Face::PosY => Vec3 { x: 0.0, y: 1.0, z: 0.0 },
            Face::NegY => Vec3 { x: 0.0, y: -1.0, z: 0.0 },
            Face::PosX => Vec3 { x: 1.0, y: 0.0, z: 0.0 },
            Face::NegX => Vec3 { x: -1.0, y: 0.0, z: 0.0 },
            Face::PosZ => Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            Face::NegZ => Vec3 { x: 0.0, y: 0.0, z: -1.0 },
        }
    }
    #[inline]
    pub fn delta(self) -> (i32, i32, i32) {
        match self {
            Face::PosY => (0, 1, 0),
            Face::NegY => (0, -1, 0),
            Face::PosX => (1, 0, 0),
            Face::NegX => (-1, 0, 0),
            Face::PosZ => (0, 0, 1),
            Face::NegZ => (0, 0, -1),
        }
    }
    #[inline]
    pub fn role(self) -> FaceRole {
        match self {
            Face::PosY => FaceRole::Top,
            Face::NegY => FaceRole::Bottom,
            _ => FaceRole::Side,
        }
    }
}

pub const SIDE_NEIGHBORS: [(i32, i32, Face, f32, f32); 4] = [
    (-1, 0, Face::PosX, 0.0, 0.0),
    (1, 0, Face::NegX, 1.0, 0.0),
    (0, -1, Face::PosZ, 0.0, 0.0),
    (0, 1, Face::NegZ, 0.0, 1.0),
];

#[inline]
pub fn is_full_cube(reg: &BlockRegistry, nb: Block) -> bool {
    reg.get(nb.id)
        .map(|t| t.is_solid(nb.state) && matches!(t.shape, geist_blocks::types::Shape::Cube | geist_blocks::types::Shape::AxisCube { .. }))
        .unwrap_or(false)
}

#[inline]
fn greedy_rects<K: Copy + Eq + Hash>(
    width: usize,
    height: usize,
    mask: &mut [Option<(K, u8)>],
    mut emit: impl FnMut(usize, usize, usize, usize, (K, u8)),
) {
    let mut used = vec![false; width * height];
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let code = mask[idx];
            if code.is_none() || used[idx] { continue; }
            let mut w = 1;
            while x + w < width && mask[y * width + (x + w)] == code && !used[y * width + (x + w)] { w += 1; }
            let mut h = 1;
            'expand: while y + h < height {
                for i in 0..w {
                    let j = (y + h) * width + (x + i);
                    if mask[j] != code || used[j] { break 'expand; }
                }
                h += 1;
            }
            emit(x, y, w, h, code.unwrap());
            for yy in 0..h { for xx in 0..w { used[(y + yy) * width + (x + xx)] = true; } }
        }
    }
}

#[inline]
fn apply_min_light(l: u8, min: Option<u8>) -> u8 { if let Some(m) = min { l.max(m) } else { l } }

pub fn build_mesh_core<K, F>(
    buf: &ChunkBuf,
    base_x: i32,
    base_z: i32,
    flip_v: [bool; 6],
    min_light: Option<u8>,
    mut face_info: F,
) -> HashMap<K, MeshBuild>
where
    K: Copy + Eq + Hash,
    F: FnMut(usize, usize, usize, Face, Block) -> Option<(K, u8)>,
{
    let sx = buf.sx;
    let sy = buf.sy;
    let sz = buf.sz;
    let mut builds: HashMap<K, MeshBuild> = HashMap::new();

    for y in 0..sy {
        let mut mask: Vec<Option<(K, u8)>> = vec![None; sx * sz];
        for z in 0..sz {
            for x in 0..sx {
                let here = buf.get_local(x, y, z);
                if let Some((fm, l)) = face_info(x, y, z, Face::PosY, here) {
                    mask[z * sx + x] = Some((fm, l));
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
            let lv = apply_min_light(codev.1, min_light);
            let rgba = [lv, lv, lv, 255];
            mb.add_face_rect(Face::PosY, Vec3 { x: fx, y: fy, z: fz }, u1, v1, flip_v[Face::PosY.index()], rgba);
        });
    }

    for y in 0..sy {
        let mut mask: Vec<Option<(K, u8)>> = vec![None; sx * sz];
        for z in 0..sz {
            for x in 0..sx {
                let here = buf.get_local(x, y, z);
                if let Some((fm, l)) = face_info(x, y, z, Face::NegY, here) {
                    mask[z * sx + x] = Some((fm, l));
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
            let lv = apply_min_light(codev.1, min_light);
            let rgba = [lv, lv, lv, 255];
            mb.add_face_rect(Face::NegY, Vec3 { x: fx, y: fy, z: fz }, u1, v1, flip_v[Face::NegY.index()], rgba);
        });
    }

    for x in 0..sx {
        for &pos in &[false, true] {
            let mut mask: Vec<Option<(K, u8)>> = vec![None; sz * sy];
            for z in 0..sz {
                for y in 0..sy {
                    let here = buf.get_local(x, y, z);
                    let face = if pos { Face::PosX } else { Face::NegX };
                    if let Some((fm, l)) = face_info(x, y, z, face, here) {
                        mask[y * sz + z] = Some((fm, l));
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
                let lv = apply_min_light(codev.1, min_light);
                let rgba = [lv, lv, lv, 255];
                let face = if pos { Face::PosX } else { Face::NegX };
                mb.add_face_rect(face, Vec3 { x: fx, y: fy, z: fz }, u1, v1, flip_v[face.index()], rgba);
            });
        }
    }

    for z in 0..sz {
        for &pos in &[false, true] {
            let mut mask: Vec<Option<(K, u8)>> = vec![None; sx * sy];
            for x in 0..sx {
                for y in 0..sy {
                    let here = buf.get_local(x, y, z);
                    let face = if pos { Face::PosZ } else { Face::NegZ };
                    if let Some((fm, l)) = face_info(x, y, z, face, here) {
                        mask[y * sx + x] = Some((fm, l));
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
                let lv = apply_min_light(codev.1, min_light);
                let rgba = [lv, lv, lv, 255];
                let face = if pos { Face::PosZ } else { Face::NegZ };
                mb.add_face_rect(face, Vec3 { x: fx, y: fy, z: fz }, u1, v1, flip_v[face.index()], rgba);
            });
        }
    }

    builds
}

use crate::blocks::{Block, BlockRegistry, FaceRole, MaterialCatalog, MaterialId, Shape};
use crate::chunkbuf::ChunkBuf;
use crate::lighting::{LightBorders, LightGrid, LightingStore};
use crate::microgrid_tables::{empty4_to_rects, occ8_to_boxes};
use crate::texture_cache::TextureCache;
use crate::voxel::World;
use raylib::core::math::BoundingBox;
use geist_geom::{Aabb, Vec3};
use geist_render_raylib::conv::aabb_to_rl;
use raylib::prelude::*;
use std::collections::HashMap;
use std::hash::Hash;

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
            base as u16,
            (base + 1) as u16,
            (base + 2) as u16,
            base as u16,
            (base + 2) as u16,
            (base + 3) as u16,
        ]);
    }

    #[inline]
    pub(crate) fn add_face_rect(
        &mut self,
        face: Face,
        origin: Vector3,
        u1: f32,
        v1: f32,
        flip_v: bool,
        rgba: [u8; 4],
    ) {
        let n = face.normal();
        let (a, b, c, d) = match face {
            // +Y: origin=(x,y,z), u is +X, v is +Z
            Face::PosY => (
                Vector3::new(origin.x, origin.y, origin.z),
                Vector3::new(origin.x + u1, origin.y, origin.z),
                Vector3::new(origin.x + u1, origin.y, origin.z + v1),
                Vector3::new(origin.x, origin.y, origin.z + v1),
            ),
            // -Y: origin=(x,y,z), u is +X, v is +Z but flipped on Z
            Face::NegY => (
                Vector3::new(origin.x, origin.y, origin.z + v1),
                Vector3::new(origin.x + u1, origin.y, origin.z + v1),
                Vector3::new(origin.x + u1, origin.y, origin.z),
                Vector3::new(origin.x, origin.y, origin.z),
            ),
            // +X: origin=(x,y,z), u is +Z, v is +Y
            Face::PosX => (
                Vector3::new(origin.x, origin.y + v1, origin.z + u1),
                Vector3::new(origin.x, origin.y + v1, origin.z),
                Vector3::new(origin.x, origin.y, origin.z),
                Vector3::new(origin.x, origin.y, origin.z + u1),
            ),
            // -X: origin=(x,y,z), u is +Z, v is +Y
            Face::NegX => (
                Vector3::new(origin.x, origin.y + v1, origin.z),
                Vector3::new(origin.x, origin.y + v1, origin.z + u1),
                Vector3::new(origin.x, origin.y, origin.z + u1),
                Vector3::new(origin.x, origin.y, origin.z),
            ),
            // +Z: origin=(x,y,z), u is +X, v is +Y
            Face::PosZ => (
                Vector3::new(origin.x + u1, origin.y + v1, origin.z),
                Vector3::new(origin.x, origin.y + v1, origin.z),
                Vector3::new(origin.x, origin.y, origin.z),
                Vector3::new(origin.x + u1, origin.y, origin.z),
            ),
            // -Z: origin=(x,y,z), u is +X, v is +Y
            Face::NegZ => (
                Vector3::new(origin.x, origin.y + v1, origin.z),
                Vector3::new(origin.x + u1, origin.y + v1, origin.z),
                Vector3::new(origin.x + u1, origin.y, origin.z),
                Vector3::new(origin.x, origin.y, origin.z),
            ),
        };
        self.add_quad(a, b, c, d, n, u1, v1, flip_v, rgba);
    }

    // Test-only accessors
    #[inline]
    pub(crate) fn positions(&self) -> &[f32] { &self.pos }
    #[inline]
    pub(crate) fn normals(&self) -> &[f32] { &self.norm }
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

// Face helpers moved to crate::meshutil

#[inline]
fn emit_face_rect_for(
    builds: &mut std::collections::HashMap<MaterialId, MeshBuild>,
    mid: MaterialId,
    face: Face,
    origin: Vector3,
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

// --- Generic neighbor fixups from micro-grid occupancy ------------------------

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
        // Determine neighbor eligibility and occupancy mask. Full cubes act like fully-occupied (all 8 cells).
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
        if !neighbor_ok { continue; }
        let mid = registry_material_for_or_unknown(nb, face, reg);
        match face {
            Face::PosX | Face::NegX => {
                // Boundary at fixed x (bx) across (v=ly, u=lz)
                let bx = if dx < 0 { 0 } else { 1 };
                let nb_bx = match face { Face::PosX => 1, Face::NegX => 0, _ => bx };
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
                    let origin = Vector3::new(fx + x_off, fy + v0, fz + u0);
                    emit_face_rect_for(builds, mid, face, origin, du, dv, rgba);
                }
            }
            Face::PosZ | Face::NegZ => {
                // Boundary at fixed z (bz) across (v=ly, u=lx)
                let bz = if dz < 0 { 0 } else { 1 };
                let nb_bz = match face { Face::PosZ => 1, Face::NegZ => 0, _ => bz };
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
                    let origin = Vector3::new(fx + u0, fy + v0, fz + z_off);
                    emit_face_rect_for(builds, mid, face, origin, du, dv, rgba);
                }
            }
            _ => {}
        }
    }

    // Vertical neighbors (±Y): project empty cells on top/bottom boundary onto neighbor faces.
    let sy = buf.sy as i32;
    for &(dy, face) in &[(-1, Face::PosY), (1, Face::NegY)] {
        let ny = y as i32 + dy;
        if ny < 0 || ny >= sy {
            continue;
        }
        let nb = buf.get_local(x, ny as usize, z);
        // Determine neighbor eligibility and occupancy (full cubes => fully occupied)
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
        if !neighbor_ok { continue; }
        let mid = registry_material_for_or_unknown(nb, face, reg);
        // Boundary at fixed y (ly) across (v=lz, u=lx)
        let ly = if dy < 0 { 0 } else { 1 } as usize;
        let nb_ly = match face { Face::PosY => 1usize, Face::NegY => 0usize, _ => ly };
        let mut mask: u8 = 0;
        for lz in 0..2 {
            for lx in 0..2 {
                let empty_here = (occ & micro_bit(lx, ly, lz)) == 0;
                let nb_has = (nb_occ & micro_bit(lx, nb_ly, lz)) != 0;
                if empty_here && nb_has {
                    let bit = ((lz << 1) | lx) as u8; // v=lz, u=lx
                    mask |= 1u8 << bit;
                }
            }
        }
        for r in empty4_to_rects(mask) {
            let u0 = r[0] as f32 * cell; // x
            let v0 = r[1] as f32 * cell; // z
            let du = r[2] as f32 * cell;
            let dv = r[3] as f32 * cell;
            let y0 = if dy < 0 { fy } else { fy + 1.0 };
            let draw_top = dy > 0; // matches previous ly==1 logic
            let lv = light_for_neighbor(x, ny as usize, z, face, draw_top);
            let rgba = [lv, lv, lv, 255];
            let origin = Vector3::new(fx + u0, y0, fz + v0);
            emit_face_rect_for(builds, mid, face, origin, du, dv, rgba);
        }
    }
}

// Table-driven stairs/slab fixups replaced by micro-grid neighbor projection.

// Replaced by micro-grid boxes above.

#[inline]
fn is_solid_runtime(b: Block, reg: &BlockRegistry) -> bool {
    reg.get(b.id)
        .map(|ty| ty.is_solid(b.state))
        .unwrap_or(false)
}

// Property decoding now lives on BlockType via registry (see state_prop_value/state_prop_is_value)

#[inline]
fn is_top_half_shape(b: Block, reg: &BlockRegistry) -> bool {
    reg.get(b.id).map_or(false, |ty| match &ty.shape {
        crate::blocks::Shape::Slab { half_from }
        | crate::blocks::Shape::Stairs { half_from, .. } => {
            ty.state_prop_is_value(b.state, half_from, "top")
        }
        _ => false,
    })
}

// Legacy world mapping removed; mesher queries runtime worldgen directly when needed.

// Local path uses the same generic slab fixups with a different light function.

// Local path uses the generic stairs fixups with a different light function.

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
    min: Vector3,
    max: Vector3,
    mut choose: impl FnMut(Face) -> Option<(MaterialId, [u8; 4])>,
) {
    const FACE_DATA: [(Face, [usize; 4], (f32, f32, f32)); 6] = [
        (Face::PosY, [0, 2, 6, 4], (0.0, 1.0, 0.0)), // +Y: min.x,max.y,min.z -> max.x,max.y,max.z
        (Face::NegY, [5, 7, 3, 1], (0.0, -1.0, 0.0)), // -Y: min.x,min.y,max.z -> max.x,min.y,min.z
        (Face::PosX, [6, 2, 3, 7], (1.0, 0.0, 0.0)), // +X: max.x,max.y,max.z -> max.x,min.y,min.z
        (Face::NegX, [0, 4, 5, 1], (-1.0, 0.0, 0.0)), // -X: min.x,max.y,min.z -> min.x,min.y,max.z
        (Face::PosZ, [4, 6, 7, 5], (0.0, 0.0, 1.0)), // +Z: min.x,max.y,max.z -> max.x,min.y,max.z
        (Face::NegZ, [2, 0, 1, 3], (0.0, 0.0, -1.0)), // -Z: max.x,max.y,min.z -> min.x,min.y,min.z
    ];

    let corners = [
        Vector3::new(min.x, max.y, min.z), // 0
        Vector3::new(min.x, min.y, min.z), // 1
        Vector3::new(max.x, max.y, min.z), // 2
        Vector3::new(max.x, min.y, min.z), // 3
        Vector3::new(min.x, max.y, max.z), // 4
        Vector3::new(min.x, min.y, max.z), // 5
        Vector3::new(max.x, max.y, max.z), // 6
        Vector3::new(max.x, min.y, max.z), // 7
    ];

    for &(face, indices, normal) in &FACE_DATA {
        if let Some((mid, rgba)) = choose(face) {
            let (u1, v1) = match face {
                Face::PosY | Face::NegY => (max.x - min.x, max.z - min.z),
                Face::PosX | Face::NegX => (max.z - min.z, max.y - min.y),
                Face::PosZ | Face::NegZ => (max.x - min.x, max.y - min.y),
            };
            builds.entry(mid).or_default().add_quad(
                corners[indices[0]],
                corners[indices[1]],
                corners[indices[2]],
                corners[indices[3]],
                Vector3::new(normal.0, normal.1, normal.2),
                u1,
                v1,
                false,
                rgba,
            );
        }
    }
}

#[inline]
fn emit_box_generic(
    builds: &mut std::collections::HashMap<MaterialId, MeshBuild>,
    min: Vector3,
    max: Vector3,
    fm_for_face: &dyn Fn(Face) -> MaterialId,
    mut occludes: impl FnMut(Face) -> bool,
    mut sample_light: impl FnMut(Face) -> u8,
) {
    emit_box_faces(builds, min, max, |face| {
        if occludes(face) {
            return None;
        }
        let lv = sample_light(face);
        let rgba = [lv, lv, lv, 255];
        let mid = fm_for_face(face);
        Some((mid, rgba))
    });
}

// world-based occluder test removed; occlusion uses only local chunk buffers.

// --- 2x2x2 micro-grid shapes -------------------------------------------------

#[inline]
fn micro_bit(x: usize, y: usize, z: usize) -> u8 {
    // index bits: x (bit0), z (bit1), y (bit2) => idx = (y<<2) | (z<<1) | x
    1u8 << (((y & 1) << 2) | ((z & 1) << 1) | (x & 1))
}

// micro_occupancy_* moved to registry precompute (ShapeVariant)

#[inline]
fn microgrid_boxes(fx: f32, fy: f32, fz: f32, occ: u8) -> Vec<(Vector3, Vector3)> {
    let cell = 0.5f32;
    let mut out = Vec::new();
    for b in occ8_to_boxes(occ) {
        let min = Vector3::new(
            fx + (b[0] as f32) * cell,
            fy + (b[1] as f32) * cell,
            fz + (b[2] as f32) * cell,
        );
        let max = Vector3::new(
            fx + (b[3] as f32) * cell,
            fy + (b[4] as f32) * cell,
            fz + (b[5] as f32) * cell,
        );
        out.push((min, max));
    }
    out
}

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
        // Seam policy: optionally avoid occluding against the same type
        if let (Some(h), Some(_n)) = (reg.get(here.id), reg.get(nb.id)) {
            if h.seam.dont_occlude_same && here.id == nb.id {
                return false;
            }
        }
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

// No legacy mapping helpers; all block resolution is via registry-backed runtime Block.

pub struct ChunkRender {
    #[allow(dead_code)]
    pub cx: i32,
    #[allow(dead_code)]
    pub cz: i32,
    pub bbox: BoundingBox,
    pub parts: Vec<(MaterialId, raylib::core::models::Model)>,
    pub leaf_tint: Option<[f32; 3]>,
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

    // Unified path via meshing_core
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
            // Skip micro-grid shapes; they are handled in a dedicated pass.
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
            // Resolve material via registry; fallback to unknown when unmapped
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
    // Special-shapes pass: micro-grid shapes from precomputed variants
    for z in 0..sz {
        for y in 0..sy {
            for x in 0..sx {
                let b = buf.get_local(x, y, z);
                if let Some(ty) = reg.get(b.id) {
                    let var = ty.variant(b.state);
                    if let Some(occ) = var.occupancy {
                        let fx = base_x as f32 + x as f32;
                        let fy = y as f32;
                        let fz = base_z as f32 + z as f32;
                        let gx = base_x + x as i32;
                        let gy = y as i32;
                        let gz = base_z + z as i32;
                        let here = buf.get_local(x, y, z);
                        let face_material = |face: Face| ty.material_for_cached(face.role(), b.state);
                        for (min, max) in microgrid_boxes(fx, fy, fz, occ) {
                            emit_box_generic(
                                &mut builds,
                                min,
                                max,
                                &face_material,
                                |face| {
                                    // Micro-accurate occlusion against neighbor occupancy or full cubes
                                    let (dx, dy, dz) = face.delta();
                                    let (nx, ny, nz) = (gx + dx, gy + dy, gz + dz);
                                    // Resolve neighbor block with neighbor-loaded guard across chunk borders
                                    let mut neighbor_loaded = true;
                                    if !buf.contains_world(nx, ny, nz) {
                                        neighbor_loaded = match (dx, dy, dz) {
                                            (-1, 0, 0) => neighbors.neg_x,
                                            (1, 0, 0) => neighbors.pos_x,
                                            (0, 0, -1) => neighbors.neg_z,
                                            (0, 0, 1) => neighbors.pos_z,
                                            _ => true,
                                        };
                                    }
                                    if !neighbor_loaded {
                                        return false;
                                    }
                                    let nb = if buf.contains_world(nx, ny, nz) {
                                        let lx = (nx - base_x) as usize;
                                        let ly = ny as usize;
                                        let lz = (nz - base_z) as usize;
                                        buf.get_local(lx, ly, lz)
                                    } else {
                                        if let Some(es) = edits {
                                            es.get(&(nx, ny, nz)).copied().unwrap_or_else(|| {
                                                world.block_at_runtime(reg, nx, ny, nz)
                                            })
                                        } else {
                                            world.block_at_runtime(reg, nx, ny, nz)
                                        }
                                    };
                                    // Seam policy: don't occlude with same ID if configured
                                    if let (Some(h), Some(nbt)) = (reg.get(here.id), reg.get(nb.id)) {
                                        if h.seam.dont_occlude_same && here.id == nb.id {
                                            return false;
                                        }
                                        // Full cubes occlude fully
                                        if matches!(nbt.shape, Shape::Cube | Shape::AxisCube { .. }) && nbt.is_solid(nb.state) {
                                            return true;
                                        }
                                        // Micro-grid neighbor: occlude only if neighbor occupancy fully covers this face area
                                        if let Some(nb_occ) = nbt.variant(nb.state).occupancy {
                                            // Compute local [0,2] micro ranges on the face plane
                                            let mut range_uv = |a0: f32, a1: f32| -> (usize, usize) {
                                                let r0 = a0.max(0.0).min(1.0);
                                                let r1 = a1.max(0.0).min(1.0);
                                                let s = if (r0 - 0.0).abs() < 1e-4 { 0 } else { 1 };
                                                let e = if (r1 - 1.0).abs() < 1e-4 { 2 } else { 1 };
                                                (s, e)
                                            };
                                            let fully = match face {
                                                Face::PosX => {
                                                    let (ys, ye) = range_uv(min.y - fy, max.y - fy);
                                                    let (zs, ze) = range_uv(min.z - fz, max.z - fz);
                                                    let bx = 0usize; // neighbor's x micro on boundary
                                                    (ys..ye).all(|ly| (zs..ze).all(|lz| (nb_occ & micro_bit(bx, ly, lz)) != 0))
                                                }
                                                Face::NegX => {
                                                    let (ys, ye) = range_uv(min.y - fy, max.y - fy);
                                                    let (zs, ze) = range_uv(min.z - fz, max.z - fz);
                                                    let bx = 1usize;
                                                    (ys..ye).all(|ly| (zs..ze).all(|lz| (nb_occ & micro_bit(bx, ly, lz)) != 0))
                                                }
                                                Face::PosZ => {
                                                    let (ys, ye) = range_uv(min.y - fy, max.y - fy);
                                                    let (xs, xe) = range_uv(min.x - fx, max.x - fx);
                                                    let bz = 0usize;
                                                    (ys..ye).all(|ly| (xs..xe).all(|lx| (nb_occ & micro_bit(lx, ly, bz)) != 0))
                                                }
                                                Face::NegZ => {
                                                    let (ys, ye) = range_uv(min.y - fy, max.y - fy);
                                                    let (xs, xe) = range_uv(min.x - fx, max.x - fx);
                                                    let bz = 1usize;
                                                    (ys..ye).all(|ly| (xs..xe).all(|lx| (nb_occ & micro_bit(lx, ly, bz)) != 0))
                                                }
                                                Face::PosY => {
                                                    let (zs, ze) = range_uv(min.z - fz, max.z - fz);
                                                    let (xs, xe) = range_uv(min.x - fx, max.x - fx);
                                                    let by = 0usize;
                                                    (zs..ze).all(|lz| (xs..xe).all(|lx| (nb_occ & micro_bit(lx, by, lz)) != 0))
                                                }
                                                Face::NegY => {
                                                    let (zs, ze) = range_uv(min.z - fz, max.z - fz);
                                                    let (xs, xe) = range_uv(min.x - fx, max.x - fx);
                                                    let by = 1usize;
                                                    (zs..ze).all(|lz| (xs..xe).all(|lx| (nb_occ & micro_bit(lx, by, lz)) != 0))
                                                }
                                            };
                                            if fully { return true; }
                                        }
                                    }
                                    false
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
                            |nx, ny, nz, face, draw_top| {
                                sample_neighbor_half_light(
                                    &light, nx, ny, nz, face, draw_top, sy,
                                )
                            },
                        );
                    } else if let Some(dyns) = var.dynamic {
                        let fx = base_x as f32 + x as f32;
                        let fy = y as f32;
                        let fz = base_z as f32 + z as f32;
                        let gx = base_x + x as i32;
                        let gy = y as i32;
                        let gz = base_z + z as i32;
                        let here = buf.get_local(x, y, z);
                        let face_material = |face: Face| ty.material_for_cached(face.role(), b.state);
                        match dyns {
                            crate::blocks::registry::DynamicShape::Pane => {
                                // Connectivity: 4-bit mask W,E,N,S
                                let mut mask: u8 = 0;
                                let dirs = [
                                    (-1, 0, 0u8), // W
                                    (1, 0, 1u8),  // E
                                    (0, -1, 2u8), // N
                                    (0, 1, 3u8),  // S
                                ];
                                for (dx, dz, bit) in dirs {
                                    let nx = gx + dx;
                                    let nz = gz + dz;
                                    let ny = gy;
                                    let mut connected = false;
                                    if buf.contains_world(nx, ny, nz) {
                                        let lx = (nx - base_x) as usize;
                                        let ly = ny as usize;
                                        let lz = (nz - base_z) as usize;
                                        let nb = buf.get_local(lx, ly, lz);
                                        connected = reg
                                            .get(nb.id)
                                            .map(|t| matches!(t.shape, crate::blocks::Shape::Pane) || is_full_cube(reg, nb))
                                            .unwrap_or(false);
                                    } else {
                                        // Cross-chunk: only connect when neighbor chunk loaded and block is connectable
                                        let x0 = buf.cx * buf.sx as i32;
                                        let z0 = buf.cz * buf.sz as i32;
                                        let x1 = x0 + buf.sx as i32;
                                        let z1 = z0 + buf.sz as i32;
                                        let mut neighbor_loaded = false;
                                        if nx < x0 { neighbor_loaded = neighbors.neg_x; }
                                        else if nx >= x1 { neighbor_loaded = neighbors.pos_x; }
                                        else if nz < z0 { neighbor_loaded = neighbors.neg_z; }
                                        else if nz >= z1 { neighbor_loaded = neighbors.pos_z; }
                                        if neighbor_loaded {
                                            let nb = if let Some(es) = edits {
                                                es.get(&(nx, ny, nz)).copied().unwrap_or_else(|| world.block_at_runtime(reg, nx, ny, nz))
                                            } else {
                                                world.block_at_runtime(reg, nx, ny, nz)
                                            };
                                            connected = reg
                                                .get(nb.id)
                                                .map(|t| matches!(t.shape, crate::blocks::Shape::Pane) || is_full_cube(reg, nb))
                                                .unwrap_or(false);
                                        }
                                    }
                                    if connected { mask |= 1u8 << bit; }
                                }
                                let t = 0.25f32;
                                // Generate boxes per mask
                                let mut boxes: Vec<(Vector3, Vector3)> = Vec::new();
                                if mask == 0 {
                                    // Cross
                                    boxes.push((
                                        Vector3::new(fx + 0.0, fy + 0.0, fz + 0.5 - t * 0.5),
                                        Vector3::new(fx + 1.0, fy + 1.0, fz + 0.5 + t * 0.5),
                                    ));
                                    boxes.push((
                                        Vector3::new(fx + 0.5 - t * 0.5, fy + 0.0, fz + 0.0),
                                        Vector3::new(fx + 0.5 + t * 0.5, fy + 1.0, fz + 1.0),
                                    ));
                                } else {
                                    if (mask & (1 << 0)) != 0 {
                                        boxes.push((
                                            Vector3::new(fx + 0.0, fy + 0.0, fz + 0.5 - t * 0.5),
                                            Vector3::new(fx + 0.5, fy + 1.0, fz + 0.5 + t * 0.5),
                                        ));
                                    }
                                    if (mask & (1 << 1)) != 0 {
                                        boxes.push((
                                            Vector3::new(fx + 0.5, fy + 0.0, fz + 0.5 - t * 0.5),
                                            Vector3::new(fx + 1.0, fy + 1.0, fz + 0.5 + t * 0.5),
                                        ));
                                    }
                                    if (mask & (1 << 2)) != 0 {
                                        boxes.push((
                                            Vector3::new(fx + 0.5 - t * 0.5, fy + 0.0, fz + 0.0),
                                            Vector3::new(fx + 0.5 + t * 0.5, fy + 1.0, fz + 0.5),
                                        ));
                                    }
                                    if (mask & (1 << 3)) != 0 {
                                        boxes.push((
                                            Vector3::new(fx + 0.5 - t * 0.5, fy + 0.0, fz + 0.5),
                                            Vector3::new(fx + 0.5 + t * 0.5, fy + 1.0, fz + 1.0),
                                        ));
                                    }
                                }
                                for (min, max) in boxes {
                                    emit_box_generic(
                                        &mut builds,
                                        min,
                                        max,
                                        &face_material,
                                        |face| {
                                            let (dx, dy, dz) = face.delta();
                                            let (nx, ny, nz) = (gx + dx, gy + dy, gz + dz);
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
                            crate::blocks::registry::DynamicShape::Fence => {
                                // Connectivity: 4-bit mask W,E,N,S (connect to same fence or full cube)
                                let mut mask: u8 = 0;
                                let dirs = [
                                    (-1, 0, 0u8), // W
                                    (1, 0, 1u8),  // E
                                    (0, -1, 2u8), // N
                                    (0, 1, 3u8),  // S
                                ];
                                for (dx, dz, bit) in dirs {
                                    let nx = gx + dx;
                                    let nz = gz + dz;
                                    let ny = gy;
                                    let mut connected = false;
                                    if buf.contains_world(nx, ny, nz) {
                                        let lx = (nx - base_x) as usize;
                                        let ly = ny as usize;
                                        let lz = (nz - base_z) as usize;
                                        let nb = buf.get_local(lx, ly, lz);
                                        connected = reg
                                            .get(nb.id)
                                            .map(|t| matches!(t.shape, crate::blocks::Shape::Fence) || is_full_cube(reg, nb))
                                            .unwrap_or(false);
                                    } else {
                                        let x0 = buf.cx * buf.sx as i32;
                                        let z0 = buf.cz * buf.sz as i32;
                                        let x1 = x0 + buf.sx as i32;
                                        let z1 = z0 + buf.sz as i32;
                                        let mut neighbor_loaded = false;
                                        if nx < x0 { neighbor_loaded = neighbors.neg_x; }
                                        else if nx >= x1 { neighbor_loaded = neighbors.pos_x; }
                                        else if nz < z0 { neighbor_loaded = neighbors.neg_z; }
                                        else if nz >= z1 { neighbor_loaded = neighbors.pos_z; }
                                        if neighbor_loaded {
                                            let nb = if let Some(es) = edits {
                                                es.get(&(nx, ny, nz)).copied().unwrap_or_else(|| world.block_at_runtime(reg, nx, ny, nz))
                                            } else {
                                                world.block_at_runtime(reg, nx, ny, nz)
                                            };
                                            connected = reg
                                                .get(nb.id)
                                                .map(|t| matches!(t.shape, crate::blocks::Shape::Fence) || is_full_cube(reg, nb))
                                                .unwrap_or(false);
                                        }
                                    }
                                    if connected { mask |= 1u8 << bit; }
                                }
                                let p = 0.25f32; // post thickness
                                let t = 0.25f32; // arm thickness
                                let mid_y0 = 0.5 - t * 0.5;
                                let mid_y1 = 0.5 + t * 0.5;
                                let mut boxes: Vec<(Vector3, Vector3)> = Vec::new();
                                // center post
                                boxes.push((
                                    Vector3::new(fx + 0.5 - p * 0.5, fy + 0.0, fz + 0.5 - p * 0.5),
                                    Vector3::new(fx + 0.5 + p * 0.5, fy + 1.0, fz + 0.5 + p * 0.5),
                                ));
                                // arms
                                if (mask & (1 << 0)) != 0 {
                                    boxes.push((
                                        Vector3::new(fx + 0.0, fy + mid_y0, fz + 0.5 - t * 0.5),
                                        Vector3::new(fx + 0.5, fy + mid_y1, fz + 0.5 + t * 0.5),
                                    ));
                                }
                                if (mask & (1 << 1)) != 0 {
                                    boxes.push((
                                        Vector3::new(fx + 0.5, fy + mid_y0, fz + 0.5 - t * 0.5),
                                        Vector3::new(fx + 1.0, fy + mid_y1, fz + 0.5 + t * 0.5),
                                    ));
                                }
                                if (mask & (1 << 2)) != 0 {
                                    boxes.push((
                                        Vector3::new(fx + 0.5 - t * 0.5, fy + mid_y0, fz + 0.0),
                                        Vector3::new(fx + 0.5 + t * 0.5, fy + mid_y1, fz + 0.5),
                                    ));
                                }
                                if (mask & (1 << 3)) != 0 {
                                    boxes.push((
                                        Vector3::new(fx + 0.5 - t * 0.5, fy + mid_y0, fz + 0.5),
                                        Vector3::new(fx + 0.5 + t * 0.5, fy + mid_y1, fz + 1.0),
                                    ));
                                }
                                for (min, max) in boxes {
                                    emit_box_generic(
                                        &mut builds,
                                        min,
                                        max,
                                        &face_material,
                                        |face| {
                                            let (dx, dy, dz) = face.delta();
                                            let (nx, ny, nz) = (gx + dx, gy + dy, gz + dz);
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
                            crate::blocks::registry::DynamicShape::Gate => {
                                // fence gate: two rails; orientation depends on facing/open
                                let mut along_x = true; // rails extend along X when closed facing N/S
                                if let crate::blocks::Shape::Gate { facing_from, open_from } = &ty.shape {
                                    let facing = ty.state_prop_value(b.state, facing_from).unwrap_or("north");
                                    along_x = matches!(facing, "north" | "south");
                                    if ty.state_prop_is_value(b.state, open_from, "true") {
                                        along_x = !along_x; // swing open 90°
                                    }
                                }
                                let t = 0.125f32; // rail thickness
                                let y0 = 0.375f32;
                                let y1 = 0.625f32;
                                let mut boxes: Vec<(Vector3, Vector3)> = Vec::new();
                                if along_x {
                                    boxes.push((Vector3::new(fx + 0.0, fy + y0, fz + 0.5 - t), Vector3::new(fx + 1.0, fy + y0 + t, fz + 0.5 + t)));
                                    boxes.push((Vector3::new(fx + 0.0, fy + y1, fz + 0.5 - t), Vector3::new(fx + 1.0, fy + y1 + t, fz + 0.5 + t)));
                                } else {
                                    boxes.push((Vector3::new(fx + 0.5 - t, fy + y0, fz + 0.0), Vector3::new(fx + 0.5 + t, fy + y0 + t, fz + 1.0)));
                                    boxes.push((Vector3::new(fx + 0.5 - t, fy + y1, fz + 0.0), Vector3::new(fx + 0.5 + t, fy + y1 + t, fz + 1.0)));
                                }
                                for (min, max) in boxes {
                                    emit_box_generic(
                                        &mut builds,
                                        min,
                                        max,
                                        &face_material,
                                        |face| {
                                            let (dx, dy, dz) = face.delta();
                                            let (nx, ny, nz) = (gx + dx, gy + dy, gz + dz);
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
                            crate::blocks::registry::DynamicShape::Carpet => {
                                let h = 0.0625f32; // 1/16
                                let min = Vector3::new(fx, fy, fz);
                                let max = Vector3::new(fx + 1.0, fy + h, fz + 1.0);
                                emit_box_generic(
                                    &mut builds,
                                    min,
                                    max,
                                    &face_material,
                                    |face| {
                                        let (dx, dy, dz) = face.delta();
                                        let (nx, ny, nz) = (gx + dx, gy + dy, gz + dz);
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
                    }
                }
            }
        }
    }
    let bbox = Aabb {
        min: Vec3 { x: base_x as f32, y: 0.0, z: base_z as f32 },
        max: Vec3 {
            x: base_x as f32 + sx as f32,
            y: sy as f32,
            z: base_z as f32 + sz as f32,
        },
    };
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
        if total_verts == 0 {
            continue;
        }
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
                std::ptr::copy_nonoverlapping(
                    mb.pos[pos_start..pos_end].as_ptr(),
                    raw.vertices,
                    v_count * 3,
                );
                std::ptr::copy_nonoverlapping(
                    mb.norm[norm_start..norm_end].as_ptr(),
                    raw.normals,
                    v_count * 3,
                );
                std::ptr::copy_nonoverlapping(
                    mb.uv[uv_start..uv_end].as_ptr(),
                    raw.texcoords,
                    v_count * 2,
                );
                std::ptr::copy_nonoverlapping(
                    mb.col[col_start..col_end].as_ptr(),
                    raw.colors,
                    v_count * 4,
                );

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
            unsafe {
                mesh.upload(false);
            }
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
        bbox: aabb_to_rl(cpu.bbox),
        parts: parts_gpu,
        leaf_tint: None,
    })
}

// Purged world-based synchronous build path; buffer-based pipeline is authoritative.

// Simple per-app texture cache keyed by file path; loads each texture once and reuses it across chunks.
// Local-body mesher: emits vertices in local-space [0..sx, 0..sz], no world/lighting deps.
pub fn build_voxel_body_cpu_buf(buf: &ChunkBuf, ambient: u8, reg: &BlockRegistry) -> ChunkMeshCPU {
    // Unified path via meshing_core + special-shapes pass to match world mesher

    // Match world mesher V orientation for all faces
    let flip_v = [false, false, false, false, false, false];

    // Skip non-cubic shapes in greedy pass; they are emitted below
    let mut builds = build_mesh_core(
        buf,
        0,
        0,
        flip_v,
        None,
        |x, y, z, face: Face, here| {
            if !is_solid_runtime(here, reg) {
                return None;
            }
            if let Some(ty) = reg.get(here.id) {
                if ty.variant(here.state).occupancy.is_some() {
                    return None;
                }
            }
            let (dx, dy, dz) = face.delta();
            let (nx, ny, nz) = (x as i32 + dx, y as i32 + dy, z as i32 + dz);
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
        },
    );

    // Helpers for special-shapes pass
    #[inline]
    fn occludes_local(
        buf: &ChunkBuf,
        x: i32,
        y: i32,
        z: i32,
        face: Face,
        reg: &BlockRegistry,
    ) -> bool {
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
    #[allow(clippy::too_many_arguments)]
    fn emit_box_local(
        builds: &mut std::collections::HashMap<MaterialId, MeshBuild>,
        buf: &ChunkBuf,
        reg: &BlockRegistry,
        x: usize,
        y: usize,
        z: usize,
        face_material: &dyn Fn(Face) -> MaterialId,
        min: Vector3,
        max: Vector3,
        ambient: u8,
    ) {
        let gx = x as i32;
        let gy = y as i32;
        let gz = z as i32;
        emit_box_faces(builds, min, max, |face| {
            let (dx, dy, dz) = face.delta();
            let (nx, ny, nz) = (gx + dx, gy + dy, gz + dz);
            if occludes_local(buf, nx, ny, nz, face, reg) {
                return None;
            }
            let lv = face_light(face, ambient);
            let rgba = [lv, lv, lv, 255];
            let mid = face_material(face);
            Some((mid, rgba))
        });
    }

    // Special-shapes pass: micro-grid shapes from precomputed variants
    let sx = buf.sx;
    let sy = buf.sy;
    let sz = buf.sz;
    for z in 0..sz {
        for y in 0..sy {
            for x in 0..sx {
                let b = buf.get_local(x, y, z);
                if let Some(ty) = reg.get(b.id) {
                    let var = ty.variant(b.state);
                    if let Some(occ) = var.occupancy {
                        let fx = x as f32;
                        let fy = y as f32;
                        let fz = z as f32;
                        let face_material = |face: Face| ty.material_for_cached(face.role(), b.state);
                        for (min, max) in microgrid_boxes(fx, fy, fz, occ) {
                            // Custom micro-accurate occlusion closure
                            let occludes = |face: Face| {
                                let (dx, dy, dz) = face.delta();
                                let (nx, ny, nz) = (x as i32 + dx, y as i32 + dy, z as i32 + dz);
                                if nx < 0 || ny < 0 || nz < 0 { return false; }
                                let (xu, yu, zu) = (nx as usize, ny as usize, nz as usize);
                                if xu >= buf.sx || yu >= buf.sy || zu >= buf.sz { return false; }
                                let nb = buf.get_local(xu, yu, zu);
                                if let (Some(h), Some(nbt)) = (reg.get(b.id), reg.get(nb.id)) {
                                    if h.seam.dont_occlude_same && b.id == nb.id { return false; }
                                    if matches!(nbt.shape, Shape::Cube | Shape::AxisCube { .. }) && nbt.is_solid(nb.state) { return true; }
                                    if let Some(nb_occ) = nbt.variant(nb.state).occupancy {
                                        let mut range_uv = |a0: f32, a1: f32| -> (usize, usize) {
                                            let r0 = a0.max(0.0).min(1.0);
                                            let r1 = a1.max(0.0).min(1.0);
                                            let s = if (r0 - 0.0).abs() < 1e-4 { 0 } else { 1 };
                                            let e = if (r1 - 1.0).abs() < 1e-4 { 2 } else { 1 };
                                            (s, e)
                                        };
                                        let fully = match face {
                                            Face::PosX => {
                                                let (ys, ye) = range_uv(min.y - fy, max.y - fy);
                                                let (zs, ze) = range_uv(min.z - fz, max.z - fz);
                                                let bx = 0usize;
                                                (ys..ye).all(|ly| (zs..ze).all(|lz| (nb_occ & micro_bit(bx, ly, lz)) != 0))
                                            }
                                            Face::NegX => {
                                                let (ys, ye) = range_uv(min.y - fy, max.y - fy);
                                                let (zs, ze) = range_uv(min.z - fz, max.z - fz);
                                                let bx = 1usize;
                                                (ys..ye).all(|ly| (zs..ze).all(|lz| (nb_occ & micro_bit(bx, ly, lz)) != 0))
                                            }
                                            Face::PosZ => {
                                                let (ys, ye) = range_uv(min.y - fy, max.y - fy);
                                                let (xs, xe) = range_uv(min.x - fx, max.x - fx);
                                                let bz = 0usize;
                                                (ys..ye).all(|ly| (xs..xe).all(|lx| (nb_occ & micro_bit(lx, ly, bz)) != 0))
                                            }
                                            Face::NegZ => {
                                                let (ys, ye) = range_uv(min.y - fy, max.y - fy);
                                                let (xs, xe) = range_uv(min.x - fx, max.x - fx);
                                                let bz = 1usize;
                                                (ys..ye).all(|ly| (xs..xe).all(|lx| (nb_occ & micro_bit(lx, ly, bz)) != 0))
                                            }
                                            Face::PosY => {
                                                let (zs, ze) = range_uv(min.z - fz, max.z - fz);
                                                let (xs, xe) = range_uv(min.x - fx, max.x - fx);
                                                let by = 0usize;
                                                (zs..ze).all(|lz| (xs..xe).all(|lx| (nb_occ & micro_bit(lx, by, lz)) != 0))
                                            }
                                            Face::NegY => {
                                                let (zs, ze) = range_uv(min.z - fz, max.z - fz);
                                                let (xs, xe) = range_uv(min.x - fx, max.x - fx);
                                                let by = 1usize;
                                                (zs..ze).all(|lz| (xs..xe).all(|lx| (nb_occ & micro_bit(lx, by, lz)) != 0))
                                            }
                                        };
                                        if fully { return true; }
                                    }
                                }
                                false
                            };
                            emit_box_faces(
                                &mut builds,
                                min,
                                max,
                                |face| {
                                    if occludes(face) { return None; }
                                    let lv = face_light(face, ambient);
                                    let rgba = [lv, lv, lv, 255];
                                    let mid = face_material(face);
                                    Some((mid, rgba))
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
                            |_, _, _, face, _| face_light(face, ambient),
                        );
                    } else if let Some(dyns) = var.dynamic {
                        let fx = x as f32;
                        let fy = y as f32;
                        let fz = z as f32;
                        let face_material = |face: Face| ty.material_for_cached(face.role(), b.state);
                        match dyns {
                            crate::blocks::registry::DynamicShape::Pane => {
                                let mut mask: u8 = 0;
                                let dirs = [(-1, 0, 0u8), (1, 0, 1u8), (0, -1, 2u8), (0, 1, 3u8)];
                                for (dx, dz, bit) in dirs {
                                    let nx = x as i32 + dx; let nz = z as i32 + dz; let ny = y as i32;
                                    if nx < 0 || nz < 0 || ny < 0 { continue; }
                                    let xu = nx as usize; let yu = ny as usize; let zu = nz as usize;
                                    if xu >= buf.sx || yu >= buf.sy || zu >= buf.sz { continue; }
                                    let nb = buf.get_local(xu, yu, zu);
                                    let connected = reg.get(nb.id).map(|t| matches!(t.shape, crate::blocks::Shape::Pane) || is_full_cube(reg, nb)).unwrap_or(false);
                                    if connected { mask |= 1u8 << bit; }
                                }
                                let t = 0.25f32;
                                let mut boxes: Vec<(Vector3, Vector3)> = Vec::new();
                                if mask == 0 {
                                    boxes.push((Vector3::new(fx + 0.0, fy + 0.0, fz + 0.5 - t * 0.5), Vector3::new(fx + 1.0, fy + 1.0, fz + 0.5 + t * 0.5)));
                                    boxes.push((Vector3::new(fx + 0.5 - t * 0.5, fy + 0.0, fz + 0.0), Vector3::new(fx + 0.5 + t * 0.5, fy + 1.0, fz + 1.0)));
                                } else {
                                    if (mask & (1 << 0)) != 0 { boxes.push((Vector3::new(fx + 0.0, fy + 0.0, fz + 0.5 - t * 0.5), Vector3::new(fx + 0.5, fy + 1.0, fz + 0.5 + t * 0.5))); }
                                    if (mask & (1 << 1)) != 0 { boxes.push((Vector3::new(fx + 0.5, fy + 0.0, fz + 0.5 - t * 0.5), Vector3::new(fx + 1.0, fy + 1.0, fz + 0.5 + t * 0.5))); }
                                    if (mask & (1 << 2)) != 0 { boxes.push((Vector3::new(fx + 0.5 - t * 0.5, fy + 0.0, fz + 0.0), Vector3::new(fx + 0.5 + t * 0.5, fy + 1.0, fz + 0.5))); }
                                    if (mask & (1 << 3)) != 0 { boxes.push((Vector3::new(fx + 0.5 - t * 0.5, fy + 0.0, fz + 0.5), Vector3::new(fx + 0.5 + t * 0.5, fy + 1.0, fz + 1.0))); }
                                }
                                for (min, max) in boxes { emit_box_local(&mut builds, buf, reg, x, y, z, &face_material, min, max, ambient); }
                            }
                            crate::blocks::registry::DynamicShape::Fence => {
                                let mut mask: u8 = 0;
                                let dirs = [(-1, 0, 0u8), (1, 0, 1u8), (0, -1, 2u8), (0, 1, 3u8)];
                                for (dx, dz, bit) in dirs {
                                    let nx = x as i32 + dx; let nz = z as i32 + dz; let ny = y as i32;
                                    if nx < 0 || nz < 0 || ny < 0 { continue; }
                                    let xu = nx as usize; let yu = ny as usize; let zu = nz as usize;
                                    if xu >= buf.sx || yu >= buf.sy || zu >= buf.sz { continue; }
                                    let nb = buf.get_local(xu, yu, zu);
                                    let connected = reg.get(nb.id).map(|t| matches!(t.shape, crate::blocks::Shape::Fence) || is_full_cube(reg, nb)).unwrap_or(false);
                                    if connected { mask |= 1u8 << bit; }
                                }
                                let p = 0.25f32; let t = 0.25f32; let mid_y0 = 0.5 - t * 0.5; let mid_y1 = 0.5 + t * 0.5;
                                let mut boxes: Vec<(Vector3, Vector3)> = Vec::new();
                                boxes.push((Vector3::new(fx + 0.5 - p * 0.5, fy + 0.0, fz + 0.5 - p * 0.5), Vector3::new(fx + 0.5 + p * 0.5, fy + 1.0, fz + 0.5 + p * 0.5)));
                                if (mask & (1 << 0)) != 0 { boxes.push((Vector3::new(fx + 0.0, fy + mid_y0, fz + 0.5 - t * 0.5), Vector3::new(fx + 0.5, fy + mid_y1, fz + 0.5 + t * 0.5))); }
                                if (mask & (1 << 1)) != 0 { boxes.push((Vector3::new(fx + 0.5, fy + mid_y0, fz + 0.5 - t * 0.5), Vector3::new(fx + 1.0, fy + mid_y1, fz + 0.5 + t * 0.5))); }
                                if (mask & (1 << 2)) != 0 { boxes.push((Vector3::new(fx + 0.5 - t * 0.5, fy + mid_y0, fz + 0.0), Vector3::new(fx + 0.5 + t * 0.5, fy + mid_y1, fz + 0.5))); }
                                if (mask & (1 << 3)) != 0 { boxes.push((Vector3::new(fx + 0.5 - t * 0.5, fy + mid_y0, fz + 0.5), Vector3::new(fx + 0.5 + t * 0.5, fy + mid_y1, fz + 1.0))); }
                                for (min, max) in boxes { emit_box_local(&mut builds, buf, reg, x, y, z, &face_material, min, max, ambient); }
                            }
                            crate::blocks::registry::DynamicShape::Gate => {
                                let mut along_x = true;
                                if let crate::blocks::Shape::Gate { facing_from, open_from } = &ty.shape {
                                    let facing = ty.state_prop_value(b.state, facing_from).unwrap_or("north");
                                    along_x = matches!(facing, "north" | "south");
                                    if ty.state_prop_is_value(b.state, open_from, "true") { along_x = !along_x; }
                                }
                                let t = 0.125f32; let y0 = 0.375f32; let y1 = 0.625f32;
                                let mut boxes: Vec<(Vector3, Vector3)> = Vec::new();
                                if along_x {
                                    boxes.push((Vector3::new(fx + 0.0, fy + y0, fz + 0.5 - t), Vector3::new(fx + 1.0, fy + y0 + t, fz + 0.5 + t)));
                                    boxes.push((Vector3::new(fx + 0.0, fy + y1, fz + 0.5 - t), Vector3::new(fx + 1.0, fy + y1 + t, fz + 0.5 + t)));
                                } else {
                                    boxes.push((Vector3::new(fx + 0.5 - t, fy + y0, fz + 0.0), Vector3::new(fx + 0.5 + t, fy + y0 + t, fz + 1.0)));
                                    boxes.push((Vector3::new(fx + 0.5 - t, fy + y1, fz + 0.0), Vector3::new(fx + 0.5 + t, fy + y1 + t, fz + 1.0)));
                                }
                                for (min, max) in boxes { emit_box_local(&mut builds, buf, reg, x, y, z, &face_material, min, max, ambient); }
                            }
                            crate::blocks::registry::DynamicShape::Carpet => {
                                let h = 0.0625f32;
                                let min = Vector3::new(fx, fy, fz); let max = Vector3::new(fx + 1.0, fy + h, fz + 1.0);
                                emit_box_local(&mut builds, buf, reg, x, y, z, &face_material, min, max, ambient);
                            }
                        }
                    }
                }
            }
        }
    }

    let bbox = Aabb { min: Vec3 { x: 0.0, y: 0.0, z: 0.0 }, max: Vec3 { x: sx as f32, y: sy as f32, z: sz as f32 } };
    ChunkMeshCPU {
        cx: 0,
        cz: 0,
        bbox,
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
    pub fn index(self) -> usize {
        self as usize
    }

    #[inline]
    #[allow(dead_code)]
    pub fn from_index(i: usize) -> Face {
        match i {
            0 => Face::PosY,
            1 => Face::NegY,
            2 => Face::PosX,
            3 => Face::NegX,
            4 => Face::PosZ,
            5 => Face::NegZ,
            _ => Face::PosY,
        }
    }

    #[inline]
    pub fn normal(self) -> Vector3 {
        match self {
            Face::PosY => Vector3::new(0.0, 1.0, 0.0),
            Face::NegY => Vector3::new(0.0, -1.0, 0.0),
            Face::PosX => Vector3::new(1.0, 0.0, 0.0),
            Face::NegX => Vector3::new(-1.0, 0.0, 0.0),
            Face::PosZ => Vector3::new(0.0, 0.0, 1.0),
            Face::NegZ => Vector3::new(0.0, 0.0, -1.0),
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

/// Ordered list of all faces; useful for compact table iteration.
#[allow(dead_code)]
pub const ALL_FACES: [Face; 6] = [
    Face::PosY,
    Face::NegY,
    Face::PosX,
    Face::NegX,
    Face::PosZ,
    Face::NegZ,
];

/// The four horizontal neighbor sides (west/east/north/south) with their face and local offsets.
/// Tuple: (dx, dz, face_to_draw_on_neighbor, x_offset, z_offset)
pub const SIDE_NEIGHBORS: [(i32, i32, Face, f32, f32); 4] = [
    (-1, 0, Face::PosX, 0.0, 0.0), // West neighbor, draw on its +X face
    (1, 0, Face::NegX, 1.0, 0.0),  // East neighbor, draw on its -X face
    (0, -1, Face::PosZ, 0.0, 0.0), // North neighbor, draw on its +Z face
    (0, 1, Face::NegZ, 0.0, 1.0),  // South neighbor, draw on its -Z face
];

#[inline]
pub fn is_full_cube(reg: &BlockRegistry, nb: Block) -> bool {
    // Treat as a full cube only when the block is solid and has a cube-like shape.
    // This avoids treating non-solid cubes like `air` as neighbors for fixups/connectivity.
    reg.get(nb.id)
        .map(|t| t.is_solid(nb.state) && matches!(t.shape, Shape::Cube | Shape::AxisCube { .. }))
        .unwrap_or(false)
}

/// Simple cardinal facing used by stairs and similar shapes.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Facing {
    North,
    South,
    West,
    East,
}

impl Facing {
    #[inline]
    pub fn from_str(s: &str) -> Facing {
        match s {
            "north" => Facing::North,
            "south" => Facing::South,
            "west" => Facing::West,
            "east" => Facing::East,
            _ => Facing::North,
        }
    }
}


// Generic greedy-rectangle sweep over a 2D mask. The mask is width*height laid out row-major.
// For each maximal rectangle of identical Some(code), calls `emit(x, y, w, h, code)` once.
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
            if code.is_none() || used[idx] {
                continue;
            }
            let mut w = 1;
            while x + w < width && mask[y * width + (x + w)] == code && !used[y * width + (x + w)] {
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
            emit(x, y, w, h, code.unwrap());
            for yy in 0..h {
                for xx in 0..w {
                    used[(y + yy) * width + (x + xx)] = true;
                }
            }
        }
    }
}

#[inline]
fn apply_min_light(l: u8, min: Option<u8>) -> u8 {
    if let Some(m) = min { l.max(m) } else { l }
}

// Core greedy meshing builder used by both world and local meshers.
// The `face_info` closure decides visibility and lighting per face; it must return None if the
// face is not visible. `flip_v[face]` controls V flipping for that face (0..5).
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
    F: FnMut(usize, usize, usize, Face, crate::blocks::Block) -> Option<(K, u8)>,
{
    let sx = buf.sx;
    let sy = buf.sy;
    let sz = buf.sz;
    let mut builds: HashMap<K, MeshBuild> = HashMap::new();

    // +Y faces
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
            mb.add_face_rect(
                Face::PosY,
                Vector3::new(fx, fy, fz),
                u1,
                v1,
                flip_v[Face::PosY.index()],
                rgba,
            );
        });
    }

    // -Y faces
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
            mb.add_face_rect(
                Face::NegY,
                Vector3::new(fx, fy, fz),
                u1,
                v1,
                flip_v[Face::NegY.index()],
                rgba,
            );
        });
    }

    // X planes (±X faces)
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
                mb.add_face_rect(
                    face,
                    Vector3::new(fx, fy, fz),
                    u1,
                    v1,
                    flip_v[face.index()],
                    rgba,
                );
            });
        }
    }

    // Z planes (±Z faces)
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
                mb.add_face_rect(
                    face,
                    Vector3::new(fx, fy, fz),
                    u1,
                    v1,
                    flip_v[face.index()],
                    rgba,
                );
            });
        }
    }

    builds
}

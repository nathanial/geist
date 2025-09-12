//! CPU meshing crate: greedy mesher and helpers (engine-only).
#![forbid(unsafe_code)]

use geist_blocks::BlockRegistry;
use geist_blocks::micro::micro_cell_solid_s2;
use geist_blocks::types::{Block, FaceRole, MaterialId};
use geist_chunk::ChunkBuf;
use geist_geom::{Aabb, Vec3};
use geist_lighting::{LightBorders, LightGrid, LightingStore, compute_light_with_borders_buf};
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
        let e1 = Vec3 {
            x: vs[1].x - vs[0].x,
            y: vs[1].y - vs[0].y,
            z: vs[1].z - vs[0].z,
        };
        let e2 = Vec3 {
            x: vs[2].x - vs[0].x,
            y: vs[2].y - vs[0].y,
            z: vs[2].z - vs[0].z,
        };
        let cross = Vec3 {
            x: e1.y * e2.z - e1.z * e2.y,
            y: e1.z * e2.x - e1.x * e2.z,
            z: e1.x * e2.y - e1.y * e2.x,
        };
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
            self.pos.extend_from_slice(&[vs[i].x, vs[i].y, vs[i].z]);
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
                Vec3 {
                    x: origin.x + u1,
                    y: origin.y,
                    z: origin.z,
                },
                Vec3 {
                    x: origin.x + u1,
                    y: origin.y,
                    z: origin.z + v1,
                },
                Vec3 {
                    x: origin.x,
                    y: origin.y,
                    z: origin.z + v1,
                },
            ),
            Face::NegY => (
                Vec3 {
                    x: origin.x,
                    y: origin.y,
                    z: origin.z + v1,
                },
                Vec3 {
                    x: origin.x + u1,
                    y: origin.y,
                    z: origin.z + v1,
                },
                Vec3 {
                    x: origin.x + u1,
                    y: origin.y,
                    z: origin.z,
                },
                origin,
            ),
            Face::PosX => (
                Vec3 {
                    x: origin.x,
                    y: origin.y + v1,
                    z: origin.z + u1,
                },
                Vec3 {
                    x: origin.x,
                    y: origin.y + v1,
                    z: origin.z,
                },
                origin,
                Vec3 {
                    x: origin.x,
                    y: origin.y,
                    z: origin.z + u1,
                },
            ),
            Face::NegX => (
                Vec3 {
                    x: origin.x,
                    y: origin.y + v1,
                    z: origin.z,
                },
                Vec3 {
                    x: origin.x,
                    y: origin.y + v1,
                    z: origin.z + u1,
                },
                Vec3 {
                    x: origin.x,
                    y: origin.y,
                    z: origin.z + u1,
                },
                origin,
            ),
            Face::PosZ => (
                Vec3 {
                    x: origin.x + u1,
                    y: origin.y + v1,
                    z: origin.z,
                },
                Vec3 {
                    x: origin.x,
                    y: origin.y + v1,
                    z: origin.z,
                },
                origin,
                Vec3 {
                    x: origin.x + u1,
                    y: origin.y,
                    z: origin.z,
                },
            ),
            Face::NegZ => (
                Vec3 {
                    x: origin.x,
                    y: origin.y + v1,
                    z: origin.z,
                },
                Vec3 {
                    x: origin.x + u1,
                    y: origin.y + v1,
                    z: origin.z,
                },
                Vec3 {
                    x: origin.x + u1,
                    y: origin.y,
                    z: origin.z,
                },
                origin,
            ),
        };
        self.add_quad(a, b, c, d, n, u1, v1, flip_v, rgba);
    }

    // Accessors used by debug/diagnostic paths
    pub fn positions(&self) -> &[f32] {
        &self.pos
    }
    pub fn normals(&self) -> &[f32] {
        &self.norm
    }
}

// Clip a face-aligned rectangle against the current chunk's interior bounds and emit if any portion remains.
// Chunk interior bounds: X in [base_x, base_x+sx), Z in [base_z, base_z+sz), Y in [0, sy).
#[inline]
fn emit_face_rect_for_clipped(
    builds: &mut std::collections::HashMap<MaterialId, MeshBuild>,
    mid: MaterialId,
    face: Face,
    mut origin: Vec3,
    mut u1: f32,
    mut v1: f32,
    rgba: [u8; 4],
    base_x: i32,
    sx: usize,
    sy: usize,
    base_z: i32,
    sz: usize,
) {
    let bx0 = base_x as f32;
    let bx1 = (base_x + sx as i32) as f32;
    let bz0 = base_z as f32;
    let bz1 = (base_z + sz as i32) as f32;
    let by0 = 0.0f32;
    let by1 = sy as f32;

    match face {
        Face::PosX | Face::NegX => {
            let fx = origin.x;
            if !(fx >= bx0 && fx < bx1) {
                return;
            }
            // Clip along Z (u)
            let z0 = origin.z;
            let z1 = origin.z + u1;
            let cz0 = z0.max(bz0);
            let cz1 = z1.min(bz1);
            if cz1 <= cz0 {
                return;
            }
            origin.z = cz0;
            u1 = cz1 - cz0;
            // Clip along Y (v)
            let y0 = origin.y;
            let y1 = origin.y + v1;
            let cy0 = y0.max(by0);
            let cy1 = y1.min(by1);
            if cy1 <= cy0 {
                return;
            }
            origin.y = cy0;
            v1 = cy1 - cy0;
        }
        Face::PosZ | Face::NegZ => {
            let fz = origin.z;
            if !(fz >= bz0 && fz < bz1) {
                return;
            }
            // Clip along X (u)
            let x0 = origin.x;
            let x1 = origin.x + u1;
            let cx0 = x0.max(bx0);
            let cx1 = x1.min(bx1);
            if cx1 <= cx0 {
                return;
            }
            origin.x = cx0;
            u1 = cx1 - cx0;
            // Clip along Y (v)
            let y0 = origin.y;
            let y1 = origin.y + v1;
            let cy0 = y0.max(by0);
            let cy1 = y1.min(by1);
            if cy1 <= cy0 {
                return;
            }
            origin.y = cy0;
            v1 = cy1 - cy0;
        }
        Face::PosY | Face::NegY => {
            let fy = origin.y;
            if !(fy >= by0 && fy < by1) {
                return;
            }
            // Clip along X (u)
            let x0 = origin.x;
            let x1 = origin.x + u1;
            let cx0 = x0.max(bx0);
            let cx1 = x1.min(bx1);
            if cx1 <= cx0 {
                return;
            }
            origin.x = cx0;
            u1 = cx1 - cx0;
            // Clip along Z (v)
            let z0 = origin.z;
            let z1 = origin.z + v1;
            let cz0 = z0.max(bz0);
            let cz1 = z1.min(bz1);
            if cz1 <= cz0 {
                return;
            }
            origin.z = cz0;
            v1 = cz1 - cz0;
        }
    }
    emit_face_rect_for(builds, mid, face, origin, u1, v1, rgba);
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
        Vec3 {
            x: min.x,
            y: max.y,
            z: min.z,
        },
        Vec3 {
            x: min.x,
            y: min.y,
            z: min.z,
        },
        Vec3 {
            x: max.x,
            y: max.y,
            z: min.z,
        },
        Vec3 {
            x: max.x,
            y: min.y,
            z: min.z,
        },
        Vec3 {
            x: min.x,
            y: max.y,
            z: max.z,
        },
        Vec3 {
            x: min.x,
            y: min.y,
            z: max.z,
        },
        Vec3 {
            x: max.x,
            y: max.y,
            z: max.z,
        },
        Vec3 {
            x: max.x,
            y: min.y,
            z: max.z,
        },
    ];

    for &(face, indices, normal) in &FACE_DATA {
        if let Some((mid, rgba)) = choose(face) {
            let (u1, v1) = match face {
                Face::PosY | Face::NegY => (max.x - min.x, max.z - min.z),
                Face::PosX | Face::NegX => (max.z - min.z, max.y - min.y),
                Face::PosZ | Face::NegZ => (max.x - min.x, max.y - min.y),
            };
            let n = Vec3 {
                x: normal.0,
                y: normal.1,
                z: normal.2,
            };
            builds.entry(mid).or_default().add_quad(
                corners[indices[0]],
                corners[indices[1]],
                corners[indices[2]],
                corners[indices[3]],
                n,
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
    min: Vec3,
    max: Vec3,
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

#[inline]
fn emit_box_generic_clipped(
    builds: &mut std::collections::HashMap<MaterialId, MeshBuild>,
    mut min: Vec3,
    mut max: Vec3,
    fm_for_face: &dyn Fn(Face) -> MaterialId,
    occludes: impl FnMut(Face) -> bool,
    sample_light: impl FnMut(Face) -> u8,
    base_x: i32,
    sx: usize,
    sy: usize,
    base_z: i32,
    sz: usize,
) {
    let bx0 = base_x as f32;
    let bx1 = (base_x + sx as i32) as f32;
    let by0 = 0.0f32;
    let by1 = sy as f32;
    let bz0 = base_z as f32;
    let bz1 = (base_z + sz as i32) as f32;
    min.x = min.x.max(bx0);
    min.y = min.y.max(by0);
    min.z = min.z.max(bz0);
    max.x = max.x.min(bx1);
    max.y = max.y.min(by1);
    max.z = max.z.min(bz1);
    if !(min.x < max.x && min.y < max.y && min.z < max.z) {
        return;
    }
    emit_box_generic(builds, min, max, fm_for_face, occludes, sample_light);
}

#[inline]
fn microgrid_boxes(fx: f32, fy: f32, fz: f32, occ: u8) -> Vec<(Vec3, Vec3)> {
    use microgrid_tables::occ8_to_boxes;
    let cell = 0.5f32;
    let mut out = Vec::new();
    for b in occ8_to_boxes(occ) {
        let min = Vec3 {
            x: fx + (b[0] as f32) * cell,
            y: fy + (b[1] as f32) * cell,
            z: fz + (b[2] as f32) * cell,
        };
        let max = Vec3 {
            x: fx + (b[3] as f32) * cell,
            y: fy + (b[4] as f32) * cell,
            z: fz + (b[5] as f32) * cell,
        };
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
    // Out of local bounds: unconditionally consult world+edits to decide occlusion (overscan default)
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
        Some(store) => compute_light_with_borders_buf(buf, store, reg, world),
        None => return None,
    };

    // Phase 2: Use a single WCC mesher at S=2 to cover full cubes and micro occupancy.
    let S: usize = 2;
    let mut wm = WccMesher::new(buf, &light, reg, S, base_x, base_z, world, edits, neighbors);

    for z in 0..sz {
        for y in 0..sy {
            for x in 0..sx {
                let b = buf.get_local(x, y, z);
                if let Some(ty) = reg.get(b.id) {
                    let var = ty.variant(b.state);
                    if let Some(occ) = var.occupancy {
                        wm.add_micro(x, y, z, b, occ);
                        continue;
                    }
                    // Water: mesh only surfaces against air, so terrain under water remains visible
                    if ty.name == "water" {
                        wm.add_water_cube(x, y, z, b);
                        continue;
                    }
                }
                if is_full_cube(reg, b) {
                    wm.add_cube(x, y, z, b);
                }
            }
        }
    }

    let mut builds: HashMap<MaterialId, MeshBuild> = HashMap::new();
    // Overscan: incorporate neighbor seam contributions before emission
    wm.seed_neighbor_seams();
    wm.emit_into(&mut builds);

    // Phase 3: thin dynamic shapes (pane, fence, gate, carpet)
    // Emit via thin-box pass reusing existing shape logic and occluder checks.
    for z in 0..sz {
        for y in 0..sy {
            for x in 0..sx {
                let here = buf.get_local(x, y, z);
                let fx = (base_x + x as i32) as f32;
                let fy = y as f32;
                let fz = (base_z + z as i32) as f32;
                if let Some(ty) = reg.get(here.id) {
                    // Skip occupancy-driven shapes; those already went through WCC.
                    if ty.variant(here.state).occupancy.is_some() {
                        continue;
                    }
                    match &ty.shape {
                        geist_blocks::types::Shape::Pane => {
                            let face_material =
                                |face: Face| ty.material_for_cached(face.role(), here.state);
                            let t = 0.0625f32;
                            let min = Vec3 {
                                x: fx + 0.5 - t,
                                y: fy,
                                z: fz,
                            };
                            let max = Vec3 {
                                x: fx + 0.5 + t,
                                y: fy + 1.0,
                                z: fz + 1.0,
                            };
                            emit_box_generic_clipped(
                                &mut builds,
                                min,
                                max,
                                &face_material,
                                |face| {
                                    let (dx, dy, dz) = face.delta();
                                    let (nx, ny, nz) =
                                        (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                    is_occluder(
                                        buf, world, edits, neighbors, reg, here, face, nx, ny, nz,
                                    )
                                },
                                |face| {
                                    let lv =
                                        light.sample_face_local_s2(buf, reg, x, y, z, face.index());
                                    lv.max(VISUAL_LIGHT_MIN)
                                },
                                base_x,
                                sx,
                                sy,
                                base_z,
                                sz,
                            );
                            // Add side connectors to adjacent panes
                            let mut connect_xn = false;
                            let mut connect_xp = false;
                            let mut connect_zn = false;
                            let mut connect_zp = false;
                            {
                                let (dx, dy, dz) = Face::PosZ.delta();
                                if let Some(nb) = buf.get_world(
                                    (fx as i32) + dx,
                                    (fy as i32) + dy,
                                    (fz as i32) + dz,
                                ) {
                                    if let Some(nb_ty) = reg.get(nb.id) {
                                        if nb_ty.shape == geist_blocks::types::Shape::Pane {
                                            connect_zp = true;
                                        }
                                    }
                                }
                            }
                            {
                                let (dx, dy, dz) = Face::NegZ.delta();
                                if let Some(nb) = buf.get_world(
                                    (fx as i32) + dx,
                                    (fy as i32) + dy,
                                    (fz as i32) + dz,
                                ) {
                                    if let Some(nb_ty) = reg.get(nb.id) {
                                        if nb_ty.shape == geist_blocks::types::Shape::Pane {
                                            connect_zn = true;
                                        }
                                    }
                                }
                            }
                            {
                                let (dx, dy, dz) = Face::PosX.delta();
                                if let Some(nb) = buf.get_world(
                                    (fx as i32) + dx,
                                    (fy as i32) + dy,
                                    (fz as i32) + dz,
                                ) {
                                    if let Some(nb_ty) = reg.get(nb.id) {
                                        if nb_ty.shape == geist_blocks::types::Shape::Pane {
                                            connect_xp = true;
                                        }
                                    }
                                }
                            }
                            {
                                let (dx, dy, dz) = Face::NegX.delta();
                                if let Some(nb) = buf.get_world(
                                    (fx as i32) + dx,
                                    (fy as i32) + dy,
                                    (fz as i32) + dz,
                                ) {
                                    if let Some(nb_ty) = reg.get(nb.id) {
                                        if nb_ty.shape == geist_blocks::types::Shape::Pane {
                                            connect_xn = true;
                                        }
                                    }
                                }
                            }
                            let t = 0.0625f32;
                            if connect_xn {
                                let min = Vec3 {
                                    x: fx + 0.0,
                                    y: fy,
                                    z: fz + 0.5 - t,
                                };
                                let max = Vec3 {
                                    x: fx + 0.5 - t,
                                    y: fy + 1.0,
                                    z: fz + 0.5 + t,
                                };
                                emit_box_generic_clipped(
                                    &mut builds,
                                    min,
                                    max,
                                    &face_material,
                                    |_face| false,
                                    |face| {
                                        light
                                            .sample_face_local_s2(buf, reg, x, y, z, face.index())
                                            .max(VISUAL_LIGHT_MIN)
                                    },
                                    base_x,
                                    sx,
                                    sy,
                                    base_z,
                                    sz,
                                );
                            }
                            if connect_xp {
                                let min = Vec3 {
                                    x: fx + 0.5 + t,
                                    y: fy,
                                    z: fz + 0.5 - t,
                                };
                                let max = Vec3 {
                                    x: fx + 1.0,
                                    y: fy + 1.0,
                                    z: fz + 0.5 + t,
                                };
                                emit_box_generic_clipped(
                                    &mut builds,
                                    min,
                                    max,
                                    &face_material,
                                    |_face| false,
                                    |face| {
                                        light
                                            .sample_face_local_s2(buf, reg, x, y, z, face.index())
                                            .max(VISUAL_LIGHT_MIN)
                                    },
                                    base_x,
                                    sx,
                                    sy,
                                    base_z,
                                    sz,
                                );
                            }
                            if connect_zn {
                                let min = Vec3 {
                                    x: fx + 0.5 - t,
                                    y: fy,
                                    z: fz + 0.0,
                                };
                                let max = Vec3 {
                                    x: fx + 0.5 + t,
                                    y: fy + 1.0,
                                    z: fz + 0.5 - t,
                                };
                                emit_box_generic_clipped(
                                    &mut builds,
                                    min,
                                    max,
                                    &face_material,
                                    |_face| false,
                                    |face| {
                                        light
                                            .sample_face_local_s2(buf, reg, x, y, z, face.index())
                                            .max(VISUAL_LIGHT_MIN)
                                    },
                                    base_x,
                                    sx,
                                    sy,
                                    base_z,
                                    sz,
                                );
                            }
                            if connect_zp {
                                let min = Vec3 {
                                    x: fx + 0.5 - t,
                                    y: fy,
                                    z: fz + 0.5 + t,
                                };
                                let max = Vec3 {
                                    x: fx + 0.5 + t,
                                    y: fy + 1.0,
                                    z: fz + 1.0,
                                };
                                emit_box_generic_clipped(
                                    &mut builds,
                                    min,
                                    max,
                                    &face_material,
                                    |_face| false,
                                    |face| {
                                        light
                                            .sample_face_local_s2(buf, reg, x, y, z, face.index())
                                            .max(VISUAL_LIGHT_MIN)
                                    },
                                    base_x,
                                    sx,
                                    sy,
                                    base_z,
                                    sz,
                                );
                            }
                        }
                        geist_blocks::types::Shape::Fence => {
                            // Posts + arms where connected to neighbors (fence/pane/gate)
                            let face_material =
                                |face: Face| ty.material_for_cached(face.role(), here.state);
                            let mut connect = [false; 4]; // xn,xp,zn,zp
                            for (i, (dx, dz)) in
                                [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().enumerate()
                            {
                                if let Some(nb) =
                                    buf.get_world((fx as i32) + dx, fy as i32, (fz as i32) + dz)
                                {
                                    if let Some(nb_ty) = reg.get(nb.id) {
                                        connect[i] = nb_ty.shape
                                            == geist_blocks::types::Shape::Fence
                                            || nb_ty.shape == geist_blocks::types::Shape::Pane
                                            || matches!(
                                                nb_ty.shape,
                                                geist_blocks::types::Shape::Gate { .. }
                                            );
                                    }
                                }
                            }
                            // Central post
                            let t = 0.125f32; // post half-width
                            let min = Vec3 {
                                x: fx + 0.5 - t,
                                y: fy,
                                z: fz + 0.5 - t,
                            };
                            let max = Vec3 {
                                x: fx + 0.5 + t,
                                y: fy + 1.0,
                                z: fz + 0.5 + t,
                            };
                            emit_box_generic_clipped(
                                &mut builds,
                                min,
                                max,
                                &face_material,
                                |_face| false,
                                |face| {
                                    light
                                        .sample_face_local_s2(buf, reg, x, y, z, face.index())
                                        .max(VISUAL_LIGHT_MIN)
                                },
                                base_x,
                                sx,
                                sy,
                                base_z,
                                sz,
                            );
                            // Arms
                            let t = 0.125f32;
                            let arm = 0.5f32 - t;
                            if connect[0] {
                                // xn
                                let min = Vec3 {
                                    x: fx + 0.0,
                                    y: fy + 0.375,
                                    z: fz + 0.5 - t,
                                };
                                let max = Vec3 {
                                    x: fx + 0.5,
                                    y: fy + 0.625,
                                    z: fz + 0.5 + t,
                                };
                                emit_box_generic_clipped(
                                    &mut builds,
                                    min,
                                    max,
                                    &face_material,
                                    |_face| false,
                                    |face| {
                                        light
                                            .sample_face_local_s2(buf, reg, x, y, z, face.index())
                                            .max(VISUAL_LIGHT_MIN)
                                    },
                                    base_x,
                                    sx,
                                    sy,
                                    base_z,
                                    sz,
                                );
                            }
                            if connect[1] {
                                // xp
                                let min = Vec3 {
                                    x: fx + 0.5,
                                    y: fy + 0.375,
                                    z: fz + 0.5 - t,
                                };
                                let max = Vec3 {
                                    x: fx + 1.0,
                                    y: fy + 0.625,
                                    z: fz + 0.5 + t,
                                };
                                emit_box_generic_clipped(
                                    &mut builds,
                                    min,
                                    max,
                                    &face_material,
                                    |_face| false,
                                    |face| {
                                        light
                                            .sample_face_local_s2(buf, reg, x, y, z, face.index())
                                            .max(VISUAL_LIGHT_MIN)
                                    },
                                    base_x,
                                    sx,
                                    sy,
                                    base_z,
                                    sz,
                                );
                            }
                            if connect[2] {
                                // zn
                                let min = Vec3 {
                                    x: fx + 0.5 - t,
                                    y: fy + 0.375,
                                    z: fz + 0.0,
                                };
                                let max = Vec3 {
                                    x: fx + 0.5 + t,
                                    y: fy + 0.625,
                                    z: fz + 0.5,
                                };
                                emit_box_generic_clipped(
                                    &mut builds,
                                    min,
                                    max,
                                    &face_material,
                                    |_face| false,
                                    |face| {
                                        light
                                            .sample_face_local_s2(buf, reg, x, y, z, face.index())
                                            .max(VISUAL_LIGHT_MIN)
                                    },
                                    base_x,
                                    sx,
                                    sy,
                                    base_z,
                                    sz,
                                );
                            }
                            if connect[3] {
                                // zp
                                let min = Vec3 {
                                    x: fx + 0.5 - t,
                                    y: fy + 0.375,
                                    z: fz + 0.5,
                                };
                                let max = Vec3 {
                                    x: fx + 0.5 + t,
                                    y: fy + 0.625,
                                    z: fz + 1.0,
                                };
                                emit_box_generic_clipped(
                                    &mut builds,
                                    min,
                                    max,
                                    &face_material,
                                    |_face| false,
                                    |face| {
                                        light
                                            .sample_face_local_s2(buf, reg, x, y, z, face.index())
                                            .max(VISUAL_LIGHT_MIN)
                                    },
                                    base_x,
                                    sx,
                                    sy,
                                    base_z,
                                    sz,
                                );
                            }
                            let _ = arm; // silence
                        }
                        geist_blocks::types::Shape::Gate { .. } => {
                            let mut along_x = true;
                            if let geist_blocks::types::Shape::Gate {
                                facing_from,
                                open_from,
                            } = &ty.shape
                            {
                                let facing = ty
                                    .state_prop_value(here.state, facing_from)
                                    .unwrap_or("north");
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
                                boxes.push((
                                    Vec3 {
                                        x: fx + 0.0,
                                        y: fy + y0,
                                        z: fz + 0.5 - t,
                                    },
                                    Vec3 {
                                        x: fx + 1.0,
                                        y: fy + y0 + t,
                                        z: fz + 0.5 + t,
                                    },
                                ));
                                boxes.push((
                                    Vec3 {
                                        x: fx + 0.0,
                                        y: fy + y1,
                                        z: fz + 0.5 - t,
                                    },
                                    Vec3 {
                                        x: fx + 1.0,
                                        y: fy + y1 + t,
                                        z: fz + 0.5 + t,
                                    },
                                ));
                            } else {
                                boxes.push((
                                    Vec3 {
                                        x: fx + 0.5 - t,
                                        y: fy + y0,
                                        z: fz + 0.0,
                                    },
                                    Vec3 {
                                        x: fx + 0.5 + t,
                                        y: fy + y0 + t,
                                        z: fz + 1.0,
                                    },
                                ));
                                boxes.push((
                                    Vec3 {
                                        x: fx + 0.5 - t,
                                        y: fy + y1,
                                        z: fz + 0.0,
                                    },
                                    Vec3 {
                                        x: fx + 0.5 + t,
                                        y: fy + y1 + t,
                                        z: fz + 1.0,
                                    },
                                ));
                            }
                            let face_material =
                                |face: Face| ty.material_for_cached(face.role(), here.state);
                            for (min, max) in boxes {
                                emit_box_generic_clipped(
                                    &mut builds,
                                    min,
                                    max,
                                    &face_material,
                                    |face| {
                                        let (dx, dy, dz) = face.delta();
                                        let (nx, ny, nz) =
                                            (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                        is_occluder(
                                            buf, world, edits, neighbors, reg, here, face, nx, ny,
                                            nz,
                                        )
                                    },
                                    |face| {
                                        let lv = light.sample_face_local_s2(
                                            buf,
                                            reg,
                                            x,
                                            y,
                                            z,
                                            face.index(),
                                        );
                                        lv.max(VISUAL_LIGHT_MIN)
                                    },
                                    base_x,
                                    sx,
                                    sy,
                                    base_z,
                                    sz,
                                );
                            }
                        }
                        geist_blocks::types::Shape::Carpet => {
                            let h = 0.0625f32;
                            let min = Vec3 {
                                x: fx,
                                y: fy,
                                z: fz,
                            };
                            let max = Vec3 {
                                x: fx + 1.0,
                                y: fy + h,
                                z: fz + 1.0,
                            };
                            let face_material =
                                |face: Face| ty.material_for_cached(face.role(), here.state);
                            emit_box_generic_clipped(
                                &mut builds,
                                min,
                                max,
                                &face_material,
                                |face| {
                                    let (dx, dy, dz) = face.delta();
                                    let (nx, ny, nz) =
                                        (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                    is_occluder(
                                        buf, world, edits, neighbors, reg, here, face, nx, ny, nz,
                                    )
                                },
                                |face| {
                                    let lv =
                                        light.sample_face_local_s2(buf, reg, x, y, z, face.index());
                                    lv.max(VISUAL_LIGHT_MIN)
                                },
                                base_x,
                                sx,
                                sy,
                                base_z,
                                sz,
                            );
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    let bbox = Aabb {
        min: Vec3 {
            x: base_x as f32,
            y: 0.0,
            z: base_z as f32,
        },
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

// ---------------- WCC (S-scaled) implementation ----------------

#[derive(Default)]
struct KeyTable {
    items: Vec<(MaterialId, u8)>,
    map: std::collections::HashMap<(MaterialId, u8), u16>,
}

impl KeyTable {
    fn new() -> Self {
        let mut kt = KeyTable {
            items: Vec::new(),
            map: HashMap::new(),
        };
        // Reserve 0 as None
        kt.items.push((MaterialId(0), 0));
        kt
    }
    #[inline]
    fn ensure(&mut self, mid: MaterialId, l: u8) -> u16 {
        if let Some(&idx) = self.map.get(&(mid, l)) {
            return idx;
        }
        let idx = self.items.len() as u16;
        self.items.push((mid, l));
        self.map.insert((mid, l), idx);
        idx
    }
    #[inline]
    fn get(&self, idx: u16) -> (MaterialId, u8) {
        self.items[idx as usize]
    }
}

struct Bitset {
    data: Vec<u64>,
    n: usize,
}
impl Bitset {
    fn new(n: usize) -> Self {
        let words = (n + 63) / 64;
        Self {
            data: vec![0; words],
            n,
        }
    }
    #[inline]
    fn toggle(&mut self, i: usize) {
        let w = i >> 6;
        let b = i & 63;
        self.data[w] ^= 1u64 << b;
    }
    #[inline]
    fn set(&mut self, i: usize, v: bool) {
        let w = i >> 6;
        let b = i & 63;
        if v {
            self.data[w] |= 1u64 << b;
        } else {
            self.data[w] &= !(1u64 << b);
        }
    }
    #[inline]
    fn get(&self, i: usize) -> bool {
        let w = i >> 6;
        let b = i & 63;
        (self.data[w] >> b) & 1 != 0
    }
}

struct FaceGrids {
    // Parity per face-cell (true if boundary)
    px: Bitset,
    py: Bitset,
    pz: Bitset,
    // Orientation bit per face-cell: true = positive face (PosX/PosY/PosZ)
    ox: Bitset,
    oy: Bitset,
    oz: Bitset,
    // Key indices per face-cell (0 = None)
    kx: Vec<u16>,
    ky: Vec<u16>,
    kz: Vec<u16>,
    // Scales and dims
    S: usize,
    sx: usize,
    sy: usize,
    sz: usize,
}

impl FaceGrids {
    fn new(S: usize, sx: usize, sy: usize, sz: usize) -> Self {
        let nx = (S * sx + 1) * (S * sy) * (S * sz);
        let ny = (S * sx) * (S * sy + 1) * (S * sz);
        let nz = (S * sx) * (S * sy) * (S * sz + 1);
        Self {
            px: Bitset::new(nx),
            py: Bitset::new(ny),
            pz: Bitset::new(nz),
            ox: Bitset::new(nx),
            oy: Bitset::new(ny),
            oz: Bitset::new(nz),
            kx: vec![0; nx],
            ky: vec![0; ny],
            kz: vec![0; nz],
            S,
            sx,
            sy,
            sz,
        }
    }
    #[inline]
    fn idx_x(&self, ix: usize, iy: usize, iz: usize) -> usize {
        let wy = self.S * self.sy;
        let wz = self.S * self.sz;
        (ix * wy + iy) * wz + iz
    }
    #[inline]
    fn idx_y(&self, ix: usize, iy: usize, iz: usize) -> usize {
        let wx = self.S * self.sx;
        let wz = self.S * self.sz;
        (iy * wz + iz) * wx + ix
    }
    #[inline]
    fn idx_z(&self, ix: usize, iy: usize, iz: usize) -> usize {
        let wx = self.S * self.sx;
        let wy = self.S * self.sy;
        (iz * wy + iy) * wx + ix
    }
}

pub struct WccMesher<'a> {
    S: usize,
    sx: usize,
    sy: usize,
    sz: usize,
    grids: FaceGrids,
    keys: KeyTable,
    reg: &'a BlockRegistry,
    light: &'a LightGrid,
    buf: &'a ChunkBuf,
    world: &'a World,
    edits: Option<&'a HashMap<(i32, i32, i32), Block>>,
    neighbors: NeighborsLoaded,
    base_x: i32,
    base_z: i32,
}

impl<'a> WccMesher<'a> {
    pub fn new(
        buf: &'a ChunkBuf,
        light: &'a LightGrid,
        reg: &'a BlockRegistry,
        S: usize,
        base_x: i32,
        base_z: i32,
        world: &'a World,
        edits: Option<&'a HashMap<(i32, i32, i32), Block>>,
        neighbors: NeighborsLoaded,
    ) -> Self {
        let (sx, sy, sz) = (buf.sx, buf.sy, buf.sz);
        Self {
            S,
            sx,
            sy,
            sz,
            grids: FaceGrids::new(S, sx, sy, sz),
            keys: KeyTable::new(),
            reg,
            light,
            buf,
            world,
            edits,
            neighbors,
            base_x,
            base_z,
        }
    }
    // Overscan: seed parity on our -X/-Z boundary planes using neighbor world blocks.
    // This cancels interior faces across seams and emits neighbor-owned faces when our side is empty.
    pub fn seed_neighbor_seams(&mut self) {
        // -X seam: toggle +X faces of neighbor cells onto ix==0
        for ly in 0..self.sy {
            for lz in 0..self.sz {
                // Neighbor at (base_x-1, ly, base_z+lz)
                let nb = self.world_block(self.base_x - 1, ly as i32, self.base_z + lz as i32);
                // Do not allow water to occlude across seams; keep terrain faces under water visible
                if self
                    .reg
                    .get(nb.id)
                    .map(|t| t.name == "water")
                    .unwrap_or(false)
                {
                    continue;
                }
                // Optional: honor seam policy to not occlude same-type neighbors
                let here = self.buf.get_local(0, ly, lz);
                if let (Some(ht), Some(_nt)) = (self.reg.get(here.id), self.reg.get(nb.id)) {
                    if ht.seam.dont_occlude_same && here.id == nb.id {
                        continue;
                    }
                }
                for iym in 0..self.S {
                    for izm in 0..self.S {
                        if micro_cell_solid_s2(self.reg, nb, 1, iym, izm) {
                            let iy = ly * self.S + iym;
                            let iz = lz * self.S + izm;
                            let mid = registry_material_for_or_unknown(nb, Face::PosX, self.reg);
                            // Shade across the -X seam using NegX sampling (neighbor lies at -X)
                            let l = self.light_bin(0, ly, lz, Face::NegX);
                            // ix==0 plane; pos=true (PosX)
                            self.toggle_x(0, 0, 0, 0, iy, iy + 1, iz, iz + 1, true, mid, l);
                        }
                    }
                }
            }
        }
        // -Z seam: toggle +Z faces of neighbor cells onto iz==0
        for ly in 0..self.sy {
            for lx in 0..self.sx {
                let nb = self.world_block(self.base_x + lx as i32, ly as i32, self.base_z - 1);
                if self
                    .reg
                    .get(nb.id)
                    .map(|t| t.name == "water")
                    .unwrap_or(false)
                {
                    continue;
                }
                let here = self.buf.get_local(lx, ly, 0);
                if let (Some(ht), Some(_nt)) = (self.reg.get(here.id), self.reg.get(nb.id)) {
                    if ht.seam.dont_occlude_same && here.id == nb.id {
                        continue;
                    }
                }
                for ixm in 0..self.S {
                    for iym in 0..self.S {
                        if micro_cell_solid_s2(self.reg, nb, ixm, iym, 1) {
                            let ix = lx * self.S + ixm;
                            let iy = ly * self.S + iym;
                            let mid = registry_material_for_or_unknown(nb, Face::PosZ, self.reg);
                            // Shade across the -Z seam using NegZ sampling (neighbor lies at -Z)
                            let l = self.light_bin(lx, ly, 0, Face::NegZ);
                            // iz==0 plane; pos=true (PosZ)
                            self.toggle_z(0, 0, 0, 0, ix, ix + 1, iy, iy + 1, true, mid, l);
                        }
                    }
                }
            }
        }
    }
    #[inline]
    fn local_micro_touches_negx(&self, here: Block, iym: usize, izm: usize) -> bool {
        micro_cell_solid_s2(self.reg, here, 0, iym, izm)
    }
    #[inline]
    fn local_micro_touches_negz(&self, here: Block, ixm: usize, iym: usize) -> bool {
        micro_cell_solid_s2(self.reg, here, ixm, iym, 0)
    }
    #[inline]
    fn neighbor_face_info_negx(
        &self,
        ly: usize,
        iym: usize,
        lz: usize,
        izm: usize,
    ) -> Option<(MaterialId, u8)> {
        // Sample neighbor block one voxel to -X; if it occupies this micro YZ cell at its +X boundary, emit PosX face
        let nx = self.base_x - 1;
        let ny = ly as i32;
        let nz = self.base_z + lz as i32;
        let nb = self.world_block(nx, ny, nz);
        if micro_cell_solid_s2(self.reg, nb, 1, iym, izm) {
            let mid = registry_material_for_or_unknown(nb, Face::PosX, self.reg);
            // Shade across the -X seam using NegX sampling
            let l = self.light_bin(0, ly, lz, Face::NegX);
            return Some((mid, l));
        }
        None
    }
    #[inline]
    fn neighbor_face_info_negz(
        &self,
        lx: usize,
        ixm: usize,
        ly: usize,
        iym: usize,
    ) -> Option<(MaterialId, u8)> {
        // Sample neighbor block one voxel to -Z; if it occupies this micro X Y cell at its +Z boundary, emit PosZ face
        let nx = self.base_x + lx as i32;
        let ny = ly as i32;
        let nz = self.base_z - 1;
        let nb = self.world_block(nx, ny, nz);
        if micro_cell_solid_s2(self.reg, nb, ixm, iym, 1) {
            let mid = registry_material_for_or_unknown(nb, Face::PosZ, self.reg);
            // Shade across the -Z seam using NegZ sampling
            let l = self.light_bin(lx, ly, 0, Face::NegZ);
            return Some((mid, l));
        }
        None
    }
    #[inline]
    fn world_block(&self, nx: i32, ny: i32, nz: i32) -> Block {
        if let Some(es) = self.edits {
            es.get(&(nx, ny, nz))
                .copied()
                .unwrap_or_else(|| self.world.block_at_runtime(self.reg, nx, ny, nz))
        } else {
            self.world.block_at_runtime(self.reg, nx, ny, nz)
        }
    }
    #[inline]
    fn neighbor_micro_occludes_negx(
        &self,
        here: Block,
        ly: usize,
        iym: usize,
        lz: usize,
        izm: usize,
    ) -> bool {
        // Sample neighbor block one voxel to -X
        let nx = self.base_x - 1;
        let ny = ly as i32;
        let nz = self.base_z + lz as i32;
        let nb = self.world_block(nx, ny, nz);
        if self
            .reg
            .get(here.id)
            .map(|t| t.seam.dont_occlude_same && here.id == nb.id)
            .unwrap_or(false)
        {
            return false;
        }
        micro_cell_solid_s2(self.reg, nb, 1, iym, izm)
    }
    #[inline]
    fn neighbor_micro_occludes_negz(
        &self,
        here: Block,
        lx: usize,
        ixm: usize,
        ly: usize,
        iym: usize,
    ) -> bool {
        // Sample neighbor block one voxel to -Z
        let nx = self.base_x + lx as i32;
        let ny = ly as i32;
        let nz = self.base_z - 1;
        let nb = self.world_block(nx, ny, nz);
        if self
            .reg
            .get(here.id)
            .map(|t| t.seam.dont_occlude_same && here.id == nb.id)
            .unwrap_or(false)
        {
            return false;
        }
        micro_cell_solid_s2(self.reg, nb, ixm, iym, 1)
    }
    #[inline]
    fn light_bin(&self, x: usize, y: usize, z: usize, face: Face) -> u8 {
        let l = self
            .light
            .sample_face_local_s2(self.buf, self.reg, x, y, z, face.index());
        l.max(VISUAL_LIGHT_MIN)
    }
    fn toggle_box(
        &mut self,
        x: usize,
        y: usize,
        z: usize,
        bx: (usize, usize, usize, usize, usize, usize),
        mat_for: impl Fn(Face) -> MaterialId,
    ) {
        let (x0, x1, y0, y1, z0, z1) = bx;
        // +X at ix=x1
        self.toggle_x(
            x,
            y,
            z,
            x1,
            y0,
            y1,
            z0,
            z1,
            true,
            mat_for(Face::PosX),
            self.light_bin(x, y, z, Face::PosX),
        );
        // -X at ix=x0
        self.toggle_x(
            x,
            y,
            z,
            x0,
            y0,
            y1,
            z0,
            z1,
            false,
            mat_for(Face::NegX),
            self.light_bin(x, y, z, Face::NegX),
        );
        // +Y at iy=y1
        self.toggle_y(
            x,
            y,
            z,
            y1,
            x0,
            x1,
            z0,
            z1,
            true,
            mat_for(Face::PosY),
            self.light_bin(x, y, z, Face::PosY),
        );
        // -Y at iy=y0
        self.toggle_y(
            x,
            y,
            z,
            y0,
            x0,
            x1,
            z0,
            z1,
            false,
            mat_for(Face::NegY),
            self.light_bin(x, y, z, Face::NegY),
        );
        // +Z at iz=z1
        self.toggle_z(
            x,
            y,
            z,
            z1,
            x0,
            x1,
            y0,
            y1,
            true,
            mat_for(Face::PosZ),
            self.light_bin(x, y, z, Face::PosZ),
        );
        // -Z at iz=z0
        self.toggle_z(
            x,
            y,
            z,
            z0,
            x0,
            x1,
            y0,
            y1,
            false,
            mat_for(Face::NegZ),
            self.light_bin(x, y, z, Face::NegZ),
        );
    }
    fn toggle_x(
        &mut self,
        bx: usize,
        by: usize,
        bz: usize,
        ix: usize,
        y0: usize,
        y1: usize,
        z0: usize,
        z1: usize,
        pos: bool,
        mid: MaterialId,
        l: u8,
    ) {
        let key = self.keys.ensure(mid, l);
        for iy in y0..y1 {
            for iz in z0..z1 {
                let idx = self.grids.idx_x(ix, iy, iz);
                self.grids.px.toggle(idx);
                if self.grids.px.get(idx) {
                    self.grids.kx[idx] = key;
                    self.grids.ox.set(idx, pos);
                } else {
                    self.grids.kx[idx] = 0;
                }
            }
        }
        let _ = (bx, by, bz); // block coords unused beyond lighting sample granularity
    }
    fn toggle_y(
        &mut self,
        bx: usize,
        by: usize,
        bz: usize,
        iy: usize,
        x0: usize,
        x1: usize,
        z0: usize,
        z1: usize,
        pos: bool,
        mid: MaterialId,
        l: u8,
    ) {
        let key = self.keys.ensure(mid, l);
        for iz in z0..z1 {
            for ix in x0..x1 {
                let idx = self.grids.idx_y(ix, iy, iz);
                self.grids.py.toggle(idx);
                if self.grids.py.get(idx) {
                    self.grids.ky[idx] = key;
                    self.grids.oy.set(idx, pos);
                } else {
                    self.grids.ky[idx] = 0;
                }
            }
        }
        let _ = (bx, by, bz);
    }
    fn toggle_z(
        &mut self,
        bx: usize,
        by: usize,
        bz: usize,
        iz: usize,
        x0: usize,
        x1: usize,
        y0: usize,
        y1: usize,
        pos: bool,
        mid: MaterialId,
        l: u8,
    ) {
        let key = self.keys.ensure(mid, l);
        for iy in y0..y1 {
            for ix in x0..x1 {
                let idx = self.grids.idx_z(ix, iy, iz);
                self.grids.pz.toggle(idx);
                if self.grids.pz.get(idx) {
                    self.grids.kz[idx] = key;
                    self.grids.oz.set(idx, pos);
                } else {
                    self.grids.kz[idx] = 0;
                }
            }
        }
        let _ = (bx, by, bz);
    }

    pub fn add_cube(&mut self, x: usize, y: usize, z: usize, b: Block) {
        let S = self.S;
        let x0 = x * S;
        let x1 = (x + 1) * S;
        let y0 = y * S;
        let y1 = (y + 1) * S;
        let z0 = z * S;
        let z1 = (z + 1) * S;
        let mid_for = |f: Face| registry_material_for_or_unknown(b, f, self.reg);
        self.toggle_box(x, y, z, (x0, x1, y0, y1, z0, z1), mid_for);
    }
    /// Water meshing path: only toggle faces against air to avoid occluding terrain under water.
    pub fn add_water_cube(&mut self, x: usize, y: usize, z: usize, b: Block) {
        let S = self.S;
        let x0 = x * S;
        let x1 = (x + 1) * S;
        let y0 = y * S;
        let y1 = (y + 1) * S;
        let z0 = z * S;
        let z1 = (z + 1) * S;
        let mid_for = |f: Face| registry_material_for_or_unknown(b, f, self.reg);
        let wx = self.base_x + x as i32;
        let wy = y as i32;
        let wz = self.base_z + z as i32;
        let air_id = self.reg.id_by_name("air").unwrap_or(0);

        // +X neighbor
        if self.world_block(wx + 1, wy, wz).id == air_id {
            self.toggle_x(
                x,
                y,
                z,
                x1,
                y0,
                y1,
                z0,
                z1,
                true,
                mid_for(Face::PosX),
                self.light_bin(x, y, z, Face::PosX),
            );
        }
        // -X neighbor
        if self.world_block(wx - 1, wy, wz).id == air_id {
            self.toggle_x(
                x,
                y,
                z,
                x0,
                y0,
                y1,
                z0,
                z1,
                false,
                mid_for(Face::NegX),
                self.light_bin(x, y, z, Face::NegX),
            );
        }
        // +Y neighbor
        if self.world_block(wx, wy + 1, wz).id == air_id {
            self.toggle_y(
                x,
                y,
                z,
                y1,
                x0,
                x1,
                z0,
                z1,
                true,
                mid_for(Face::PosY),
                self.light_bin(x, y, z, Face::PosY),
            );
        }
        // -Y neighbor
        if self.world_block(wx, wy - 1, wz).id == air_id {
            self.toggle_y(
                x,
                y,
                z,
                y0,
                x0,
                x1,
                z0,
                z1,
                false,
                mid_for(Face::NegY),
                self.light_bin(x, y, z, Face::NegY),
            );
        }
        // +Z neighbor
        if self.world_block(wx, wy, wz + 1).id == air_id {
            self.toggle_z(
                x,
                y,
                z,
                z1,
                x0,
                x1,
                y0,
                y1,
                true,
                mid_for(Face::PosZ),
                self.light_bin(x, y, z, Face::PosZ),
            );
        }
        // -Z neighbor
        if self.world_block(wx, wy, wz - 1).id == air_id {
            self.toggle_z(
                x,
                y,
                z,
                z0,
                x0,
                x1,
                y0,
                y1,
                false,
                mid_for(Face::NegZ),
                self.light_bin(x, y, z, Face::NegZ),
            );
        }
    }
    pub fn add_micro(&mut self, x: usize, y: usize, z: usize, b: Block, occ: u8) {
        use crate::microgrid_tables::occ8_to_boxes;
        let S = self.S;
        let mid_for = |f: Face| registry_material_for_or_unknown(b, f, self.reg);
        for mb in occ8_to_boxes(occ) {
            let bx0 = x * S + (mb[0] as usize);
            let by0 = y * S + (mb[1] as usize);
            let bz0 = z * S + (mb[2] as usize);
            let bx1 = x * S + (mb[3] as usize);
            let by1 = y * S + (mb[4] as usize);
            let bz1 = z * S + (mb[5] as usize);
            self.toggle_box(x, y, z, (bx0, bx1, by0, by1, bz0, bz1), mid_for);
        }
    }

    pub fn emit_into(&self, builds: &mut HashMap<MaterialId, MeshBuild>) {
        let S = self.S as f32;
        let scale = 1.0 / S;
        let (sx, sy, sz) = (self.sx, self.sy, self.sz);
        // X planes: ix in [0, S*sx) (skip +X at ix==S*sx)
        let width_x = self.S * sz;
        let height_x = self.S * sy;
        for ix in 0..(self.S * sx) {
            let mut mask: Vec<Option<((MaterialId, bool), u8)>> = vec![None; width_x * height_x];
            for iy in 0..height_x {
                for iz in 0..width_x {
                    let idx = self.grids.idx_x(ix, iy, iz);
                    if self.grids.px.get(idx) {
                        let key = self.grids.kx[idx];
                        if key != 0 {
                            let (mid, l) = self.keys.get(key);
                            let pos = self.grids.ox.get(idx);
                            mask[iy * width_x + iz] = Some(((mid, pos), l));
                        }
                    }
                }
            }
            // Overscan enabled: parity already includes neighbor seam toggles; no seam-specific mask edits here.
            greedy_rects(width_x, height_x, &mut mask, |u0, v0, w, h, codev| {
                let ((mid, pos), l) = codev;
                let lv = apply_min_light(l, Some(VISUAL_LIGHT_MIN));
                let rgba = [lv, lv, lv, 255];
                let face = if pos { Face::PosX } else { Face::NegX };
                let fx = (self.base_x as f32) + (ix as f32) * scale;
                let origin = Vec3 {
                    x: fx,
                    y: (v0 as f32) * scale,
                    z: (self.base_z as f32) + (u0 as f32) * scale,
                };
                let u1 = (w as f32) * scale;
                let v1 = (h as f32) * scale;
                emit_face_rect_for_clipped(
                    builds,
                    mid,
                    face,
                    origin,
                    u1,
                    v1,
                    rgba,
                    self.base_x,
                    self.sx,
                    self.sy,
                    self.base_z,
                    self.sz,
                );
            });
        }
        // Y planes
        let width_y = self.S * sx;
        let height_y = self.S * sz;
        for iy in 0..(self.S * sy) {
            let mut mask: Vec<Option<((MaterialId, bool), u8)>> = vec![None; width_y * height_y];
            for iz in 0..height_y {
                for ix in 0..width_y {
                    let idx = self.grids.idx_y(ix, iy, iz);
                    if self.grids.py.get(idx) {
                        let key = self.grids.ky[idx];
                        if key != 0 {
                            let (mid, l) = self.keys.get(key);
                            let pos = self.grids.oy.get(idx);
                            mask[iz * width_y + ix] = Some(((mid, pos), l));
                        }
                    }
                }
            }
            greedy_rects(width_y, height_y, &mut mask, |u0, v0, w, h, codev| {
                let ((mid, pos), l) = codev;
                let lv = apply_min_light(l, Some(VISUAL_LIGHT_MIN));
                let rgba = [lv, lv, lv, 255];
                let face = if pos { Face::PosY } else { Face::NegY };
                let fy = (iy as f32) * scale;
                let origin = Vec3 {
                    x: (self.base_x as f32) + (u0 as f32) * scale,
                    y: fy,
                    z: (self.base_z as f32) + (v0 as f32) * scale,
                };
                let u1 = (w as f32) * scale;
                let v1 = (h as f32) * scale;
                emit_face_rect_for_clipped(
                    builds,
                    mid,
                    face,
                    origin,
                    u1,
                    v1,
                    rgba,
                    self.base_x,
                    self.sx,
                    self.sy,
                    self.base_z,
                    self.sz,
                );
            });
        }
        // Z planes
        let width_z = self.S * sx;
        let height_z = self.S * sy;
        for iz in 0..(self.S * sz) {
            let mut mask: Vec<Option<((MaterialId, bool), u8)>> = vec![None; width_z * height_z];
            for iy in 0..height_z {
                for ix in 0..width_z {
                    let idx = self.grids.idx_z(ix, iy, iz);
                    if self.grids.pz.get(idx) {
                        let key = self.grids.kz[idx];
                        if key != 0 {
                            let (mid, l) = self.keys.get(key);
                            let pos = self.grids.oz.get(idx);
                            mask[iy * width_z + ix] = Some(((mid, pos), l));
                        }
                    }
                }
            }
            // Overscan enabled: parity already includes neighbor seam toggles; no seam-specific mask edits here.
            greedy_rects(width_z, height_z, &mut mask, |u0, v0, w, h, codev| {
                let ((mid, pos), l) = codev;
                let lv = apply_min_light(l, Some(VISUAL_LIGHT_MIN));
                let rgba = [lv, lv, lv, 255];
                let face = if pos { Face::PosZ } else { Face::NegZ };
                let fz = (self.base_z as f32) + (iz as f32) * scale;
                let origin = Vec3 {
                    x: (self.base_x as f32) + (u0 as f32) * scale,
                    y: (v0 as f32) * scale,
                    z: fz,
                };
                let u1 = (w as f32) * scale;
                let v1 = (h as f32) * scale;
                emit_face_rect_for_clipped(
                    builds,
                    mid,
                    face,
                    origin,
                    u1,
                    v1,
                    rgba,
                    self.base_x,
                    self.sx,
                    self.sy,
                    self.base_z,
                    self.sz,
                );
            });
        }
    }
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
                let face_material = |face: Face| {
                    reg.get(here.id)
                        .map(|ty| ty.material_for_cached(face.role(), here.state))
                        .unwrap_or_else(|| unknown_material_id(reg))
                };
                // Prefer microgrid occupancy for shapes like slabs/stairs
                if let Some(ty) = reg.get(here.id) {
                    let var = ty.variant(here.state);
                    if let Some(occ) = var.occupancy {
                        let face_material =
                            |face: Face| ty.material_for_cached(face.role(), here.state);
                        for (min, max) in microgrid_boxes(fx, fy, fz, occ) {
                            emit_box_generic_clipped(
                                &mut builds,
                                min,
                                max,
                                &face_material,
                                |_face| false,
                                |_face| ambient,
                                base_x,
                                buf.sx,
                                buf.sy,
                                base_z,
                                buf.sz,
                            );
                        }
                        continue;
                    }
                }
                match reg.get(here.id).map(|t| &t.shape) {
                    Some(geist_blocks::types::Shape::Cube)
                    | Some(geist_blocks::types::Shape::AxisCube { .. }) => {
                        let min = Vec3 {
                            x: fx,
                            y: fy,
                            z: fz,
                        };
                        let max = Vec3 {
                            x: fx + 1.0,
                            y: fy + 1.0,
                            z: fz + 1.0,
                        };
                        emit_box_generic_clipped(
                            &mut builds,
                            min,
                            max,
                            &face_material,
                            |_face| false,
                            |_face| ambient,
                            base_x,
                            buf.sx,
                            buf.sy,
                            base_z,
                            buf.sz,
                        );
                    }
                    Some(geist_blocks::types::Shape::Slab { .. }) => {
                        let top = is_top_half_shape(here, reg);
                        let h = 0.5f32;
                        let min = Vec3 {
                            x: fx,
                            y: if top { fy + 0.5 } else { fy },
                            z: fz,
                        };
                        let max = Vec3 {
                            x: fx + 1.0,
                            y: if top { fy + 1.0 } else { fy + 0.5 },
                            z: fz + 1.0,
                        };
                        emit_box_generic_clipped(
                            &mut builds,
                            min,
                            max,
                            &face_material,
                            |_face| false,
                            |_face| ambient,
                            base_x,
                            buf.sx,
                            buf.sy,
                            base_z,
                            buf.sz,
                        );
                    }
                    Some(geist_blocks::types::Shape::Pane) => {
                        let t = 0.0625f32;
                        let min = Vec3 {
                            x: fx + 0.5 - t,
                            y: fy,
                            z: fz,
                        };
                        let max = Vec3 {
                            x: fx + 0.5 + t,
                            y: fy + 1.0,
                            z: fz + 1.0,
                        };
                        emit_box_generic_clipped(
                            &mut builds,
                            min,
                            max,
                            &face_material,
                            |_face| false,
                            |_face| ambient,
                            base_x,
                            buf.sx,
                            buf.sy,
                            base_z,
                            buf.sz,
                        );
                    }
                    Some(geist_blocks::types::Shape::Fence) => {
                        let t = 0.125f32;
                        let p = 0.375f32;
                        let mut boxes: Vec<(Vec3, Vec3)> = Vec::new();
                        boxes.push((
                            Vec3 {
                                x: fx + 0.5 - t,
                                y: fy,
                                z: fz + 0.5 - t,
                            },
                            Vec3 {
                                x: fx + 0.5 + t,
                                y: fy + 1.0,
                                z: fz + 0.5 + t,
                            },
                        ));
                        boxes.push((
                            Vec3 {
                                x: fx,
                                y: fy + p,
                                z: fz + 0.5 - t,
                            },
                            Vec3 {
                                x: fx + 1.0,
                                y: fy + 1.0 - p,
                                z: fz + 0.5 + t,
                            },
                        ));
                        boxes.push((
                            Vec3 {
                                x: fx + 0.5 - t,
                                y: fy + p,
                                z: fz,
                            },
                            Vec3 {
                                x: fx + 0.5 + t,
                                y: fy + 1.0 - p,
                                z: fz + 1.0,
                            },
                        ));
                        for (min, max) in boxes {
                            emit_box_generic_clipped(
                                &mut builds,
                                min,
                                max,
                                &face_material,
                                |_face| false,
                                |_face| ambient,
                                base_x,
                                buf.sx,
                                buf.sy,
                                base_z,
                                buf.sz,
                            );
                        }
                    }
                    Some(geist_blocks::types::Shape::Gate { .. }) => {
                        let t = 0.125f32;
                        let y0 = 0.375f32;
                        let y1 = 0.625f32;
                        let mut boxes: Vec<(Vec3, Vec3)> = Vec::new();
                        boxes.push((
                            Vec3 {
                                x: fx + 0.0,
                                y: fy + y0,
                                z: fz + 0.5 - t,
                            },
                            Vec3 {
                                x: fx + 1.0,
                                y: fy + y0 + t,
                                z: fz + 0.5 + t,
                            },
                        ));
                        boxes.push((
                            Vec3 {
                                x: fx + 0.0,
                                y: fy + y1,
                                z: fz + 0.5 - t,
                            },
                            Vec3 {
                                x: fx + 1.0,
                                y: fy + y1 + t,
                                z: fz + 0.5 + t,
                            },
                        ));
                        for (min, max) in boxes {
                            emit_box_generic_clipped(
                                &mut builds,
                                min,
                                max,
                                &face_material,
                                |_face| false,
                                |_face| ambient,
                                base_x,
                                buf.sx,
                                buf.sy,
                                base_z,
                                buf.sz,
                            );
                        }
                    }
                    Some(geist_blocks::types::Shape::Carpet) => {
                        let h = 0.0625f32;
                        let min = Vec3 {
                            x: fx,
                            y: fy,
                            z: fz,
                        };
                        let max = Vec3 {
                            x: fx + 1.0,
                            y: fy + h,
                            z: fz + 1.0,
                        };
                        emit_box_generic_clipped(
                            &mut builds,
                            min,
                            max,
                            &face_material,
                            |_face| false,
                            |_face| ambient,
                            base_x,
                            buf.sx,
                            buf.sy,
                            base_z,
                            buf.sz,
                        );
                    }
                    _ => {}
                }
            }
        }
    }
    ChunkMeshCPU {
        cx: buf.cx,
        cz: buf.cz,
        bbox: Aabb {
            min: Vec3 {
                x: base_x as f32,
                y: 0.0,
                z: base_z as f32,
            },
            max: Vec3 {
                x: base_x as f32 + buf.sx as f32,
                y: buf.sy as f32,
                z: base_z as f32 + buf.sz as f32,
            },
        },
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
    pub fn normal(self) -> Vec3 {
        match self {
            Face::PosY => Vec3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            Face::NegY => Vec3 {
                x: 0.0,
                y: -1.0,
                z: 0.0,
            },
            Face::PosX => Vec3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Face::NegX => Vec3 {
                x: -1.0,
                y: 0.0,
                z: 0.0,
            },
            Face::PosZ => Vec3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            Face::NegZ => Vec3 {
                x: 0.0,
                y: 0.0,
                z: -1.0,
            },
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
        .map(|t| {
            t.is_solid(nb.state)
                && matches!(
                    t.shape,
                    geist_blocks::types::Shape::Cube | geist_blocks::types::Shape::AxisCube { .. }
                )
        })
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

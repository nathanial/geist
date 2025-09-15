use std::collections::HashMap;

use geist_blocks::types::MaterialId;
use geist_geom::Vec3;

use crate::mesh_build::MeshBuild;
use crate::face::Face;
use crate::constants::OPAQUE_ALPHA;

// Fast-path sink for writing into per-material mesh buffers without HashMap overhead.
pub trait BuildSink {
    fn get_build_mut(&mut self, mid: MaterialId) -> &mut MeshBuild;
}

impl BuildSink for HashMap<MaterialId, MeshBuild> {
    #[inline]
    fn get_build_mut(&mut self, mid: MaterialId) -> &mut MeshBuild {
        self.entry(mid).or_default()
    }
}

impl BuildSink for Vec<MeshBuild> {
    #[inline]
    fn get_build_mut(&mut self, mid: MaterialId) -> &mut MeshBuild {
        let ix = mid.0 as usize;
        let mb = &mut self[ix];
        if mb.pos.capacity() == 0 {
            // Lazy small reserve to reduce early reallocs when a material is first used in a chunk
            const INITIAL_QUAD_CAP: usize = 256; // tune as needed
            mb.reserve_quads(INITIAL_QUAD_CAP);
        }
        mb
    }
}

#[inline]
/// Emits a face-aligned rectangle into the material's mesh build.
pub(crate) fn emit_face_rect_for(
    builds: &mut impl BuildSink,
    mid: MaterialId,
    face: Face,
    origin: Vec3,
    u1: f32,
    v1: f32,
    rgba: [u8; 4],
) {
    let mb = builds.get_build_mut(mid);
    mb.add_face_rect(face, origin, u1, v1, false, rgba);
}

/// Clips a face-aligned rectangle to the current chunk interior and emits any visible portion.
/// Chunk interior bounds: X in [base_x, base_x+sx), Z in [base_z, base_z+sz), Y in [0, sy).
#[inline]
pub(crate) fn emit_face_rect_for_clipped(
    builds: &mut impl BuildSink,
    mid: MaterialId,
    face: Face,
    origin: Vec3,
    u1: f32,
    v1: f32,
    rgba: [u8; 4],
    base_x: i32,
    sx: usize,
    sy: usize,
    base_z: i32,
    sz: usize,
) {
    #[inline]
    fn clip_span(start: f32, len: f32, lo: f32, hi: f32) -> Option<(f32, f32)> {
        let s0 = start.max(lo);
        let s1 = (start + len).min(hi);
        if s1 <= s0 { None } else { Some((s0, s1 - s0)) }
    }
    let bx0 = base_x as f32;
    let bx1 = (base_x + sx as i32) as f32;
    let bz0 = base_z as f32;
    let bz1 = (base_z + sz as i32) as f32;
    let by0 = 0.0f32;
    let by1 = sy as f32;

    let mut out = None;
    match face {
        Face::PosX | Face::NegX => {
            if origin.x >= bx0 && origin.x < bx1 {
                if let Some((z, u)) = clip_span(origin.z, u1, bz0, bz1) {
                    if let Some((y, v)) = clip_span(origin.y, v1, by0, by1) {
                        let mut o = origin;
                        o.z = z;
                        o.y = y;
                        out = Some((o, u, v));
                    }
                }
            }
        }
        Face::PosZ | Face::NegZ => {
            if origin.z >= bz0 && origin.z < bz1 {
                if let Some((x, u)) = clip_span(origin.x, u1, bx0, bx1) {
                    if let Some((y, v)) = clip_span(origin.y, v1, by0, by1) {
                        let mut o = origin;
                        o.x = x;
                        o.y = y;
                        out = Some((o, u, v));
                    }
                }
            }
        }
        Face::PosY | Face::NegY => {
            if origin.y >= by0 && origin.y < by1 {
                if let Some((x, u)) = clip_span(origin.x, u1, bx0, bx1) {
                    if let Some((z, v)) = clip_span(origin.z, v1, bz0, bz1) {
                        let mut o = origin;
                        o.x = x;
                        o.z = z;
                        out = Some((o, u, v));
                    }
                }
            }
        }
    }
    if let Some((o, cu, cv)) = out {
        emit_face_rect_for(builds, mid, face, o, cu, cv, rgba);
    }
}

#[inline]
/// Emits up to six faces of an axis-aligned box using a chooser to pick material and color.
pub(crate) fn emit_box_faces(
    builds: &mut impl BuildSink,
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
            let (_u1, _v1) = match face {
                Face::PosY | Face::NegY => (max.x - min.x, max.z - min.z),
                Face::PosX | Face::NegX => (max.z - min.z, max.y - min.y),
                Face::PosZ | Face::NegZ => (max.x - min.x, max.y - min.y),
            };
            let n = Vec3 { x: normal.0, y: normal.1, z: normal.2 };
            // Absolute UVs anchored to world-space
            let a = corners[indices[0]];
            let b = corners[indices[1]];
            let c = corners[indices[2]];
            let d = corners[indices[3]];
            let uv_from = |p: Vec3| match face {
                Face::PosY | Face::NegY => (p.x, p.z),
                Face::PosX | Face::NegX => (p.z, p.y),
                Face::PosZ | Face::NegZ => (p.x, p.y),
            };
            let uvs = [uv_from(a), uv_from(d), uv_from(c), uv_from(b)];
            builds.get_build_mut(mid).add_quad_uv(a, b, c, d, n, uvs, false, rgba);
        }
    }
}

#[inline]
/// Emits a box, skipping faces that occlude and sampling light per face.
pub(crate) fn emit_box_generic(
    builds: &mut impl BuildSink,
    min: Vec3,
    max: Vec3,
    fm_for_face: &dyn Fn(Face) -> MaterialId,
    mut occludes: impl FnMut(Face) -> bool,
    mut sample_light: impl FnMut(Face) -> u8,
) {
    emit_box_faces(builds, min, max, |face| {
        if occludes(face) { return None; }
        let lv = sample_light(face);
        let rgba = [lv, lv, lv, OPAQUE_ALPHA];
        let mid = fm_for_face(face);
        Some((mid, rgba))
    });
}

#[inline]
/// Emits a box clipped to the chunk interior using occlusion and per-face light sampling.
pub(crate) fn emit_box_generic_clipped(
    builds: &mut impl BuildSink,
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
    if !(min.x < max.x && min.y < max.y && min.z < max.z) { return; }
    emit_box_generic(builds, min, max, fm_for_face, occludes, sample_light);
}

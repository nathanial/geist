use std::collections::HashMap;
use std::hash::Hash;

use geist_blocks::BlockRegistry;
use geist_blocks::types::{Block, MaterialId};
use geist_chunk::ChunkBuf;
use geist_world::World;
use geist_geom::Vec3;

use crate::face::Face;
use crate::constants::MICRO_HALF_STEP_SIZE;

// Visual-only lighting floor to avoid pitch-black faces in darkness.
// Does not affect logical light propagation.
pub(crate) const VISUAL_LIGHT_MIN: u8 = 18; // ~7% brightness floor

#[inline]
/// Returns the "unknown" material id, or `0` if missing from the registry.
pub(crate) fn unknown_material_id(reg: &BlockRegistry) -> MaterialId {
    reg.materials.get_id("unknown").unwrap_or(MaterialId(0))
}

#[inline]
/// Looks up the material for a block's face; falls back to `unknown` if not found.
pub(crate) fn registry_material_for_or_unknown(
    block: Block,
    face: Face,
    reg: &BlockRegistry,
) -> MaterialId {
    reg.get(block.id)
        .map(|ty| ty.material_for_cached(face.role(), block.state))
        .unwrap_or_else(|| unknown_material_id(reg))
}

#[inline]
/// Returns whether the block is solid at runtime according to its type.
pub(crate) fn is_solid_runtime(b: Block, reg: &BlockRegistry) -> bool {
    reg.get(b.id)
        .map(|ty| ty.is_solid(b.state))
        .unwrap_or(false)
}

#[inline]
/// Returns true if the block's shape is a top-half variant (slab or stairs).
pub(crate) fn is_top_half_shape(b: Block, reg: &BlockRegistry) -> bool {
    reg.get(b.id).map_or(false, |ty| match &ty.shape {
        geist_blocks::types::Shape::Slab { half_from }
        | geist_blocks::types::Shape::Stairs { half_from, .. } => {
            ty.state_prop_is_value(b.state, half_from, "top")
        }
        _ => false,
    })
}

#[inline]
/// Checks whether the neighbor block in the given face direction is a Pane.
pub(crate) fn neighbor_is_pane(
    buf: &ChunkBuf,
    reg: &BlockRegistry,
    wx: i32,
    wy: i32,
    wz: i32,
    face: Face,
) -> bool {
    let (dx, dy, dz) = face.delta();
    if let Some(nb) = buf.get_world(wx + dx, wy + dy, wz + dz) {
        if let Some(nb_ty) = reg.get(nb.id) {
            return matches!(nb_ty.shape, geist_blocks::types::Shape::Pane);
        }
    }
    false
}

#[inline]
/// True if the block is a full solid cube (including axis-aligned variants).
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
/// Returns the cached occlusion mask for a block or `0` if unknown.
pub(crate) fn occlusion_mask_for(reg: &BlockRegistry, nb: Block) -> u8 {
    reg.get(nb.id)
        .map(|ty| ty.occlusion_mask_cached(nb.state))
        .unwrap_or(0)
}

#[inline]
/// Returns true if the block occludes the given face.
pub(crate) fn occludes_face(nb: Block, face: Face, reg: &BlockRegistry) -> bool {
    let mask = occlusion_mask_for(reg, nb);
    (mask >> face.index()) & 1 == 1
}

#[inline]
/// Determines if the neighbor at `(nx,ny,nz)` occludes the face of `here`, using edits/world as needed.
pub(crate) fn is_occluder(
    buf: &ChunkBuf,
    world: &World,
    edits: Option<&HashMap<(i32, i32, i32), Block>>,
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

#[inline]
/// Greedy rectangle packing over a 2D mask; calls `emit` for each rectangle found.
pub(crate) fn greedy_rects<K: Copy + Eq + Hash>(
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
/// Applies an optional minimum light floor to avoid too-dark faces in rendering.
pub(crate) fn apply_min_light(l: u8, min: Option<u8>) -> u8 {
    if let Some(m) = min { l.max(m) } else { l }
}

#[inline]
/// Converts a micro occupancy mask into world-space AABBs at half-step resolution.
pub(crate) fn microgrid_boxes(fx: f32, fy: f32, fz: f32, occ: u8) -> Vec<(Vec3, Vec3)> {
    use crate::microgrid_tables::occ8_to_boxes;
    let cell = MICRO_HALF_STEP_SIZE;
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

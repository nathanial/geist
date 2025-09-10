//! Shared S=2 micro-grid helpers for occupancy and face openness.
#![forbid(unsafe_code)]

use crate::{Block, BlockRegistry};

#[inline]
fn occ8_for(reg: &BlockRegistry, b: Block) -> Option<u8> {
    reg.get(b.id).and_then(|ty| ty.variant(b.state).occupancy)
}

#[inline]
fn occ_bit(occ: u8, x: usize, y: usize, z: usize) -> bool {
    let idx = ((y & 1) << 2) | ((z & 1) << 1) | (x & 1);
    (occ & (1u8 << idx)) != 0
}

#[inline]
fn is_full_cube(reg: &BlockRegistry, b: Block) -> bool {
    reg.get(b.id)
        .map(|ty| ty.is_solid(b.state)
            && matches!(ty.shape, crate::types::Shape::Cube | crate::types::Shape::AxisCube { .. }))
        .unwrap_or(false)
}

/// Returns true if the micro voxel at local micro coordinates (mx,my,mz) is solid for S=2.
/// Coordinates are in {0,1} for each axis.
#[inline]
pub fn micro_cell_solid_s2(reg: &BlockRegistry, b: Block, mx: usize, my: usize, mz: usize) -> bool {
    if is_full_cube(reg, b) {
        return true;
    }
    if let Some(occ) = occ8_for(reg, b) {
        return occ_bit(occ, mx, my, mz);
    }
    false
}

/// Returns true if the micro face cell on `face` between `here` and `there` is open (not sealed) for S=2.
/// - `face`: 0=+Y,1=-Y,2=+X,3=-X,4=+Z,5=-Z
/// - `i0`,`i1` are the two micro indices (0 or 1) along the face plane's axes.
/// Seam policy `dont_occlude_same` is applied from the `here` type: identical neighbor blocks may be ignored as occluders.
#[inline]
pub fn micro_face_cell_open_s2(
    reg: &BlockRegistry,
    here: Block,
    there: Block,
    face: usize,
    i0: usize,
    i1: usize,
) -> bool {
    let (a, b) = match face {
        2 => ((1, i0, i1), (0, i0, i1)), // +X: here(x=1), there(x=0)
        3 => ((0, i0, i1), (1, i0, i1)), // -X: here(x=0), there(x=1)
        0 => ((i0, 1, i1), (i0, 0, i1)), // +Y: here(y=1), there(y=0)
        1 => ((i0, 0, i1), (i0, 1, i1)), // -Y: here(y=0), there(y=1)
        4 => ((i0, i1, 1), (i0, i1, 0)), // +Z: here(z=1), there(z=0)
        5 => ((i0, i1, 0), (i0, i1, 1)), // -Z: here(z=0), there(z=1)
        _ => return true,
    };
    let local_solid = micro_cell_solid_s2(reg, here, a.0, a.1, a.2);
    // Neighbor solid may be ignored when seam policy says identical neighbors don't occlude
    let ignore_same = reg
        .get(here.id)
        .map(|t| t.seam.dont_occlude_same && here.id == there.id)
        .unwrap_or(false);
    let neighbor_solid = if ignore_same {
        false
    } else {
        micro_cell_solid_s2(reg, there, b.0, b.1, b.2)
    };
    // Face cell is open if neither side occupies the face-adjacent micro voxel
    !(local_solid || neighbor_solid)
}


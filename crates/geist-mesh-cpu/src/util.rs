use hashbrown::HashMap;

use geist_blocks::BlockRegistry;
use geist_blocks::types::Block;
use geist_chunk::ChunkBuf;
use geist_world::World;

use crate::face::Face;

// Visual lighting floor logic removed; renderer handles tone mapping and fog.

#[inline]
/// Returns whether the block is solid at runtime according to its type.
pub(crate) fn is_solid_runtime(b: Block, reg: &BlockRegistry) -> bool {
    reg.get(b.id)
        .map(|ty| ty.is_solid(b.state))
        .unwrap_or(false)
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
    world: Option<&World>,
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
        let x0 = buf.coord.cx * buf.sx as i32;
        let y0 = buf.coord.cy * buf.sy as i32;
        let z0 = buf.coord.cz * buf.sz as i32;
        if ny < y0 || ny >= y0 + buf.sy as i32 {
            return false;
        }
        let lx = (nx - x0) as usize;
        let ly = (ny - y0) as usize;
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
        if let Some(b) = es.get(&(nx, ny, nz)) {
            *b
        } else if let Some(world) = world {
            world.block_at_runtime(reg, nx, ny, nz)
        } else {
            Block::AIR
        }
    } else if let Some(world) = world {
        world.block_at_runtime(reg, nx, ny, nz)
    } else {
        Block::AIR
    };
    if let (Some(h), Some(_n)) = (reg.get(here.id), reg.get(nb.id)) {
        if h.seam.dont_occlude_same && here.id == nb.id {
            return false;
        }
    }
    occludes_face(nb, face, reg)
}

// apply_min_light deprecated; keep behavior in shaders if needed.

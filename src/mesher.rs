use crate::blocks::{Block, BlockRegistry, MaterialCatalog, MaterialId};
use crate::chunkbuf::ChunkBuf;
use crate::lighting::{LightBorders, LightGrid, LightingStore};
use crate::meshutil::{Face, Facing, SIDE_NEIGHBORS, is_full_cube};
use crate::texture_cache::TextureCache;
use crate::voxel::World;
use raylib::core::math::BoundingBox;
use raylib::prelude::*;
use std::collections::HashMap as StdHashMap;

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
}

#[inline]
fn unknown_material_id(reg: &BlockRegistry) -> MaterialId {
    reg.materials.get_id("unknown").unwrap_or(MaterialId(0))
}

#[inline]
fn registry_material_for(block: Block, face: Face, reg: &BlockRegistry) -> Option<MaterialId> {
    reg.get(block.id)
        .map(|ty| ty.material_for_cached(face.role(), block.state))
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
    mut light_for_neighbor: impl FnMut(usize, usize, usize, Face, bool) -> u8,
) {
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
        if !is_full_cube(reg, nb) {
            continue;
        }
        let mid = registry_material_for_or_unknown(nb, face, reg);
        match face {
            Face::PosX | Face::NegX => {
                let bx = if dx < 0 { 0 } else { 1 }; // boundary column in micro-grid along X
                for ly in 0..2 {
                    let y0 = fy + (ly as f32) * cell;
                    let e0 = (occ & micro_bit(bx, ly, 0)) == 0;
                    let e1 = (occ & micro_bit(bx, ly, 1)) == 0;
                    if e0 || e1 {
                        let (z_idx, span) = if e0 && e1 { (0, 2) } else if e0 { (0, 1) } else { (1, 1) };
                        let z0 = fz + (z_idx as f32) * cell;
                        let draw_top = ly == 1;
                        let lv = light_for_neighbor(nx as usize, y, nz as usize, face, draw_top);
                        let rgba = [lv, lv, lv, 255];
                        let origin = Vector3::new(fx + x_off, y0, z0);
                        emit_face_rect_for(builds, mid, face, origin, span as f32 * cell, cell, rgba);
                    }
                }
            }
            Face::PosZ | Face::NegZ => {
                let bz = if dz < 0 { 0 } else { 1 }; // boundary row in micro-grid along Z
                for ly in 0..2 {
                    let y0 = fy + (ly as f32) * cell;
                    let e0 = (occ & micro_bit(0, ly, bz)) == 0;
                    let e1 = (occ & micro_bit(1, ly, bz)) == 0;
                    if e0 || e1 {
                        let (x_idx, span) = if e0 && e1 { (0, 2) } else if e0 { (0, 1) } else { (1, 1) };
                        let x0 = fx + (x_idx as f32) * cell;
                        let draw_top = ly == 1;
                        let lv = light_for_neighbor(nx as usize, y, nz as usize, face, draw_top);
                        let rgba = [lv, lv, lv, 255];
                        let origin = Vector3::new(x0, y0, fz + z_off);
                        emit_face_rect_for(builds, mid, face, origin, span as f32 * cell, cell, rgba);
                    }
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
        if !is_full_cube(reg, nb) {
            continue;
        }
        let mid = registry_material_for_or_unknown(nb, face, reg);
        let ly = if dy < 0 { 0 } else { 1 };
        // Build 2x2 empty grid on boundary layer (X×Z)
        let mut grid = [[false; 2]; 2];
        for lz in 0..2 {
            for lx in 0..2 {
                grid[lz][lx] = (occ & micro_bit(lx, ly, lz)) == 0;
            }
        }
        let mut used = [[false; 2]; 2];
        for lz in 0..2 {
            for lx in 0..2 {
                if !grid[lz][lx] || used[lz][lx] {
                    continue;
                }
                let w = if lx == 0 && grid[lz][1] && !used[lz][1] { 2 } else { 1 };
                let h = if lz == 0 {
                    let mut ok = true;
                    for xi in lx..(lx + w) {
                        if !grid[1][xi] || used[1][xi] {
                            ok = false;
                            break;
                        }
                    }
                    if ok { 2 } else { 1 }
                } else {
                    1
                };
                for dz in 0..h {
                    for dx in 0..w {
                        used[lz + dz][lx + dx] = true;
                    }
                }
                let x0 = fx + (lx as f32) * cell;
                let z0 = fz + (lz as f32) * cell;
                let y0 = if dy < 0 { fy } else { fy + 1.0 };
                let draw_top = ly == 1;
                let lv = light_for_neighbor(x, ny as usize, z, face, draw_top);
                let rgba = [lv, lv, lv, 255];
                let origin = Vector3::new(x0, y0, z0);
                emit_face_rect_for(
                    builds,
                    mid,
                    face,
                    origin,
                    w as f32 * cell,
                    h as f32 * cell,
                    rgba,
                );
            }
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

#[inline]
fn face_light(face: Face, ambient: u8) -> u8 {
    match face {
        Face::PosY => ambient.saturating_add(40),
        Face::NegY => ambient.saturating_sub(60),
        _ => ambient,
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

#[inline]
fn micro_occupancy_slab(is_top: bool) -> u8 {
    // Fill top or bottom y-layer fully (4 cells)
    let y = if is_top { 1 } else { 0 };
    micro_bit(0, y, 0)
        | micro_bit(1, y, 0)
        | micro_bit(0, y, 1)
        | micro_bit(1, y, 1)
}

#[inline]
fn micro_occupancy_stairs(facing: Facing, is_top: bool) -> u8 {
    // Major half-slab on chosen y, plus riser on opposite y occupying a half-plane along facing.
    let y_major = if is_top { 1 } else { 0 };
    let y_minor = 1 - y_major;
    let full_layer = micro_bit(0, y_major, 0)
        | micro_bit(1, y_major, 0)
        | micro_bit(0, y_major, 1)
        | micro_bit(1, y_major, 1);
    let half_minor = match facing {
        Facing::North => micro_bit(0, y_minor, 0) | micro_bit(1, y_minor, 0),
        Facing::South => micro_bit(0, y_minor, 1) | micro_bit(1, y_minor, 1),
        Facing::West => micro_bit(0, y_minor, 0) | micro_bit(0, y_minor, 1),
        Facing::East => micro_bit(1, y_minor, 0) | micro_bit(1, y_minor, 1),
    };
    full_layer | half_minor
}

#[inline]
fn microgrid_boxes(fx: f32, fy: f32, fz: f32, occ: u8) -> Vec<(Vector3, Vector3)> {
    // Greedy 2D merge per y-layer on a 2x2 grid; each rect becomes an AABB of height 0.5.
    let mut out = Vec::new();
    let cell = 0.5f32;
    for y in 0..2 {
        let mut grid = [[false; 2]; 2]; // [z][x]
        for z in 0..2 {
            for x in 0..2 {
                let bit = micro_bit(x, y, z);
                grid[z][x] = (occ & bit) != 0;
            }
        }
        let mut used = [[false; 2]; 2];
        for z in 0..2 {
            for x in 0..2 {
                if !grid[z][x] || used[z][x] {
                    continue;
                }
                // Try expand in X
                let w = if x == 0 && grid[z][1] && !used[z][1] { 2 } else { 1 };
                // Try expand in Z with width `w`
                let h = if z == 0 {
                    let mut ok = true;
                    for xi in x..(x + w) {
                        if !grid[1][xi] || used[1][xi] {
                            ok = false;
                            break;
                        }
                    }
                    if ok { 2 } else { 1 }
                } else {
                    1
                };
                for dz in 0..h {
                    for dx in 0..w {
                        used[z + dz][x + dx] = true;
                    }
                }
                let min = Vector3::new(
                    fx + (x as f32) * cell,
                    fy + (y as f32) * cell,
                    fz + (z as f32) * cell,
                );
                let max = Vector3::new(
                    fx + ((x + w) as f32) * cell,
                    fy + ((y + 1) as f32) * cell,
                    fz + ((z + h) as f32) * cell,
                );
                out.push((min, max));
            }
        }
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
    edits: Option<&StdHashMap<(i32, i32, i32), Block>>,
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

fn occlusion_mask_for(reg: &BlockRegistry, nb: Block) -> u8 {
    if let Some(ty) = reg.get(nb.id) {
        match &ty.shape {
            crate::blocks::Shape::Slab { half_from }
            | crate::blocks::Shape::Stairs { half_from, .. } => {
                let is_top = ty.state_prop_is_value(nb.state, half_from, "top");
                let posy = (!is_top) as u8;
                let negy = (is_top) as u8;
                // Bits by Face::index(): 0..5 = PosY, NegY, PosX, NegX, PosZ, NegZ
                (posy << Face::PosY.index())
                    | (negy << Face::NegY.index())
                    | (1 << Face::PosX.index())
                    | (1 << Face::NegX.index())
                    | (1 << Face::PosZ.index())
                    | (1 << Face::NegZ.index())
            }
            _ => {
                if ty.is_solid(nb.state) {
                    0b11_1111
                } else {
                    0
                }
            }
        }
    } else {
        0
    }
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
        |x, y, z, face: Face, here| {
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
                            let occ = micro_occupancy_slab(is_top);
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
                                |nx, ny, nz, face, draw_top| {
                                    sample_neighbor_half_light(
                                        &light, nx, ny, nz, face, draw_top, sy,
                                    )
                                },
                            );
                        }
                        crate::blocks::Shape::Stairs {
                            facing_from,
                            half_from,
                        } => {
                            let fx = base_x as f32 + x as f32;
                            let fy = y as f32;
                            let fz = base_z as f32 + z as f32;
                            let is_top = ty.state_prop_is_value(b.state, half_from, "top");
                            let facing = Facing::from_str(
                                ty.state_prop_value(b.state, facing_from).unwrap_or("north"),
                            );
                            let occ = micro_occupancy_stairs(facing, is_top);
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
                                |nx, ny, nz, face, draw_top| {
                                    sample_neighbor_half_light(
                                        &light, nx, ny, nz, face, draw_top, sy,
                                    )
                                },
                            );
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
        bbox: cpu.bbox,
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
    let mut builds = crate::meshing_core::build_mesh_core(
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
                match ty.shape {
                    crate::blocks::Shape::Slab { .. } | crate::blocks::Shape::Stairs { .. } => {
                        return None;
                    }
                    _ => {}
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
                            let occ = micro_occupancy_slab(is_top);
                            let face_material = |face: Face| ty.material_for_cached(face.role(), b.state);
                            for (min, max) in microgrid_boxes(fx, fy, fz, occ) {
                                emit_box_local(
                                    &mut builds,
                                    buf,
                                    reg,
                                    x,
                                    y,
                                    z,
                                    &face_material,
                                    min,
                                    max,
                                    ambient,
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
                                |_, _, _, face, _| face_light(face, ambient),
                            );
                        }
                        crate::blocks::Shape::Stairs {
                            facing_from,
                            half_from,
                        } => {
                            let fx = x as f32;
                            let fy = y as f32;
                            let fz = z as f32;
                            let is_top = ty.state_prop_is_value(b.state, half_from, "top");
                            let facing = Facing::from_str(
                                ty.state_prop_value(b.state, facing_from).unwrap_or("north"),
                            );
                            let occ = micro_occupancy_stairs(facing, is_top);
                            let face_material = |face: Face| ty.material_for_cached(face.role(), b.state);
                            for (min, max) in microgrid_boxes(fx, fy, fz, occ) {
                                emit_box_local(
                                    &mut builds,
                                    buf,
                                    reg,
                                    x,
                                    y,
                                    z,
                                    &face_material,
                                    min,
                                    max,
                                    ambient,
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
                                |_, _, _, face, _| face_light(face, ambient),
                            );
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

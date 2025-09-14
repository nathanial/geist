use std::collections::HashMap;

use geist_blocks::BlockRegistry;
use geist_blocks::micro::micro_cell_solid_s2;
use geist_blocks::types::{Block, MaterialId};
use geist_chunk::ChunkBuf;
use geist_geom::Vec3;
use geist_world::World;

use crate::emit::emit_face_rect_for_clipped;
use crate::face::Face;
use crate::mesh_build::MeshBuild;
use crate::util::registry_material_for_or_unknown;
use crate::constants::{OPAQUE_ALPHA, BITS_PER_WORD, WORD_INDEX_MASK, WORD_INDEX_SHIFT};

// Emit per-cell face quads for a given axis by expanding a mask sourced from FaceGrids.
// Greedy plane emission: merges adjacent face-cells with the same material/orientation into rectangles.
// This replaces the previous per-cell emission and avoids large temporary masks.
fn emit_plane_x(
    s: usize,
    sx: usize,
    sy: usize,
    sz: usize,
    base_x: i32,
    base_z: i32,
    grids: &FaceGrids,
    builds: &mut HashMap<MaterialId, MeshBuild>,
) {
    let scale = 1.0 / s as f32;
    let width = s * sz; // u across +Z
    let height = s * sy; // v across +Y
    for ix in 0..(s * sx) {
        let mut visited = vec![false; width * height];
        let idx2d = |u: usize, v: usize| v * width + u;
        for v in 0..height {
            for u in 0..width {
                let vi = idx2d(u, v);
                if visited[vi] { continue; }
                let idx = grids.idx_x(ix, v, u);
                if !grids.px.get(idx) { continue; }
                let mid = grids.kx[idx];
                if mid.0 == 0 { continue; }
                let pos = grids.ox.get(idx);
                // Greedily extend width
                let mut run_w = 1usize;
                while u + run_w < width {
                    if visited[idx2d(u + run_w, v)] { break; }
                    let idx_n = grids.idx_x(ix, v, u + run_w);
                    if !grids.px.get(idx_n) || grids.kx[idx_n] != mid || grids.ox.get(idx_n) != pos {
                        break;
                    }
                    run_w += 1;
                }
                // Greedily extend height
                let mut run_h = 1usize;
                'outer: while v + run_h < height {
                    for uu in u..(u + run_w) {
                        if visited[idx2d(uu, v + run_h)] { run_h = run_h; break 'outer; }
                        let idx_n = grids.idx_x(ix, v + run_h, uu);
                        if !grids.px.get(idx_n) || grids.kx[idx_n] != mid || grids.ox.get(idx_n) != pos {
                            break 'outer;
                        }
                    }
                    run_h += 1;
                }
                // Emit merged rectangle
                let face = if pos { Face::PosX } else { Face::NegX };
                let origin = Vec3 {
                    x: (base_x as f32) + (ix as f32) * scale,
                    y: (v as f32) * scale,
                    z: (base_z as f32) + (u as f32) * scale,
                };
                let u1 = (run_w as f32) * scale;
                let v1 = (run_h as f32) * scale;
                let rgba = [255u8, 255u8, 255u8, OPAQUE_ALPHA];
                emit_face_rect_for_clipped(
                    builds,
                    mid,
                    face,
                    origin,
                    u1,
                    v1,
                    rgba,
                    base_x,
                    sx,
                    sy,
                    base_z,
                    sz,
                );
                // Mark visited
                for dv in 0..run_h { for du in 0..run_w { visited[idx2d(u + du, v + dv)] = true; } }
            }
        }
    }
}

fn emit_plane_y(
    s: usize,
    sx: usize,
    sy: usize,
    sz: usize,
    base_x: i32,
    base_z: i32,
    grids: &FaceGrids,
    builds: &mut HashMap<MaterialId, MeshBuild>,
) {
    let scale = 1.0 / s as f32;
    let width = s * sx; // u across +X
    let height = s * sz; // v across +Z
    for iy in 0..(s * sy) {
        let mut visited = vec![false; width * height];
        let idx2d = |u: usize, v: usize| v * width + u;
        for v in 0..height {
            for u in 0..width {
                let vi = idx2d(u, v);
                if visited[vi] { continue; }
                let idx = grids.idx_y(u, iy, v);
                if !grids.py.get(idx) { continue; }
                let mid = grids.ky[idx];
                if mid.0 == 0 { continue; }
                let pos = grids.oy.get(idx);
                // Greedy width
                let mut run_w = 1usize;
                while u + run_w < width {
                    if visited[idx2d(u + run_w, v)] { break; }
                    let idx_n = grids.idx_y(u + run_w, iy, v);
                    if !grids.py.get(idx_n) || grids.ky[idx_n] != mid || grids.oy.get(idx_n) != pos { break; }
                    run_w += 1;
                }
                // Greedy height
                let mut run_h = 1usize;
                'outer: while v + run_h < height {
                    for uu in u..(u + run_w) {
                        if visited[idx2d(uu, v + run_h)] { run_h = run_h; break 'outer; }
                        let idx_n = grids.idx_y(uu, iy, v + run_h);
                        if !grids.py.get(idx_n) || grids.ky[idx_n] != mid || grids.oy.get(idx_n) != pos { break 'outer; }
                    }
                    run_h += 1;
                }
                // Emit
                let face = if pos { Face::PosY } else { Face::NegY };
                let origin = Vec3 {
                    x: (base_x as f32) + (u as f32) * scale,
                    y: (iy as f32) * scale,
                    z: (base_z as f32) + (v as f32) * scale,
                };
                let u1 = (run_w as f32) * scale;
                let v1 = (run_h as f32) * scale;
                let rgba = [255u8, 255u8, 255u8, OPAQUE_ALPHA];
                emit_face_rect_for_clipped(
                    builds, mid, face, origin, u1, v1, rgba, base_x, sx, sy, base_z, sz,
                );
                for dv in 0..run_h { for du in 0..run_w { visited[idx2d(u + du, v + dv)] = true; } }
            }
        }
    }
}

fn emit_plane_z(
    s: usize,
    sx: usize,
    sy: usize,
    sz: usize,
    base_x: i32,
    base_z: i32,
    grids: &FaceGrids,
    builds: &mut HashMap<MaterialId, MeshBuild>,
) {
    let scale = 1.0 / s as f32;
    let width = s * sx; // u across +X
    let height = s * sy; // v across +Y
    for iz in 0..(s * sz) {
        let mut visited = vec![false; width * height];
        let idx2d = |u: usize, v: usize| v * width + u;
        for v in 0..height {
            for u in 0..width {
                let vi = idx2d(u, v);
                if visited[vi] { continue; }
                let idx = grids.idx_z(u, v, iz);
                if !grids.pz.get(idx) { continue; }
                let mid = grids.kz[idx];
                if mid.0 == 0 { continue; }
                let pos = grids.oz.get(idx);
                // Greedy width
                let mut run_w = 1usize;
                while u + run_w < width {
                    if visited[idx2d(u + run_w, v)] { break; }
                    let idx_n = grids.idx_z(u + run_w, v, iz);
                    if !grids.pz.get(idx_n) || grids.kz[idx_n] != mid || grids.oz.get(idx_n) != pos { break; }
                    run_w += 1;
                }
                // Greedy height
                let mut run_h = 1usize;
                'outer: while v + run_h < height {
                    for uu in u..(u + run_w) {
                        if visited[idx2d(uu, v + run_h)] { run_h = run_h; break 'outer; }
                        let idx_n = grids.idx_z(uu, v + run_h, iz);
                        if !grids.pz.get(idx_n) || grids.kz[idx_n] != mid || grids.oz.get(idx_n) != pos { break 'outer; }
                    }
                    run_h += 1;
                }
                // Emit
                let face = if pos { Face::PosZ } else { Face::NegZ };
                let origin = Vec3 {
                    x: (base_x as f32) + (u as f32) * scale,
                    y: (v as f32) * scale,
                    z: (base_z as f32) + (iz as f32) * scale,
                };
                let u1 = (run_w as f32) * scale;
                let v1 = (run_h as f32) * scale;
                let rgba = [255u8, 255u8, 255u8, OPAQUE_ALPHA];
                emit_face_rect_for_clipped(
                    builds, mid, face, origin, u1, v1, rgba, base_x, sx, sy, base_z, sz,
                );
                for dv in 0..run_h { for du in 0..run_w { visited[idx2d(u + du, v + dv)] = true; } }
            }
        }
    }
}

#[derive(Default)]
/// Simple growable bitset backed by `u64` words.
struct Bitset { data: Vec<u64> }
impl Bitset {
    /// Creates a bitset large enough to hold `n` bits.
    fn new(n: usize) -> Self { Self { data: vec![0; (n + WORD_INDEX_MASK) / BITS_PER_WORD] } }
    #[inline]
    /// Flips the bit at index `i`.
    fn toggle(&mut self, i: usize) { let w = i >> WORD_INDEX_SHIFT; let b = i & WORD_INDEX_MASK; self.data[w] ^= 1u64 << b; }
    #[inline]
    /// Sets or clears the bit at index `i`.
    fn set(&mut self, i: usize, v: bool) { let w = i >> WORD_INDEX_SHIFT; let b = i & WORD_INDEX_MASK; if v { self.data[w] |= 1u64 << b; } else { self.data[w] &= !(1u64 << b); } }
    #[inline]
    /// Returns `true` if the bit at index `i` is set.
    fn get(&self, i: usize) -> bool { let w = i >> WORD_INDEX_SHIFT; let b = i & WORD_INDEX_MASK; (self.data[w] >> b) & 1 != 0 }
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
    // Material id per face-cell (MaterialId(0) = None)
    kx: Vec<MaterialId>,
    ky: Vec<MaterialId>,
    kz: Vec<MaterialId>,
    // Scales and dims
    s: usize,
    sx: usize,
    sy: usize,
    sz: usize,
}

impl FaceGrids {
    /// Creates face-grid storage sized for the given micro-scaling `s` and chunk dims.
    fn new(s: usize, sx: usize, sy: usize, sz: usize) -> Self {
        let nx = (s * sx + 1) * (s * sy) * (s * sz);
        let ny = (s * sx) * (s * sy + 1) * (s * sz);
        let nz = (s * sx) * (s * sy) * (s * sz + 1);
        Self {
            px: Bitset::new(nx), py: Bitset::new(ny), pz: Bitset::new(nz),
            ox: Bitset::new(nx), oy: Bitset::new(ny), oz: Bitset::new(nz),
            kx: vec![MaterialId(0); nx], ky: vec![MaterialId(0); ny], kz: vec![MaterialId(0); nz],
            s, sx, sy, sz,
        }
    }
    /// Linear index into +X face grid at `(ix,iy,iz)`.
    #[inline]
    fn idx_x(&self, ix: usize, iy: usize, iz: usize) -> usize { let wy = self.s * self.sy; let wz = self.s * self.sz; (ix * wy + iy) * wz + iz }
    /// Linear index into +Y face grid at `(ix,iy,iz)`.
    #[inline]
    fn idx_y(&self, ix: usize, iy: usize, iz: usize) -> usize { let wx = self.s * self.sx; let wz = self.s * self.sz; (iy * wz + iz) * wx + ix }
    /// Linear index into +Z face grid at `(ix,iy,iz)`.
    #[inline]
    fn idx_z(&self, ix: usize, iy: usize, iz: usize) -> usize { let wx = self.s * self.sx; let wy = self.s * self.sy; (iz * wy + iy) * wx + ix }
}

pub struct WccMesher<'a> {
    s: usize,
    sx: usize,
    sy: usize,
    sz: usize,
    grids: FaceGrids,
    reg: &'a BlockRegistry,
    buf: &'a ChunkBuf,
    world: &'a World,
    edits: Option<&'a HashMap<(i32, i32, i32), Block>>,
    base_x: i32,
    base_z: i32,
}

impl<'a> WccMesher<'a> {
    /// Creates a new WCC mesher for the chunk buffer and lighting context.
    pub fn new(
        buf: &'a ChunkBuf,
        reg: &'a BlockRegistry,
        s: usize,
        base_x: i32,
        base_z: i32,
        world: &'a World,
        edits: Option<&'a HashMap<(i32, i32, i32), Block>>,
    ) -> Self {
        let (sx, sy, sz) = (buf.sx, buf.sy, buf.sz);
        Self {
            s, sx, sy, sz,
            grids: FaceGrids::new(s, sx, sy, sz),
            reg, buf, world, edits, base_x, base_z,
        }
    }

    // Overscan: seed parity on our -X/-Z boundary planes using neighbor world blocks.
    // This cancels interior faces across seams and emits neighbor-owned faces when our side is empty.
    /// Seeds parity on -X/-Z seams using neighbor world data to prevent cracks and duplicates.
    pub fn seed_neighbor_seams(&mut self) {
        // -X seam: toggle +X faces of neighbor cells onto ix==0
        for ly in 0..self.sy {
            for lz in 0..self.sz {
                let nb = self.world_block(self.base_x - 1, ly as i32, self.base_z + lz as i32);
                if self.reg.get(nb.id).map(|t| t.name == "water").unwrap_or(false) { continue; }
                let here = self.buf.get_local(0, ly, lz);
                if let (Some(ht), Some(_nt)) = (self.reg.get(here.id), self.reg.get(nb.id)) {
                    if ht.seam.dont_occlude_same && here.id == nb.id { continue; }
                }
                for iym in 0..self.s {
                    for izm in 0..self.s {
                        if micro_cell_solid_s2(self.reg, nb, 1, iym, izm) {
                            let iy = ly * self.s + iym;
                            let iz = lz * self.s + izm;
                            let mid = registry_material_for_or_unknown(nb, Face::PosX, self.reg);
                            self.toggle_x(0, 0, 0, 0, iy, iy + 1, iz, iz + 1, true, mid);
                        }
                    }
                }
            }
        }
        // -Z seam: toggle +Z faces of neighbor cells onto iz==0
        for ly in 0..self.sy {
            for lx in 0..self.sx {
                let nb = self.world_block(self.base_x + lx as i32, ly as i32, self.base_z - 1);
                if self.reg.get(nb.id).map(|t| t.name == "water").unwrap_or(false) { continue; }
                let here = self.buf.get_local(lx, ly, 0);
                if let (Some(ht), Some(_nt)) = (self.reg.get(here.id), self.reg.get(nb.id)) {
                    if ht.seam.dont_occlude_same && here.id == nb.id { continue; }
                }
                for ixm in 0..self.s {
                    for iym in 0..self.s {
                        if micro_cell_solid_s2(self.reg, nb, ixm, iym, 1) {
                            let ix = lx * self.s + ixm;
                            let iy = ly * self.s + iym;
                            let mid = registry_material_for_or_unknown(nb, Face::PosZ, self.reg);
                            self.toggle_z(0, 0, 0, 0, ix, ix + 1, iy, iy + 1, true, mid);
                        }
                    }
                }
            }
        }
    }

    #[inline]
    /// Reads a block from edits (if present) or the world at the given coords.
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
    // Note: Lighting is decoupled from meshing in Phase 1. Colors are recomputed separately.

    /// Toggles parity and material/light keys over a span on an X-oriented face column.
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
    ) {
        for iy in y0..y1 {
            for iz in z0..z1 {
                let idx = self.grids.idx_x(ix, iy, iz);
                self.grids.px.toggle(idx);
                if self.grids.px.get(idx) { self.grids.kx[idx] = mid; self.grids.ox.set(idx, pos); } else { self.grids.kx[idx] = MaterialId(0); }
            }
        }
        let _ = (bx, by, bz);
    }
    /// Toggles parity and material/light keys over a span on a Y-oriented face row.
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
    ) {
        for iz in z0..z1 {
            for ix in x0..x1 {
                let idx = self.grids.idx_y(ix, iy, iz);
                self.grids.py.toggle(idx);
                if self.grids.py.get(idx) { self.grids.ky[idx] = mid; self.grids.oy.set(idx, pos); } else { self.grids.ky[idx] = MaterialId(0); }
            }
        }
        let _ = (bx, by, bz);
    }
    /// Toggles parity and material/light keys over a span on a Z-oriented face column.
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
    ) {
        for iy in y0..y1 {
            for ix in x0..x1 {
                let idx = self.grids.idx_z(ix, iy, iz);
                self.grids.pz.toggle(idx);
                if self.grids.pz.get(idx) { self.grids.kz[idx] = mid; self.grids.oz.set(idx, pos); } else { self.grids.kz[idx] = MaterialId(0); }
            }
        }
        let _ = (bx, by, bz);
    }

    /// Toggles all six faces of an axis-aligned box using provided material-per-face.
    fn toggle_box(
        &mut self,
        x: usize,
        y: usize,
        z: usize,
        bx: (usize, usize, usize, usize, usize, usize),
        mat_for: impl Fn(Face) -> MaterialId,
    ) {
        let (x0, x1, y0, y1, z0, z1) = bx;
        self.toggle_x(x, y, z, x1, y0, y1, z0, z1, true,  mat_for(Face::PosX));
        self.toggle_x(x, y, z, x0, y0, y1, z0, z1, false, mat_for(Face::NegX));
        self.toggle_y(x, y, z, y1, x0, x1, z0, z1, true,  mat_for(Face::PosY));
        self.toggle_y(x, y, z, y0, x0, x1, z0, z1, false, mat_for(Face::NegY));
        self.toggle_z(x, y, z, z1, x0, x1, y0, y1, true,  mat_for(Face::PosZ));
        self.toggle_z(x, y, z, z0, x0, x1, y0, y1, false, mat_for(Face::NegZ));
    }

    /// Adds a full cube at `(x,y,z)` into the WCC grids.
    pub fn add_cube(&mut self, x: usize, y: usize, z: usize, b: Block) {
        let s = self.s;
        let (x0, x1, y0, y1, z0, z1) = (x * s, (x + 1) * s, y * s, (y + 1) * s, z * s, (z + 1) * s);
        let mid_for = |f: Face| registry_material_for_or_unknown(b, f, self.reg);
        self.toggle_box(x, y, z, (x0, x1, y0, y1, z0, z1), mid_for);
    }

    /// Water meshing path: only toggle faces against air to avoid occluding terrain under water.
    pub fn add_water_cube(&mut self, x: usize, y: usize, z: usize, b: Block) {
        let s = self.s;
        let (x0, x1, y0, y1, z0, z1) = (x * s, (x + 1) * s, y * s, (y + 1) * s, z * s, (z + 1) * s);
        let mid_for = |f: Face| registry_material_for_or_unknown(b, f, self.reg);
        let (wx, wy, wz) = (self.base_x + x as i32, y as i32, self.base_z + z as i32);
        let air_id = self.reg.id_by_name("air").unwrap_or(0);
        if self.world_block(wx + 1, wy, wz).id == air_id {
            self.toggle_x(x, y, z, x1, y0, y1, z0, z1, true,  mid_for(Face::PosX));
        }
        if self.world_block(wx - 1, wy, wz).id == air_id {
            self.toggle_x(x, y, z, x0, y0, y1, z0, z1, false, mid_for(Face::NegX));
        }
        if self.world_block(wx, wy + 1, wz).id == air_id {
            self.toggle_y(x, y, z, y1, x0, x1, z0, z1, true,  mid_for(Face::PosY));
        }
        if self.world_block(wx, wy - 1, wz).id == air_id {
            self.toggle_y(x, y, z, y0, x0, x1, z0, z1, false, mid_for(Face::NegY));
        }
        if self.world_block(wx, wy, wz + 1).id == air_id {
            self.toggle_z(x, y, z, z1, x0, x1, y0, y1, true,  mid_for(Face::PosZ));
        }
        if self.world_block(wx, wy, wz - 1).id == air_id {
            self.toggle_z(x, y, z, z0, x0, x1, y0, y1, false, mid_for(Face::NegZ));
        }
    }

    /// Adds micro occupancy at `(x,y,z)` by toggling each micro-box from the occupancy mask.
    pub fn add_micro(&mut self, x: usize, y: usize, z: usize, b: Block, occ: u8) {
        use crate::microgrid_tables::occ8_to_boxes;
        let s = self.s;
        let mid_for = |f: Face| registry_material_for_or_unknown(b, f, self.reg);
        for mb in occ8_to_boxes(occ) {
            let bx0 = x * s + (mb[0] as usize);
            let by0 = y * s + (mb[1] as usize);
            let bz0 = z * s + (mb[2] as usize);
            let bx1 = x * s + (mb[3] as usize);
            let by1 = y * s + (mb[4] as usize);
            let bz1 = z * s + (mb[5] as usize);
            self.toggle_box(x, y, z, (bx0, bx1, by0, by1, bz0, bz1), mid_for);
        }
    }

    /// Emits the per-cell faces for all three axes into material builds.
    pub fn emit_into(&self, builds: &mut HashMap<MaterialId, MeshBuild>) {
        emit_plane_x(self.s, self.sx, self.sy, self.sz, self.base_x, self.base_z, &self.grids, builds);
        emit_plane_y(self.s, self.sx, self.sy, self.sz, self.base_x, self.base_z, &self.grids, builds);
        emit_plane_z(self.s, self.sx, self.sy, self.sz, self.base_x, self.base_z, &self.grids, builds);
    }

}

 

use std::collections::HashMap;

use geist_blocks::BlockRegistry;
use geist_blocks::micro::micro_cell_solid_s2;
use geist_blocks::types::{Block, MaterialId};
use geist_chunk::ChunkBuf;
use geist_geom::Vec3;
use geist_lighting::LightGrid;
use geist_world::World;

use crate::emit::emit_face_rect_for_clipped;
use crate::face::Face;
use crate::mesh_build::MeshBuild;
use crate::util::{apply_min_light, registry_material_for_or_unknown, VISUAL_LIGHT_MIN, greedy_rects};

// Emit greedy quads for a given axis by expanding a mask sourced from FaceGrids.
macro_rules! emit_plane_mask {
    ($self:ident, $builds:ident, X) => {{
        let width = $self.S * $self.sz;
        let height = $self.S * $self.sy;
        for ix in 0..($self.S * $self.sx) {
            let mut mask: Vec<Option<((MaterialId, bool), u8)>> = vec![None; width * height];
            for iy in 0..height {
                for iz in 0..width {
                    let idx = $self.grids.idx_x(ix, iy, iz);
                    if $self.grids.px.get(idx) {
                        let key = $self.grids.kx[idx];
                        if key != 0 {
                            let (mid, l) = $self.keys.get(key);
                            let pos = $self.grids.ox.get(idx);
                            mask[iy * width + iz] = Some(((mid, pos), l));
                        }
                    }
                }
            }
            greedy_rects(width, height, &mut mask, |u0, v0, w, h, codev| {
                let ((mid, pos), l) = codev;
                let lv = apply_min_light(l, Some(VISUAL_LIGHT_MIN));
                let rgba = [lv, lv, lv, 255];
                let face = if pos { Face::PosX } else { Face::NegX };
                let scale = 1.0 / $self.S as f32;
                let origin = Vec3 { x: ($self.base_x as f32) + (ix as f32) * scale, y: (v0 as f32) * scale, z: ($self.base_z as f32) + (u0 as f32) * scale };
                let u1 = (w as f32) * scale;
                let v1 = (h as f32) * scale;
                emit_face_rect_for_clipped($builds, mid, face, origin, u1, v1, rgba, $self.base_x, $self.sx, $self.sy, $self.base_z, $self.sz);
            });
        }
    }};
    ($self:ident, $builds:ident, Y) => {{
        let width = $self.S * $self.sx;
        let height = $self.S * $self.sz;
        for iy in 0..($self.S * $self.sy) {
            let mut mask: Vec<Option<((MaterialId, bool), u8)>> = vec![None; width * height];
            for iz in 0..height {
                for ix in 0..width {
                    let idx = $self.grids.idx_y(ix, iy, iz);
                    if $self.grids.py.get(idx) {
                        let key = $self.grids.ky[idx];
                        if key != 0 {
                            let (mid, l) = $self.keys.get(key);
                            let pos = $self.grids.oy.get(idx);
                            mask[iz * width + ix] = Some(((mid, pos), l));
                        }
                    }
                }
            }
            greedy_rects(width, height, &mut mask, |u0, v0, w, h, codev| {
                let ((mid, pos), l) = codev;
                let lv = apply_min_light(l, Some(VISUAL_LIGHT_MIN));
                let rgba = [lv, lv, lv, 255];
                let face = if pos { Face::PosY } else { Face::NegY };
                let scale = 1.0 / $self.S as f32;
                let origin = Vec3 { x: ($self.base_x as f32) + (u0 as f32) * scale, y: (iy as f32) * scale, z: ($self.base_z as f32) + (v0 as f32) * scale };
                let u1 = (w as f32) * scale;
                let v1 = (h as f32) * scale;
                emit_face_rect_for_clipped($builds, mid, face, origin, u1, v1, rgba, $self.base_x, $self.sx, $self.sy, $self.base_z, $self.sz);
            });
        }
    }};
    ($self:ident, $builds:ident, Z) => {{
        let width = $self.S * $self.sx;
        let height = $self.S * $self.sy;
        for iz in 0..($self.S * $self.sz) {
            let mut mask: Vec<Option<((MaterialId, bool), u8)>> = vec![None; width * height];
            for iy in 0..height {
                for ix in 0..width {
                    let idx = $self.grids.idx_z(ix, iy, iz);
                    if $self.grids.pz.get(idx) {
                        let key = $self.grids.kz[idx];
                        if key != 0 {
                            let (mid, l) = $self.keys.get(key);
                            let pos = $self.grids.oz.get(idx);
                            mask[iy * width + ix] = Some(((mid, pos), l));
                        }
                    }
                }
            }
            greedy_rects(width, height, &mut mask, |u0, v0, w, h, codev| {
                let ((mid, pos), l) = codev;
                let lv = apply_min_light(l, Some(VISUAL_LIGHT_MIN));
                let rgba = [lv, lv, lv, 255];
                let face = if pos { Face::PosZ } else { Face::NegZ };
                let scale = 1.0 / $self.S as f32;
                let origin = Vec3 { x: ($self.base_x as f32) + (u0 as f32) * scale, y: (v0 as f32) * scale, z: ($self.base_z as f32) + (iz as f32) * scale };
                let u1 = (w as f32) * scale;
                let v1 = (h as f32) * scale;
                emit_face_rect_for_clipped($builds, mid, face, origin, u1, v1, rgba, $self.base_x, $self.sx, $self.sy, $self.base_z, $self.sz);
            });
        }
    }};
}

#[derive(Default)]
struct KeyTable {
    items: Vec<(MaterialId, u8)>,
    map: HashMap<(MaterialId, u8), u16>,
}

impl KeyTable {
    fn new() -> Self {
        let mut kt = KeyTable { items: Vec::new(), map: HashMap::new() };
        // Reserve 0 as None
        kt.items.push((MaterialId(0), 0));
        kt
    }
    #[inline]
    fn ensure(&mut self, mid: MaterialId, l: u8) -> u16 {
        if let Some(&idx) = self.map.get(&(mid, l)) { return idx; }
        let idx = self.items.len() as u16;
        self.items.push((mid, l));
        self.map.insert((mid, l), idx);
        idx
    }
    #[inline]
    fn get(&self, idx: u16) -> (MaterialId, u8) { self.items[idx as usize] }
}

struct Bitset { data: Vec<u64> }
impl Bitset {
    fn new(n: usize) -> Self { Self { data: vec![0; (n + 63) / 64] } }
    #[inline]
    fn toggle(&mut self, i: usize) { let w = i >> 6; let b = i & 63; self.data[w] ^= 1u64 << b; }
    #[inline]
    fn set(&mut self, i: usize, v: bool) { let w = i >> 6; let b = i & 63; if v { self.data[w] |= 1u64 << b; } else { self.data[w] &= !(1u64 << b); } }
    #[inline]
    fn get(&self, i: usize) -> bool { let w = i >> 6; let b = i & 63; (self.data[w] >> b) & 1 != 0 }
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
            px: Bitset::new(nx), py: Bitset::new(ny), pz: Bitset::new(nz),
            ox: Bitset::new(nx), oy: Bitset::new(ny), oz: Bitset::new(nz),
            kx: vec![0; nx], ky: vec![0; ny], kz: vec![0; nz],
            S, sx, sy, sz,
        }
    }
    #[inline] fn idx_x(&self, ix: usize, iy: usize, iz: usize) -> usize { let wy = self.S * self.sy; let wz = self.S * self.sz; (ix * wy + iy) * wz + iz }
    #[inline] fn idx_y(&self, ix: usize, iy: usize, iz: usize) -> usize { let wx = self.S * self.sx; let wz = self.S * self.sz; (iy * wz + iz) * wx + ix }
    #[inline] fn idx_z(&self, ix: usize, iy: usize, iz: usize) -> usize { let wx = self.S * self.sx; let wy = self.S * self.sy; (iz * wy + iy) * wx + ix }
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
    ) -> Self {
        let (sx, sy, sz) = (buf.sx, buf.sy, buf.sz);
        Self {
            S, sx, sy, sz,
            grids: FaceGrids::new(S, sx, sy, sz),
            keys: KeyTable::new(),
            reg, light, buf, world, edits, base_x, base_z,
        }
    }

    // Overscan: seed parity on our -X/-Z boundary planes using neighbor world blocks.
    // This cancels interior faces across seams and emits neighbor-owned faces when our side is empty.
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
                for iym in 0..self.S {
                    for izm in 0..self.S {
                        if micro_cell_solid_s2(self.reg, nb, 1, iym, izm) {
                            let iy = ly * self.S + iym;
                            let iz = lz * self.S + izm;
                            let mid = registry_material_for_or_unknown(nb, Face::PosX, self.reg);
                            let l = self.light_bin(0, ly, lz, Face::NegX);
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
                if self.reg.get(nb.id).map(|t| t.name == "water").unwrap_or(false) { continue; }
                let here = self.buf.get_local(lx, ly, 0);
                if let (Some(ht), Some(_nt)) = (self.reg.get(here.id), self.reg.get(nb.id)) {
                    if ht.seam.dont_occlude_same && here.id == nb.id { continue; }
                }
                for ixm in 0..self.S {
                    for iym in 0..self.S {
                        if micro_cell_solid_s2(self.reg, nb, ixm, iym, 1) {
                            let ix = lx * self.S + ixm;
                            let iy = ly * self.S + iym;
                            let mid = registry_material_for_or_unknown(nb, Face::PosZ, self.reg);
                            let l = self.light_bin(lx, ly, 0, Face::NegZ);
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
        let nx = self.base_x - 1; let ny = ly as i32; let nz = self.base_z + lz as i32;
        let nb = self.world_block(nx, ny, nz);
        if micro_cell_solid_s2(self.reg, nb, 1, iym, izm) {
            let mid = registry_material_for_or_unknown(nb, Face::PosX, self.reg);
            let l = self.light_bin(0, ly, lz, Face::NegX);
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
    fn light_bin(&self, x: usize, y: usize, z: usize, face: Face) -> u8 {
        let l = self.light.sample_face_local_s2(self.buf, self.reg, x, y, z, face.index());
        l.max(VISUAL_LIGHT_MIN)
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
                if self.grids.px.get(idx) { self.grids.kx[idx] = key; self.grids.ox.set(idx, pos); } else { self.grids.kx[idx] = 0; }
            }
        }
        let _ = (bx, by, bz);
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
                if self.grids.py.get(idx) { self.grids.ky[idx] = key; self.grids.oy.set(idx, pos); } else { self.grids.ky[idx] = 0; }
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
                if self.grids.pz.get(idx) { self.grids.kz[idx] = key; self.grids.oz.set(idx, pos); } else { self.grids.kz[idx] = 0; }
            }
        }
        let _ = (bx, by, bz);
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
        self.toggle_x(x, y, z, x1, y0, y1, z0, z1, true,  mat_for(Face::PosX), self.light_bin(x, y, z, Face::PosX));
        self.toggle_x(x, y, z, x0, y0, y1, z0, z1, false, mat_for(Face::NegX), self.light_bin(x, y, z, Face::NegX));
        self.toggle_y(x, y, z, y1, x0, x1, z0, z1, true,  mat_for(Face::PosY), self.light_bin(x, y, z, Face::PosY));
        self.toggle_y(x, y, z, y0, x0, x1, z0, z1, false, mat_for(Face::NegY), self.light_bin(x, y, z, Face::NegY));
        self.toggle_z(x, y, z, z1, x0, x1, y0, y1, true,  mat_for(Face::PosZ), self.light_bin(x, y, z, Face::PosZ));
        self.toggle_z(x, y, z, z0, x0, x1, y0, y1, false, mat_for(Face::NegZ), self.light_bin(x, y, z, Face::NegZ));
    }

    pub fn add_cube(&mut self, x: usize, y: usize, z: usize, b: Block) {
        let S = self.S;
        let (x0, x1, y0, y1, z0, z1) = (x * S, (x + 1) * S, y * S, (y + 1) * S, z * S, (z + 1) * S);
        let mid_for = |f: Face| registry_material_for_or_unknown(b, f, self.reg);
        self.toggle_box(x, y, z, (x0, x1, y0, y1, z0, z1), mid_for);
    }

    /// Water meshing path: only toggle faces against air to avoid occluding terrain under water.
    pub fn add_water_cube(&mut self, x: usize, y: usize, z: usize, b: Block) {
        let S = self.S;
        let (x0, x1, y0, y1, z0, z1) = (x * S, (x + 1) * S, y * S, (y + 1) * S, z * S, (z + 1) * S);
        let mid_for = |f: Face| registry_material_for_or_unknown(b, f, self.reg);
        let (wx, wy, wz) = (self.base_x + x as i32, y as i32, self.base_z + z as i32);
        let air_id = self.reg.id_by_name("air").unwrap_or(0);
        if self.world_block(wx + 1, wy, wz).id == air_id {
            self.toggle_x(x, y, z, x1, y0, y1, z0, z1, true,  mid_for(Face::PosX), self.light_bin(x, y, z, Face::PosX));
        }
        if self.world_block(wx - 1, wy, wz).id == air_id {
            self.toggle_x(x, y, z, x0, y0, y1, z0, z1, false, mid_for(Face::NegX), self.light_bin(x, y, z, Face::NegX));
        }
        if self.world_block(wx, wy + 1, wz).id == air_id {
            self.toggle_y(x, y, z, y1, x0, x1, z0, z1, true,  mid_for(Face::PosY), self.light_bin(x, y, z, Face::PosY));
        }
        if self.world_block(wx, wy - 1, wz).id == air_id {
            self.toggle_y(x, y, z, y0, x0, x1, z0, z1, false, mid_for(Face::NegY), self.light_bin(x, y, z, Face::NegY));
        }
        if self.world_block(wx, wy, wz + 1).id == air_id {
            self.toggle_z(x, y, z, z1, x0, x1, y0, y1, true,  mid_for(Face::PosZ), self.light_bin(x, y, z, Face::PosZ));
        }
        if self.world_block(wx, wy, wz - 1).id == air_id {
            self.toggle_z(x, y, z, z0, x0, x1, y0, y1, false, mid_for(Face::NegZ), self.light_bin(x, y, z, Face::NegZ));
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
        emit_plane_mask!(self, builds, X);
        emit_plane_mask!(self, builds, Y);
        emit_plane_mask!(self, builds, Z);
    }
}

 

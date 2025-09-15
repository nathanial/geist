use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Instant;

use geist_blocks::BlockRegistry;
use geist_blocks::types::{Block, MaterialId};
use geist_chunk::ChunkBuf;
use geist_geom::Vec3;
use geist_world::World;

use crate::constants::{BITS_PER_WORD, OPAQUE_ALPHA, WORD_INDEX_MASK, WORD_INDEX_SHIFT};
use crate::emit::emit_face_rect_for_clipped;
use crate::face::Face;

// Local small bitset type
#[derive(Default)]
struct Bitset {
    data: Vec<u64>,
}
impl Bitset {
    fn new(nbits: usize) -> Self {
        Self {
            data: vec![0; (nbits + WORD_INDEX_MASK) / BITS_PER_WORD],
        }
    }
    #[inline]
    fn set(&mut self, i: usize, v: bool) {
        let w = i >> WORD_INDEX_SHIFT;
        let b = i & WORD_INDEX_MASK;
        if v {
            self.data[w] |= 1u64 << b;
        } else {
            self.data[w] &= !(1u64 << b);
        }
    }
    #[inline]
    fn get(&self, i: usize) -> bool {
        let w = i >> WORD_INDEX_SHIFT;
        let b = i & WORD_INDEX_MASK;
        ((self.data[w] >> b) & 1) != 0
    }
    #[inline]
    fn clear(&mut self) {
        self.data.fill(0);
    }
}

// Dense face grids (same shape as v2)
struct FaceGrids {
    px: Bitset,
    py: Bitset,
    pz: Bitset,
    ox: Bitset,
    oy: Bitset,
    oz: Bitset,
    kx: Vec<MaterialId>,
    ky: Vec<MaterialId>,
    kz: Vec<MaterialId>,
    s: usize,
    sx: usize,
    sy: usize,
    sz: usize,
}
impl FaceGrids {
    fn new(s: usize, sx: usize, sy: usize, sz: usize) -> Self {
        let nx = (s * sx + 1) * (s * sy) * (s * sz);
        let ny = (s * sx) * (s * sy + 1) * (s * sz);
        let nz = (s * sx) * (s * sy) * (s * sz + 1);
        Self {
            px: Bitset::new(nx),
            py: Bitset::new(ny),
            pz: Bitset::new(nz),
            ox: Bitset::new(nx),
            oy: Bitset::new(ny),
            oz: Bitset::new(nz),
            kx: vec![MaterialId(0); nx],
            ky: vec![MaterialId(0); ny],
            kz: vec![MaterialId(0); nz],
            s,
            sx,
            sy,
            sz,
        }
    }
    #[inline]
    fn idx_x(&self, ix: usize, iy: usize, iz: usize) -> usize {
        let wy = self.s * self.sy;
        let wz = self.s * self.sz;
        (ix * wy + iy) * wz + iz
    }
    #[inline]
    fn idx_y(&self, ix: usize, iy: usize, iz: usize) -> usize {
        let wx = self.s * self.sx;
        let wz = self.s * self.sz;
        (iy * wz + iz) * wx + ix
    }
    #[inline]
    fn idx_z(&self, ix: usize, iy: usize, iz: usize) -> usize {
        let wx = self.s * self.sx;
        let wy = self.s * self.sy;
        (iz * wy + iy) * wx + ix
    }
}

// Dense micro occupancy for Nx*Ny*Nz and seam layers for -X and -Z
struct OccGrids {
    // interior occupancy
    occ: Bitset,
    // seam layers (-X: Ny*Nz, -Z: Nx*Ny)
    seam_x: Bitset,
    seam_z: Bitset,
    nx: usize,
    ny: usize,
    nz: usize,
}
impl OccGrids {
    fn new(nx: usize, ny: usize, nz: usize) -> Self {
        Self {
            occ: Bitset::new(nx * ny * nz),
            seam_x: Bitset::new(ny * nz),
            seam_z: Bitset::new(nx * ny),
            nx,
            ny,
            nz,
        }
    }
    #[inline]
    fn idx(&self, ix: usize, iy: usize, iz: usize) -> usize {
        (ix * self.ny + iy) * self.nz + iz
    }
    #[inline]
    fn idx_sx(&self, iy: usize, iz: usize) -> usize {
        iy * self.nz + iz
    }
    #[inline]
    fn idx_sz(&self, ix: usize, iy: usize) -> usize {
        (iy * self.nx) + ix
    }
    #[inline]
    fn occ_get(&self, ix: usize, iy: usize, iz: usize) -> bool {
        self.occ.get(self.idx(ix, iy, iz))
    }
    #[inline]
    fn occ_set(&mut self, ix: usize, iy: usize, iz: usize, v: bool) {
        let i = self.idx(ix, iy, iz);
        self.occ.set(i, v);
    }
}

// Thread-local scratch for v3 mesher
thread_local! {
    static FACEGRID_SCRATCH_V3: RefCell<Option<FaceGrids>> = RefCell::new(None);
    static OCC_SCRATCH_V3: RefCell<Option<OccGrids>> = RefCell::new(None);
    // Separate pools for water grids/occs to avoid reallocs
    static FACEGRID_SCRATCH_V3_WATER: RefCell<Option<FaceGrids>> = RefCell::new(None);
    static OCC_SCRATCH_V3_WATER: RefCell<Option<OccGrids>> = RefCell::new(None);
    // Reusable 2D visitation bitmap used by greedy plane emission.
    // Sized to the maximum width*height needed across axes for current (s, sx, sy, sz).
    static VISITED_SCRATCH_V3: RefCell<Vec<u8>> = RefCell::new(Vec::new());
}

pub struct ParityMesher<'a> {
    s: usize,
    sx: usize,
    sy: usize,
    sz: usize,
    base_x: i32,
    base_z: i32,
    // World scale: how many world units per micro-step
    world_scale: f32,
    // Clip extents for chunk interior in world units (usually original chunk size)
    clip_sx: usize,
    clip_sy: usize,
    clip_sz: usize,
    // Policy: include water surfaces?
    include_water: bool,
    reg: &'a BlockRegistry,
    buf: &'a ChunkBuf,
    world: &'a World,
    edits: Option<&'a HashMap<(i32, i32, i32), Block>>,
    air_id: u16,
    // scratch (solids + water)
    grids: FaceGrids,
    grids_water: FaceGrids,
    occs: OccGrids,
    occs_water: OccGrids,
}

impl<'a> ParityMesher<'a> {
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
        Self::new_with_scale_and_clip(
            buf, reg, s, base_x, base_z, 1.0, sx, sy, sz, true, world, edits,
        )
    }

    pub fn new_with_scale_and_clip(
        buf: &'a ChunkBuf,
        reg: &'a BlockRegistry,
        s: usize,
        base_x: i32,
        base_z: i32,
        world_scale: f32,
        clip_sx: usize,
        clip_sy: usize,
        clip_sz: usize,
        include_water: bool,
        world: &'a World,
        edits: Option<&'a HashMap<(i32, i32, i32), Block>>,
    ) -> Self {
        let (sx, sy, sz) = (buf.sx, buf.sy, buf.sz);
        let nx = s * sx;
        let ny = s * sy;
        let nz = s * sz;
        let grids = FACEGRID_SCRATCH_V3.with(|cell| {
            if let Some(mut g) = cell.borrow_mut().take() {
                if g.s == s && g.sx == sx && g.sy == sy && g.sz == sz {
                    g.px.clear();
                    g.py.clear();
                    g.pz.clear();
                    g.ox.clear();
                    g.oy.clear();
                    g.oz.clear();
                    g.kx.fill(MaterialId(0));
                    g.ky.fill(MaterialId(0));
                    g.kz.fill(MaterialId(0));
                    g
                } else {
                    FaceGrids::new(s, sx, sy, sz)
                }
            } else {
                FaceGrids::new(s, sx, sy, sz)
            }
        });
        let occs = OCC_SCRATCH_V3.with(|cell| {
            if let Some(mut o) = cell.borrow_mut().take() {
                if o.nx == nx && o.ny == ny && o.nz == nz {
                    o.occ.clear();
                    o.seam_x.clear();
                    o.seam_z.clear();
                    o
                } else {
                    OccGrids::new(nx, ny, nz)
                }
            } else {
                OccGrids::new(nx, ny, nz)
            }
        });
        let grids_water = FACEGRID_SCRATCH_V3_WATER.with(|cell| {
            if let Some(mut g) = cell.borrow_mut().take() {
                if g.s == s && g.sx == sx && g.sy == sy && g.sz == sz {
                    g.px.clear();
                    g.py.clear();
                    g.pz.clear();
                    g.ox.clear();
                    g.oy.clear();
                    g.oz.clear();
                    g.kx.fill(MaterialId(0));
                    g.ky.fill(MaterialId(0));
                    g.kz.fill(MaterialId(0));
                    g
                } else {
                    FaceGrids::new(s, sx, sy, sz)
                }
            } else {
                FaceGrids::new(s, sx, sy, sz)
            }
        });
        let occs_water = OCC_SCRATCH_V3_WATER.with(|cell| {
            if let Some(mut o) = cell.borrow_mut().take() {
                if o.nx == nx && o.ny == ny && o.nz == nz {
                    o.occ.clear();
                    o.seam_x.clear();
                    o.seam_z.clear();
                    o
                } else {
                    OccGrids::new(nx, ny, nz)
                }
            } else {
                OccGrids::new(nx, ny, nz)
            }
        });
        Self {
            s,
            sx,
            sy,
            sz,
            base_x,
            base_z,
            world_scale,
            clip_sx,
            clip_sy,
            clip_sz,
            include_water,
            reg,
            buf,
            world,
            edits,
            air_id: reg.id_by_name("air").unwrap_or(0),
            grids,
            grids_water,
            occs,
            occs_water,
        }
    }

    pub fn recycle(self) {
        FACEGRID_SCRATCH_V3.with(|cell| cell.borrow_mut().replace(self.grids));
        FACEGRID_SCRATCH_V3_WATER.with(|cell| cell.borrow_mut().replace(self.grids_water));
        OCC_SCRATCH_V3.with(|cell| cell.borrow_mut().replace(self.occs));
        OCC_SCRATCH_V3_WATER.with(|cell| cell.borrow_mut().replace(self.occs_water));
    }

    #[inline]
    fn world_block(&self, nx: i32, ny: i32, nz: i32) -> Block {
        if let Some(ed) = self.edits {
            ed.get(&(nx, ny, nz))
                .copied()
                .unwrap_or_else(|| self.world.block_at_runtime(self.reg, nx, ny, nz))
        } else {
            self.world.block_at_runtime(self.reg, nx, ny, nz)
        }
    }

    pub fn build_occupancy(&mut self) {
        let s = self.s;
        let (sx, sy, sz) = (self.sx, self.sy, self.sz);
        #[inline]
        fn occ_bit_s2(occ: u8, mx: usize, my: usize, mz: usize) -> bool {
            let i = ((my & 1) << 2) | ((mz & 1) << 1) | (mx & 1);
            ((occ >> i) & 1) != 0
        }
        for z in 0..sz {
            for y in 0..sy {
                for x in 0..sx {
                    let b = self.buf.get_local(x, y, z);
                    if b.id == 0 {
                        continue;
                    }
                    if let Some(ty) = self.reg.get(b.id) {
                        // water first: mark only in water grid (exclude from solids)
                        if self.include_water && ty.name == "water" {
                            let (x0, x1, y0, y1, z0, z1) =
                                (x * s, (x + 1) * s, y * s, (y + 1) * s, z * s, (z + 1) * s);
                            for iz in z0..z1 {
                                for iy in y0..y1 {
                                    for ix in x0..x1 {
                                        self.occs_water.occ_set(ix, iy, iz, true);
                                    }
                                }
                            }
                            continue;
                        }
                        // micro occupancy (solids)
                        if self.s > 1 {
                            if let Some(occ) = ty.variant(b.state).occupancy {
                                // S=2 patterns
                                for mz in 0..s {
                                    for my in 0..s {
                                        for mx in 0..s {
                                            if occ_bit_s2(occ, mx, my, mz) {
                                                let ix = x * s + mx;
                                                let iy = y * s + my;
                                                let iz = z * s + mz;
                                                self.occs.occ_set(ix, iy, iz, true);
                                            }
                                        }
                                    }
                                }
                                continue;
                            }
                        }
                        // full cubes (cubes + axis cubes) (solids only; water handled above)
                        if ty.is_solid(b.state)
                            && matches!(
                                ty.shape,
                                geist_blocks::types::Shape::Cube
                                    | geist_blocks::types::Shape::AxisCube { .. }
                            )
                        {
                            let (x0, x1, y0, y1, z0, z1) =
                                (x * s, (x + 1) * s, y * s, (y + 1) * s, z * s, (z + 1) * s);
                            for iz in z0..z1 {
                                for iy in y0..y1 {
                                    for ix in x0..x1 {
                                        self.occs.occ_set(ix, iy, iz, true);
                                    }
                                }
                            }
                            continue;
                        }
                    }
                }
            }
        }
    }

    pub fn seed_seam_layers(&mut self) {
        // -X seam layer (ix = -1)
        let t_x = Instant::now();
        let s = self.s;
        let (sx, sy, sz) = (self.sx, self.sy, self.sz);
        #[inline]
        fn occ_bit_s2(occ: u8, mx: usize, my: usize, mz: usize) -> bool {
            let i = ((my & 1) << 2) | ((mz & 1) << 1) | (mx & 1);
            ((occ >> i) & 1) != 0
        }
        for ly in 0..sy {
            for lz in 0..sz {
                let nb = self.world_block(self.base_x - 1, ly as i32, self.base_z + lz as i32);
                if nb.id == 0 {
                    continue;
                }
                if let Some(ty) = self.reg.get(nb.id) {
                    if self.include_water && ty.name == "water" {
                        let y0 = ly * s;
                        let z0 = lz * s;
                        for iz in z0..(z0 + s) {
                            for iy in y0..(y0 + s) {
                                let i = self.occs_water.idx_sx(iy, iz);
                                self.occs_water.seam_x.set(i, true);
                            }
                        }
                    } else if ty.is_solid(nb.state)
                        && matches!(
                            ty.shape,
                            geist_blocks::types::Shape::Cube
                                | geist_blocks::types::Shape::AxisCube { .. }
                        )
                    {
                        let y0 = ly * s;
                        let z0 = lz * s;
                        for iz in z0..(z0 + s) {
                            for iy in y0..(y0 + s) {
                                let i = self.occs.idx_sx(iy, iz);
                                self.occs.seam_x.set(i, true);
                            }
                        }
                    } else if self.s > 1 {
                        if let Some(occ) = ty.variant(nb.state).occupancy {
                            let y0 = ly * s;
                            let z0 = lz * s;
                            for mz in 0..s {
                                for my in 0..s {
                                    if occ_bit_s2(occ, 1, my, mz) {
                                        let iy = y0 + my;
                                        let iz = z0 + mz;
                                        let i = self.occs.idx_sx(iy, iz);
                                        self.occs.seam_x.set(i, true);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let ms_x: u32 = t_x.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
        log::info!(target: "perf", "ms={} mesher_seed_seam axis=X s={} dims=({}, {}, {}) base_x={} base_z={}", ms_x, self.s, self.sx, self.sy, self.sz, self.base_x, self.base_z);

        // -Z seam layer (iz = -1)
        let t_z = Instant::now();
        for ly in 0..sy {
            for lx in 0..sx {
                let nb = self.world_block(self.base_x + lx as i32, ly as i32, self.base_z - 1);
                if nb.id == 0 {
                    continue;
                }
                if let Some(ty) = self.reg.get(nb.id) {
                    if self.include_water && ty.name == "water" {
                        let x0 = lx * s;
                        let y0 = ly * s;
                        for ix in x0..(x0 + s) {
                            for iy in y0..(y0 + s) {
                                let i = self.occs_water.idx_sz(ix, iy);
                                self.occs_water.seam_z.set(i, true);
                            }
                        }
                    } else if ty.is_solid(nb.state)
                        && matches!(
                            ty.shape,
                            geist_blocks::types::Shape::Cube
                                | geist_blocks::types::Shape::AxisCube { .. }
                        )
                    {
                        let x0 = lx * s;
                        let y0 = ly * s;
                        for ix in x0..(x0 + s) {
                            for iy in y0..(y0 + s) {
                                let i = self.occs.idx_sz(ix, iy);
                                self.occs.seam_z.set(i, true);
                            }
                        }
                    } else if self.s > 1 {
                        if let Some(occ) = ty.variant(nb.state).occupancy {
                            let x0 = lx * s;
                            let y0 = ly * s;
                            for my in 0..s {
                                for mx in 0..s {
                                    if occ_bit_s2(occ, mx, my, 1) {
                                        let ix = x0 + mx;
                                        let iy = y0 + my;
                                        let i = self.occs.idx_sz(ix, iy);
                                        self.occs.seam_z.set(i, true);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let ms_z: u32 = t_z.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
        log::info!(target: "perf", "ms={} mesher_seed_seam axis=Z s={} dims=({}, {}, {}) base_x={} base_z={}", ms_z, self.s, self.sx, self.sy, self.sz, self.base_x, self.base_z);
    }

    pub fn compute_parity_and_materials(&mut self) {
        let s = self.s;
        let (nx, ny, nz) = (self.occs.nx, self.occs.ny, self.occs.nz);
        // X faces
        let t0 = Instant::now();
        for ix in 0..=nx {
            // include +X boundary
            for iy in 0..ny {
                for iz in 0..nz {
                    let a = if ix == 0 {
                        self.occs.seam_x.get(self.occs.idx_sx(iy, iz))
                    } else {
                        self.occs.occ_get(ix - 1, iy, iz)
                    };
                    let b = if ix == nx {
                        false
                    } else {
                        self.occs.occ_get(ix, iy, iz)
                    };
                    let p = a ^ b;
                    let idx = self.grids.idx_x(ix, iy, iz);
                    self.grids.px.set(idx, p);
                    if !p {
                        self.grids.kx[idx] = MaterialId(0);
                        continue;
                    }
                    // Owner is the occupied side; outward normal points from solid to empty
                    // a==1 => left cell owns → +X face; b==1 => right cell owns → -X face
                    let owner_pos = a;
                    self.grids.ox.set(idx, owner_pos);
                    let face = if owner_pos { Face::PosX } else { Face::NegX };
                    let mid = if owner_pos {
                        // +X face of left cell; may be neighbor at ix==0
                        if ix == 0 {
                            // world neighbor at x = base_x - 1
                            let by = (iy / s).min(self.sy - 1);
                            let bz = (iz / s).min(self.sz - 1);
                            let nb = self.world_block(
                                self.base_x - 1,
                                by as i32,
                                self.base_z + bz as i32,
                            );
                            self.reg
                                .get(nb.id)
                                .map(|ty| ty.material_for_cached(face.role(), nb.state))
                                .unwrap_or(MaterialId(0))
                        } else {
                            let bx = ((ix - 1) / s).min(self.sx - 1);
                            let by = (iy / s).min(self.sy - 1);
                            let bz = (iz / s).min(self.sz - 1);
                            let here = self.buf.get_local(bx, by, bz);
                            self.reg
                                .get(here.id)
                                .map(|ty| ty.material_for_cached(face.role(), here.state))
                                .unwrap_or(MaterialId(0))
                        }
                    } else {
                        // -X face of right cell (always within chunk for ix<nx)
                        let bx = (ix / s).min(self.sx - 1);
                        let by = (iy / s).min(self.sy - 1);
                        let bz = (iz / s).min(self.sz - 1);
                        let here = self.buf.get_local(bx, by, bz);
                        self.reg
                            .get(here.id)
                            .map(|ty| ty.material_for_cached(face.role(), here.state))
                            .unwrap_or(MaterialId(0))
                    };
                    self.grids.kx[idx] = mid;
                }
            }
        }
        // X faces (water-only): emit only water-air faces (skip water-solid)
        for ix in 0..=nx {
            for iy in 0..ny {
                for iz in 0..nz {
                    let a_w = if ix == 0 {
                        self.occs_water.seam_x.get(self.occs_water.idx_sx(iy, iz))
                    } else {
                        self.occs_water.occ_get(ix - 1, iy, iz)
                    };
                    let b_w = if ix == nx {
                        false
                    } else {
                        self.occs_water.occ_get(ix, iy, iz)
                    };
                    let p_w = a_w ^ b_w;
                    let idx_w = self.grids_water.idx_x(ix, iy, iz);
                    self.grids_water.px.set(idx_w, p_w);
                    if !p_w {
                        self.grids_water.kx[idx_w] = MaterialId(0);
                        continue;
                    }
                    let owner_pos_w = a_w;
                    self.grids_water.ox.set(idx_w, owner_pos_w);
                    // Solid occupancy on the opposite side?
                    let a_s = if ix == 0 {
                        self.occs.seam_x.get(self.occs.idx_sx(iy, iz))
                    } else {
                        self.occs.occ_get(ix - 1, iy, iz)
                    };
                    let b_s = if ix == nx {
                        false
                    } else {
                        self.occs.occ_get(ix, iy, iz)
                    };
                    let solid_other = if owner_pos_w { b_s } else { a_s };
                    if solid_other {
                        self.grids_water.kx[idx_w] = MaterialId(0);
                        continue;
                    }
                    let face = if owner_pos_w { Face::PosX } else { Face::NegX };
                    let mid_w = if owner_pos_w {
                        if ix == 0 {
                            let by = (iy / s).min(self.sy - 1);
                            let bz = (iz / s).min(self.sz - 1);
                            let nb = self.world_block(
                                self.base_x - 1,
                                by as i32,
                                self.base_z + bz as i32,
                            );
                            self.reg
                                .get(nb.id)
                                .map(|ty| ty.material_for_cached(face.role(), nb.state))
                                .unwrap_or(MaterialId(0))
                        } else {
                            let bx = ((ix - 1) / s).min(self.sx - 1);
                            let by = (iy / s).min(self.sy - 1);
                            let bz = (iz / s).min(self.sz - 1);
                            let here = self.buf.get_local(bx, by, bz);
                            self.reg
                                .get(here.id)
                                .map(|ty| ty.material_for_cached(face.role(), here.state))
                                .unwrap_or(MaterialId(0))
                        }
                    } else {
                        let bx = (ix / s).min(self.sx - 1);
                        let by = (iy / s).min(self.sy - 1);
                        let bz = (iz / s).min(self.sz - 1);
                        let here = self.buf.get_local(bx, by, bz);
                        self.reg
                            .get(here.id)
                            .map(|ty| ty.material_for_cached(face.role(), here.state))
                            .unwrap_or(MaterialId(0))
                    };
                    self.grids_water.kx[idx_w] = mid_w;
                }
            }
        }
        // Y faces
        for iy in 0..=ny {
            // include top boundary
            for iz in 0..nz {
                for ix in 0..nx {
                    let a = if iy == 0 {
                        false
                    } else {
                        self.occs.occ_get(ix, iy - 1, iz)
                    };
                    let b = if iy == ny {
                        false
                    } else {
                        self.occs.occ_get(ix, iy, iz)
                    };
                    let p = a ^ b;
                    let idx = self.grids.idx_y(ix, iy, iz);
                    self.grids.py.set(idx, p);
                    if !p {
                        self.grids.ky[idx] = MaterialId(0);
                        continue;
                    }
                    // a==1 => +Y face of below cell; b==1 => -Y face of above cell
                    let owner_pos = a;
                    self.grids.oy.set(idx, owner_pos);
                    let face = if owner_pos { Face::PosY } else { Face::NegY };
                    let by_owner = if owner_pos { iy.saturating_sub(1) } else { iy };
                    let bx = (ix / s).min(self.sx - 1);
                    let by = (by_owner / s).min(self.sy - 1);
                    let bz = (iz / s).min(self.sz - 1);
                    let here = self.buf.get_local(bx, by, bz);
                    let mid = self
                        .reg
                        .get(here.id)
                        .map(|ty| ty.material_for_cached(face.role(), here.state))
                        .unwrap_or(MaterialId(0));
                    self.grids.ky[idx] = mid;
                }
            }
        }
        // Y faces (water-only)
        for iy in 0..=ny {
            for iz in 0..nz {
                for ix in 0..nx {
                    let a_w = if iy == 0 {
                        false
                    } else {
                        self.occs_water.occ_get(ix, iy - 1, iz)
                    };
                    let b_w = if iy == ny {
                        false
                    } else {
                        self.occs_water.occ_get(ix, iy, iz)
                    };
                    let p_w = a_w ^ b_w;
                    let idx_w = self.grids_water.idx_y(ix, iy, iz);
                    self.grids_water.py.set(idx_w, p_w);
                    if !p_w {
                        self.grids_water.ky[idx_w] = MaterialId(0);
                        continue;
                    }
                    let owner_pos_w = a_w;
                    self.grids_water.oy.set(idx_w, owner_pos_w);
                    let a_s = if iy == 0 {
                        false
                    } else {
                        self.occs.occ_get(ix, iy - 1, iz)
                    };
                    let b_s = if iy == ny {
                        false
                    } else {
                        self.occs.occ_get(ix, iy, iz)
                    };
                    let solid_other = if owner_pos_w { b_s } else { a_s };
                    if solid_other {
                        self.grids_water.ky[idx_w] = MaterialId(0);
                        continue;
                    }
                    let face = if owner_pos_w { Face::PosY } else { Face::NegY };
                    let by_owner = if owner_pos_w {
                        iy.saturating_sub(1)
                    } else {
                        iy
                    };
                    let bx = (ix / s).min(self.sx - 1);
                    let by = (by_owner / s).min(self.sy - 1);
                    let bz = (iz / s).min(self.sz - 1);
                    let here = self.buf.get_local(bx, by, bz);
                    let mid_w = self
                        .reg
                        .get(here.id)
                        .map(|ty| ty.material_for_cached(face.role(), here.state))
                        .unwrap_or(MaterialId(0));
                    self.grids_water.ky[idx_w] = mid_w;
                }
            }
        }
        // Z faces
        for iz in 0..=nz {
            for iy in 0..ny {
                for ix in 0..nx {
                    let a = if iz == 0 {
                        self.occs.seam_z.get(self.occs.idx_sz(ix, iy))
                    } else {
                        self.occs.occ_get(ix, iy, iz - 1)
                    };
                    let b = if iz == nz {
                        false
                    } else {
                        self.occs.occ_get(ix, iy, iz)
                    };
                    let p = a ^ b;
                    let idx = self.grids.idx_z(ix, iy, iz);
                    self.grids.pz.set(idx, p);
                    if !p {
                        self.grids.kz[idx] = MaterialId(0);
                        continue;
                    }
                    // a==1 => +Z face of back cell; b==1 => -Z face of front cell
                    let owner_pos = a;
                    self.grids.oz.set(idx, owner_pos);
                    let face = if owner_pos { Face::PosZ } else { Face::NegZ };
                    let mid = if owner_pos {
                        if iz == 0 {
                            // neighbor at z = base_z - 1
                            let bx = (ix / s).min(self.sx - 1);
                            let by = (iy / s).min(self.sy - 1);
                            let nb = self.world_block(
                                self.base_x + bx as i32,
                                by as i32,
                                self.base_z - 1,
                            );
                            self.reg
                                .get(nb.id)
                                .map(|ty| ty.material_for_cached(face.role(), nb.state))
                                .unwrap_or(MaterialId(0))
                        } else {
                            let bz = ((iz - 1) / s).min(self.sz - 1);
                            let bx = (ix / s).min(self.sx - 1);
                            let by = (iy / s).min(self.sy - 1);
                            let here = self.buf.get_local(bx, by, bz);
                            self.reg
                                .get(here.id)
                                .map(|ty| ty.material_for_cached(face.role(), here.state))
                                .unwrap_or(MaterialId(0))
                        }
                    } else {
                        let bz = (iz / s).min(self.sz - 1);
                        let bx = (ix / s).min(self.sx - 1);
                        let by = (iy / s).min(self.sy - 1);
                        let here = self.buf.get_local(bx, by, bz);
                        self.reg
                            .get(here.id)
                            .map(|ty| ty.material_for_cached(face.role(), here.state))
                            .unwrap_or(MaterialId(0))
                    };
                    self.grids.kz[idx] = mid;
                }
            }
        }
        // Z faces (water-only)
        for iz in 0..=nz {
            for iy in 0..ny {
                for ix in 0..nx {
                    let a_w = if iz == 0 {
                        self.occs_water.seam_z.get(self.occs_water.idx_sz(ix, iy))
                    } else {
                        self.occs_water.occ_get(ix, iy, iz - 1)
                    };
                    let b_w = if iz == nz {
                        false
                    } else {
                        self.occs_water.occ_get(ix, iy, iz)
                    };
                    let p_w = a_w ^ b_w;
                    let idx_w = self.grids_water.idx_z(ix, iy, iz);
                    self.grids_water.pz.set(idx_w, p_w);
                    if !p_w {
                        self.grids_water.kz[idx_w] = MaterialId(0);
                        continue;
                    }
                    let owner_pos_w = a_w;
                    self.grids_water.oz.set(idx_w, owner_pos_w);
                    let a_s = if iz == 0 {
                        self.occs.seam_z.get(self.occs.idx_sz(ix, iy))
                    } else {
                        self.occs.occ_get(ix, iy, iz - 1)
                    };
                    let b_s = if iz == nz {
                        false
                    } else {
                        self.occs.occ_get(ix, iy, iz)
                    };
                    let solid_other = if owner_pos_w { b_s } else { a_s };
                    if solid_other {
                        self.grids_water.kz[idx_w] = MaterialId(0);
                        continue;
                    }
                    let face = if owner_pos_w { Face::PosZ } else { Face::NegZ };
                    let mid_w = if owner_pos_w {
                        if iz == 0 {
                            let bx = (ix / s).min(self.sx - 1);
                            let by = (iy / s).min(self.sy - 1);
                            let nb = self.world_block(
                                self.base_x + bx as i32,
                                by as i32,
                                self.base_z - 1,
                            );
                            self.reg
                                .get(nb.id)
                                .map(|ty| ty.material_for_cached(face.role(), nb.state))
                                .unwrap_or(MaterialId(0))
                        } else {
                            let bz = ((iz - 1) / s).min(self.sz - 1);
                            let bx = (ix / s).min(self.sx - 1);
                            let by = (iy / s).min(self.sy - 1);
                            let here = self.buf.get_local(bx, by, bz);
                            self.reg
                                .get(here.id)
                                .map(|ty| ty.material_for_cached(face.role(), here.state))
                                .unwrap_or(MaterialId(0))
                        }
                    } else {
                        let bz = (iz / s).min(self.sz - 1);
                        let bx = (ix / s).min(self.sx - 1);
                        let by = (iy / s).min(self.sy - 1);
                        let here = self.buf.get_local(bx, by, bz);
                        self.reg
                            .get(here.id)
                            .map(|ty| ty.material_for_cached(face.role(), here.state))
                            .unwrap_or(MaterialId(0))
                    };
                    self.grids_water.kz[idx_w] = mid_w;
                }
            }
        }
        let ms: u32 = t0.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
        log::info!(target: "perf", "ms={} mesher_parity_build s={} dims=({}, {}, {}) base_x={} base_z={}", ms, self.s, self.sx, self.sy, self.sz, self.base_x, self.base_z);
    }

    pub fn emit_into<B: crate::emit::BuildSink>(&self, builds: &mut B) {
        // Ensure a shared visited scratch buffer large enough for any axis
        // X: (width,height) = (s*sz, s*sy)
        // Y: (s*sx, s*sz)
        // Z: (s*sx, s*sy)
        let s = self.s;
        let (sx, sy, sz) = (self.sx, self.sy, self.sz);
        let need_x = (s * sz) * (s * sy);
        let need_y = (s * sx) * (s * sz);
        let need_z = (s * sx) * (s * sy);
        let need = need_x.max(need_y).max(need_z);
        VISITED_SCRATCH_V3.with(|cell| {
            let mut buf = cell.borrow_mut();
            if buf.len() < need {
                buf.resize(need, 0);
            }
            // Opaque solids first
            emit_plane_x(
                self.s,
                self.sx,
                self.sy,
                self.sz,
                self.base_x,
                self.base_z,
                &self.grids,
                builds,
                &mut buf[..],
                self.world_scale,
                self.clip_sx,
                self.clip_sy,
                self.clip_sz,
            );
            emit_plane_y(
                self.s,
                self.sx,
                self.sy,
                self.sz,
                self.base_x,
                self.base_z,
                &self.grids,
                builds,
                &mut buf[..],
                self.world_scale,
                self.clip_sx,
                self.clip_sy,
                self.clip_sz,
            );
            emit_plane_z(
                self.s,
                self.sx,
                self.sy,
                self.sz,
                self.base_x,
                self.base_z,
                &self.grids,
                builds,
                &mut buf[..],
                self.world_scale,
                self.clip_sx,
                self.clip_sy,
                self.clip_sz,
            );
            // Water-only faces (transparent pass later)
            if self.include_water {
                emit_plane_x(
                    self.s,
                    self.sx,
                    self.sy,
                    self.sz,
                    self.base_x,
                    self.base_z,
                    &self.grids_water,
                    builds,
                    &mut buf[..],
                    self.world_scale,
                    self.clip_sx,
                    self.clip_sy,
                    self.clip_sz,
                );
                emit_plane_y(
                    self.s,
                    self.sx,
                    self.sy,
                    self.sz,
                    self.base_x,
                    self.base_z,
                    &self.grids_water,
                    builds,
                    &mut buf[..],
                    self.world_scale,
                    self.clip_sx,
                    self.clip_sy,
                    self.clip_sz,
                );
                emit_plane_z(
                    self.s,
                    self.sx,
                    self.sy,
                    self.sz,
                    self.base_x,
                    self.base_z,
                    &self.grids_water,
                    builds,
                    &mut buf[..],
                    self.world_scale,
                    self.clip_sx,
                    self.clip_sy,
                    self.clip_sz,
                );
            }
        });
    }
}

// Emission helpers (cloned from v2 for private use)
fn emit_plane_x<B: crate::emit::BuildSink>(
    s: usize,
    sx: usize,
    sy: usize,
    sz: usize,
    base_x: i32,
    base_z: i32,
    grids: &FaceGrids,
    builds: &mut B,
    visited_buf: &mut [u8],
    world_scale: f32,
    clip_sx: usize,
    clip_sy: usize,
    clip_sz: usize,
) {
    let t0 = Instant::now();
    let scale = world_scale / s as f32;
    let width = s * sz;
    let height = s * sy;
    let needed = width * height;
    debug_assert!(visited_buf.len() >= needed);
    // Reused buffer may contain epochs from previous axis; clear the active window.
    visited_buf[..needed].fill(0);
    let mut epoch: u8 = 1;
    for ix in 0..(s * sx) {
        epoch = epoch.wrapping_add(1);
        if epoch == 0 {
            visited_buf[..needed].fill(0);
            epoch = 1;
        }
        let idx2d = |u: usize, v: usize| v * width + u;
        let mut v = 0usize;
        while v < height {
            let mut u = 0usize;
            while u < width {
                let vi = idx2d(u, v);
                if visited_buf[vi] == epoch {
                    u += 1;
                    continue;
                }
                let idx = grids.idx_x(ix, v, u);
                if !grids.px.get(idx) {
                    u += 1;
                    continue;
                }
                let mid = grids.kx[idx];
                if mid.0 == 0 {
                    u += 1;
                    continue;
                }
                let pos = grids.ox.get(idx);
                let mut run_w = 1usize;
                while u + run_w < width {
                    if visited_buf[idx2d(u + run_w, v)] == epoch {
                        break;
                    }
                    let idx_n = grids.idx_x(ix, v, u + run_w);
                    if !grids.px.get(idx_n) || grids.kx[idx_n] != mid || grids.ox.get(idx_n) != pos
                    {
                        break;
                    }
                    run_w += 1;
                }
                let mut run_h = 1usize;
                'outer: while v + run_h < height {
                    for uu in u..(u + run_w) {
                        if visited_buf[idx2d(uu, v + run_h)] == epoch {
                            break 'outer;
                        }
                        let idx_n = grids.idx_x(ix, v + run_h, uu);
                        if !grids.px.get(idx_n)
                            || grids.kx[idx_n] != mid
                            || grids.ox.get(idx_n) != pos
                        {
                            break 'outer;
                        }
                    }
                    run_h += 1;
                }
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
                    builds, mid, face, origin, u1, v1, rgba, base_x, clip_sx, clip_sy, base_z,
                    clip_sz,
                );
                for dv in 0..run_h {
                    for du in 0..run_w {
                        visited_buf[idx2d(u + du, v + dv)] = epoch;
                    }
                }
                u += run_w;
            }
            v += 1;
        }
    }
    let ms: u32 = t0.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    log::info!(target: "perf", "ms={} mesher_emit_plane axis=X s={} dims=({}, {}, {}) base_x={} base_z={}", ms, s, sx, sy, sz, base_x, base_z);
}

fn emit_plane_y<B: crate::emit::BuildSink>(
    s: usize,
    sx: usize,
    sy: usize,
    sz: usize,
    base_x: i32,
    base_z: i32,
    grids: &FaceGrids,
    builds: &mut B,
    visited_buf: &mut [u8],
    world_scale: f32,
    clip_sx: usize,
    clip_sy: usize,
    clip_sz: usize,
) {
    let t0 = Instant::now();
    let scale = world_scale / s as f32;
    let width = s * sx;
    let height = s * sz;
    let needed = width * height;
    debug_assert!(visited_buf.len() >= needed);
    // Reused buffer may contain epochs from previous axis; clear the active window.
    visited_buf[..needed].fill(0);
    let mut epoch: u8 = 1;
    for iy in 0..(s * sy) {
        epoch = epoch.wrapping_add(1);
        if epoch == 0 {
            visited_buf[..needed].fill(0);
            epoch = 1;
        }
        let idx2d = |u: usize, v: usize| v * width + u;
        let mut v = 0usize;
        while v < height {
            let mut u = 0usize;
            while u < width {
                let vi = idx2d(u, v);
                if visited_buf[vi] == epoch {
                    u += 1;
                    continue;
                }
                let idx = grids.idx_y(u, iy, v);
                if !grids.py.get(idx) {
                    u += 1;
                    continue;
                }
                let mid = grids.ky[idx];
                if mid.0 == 0 {
                    u += 1;
                    continue;
                }
                let pos = grids.oy.get(idx);
                let mut run_w = 1usize;
                while u + run_w < width {
                    if visited_buf[idx2d(u + run_w, v)] == epoch {
                        break;
                    }
                    let idx_n = grids.idx_y(u + run_w, iy, v);
                    if !grids.py.get(idx_n) || grids.ky[idx_n] != mid || grids.oy.get(idx_n) != pos
                    {
                        break;
                    }
                    run_w += 1;
                }
                let mut run_h = 1usize;
                'outer: while v + run_h < height {
                    for uu in u..(u + run_w) {
                        if visited_buf[idx2d(uu, v + run_h)] == epoch {
                            break 'outer;
                        }
                        let idx_n = grids.idx_y(uu, iy, v + run_h);
                        if !grids.py.get(idx_n)
                            || grids.ky[idx_n] != mid
                            || grids.oy.get(idx_n) != pos
                        {
                            break 'outer;
                        }
                    }
                    run_h += 1;
                }
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
                    builds, mid, face, origin, u1, v1, rgba, base_x, clip_sx, clip_sy, base_z,
                    clip_sz,
                );
                for dv in 0..run_h {
                    for du in 0..run_w {
                        visited_buf[idx2d(u + du, v + dv)] = epoch;
                    }
                }
                u += run_w;
            }
            v += 1;
        }
    }
    let ms: u32 = t0.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    log::info!(target: "perf", "ms={} mesher_emit_plane axis=Y s={} dims=({}, {}, {}) base_x={} base_z={}", ms, s, sx, sy, sz, base_x, base_z);
}

fn emit_plane_z<B: crate::emit::BuildSink>(
    s: usize,
    sx: usize,
    sy: usize,
    sz: usize,
    base_x: i32,
    base_z: i32,
    grids: &FaceGrids,
    builds: &mut B,
    visited_buf: &mut [u8],
    world_scale: f32,
    clip_sx: usize,
    clip_sy: usize,
    clip_sz: usize,
) {
    let t0 = Instant::now();
    let scale = world_scale / s as f32;
    let width = s * sx;
    let height = s * sy;
    let needed = width * height;
    debug_assert!(visited_buf.len() >= needed);
    // Reused buffer may contain epochs from previous axis; clear the active window.
    visited_buf[..needed].fill(0);
    let mut epoch: u8 = 1;
    for iz in 0..(s * sz) {
        epoch = epoch.wrapping_add(1);
        if epoch == 0 {
            visited_buf[..needed].fill(0);
            epoch = 1;
        }
        let idx2d = |u: usize, v: usize| v * width + u;
        let mut v = 0usize;
        while v < height {
            let mut u = 0usize;
            while u < width {
                let vi = idx2d(u, v);
                if visited_buf[vi] == epoch {
                    u += 1;
                    continue;
                }
                let idx = grids.idx_z(u, v, iz);
                if !grids.pz.get(idx) {
                    u += 1;
                    continue;
                }
                let mid = grids.kz[idx];
                if mid.0 == 0 {
                    u += 1;
                    continue;
                }
                let pos = grids.oz.get(idx);
                let mut run_w = 1usize;
                while u + run_w < width {
                    if visited_buf[idx2d(u + run_w, v)] == epoch {
                        break;
                    }
                    let idx_n = grids.idx_z(u + run_w, v, iz);
                    if !grids.pz.get(idx_n) || grids.kz[idx_n] != mid || grids.oz.get(idx_n) != pos
                    {
                        break;
                    }
                    run_w += 1;
                }
                let mut run_h = 1usize;
                'outer: while v + run_h < height {
                    for uu in u..(u + run_w) {
                        if visited_buf[idx2d(uu, v + run_h)] == epoch {
                            break 'outer;
                        }
                        let idx_n = grids.idx_z(uu, v + run_h, iz);
                        if !grids.pz.get(idx_n)
                            || grids.kz[idx_n] != mid
                            || grids.oz.get(idx_n) != pos
                        {
                            break 'outer;
                        }
                    }
                    run_h += 1;
                }
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
                    builds, mid, face, origin, u1, v1, rgba, base_x, clip_sx, clip_sy, base_z,
                    clip_sz,
                );
                for dv in 0..run_h {
                    for du in 0..run_w {
                        visited_buf[idx2d(u + du, v + dv)] = epoch;
                    }
                }
                u += run_w;
            }
            v += 1;
        }
    }
    let ms: u32 = t0.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    log::info!(target: "perf", "ms={} mesher_emit_plane axis=Z s={} dims=({}, {}, {}) base_x={} base_z={}", ms, s, sx, sy, sz, base_x, base_z);
}

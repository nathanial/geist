use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Instant;

use geist_blocks::BlockRegistry;
use geist_blocks::registry::BlockType;
use geist_blocks::types::{Block, MaterialId};
use geist_chunk::ChunkBuf;
use geist_geom::{Aabb, Vec3};
use geist_lighting::{LightBorders, LightGrid, LightingStore, compute_light_with_borders_buf};
use geist_world::World;

use crate::chunk::ChunkMeshCPU;
use crate::constants::MICROGRID_STEPS;
use crate::emit::emit_box_generic_clipped;
use crate::face::Face;
use crate::mesh_build::MeshBuild;
use crate::parity::ParityMesher;
use crate::util::{for_each_micro_box, is_occluder, is_top_half_shape, unknown_material_id};

thread_local! {
    static LAST_MESH_RESERVE: RefCell<Vec<usize>> = RefCell::new(Vec::new());
}

const LIGHT_FULL: u8 = 255;

#[derive(Default)]
struct MesherPerf {
    scan_ms: u32,
    seed_ms: u32,
    emit_ms: u32,
    thin_ms: u32,
    total_ms: u32,
}

struct WccOutput {
    builds: Vec<MeshBuild>,
    scan_ms: u32,
    seed_ms: u32,
    emit_ms: u32,
}

fn elapsed_ms(start: Instant) -> u32 {
    start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32
}

fn prepare_builds(mat_count: usize) -> Vec<MeshBuild> {
    let mut builds = vec![MeshBuild::default(); mat_count];
    LAST_MESH_RESERVE.with(|cell| {
        let mut caps = cell.borrow_mut();
        if caps.len() != mat_count {
            caps.resize(mat_count, 64);
        }
        for i in 0..mat_count {
            let reserve = caps[i].max(64);
            builds[i].reserve_quads(reserve);
        }
    });
    builds
}

fn run_wcc_phase(
    buf: &ChunkBuf,
    reg: &BlockRegistry,
    world: &World,
    edits: Option<&HashMap<(i32, i32, i32), Block>>,
    s: usize,
    base_x: i32,
    base_z: i32,
    mat_count: usize,
) -> WccOutput {
    let mut pm = ParityMesher::new(buf, reg, s, base_x, base_z, world, edits);

    let t_scan_start = Instant::now();
    pm.build_occupancy();
    let scan_ms = elapsed_ms(t_scan_start);

    let mut builds = prepare_builds(mat_count);

    let t_seed_start = Instant::now();
    pm.seed_seam_layers();
    let seed_ms = elapsed_ms(t_seed_start);

    let t_emit_start = Instant::now();
    pm.compute_parity_and_materials();
    pm.emit_into(&mut builds);
    let emit_ms = elapsed_ms(t_emit_start);

    pm.recycle();

    WccOutput {
        builds,
        scan_ms,
        seed_ms,
        emit_ms,
    }
}

fn thin_dynamic_shapes(
    builds: &mut Vec<MeshBuild>,
    buf: &ChunkBuf,
    reg: &BlockRegistry,
    world: &World,
    edits: Option<&HashMap<(i32, i32, i32), Block>>,
    base_x: i32,
    base_z: i32,
    sx: usize,
    sy: usize,
    sz: usize,
) -> u32 {
    let start = Instant::now();
    for z in 0..sz {
        for y in 0..sy {
            for x in 0..sx {
                let here = buf.get_local(x, y, z);
                if let Some(ty) = reg.get(here.id) {
                    if ty.variant(here.state).occupancy.is_some() {
                        continue;
                    }
                    let fx = (base_x + x as i32) as f32;
                    let fy = y as f32;
                    let fz = (base_z + z as i32) as f32;
                    emit_thin_shape(
                        builds, buf, reg, world, edits, here, ty, fx, fy, fz, base_x, base_z, sx,
                        sy, sz,
                    );
                }
            }
        }
    }
    elapsed_ms(start)
}

fn emit_thin_shape(
    builds: &mut Vec<MeshBuild>,
    buf: &ChunkBuf,
    reg: &BlockRegistry,
    world: &World,
    edits: Option<&HashMap<(i32, i32, i32), Block>>,
    here: Block,
    ty: &BlockType,
    fx: f32,
    fy: f32,
    fz: f32,
    base_x: i32,
    base_z: i32,
    sx: usize,
    sy: usize,
    sz: usize,
) {
    use geist_blocks::types::Shape;

    match &ty.shape {
        Shape::Pane => emit_pane(
            builds, buf, reg, world, edits, here, ty, fx, fy, fz, base_x, base_z, sx, sy, sz,
        ),
        Shape::Fence => emit_fence(
            builds, buf, reg, world, edits, here, ty, fx, fy, fz, base_x, base_z, sx, sy, sz,
        ),
        Shape::Carpet => emit_carpet(
            builds, buf, reg, world, edits, here, ty, fx, fy, fz, base_x, base_z, sx, sy, sz,
        ),
        _ => {}
    }
}

fn emit_pane(
    builds: &mut Vec<MeshBuild>,
    buf: &ChunkBuf,
    reg: &BlockRegistry,
    world: &World,
    edits: Option<&HashMap<(i32, i32, i32), Block>>,
    here: Block,
    ty: &BlockType,
    fx: f32,
    fy: f32,
    fz: f32,
    base_x: i32,
    base_z: i32,
    sx: usize,
    sy: usize,
    sz: usize,
) {
    let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
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
        builds,
        min,
        max,
        &face_material,
        |face| {
            let (dx, dy, dz) = face.delta();
            let nx = fx as i32 + dx;
            let ny = fy as i32 + dy;
            let nz = fz as i32 + dz;
            is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
        },
        |_| LIGHT_FULL,
        base_x,
        sx,
        sy,
        base_z,
        sz,
    );

    let wx = fx as i32;
    let wy = fy as i32;
    let wz = fz as i32;
    let connect_zp = crate::util::neighbor_is_pane(buf, reg, wx, wy, wz, Face::PosZ);
    let connect_zn = crate::util::neighbor_is_pane(buf, reg, wx, wy, wz, Face::NegZ);
    let connect_xp = crate::util::neighbor_is_pane(buf, reg, wx, wy, wz, Face::PosX);
    let connect_xn = crate::util::neighbor_is_pane(buf, reg, wx, wy, wz, Face::NegX);

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
            builds,
            min,
            max,
            &face_material,
            |_| false,
            |_| LIGHT_FULL,
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
            builds,
            min,
            max,
            &face_material,
            |_| false,
            |_| LIGHT_FULL,
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
            z: fz,
        };
        let max = Vec3 {
            x: fx + 0.5 + t,
            y: fy + 1.0,
            z: fz + 0.5 - t,
        };
        emit_box_generic_clipped(
            builds,
            min,
            max,
            &face_material,
            |_| false,
            |_| LIGHT_FULL,
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
            builds,
            min,
            max,
            &face_material,
            |_| false,
            |_| LIGHT_FULL,
            base_x,
            sx,
            sy,
            base_z,
            sz,
        );
    }
}

fn emit_fence(
    builds: &mut Vec<MeshBuild>,
    buf: &ChunkBuf,
    reg: &BlockRegistry,
    world: &World,
    edits: Option<&HashMap<(i32, i32, i32), Block>>,
    here: Block,
    ty: &BlockType,
    fx: f32,
    fy: f32,
    fz: f32,
    base_x: i32,
    base_z: i32,
    sx: usize,
    sy: usize,
    sz: usize,
) {
    let t = 0.125f32;
    let p = 0.375f32;
    let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
    emit_box_generic_clipped(
        builds,
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
        &face_material,
        |face| {
            let (dx, dy, dz) = face.delta();
            let nx = fx as i32 + dx;
            let ny = fy as i32 + dy;
            let nz = fz as i32 + dz;
            is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
        },
        |_| LIGHT_FULL,
        base_x,
        sx,
        sy,
        base_z,
        sz,
    );
    for &(dx, dz, _face, ox, oz) in &crate::face::SIDE_NEIGHBORS {
        if let Some(nb) = buf.get_world((fx as i32) + dx, fy as i32, (fz as i32) + dz) {
            if let Some(nb_ty) = reg.get(nb.id) {
                if matches!(
                    nb_ty.shape,
                    geist_blocks::types::Shape::Fence | geist_blocks::types::Shape::Pane
                ) {
                    emit_box_generic_clipped(
                        builds,
                        Vec3 {
                            x: fx + 0.5 - t,
                            y: fy + 0.5,
                            z: fz + 0.5 - t,
                        },
                        Vec3 {
                            x: fx + 0.5 + t,
                            y: fy + 1.0,
                            z: fz + 0.5 + t,
                        },
                        &face_material,
                        |face| {
                            let (dx, dy, dz) = face.delta();
                            let nx = fx as i32 + dx;
                            let ny = fy as i32 + dy;
                            let nz = fz as i32 + dz;
                            is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
                        },
                        |_| LIGHT_FULL,
                        base_x,
                        sx,
                        sy,
                        base_z,
                        sz,
                    );
                    let (x0, z0) = (fx + ox * p, fz + oz * p);
                    let (x1, z1) = (fx + ox * 0.5, fz + oz * 0.5);
                    emit_box_generic_clipped(
                        builds,
                        Vec3 {
                            x: x0 - t,
                            y: fy + 0.375,
                            z: z0 - t,
                        },
                        Vec3 {
                            x: x1 + t,
                            y: fy + 0.375 + 0.125,
                            z: z1 + t,
                        },
                        &face_material,
                        |face| {
                            let (dx, dy, dz) = face.delta();
                            let nx = fx as i32 + dx;
                            let ny = fy as i32 + dy;
                            let nz = fz as i32 + dz;
                            is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
                        },
                        |_| LIGHT_FULL,
                        base_x,
                        sx,
                        sy,
                        base_z,
                        sz,
                    );
                    emit_box_generic_clipped(
                        builds,
                        Vec3 {
                            x: x0 - t,
                            y: fy + 0.75,
                            z: z0 - t,
                        },
                        Vec3 {
                            x: x1 + t,
                            y: fy + 0.75 + 0.125,
                            z: z1 + t,
                        },
                        &face_material,
                        |face| {
                            let (dx, dy, dz) = face.delta();
                            let nx = fx as i32 + dx;
                            let ny = fy as i32 + dy;
                            let nz = fz as i32 + dz;
                            is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
                        },
                        |_| LIGHT_FULL,
                        base_x,
                        sx,
                        sy,
                        base_z,
                        sz,
                    );
                }
            }
        }
    }
}

fn emit_carpet(
    builds: &mut Vec<MeshBuild>,
    buf: &ChunkBuf,
    reg: &BlockRegistry,
    world: &World,
    edits: Option<&HashMap<(i32, i32, i32), Block>>,
    here: Block,
    ty: &BlockType,
    fx: f32,
    fy: f32,
    fz: f32,
    base_x: i32,
    base_z: i32,
    sx: usize,
    sy: usize,
    sz: usize,
) {
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
    let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
    emit_box_generic_clipped(
        builds,
        min,
        max,
        &face_material,
        |face| {
            let (dx, dy, dz) = face.delta();
            let nx = fx as i32 + dx;
            let ny = fy as i32 + dy;
            let nz = fz as i32 + dz;
            is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
        },
        |_| LIGHT_FULL,
        base_x,
        sx,
        sy,
        base_z,
        sz,
    );
}

fn log_mesher_perf(s: usize, cx: i32, cz: i32, perf: &MesherPerf) {
    log::info!(
        target: "perf",
        "ms scan={} seed={} emit={} thin={} total={} mesher_sections s={} cx={} cz={}",
        perf.scan_ms,
        perf.seed_ms,
        perf.emit_ms,
        perf.thin_ms,
        perf.total_ms,
        s,
        cx,
        cz
    );
}

fn update_last_mesh_reserve(mat_count: usize, builds: &[MeshBuild]) {
    LAST_MESH_RESERVE.with(|cell| {
        let mut caps = cell.borrow_mut();
        caps.resize(mat_count, 64);
        for (i, mb) in builds.iter().enumerate() {
            let quads = mb.pos.len() / 12;
            let suggested = quads + quads / 4 + 64;
            caps[i] = suggested.max(64);
        }
    });
}

fn finalize_chunk(
    builds: Vec<MeshBuild>,
    light: &LightGrid,
    base_x: i32,
    base_z: i32,
    sx: usize,
    sy: usize,
    sz: usize,
    cx: i32,
    cz: i32,
) -> (ChunkMeshCPU, Option<LightBorders>) {
    let mat_count = builds.len();
    update_last_mesh_reserve(mat_count, &builds);

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
    let light_borders = Some(LightBorders::from_grid(light));
    let non_empty = builds.iter().filter(|mb| !mb.pos.is_empty()).count();
    let mut parts: HashMap<MaterialId, MeshBuild> = HashMap::with_capacity(non_empty);
    for (i, mb) in builds.into_iter().enumerate() {
        if !mb.pos.is_empty() {
            parts.insert(MaterialId(i as u16), mb);
        }
    }

    (
        ChunkMeshCPU {
            cx,
            cz,
            bbox,
            parts,
        },
        light_borders,
    )
}

/// Build a chunk mesh using Watertight Cubical Complex (WCC) at S=1 (full cubes only).
/// Phase 1: Only full cubes contribute; micro/dynamic shapes are ignored here.
/// Builds a chunk mesh using WCC at micro scale, with seam handling and thin-shape pass.
pub fn build_chunk_wcc_cpu_buf(
    buf: &ChunkBuf,
    lighting: Option<&LightingStore>,
    world: &World,
    edits: Option<&HashMap<(i32, i32, i32), Block>>,
    cx: i32,
    cz: i32,
    reg: &BlockRegistry,
) -> Option<(ChunkMeshCPU, Option<LightBorders>)> {
    let light = match lighting {
        Some(store) => compute_light_with_borders_buf(buf, store, reg, world),
        None => return None,
    };

    build_chunk_wcc_cpu_buf_with_light(buf, &light, world, edits, cx, cz, reg)
}

/// Same as `build_chunk_wcc_cpu_buf` but reuses a precomputed `LightGrid`.
pub fn build_chunk_wcc_cpu_buf_with_light(
    buf: &ChunkBuf,
    light: &LightGrid,
    world: &World,
    edits: Option<&HashMap<(i32, i32, i32), Block>>,
    cx: i32,
    cz: i32,
    reg: &BlockRegistry,
) -> Option<(ChunkMeshCPU, Option<LightBorders>)> {
    let sx = buf.sx;
    let sy = buf.sy;
    let sz = buf.sz;
    let base_x = buf.cx * sx as i32;
    let base_z = buf.cz * sz as i32;
    let mat_count = reg.materials.materials.len();

    let s: usize = MICROGRID_STEPS;
    let total_start = Instant::now();

    let WccOutput {
        mut builds,
        scan_ms,
        seed_ms,
        emit_ms,
    } = run_wcc_phase(buf, reg, world, edits, s, base_x, base_z, mat_count);

    let thin_ms = thin_dynamic_shapes(
        &mut builds,
        buf,
        reg,
        world,
        edits,
        base_x,
        base_z,
        sx,
        sy,
        sz,
    );

    let total_ms = elapsed_ms(total_start);
    let perf = MesherPerf {
        scan_ms,
        seed_ms,
        emit_ms,
        thin_ms,
        total_ms,
    };
    log_mesher_perf(s, cx, cz, &perf);

    let (chunk, light_borders) = finalize_chunk(builds, light, base_x, base_z, sx, sy, sz, cx, cz);

    Some((chunk, light_borders))
}

/// Builds a simple voxel body mesh for debug/solid rendering using a flat ambient light.
pub fn build_voxel_body_cpu_buf(buf: &ChunkBuf, ambient: u8, reg: &BlockRegistry) -> ChunkMeshCPU {
    let base_x = buf.cx * buf.sx as i32;
    let base_z = buf.cz * buf.sz as i32;
    let mat_count = reg.materials.materials.len();
    let mut builds_v: Vec<MeshBuild> = vec![MeshBuild::default(); mat_count];
    for z in 0..buf.sz {
        for y in 0..buf.sy {
            for x in 0..buf.sx {
                let here = buf.get_local(x, y, z);
                if !crate::util::is_solid_runtime(here, reg) {
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
                        for_each_micro_box(fx, fy, fz, occ, |min, max| {
                            emit_box_generic_clipped(
                                &mut builds_v,
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
                        });
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
                            &mut builds_v,
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
                            &mut builds_v,
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
                            &mut builds_v,
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
                        // Connectors by side neighbors
                        for &(dx, dz, _face, ox, oz) in &crate::face::SIDE_NEIGHBORS {
                            if let Some(nb) =
                                buf.get_world((fx as i32) + dx, fy as i32, (fz as i32) + dz)
                            {
                                if let Some(nb_ty) = reg.get(nb.id) {
                                    if matches!(
                                        nb_ty.shape,
                                        geist_blocks::types::Shape::Fence
                                            | geist_blocks::types::Shape::Pane
                                    ) {
                                        let min = Vec3 {
                                            x: fx + 0.5 - t,
                                            y: fy + 0.5,
                                            z: fz + 0.5 - t,
                                        };
                                        let max = Vec3 {
                                            x: fx + 0.5 + t,
                                            y: fy + 1.0,
                                            z: fz + 0.5 + t,
                                        };
                                        boxes.push((min, max));
                                        let (x0, z0) = (fx + ox * p, fz + oz * p);
                                        let (x1, z1) = (fx + ox * 0.5, fz + oz * 0.5);
                                        // Lower bar
                                        boxes.push((
                                            Vec3 {
                                                x: x0 - t,
                                                y: fy + 0.375,
                                                z: z0 - t,
                                            },
                                            Vec3 {
                                                x: x1 + t,
                                                y: fy + 0.375 + 0.125,
                                                z: z1 + t,
                                            },
                                        ));
                                        // Upper bar
                                        boxes.push((
                                            Vec3 {
                                                x: x0 - t,
                                                y: fy + 0.75,
                                                z: z0 - t,
                                            },
                                            Vec3 {
                                                x: x1 + t,
                                                y: fy + 0.75 + 0.125,
                                                z: z1 + t,
                                            },
                                        ));
                                    }
                                }
                            }
                        }
                        for (min, max) in boxes {
                            emit_box_generic_clipped(
                                &mut builds_v,
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
                            &mut builds_v,
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
    // Update last reserve caps for this thread based on usage
    LAST_MESH_RESERVE.with(|cell| {
        let mut caps = cell.borrow_mut();
        caps.resize(mat_count, 64);
        for i in 0..mat_count {
            let quads = builds_v[i].pos.len() / 12;
            let suggested = quads + quads / 4 + 64;
            caps[i] = suggested.max(64);
        }
    });

    let non_empty = builds_v.iter().filter(|mb| !mb.pos.is_empty()).count();
    let mut parts_hm: HashMap<MaterialId, MeshBuild> = HashMap::with_capacity(non_empty);
    for (i, mb) in builds_v.into_iter().enumerate() {
        if !mb.pos.is_empty() {
            parts_hm.insert(MaterialId(i as u16), mb);
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
        parts: parts_hm,
    }
}

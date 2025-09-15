use std::collections::HashMap;
use std::time::Instant;
use std::cell::RefCell;

use geist_blocks::BlockRegistry;
use geist_blocks::types::{Block, MaterialId};
use geist_chunk::ChunkBuf;
use geist_geom::{Aabb, Vec3};
use geist_lighting::{LightBorders, LightGrid, LightingStore, compute_light_with_borders_buf};
use geist_world::World;

use crate::chunk::ChunkMeshCPU;
use crate::emit::{emit_box_generic_clipped, BuildSink};
use crate::face::Face;
use crate::mesh_build::MeshBuild;
use crate::util::{is_occluder, is_top_half_shape, microgrid_boxes, for_each_micro_box, unknown_material_id};
use crate::parity::ParityMesher;
use crate::constants::MICROGRID_STEPS;

thread_local! {
    static LAST_MESH_RESERVE: RefCell<Vec<usize>> = RefCell::new(Vec::new());
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

    // Phase 2: Use S=MICROGRID_STEPS mesher for full + micro occupancy.
    let s: usize = MICROGRID_STEPS;
    let t_total = Instant::now();

    let mut pm = ParityMesher::new(buf, reg, s, base_x, base_z, world, edits);

    let t_scan_start = Instant::now();
    pm.build_occupancy();
    let scan_ms: u32 = t_scan_start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;

    // Use a dense per-material vector for cache-friendly writes during emission,
    // then convert to HashMap<MaterialId, MeshBuild> for public API compatibility.
    let mat_count = reg.materials.materials.len();
    let mut builds_v: Vec<MeshBuild> = vec![MeshBuild::default(); mat_count];
    // Pre-reserve per-material meshes based on last chunk usage in this thread
    LAST_MESH_RESERVE.with(|cell| {
        let mut caps = cell.borrow_mut();
        if caps.len() != mat_count { caps.resize(mat_count, 64); }
        for i in 0..mat_count {
            let q = caps[i].max(64);
            builds_v[i].reserve_quads(q);
        }
    });
    // Overscan: incorporate neighbor seam contributions before emission
    let t_seed_start = Instant::now();
    pm.seed_seam_layers();
    let seed_ms: u32 = t_seed_start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;

    let t_emit_start = Instant::now();
    pm.compute_parity_and_materials();
    pm.emit_into(&mut builds_v);
    let emit_ms: u32 = t_emit_start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    // Return FaceGrids to scratch for reuse before thin pass
    pm.recycle();

    // Phase 3: thin dynamic shapes (pane, fence, gate, carpet)
    let t_thin_start = Instant::now();
    for z in 0..sz {
        for y in 0..sy {
            for x in 0..sx {
                let here = buf.get_local(x, y, z);
                let fx = (base_x + x as i32) as f32;
                let fy = y as f32;
                let fz = (base_z + z as i32) as f32;
                if let Some(ty) = reg.get(here.id) {
                    // Skip occupancy-driven shapes; those already went through WCC.
                    if ty.variant(here.state).occupancy.is_some() { continue; }
                    match &ty.shape {
                        geist_blocks::types::Shape::Pane => {
                            let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
                            let t = 0.0625f32;
                            let min = Vec3 { x: fx + 0.5 - t, y: fy, z: fz };
                            let max = Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 1.0 };
                            emit_box_generic_clipped(
                                &mut builds_v,
                                min,
                                max,
                                &face_material,
                                |face| {
                                    let (dx, dy, dz) = face.delta();
                                    let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                    is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
                                },
                                |_face| { 255u8 },
                                base_x, sx, sy, base_z, sz,
                            );
                            // Add side connectors to adjacent panes
                            let wx = fx as i32; let wy = fy as i32; let wz = fz as i32;
                            let connect_zp = crate::util::neighbor_is_pane(buf, reg, wx, wy, wz, Face::PosZ);
                            let connect_zn = crate::util::neighbor_is_pane(buf, reg, wx, wy, wz, Face::NegZ);
                            let connect_xp = crate::util::neighbor_is_pane(buf, reg, wx, wy, wz, Face::PosX);
                            let connect_xn = crate::util::neighbor_is_pane(buf, reg, wx, wy, wz, Face::NegX);
                            let t = 0.0625f32;
                            if connect_xn {
                                let min = Vec3 { x: fx + 0.0, y: fy, z: fz + 0.5 - t };
                                let max = Vec3 { x: fx + 0.5 - t, y: fy + 1.0, z: fz + 0.5 + t };
                                emit_box_generic_clipped(&mut builds_v, min, max, &face_material, |_face| false, |_face| { 255u8 }, base_x, sx, sy, base_z, sz);
                            }
                            if connect_xp {
                                let min = Vec3 { x: fx + 0.5 + t, y: fy, z: fz + 0.5 - t };
                                let max = Vec3 { x: fx + 1.0, y: fy + 1.0, z: fz + 0.5 + t };
                                emit_box_generic_clipped(&mut builds_v, min, max, &face_material, |_face| false, |_face| { 255u8 }, base_x, sx, sy, base_z, sz);
                            }
                            if connect_zn {
                                let min = Vec3 { x: fx + 0.5 - t, y: fy, z: fz + 0.0 };
                                let max = Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 0.5 - t };
                                emit_box_generic_clipped(&mut builds_v, min, max, &face_material, |_face| false, |_face| { 255u8 }, base_x, sx, sy, base_z, sz);
                            }
                            if connect_zp {
                                let min = Vec3 { x: fx + 0.5 - t, y: fy, z: fz + 0.5 + t };
                                let max = Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 1.0 };
                                emit_box_generic_clipped(&mut builds_v, min, max, &face_material, |_face| false, |_face| { 255u8 }, base_x, sx, sy, base_z, sz);
                            }
                        }
                        geist_blocks::types::Shape::Fence => {
                            let t = 0.125f32; let p = 0.375f32;
                            let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
                            // Center post
                            emit_box_generic_clipped(
                                &mut builds_v,
                                Vec3 { x: fx + 0.5 - t, y: fy, z: fz + 0.5 - t },
                                Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 0.5 + t },
                                &face_material,
                                |face| {
                                    let (dx, dy, dz) = face.delta();
                                    let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                    is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
                                },
                                |_face| { 255u8 },
                                base_x, sx, sy, base_z, sz,
                            );
                            // Connectors by side neighbors
                            for &(dx, dz, _face, ox, oz) in &crate::face::SIDE_NEIGHBORS {
                                if let Some(nb) = buf.get_world((fx as i32) + dx, fy as i32, (fz as i32) + dz) {
                                    if let Some(nb_ty) = reg.get(nb.id) {
                                        if matches!(nb_ty.shape, geist_blocks::types::Shape::Fence | geist_blocks::types::Shape::Pane) {
                                            // Vertical connector (top half)
                                            emit_box_generic_clipped(
                                                &mut builds_v,
                                                Vec3 { x: fx + 0.5 - t, y: fy + 0.5, z: fz + 0.5 - t },
                                                Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 0.5 + t },
                                                &face_material,
                                                |face| {
                                                    let (dx, dy, dz) = face.delta();
                                                    let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                                    is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
                                                },
                                                |_face| { 255u8 },
                                                base_x, sx, sy, base_z, sz,
                                            );
                                            // Horizontal bars toward neighbor
                                            let (x0, z0) = (fx + ox * p, fz + oz * p);
                                            let (x1, z1) = (fx + ox * 0.5, fz + oz * 0.5);
                                            // Lower bar
                                            emit_box_generic_clipped(
                                                &mut builds_v,
                                                Vec3 { x: x0 - t, y: fy + 0.375, z: z0 - t },
                                                Vec3 { x: x1 + t, y: fy + 0.375 + 0.125, z: z1 + t },
                                                &face_material,
                                                |face| {
                                                    let (dx, dy, dz) = face.delta();
                                                    let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                                    is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
                                                },
                                                |_face| { 255u8 },
                                                base_x, sx, sy, base_z, sz,
                                            );
                                            // Upper bar
                                            emit_box_generic_clipped(
                                                &mut builds_v,
                                                Vec3 { x: x0 - t, y: fy + 0.75, z: z0 - t },
                                                Vec3 { x: x1 + t, y: fy + 0.75 + 0.125, z: z1 + t },
                                                &face_material,
                                                |face| {
                                                    let (dx, dy, dz) = face.delta();
                                                    let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                                    is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
                                                },
                                                |_face| { 255u8 },
                                                base_x, sx, sy, base_z, sz,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        geist_blocks::types::Shape::Carpet => {
                            let h = 0.0625f32;
                            let min = Vec3 { x: fx, y: fy, z: fz };
                            let max = Vec3 { x: fx + 1.0, y: fy + h, z: fz + 1.0 };
                            let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
                            emit_box_generic_clipped(&mut builds_v, min, max, &face_material, |face| {
                                let (dx, dy, dz) = face.delta();
                                let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
                            }, |_face| { 255u8 }, base_x, sx, sy, base_z, sz);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    let thin_ms: u32 = t_thin_start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;

    // Aggregate + log perf for mesher sections
    let total_ms: u32 = t_total.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    log::info!(
        target: "perf",
        "ms scan={} seed={} emit={} thin={} total={} mesher_sections s={} cx={} cz={}",
        scan_ms,
        seed_ms,
        emit_ms,
        thin_ms,
        total_ms,
        s,
        cx,
        cz
    );

    // Update last reserve caps based on actual quads used (positions/12)
    LAST_MESH_RESERVE.with(|cell| {
        let mut caps = cell.borrow_mut();
        caps.resize(mat_count, 64);
        for i in 0..mat_count {
            let quads = builds_v[i].pos.len() / 12;
            // Keep some headroom to reduce reallocations next time
            let suggested = quads + quads / 4 + 64;
            caps[i] = suggested.max(64);
        }
    });

    let bbox = Aabb {
        min: Vec3 { x: base_x as f32, y: 0.0, z: base_z as f32 },
        max: Vec3 { x: base_x as f32 + sx as f32, y: sy as f32, z: base_z as f32 + sz as f32 },
    };
    let light_borders = Some(LightBorders::from_grid(light));
    // Convert dense vector into sparse HashMap
    let non_empty = builds_v.iter().filter(|mb| !mb.pos.is_empty()).count();
    let mut builds_hm: HashMap<MaterialId, MeshBuild> = HashMap::with_capacity(non_empty);
    for (i, mb) in builds_v.into_iter().enumerate() {
        if !mb.pos.is_empty() {
            builds_hm.insert(MaterialId(i as u16), mb);
        }
    }
    Some((ChunkMeshCPU { cx, cz, bbox, parts: builds_hm }, light_borders))
}

/// Builds a chunk mesh directly into a provided sink without allocating output containers.
/// Returns `Some(LightBorders)` if lighting is enabled.
pub fn build_chunk_into_sink_with_light(
    buf: &ChunkBuf,
    light: &LightGrid,
    world: &World,
    edits: Option<&HashMap<(i32, i32, i32), Block>>,
    cx: i32,
    cz: i32,
    reg: &BlockRegistry,
    sink: &mut impl BuildSink,
) -> Option<LightBorders> {
    let sx = buf.sx;
    let sy = buf.sy;
    let sz = buf.sz;
    let base_x = buf.cx * sx as i32;
    let base_z = buf.cz * sz as i32;

    let s: usize = MICROGRID_STEPS;
    let t_total = Instant::now();
    let mut pm = ParityMesher::new(buf, reg, s, base_x, base_z, world, edits);

    let t_scan_start = Instant::now();
    pm.build_occupancy();
    let scan_ms: u32 = t_scan_start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;

    let t_seed_start = Instant::now();
    pm.seed_seam_layers();
    let seed_ms: u32 = t_seed_start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;

    let t_emit_start = Instant::now();
    pm.compute_parity_and_materials();
    pm.emit_into(sink);
    let emit_ms: u32 = t_emit_start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    pm.recycle();

    // Thin dynamic shapes (pane, fence, carpet)
    let t_thin_start = Instant::now();
    for z in 0..sz {
        for y in 0..sy {
            for x in 0..sx {
                let here = buf.get_local(x, y, z);
                let fx = (base_x + x as i32) as f32;
                let fy = y as f32;
                let fz = (base_z + z as i32) as f32;
                if let Some(ty) = reg.get(here.id) {
                    if ty.variant(here.state).occupancy.is_some() { continue; }
                    match &ty.shape {
                        geist_blocks::types::Shape::Pane => {
                            let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
                            let t = 0.0625f32;
                            let min = Vec3 { x: fx + 0.5 - t, y: fy, z: fz };
                            let max = Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 1.0 };
                            emit_box_generic_clipped(
                                sink, min, max, &face_material,
                                |face| {
                                    let (dx, dy, dz) = face.delta();
                                    let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                    is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
                                },
                                |face| light.sample_face_local_s2(buf, reg, x, y, z, face.index()),
                                base_x, sx, sy, base_z, sz,
                            );
                            // Add side connectors to adjacent panes
                            let wx = fx as i32; let wy = fy as i32; let wz = fz as i32;
                            let connect_zp = crate::util::neighbor_is_pane(buf, reg, wx, wy, wz, Face::PosZ);
                            let connect_zn = crate::util::neighbor_is_pane(buf, reg, wx, wy, wz, Face::NegZ);
                            let connect_xp = crate::util::neighbor_is_pane(buf, reg, wx, wy, wz, Face::PosX);
                            let connect_xn = crate::util::neighbor_is_pane(buf, reg, wx, wy, wz, Face::NegX);
                            let t = 0.0625f32;
                            if connect_xn {
                                let min = Vec3 { x: fx + 0.0, y: fy, z: fz + 0.5 - t };
                                let max = Vec3 { x: fx + 0.5 - t, y: fy + 1.0, z: fz + 0.5 + t };
                                emit_box_generic_clipped(sink, min, max, &face_material, |_face| false, |face| light.sample_face_local_s2(buf, reg, x, y, z, face.index()), base_x, sx, sy, base_z, sz);
                            }
                            if connect_xp {
                                let min = Vec3 { x: fx + 0.5 + t, y: fy, z: fz + 0.5 - t };
                                let max = Vec3 { x: fx + 1.0, y: fy + 1.0, z: fz + 0.5 + t };
                                emit_box_generic_clipped(sink, min, max, &face_material, |_face| false, |face| light.sample_face_local_s2(buf, reg, x, y, z, face.index()), base_x, sx, sy, base_z, sz);
                            }
                            if connect_zn {
                                let min = Vec3 { x: fx + 0.5 - t, y: fy, z: fz + 0.0 };
                                let max = Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 0.5 - t };
                                emit_box_generic_clipped(sink, min, max, &face_material, |_face| false, |face| light.sample_face_local_s2(buf, reg, x, y, z, face.index()), base_x, sx, sy, base_z, sz);
                            }
                            if connect_zp {
                                let min = Vec3 { x: fx + 0.5 - t, y: fy, z: fz + 0.5 + t };
                                let max = Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 1.0 };
                                emit_box_generic_clipped(sink, min, max, &face_material, |_face| false, |face| light.sample_face_local_s2(buf, reg, x, y, z, face.index()), base_x, sx, sy, base_z, sz);
                            }
                        }
                        geist_blocks::types::Shape::Fence => {
                            let t = 0.125f32; let p = 0.375f32;
                            let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
                            // Center post
                            emit_box_generic_clipped(
                                sink,
                                Vec3 { x: fx + 0.5 - t, y: fy, z: fz + 0.5 - t },
                                Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 0.5 + t },
                                &face_material,
                                |face| {
                                    let (dx, dy, dz) = face.delta();
                                    let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                    is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
                                },
                                |face| light.sample_face_local_s2(buf, reg, x, y, z, face.index()),
                                base_x, sx, sy, base_z, sz,
                            );
                            // Connectors by side neighbors
                            for &(dx, dz, _face, ox, oz) in &crate::face::SIDE_NEIGHBORS {
                                if let Some(nb) = buf.get_world((fx as i32) + dx, fy as i32, (fz as i32) + dz) {
                                    if let Some(nb_ty) = reg.get(nb.id) {
                                        if matches!(nb_ty.shape, geist_blocks::types::Shape::Fence | geist_blocks::types::Shape::Pane) {
                                            // Vertical connector (top half)
                                            emit_box_generic_clipped(
                                                sink,
                                                Vec3 { x: fx + 0.5 - t, y: fy + 0.5, z: fz + 0.5 - t },
                                                Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 0.5 + t },
                                                &face_material,
                                                |face| {
                                                    let (dx, dy, dz) = face.delta();
                                                    let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                                    is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
                                                },
                                                |face| light.sample_face_local_s2(buf, reg, x, y, z, face.index()),
                                                base_x, sx, sy, base_z, sz,
                                            );
                                            // Horizontal bars toward neighbor
                                            let (x0, z0) = (fx + ox * p, fz + oz * p);
                                            let (x1, z1) = (fx + ox * 0.5, fz + oz * 0.5);
                                            // Lower bar
                                            emit_box_generic_clipped(
                                                sink,
                                                Vec3 { x: x0 - t, y: fy + 0.375, z: z0 - t },
                                                Vec3 { x: x1 + t, y: fy + 0.375 + 0.125, z: z1 + t },
                                                &face_material,
                                                |face| {
                                                    let (dx, dy, dz) = face.delta();
                                                    let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                                    is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
                                                },
                                                |face| light.sample_face_local_s2(buf, reg, x, y, z, face.index()),
                                                base_x, sx, sy, base_z, sz,
                                            );
                                            // Upper bar
                                            emit_box_generic_clipped(
                                                sink,
                                                Vec3 { x: x0 - t, y: fy + 0.75, z: z0 - t },
                                                Vec3 { x: x1 + t, y: fy + 0.75 + 0.125, z: z1 + t },
                                                &face_material,
                                                |face| {
                                                    let (dx, dy, dz) = face.delta();
                                                    let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                                    is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
                                                },
                                                |face| light.sample_face_local_s2(buf, reg, x, y, z, face.index()),
                                                base_x, sx, sy, base_z, sz,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        geist_blocks::types::Shape::Carpet => {
                            let h = 0.0625f32;
                            let min = Vec3 { x: fx, y: fy, z: fz };
                            let max = Vec3 { x: fx + 1.0, y: fy + h, z: fz + 1.0 };
                            let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
                            emit_box_generic_clipped(
                                sink,
                                min,
                                max,
                                &face_material,
                                |face| {
                                    let (dx, dy, dz) = face.delta();
                                    let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                    is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
                                },
                                |face| light.sample_face_local_s2(buf, reg, x, y, z, face.index()),
                                base_x, sx, sy, base_z, sz,
                            );
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    let thin_ms: u32 = t_thin_start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    let total_ms: u32 = t_total.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    log::info!(
        target: "perf",
        "ms scan={} seed={} emit={} thin={} total={} mesher_sections s={} cx={} cz={}",
        scan_ms, seed_ms, emit_ms, thin_ms, total_ms, s, cx, cz
    );

    Some(LightBorders::from_grid(light))
}

/// Convenience: builds into a provided `Vec<MeshBuild>`; caller provides a pre-sized vector of length `reg.materials.materials.len()`.
pub fn build_chunk_into_builds_with_light(
    buf: &ChunkBuf,
    light: &LightGrid,
    world: &World,
    edits: Option<&HashMap<(i32, i32, i32), Block>>,
    cx: i32,
    cz: i32,
    reg: &BlockRegistry,
    builds: &mut Vec<MeshBuild>,
) -> Option<LightBorders> {
    let mat_count = reg.materials.materials.len();
    if builds.len() != mat_count { builds.resize(mat_count, MeshBuild::default()); }
    build_chunk_into_sink_with_light(buf, light, world, edits, cx, cz, reg, builds)
}

/// Same as `build_chunk_into_builds_with_light` but takes lighting store; returns `None` if lighting is disabled.
pub fn build_chunk_into_builds(
    buf: &ChunkBuf,
    lighting: Option<&LightingStore>,
    world: &World,
    edits: Option<&HashMap<(i32, i32, i32), Block>>,
    cx: i32,
    cz: i32,
    reg: &BlockRegistry,
    builds: &mut Vec<MeshBuild>,
) -> Option<LightBorders> {
    let light = match lighting {
        Some(store) => compute_light_with_borders_buf(buf, store, reg, world),
        None => return None,
    };
    build_chunk_into_builds_with_light(buf, &light, world, edits, cx, cz, reg, builds)
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
                if !crate::util::is_solid_runtime(here, reg) { continue; }
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
                        let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
                        for_each_micro_box(fx, fy, fz, occ, |min, max| {
                            emit_box_generic_clipped(&mut builds_v, min, max, &face_material, |_face| false, |_face| ambient, base_x, buf.sx, buf.sy, base_z, buf.sz);
                        });
                        continue;
                    }
                }
                match reg.get(here.id).map(|t| &t.shape) {
                    Some(geist_blocks::types::Shape::Cube) | Some(geist_blocks::types::Shape::AxisCube { .. }) => {
                        let min = Vec3 { x: fx, y: fy, z: fz };
                        let max = Vec3 { x: fx + 1.0, y: fy + 1.0, z: fz + 1.0 };
                        emit_box_generic_clipped(&mut builds_v, min, max, &face_material, |_face| false, |_face| ambient, base_x, buf.sx, buf.sy, base_z, buf.sz);
                    }
                    Some(geist_blocks::types::Shape::Slab { .. }) => {
                        let top = is_top_half_shape(here, reg);
                        let min = Vec3 { x: fx, y: if top { fy + 0.5 } else { fy }, z: fz };
                        let max = Vec3 { x: fx + 1.0, y: if top { fy + 1.0 } else { fy + 0.5 }, z: fz + 1.0 };
                        emit_box_generic_clipped(&mut builds_v, min, max, &face_material, |_face| false, |_face| ambient, base_x, buf.sx, buf.sy, base_z, buf.sz);
                    }
                    Some(geist_blocks::types::Shape::Pane) => {
                        let t = 0.0625f32;
                        let min = Vec3 { x: fx + 0.5 - t, y: fy, z: fz };
                        let max = Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 1.0 };
                        emit_box_generic_clipped(&mut builds_v, min, max, &face_material, |_face| false, |_face| ambient, base_x, buf.sx, buf.sy, base_z, buf.sz);
                    }
                    Some(geist_blocks::types::Shape::Fence) => {
                        let t = 0.125f32; let p = 0.375f32;
                        let mut boxes: Vec<(Vec3, Vec3)> = Vec::new();
                        boxes.push((Vec3 { x: fx + 0.5 - t, y: fy, z: fz + 0.5 - t }, Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 0.5 + t }));
                        // Connectors by side neighbors
                        for &(dx, dz, _face, ox, oz) in &crate::face::SIDE_NEIGHBORS {
                            if let Some(nb) = buf.get_world((fx as i32) + dx, fy as i32, (fz as i32) + dz) {
                                if let Some(nb_ty) = reg.get(nb.id) {
                                    if matches!(nb_ty.shape, geist_blocks::types::Shape::Fence | geist_blocks::types::Shape::Pane) {
                                        let min = Vec3 { x: fx + 0.5 - t, y: fy + 0.5, z: fz + 0.5 - t };
                                        let max = Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 0.5 + t };
                                        boxes.push((min, max));
                                        let (x0, z0) = (fx + ox * p, fz + oz * p);
                                        let (x1, z1) = (fx + ox * 0.5, fz + oz * 0.5);
                                        // Lower bar
                                        boxes.push((Vec3 { x: x0 - t, y: fy + 0.375, z: z0 - t }, Vec3 { x: x1 + t, y: fy + 0.375 + 0.125, z: z1 + t }));
                                        // Upper bar
                                        boxes.push((Vec3 { x: x0 - t, y: fy + 0.75, z: z0 - t }, Vec3 { x: x1 + t, y: fy + 0.75 + 0.125, z: z1 + t }));
                                    }
                                }
                            }
                        }
                        for (min, max) in boxes {
                            emit_box_generic_clipped(&mut builds_v, min, max, &face_material, |_face| false, |_face| ambient, base_x, buf.sx, buf.sy, base_z, buf.sz);
                        }
                    }
                    Some(geist_blocks::types::Shape::Carpet) => {
                        let h = 0.0625f32;
                        let min = Vec3 { x: fx, y: fy, z: fz };
                        let max = Vec3 { x: fx + 1.0, y: fy + h, z: fz + 1.0 };
                        emit_box_generic_clipped(&mut builds_v, min, max, &face_material, |_face| false, |_face| ambient, base_x, buf.sx, buf.sy, base_z, buf.sz);
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
        if !mb.pos.is_empty() { parts_hm.insert(MaterialId(i as u16), mb); }
    }
    ChunkMeshCPU {
        cx: buf.cx,
        cz: buf.cz,
        bbox: Aabb { min: Vec3 { x: base_x as f32, y: 0.0, z: base_z as f32 }, max: Vec3 { x: base_x as f32 + buf.sx as f32, y: buf.sy as f32, z: base_z as f32 + buf.sz as f32 } },
        parts: parts_hm,
    }
}

/* PHASE 1 color path removed
// Color-only emission helpers for Phase 1 decoupled lighting
#[inline]
fn collect_face_rect_colors(
    out: &mut std::collections::HashMap<MaterialId, Vec<u8>>,
    mid: MaterialId,
    rgba: [u8; 4],
) {
    let v = out.entry(mid).or_default();
    v.extend_from_slice(&rgba);
    v.extend_from_slice(&rgba);
    v.extend_from_slice(&rgba);
    v.extend_from_slice(&rgba);
}

#[inline]
fn collect_face_rect_colors_clipped(
    out: &mut std::collections::HashMap<MaterialId, Vec<u8>>,
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
    let mut out_span = None;
    match face {
        Face::PosX | Face::NegX => {
            if origin.x >= bx0 && origin.x < bx1 {
                if let Some((z, _u)) = clip_span(origin.z, u1, bz0, bz1) {
                    if let Some((y, _v)) = clip_span(origin.y, v1, by0, by1) {
                        let mut o = origin; o.z = z; o.y = y; out_span = Some(o);
                    }
                }
            }
        }
        Face::PosZ | Face::NegZ => {
            if origin.z >= bz0 && origin.z < bz1 {
                if let Some((x, _u)) = clip_span(origin.x, u1, bx0, bx1) {
                    if let Some((y, _v)) = clip_span(origin.y, v1, by0, by1) {
                        let mut o = origin; o.x = x; o.y = y; out_span = Some(o);
                    }
                }
            }
        }
        Face::PosY | Face::NegY => {
            if origin.y >= by0 && origin.y < by1 {
                if let Some((x, _u)) = clip_span(origin.x, u1, bx0, bx1) {
                    if let Some((z, _v)) = clip_span(origin.z, v1, bz0, bz1) {
                        let mut o = origin; o.x = x; o.z = z; out_span = Some(o);
                    }
                }
            }
        }
    }
    if out_span.is_some() {
        collect_face_rect_colors(out, mid, rgba);
    }
}

#[inline]
fn collect_box_colors_clipped(
    out: &mut std::collections::HashMap<MaterialId, Vec<u8>>,
    min: Vec3,
    max: Vec3,
    fm_for_face: &dyn Fn(Face) -> MaterialId,
    mut occludes: impl FnMut(Face) -> bool,
    mut sample_light: impl FnMut(Face) -> u8,
    base_x: i32,
    sx: usize,
    sy: usize,
    base_z: i32,
    sz: usize,
) {
    const FACE_DATA: [(Face, [usize; 4]); 6] = [
        (crate::face::Face::PosY, [0, 2, 6, 4]),
        (crate::face::Face::NegY, [5, 7, 3, 1]),
        (crate::face::Face::PosX, [6, 2, 3, 7]),
        (crate::face::Face::NegX, [0, 4, 5, 1]),
        (crate::face::Face::PosZ, [4, 6, 7, 5]),
        (crate::face::Face::NegZ, [2, 0, 1, 3]),
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
    for &(face, idxs) in &FACE_DATA {
        if occludes(face) { continue; }
        let rgba = {
            let lv = sample_light(face).max(VISUAL_LIGHT_MIN);
            [lv, lv, lv, OPAQUE_ALPHA]
        };
        let (a, b, c, d) = (corners[idxs[0]], corners[idxs[1]], corners[idxs[2]], corners[idxs[3]]);
        // Derive u1/v1 and origin per face (match MeshBuild's add_face_rect)
        let (u1, v1, origin) = match face {
            Face::PosY | Face::NegY => (max.x - min.x, max.z - min.z, Vec3 { x: a.x.min(b.x).min(c.x).min(d.x), y: a.y.min(b.y).min(c.y).min(d.y), z: a.z.min(b.z).min(c.z).min(d.z) }),
            Face::PosX | Face::NegX => (max.z - min.z, max.y - min.y, Vec3 { x: a.x.min(b.x).min(c.x).min(d.x), y: a.y.min(b.y).min(c.y).min(d.y), z: a.z.min(b.z).min(c.z).min(d.z) }),
            Face::PosZ | Face::NegZ => (max.x - min.x, max.y - min.y, Vec3 { x: a.x.min(b.x).min(c.x).min(d.x), y: a.y.min(b.y).min(c.y).min(d.y), z: a.z.min(b.z).min(c.z).min(d.z) }),
        };
        let mid = fm_for_face(face);
        collect_face_rect_colors_clipped(out, mid, face, origin, u1, v1, rgba, base_x, sx, sy, base_z, sz);
    }
}

/// Compute per-vertex RGBA colors for a chunk using the same deterministic emission order
/// as geometry building, without generating/uploading geometry. Returns per-material color
/// arrays and the computed light borders (for seam propagation).
pub fn compute_chunk_colors_wcc_cpu_buf(
    buf: &ChunkBuf,
    lighting: &LightingStore,
    world: &World,
    edits: Option<&HashMap<(i32, i32, i32), Block>>,
    cx: i32,
    cz: i32,
    reg: &BlockRegistry,
) -> Option<HashMap<MaterialId, Vec<u8>>> {
    let sx = buf.sx; let sy = buf.sy; let sz = buf.sz;
    let base_x = buf.cx * sx as i32; let base_z = buf.cz * sz as i32;
    let light = compute_light_with_borders_buf(buf, lighting, reg, world);
    let s: usize = crate::constants::MICROGRID_STEPS;
    let mut wm = crate::wcc::WccMesher::new(buf, reg, s, base_x, base_z, world, edits);
    // Occupancy (same as geometry pass)
    for z in 0..sz { for y in 0..sy { for x in 0..sx {
        let b = buf.get_local(x, y, z);
        if let Some(ty) = reg.get(b.id) {
            let var = ty.variant(b.state);
            if let Some(occ) = var.occupancy { wm.add_micro(x, y, z, b, occ); continue; }
            if ty.name == "water" { wm.add_water_cube(x, y, z, b); continue; }
        }
        if crate::util::is_full_cube(reg, b) { wm.add_cube(x, y, z, b); }
    }}}
    let mut colors: HashMap<MaterialId, Vec<u8>> = HashMap::new();
    // Seed seams and emit WCC colors
    wm.seed_neighbor_seams();
    wm.emit_colors_into(buf, reg, &light, &mut colors);
    // Thin shapes: collect colors in same order as geometry pass
    for z in 0..sz { for y in 0..sy { for x in 0..sx {
        let here = buf.get_local(x, y, z);
        let fx = (base_x + x as i32) as f32; let fy = y as f32; let fz = (base_z + z as i32) as f32;
        if let Some(ty) = reg.get(here.id) {
            if ty.variant(here.state).occupancy.is_some() { continue; }
            match &ty.shape {
                geist_blocks::types::Shape::Pane => {
                    let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
                    let t = 0.0625f32;
                    let min = Vec3 { x: fx + 0.5 - t, y: fy, z: fz };
                    let max = Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 1.0 };
                    collect_box_colors_clipped(
                        &mut colors, min, max, &face_material,
                        |face| {
                            let (dx, dy, dz) = face.delta();
                            let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                            crate::util::is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
                        },
                        |face| light.sample_face_local_s2(buf, reg, x, y, z, face.index()),
                        base_x, sx, sy, base_z, sz);
                    // Side connectors to adjacent panes
                    let wx = fx as i32; let wy = fy as i32; let wz = fz as i32;
                    let connect_zp = crate::util::neighbor_is_pane(buf, reg, wx, wy, wz, Face::PosZ);
                    let connect_zn = crate::util::neighbor_is_pane(buf, reg, wx, wy, wz, Face::NegZ);
                    let connect_xp = crate::util::neighbor_is_pane(buf, reg, wx, wy, wz, Face::PosX);
                    let connect_xn = crate::util::neighbor_is_pane(buf, reg, wx, wy, wz, Face::NegX);
                    if connect_xn {
                        let min = Vec3 { x: fx + 0.0, y: fy, z: fz + 0.5 - t };
                        let max = Vec3 { x: fx + 0.5 - t, y: fy + 1.0, z: fz + 0.5 + t };
                        collect_box_colors_clipped(&mut colors, min, max, &face_material, |_face| false, |face| light.sample_face_local_s2(buf, reg, x, y, z, face.index()), base_x, sx, sy, base_z, sz);
                    }
                    if connect_xp {
                        let min = Vec3 { x: fx + 0.5 + t, y: fy, z: fz + 0.5 - t };
                        let max = Vec3 { x: fx + 1.0, y: fy + 1.0, z: fz + 0.5 + t };
                        collect_box_colors_clipped(&mut colors, min, max, &face_material, |_face| false, |face| light.sample_face_local_s2(buf, reg, x, y, z, face.index()), base_x, sx, sy, base_z, sz);
                    }
                    if connect_zn {
                        let min = Vec3 { x: fx + 0.5 - t, y: fy, z: fz + 0.0 };
                        let max = Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 0.5 - t };
                        collect_box_colors_clipped(&mut colors, min, max, &face_material, |_face| false, |face| light.sample_face_local_s2(buf, reg, x, y, z, face.index()), base_x, sx, sy, base_z, sz);
                    }
                    if connect_zp {
                        let min = Vec3 { x: fx + 0.5 - t, y: fy, z: fz + 0.5 + t };
                        let max = Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 1.0 };
                        collect_box_colors_clipped(&mut colors, min, max, &face_material, |_face| false, |face| light.sample_face_local_s2(buf, reg, x, y, z, face.index()), base_x, sx, sy, base_z, sz);
                    }
                }
                geist_blocks::types::Shape::Fence => {
                    let t = 0.125f32; let p = 0.375f32;
                    let mut boxes: Vec<(Vec3, Vec3)> = Vec::new();
                    boxes.push((Vec3 { x: fx + 0.5 - t, y: fy, z: fz + 0.5 - t }, Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 0.5 + t }));
                    for &(dx, dz, _face, ox, oz) in &crate::face::SIDE_NEIGHBORS {
                        if let Some(nb) = buf.get_world((fx as i32) + dx, fy as i32, (fz as i32) + dz) {
                            if let Some(nb_ty) = reg.get(nb.id) {
                                if matches!(nb_ty.shape, geist_blocks::types::Shape::Fence | geist_blocks::types::Shape::Pane) {
                                    let min = Vec3 { x: fx + 0.5 - t, y: fy + 0.5, z: fz + 0.5 - t };
                                    let max = Vec3 { x: fx + 0.5 + t, y: fy + 1.0, z: fz + 0.5 + t };
                                    boxes.push((min, max));
                                    let (x0, z0) = (fx + ox * p, fz + oz * p);
                                    let (x1, z1) = (fx + ox * 0.5, fz + oz * 0.5);
                                    boxes.push((Vec3 { x: x0 - t, y: fy + 0.375, z: z0 - t }, Vec3 { x: x1 + t, y: fy + 0.375 + 0.125, z: z1 + t }));
                                    boxes.push((Vec3 { x: x0 - t, y: fy + 0.75, z: z0 - t }, Vec3 { x: x1 + t, y: fy + 0.75 + 0.125, z: z1 + t }));
                                }
                            }
                        }
                    }
                    let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
                    for (min, max) in boxes {
                        collect_box_colors_clipped(&mut colors, min, max, &face_material,
                            |face| {
                                let (dx, dy, dz) = face.delta();
                                let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                                crate::util::is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
                            },
                            |face| light.sample_face_local_s2(buf, reg, x, y, z, face.index()),
                            base_x, sx, sy, base_z, sz);
                    }
                }
                geist_blocks::types::Shape::Carpet => {
                    let h = 0.0625f32;
                    let min = Vec3 { x: fx, y: fy, z: fz };
                    let max = Vec3 { x: fx + 1.0, y: fy + h, z: fz + 1.0 };
                    let face_material = |face: Face| ty.material_for_cached(face.role(), here.state);
                    collect_box_colors_clipped(&mut colors, min, max, &face_material,
                        |face| {
                            let (dx, dy, dz) = face.delta();
                            let (nx, ny, nz) = (fx as i32 + dx, fy as i32 + dy, fz as i32 + dz);
                            crate::util::is_occluder(buf, world, edits, reg, here, face, nx, ny, nz)
                        },
                        |face| light.sample_face_local_s2(buf, reg, x, y, z, face.index()),
                        base_x, sx, sy, base_z, sz);
                }
                _ => {}
            }
        }
    }}}
    Some(colors)
}
*/

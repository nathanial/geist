use std::collections::HashMap;

use geist_blocks::BlockRegistry;
use geist_blocks::types::Block;
use geist_chunk::ChunkBuf;
use geist_lighting::{LightGrid, LightingStore};
use geist_mesh_cpu::{ChunkMeshCPU, WccMesher, NeighborsLoaded};
use geist_world::{World, WorldGenMode};

fn load_registry() -> BlockRegistry {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let vox = root.join("../../assets/voxels");
    BlockRegistry::load_from_paths(vox.join("materials.toml"), vox.join("blocks.toml")).unwrap()
}

fn make_buf(cx: i32, cz: i32, sx: usize, sy: usize, sz: usize, blocks: Vec<Block>) -> ChunkBuf {
    ChunkBuf::from_blocks_local(cx, cz, sx, sy, sz, blocks)
}

fn tri_area_sum(cpu: &ChunkMeshCPU) -> f32 {
    let mut total = 0.0f32;
    for (_mid, part) in &cpu.parts {
        // Use indices if present; otherwise infer by sequential order
        let idx = if !part.idx.is_empty() {
            part.idx.iter().map(|&i| i as usize).collect::<Vec<_>>()
        } else {
            (0..(part.pos.len() / 3)).collect::<Vec<_>>()
        };
        let pos = &part.pos;
        for t in (0..idx.len()).step_by(3) {
            let ia = idx[t] * 3; let ib = idx[t + 1] * 3; let ic = idx[t + 2] * 3;
            let ax = pos[ia]; let ay = pos[ia + 1]; let az = pos[ia + 2];
            let bx = pos[ib]; let by = pos[ib + 1]; let bz = pos[ib + 2];
            let cx = pos[ic]; let cy = pos[ic + 1]; let cz = pos[ic + 2];
            let ab = [bx - ax, by - ay, bz - az];
            let ac = [cx - ax, cy - ay, cz - az];
            let cross = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            let area = 0.5 * (cross[0].powi(2) + cross[1].powi(2) + cross[2].powi(2)).sqrt();
            total += area;
        }
    }
    total
}

fn expected_surface_area_voxels(sx: usize, sy: usize, sz: usize, solid: &dyn Fn(usize, usize, usize) -> bool) -> f32 {
    let mut area = 0usize;
    let mut count_face = |x: i32, y: i32, z: i32, face: (i32, i32, i32)| {
        // Half-open rule: do not count +X/+Y/+Z boundary faces
        let (dx, dy, dz) = face;
        if x < 0 || y < 0 || z < 0 || x >= sx as i32 || y >= sy as i32 || z >= sz as i32 {
            if dx > 0 && x == sx as i32 { return; }
            if dy > 0 && y == sy as i32 { return; }
            if dz > 0 && z == sz as i32 { return; }
        }
        area += 1;
    };
    for z in 0..sz { for y in 0..sy { for x in 0..sx {
        let here = solid(x, y, z);
        if !here { continue; }
        // -X
        if x == 0 { count_face(0, y as i32, z as i32, (-1,0,0)); }
        if x > 0 && !solid(x - 1, y, z) { count_face(x as i32, y as i32, z as i32, (-1,0,0)); }
        // +X
        if x + 1 < sx && !solid(x + 1, y, z) { count_face((x + 1) as i32, y as i32, z as i32, (1,0,0)); }
        if x + 1 == sx && !false { /* skip +X boundary by rule */ }
        // -Y
        if y == 0 { count_face(x as i32, 0, z as i32, (0,-1,0)); }
        if y > 0 && !solid(x, y - 1, z) { count_face(x as i32, y as i32, z as i32, (0,-1,0)); }
        // +Y
        if y + 1 < sy && !solid(x, y + 1, z) { count_face(x as i32, (y + 1) as i32, z as i32, (0,1,0)); }
        // -Z
        if z == 0 { count_face(x as i32, y as i32, 0, (0,0,-1)); }
        if z > 0 && !solid(x, y, z - 1) { count_face(x as i32, y as i32, z as i32, (0,0,-1)); }
        // +Z
        if z + 1 < sz && !solid(x, y, z + 1) { count_face(x as i32, y as i32, (z + 1) as i32, (0,0,1)); }
    }}}
    area as f32
}

#[test]
fn parity_area_random_full_cubes_s1() {
    let sx = 8; let sy = 8; let sz = 8;
    let reg = load_registry();
    let stone = reg.id_by_name("stone").unwrap_or(1);
    let air = reg.id_by_name("air").unwrap_or(0);
    // Random but deterministic pattern
    let mut blocks: Vec<Block> = Vec::with_capacity(sx * sy * sz);
    for i in 0..(sx * sy * sz) {
        let r = (i as u64 * 1664525 + 1013904223) & 0xFFFF_FFFF;
        let solid = (r & 1) == 0;
        let id = if solid { stone } else { air };
        blocks.push(Block { id, state: 0 });
    }
    let buf = make_buf(0, 0, sx, sy, sz, blocks.clone());
    let store = LightingStore::new(sx, sy, sz);
    let light = LightGrid::compute_with_borders_buf(&buf, &store, &reg);
    // Minimal world; neighbors disabled so seam logic does not run
    let world = World::new(1, 1, sx, sy, sz, 0, WorldGenMode::Flat { thickness: 0 });
    let mut wm = WccMesher::new(&buf, &light, &reg, 1, 0, 0, &world, None, NeighborsLoaded { neg_x: false, pos_x: false, neg_z: false, pos_z: false });
    for z in 0..sz { for y in 0..sy { for x in 0..sx {
        let b = blocks[(y * sz + z) * sx + x];
        if reg.get(b.id).map(|t| t.is_solid(b.state)).unwrap_or(false) {
            wm.add_cube(x, y, z, b);
        }
    }}}
    let mut builds: HashMap<_, _> = HashMap::new();
    wm.emit_into(&mut builds);
    let cpu = ChunkMeshCPU { cx: 0, cz: 0, bbox: geist_geom::Aabb { min: geist_geom::Vec3 { x: 0.0, y: 0.0, z: 0.0 }, max: geist_geom::Vec3 { x: sx as f32, y: sy as f32, z: sz as f32 } }, parts: builds };
    let area_mesh = tri_area_sum(&cpu);
    let solid_fn = |x: usize, y: usize, z: usize| blocks[(y * sz + z) * sx + x].id == stone;
    let area_expected = expected_surface_area_voxels(sx, sy, sz, &solid_fn);
    let diff = (area_mesh - area_expected).abs();
    assert!(diff < 1e-3, "area mismatch: mesh={} expected={} diff={}", area_mesh, area_expected, diff);
}

#[test]
fn seam_stitch_no_faces_on_shared_plane_s1() {
    let sx = 8; let sy = 8; let sz = 8;
    let reg = load_registry();
    let stone = reg.id_by_name("stone").unwrap_or(1);
    let air = reg.id_by_name("air").unwrap_or(0);
    // Deterministic random solids
    let mut blocks_a: Vec<Block> = Vec::with_capacity(sx * sy * sz);
    for i in 0..(sx * sy * sz) { let r = (i as u64 * 1103515245 + 12345) & 0xFFFF_FFFF; let id = if (r & 1)==0 { stone } else { air }; blocks_a.push(Block { id, state: 0 }); }
    let blocks_b = blocks_a.clone();

    let buf_a = make_buf(0, 0, sx, sy, sz, blocks_a);
    let buf_b = make_buf(1, 0, sx, sy, sz, blocks_b);
    let store = LightingStore::new(sx, sy, sz);
    let light_a = LightGrid::compute_with_borders_buf(&buf_a, &store, &reg);
    let light_b = LightGrid::compute_with_borders_buf(&buf_b, &store, &reg);
    let world = World::new(2, 1, sx, sy, sz, 0, WorldGenMode::Flat { thickness: 0 });
    // Indicate neighbor presence along X so stitch logic is active
    let mut wa = WccMesher::new(&buf_a, &light_a, &reg, 1, 0, 0, &world, None, NeighborsLoaded { neg_x: false, pos_x: true, neg_z: false, pos_z: false });
    let mut wb = WccMesher::new(&buf_b, &light_b, &reg, 1, sx as i32, 0, &world, None, NeighborsLoaded { neg_x: true, pos_x: false, neg_z: false, pos_z: false });
    for z in 0..sz { for y in 0..sy { for x in 0..sx {
        let a = buf_a.get_local(x, y, z);
        if reg.get(a.id).map(|t| t.is_solid(a.state)).unwrap_or(false) { wa.add_cube(x, y, z, a); }
        let b = buf_b.get_local(x, y, z);
        if reg.get(b.id).map(|t| t.is_solid(b.state)).unwrap_or(false) { wb.add_cube(x, y, z, b); }
    }}}
    let mut pa = HashMap::new(); wa.emit_into(&mut pa);
    let mut pb = HashMap::new(); wb.emit_into(&mut pb);
    let cpu_a = ChunkMeshCPU { cx: 0, cz: 0, bbox: geist_geom::Aabb { min: geist_geom::Vec3 { x: 0.0, y: 0.0, z: 0.0 }, max: geist_geom::Vec3 { x: sx as f32, y: sy as f32, z: sz as f32 } }, parts: pa };
    let cpu_b = ChunkMeshCPU { cx: 1, cz: 0, bbox: geist_geom::Aabb { min: geist_geom::Vec3 { x: sx as f32, y: 0.0, z: 0.0 }, max: geist_geom::Vec3 { x: 2.0 * sx as f32, y: sy as f32, z: sz as f32 } }, parts: pb };
    let seam_x = sx as f32;
    let eps = 1e-6f32;
    let mut seam_tris = 0usize;
    for cpu in [&cpu_a, &cpu_b] {
        for part in cpu.parts.values() {
            let idx = if !part.idx.is_empty() { part.idx.iter().map(|&i| i as usize).collect::<Vec<_>>() } else { (0..(part.pos.len() / 3)).collect::<Vec<_>>() };
            for t in (0..idx.len()).step_by(3) {
                let a = idx[t] * 3; let b = idx[t + 1] * 3; let c = idx[t + 2] * 3;
                let xs = [part.pos[a], part.pos[b], part.pos[c]];
                if (xs[0] - seam_x).abs() < eps && (xs[1] - seam_x).abs() < eps && (xs[2] - seam_x).abs() < eps {
                    seam_tris += 1;
                }
            }
        }
    }
    assert_eq!(seam_tris, 0, "expected no triangles exactly on shared seam plane, found {}", seam_tris);
}

#[test]
fn merge_reduces_triangles_on_slab() {
    let sx = 12; let sy = 6; let sz = 12;
    let reg = load_registry();
    let stone = reg.id_by_name("stone").unwrap_or(1);
    let air = reg.id_by_name("air").unwrap_or(0);
    // Thick slab: y in [2,4)
    let mut blocks: Vec<Block> = Vec::with_capacity(sx * sy * sz);
    for y in 0..sy { for z in 0..sz { for x in 0..sx {
        let id = if y >= 2 && y < 4 { stone } else { air };
        blocks.push(Block { id, state: 0 });
    }}}
    let buf = make_buf(0, 0, sx, sy, sz, blocks.clone());
    let store = LightingStore::new(sx, sy, sz);
    let light = LightGrid::compute_with_borders_buf(&buf, &store, &reg);
    let world = World::new(1, 1, sx, sy, sz, 0, WorldGenMode::Flat { thickness: 0 });
    let mut wm = WccMesher::new(&buf, &light, &reg, 1, 0, 0, &world, None, NeighborsLoaded { neg_x: false, pos_x: false, neg_z: false, pos_z: false });
    for z in 0..sz { for y in 0..sy { for x in 0..sx {
        let b = blocks[(y * sz + z) * sx + x];
        if reg.get(b.id).map(|t| t.is_solid(b.state)).unwrap_or(false) { wm.add_cube(x, y, z, b); }
    }}}
    let mut parts = HashMap::new();
    wm.emit_into(&mut parts);
    let cpu = ChunkMeshCPU { cx: 0, cz: 0, bbox: geist_geom::Aabb { min: geist_geom::Vec3 { x: 0.0, y: 0.0, z: 0.0 }, max: geist_geom::Vec3 { x: sx as f32, y: sy as f32, z: sz as f32 } }, parts };
    let tris = cpu.parts.values().map(|p| p.idx.len() / 3).sum::<usize>();
    let naive_top = sx * sz * 2; // two triangles per top face cell
    let naive_bottom = sx * sz * 2;
    let naive_sides = 2 * (sx * 2 + sz * 2) * 2; // over-count OK
    let naive_total = naive_top + naive_bottom + naive_sides;
    assert!(tris < naive_total, "expected fewer triangles than naive cover: tris={} naive={}", tris, naive_total);
}

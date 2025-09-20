use hashbrown::HashMap;

use geist_blocks::BlockRegistry;
use geist_blocks::types::Block;
use geist_chunk::ChunkBuf;
use geist_lighting::{LightGrid, LightingStore};
use geist_mesh_cpu::{ChunkMeshCPU, ParityMesher, build_chunk_wcc_cpu_buf_with_light};
use geist_world::{ChunkCoord, World, WorldGenMode};

fn load_registry() -> BlockRegistry {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let vox = root.join("../../assets/voxels");
    BlockRegistry::load_from_paths(vox.join("materials.toml"), vox.join("blocks.toml")).unwrap()
}

fn make_buf_at(
    cx: i32,
    cy: i32,
    cz: i32,
    sx: usize,
    sy: usize,
    sz: usize,
    blocks: Vec<Block>,
) -> ChunkBuf {
    ChunkBuf::from_blocks_local(ChunkCoord::new(cx, cy, cz), sx, sy, sz, blocks)
}

fn make_buf(cx: i32, cz: i32, sx: usize, sy: usize, sz: usize, blocks: Vec<Block>) -> ChunkBuf {
    make_buf_at(cx, 0, cz, sx, sy, sz, blocks)
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
            let ia = idx[t] * 3;
            let ib = idx[t + 1] * 3;
            let ic = idx[t + 2] * 3;
            let ax = pos[ia];
            let ay = pos[ia + 1];
            let az = pos[ia + 2];
            let bx = pos[ib];
            let by = pos[ib + 1];
            let bz = pos[ib + 2];
            let cx = pos[ic];
            let cy = pos[ic + 1];
            let cz = pos[ic + 2];
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

fn expected_surface_area_voxels(
    base_x: i32,
    base_y: i32,
    base_z: i32,
    sx: usize,
    sy: usize,
    sz: usize,
    solid: &dyn Fn(usize, usize, usize) -> bool,
    solid_world: &dyn Fn(i32, i32, i32) -> bool,
) -> f32 {
    let mut area = 0usize;
    let mut count_face = |_x: i32, _y: i32, _z: i32, _face: (i32, i32, i32)| {
        area += 1;
    };
    for z in 0..sz {
        for y in 0..sy {
            for x in 0..sx {
                let here = solid(x, y, z);
                if !here {
                    continue;
                }
                let wx = base_x + x as i32;
                let wy = base_y + y as i32;
                let wz = base_z + z as i32;
                // -X
                if x == 0 {
                    count_face(0, y as i32, z as i32, (-1, 0, 0));
                }
                if x > 0 && !solid(x - 1, y, z) {
                    count_face(x as i32, y as i32, z as i32, (-1, 0, 0));
                }
                // +X
                if x + 1 < sx {
                    if !solid(x + 1, y, z) {
                        count_face((x + 1) as i32, y as i32, z as i32, (1, 0, 0));
                    }
                } else if !solid_world(wx + 1, wy, wz) {
                    count_face((x + 1) as i32, y as i32, z as i32, (1, 0, 0));
                }
                // -Y
                if y == 0 {
                    count_face(x as i32, 0, z as i32, (0, -1, 0));
                }
                if y > 0 && !solid(x, y - 1, z) {
                    count_face(x as i32, y as i32, z as i32, (0, -1, 0));
                }
                // +Y
                if y + 1 < sy {
                    if !solid(x, y + 1, z) {
                        count_face(x as i32, (y + 1) as i32, z as i32, (0, 1, 0));
                    }
                } else if !solid_world(wx, wy + 1, wz) {
                    count_face(x as i32, (y + 1) as i32, z as i32, (0, 1, 0));
                }
                // -Z
                if z == 0 {
                    count_face(x as i32, y as i32, 0, (0, 0, -1));
                }
                if z > 0 && !solid(x, y, z - 1) {
                    count_face(x as i32, y as i32, z as i32, (0, 0, -1));
                }
                // +Z
                if z + 1 < sz {
                    if !solid(x, y, z + 1) {
                        count_face(x as i32, y as i32, (z + 1) as i32, (0, 0, 1));
                    }
                } else if !solid_world(wx, wy, wz + 1) {
                    count_face(x as i32, y as i32, (z + 1) as i32, (0, 0, 1));
                }
            }
        }
    }
    area as f32
}

#[test]
fn parity_area_random_full_cubes_s1() {
    let sx = 8;
    let sy = 8;
    let sz = 8;
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
    let _light = LightGrid::compute_with_borders_buf(&buf, &store, &reg);
    // Minimal world; neighbors disabled so seam logic does not run
    let world = World::new(1, 1, 1, 0, WorldGenMode::Flat { thickness: 0 });
    let base_x = buf.coord.cx * buf.sx as i32;
    let base_y = buf.coord.cy * buf.sy as i32;
    let base_z = buf.coord.cz * buf.sz as i32;
    let mut wm = ParityMesher::new(&buf, &reg, 1, base_x, base_y, base_z, &world, None);
    wm.build_occupancy();
    wm.seed_seam_layers();
    wm.compute_parity_and_materials();
    let mut builds: HashMap<_, _> = HashMap::new();
    wm.emit_into(&mut builds);
    let cpu = ChunkMeshCPU {
        coord: ChunkCoord::new(0, 0, 0),
        bbox: geist_geom::Aabb {
            min: geist_geom::Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            max: geist_geom::Vec3 {
                x: sx as f32,
                y: sy as f32,
                z: sz as f32,
            },
        },
        parts: builds,
    };
    let area_mesh = tri_area_sum(&cpu);
    let solid_fn = |x: usize, y: usize, z: usize| blocks[(y * sz + z) * sx + x].id == stone;
    let area_expected = expected_surface_area_voxels(
        base_x,
        base_y,
        base_z,
        sx,
        sy,
        sz,
        &solid_fn,
        &|wx, wy, wz| {
            let b = world.block_at_runtime(&reg, wx, wy, wz);
            b.id != air
        },
    );
    let diff = (area_mesh - area_expected).abs();
    assert!(
        diff < 1e-3,
        "area mismatch: mesh={} expected={} diff={}",
        area_mesh,
        area_expected,
        diff
    );
}

#[test]
fn seam_stitch_no_faces_on_shared_plane_s1() {
    let sx = 8;
    let sy = 8;
    let sz = 8;
    let reg = load_registry();
    let stone = reg.id_by_name("stone").unwrap_or(1);
    let air = reg.id_by_name("air").unwrap_or(0);
    // Deterministic random solids
    let mut blocks_a: Vec<Block> = Vec::with_capacity(sx * sy * sz);
    for i in 0..(sx * sy * sz) {
        let r = (i as u64 * 1103515245 + 12345) & 0xFFFF_FFFF;
        let id = if (r & 1) == 0 { stone } else { air };
        blocks_a.push(Block { id, state: 0 });
    }
    let mut blocks_b = blocks_a.clone();
    for y in 0..sy {
        for z in 0..sz {
            let idx_a = (y * sz + z) * sx + (sx - 1);
            let idx_b = (y * sz + z) * sx;
            blocks_b[idx_b] = blocks_a[idx_a];
        }
    }

    let buf_a = make_buf(0, 0, sx, sy, sz, blocks_a);
    let buf_b = make_buf(1, 0, sx, sy, sz, blocks_b);
    let store = LightingStore::new(sx, sy, sz);
    let _light_a = LightGrid::compute_with_borders_buf(&buf_a, &store, &reg);
    let _light_b = LightGrid::compute_with_borders_buf(&buf_b, &store, &reg);
    let world = World::new(2, 1, 1, 0, WorldGenMode::Flat { thickness: 0 });
    // Indicate neighbor presence along X so stitch logic is active
    let base_wa_x = buf_a.coord.cx * buf_a.sx as i32;
    let base_wa_y = buf_a.coord.cy * buf_a.sy as i32;
    let base_wa_z = buf_a.coord.cz * buf_a.sz as i32;
    let base_wb_x = buf_b.coord.cx * buf_b.sx as i32;
    let base_wb_y = buf_b.coord.cy * buf_b.sy as i32;
    let base_wb_z = buf_b.coord.cz * buf_b.sz as i32;
    let mut edits_a: HashMap<(i32, i32, i32), Block> = HashMap::new();
    for y in 0..sy {
        for z in 0..sz {
            for x in 0..sx {
                let block = buf_b.get_local(x, y, z);
                if block.id == air {
                    continue;
                }
                let wx = base_wb_x + x as i32;
                let wy = base_wb_y + y as i32;
                let wz = base_wb_z + z as i32;
                edits_a.insert((wx, wy, wz), block);
            }
        }
    }
    assert!(
        edits_a.contains_key(&(base_wb_x, base_wb_y, base_wb_z)),
        "expected neighbor seam column to be present in edits"
    );
    let mut edits_b: HashMap<(i32, i32, i32), Block> = HashMap::new();
    for y in 0..sy {
        for z in 0..sz {
            for x in 0..sx {
                let block = buf_a.get_local(x, y, z);
                if block.id == air {
                    continue;
                }
                let wx = base_wa_x + x as i32;
                let wy = base_wa_y + y as i32;
                let wz = base_wa_z + z as i32;
                edits_b.insert((wx, wy, wz), block);
            }
        }
    }
    let mut wa = ParityMesher::new(
        &buf_a,
        &reg,
        1,
        base_wa_x,
        base_wa_y,
        base_wa_z,
        &world,
        Some(&edits_a),
    );
    let mut wb = ParityMesher::new(
        &buf_b,
        &reg,
        1,
        base_wb_x,
        base_wb_y,
        base_wb_z,
        &world,
        Some(&edits_b),
    );
    wa.build_occupancy();
    wb.build_occupancy();
    wa.seed_seam_layers();
    wb.seed_seam_layers();
    wa.compute_parity_and_materials();
    wb.compute_parity_and_materials();
    let mut pa = HashMap::new();
    wa.emit_into(&mut pa);
    let mut pb = HashMap::new();
    wb.emit_into(&mut pb);
    let cpu_a = ChunkMeshCPU {
        coord: ChunkCoord::new(0, 0, 0),
        bbox: geist_geom::Aabb {
            min: geist_geom::Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            max: geist_geom::Vec3 {
                x: sx as f32,
                y: sy as f32,
                z: sz as f32,
            },
        },
        parts: pa,
    };
    let cpu_b = ChunkMeshCPU {
        coord: ChunkCoord::new(1, 0, 0),
        bbox: geist_geom::Aabb {
            min: geist_geom::Vec3 {
                x: sx as f32,
                y: 0.0,
                z: 0.0,
            },
            max: geist_geom::Vec3 {
                x: 2.0 * sx as f32,
                y: sy as f32,
                z: sz as f32,
            },
        },
        parts: pb,
    };
    let seam_x = sx as f32;
    let eps = 1e-6f32;
    let mut seam_tris = 0usize;
    for cpu in [&cpu_a, &cpu_b] {
        for part in cpu.parts.values() {
            let idx = if !part.idx.is_empty() {
                part.idx.iter().map(|&i| i as usize).collect::<Vec<_>>()
            } else {
                (0..(part.pos.len() / 3)).collect::<Vec<_>>()
            };
            for t in (0..idx.len()).step_by(3) {
                let a = idx[t] * 3;
                let b = idx[t + 1] * 3;
                let c = idx[t + 2] * 3;
                let xs = [part.pos[a], part.pos[b], part.pos[c]];
                if (xs[0] - seam_x).abs() < eps
                    && (xs[1] - seam_x).abs() < eps
                    && (xs[2] - seam_x).abs() < eps
                {
                    seam_tris += 1;
                }
            }
        }
    }
    assert_eq!(
        seam_tris, 0,
        "expected no triangles exactly on shared seam plane, found {}",
        seam_tris
    );
}

#[test]
fn seam_vertical_no_faces_on_shared_plane_s1() {
    let sx = 8;
    let sy = 8;
    let sz = 8;
    let reg = load_registry();
    let stone = reg.id_by_name("stone").unwrap_or(1);
    let air = reg.id_by_name("air").unwrap_or(0);
    // Deterministic random solids
    let mut blocks_lo: Vec<Block> = Vec::with_capacity(sx * sy * sz);
    for i in 0..(sx * sy * sz) {
        let r = (i as u64 * 1664525 + 1013904223) & 0xFFFF_FFFF;
        let id = if (r & 1) == 0 { stone } else { air };
        blocks_lo.push(Block { id, state: 0 });
    }
    let blocks_hi = blocks_lo.clone();

    let buf_lo = make_buf_at(0, 0, 0, sx, sy, sz, blocks_lo);
    let buf_hi = make_buf_at(0, 1, 0, sx, sy, sz, blocks_hi);
    let store = LightingStore::new(sx, sy, sz);
    let _light_lo = LightGrid::compute_with_borders_buf(&buf_lo, &store, &reg);
    let _light_hi = LightGrid::compute_with_borders_buf(&buf_hi, &store, &reg);
    let world = World::new(1, 2, 1, 0, WorldGenMode::Flat { thickness: 0 });

    let base_lo_x = buf_lo.coord.cx * buf_lo.sx as i32;
    let base_lo_y = buf_lo.coord.cy * buf_lo.sy as i32;
    let base_lo_z = buf_lo.coord.cz * buf_lo.sz as i32;
    let base_hi_x = buf_hi.coord.cx * buf_hi.sx as i32;
    let base_hi_y = buf_hi.coord.cy * buf_hi.sy as i32;
    let base_hi_z = buf_hi.coord.cz * buf_hi.sz as i32;

    let mut edits_lo: HashMap<(i32, i32, i32), Block> = HashMap::new();
    for y in 0..sy {
        for z in 0..sz {
            for x in 0..sx {
                let block = buf_hi.get_local(x, y, z);
                if block.id == air {
                    continue;
                }
                let wx = base_hi_x + x as i32;
                let wy = base_hi_y + y as i32;
                let wz = base_hi_z + z as i32;
                edits_lo.insert((wx, wy, wz), block);
            }
        }
    }
    let mut edits_hi: HashMap<(i32, i32, i32), Block> = HashMap::new();
    for y in 0..sy {
        for z in 0..sz {
            for x in 0..sx {
                let block = buf_lo.get_local(x, y, z);
                if block.id == air {
                    continue;
                }
                let wx = base_lo_x + x as i32;
                let wy = base_lo_y + y as i32;
                let wz = base_lo_z + z as i32;
                edits_hi.insert((wx, wy, wz), block);
            }
        }
    }

    let mut lo = ParityMesher::new(
        &buf_lo,
        &reg,
        1,
        base_lo_x,
        base_lo_y,
        base_lo_z,
        &world,
        Some(&edits_lo),
    );
    let mut hi = ParityMesher::new(
        &buf_hi,
        &reg,
        1,
        base_hi_x,
        base_hi_y,
        base_hi_z,
        &world,
        Some(&edits_hi),
    );
    lo.build_occupancy();
    hi.build_occupancy();
    lo.seed_seam_layers();
    hi.seed_seam_layers();
    lo.compute_parity_and_materials();
    hi.compute_parity_and_materials();

    let mut builds_lo = HashMap::new();
    lo.emit_into(&mut builds_lo);
    let mut builds_hi = HashMap::new();
    hi.emit_into(&mut builds_hi);

    let cpu_lo = ChunkMeshCPU {
        coord: buf_lo.coord,
        bbox: geist_geom::Aabb {
            min: geist_geom::Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            max: geist_geom::Vec3 {
                x: sx as f32,
                y: sy as f32,
                z: sz as f32,
            },
        },
        parts: builds_lo,
    };
    let cpu_hi = ChunkMeshCPU {
        coord: buf_hi.coord,
        bbox: geist_geom::Aabb {
            min: geist_geom::Vec3 {
                x: 0.0,
                y: sy as f32,
                z: 0.0,
            },
            max: geist_geom::Vec3 {
                x: sx as f32,
                y: (2 * sy) as f32,
                z: sz as f32,
            },
        },
        parts: builds_hi,
    };

    let seam_y = sy as f32;
    let eps = 1e-6f32;
    let mut seam_tris = 0usize;
    for cpu in [&cpu_lo, &cpu_hi] {
        for part in cpu.parts.values() {
            let idx = if !part.idx.is_empty() {
                part.idx.iter().map(|&i| i as usize).collect::<Vec<_>>()
            } else {
                (0..(part.pos.len() / 3)).collect::<Vec<_>>()
            };
            for t in (0..idx.len()).step_by(3) {
                let a = idx[t] * 3;
                let b = idx[t + 1] * 3;
                let c = idx[t + 2] * 3;
                let ys = [part.pos[a + 1], part.pos[b + 1], part.pos[c + 1]];
                if (ys[0] - seam_y).abs() < eps
                    && (ys[1] - seam_y).abs() < eps
                    && (ys[2] - seam_y).abs() < eps
                {
                    seam_tris += 1;
                }
            }
        }
    }
    assert_eq!(
        seam_tris, 0,
        "expected no triangles exactly on shared vertical seam plane, found {}",
        seam_tris
    );
}

#[test]
fn boundary_pos_x_faces_exist_when_neighbor_air_s1() {
    let sx = 4;
    let sy = 4;
    let sz = 4;
    let reg = load_registry();
    let stone = reg.id_by_name("stone").unwrap_or(1);
    let air = reg.id_by_name("air").unwrap_or(0);

    let mut blocks = vec![Block { id: air, state: 0 }; sx * sy * sz];
    for y in 0..sy {
        for z in 0..sz {
            let idx = (y * sz + z) * sx + (sx - 1);
            blocks[idx] = Block {
                id: stone,
                state: 0,
            };
        }
    }

    let buf = make_buf(0, 0, sx, sy, sz, blocks);
    let store = LightingStore::new(sx, sy, sz);
    let light = LightGrid::compute_with_borders_buf(&buf, &store, &reg);
    let world = World::new(1, 1, 1, 0, WorldGenMode::Flat { thickness: 0 });

    let (mesh, _) = build_chunk_wcc_cpu_buf_with_light(&buf, &light, &world, None, buf.coord, &reg)
        .expect("chunk mesh");
    let seam_x = sx as f32;
    let eps = 1e-6f32;
    let mut seam_tris = 0usize;
    for part in mesh.parts.values() {
        let idx = if !part.idx.is_empty() {
            part.idx.iter().map(|&i| i as usize).collect::<Vec<_>>()
        } else {
            (0..(part.pos.len() / 3)).collect::<Vec<_>>()
        };
        let pos = &part.pos;
        for t in (0..idx.len()).step_by(3) {
            let a = idx[t] * 3;
            let b = idx[t + 1] * 3;
            let c = idx[t + 2] * 3;
            if (pos[a] - seam_x).abs() < eps
                && (pos[b] - seam_x).abs() < eps
                && (pos[c] - seam_x).abs() < eps
            {
                seam_tris += 1;
            }
        }
    }

    assert!(
        seam_tris > 0,
        "expected exposed +X faces when neighbor is air, found none"
    );
}

#[test]
fn per_face_quads_triangle_count_on_slab() {
    let sx = 12;
    let sy = 6;
    let sz = 12;
    let reg = load_registry();
    let stone = reg.id_by_name("stone").unwrap_or(1);
    let air = reg.id_by_name("air").unwrap_or(0);
    // Thick slab: y in [2,4)
    let mut blocks: Vec<Block> = Vec::with_capacity(sx * sy * sz);
    for y in 0..sy {
        for _z in 0..sz {
            for _x in 0..sx {
                let id = if y >= 2 && y < 4 { stone } else { air };
                blocks.push(Block { id, state: 0 });
            }
        }
    }
    let buf = make_buf(0, 0, sx, sy, sz, blocks.clone());
    let store = LightingStore::new(sx, sy, sz);
    let light = LightGrid::compute_with_borders_buf(&buf, &store, &reg);
    let world = World::new(1, 1, 1, 0, WorldGenMode::Flat { thickness: 0 });
    let (cpu, _) = build_chunk_wcc_cpu_buf_with_light(&buf, &light, &world, None, buf.coord, &reg)
        .expect("mesh generation");
    // Compute expected surface area and compare against mesh area to ensure all faces emitted.
    let blocks_clone = blocks.clone();
    let solid_fn = |x: usize, y: usize, z: usize| blocks_clone[(y * sz + z) * sx + x].id == stone;
    let base_x = buf.coord.cx * buf.sx as i32;
    let base_y = buf.coord.cy * buf.sy as i32;
    let base_z = buf.coord.cz * buf.sz as i32;
    let expected_area = expected_surface_area_voxels(
        base_x,
        base_y,
        base_z,
        sx,
        sy,
        sz,
        &solid_fn,
        &|wx, wy, wz| {
            let b = world.block_at_runtime(&reg, wx, wy, wz);
            b.id != air
        },
    );
    let area_mesh = tri_area_sum(&cpu);
    let diff = (area_mesh - expected_area).abs();
    assert!(
        diff < 1e-3,
        "surface area mismatch: mesh={} expected={} diff={}",
        area_mesh,
        expected_area,
        diff
    );
}

#![cfg(test)]

use crate::blocks::{types::Block, BlockRegistry};
use crate::chunkbuf::ChunkBuf;
use crate::lighting::LightingStore;
use crate::mesher::{build_chunk_greedy_cpu_buf, NeighborsLoaded};
use crate::voxel::{build_showcase_stairs_cluster, World, WorldGenMode};
use raylib::prelude::Vector3;

fn reg() -> BlockRegistry {
    BlockRegistry::load_from_paths(
        "assets/voxels/materials.toml",
        "assets/voxels/blocks.toml",
    )
    .expect("load registry")
}

fn count_quads_in_box_with_normal(
    parts: &std::collections::HashMap<crate::blocks::types::MaterialId, crate::mesher::MeshBuild>,
    min: Vector3,
    max: Vector3,
    normal: Vector3,
) -> usize {
    let mut total = 0usize;
    let eps = 1e-4f32;
    let n_target = normal;
    for (_mid, mb) in parts.iter() {
        let pos = mb.positions();
        let nor = mb.normals();
        let verts = pos.len() / 3;
        if verts == 0 { continue; }
        let quads = verts / 4;
        for i in 0..quads {
            let pbase = i * 12; // 4 verts * 3 comps
            let nbase = i * 12;
            let nx = nor[nbase + 0];
            let ny = nor[nbase + 1];
            let nz = nor[nbase + 2];
            // normals are axis-aligned; require exact match with tolerance
            if (nx - n_target.x).abs() > 1e-5 || (ny - n_target.y).abs() > 1e-5 || (nz - n_target.z).abs() > 1e-5 {
                continue;
            }
            let mut inside = true;
            for v in 0..4 {
                let x = pos[pbase + v * 3 + 0];
                let y = pos[pbase + v * 3 + 1];
                let z = pos[pbase + v * 3 + 2];
                if x < min.x - eps || x > max.x + eps || y < min.y - eps || y > max.y + eps || z < min.z - eps || z > max.z + eps {
                    inside = false;
                    break;
                }
            }
            if inside { total += 1; }
        }
    }
    total
}

fn count_up_quads_at_y(
    parts: &std::collections::HashMap<crate::blocks::types::MaterialId, crate::mesher::MeshBuild>,
    min: Vector3,
    max: Vector3,
    y_level: f32,
) -> usize {
    let mut total = 0usize;
    let eps = 1e-4f32;
    for (_mid, mb) in parts.iter() {
        let pos = mb.positions();
        let nor = mb.normals();
        let verts = pos.len() / 3;
        if verts == 0 { continue; }
        let quads = verts / 4;
        for i in 0..quads {
            let pbase = i * 12; // 4 verts * 3 comps
            let nbase = i * 12;
            let nx = nor[nbase + 0];
            let ny = nor[nbase + 1];
            let nz = nor[nbase + 2];
            if (nx).abs() > 1e-5 || (ny - 1.0).abs() > 1e-5 || (nz).abs() > 1e-5 {
                continue; // only +Y faces
            }
            let mut inside = true;
            for v in 0..4 {
                let x = pos[pbase + v * 3 + 0];
                let y = pos[pbase + v * 3 + 1];
                let z = pos[pbase + v * 3 + 2];
                if x < min.x - eps || x > max.x + eps || z < min.z - eps || z > max.z + eps {
                    inside = false; break;
                }
                if (y - y_level).abs() > 1e-4 { inside = false; break; }
            }
            if inside { total += 1; }
        }
    }
    total
}

fn build_chunk_with_stairs_cluster(
    reg: &BlockRegistry,
) -> (ChunkBuf, World, i32, i32, i32, Vec<(i32, i32, Block)>) {
    let placements = build_showcase_stairs_cluster(reg);
    assert!(!placements.is_empty(), "stairs cluster not built");
    let max_dx = placements.iter().map(|p| p.dx).max().unwrap_or(0);
    let max_dz = placements.iter().map(|p| p.dz).max().unwrap_or(0);
    let sx = (max_dx + 8).max(16) as usize; // margin
    let sz = (max_dz + 8).max(16) as usize;
    let sy = 32usize;
    let y = 10i32;
    let start_x = 2i32;
    let start_z = 2i32;

    let mut blocks = vec![Block::AIR; sx * sy * sz];
    let mut world_positions: Vec<(i32, i32, Block)> = Vec::new();
    for p in placements {
        let wx = start_x + p.dx;
        let wz = start_z + p.dz;
        let wy = y;
        world_positions.push((wx, wz, Block { id: p.block.id, state: p.block.state }));
        let idx = ((wy as usize) * sz + (wz as usize)) * sx + (wx as usize);
        blocks[idx] = Block { id: p.block.id, state: p.block.state };
    }

    let buf = ChunkBuf::from_blocks_local(0, 0, sx, sy, sz, blocks);
    let world = World::new(1, 1, sx, sy, sz, 1337, WorldGenMode::Flat { thickness: 0 });
    (buf, world, start_x, start_z, y, world_positions)
}

#[test]
fn stairs_cluster_faces_present() {
    let reg = reg();
    let (buf, world, start_x, start_z, y, positions) = build_chunk_with_stairs_cluster(&reg);
    let store = LightingStore::new(buf.sx, buf.sy, buf.sz);
    let neighbors = NeighborsLoaded::default();
    let (cpu, _lb) = build_chunk_greedy_cpu_buf(&buf, Some(&store), &world, None, neighbors, 0, 0, &reg)
        .expect("mesh");

    // For each placed stair: must have some +Y quads in its box (visible treads), and some side faces.
    for (wx, wz, b) in positions.iter().copied() {
        let ty = reg.get(b.id).expect("block type");
        assert_eq!(ty.name, "stairs");
        let min = Vector3::new(wx as f32, y as f32, wz as f32);
        let max = Vector3::new(wx as f32 + 1.0, y as f32 + 1.0, wz as f32 + 1.0);
        let up = count_quads_in_box_with_normal(&cpu.parts, min, max, Vector3::new(0.0, 1.0, 0.0));
        let sxp = count_quads_in_box_with_normal(&cpu.parts, min, max, Vector3::new(1.0, 0.0, 0.0));
        let sxn = count_quads_in_box_with_normal(&cpu.parts, min, max, Vector3::new(-1.0, 0.0, 0.0));
        let szp = count_quads_in_box_with_normal(&cpu.parts, min, max, Vector3::new(0.0, 0.0, 1.0));
        let szn = count_quads_in_box_with_normal(&cpu.parts, min, max, Vector3::new(0.0, 0.0, -1.0));
        let sides = sxp + sxn + szp + szn;
        assert!(up >= 2, "stair at ({},{}) expected at least 2 +Y faces, got {}", wx, wz, up);
        // Expect a +Y face at both y+0.5 and y+1.0 levels
        let up_05 = count_up_quads_at_y(&cpu.parts, min, max, y as f32 + 0.5);
        let up_10 = count_up_quads_at_y(&cpu.parts, min, max, y as f32 + 1.0);
        assert!(up_05 > 0, "stair at ({},{}) missing +Y face at y+0.5", wx, wz);
        assert!(up_10 > 0, "stair at ({},{}) missing +Y face at y+1.0", wx, wz);
        assert!(sides > 0, "stair at ({},{}) missing side faces", wx, wz);
    }
}

#[test]
fn stair_singles_have_two_up_faces() {
    let reg = reg();
    let (buf, world, _start_x, _start_z, y, positions) = build_chunk_with_stairs_cluster(&reg);
    let store = LightingStore::new(buf.sx, buf.sy, buf.sz);
    let neighbors = NeighborsLoaded::default();
    let (cpu, _lb) = build_chunk_greedy_cpu_buf(&buf, Some(&store), &world, None, neighbors, 0, 0, &reg)
        .expect("mesh");
    // First four placements are singles N,E,S,W at dz=0
    for (i, (wx, wz, _b)) in positions.iter().copied().enumerate() {
        if i >= 4 { break; }
        let min = Vector3::new(wx as f32, y as f32, wz as f32);
        let max = Vector3::new(wx as f32 + 1.0, y as f32 + 1.0, wz as f32 + 1.0);
        let up = count_quads_in_box_with_normal(&cpu.parts, min, max, Vector3::new(0.0, 1.0, 0.0));
        assert!(up >= 2, "single stair at ({},{}) expected at least 2 +Y faces, got {}", wx, wz, up);
    }
}

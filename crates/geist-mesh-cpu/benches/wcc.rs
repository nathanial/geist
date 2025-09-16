use std::collections::HashMap;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use std::time::Duration;

use geist_blocks::registry::BlockRegistry;
use geist_blocks::types::Block;
use geist_chunk::{ChunkBuf, generate_chunk_buffer};
use geist_lighting::{LightGrid, LightingStore};
use geist_mesh_cpu::{ParityMesher, build_chunk_wcc_cpu_buf};
use geist_world::{ChunkCoord, World, WorldGenMode};

fn load_registry() -> BlockRegistry {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let vox = root.join("../../assets/voxels");
    BlockRegistry::load_from_paths(vox.join("materials.toml"), vox.join("blocks.toml")).unwrap()
}

fn bench_build_chunk_wcc_flat(c: &mut Criterion) {
    let mut group = c.benchmark_group("build_chunk_wcc_flat");
    let reg = load_registry();
    let chunks_y = 2;
    let world = World::new(
        1,
        chunks_y,
        1,
        0xC0FFEE as i32,
        WorldGenMode::Flat { thickness: 32 },
    );
    let (sx, sy, sz) = (world.chunk_size_x, world.chunk_size_y, world.chunk_size_z);
    let store = LightingStore::new(sx, sy, sz);
    group.bench_function("flat_32x64x32", |b| {
        b.iter(|| {
            let buf = generate_chunk_buffer(&world, ChunkCoord::new(0, 0, 0), &reg);
            let out = build_chunk_wcc_cpu_buf(
                &buf,
                Some(&store),
                &world,
                None,
                ChunkCoord::new(0, 0, 0),
                &reg,
            );
            black_box(out);
        })
    });
    group.finish();
}

fn bench_build_chunk_wcc_normal_dims(c: &mut Criterion) {
    let mut group = c.benchmark_group("build_chunk_wcc_normal_dims");
    let reg = load_registry();
    // Match normal worldgen defaults: 32 x 256 x 32
    let world = World::new(1, 8, 1, 1337, WorldGenMode::Normal);
    let (sx, sy, sz) = (world.chunk_size_x, world.chunk_size_y, world.chunk_size_z);
    let store = LightingStore::new(sx, sy, sz);
    group.bench_function("normal_32x256x32", |b| {
        b.iter(|| {
            let buf = generate_chunk_buffer(&world, ChunkCoord::new(0, 0, 0), &reg);
            let out = build_chunk_wcc_cpu_buf(
                &buf,
                Some(&store),
                &world,
                None,
                ChunkCoord::new(0, 0, 0),
                &reg,
            );
            black_box(out);
        })
    });
    group.finish();
}

fn bench_worldgen_normal_dims(c: &mut Criterion) {
    let mut group = c.benchmark_group("worldgen_normal_dims");
    let reg = load_registry();
    let world = World::new(1, 8, 1, 1337, WorldGenMode::Normal);
    group.bench_function("generate_chunk_buffer_32x256x32", |b| {
        b.iter(|| {
            let buf = generate_chunk_buffer(&world, ChunkCoord::new(0, 0, 0), &reg);
            black_box(buf);
        })
    });
    group.finish();
}

fn bench_light_compute_normal_dims(c: &mut Criterion) {
    let mut group = c.benchmark_group("lighting_normal_dims");
    let reg = load_registry();
    let world = World::new(1, 8, 1, 1337, WorldGenMode::Normal);
    let (sx, sy, sz) = (world.chunk_size_x, world.chunk_size_y, world.chunk_size_z);
    let store = LightingStore::new(sx, sy, sz);
    let buf = generate_chunk_buffer(&world, ChunkCoord::new(0, 0, 0), &reg);
    group.bench_function("compute_light_with_borders_32x256x32", |b| {
        b.iter(|| {
            let lg = LightGrid::compute_with_borders_buf(&buf, &store, &reg);
            black_box(lg);
        })
    });
    group.finish();
}

fn bench_wcc_toggle_emit_normal_dims(c: &mut Criterion) {
    let mut group = c.benchmark_group("wcc_toggle_emit_normal_dims");
    let reg = load_registry();
    let world = World::new(1, 8, 1, 1337, WorldGenMode::Normal);
    let (sx, sy, sz) = (world.chunk_size_x, world.chunk_size_y, world.chunk_size_z);
    let store = LightingStore::new(sx, sy, sz);
    let buf = generate_chunk_buffer(&world, ChunkCoord::new(0, 0, 0), &reg);
    let light = LightGrid::compute_with_borders_buf(&buf, &store, &reg);
    group.bench_function("toggle_emit_S2_no_thin_32x256x32", |b| {
        b.iter(|| {
            let base_x = buf.coord.cx * sx as i32;
            let base_z = buf.coord.cz * sz as i32;
            let mut pm = ParityMesher::new(&buf, &reg, 2, base_x, base_z, &world, None);
            pm.build_occupancy();
            pm.seed_seam_layers();
            pm.compute_parity_and_materials();
            let mut parts: HashMap<_, _> = HashMap::new();
            pm.emit_into(&mut parts);
            black_box(parts);
        })
    });
    group.finish();
}

fn bench_build_chunk_wcc_normal_neighbors(c: &mut Criterion) {
    let mut group = c.benchmark_group("build_chunk_wcc_normal_neighbors");
    let reg = load_registry();
    // Use 2x2 neighbor grid at normal world dims
    let (chunks_x, chunks_z) = (2usize, 2usize);
    let chunks_y = 8usize;
    let world = World::new(chunks_x, chunks_y, chunks_z, 2025, WorldGenMode::Normal);
    let (sx, sy, sz) = (world.chunk_size_x, world.chunk_size_y, world.chunk_size_z);
    let store = LightingStore::new(sx, sy, sz);
    group.bench_function("normal_neighbors_2x2_32x256x32", |b| {
        b.iter(|| {
            let mut total_parts = 0usize;
            for cz in 0..(chunks_z as i32) {
                for cx in 0..(chunks_x as i32) {
                    let buf = generate_chunk_buffer(&world, ChunkCoord::new(cx, 0, cz), &reg);
                    if let Some((cpu, _borders)) = build_chunk_wcc_cpu_buf(
                        &buf,
                        Some(&store),
                        &world,
                        None,
                        ChunkCoord::new(cx, 0, cz),
                        &reg,
                    ) {
                        total_parts += cpu.parts.len();
                    }
                }
            }
            black_box(total_parts);
        })
    });
    group.finish();
}
fn make_uniform_chunk(cx: i32, cz: i32, sx: usize, sy: usize, sz: usize, id: u16) -> ChunkBuf {
    let mut blocks = Vec::with_capacity(sx * sy * sz);
    blocks.resize(sx * sy * sz, Block { id, state: 0 });
    ChunkBuf::from_blocks_local(ChunkCoord::new(cx, 0, cz), sx, sy, sz, blocks)
}

fn bench_wcc_mesher_s1_uniform(c: &mut Criterion) {
    let mut group = c.benchmark_group("wcc_mesher_s1_uniform");
    let reg = load_registry();
    let world = World::new(1, 2, 1, 42, WorldGenMode::Flat { thickness: 0 });
    let (sx, sy, sz) = (world.chunk_size_x, world.chunk_size_y, world.chunk_size_z);
    let stone = reg.id_by_name("stone").unwrap_or(1);
    let buf = make_uniform_chunk(0, 0, sx, sy, sz, stone);
    let store = LightingStore::new(sx, sy, sz);
    let light = LightGrid::compute_with_borders_buf(&buf, &store, &reg);
    group.bench_function("toggle_emit_solid_32x64x32", |b| {
        b.iter(|| {
            let mut pm = ParityMesher::new(&buf, &reg, 1, 0, 0, &world, None);
            pm.build_occupancy();
            pm.seed_seam_layers();
            pm.compute_parity_and_materials();
            let mut parts: HashMap<_, _> = HashMap::new();
            pm.emit_into(&mut parts);
            black_box(parts);
        })
    });
    group.finish();
}

fn bench_wcc_mesher_s2_mixed(c: &mut Criterion) {
    let mut group = c.benchmark_group("wcc_mesher_s2_mixed");
    let reg = load_registry();
    let world = World::new(1, 2, 1, 7, WorldGenMode::Flat { thickness: 0 });
    let (sx, sy, sz) = (world.chunk_size_x, world.chunk_size_y, world.chunk_size_z);
    let stone = reg.id_by_name("stone").unwrap_or(1);
    let slab = reg.id_by_name("slab").unwrap_or(stone);
    let slab_state_bottom = reg
        .get(slab)
        .map(|ty| {
            let mut props = std::collections::HashMap::new();
            props.insert("half".to_string(), "bottom".to_string());
            ty.pack_state(&props)
        })
        .unwrap_or(0);
    // Build a mixed buffer: checkerboard stone/slab-bottom
    let mut blocks = Vec::with_capacity(sx * sy * sz);
    for y in 0..sy {
        for z in 0..sz {
            for x in 0..sx {
                let use_slab = ((x ^ z ^ (y / 8)) & 1) == 0;
                if use_slab {
                    blocks.push(Block {
                        id: slab,
                        state: slab_state_bottom,
                    });
                } else {
                    blocks.push(Block {
                        id: stone,
                        state: 0,
                    });
                }
            }
        }
    }
    let buf = ChunkBuf::from_blocks_local(ChunkCoord::new(0, 0, 0), sx, sy, sz, blocks);
    let store = LightingStore::new(sx, sy, sz);
    let light = LightGrid::compute_with_borders_buf(&buf, &store, &reg);
    group.bench_function("toggle_emit_mixed_s2_32x64x32", |b| {
        b.iter(|| {
            let mut pm = ParityMesher::new(&buf, &reg, 2, 0, 0, &world, None);
            pm.build_occupancy();
            pm.seed_seam_layers();
            pm.compute_parity_and_materials();
            let mut parts: HashMap<_, _> = HashMap::new();
            pm.emit_into(&mut parts);
            black_box(parts);
        })
    });
    group.finish();
}

fn long_config() -> Criterion {
    Criterion::default()
        .measurement_time(Duration::from_secs(30))
        .warm_up_time(Duration::from_secs(10))
        .sample_size(10)
}

fn heavy_config() -> Criterion {
    // Longer budget and fewer samples for heavy multi-chunk runs
    Criterion::default()
        .measurement_time(Duration::from_secs(180))
        .warm_up_time(Duration::from_secs(20))
        .sample_size(10)
        .confidence_level(0.90)
}

criterion_group! {
    name = benches;
    config = long_config();
    targets =
        bench_build_chunk_wcc_flat,
        bench_build_chunk_wcc_normal_dims,
        bench_worldgen_normal_dims,
        bench_light_compute_normal_dims,
        bench_wcc_toggle_emit_normal_dims,
        bench_wcc_mesher_s1_uniform,
        bench_wcc_mesher_s2_mixed
}

criterion_group! {
    name = benches_heavy;
    config = heavy_config();
    targets =
        bench_build_chunk_wcc_normal_neighbors
}
criterion_main!(benches, benches_heavy);

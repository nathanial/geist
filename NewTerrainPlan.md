# New Terrain Streaming Plan

## Executive Summary
This document outlines a comprehensive refactoring of the Geist terrain generation pipeline to eliminate redundant computation, improve streaming performance, and enable lightweight terrain analytics. The key innovation is separating 2D column-level worldgen from 3D voxel materialization, allowing height/biome data to be computed once and reused across all vertical chunk layers.

## Goals
- **Predictable Streaming**: Stream visible chunks in a stable, distance-prioritized order so the camera never outruns terrain generation
- **Data Reuse**: Cache and share noise, height, biome, and feature computations across chunks and vertical stacks to avoid recomputing 2D data
- **Column-First Architecture**: Separate column-level worldgen from voxel materialization so we can render overviews and analytics without touching every voxel
- **Hot-Reload Support**: Maintain hot-reload of worldgen params and existing edit systems while tightening CPU budgets
- **Performance Targets**: Reduce chunk generation time by 60-80% through caching and column-oriented processing

## Current Implementation Analysis

### Codebase Structure
The terrain generation system spans several crates:
- `geist-chunk`: Core chunk buffer and generation (`generate_chunk_buffer`)
- `geist-world`: World generation logic, parameters, and GenCtx management
- `geist-runtime`: Job scheduling and worker thread management
- `app/events.rs`: Chunk streaming logic using `spherical_chunk_coords`

### Critical Bottlenecks

#### 1. Per-Voxel Column Lookups (262k calls per chunk)
```rust
// Current: geist-chunk/src/lib.rs:126-138
for z in 0..sz {
    for y in 0..sy {
        for x in 0..sx {
            // Every voxel queries world generation independently
            let block = world.block_at_runtime_with(reg, &mut ctx, wx, wy, wz);
            // This triggers height sampling, biome lookup, cave carving, etc.
            // for EVERY voxel, even though most data is column-scoped
        }
    }
}
```
**Impact**: A 64³ chunk performs ~262,144 world generation queries, but only needs 64×64 = 4,096 unique column evaluations.

#### 2. GenCtx Lifecycle Waste
```rust
// Current: Per-chunk context creation
let mut ctx = world.make_gen_ctx(); // Creates 3+ FastNoiseLite instances
world.prepare_height_tile(&mut ctx, base_x, base_z, sx, sz);
// ... use ctx for one chunk ...
// ctx dropped, losing all cached data
```
**Impact**: Each chunk rebuild creates fresh noise generators and discards column memoization after use.

#### 3. HeightTile Cache Inefficiency
The current `HeightTile` caching in `GenCtx`:
- Only lives for single chunk generation
- Gets recomputed for every vertical layer (cy=0, cy=1, cy=2...)
- No sharing between adjacent chunks processing simultaneously
- Tree/cave helpers re-sample the same heights independently

#### 4. Unoptimized Streaming Order
```rust
// Current: app/events.rs:22 - spherical_chunk_coords
// Produces chunks in nested loop order:
for dy in -radius..=radius {
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            // Mixed near/far chunks in submission order
        }
    }
}
```
**Impact**: Worker threads process distant chunks before nearby ones, causing cache thrashing and visible pop-in.

## Proposed Architecture

### 1. Chunk Stream Planner
**Purpose**: Replace chaotic loading order with predictable, cache-friendly progression.

**Design**:
```rust
pub struct ChunkStreamPlanner {
    center: ChunkCoord,
    load_radius: i32,
    prefetch_radius: i32,
    priority_queue: BinaryHeap<PriorityChunk>,
}

struct PriorityChunk {
    coord: ChunkCoord,
    distance_sq: i32,
    angular_hash: u32,  // Ensures stable ordering for equidistant chunks
}
```

**Key Features**:
- Sorts candidates by `distance²` ensuring nearest chunks load first
- Angular hash breaks distance ties, creating consistent spiral patterns
- Integrates with `App::record_intent` so all loading paths (streaming, edits, hot-reload) share the same priority system
- Maintains prefetch ring at `radius + 1` for anticipatory loading

**Benefits**:
- Camera never outruns terrain as nearest chunks always prioritized
- Workers process spatially coherent chunks, improving cache locality
- Predictable loading pattern reduces visual "popping"

### 2. Terrain Tile Cache
**Purpose**: Share expensive 2D computations across all chunks in a column.

**Design**:
```rust
pub struct TerrainTileCache {
    tiles: DashMap<(i32, i32), Arc<TerrainTile>>,
    lru_tracker: Mutex<LruCache<(i32, i32), ()>>,
    worldgen_rev: AtomicU32,
}

pub struct TerrainTile {
    base_x: i32,
    base_z: i32,
    worldgen_rev: u32,

    // Core terrain data (64×64 columns)
    surface_heights: Vec<i16>,        // Y coordinate of top solid block
    water_levels: Vec<i16>,           // Y coordinate of water surface

    // Climate & biome data
    temperatures: Vec<f32>,           // Temperature at each column
    moistures: Vec<f32>,              // Moisture at each column
    biome_ids: Vec<u8>,               // Resolved biome per column

    // Feature placement data
    tree_seeds: Vec<u32>,             // RNG seed for tree placement
    warp_offsets: Vec<(f32, f32)>,   // Cave warping noise values

    // Statistics
    compute_time_us: u32,
    reuse_count: AtomicU32,
}
```

**Key Features**:
- Shared via `Arc` for zero-copy access across worker threads
- LRU eviction sized to `stream_radius² × 1.5` tiles
- Worldgen revision tracking for instant invalidation on param changes
- Stores all column-invariant data computed once

**Benefits**:
- 64× reduction in height sampling (once per column vs once per voxel)
- Vertical chunk stacks (cy=0,1,2...) share the same tile
- Adjacent chunks can reuse overlapping tile computations
- Trees/caves access pre-computed data instead of re-sampling

### 3. Column-First Chunk Builder
**Purpose**: Transform O(n³) voxel operations into O(n²) column operations plus efficient span fills.

**Two-Phase Design**:

**Phase 1: Column Extraction (Per XZ)**
```rust
pub struct ColumnInfo {
    surface_y: i16,           // Top of terrain
    water_y: Option<i16>,     // Water level if present
    top_block: BlockId,       // Surface block type
    sub_blocks: Vec<BlockId>, // Subsurface layers
    feature_mask: u32,        // Bit flags for trees, ores, etc.
    column_seed: u32,         // Deterministic RNG seed
}

// New column-oriented generation
for z in 0..chunk_sz {
    for x in 0..chunk_sx {
        let column = extract_column_info(tile, x, z);
        columns[z * chunk_sx + x] = column;
    }
}
```

**Phase 2: Voxel Materialization (Span Fills)**
```rust
for column in columns {
    // Instead of 64 individual voxel writes:
    // 1. Write bedrock span (0..bedrock_y)
    // 2. Write stone span (bedrock_y..surface_y-3)
    // 3. Write dirt span (surface_y-3..surface_y)
    // 4. Write grass at surface_y
    // 5. Fill water if present (surface_y+1..water_y)
    // Total: ~4-5 operations vs 64
}
```

**Benefits**:
- 10-20× fewer operations per chunk
- Better CPU cache utilization (sequential memory access)
- Deterministic feature placement via column seeds
- Can serialize `ColumnInfo` arrays for instant chunk reloads

### 4. GenCtx Pooling
**Purpose**: Eliminate repeated noise generator initialization and retain hot caches.

**Design**:
```rust
pub struct GenCtxPool {
    available: crossbeam::queue::SegQueue<GenCtx>,
    max_contexts: usize,
}

impl GenCtxPool {
    pub fn acquire(&self) -> PooledGenCtx {
        let ctx = self.available.pop()
            .unwrap_or_else(|| GenCtx::new());
        PooledGenCtx { ctx, pool: self }
    }
}

// RAII wrapper returns context to pool on drop
pub struct PooledGenCtx<'a> {
    ctx: GenCtx,
    pool: &'a GenCtxPool,
}
```

**Key Features**:
- Lock-free queue for zero contention
- Contexts preserve FastNoiseLite instances across jobs
- Small per-context HashMap caches recent column lookups
- Automatic return to pool via RAII pattern

**Benefits**:
- Eliminates ~3-5ms of noise generator setup per chunk
- Preserves column memoization for adjacent chunks
- Reduces allocation pressure on worker threads

### 5. Overview & Analytics Layer
**Purpose**: Enable terrain visualization without voxel generation cost.

**Design**:
```rust
pub struct WorldOverview {
    tile_cache: Arc<TerrainTileCache>,
    render_target: OverviewTarget,
}

pub enum OverviewTarget {
    HeightMap,
    BiomeMap,
    MoistureGradient,
    CavePreview,
}

impl WorldOverview {
    pub async fn generate_region(&self, min: (i32, i32), max: (i32, i32)) -> Image {
        // Fetch tiles for region (no voxel generation)
        let tiles = self.fetch_tiles(min, max).await;

        // Aggregate column data into image
        match self.render_target {
            HeightMap => render_heights(tiles),
            BiomeMap => render_biomes(tiles),
            // ...
        }
    }
}
```

**Benefits**:
- Generate 1024×1024 terrain overview in <100ms
- No voxel materialization overhead
- Perfect for minimaps, world selection, debugging
- Can run in parallel with main generation

### 6. Scheduling & Streaming Integration
**Purpose**: Maximize cache reuse and worker efficiency.

**Integration Points**:

1. **BuildJob Enhancement**:
```rust
pub struct BuildJob {
    coord: ChunkCoord,
    tile: Option<Arc<TerrainTile>>,  // Pre-fetched tile
    priority: i32,                    // Distance-based priority
}
```

2. **ChunkColumnCache** (Runtime-level):
```rust
// Small cache for in-flight column data
pub struct ChunkColumnCache {
    entries: DashMap<(i32, i32), Arc<Vec<ColumnInfo>>>,
    max_entries: usize,  // ~10-20 chunks
}
```

3. **Performance Instrumentation**:
- Tile cache hit/miss rates
- Column span efficiency metrics
- Worker thread utilization graphs
- Generation time percentiles

**Benefits**:
- Adjacent chunks share tile and column data
- Workers stay saturated with high-priority work
- Real-time performance visibility

## Implementation Phases

### Phase 1: Instrumentation & GenCtx Pool (Week 1)
**Goals**: Establish performance baseline and eliminate context creation overhead.

**Tasks**:
- [x] Add detailed timing instrumentation to `generate_chunk_buffer`
  - Per-phase timers (height sampling, voxel fill, feature placement)
  - Histogram of chunk generation times
  - Export metrics to debug overlay
- [x] Implement `GenCtxPool` in `geist-runtime`
  - Lock-free pool backed by a bounded crossbeam channel
  - Pool size = worker_threads × 2
  - Benchmark: expect 3-5ms reduction per chunk
- [x] Verify no regressions in lighting/edit rebuild paths

**Phase 1 Baseline Checklist (2025-09-20)**
- Launch viewer via `cargo run -- run --world flat --seed 42` and allow two full stream radii to populate. Capture 60s of samples with the Diagnostics → Terrain Pipeline overlay focused.
- Export rolling stats from the overlay: `Chunk Build` (total, fill, features) plus per-stage averages. Record p50/p95 from the overlay’s hover tooltips.
- While the viewer runs, tail the perf logger (`RUST_LOG=perf=info`) and compute chunks/sec from `BuildChunkJobCompleted` lines (count / sampling interval).
- Populate a new table in this document once metrics are gathered on hardware (hiDPI MBP, Release build) so future phases can compare against a known baseline.

**Success Metrics**:
- 10-15% reduction in chunk generation time
- Zero allocation overhead in steady-state generation

### Phase 2: TerrainTileCache Infrastructure (Week 2)
**Goals**: Build foundation for column data sharing.

**Tasks**:
- [ ] Define `TerrainTile` struct with all column data
- [ ] Implement `TerrainTileCache` with DashMap backend
- [ ] Add worldgen revision tracking and invalidation hooks
- [ ] Integrate with existing `prepare_height_tile` as proof-of-concept
- [ ] Add cache metrics (hit rate, evictions, memory usage)

**Success Metrics**:
- 90%+ cache hit rate for adjacent chunks
- <0.5ms tile lookup latency

### Phase 3: Column-First Builder (Week 3-4)
**Goals**: Transform core generation from O(n³) to O(n²).

**Tasks**:
- [ ] Refactor `generate_chunk_buffer` into two phases:
  - Phase 1: Column extraction (4,096 ops)
  - Phase 2: Span-based voxel fill
- [ ] Implement efficient span fill algorithms:
  - Bedrock/stone/dirt layer spans
  - Water volume fills
  - Air space skipping
- [ ] Update cave carving to work with column data
- [ ] Ensure deterministic tree/feature placement via column seeds
- [ ] Profile and optimize memory access patterns

**Success Metrics**:
- 50-70% reduction in voxel operation count
- 40-60% overall speedup in chunk generation

### Phase 4: Chunk Stream Planner (Week 4)
**Goals**: Optimize loading order for cache coherency.

**Tasks**:
- [ ] Implement `ChunkStreamPlanner` with priority queue
- [ ] Replace all `spherical_chunk_coords` calls
- [ ] Add distance-based prioritization to job queue
- [ ] Implement prefetch ring logic (radius + 1)
- [ ] Tune queue depths and submission rates

**Success Metrics**:
- Visible chunks load before distant ones 100% of the time
- 20-30% improvement in cache hit rates
- Smoother terrain appearance during movement

### Phase 5: Column Cache & Overview (Week 5)
**Goals**: Enable fast terrain analytics and chunk reloads.

**Tasks**:
- [ ] Implement `ChunkColumnCache` for in-flight data sharing
- [ ] Add column profile serialization for instant chunk reloads
- [ ] Build `WorldOverview` service with multiple render modes:
  - Height map generation
  - Biome distribution visualization
  - Cave system previews
- [ ] Add CLI command: `geist overview --region 0,0,1024,1024 --mode heightmap`
- [ ] Create async job system for background overview generation

**Success Metrics**:
- 1024×1024 overview generation in <100ms
- Chunk reload from serialized columns in <1ms
- Zero impact on runtime generation performance

### Phase 6: Cleanup & Telemetry (Week 6)
**Goals**: Polish, document, and ship.

**Tasks**:
- [ ] Remove legacy height tile code paths
- [ ] Add comprehensive performance dashboard:
  - Real-time chunk generation rate
  - Cache statistics
  - Worker utilization graphs
  - Memory usage breakdown
- [ ] Document configuration options:
  - Cache sizing parameters
  - Prefetch radius tuning
  - Worker pool sizing
- [ ] Write migration guide for downstream code
- [ ] Create performance regression tests

**Success Metrics**:
- All metrics visible in debug overlay
- Zero performance regressions in edge cases
- Documentation complete with tuning guide

## Expected Outcomes

### Performance Improvements
- **Chunk Generation**: 60-80% faster (from ~50ms to ~10-20ms per chunk)
- **Memory Usage**: 30% reduction through shared tile data
- **Cache Efficiency**: 90%+ hit rate for terrain tiles
- **Worker Utilization**: 95%+ thread saturation during streaming
- **Startup Time**: 2-3× faster initial world load

### New Capabilities
- **Instant Overview Maps**: Generate world previews without voxels
- **Chunk Reload**: Restore evicted chunks from cached columns
- **Performance Analytics**: Real-time visibility into generation pipeline
- **Predictable Streaming**: Guaranteed near-to-far loading order
- **Hot-Reload Friendly**: Instant worldgen param updates with smart invalidation

### Technical Debt Reduction
- Cleaner separation between 2D terrain and 3D voxelization
- Reduced coupling between generation stages
- Better testability through phase isolation
- Foundation for future optimizations (GPU terrain, LOD system)

## Risk Mitigation

### Potential Issues & Solutions

1. **Memory Pressure from Tile Cache**
   - Solution: Aggressive LRU eviction, configurable cache size
   - Monitoring: Track cache memory in debug overlay

2. **Thread Contention on Shared Caches**
   - Solution: DashMap for lock-free reads, Arc for immutable sharing
   - Monitoring: Cache access latency percentiles

3. **Determinism Breakage**
   - Solution: Column seeds ensure identical output
   - Testing: Regression tests comparing chunk hashes

4. **Compatibility with Existing Systems**
   - Solution: Maintain current public APIs, internal refactor only
   - Testing: Full test suite pass required for each phase

## Conclusion

This architectural overhaul addresses the fundamental inefficiency of computing column data 64 times per chunk. By caching and sharing this work, we can achieve near-linear scaling with chunk count rather than cubic scaling with voxel count. The phased approach ensures we can measure improvements at each step while maintaining system stability. The end result will be a terrain generation system that's not only faster but also more predictable, observable, and extensible.

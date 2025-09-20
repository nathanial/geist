# New Terrain Streaming Plan

## Goals
- Stream visible chunks in a stable, distance-prioritized order so the camera never outruns terrain.
- Reuse noise, height, biome, and feature work across chunks and vertical stacks to avoid recomputing 2D data.
- Separate column-level worldgen from voxel materialization so we can render overviews and analytics without touching every voxel.
- Keep hot-reload of worldgen params and existing edit systems functional while tightening CPU budgets.

## Current Bottlenecks
- `generate_chunk_buffer` in `crates/geist-chunk` walks every `(x, y, z)` voxel and calls `World::block_at_runtime_with`, so a 64³ chunk performs ~262k column lookups even though most work is column-scoped (see `TerrainGenBrainstorm.md`).
- Each chunk build spawns a fresh `GenCtx` (`World::make_gen_ctx`), recreating multiple `FastNoiseLite` instances and dropping any column memoization as soon as the job ends.
- Height/biome sampling is repeated for every vertical chunk layer and again for tree/cave helpers, because the cached `HeightTile` only lives for one chunk job.
- `spherical_chunk_coords` produces load candidates in nested-loop order; the queue mixes near/far chunks so worker utilization is bursty and cache reuse is mostly luck.
- There is no fast path to answer "what does this region look like" without instantiating full chunk buffers, so overview maps pay the same cost as in-view terrain.

## Proposed Architecture

### 1. Chunk Stream Planner
- Replace the ad-hoc `spherical_chunk_coords` ordering with a `ChunkStreamPlanner` that sorts candidates by `distance²` and a small angular hash so loads advance ring-by-ring.
- Push this planner into `App::record_intent` so every cause (streaming, edits, hot reload) cooperates with the priority queue.
- Maintain a short-term prefetch ring (e.g. radius +1) so generation can start before the camera moves.

### 2. Terrain Tile Cache
- Introduce `TerrainTileCache` inside `geist-world`, keyed by `(tile_x, tile_z)` at chunk granularity. Each tile stores:
  - `surface_height[y]` (i16) for `64×64` columns.
  - Precomputed climate samples (temp/moisture), biome ids, and tree seeds.
  - Warp noise offsets needed by caves/features.
- Tiles carry a `worldgen_rev` so `World::update_worldgen_params` can invalidate them en masse.
- Use an `Arc<TerrainTile>` plus `parking_lot::RwLock` or `dashmap` to share immutable tiles across worker threads with LRU eviction sized to the active stream radius.

### 3. Column-First Chunk Builder
- Refactor `generate_chunk_buffer` into two phases:
  1. **Column extraction**: for each `(x, z)` look up the cached tile row, compute column metadata once (`ColumnInfo { surface_y, water_y, top_block, sub_block, feature_mask }`).
  2. **Voxel fill**: emit solid spans per column: ground (solid), cave replacements, water fill, air above height. This collapses the inner loop to two or three slice writes instead of `64` per column.
- Trees/features already request neighbor heights; feed them from the tile cache and stash per-column RNG state in `ColumnInfo` so replays are deterministic without re-sampling noise.
- Store optional `ColumnProfile` blobs (compressed) alongside `ChunkEntry` so reloading an evicted vertical layer can skip recomputing phase 1.

### 4. GenCtx Pooling
- Add a lock-free `GenCtxPool` owned by `Runtime`; worker threads fetch a context on job start and return it afterwards. Contexts retain the last-used `TerrainTile` handles and small `HashMap<(wx, wz), ColumnInfo>` for transient memoization.
- Keep a fallback path for single-voxel queries (`World::block_at_runtime`) by lazily allocating a short-lived context, as today.

### 5. Overview & Analytics Layer
- Build a `WorldOverview` service that can request `TerrainTile`s directly, aggregate them into a raster (heightmap, moisture, biome ids) and emit a texture or CPU buffer.
- Overview requests operate on the column metadata only—no voxel fill—so generating a map of N×N chunks is O(N²) instead of O(N²×chunk_height).
- Expose an async job (`OverviewJob`) that can run on the background worker pool, writing into `showcase_output/` or an in-memory image for UI overlays.

### 6. Scheduling & Streaming Integration
- Extend `BuildJob` to carry an optional `Arc<TerrainTile>` so the runtime doesn’t refetch the same tile per lane.
- When a chunk finishes phase 1, insert its column metadata into a small `ChunkColumnCache` keyed by `(cx, cz)`; neighboring chunk builds check this cache first so the same tile feeds multiple jobs in flight.
- Add instrumentation (perf HUD counters) to display tile cache hit rate, column span counts, and overview generation latency.

## Implementation Phases
1. **Instrumentation & GenCtx Pool**
   - Introduce scoped timers around chunk build phases.
   - Add `GenCtxPool` and confirm no regressions in lighting/edit rebuild paths.
2. **TerrainTileCache Infrastructure**
   - Define tile struct + cache, hook invalidation to `World::update_worldgen_params`.
   - Wire cache reads into existing `prepare_height_tile` to maintain behaviour while measuring gains.
3. **Column-First Builder**
   - Split `generate_chunk_buffer` into column phase + span fill.
   - Update `process_build_job` to pass cached metadata into mesher/light workers (e.g., for occupancy tests).
4. **Chunk Stream Planner**
   - Replace `spherical_chunk_coords` usage with the priority planner and tune queue limits.
   - Measure worker utilization before/after.
5. **Column Cache & Overview**
   - Store per-chunk column profiles for quick reloads.
   - Implement `WorldOverview` APIs and add a CLI command to dump a terrain map without voxel generation.
6. **Cleanup & Telemetry**
   - Remove obsolete height tile code paths.
   - Ship metrics to the debug overlay and document cache sizing knobs in README.

This plan keeps generation deterministic, lets background jobs reuse expensive noise work, and opens the door to lightweight terrain overviews without voxel churn.

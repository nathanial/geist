# Terrain Generation Brainstorm

## Current observations
- `crates/geist-world/src/voxel/generation.rs:44` runs `ColumnSampler::height_for` for every voxel. With `CHUNK_SIZE = 64` this means ~262k height evaluations per chunk, plus neighbor lookups for caves/trees.
- `ColumnSampler::height_for` re-samples the `terrain` noise each time. Because the sampler sits on `GenCtx`, we do reuse the noise generator, but not the per-column results.
- Vertical chunk stacks (`ChunkCoord { cy: ... }`) re-evaluate the exact same `(wx, wz)` columns, so in tall worlds we duplicate work per layer.
- Tree and feature helpers call `height_for` on adjacent columns (`generation.rs:428`, `generation.rs:643`), multiplying the cost.
- Water level and biome lookups are comparatively cheap but could ride along with a height cache once we build one.

## Requirements & constraints
- Chunk generation order is sparse/random; any cache needs to tolerate missing neighbors and eviction.
- Worldgen params are hot-reloadable (`World::update_worldgen_params`), so cached data must flush on change.
- Lighting and other systems call `block_at_runtime_with` for single voxels; we should avoid penalising those paths with mandatory cache construction.
- Memory footprint matters: a 64×64 scalar grid per chunk is ~16 KiB (`i16`). That's fine per chunk, but global caches need pruning.

## Brainstormed directions

### 1. Per-chunk height pre-pass
- In `crates/geist-chunk/src/lib.rs`, compute a `Vec<i16>` (or `SmallVec<[i16; 4096]>`) of surface heights before filling voxels. Feed a new `ChunkColumnHeights` into the inner loop so `block_at_runtime_with` can skip `height_for` and consult the cache.
- Extend `GenCtx` with an optional `height_tile: Option<ChunkColumnHeights>` to keep API compatibility. When absent (lighting probes), fall back to current on-demand sampling.
- Pros: deterministic reuse for the chunk we're already building; simple to drop when chunk is done. Cons: requires touching both the chunk crate and world crate; still recomputes heights when another system asks for the same column later.

### 2. Column-first filling workflow
- Instead of the triple nested voxel loop, iterate `(x, z)` columns first, compute height + biome + water once, then fill the Y span in one go. This can live in a helper `world.fill_chunk_column(&mut ctx, ... )`.
- Lets us amortise other per-column computations (tree RNG seeding, column noise warping) and avoid re-fetching neighbors by staging a small 3×3 ring of heights for feature placement.
- Requires more invasive refactor but keeps the cache local and invalidation-free.

### 3. Persistent 2D height cache keyed by `(cx, cz)`
- Maintain a `DashMap<(i32, i32), Arc<HeightTile>>` in `World` or behind `GenCtx`. Any vertical chunk reuses the same tile, and we can adopt an LRU bound (e.g. 256 tiles) to control memory.
- Need invalidation hooks when worldgen params change. Multi-threaded chunk builds would benefit from sharing the tile via `Arc`.
- Adds synchronisation overhead; must design eviction carefully to avoid stalling hot paths.

### 4. Lightweight column memoization inside `GenCtx`
- Add a small `hashbrown::HashMap<(i32, i32), ColumnSample>` to `GenCtx` that caches height/biome/water for the lifetime of the context (i.e. a single chunk build today).
- `ColumnSampler::height_for`/`biome_for` check the map before sampling noise; features automatically benefit without API changes.
- Minimal churn, but still redoes the work when another chunk uses a fresh `GenCtx`. Could be combined with idea #3 for longer-lived caching.

### 5. Batch noise sampling via `FastNoiseLite::get_noise_set`
- The library can compute batched grids faster than repeated scalar calls. For a 64×64 tile we call `get_noise_set(width=64, height=64, start_x, start_z, step)` once.
- We can pipe the results straight into a height array, then reuse as in idea #1 or #2. Would substantially cut CPU per chunk even without broader caching.
- Needs a wrapper to account for warp noise (if we keep using it) and to handle the `min_y_ratio` / `max_y_ratio` scaling.

### 6. Async/prewarm height workers
- As a follow-up to caching, spawn a lightweight worker that watches the chunk streamer and precomputes height tiles for `(cx, cz)` likely to load soon (e.g. neighbors of current camera column).
- Helps hide the cost even if we keep expensive sampling, but requires plumbing futures/tasks into the chunk manager.

## Open questions / follow-ups
- Should tree placement key off the same cache? Several helpers currently mutate RNG state based on `ColumnSampler`; we may need to separate deterministic tree seeding from height storage.
- Do other systems (lighting, structures) rely on `height_for` side effects? A quick audit suggests no, but worth confirming before memoizing.
- How much variance does the warp noise add? If it's substantial, a cache also needs to capture warped coordinates, not just raw `(wx, wz)`.
- Next step: prototype idea #4 (memoization in `GenCtx`) since it's low-risk, measure, then consider lifting it into a chunk-wide or world-wide cache if gains are good.

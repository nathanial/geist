# Terrain Generation Improvement Plan

## Current Pipeline
- `generate_chunk_buffer` iterates every voxel in a 64³ chunk and calls `World::block_at_runtime_with` for each sample (`crates/geist-chunk/src/lib.rs:53`).
- `block_at_runtime_with` mixes terrain height sampling, biome lookup, cave carving, features, trees, and water decisions in a monolithic function (`crates/geist-world/src/voxel.rs:401`).
- Each invocation rebuilds the same per-column data (terrain height, biome climate, tree randomness) even though `x`/`z` stay constant across the inner `y` loop.
- Cave carving applies multiple fractal noise and Worley evaluations per voxel, then the feature and tree systems revisit the same neighbourhood data repeatedly.

## Observed Hotspots & Redundant Work
| Area | Evidence | Estimated Cost per 64³ chunk |
| ---- | -------- | ----------------------------- |
| Terrain height (`height_for`) | Called once per voxel when deciding `base`, again through `trunk_at`, and inside neighbour checks for cave features (`crates/geist-world/src/voxel.rs:526`, `742`, `824`). | ≥262k 2D noise calls baseline, doubling to ≈524k once tree probes are counted; neighbour probes can push this into the millions for leafy volumes. |
| Climate / biome selection | `top_block_for_column`, `tree_prob_for`, and `trunk_at` all recompute temperature/moisture noise (`crates/geist-world/src/voxel.rs:546`, `861`). | Upwards of 3× the column count (≈12k) per chunk for pure surface logic, ballooning to ≈262k when tree loops run for every voxel. |
| Tree detection | `trunk_at` recomputes height, biome, random hashes for every `(x, z)` request, then the leaf fill loops call it for up to 25 neighbouring columns per air voxel (`crates/geist-world/src/voxel.rs:885-949`). | Worst case tens of millions of hash/noise calls in dense canopy layers. |
| Cave carving noise | Every subterranean block evaluates 3 warp fractals, 1 tunnel fractal, plus a 27-cell Worley search; neighbour-solid checks redo the full stack for up to six neighbours (`crates/geist-world/src/voxel.rs:640-780`). | For half-solid chunks this is >130k fractal calls and ~3.5M hash operations, multiplied again when features query `near_solid`. |
| Feature loop | Each voxel iterates the full feature list and hashes coordinates for probability gates even when early filters fail (`crates/geist-world/src/voxel.rs:792-820`). | ~262k rule iterations; `glowstone_sprinkle` additionally triggers expensive neighbour-solid checks. |
| Block ID resolution | We hit an `RwLock<HashMap>` for every voxel (`crates/geist-world/src/voxel.rs:955`). The cache helps, but the lock traffic remains high under parallel generation. | Adds contention and cache-line churn once multi-threaded. |

These hotspots explain the ~1 s per chunk load time that was observed: the same expensive noise and rule evaluation work is repeated for each `y` slice instead of being shared at the column or chunk level.

## Recommended Improvements

### 1. Build Column Caches Up Front (High Impact, Moderate Effort)
- During chunk generation, iterate `(x, z)` once to compute and store:
  - Terrain height, surface block name, water depth thresholds.
  - Biome/temperature/moisture triples and resolved tree probability/species choices.
  - Any feature rule flags that depend only on column data (e.g. `below_height_offset`, biome filters).
- Feed this cache into a second pass over `y`, eliminating redundant noise calls and hashmap lookups. This mirrors the previously spiked “pre-compact” approach that was reverted; reintroduce it with focused tests to guarantee behavioural parity.
- Expected results: remove ~70–80% of the 2D noise and hash work per chunk.

### 2. Precompute Tree Instances per Column (High Impact, Higher Effort)
- Generate a sparse list of tree placements per chunk using cached column data (surface height, biome, hashed randomness) and store `(x, z, surface_y, height, species)`.
- Use this list to stamp trunks and leaf volumes during the `y` sweep instead of calling `trunk_at` dynamically.
- Cache leaf neighbourhoods (e.g. store per-tree bounding boxes) so leaf filling becomes a simple geometric test with integer math.
- Benefit: removes the nested `trunk_at` calls and associated noise/hash churn; clears up to tens of millions of redundant operations in forested areas.

### 3. Cache Carver Noise Fields (Medium Impact, Medium Effort)
- Evaluate the warp (`ctx.warp`) and tunnel (`ctx.tunnel`) noise on a grid once per chunk (e.g. using `FastNoiseLite::get_noise_set` or manual tiling) and reuse the values for both the voxel itself and neighbour-solid checks.
- Precompute Worley offsets for a chunk cell neighbourhood so the 27-sample loop pulls from arrays rather than rehashing per voxel.
- Split the cave logic so neighbour-solid checks read from cached results instead of recomputing their own warp/tunnel paths.
- This should cut fractal/Worley calls by roughly 5–6× and trim the hash workload dramatically.

### 4. Feature Rule Acceleration (Medium Impact, Low Effort)
- Partition `ctx.params.features` by simple predicates (e.g. base block, y-range) before voxel iteration, so most rules are skipped without touching per-voxel hashes.
- Memoise the glowstone `near_solid` decision per voxel once the cave cache (step 3) exists, or precompute a boolean solid mask for the chunk column to avoid repeated neighbour evaluations.
- Add short-circuit evaluation when `base` resolves to air early; most features target solid blocks.

### 5. Address Shared Block ID Lookups (Low Impact, Low Effort)
- Replace the `RwLock<HashMap<String, u16>>` with an `Arc<[AtomicU16]>` keyed by registry index or stash resolved IDs inside the column cache.
- Reduces locking overhead when multiple threads generate chunks simultaneously.

### 6. Instrument & Validate
- Add lightweight timing around the column preprocessing, carver evaluation, and tree stamping to confirm wins and guard against regressions.
- Capture histograms in the existing debug overlay for: `height_for` cache hit rate, carver noise reuse, tree placement counts.
- Run `cargo fmt`, `cargo clippy --workspace --all-targets`, and `cargo test --workspace` after each optimisation phase to ensure stability.

## Rollout Strategy
1. **Column Cache Refactor** – Introduce a `ChunkColumn` struct, adjust `generate_chunk_buffer`, and verify functional parity via snapshot tests of small worlds.
2. **Tree Placement Refine** – Migrate trunk/leaf logic to consume cached placements; compare canopy silhouettes before/after using deterministic seeds.
3. **Carver Noise Cache** – Implement chunk-level noise buffers and reuse them in neighbour-solid checks; profile the resulting carving pass.
4. **Feature Filtering & ID Cache** – Layer on rule partitioning and block ID improvements once the heavy math is reduced.
5. **Telemetry & Regression Tests** – Extend debug metrics to track chunk-gen durations and ensure new caches stay hot under different seeds.

## Risks & Mitigations
- **Behaviour drift**: caching must respect waterline clamping—and cached values should remain tied to the same world-space coordinates to avoid off-by-one artefacts. Add targeted tests for surface block selection, water filling, and tree layouts.
- **Memory spikes**: chunk-level buffers should be stack-allocated or pooled to avoid GC churn; reuse allocations via thread-local scratch space.
- **Thread safety**: precomputed caches must be confined to the worker thread to avoid sharing mutable noise state across jobs.

By reorganising generation around reusable column and chunk caches, we can remove the vast majority of redundant noise and hash evaluations that currently dominate chunk load time, bringing terrain builds back into the tens-of-milliseconds range per chunk.

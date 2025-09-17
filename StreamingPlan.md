# StreamingPlan

## Goals
- Support an unbounded vertical world by removing the fixed `chunks_y` dimension and letting the runtime stream chunks on demand in all directions.
- Avoid preallocating large contiguous arenas for lighting, edits, meshing, or rendering; instead allocate per-chunk resources lazily and release them when chunks fall outside the active set.
- Preserve (or improve) existing streaming behavior for X/Z movement while extending it to vertical travel, keeping lighting, edits, and rendering seamless across newly loaded slices.
- Maintain determinism and save/load fidelity even though the set of resident chunks becomes dynamic.

## Guiding Principles
- Prefer sparse maps (`HashMap<ChunkCoord, T>` or slab pools keyed by handles) over index math derived from a fixed world envelope.
- Isolate chunk lifecycle changes behind clear APIs (e.g., `WorldStore`, `LightingStore`, `RuntimeScheduler`) so subsystems share the same materialize/evict rules.
- Keep empty-chunk short-circuiting in place to avoid spending work on all-air volumes; integrate occupancy tracking with the new sparse stores.
- Introduce eviction/TTL policies early to prevent unbounded memory growth when players explore far-away regions.

## Phase Breakdown

### Phase 0 – Discovery & Instrumentation
- Audit every subsystem that still relies on `chunks_y`, `world_size_*`, or dense indexing helpers (lighting, edits, runtime job queues, minimap caches, renderer staging buffers, CLI/config).
- Add temporary metrics (e.g., gauge of resident chunks per axis, buffer pool usage) so we can observe pressure once the world becomes unbounded.
- Document expectations for neighbor availability (e.g., lighting requires ±Y borders before finalizing a chunk) to ensure we replicate them with sparse lookups.

### Phase 1 – Configuration & World Metadata
- Update CLI (`RunArgs`) and `World::new` so vertical extent becomes an optional hint: accept `--chunks-y-hint` (defaulting to 8) but do not clamp chunk coordinates to it.
- Replace `World::world_size_y()` and related helpers with variants that use the hint only for spawn height / heuristics, while authoritative size derives from active chunks.
- Ensure worldgen and terrain functions can request voxels for arbitrary `(cx, cy, cz)` without panicking when `cy` exceeds the original hint.

### Phase 2 – Core Stores → Sparse
- Refactor `LightingStore` to manage per-chunk lighting data in a `HashMap<ChunkCoord, LightingChunk>` (or arena keyed by `ChunkHandle`), with lazy creation and eviction hooks.
- Apply the same pattern to `EditStore`, `ChunkEntry` buffers, and mesh caches so no structure assumes a packed index range.
- Introduce a shared `ChunkInventory` (authoritative map of chunk states: Missing, Loading, Ready{occupancy, lighting, mesh}) to coordinate between runtime workers and renderer.
- Update neighbor queries (lighting borders, finalize state, mesh seam resolution) to fetch optional neighbors from the sparse store instead of computing indices.

### Phase 3 – Runtime & Streaming Scheduler
- Rewrite the scheduler to derive required chunk coordinates solely from view position + radius, independent of any world bounds.
- Allow jobs to spawn for arbitrary `cy` values; handle requeues when vertical neighbors arrive later.
- Implement eviction policies (LRU by distance, or “keep N shells around view center”) so memory stays bounded as players travel.
- Ensure empty-chunk fast paths still tag neighbors as satisfied even when chunks are skipped due to emptiness.

### Phase 4 – Rendering & UI Adjustments
- Update render loop and minimap to iterate over the sparse chunk registry, respecting occupancy and eviction state.
- Provide debug overlays for chunk residency, eviction age, and vertical stacks to help diagnose sparse streaming behavior.
- Verify frustum culling, chunk bounds display, and minimap legend continue to function without relying on contiguous indices.

### Phase 5 – Persistence & Hot Reload
- Adapt save/load formats so only resident chunks (plus edit/lighting deltas) are serialized; rehydrate by replaying edits and regenerating chunks on demand.
- Ensure worldgen hot-reload and terrain edits can materialize chunks that were previously evicted or never loaded.
- Revisit deterministic material assignment (textures/material catalog) in light of non-deterministic load order to guarantee consistent IDs across runs.

### Phase 6 – Testing & Stabilization
- Extend automated tests to cover vertical streaming scenarios: load chunks at high `cy`, evict them, then reload while verifying lighting and meshes remain correct.
- Add soak tests that simulate wandering players to measure memory usage, job churn, and average frame time with the sparse pipeline.
- Update developer tooling (profilers, logging) to surface chunk residency counts, eviction events, and outstanding job queues.

## Open Questions
- How aggressively should we evict lighting/edit data for far-away chunks, and what is the re-hydration cost when the player returns?
- Do we want background compaction (e.g., writing far chunks to disk) or purely in-memory eviction for now?
- Can we share buffer pools between lighting/meshing to amortize allocations once stores become sparse?
- How will multiplayer or replay capture interact with dynamic chunk residency (if/when those features arrive)?

## Immediate Next Steps
1. Implement Phase 0 audit: produce a checklist of `chunks_y` usages and current dense-array owners.
2. Prototype a sparse `LightingStore` behind a feature flag to validate API changes before touching every subsystem.
3. Decide on eviction heuristics (distance-based vs. LRU) so later phases can build against a concrete policy.

## Phase 0 Findings
- **Configuration & World Metadata:** `src/main.rs:64-135` still exposes `--chunks-y` and forwards the value into `World::new`, so the world stack remains bounded by the hint supplied on the CLI. `crates/geist-world/src/voxel.rs:203-257` stores `chunks_y` and derives `world_size_y()`, and terrain sampling clamps `y` into `[0, world_height)` (`crates/geist-world/src/voxel.rs:409-512`).
- **Rendering & UI:** Camera spawn logic and debug overlays depend on finite world height via `world.world_size_y()` (`src/app/init.rs:20-66`, `src/app/render.rs:143-159`, `src/app/render.rs:529-566`).
- **Runtime Finalization:** Neighbor readiness flags assume vertical owners exist inside the current stack (`src/gamestate.rs:12-53`, `src/app/runtime.rs:175-208`, `src/app/events.rs:399-699`).
- **Streaming Scheduler:** View-radius updates build spherical load sets but still gate intent queues with finite vertical shells (`src/app/events.rs:314-546`, `src/app/runtime.rs:180-220`).
- **Lighting & Edits:** Lighting borders are fetched from neighboring chunks stored in hash maps keyed by `(cx, cy, cz)` and expect contiguous availability for seam propagation (`crates/geist-lighting/src/lib.rs:1480-1660`), while the edit store tracks revisions per chunk coordinate and bumps neighbor slices on border hits (`crates/geist-edit/src/lib.rs:8-215`).

### Instrumentation Added
- Debug overlay now reports loaded vs active chunk counts, unique axis coverage, renderer cache size, and store residency metrics for lighting and edits (`src/app/state.rs:60-86`, `src/app/render.rs:21-156`).

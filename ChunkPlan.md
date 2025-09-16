# ChunkPlan

## Goals
- Switch chunking from 2D columns (x,z) of height 256 to true 3D chunks with a fixed extent of 32×32×32 voxels.
- Treat chunk size as a compile-time constant (no CLI/runtime overrides) while allowing the world height to span multiple stacked chunks.
- Stream chunks by evaluating a 3D sphere around the camera so vertical motion loads/unloads neighboring stacks consistently.
- Preserve existing gameplay features (edits, lighting, schematics, showcase) and keep hot-reload/watch flows working after the refactor.

## Constraints & Open Questions
- Chunk size becomes `const CHUNK_SIZE: usize = 32`; legacy CLI flags for per-axis chunk sizes need to be removed or ignored.
- Decide how many vertical chunks (`chunks_y`) we support by default (likely 8 to preserve a 256-block world). Allow configuration for total world height if needed.
- Finalization/lighting currently assumes horizontal neighbors only; the refactor must generalize ownership handshakes to six faces without introducing rebuild storms.
- Minimap/debug overlays are 2D today; we need a strategy (e.g., show current `cy` slice) so tooling remains useful.

## Phase Plan

### Phase 1 – Configuration & Constants
- Introduce a shared `CHUNK_SIZE` constant (and potentially helper accessors) and replace scattered literals/default args.
- Extend `World::new` to accept `chunks_y` (derive from existing height or CLI) and drop the adjustable `chunk_size_*` parameters.
- Update CLI (`RunArgs`) to remove chunk-size flags, add optional `chunks_y` / world-height flag, and adapt call-sites/tests.

### Phase 2 – Core Coordinate Plumbing
- Add a canonical chunk coordinate type (e.g., `struct ChunkKey { cx, cy, cz }`) or use `(i32, i32, i32)` consistently.
- Update `ChunkBuf`, `ChunkMeshCPU`, `ChunkRender`, `ChunkEntry`, runtime `BuildJob`/`JobOut`, and `NeighborsLoaded` to carry `cy` and expose helper constructors.
- Adjust serialization/debug logging (`Event::BuildChunkJobRequested`, etc.) to print the new dimension.

### Phase 3 – World Data & Generation
- Teach `World` about `chunks_y`, add `world_height()` helpers, and audit every `chunk_size_y` usage—replace ones that meant “world height” with the new accessor.
- Update `generate_chunk_buffer` to accept `(cx, cy, cz)`, compute `base_y`, and pass full world coordinates into `block_at_runtime_with`.
- Review `block_at_runtime` and related terrain math to ensure ratios, clamps, and cache keys now reference total world height instead of a single chunk.
- Verify showcase helpers and cached placements stay valid when height > one chunk (adjust spawn Y if needed).

### Phase 4 – Persistent Edits & Lighting Seeds
- Generalize `EditStore` to key edits/revisions by `(cx, cy, cz)`, update neighbor bumping logic to consider ±Y faces/corners, and extend snapshot helpers to cover vertical radius.
- Update disk I/O helpers (schematic import/export) to respect the new chunk indexing.

### Phase 5 – Lighting Pipeline
- Expand `LightingStore` internal maps to `(cx, cy, cz)` and expose vertical neighbor lookups (`get_neighbor_borders`, `get_neighbor_micro_borders`).
- Update micro-lighting routines to request and publish ±Y borders; adjust `LightBordersUpdated` event and change masks to include `yn/yp`.
- Ensure emitter add/remove helpers compute `cy` and that seam ownership rules cover all six directions.

### Phase 6 – Runtime Streaming & Scheduling
- Refactor `GameState` to store loaded chunks, inflight revisions, mesh/light counts, and finalize states keyed by `(cx, cy, cz)`.
- Change `center_chunk` to track `(ccx, ccy, ccz)` and recompute view volume using a spherical (distance²) test around the camera.
- Update `ViewCenterChanged`, `EnsureChunkLoaded/Unloaded`, and intent prioritization to operate in 3D (distance buckets, hysteresis, budgets).
- Redesign `FinalizeState` so each chunk tracks readiness per face (or encode owner combinations) and only requests finalize builds when all required neighbors published seams.
- Extend `NeighborsLoaded` and job hashing so scheduling differentiates ±Y readiness.

### Phase 7 – Meshing & Chunk Buffers
- Make `ChunkBuf::contains_world/get_world` aware of `cy` and chunk base Y; validate loops respect buffer bounds.
- Ensure WCC meshing (and thin-shape logic) uses the correct `base_y` in AABB creation, lighting sampling, and neighbor lookups.
- Update `build_chunk_wcc_cpu_buf_with_light` signatures to accept `cy` (or read from `ChunkBuf`) and propagate through `Runtime` workers.
- Confirm dynamic structures (`build_voxel_body_cpu_buf`) still work with the new coordinates.

### Phase 8 – Rendering & UI
- Adjust `ChunkRender` origins/bounding boxes to include vertical offsets so frustum culling and debug bounding boxes are accurate.
- Update draw loops to iterate over 3D keys, compute chunk origins `[cx*32, cy*32, cz*32]`, and feed new origins into shader uniforms.
- Revisit chunk bound overlays, minimap, and debug overlays to present meaningful info in 3D (e.g., highlight current `cy`, filter visible layers).
- Ensure structure rendering and raycasting continue to fetch blocks from edits/chunks with the new key lookup helpers.

### Phase 9 – Events, Watchers, and Hot Reload
- Propagate the new signatures through event dispatchers, watchers, and hot reload logic (worldgen, textures) so rebuild triggers land on the correct `(cx, cy, cz)`.
- Update worldgen hot reload invalidation to clear caches for every vertical slice and schedule rebuilds accordingly.

### Phase 10 – Tests, Benches, Docs
- Update unit tests, benches, and property tests in `geist-mesh-cpu`, `geist-lighting`, and elsewhere to use the new constructors and fixed chunk size.
- Add targeted tests covering:
  - Edit propagation across ±Y seams.
  - Lighting seam correctness when stacking chunks vertically.
  - Streaming sphere selection for diagonal/upward movement.
- Refresh documentation/README snippets and configuration comments to explain the new 3D chunk layout.

## Validation & Rollout
- Continuous sanity checks (`cargo fmt`, `cargo check --workspace`, existing test suites) after major phases.
- Manual verification steps once integrated:
  1. Fly vertically to ensure chunks unload/load correctly.
  2. Inspect lighting between stacked chunks for seam artifacts.
  3. Exercise edits, light emitters, and structure interactions spanning multiple `cy` levels.
- Consider staging behind a feature branch and gating with a smoke test run in the showcase world before merging.

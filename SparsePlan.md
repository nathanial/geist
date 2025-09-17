# SparsePlan

## Goals
- Avoid allocating, storing, or rendering chunks that contain only air to reduce memory use, meshing cost, and draw calls.
- Keep the world API coherent so subsystems (lighting, edits, streaming, shaders) can still reason about chunk neighbors even when some chunks are virtual/missing.
- Preserve determinism and disk persistence: saving/reloading should not silently reintroduce empty chunks or lose player edits.

## Constraints & Unknowns
- Edits can turn an empty chunk into a non-empty one later; we need a path to materialize chunks on demand.
- Lighting, physics, and worldgen queries expect neighbor data even if the neighbor is empty.
- Streaming budgets and minimap/debug tools assume every chunk has bookkeeping entries; we must keep metadata lightweight without breaking UI expectations.
- Schematics/hot-reload flows may rely on iterating every chunk in a region.

## Strategy Ideas

### 1. Generation-Time Emptiness Culling
- Teach the terrain generator to report whether a chunk contains any non-air voxels (e.g., maintain a bitset or occupancy counter during generation).
- If the generator reports empty, skip allocating `ChunkBuf`, `LightingStore`, and mesh jobs; instead insert a lightweight `EmptyChunk` record (coord + flags).
- Keep a version/hash for the generator so that when world settings change we can re-run emptiness checks.
- Risks: generation code paths that currently rely on writing directly into buffers need a fast path to early-out without allocating the buffer first.

### 2. Sparse Chunk Metadata Layer
- Introduce a `ChunkRegistry` that tracks chunk states: `Missing`, `Empty`, `Solid`. Store only metadata (timestamp, revision, light level ranges) for `Empty` chunks.
- Update streaming logic to work off this registry: treat `Empty` as loaded for neighbor checks so finalize/lighting can proceed without allocation.
- Expose helper queries (`is_chunk_occupied`, `ensure_chunk_materialized`) so edits or runtime features can promote an `Empty` chunk to a real buffer when needed.
- Renderer/minimap should skip `Empty` entries by default but still know they exist for controls like “show empty shells”.

### 3. Procedural Virtualization & On-Demand Materialization
- Allow worldgen and lighting queries to operate against a virtual chunk that lazily populates voxels on demand (e.g., compute-at-query using noise functions).
- Cache only the slices that have been queried; evict them using LRU when memory pressure rises.
- Ideal for infinite worlds where many chunks stay untouched; heavier CPU usage per query but minimal memory footprint.
- Requires deterministic worldgen functions and memoization so repeated queries produce consistent results without storing full chunk data.

### 4. Compressed Storage for Rarely Edited Regions
- Instead of raw voxel arrays, store empty or low-entropy chunks in a compressed form (RLE, sparse bitsets, palette compression) and decompress only when a worker needs mutable access.
- Combine with copy-on-write: read-only systems (meshing, lighting) can operate on iterators over the compressed representation.
- Offers middle ground when chunks are mostly air but contain a few blocks (cloud layers, ore veins) where full skipping is unsafe.

### 5. Hierarchical Spatial Index (Region/Octree)
- Group chunks into higher-level regions and maintain occupancy counts per region. Skip traversing child chunks when the region is marked empty.
- Useful for culling entire vertical columns or distant areas before the streaming scheduler considers them.
- Could drive renderer instancing (only submit visible non-empty regions) and disk compaction.

### 6. Edit & Persistence Workflow Adjustments
- Store edits against coordinates even if the owning chunk is currently virtual; applying the edit should materialize the chunk and seed lighting/meshing jobs.
- During save/load, omit empty chunks from disk snapshots to keep worlds small; rehydrate virtual markers based on generator + edit data.
- Provide tooling to audit which chunks are virtual vs. materialized to aid debugging.

### 7. Lighting & Physics Integration
- Lighting solver can treat missing neighbors as fully transparent, but needs border caches (e.g., baked skylight) for seamless transitions; store minimal per-face data instead of full volumes.
- Physics queries that need solidity should fall back to procedural generation if the chunk is virtual; cache results for repeated hits (raycasts, collision checks).
- Ensure AI/pathfinding and sound systems interpret virtual air chunks correctly so entities do not fall through unexpected holes when the chunk materializes later.

### 8. Rendering Considerations
- Mesh scheduler should request meshes only for `Solid` chunks; `Empty` chunks simply mark neighboring seams as satisfied with no geometry submission.
- Update culling to leverage sparse metadata (e.g., skip bounding boxes for `Empty` chunks) and keep minimap legend accurate.
- Consider optional placeholder visuals (wireframe boxes) when debugging virtual chunks to avoid confusion.

## Next Experiments
- Prototype generator-side occupancy detection on a single biome to measure the percentage of empty chunks and its impact on generation time.
- Implement a metadata-only `EmptyChunk` path in the world runtime behind a feature flag and verify that streaming, lighting, and edits still behave.
- Add instrumentation to track memory/time savings and identify hot spots where compressed chunks outperform binary skip vs. materialize decisions.

# Modularity Plan for Geist

This document proposes a clean workspace split with well-defined crate boundaries and stable APIs so multiple people (or agents) can build in parallel without stepping on each other. It reflects the current codebase and the recent runtime refactor (separate lanes for edit/light/bg) and lays out how to extract engine logic from rendering while preserving a straightforward application layer.

## Goals

- Isolate CPU “engine” from Raylib/GPU “renderer”.
- Provide small, stable crate APIs for parallel development.
- Keep dependency direction acyclic and simple to reason about.
- Enable incremental migration with minimal churn.
- Preserve performance (no unnecessary copies) and support existing runtime worker model.

## Current State (High-Level)

Key modules and responsibilities in `src/` today:

- `app.rs`: Main app loop, inputs/intents/backpressure, HUD/debug, event queue, drawing via Raylib.
- `runtime.rs`: Job lanes (edit/light/bg), workers, result drain, counters; also some Raylib concerns (texture reload/rebind, ChunkRender maps) that should move.
- `mesher.rs`: CPU mesh generation and Raylib model building (mixed responsibilities).
- `meshing_core.rs`: Greedy meshing core utilities.
- `voxel.rs` + `worldgen/`: World shape and runtime sampling; worldgen params/config.
- `lighting.rs`: LightGrid compute, neighbor borders, LightingStore; derives `LightBorders` for seams.
- `blocks/`: Materials/blocks config, registry, runtime `Block` shape/state.
- `chunkbuf.rs`: Chunk buffer type and worldgen-to-buffer generator.
- `edit.rs`: EditStore + revision tracking for chunk rebuilds.
- `event.rs`: Event types, queue, scheduling hints/cause.
- `schem.rs` (+ `mcworld.rs` feature): Schematic loader, palette mapping to runtime blocks.
- `structure.rs`: Structure types, local edits, poses; uses Vector3 and simple math helpers.
- `camera.rs`, `player.rs`, `raycast.rs`, `shaders.rs`: App/renderer utilities.

## Proposed Workspace Layout

Introduce a Cargo workspace with the following crates:

- `geist-blocks`
  - Responsibility: Block/Material/Registry types and loaders.
  - Public API: `Block`, `MaterialId`, `MaterialCatalog`, `BlockRegistry`, `BlockRegistry::load_from_paths()`, `unknown_block_id_or_panic()`.
  - Depends on: `serde`, `toml`.
  - No Raylib.

- `geist-world`
  - Responsibility: World geometry/sizing, runtime worldgen sampling, and worldgen params I/O.
  - Public API: `World`, `WorldGenMode`, `GenCtx`, `block_at_runtime[_with]()`, `update_worldgen_params()`, dimensions helpers; `worldgen::{WorldGenParams, load_params_from_path}`.
  - Depends on: `geist-blocks`, `fastnoise-lite`, `serde`, `toml`.

- `geist-chunk`
  - Responsibility: Chunk backing buffer and interface.
  - Public API: `ChunkBuf`, `get_local()`, `get_world()`, `contains_world()`, `generate_chunk_buffer(world, cx, cz, reg)`.
  - Depends on: `geist-world`, `geist-blocks`.

- `geist-lighting`
  - Responsibility: In-chunk lighting computation and neighbor border management.
  - Public API: `LightGrid::compute_with_borders_buf()`, `LightBorders`, `NeighborBorders`, `LightingStore` (persist borders, dynamic emitters).
  - Depends on: `geist-chunk`, `geist-blocks`.

- `geist-mesh-cpu`
  - Responsibility: CPU meshing using greedy rectangles + lighting data.
  - Public API: `NeighborsLoaded`, `MeshBuild`, `ChunkMeshCPU`, `build_chunk_greedy_cpu_buf() -> (ChunkMeshCPU, Option<LightBorders>)`, `build_voxel_body_cpu_buf()`.
  - Depends on: `geist-chunk`, `geist-world`, `geist-lighting`, `geist-blocks`.
  - Note: Define small math/AABB locally or via `geist-geom` (see Decouplings); do not depend on Raylib.

- `geist-runtime`
  - Responsibility: Job lanes/queues/worker pools for chunk builds; drains CPU results.
  - Public API: `RebuildCause`, `BuildJob`, `JobOut`, `submit_build_job_edit/light/bg()`, `drain_worker_results()`, `queue_debug_counts()`, worker counts; `StructureBuildJob/StructureJobOut`.
  - Depends on: `geist-world`, `geist-lighting`, `geist-chunk`, `geist-mesh-cpu`, `geist-blocks`.
  - Excludes: All Raylib/GPU/texture concerns and file watching.

- `geist-structures`
  - Responsibility: Structure buffer + local edits + transforms.
  - Public API: `StructureId`, `Pose`, `Structure`, `StructureEditStore`, `rotate_yaw[_inv]()`.
  - Depends on: `geist-blocks`. Uses shared math types (see Decouplings).

- `geist-edit`
  - Responsibility: Persistent world edit store and revision tracking.
  - Public API: `EditStore` (get/set, snapshots, bump_region_around(), get_affected_chunks(), needs_rebuild()).
  - Depends on: `geist-blocks`.

- `geist-io`
  - Responsibility: External I/O for world data (schematics and optional Bedrock).
  - Public API: `schem::{load_any_schematic_apply_edits, find_unsupported_blocks_in_file, count_blocks_in_file}`.
  - Depends on: `geist-blocks`, `geist-edit`. Feature `mcworld` remains optional.

- `geist-render-raylib`
  - Responsibility: GPU-facing rendering utilities and resource management.
  - Public API: `ChunkRender`, shader wrappers, `TextureCache`, `upload_chunk_mesh(cpu, reg, rl, thread, cache) -> ChunkRender`, `rebind_textures()`, `draw_chunk_render()`, `drop_chunk_render()`; optional texture file-watching helpers.
  - Depends on: `raylib`, `geist-blocks`, `geist-mesh-cpu`.

- `geist-app` (binary)
  - Responsibility: Application wiring, UI/HUD, inputs/intents, event queue, scheduling/backpressure; calls into runtime + renderer.
  - Depends on: `geist-runtime`, `geist-render-raylib`, `geist-edit`, `geist-io`, `geist-blocks`, `geist-world`, `geist-lighting`.

### Dependency Direction (Acyclic)

```
geist-blocks
  → geist-world
    → geist-chunk
      → geist-lighting
        → geist-mesh-cpu
          → geist-runtime

geist-mesh-cpu → geist-render-raylib → geist-app

geist-edit → geist-io → geist-app

geist-runtime → geist-app
```

- Renderer depends on CPU mesh, not vice versa.
- App depends on runtime and renderer; runtime is engine-only.

## API Boundaries (Stabilize Early)

- `geist-mesh-cpu`
  - Types: `NeighborsLoaded`, `ChunkMeshCPU { bbox: Aabb, parts: HashMap<MaterialId, MeshBuild> }`
  - Fns: `build_chunk_greedy_cpu_buf(buf, Some(&lighting_store), world, edits_map, neighbors, cx, cz, reg)` and `build_voxel_body_cpu_buf(buf, ambient, reg)`

- `geist-runtime`
  - Types: `RebuildCause::{Edit, LightingBorder, StreamLoad}`, `BuildJob { cx, cz, neighbors, rev, job_id, chunk_edits, region_edits, prev_buf, cause }`, `JobOut { cpu, buf, light_borders, cx, cz, rev, job_id, cause }`
  - Fns: `submit_build_job_edit()`, `submit_build_job_light()`, `submit_build_job_bg()`, `drain_worker_results()`, `queue_debug_counts()`; `submit_structure_build_job()`, `drain_structure_results()`

- `geist-render-raylib`
  - Types: `ChunkRender`
  - Fns: `upload_chunk_mesh(&mut rl, &thread, &reg, &mut cache, &ChunkMeshCPU) -> ChunkRender`, `rebind_textures(changed_paths, cache, renders)`, `draw_chunk_render(rl, model)`, `drop_chunk_render(model)`

Keep these signatures minimal and data-oriented to avoid unnecessary coupling.

## Decouplings and Refactors

- Remove Raylib types from engine crates (meshing/structure):
  - Replace `raylib::prelude::Vector3`/`BoundingBox` with a small `geist-geom` or local types:
    - `pub struct Vec3 { pub x: f32, pub y: f32, pub z: f32 }`
    - `pub struct Aabb { pub min: Vec3, pub max: Vec3 }`
  - Implement conversion helpers in `geist-render-raylib`.

- Move GPU/texture logic out of `runtime.rs`:
  - File watching for textures and `TextureCache` → `geist-render-raylib`.
  - `renders` maps and rebind routines → `geist-render-raylib` and/or `geist-app`.
  - `runtime` focuses on submission lanes and CPU build results only.

- Split `mesher.rs` into:
  - CPU: in `geist-mesh-cpu` (returns `ChunkMeshCPU`, optionally `LightBorders`).
  - GPU: in `geist-render-raylib` (produces `ChunkRender` from `ChunkMeshCPU`).

- Keep `event.rs` local to app for now (or a small `geist-events` if needed). It depends on runtime/light/edit types—ensure it doesn’t drag in Raylib if extracted.

## File Mapping (Old → New)

- `src/blocks/*` → `crates/geist-blocks/src/*`
- `src/worldgen/*`, `src/voxel.rs` → `crates/geist-world/src/*`
- `src/chunkbuf.rs` → `crates/geist-chunk/src/lib.rs`
- `src/lighting.rs` → `crates/geist-lighting/src/lib.rs`
- `src/meshing_core.rs` → `crates/geist-mesh-cpu/src/meshing_core.rs`
- `src/mesher.rs`
  - CPU parts → `crates/geist-mesh-cpu/src/lib.rs`
  - GPU upload + `ChunkRender`/`TextureCache` → `crates/geist-render-raylib/src/lib.rs`
- `src/runtime.rs` → `crates/geist-runtime/src/lib.rs` (minus Raylib)
- `src/structure.rs` → `crates/geist-structures/src/lib.rs` (use shared math)
- `src/edit.rs` → `crates/geist-edit/src/lib.rs`
- `src/schem.rs` (+ `src/mcworld.rs`) → `crates/geist-io/src/*`
- Keep `src/app.rs`, `camera.rs`, `player.rs`, `raycast.rs`, `shaders.rs` in `geist-app` bin crate.

## Migration Plan (Incremental, Low-Churn)

1) Workspace Scaffolding
- Add `[workspace]` root and create empty crates with liberal `pub use` re-exports to keep the app compiling.

2) Extract Blocks + World
- Move `blocks/` into `geist-blocks`.
- Move `worldgen/` + `voxel.rs` into `geist-world`.
- Update imports in `chunkbuf.rs`, `mesher.rs`, `lighting.rs`.

3) Extract Chunk + Lighting
- Move `chunkbuf.rs` → `geist-chunk`.
- Move `lighting.rs` → `geist-lighting`.
- Fix users in mesher/runtime.

4) Introduce Engine Math
- Add `geist-geom` (or local types in `geist-mesh-cpu`) for `Vec3`/`Aabb`.
- Update `meshing_core.rs`, CPU mesher, and `structure.rs` to use new types.

5) Split Mesher (CPU vs GPU)
- Move CPU meshing to `geist-mesh-cpu` and keep signatures stable.
- Create `geist-render-raylib` with `ChunkRender`, shader helpers, `TextureCache`, and mesh-upload.

6) Slim Runtime
- Move remaining texture/file-watch/render bits out of `runtime.rs` into renderer/app.
- Leave lanes, job submission, result draining, debug counters.

7) Extract Edit + IO
- Move `edit.rs` → `geist-edit`.
- Move `schem.rs` (+ `mcworld.rs` feature) → `geist-io`.

8) Wire App to New APIs
- Update `geist-app` to depend on new crates.
- Keep UI/backpressure logic in app using `queue_debug_counts()`, intents size, etc.

9) Cleanup + Docs
- Remove deprecated paths and adapters.
- Update README and add crate READMEs.

### Parallelization (Who Can Work Now)

- Team A: `geist-blocks`, `geist-world`, `geist-chunk` extraction.
- Team B: `geist-lighting` extraction and polishing.
- Team C: CPU meshing (`geist-mesh-cpu`) + `meshing_core` integration.
- Team D: Raylib renderer crate (GPU upload, shaders, texture reload/rebind).
- Team E: `geist-runtime` refinement (lanes/counters already implemented).
- Team F: `geist-app` intents/backpressure/HUD using new debug counters.
- Team G: `geist-io` (schem/mcworld path) + `geist-edit` wiring.

## Runtime Notes (Existing Design Assumptions)

- Lanes: Three submission lanes (`edit`, `light`, `bg`) with dedicated worker pools; `edit` is exclusive.
- Worker split: ensure at least one edit worker; keep a light worker when possible; rest are background.
- Light assistance: BG workers can help with `light` when BG is idle; no fallback into `edit`.
- Counters/metrics: per-lane queued and inflight atomics; expose worker counts and `queue_debug_counts()`.
- Causes: `BuildJob`/`JobOut` carry `RebuildCause` to decrement inflight correctly.
- App-side backpressure: App budgets per lane from `(q + inflight)` vs `(workers + 1)` to avoid FIFO overfill.

These remain valid and should be preserved across the refactor, since they live in `geist-runtime` and the app.

## Testing Strategy

- `geist-blocks`: load/parse tests for materials/blocks TOML; state property mapping.
- `geist-world`: deterministic noise snapshots with fixed seeds; config parse/roundtrip.
- `geist-chunk`: buffer indexing and `contains_world()` correctness.
- `geist-lighting`: unit tests for `LightGrid` propagation and `LightBorders` seam transfer.
- `geist-mesh-cpu`: golden tests on simple buffers (cubes/slabs/stairs) and face counts; neighbor occlusion.
- `geist-runtime`: lane selection and inflight accounting (mock worker thread that echoes jobs).
- `geist-render-raylib`: limited—it’s integration-heavy; add smoke tests guarded by feature flag if needed.
- `geist-io`: palette mapping from `palette_map.toml`, unsupported list correctness on fixtures.

## Features and Configuration

- `geist-io`: `mcworld` feature (optional Bedrock support), aligns with current top-level features.
- `geist-render-raylib`: `watch_textures` optional helper; avoid mandatory threading when not used.
- `geist-runtime`: consider a cfg to tune worker split defaults; later expose runtime tuning knobs via app.
- `geist-geom` (optional): could be replaced by `glam` if we prefer a standard math crate.

## Versioning and Stability

- Stabilize the following first to reduce churn:
  - `BuildJob`/`JobOut` (and `RebuildCause`).
  - `ChunkMeshCPU` (`Aabb`, `parts` layout) and `NeighborsLoaded`.
  - Renderer’s `upload_chunk_mesh()` shape.
- Use SemVer for each crate. Start at `0.1.x` and avoid breaking changes during the migration unless necessary.

## Example Flows

- CPU Mesh Build (app-driven):
  1) App creates a `BuildJob` with edits snapshots and neighbors mask.
  2) `geist-runtime` routes job by cause to lane; worker builds `ChunkBuf`, applies edits, computes lighting and mesh via `geist-mesh-cpu`.
  3) `JobOut` drained by app; app updates lighting borders store, then calls `upload_chunk_mesh()` (renderer) to produce `ChunkRender`.

- Texture Reload:
  - Renderer watches `assets/blocks/**`. On change, reloads `Texture2D` into `TextureCache` and rebinds materials on `ChunkRender`s. No runtime involvement.

## Risks and Mitigations

- Raylib bleed into engine crates: Introduce `geist-geom` early and convert inside renderer.
- Cycles via event types: Keep `event.rs` in `geist-app` for now; extract only if needed and ensure it only depends on engine types.
- Build breakage during move: Use re-export “facade” modules temporarily (e.g., `pub use geist_mesh_cpu::*;`) to maintain paths until all callsites are updated.
- Performance regressions: Avoid extra allocations when moving between crates; pass references/Arcs as today.

## Workspace Setup Outline

- Root `Cargo.toml`:

```toml
[workspace]
members = [
  "crates/geist-blocks",
  "crates/geist-world",
  "crates/geist-chunk",
  "crates/geist-lighting",
  "crates/geist-mesh-cpu",
  "crates/geist-runtime",
  "crates/geist-structures",
  "crates/geist-edit",
  "crates/geist-io",
  "crates/geist-render-raylib",
  "geist-app",
]
resolver = "2"
```

- Each crate `Cargo.toml` declares minimal dependencies and uses `edition = "2021"` or `2024` to match root.

## Work Items Checklist

- [ ] Create workspace and crate skeletons.
- [ ] Extract `geist-blocks` and `geist-world`; update imports.
- [ ] Extract `geist-chunk` and `geist-lighting`; update imports.
- [ ] Add `geist-geom` or local math types; switch engine crates off Raylib types.
- [ ] Split mesher into CPU (`geist-mesh-cpu`) and GPU (`geist-render-raylib`).
- [ ] Move texture watchers and rebind logic into renderer.
- [ ] Slim `geist-runtime` to engine-only.
- [ ] Extract `geist-edit` and `geist-io`.
- [ ] Wire `geist-app` to new crates. Ensure HUD/backpressure reads from `queue_debug_counts()`.
- [ ] Update README and add crate READMEs.

## Future Opportunities

- Central priority queue for light lane (or global), enabling dedup/cancel/fairness without relying solely on app-side backpressure.
- Runtime-tunable worker allocation per lane (config/UI control).
- Additional HUD metrics (per-lane throughput, age of oldest task, structure build perf).
- Optional ECS boundary experiment once data types are stabilized (non-goal for this split).
- Replace homegrown `EventQueue` with a simpler tickless scheduler once backpressure is stable.

---

This plan aims to unblock parallel ownership while respecting current performance and architecture. If you want, we can stage the workspace scaffolding and migrate the first 2–3 crates to validate the approach before proceeding further.


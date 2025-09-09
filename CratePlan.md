# CratePlan: Splitting Geist into a Cargo Workspace

This plan tracks the migration of Geist from a single crate to a multi‑crate Cargo workspace with clear engine vs. renderer boundaries, stable APIs, and incremental checkpoints.

## Current Status

- Completed
  - Phase 0: Workspace scaffolded; crates created under `crates/` for planned packages; resolver = "2".
  - Phase 1: Blocks + World extracted.
    - `blocks/` moved to `crates/geist-blocks` (config, material, registry, types).
    - `voxel.rs` and `worldgen/` moved to `crates/geist-world` with worldgen parity (biomes, trees, caves, features) and `load_params_from_path`.
  - Phase 2: Chunk + Lighting extracted.
    - `crates/geist-chunk` provides `ChunkBuf` and `generate_chunk_buffer()`.
    - `crates/geist-lighting` provides `LightGrid`, `LightBorders`, `NeighborBorders`, `LightingStore` (includes `sample_face_local`).
  - Phase 3: Engine math introduced; Raylib removed from CPU engine code.
    - `crates/geist-geom`: `Vec3` (ops) and `Aabb`.
    - `crates/geist-render-raylib`: conversion helpers `conv::{vec3_to_rl, vec3_from_rl, aabb_to_rl, aabb_from_rl}`.
    - `src/structure.rs`: uses `geist_geom::Vec3` for `Pose` and helpers.
    - `src/mesher.rs`: `ChunkMeshCPU.bbox` now `geist_geom::Aabb` and converted at upload boundary.
    - App sites updated with explicit conversions.
  - Phase 4: Split mesher CPU/GPU.
    - Added `crates/geist-mesh-cpu` and moved CPU mesher there:
      - Types: `Face`, `MeshBuild`, `NeighborsLoaded`, `ChunkMeshCPU`.
      - Builders: `build_mesh_core`, `build_chunk_greedy_cpu_buf`, `build_voxel_body_cpu_buf`.
      - Tables: `microgrid_tables` (2x2x2 occupancy and 2x2 rect greedies).
      - No Raylib usage; now uses `geist-geom::Vec3/Aabb` exclusively.
    - Expanded `crates/geist-render-raylib` with GPU parts:
      - `TextureCache`, `LeavesShader`, `FogShader`.
      - `ChunkRender` and `upload_chunk_mesh` (Raylib mesh upload + texture binding).
      - Kept `conv` helpers.
    - Root shims for stable imports:
      - `src/mesher.rs` now re-exports CPU (`geist-mesh-cpu`) and GPU (`geist-render-raylib`) APIs.
      - `src/shaders.rs` and `src/texture_cache.rs` re-export from `geist-render-raylib`.
      - Removed `src/microgrid_tables.rs`; removed `mod microgrid_tables;` from `src/main.rs`.
    - Workspace builds with `cargo check` (warnings remain; behavior unchanged).
  - Shims in root keep paths stable (`src/blocks/`, `src/worldgen/`, `src/voxel.rs`).
  - `cargo check` passes for the workspace.

- Pending (next phases)
  - Phase 5: Slim runtime by moving GPU/texture/file‑watch to renderer/app; keep job lanes and results in `geist-runtime`.
  - Phase 6: Extract edits, IO, structures into `geist-edit`, `geist-io`, `geist-structures`.
  - Phase 7: Wire app fully to new crates, remove shims, clean imports.

## Workspace Overview

- geist-geom
  - Responsibility: Minimal math types for engine crates; no Raylib.
  - API: `Vec3`, `Aabb`.

- geist-blocks
  - Responsibility: Blocks/materials/registry/config.
  - API: `Block`, `MaterialId`, `MaterialCatalog`, `BlockRegistry` (+ loaders).

- geist-world
  - Responsibility: World sizing, sampling, worldgen params and config I/O.
  - API: `World`, `worldgen::{...}`, `load_params_from_path`.

- geist-chunk
  - Responsibility: Chunk buffer and worldgen helpers.
  - API: `ChunkBuf`, `generate_chunk_buffer`.

- geist-lighting
  - Responsibility: In‑chunk lighting; neighbor border planes; dynamic emitters.
  - API: `LightGrid::{compute_with_borders_buf, sample_face_local}`, `LightBorders`, `LightingStore`.

- geist-mesh-cpu (planned)
  - Responsibility: CPU meshing (no Raylib types), microgrid tables.
  - API: `NeighborsLoaded`, `MeshBuild`, `ChunkMeshCPU`, `build_*` functions.

- geist-render-raylib
  - Responsibility: GPU upload, shaders, textures, conversions with Raylib.
  - API: `ChunkRender`, `upload_chunk_mesh`, `conv::{...}`.

- geist-runtime (planned slim)
  - Responsibility: Job lanes and worker pools; drain CPU results.
  - API: `BuildJob`, `JobOut`, `submit_*`, `drain_*`, counters.

- geist-structures (planned)
  - Responsibility: Structure buffers, transforms, local edits (engine‑only types).
  - API: `StructureId`, `Pose`, `Structure`, `rotate_yaw[_inv]`.

- geist-edit (planned)
  - Responsibility: Persistent world edits + revisions.
  - API: `EditStore`.

- geist-io (planned)
  - Responsibility: Import/export; schematics and Bedrock (feature‑gated).
  - API: `schem::{...}`, mcworld (feature).

## Dependency Direction (condensed)

`geist-geom`
  → `geist-blocks` → `geist-world` → `geist-chunk` → `geist-lighting` → `geist-mesh-cpu` → `geist-runtime` → app

`geist-render-raylib` depends on `raylib`, `geist-mesh-cpu`, `geist-blocks`, and converts to/from `geist-geom`.

## Migration Plan (Phased)

Phase 0: Workspace scaffold
- Create crates and activate resolver = "2"; add temporary re‑exports as needed.

Phase 1: Blocks + World
- Move `src/blocks/` → `geist-blocks`.
- Move `src/worldgen/` and `src/voxel.rs` → `geist-world`.

Phase 2: Chunk + Lighting
- Move `src/chunkbuf.rs` → `geist-chunk`.
- Move `src/lighting.rs` → `geist-lighting`.

Phase 3: Engine math
- Add `geist-geom` and refactor engine code to use it instead of Raylib types.
- Add Raylib↔geom conversions in `geist-render-raylib`.

Phase 4: Split Mesher (next)
- Done in this change.

Phase 5: Slim Runtime (next)
- Move GPU/texture/file‑watch out of runtime; keep lanes, queues, results only.

Phase 6: Edits, IO, Structures (next)
- Move `edit.rs` → `geist-edit`, `schem.rs`/`mcworld.rs` → `geist-io`, `structure.rs` → `geist-structures`.

Phase 7: App wiring and cleanup (next)
- Remove shims; update imports; finalize crate boundaries.

## Temporary Shims (remove in Phase 7)

- `src/blocks/mod.rs` re‑exports `geist-blocks`.
- `src/worldgen/mod.rs` re‑exports `geist-world::worldgen`.
- `src/voxel.rs` re‑exports `geist-world::voxel`.

## Validation Checklist (per phase)

- Build: `cargo check` / `cargo build --workspace` passes.
- Visual parity: worldgen/caves/trees/features intact.
- Engine crates have no `raylib` dependency.
- Imports: root `src/` use new crates (or shims until Phase 7).

## Build/Run Commands

- Build all: `cargo build --workspace`
- Run app: `cargo run` (current root bin is `geist`)
- With Bedrock: `cargo run -F mcworld`

## Guidelines

- No Raylib types in engine crates.
- Keep public surfaces minimal; document with rustdoc.
- Avoid cyclic deps; push shared helpers down (e.g., `geist-geom`).
- Prefer slices over owned `Vec` in hot paths when possible.

## Risks & Mitigations

- Raylib leakage into engine crates → introduce `geist-geom` first; convert only at renderer boundary.
- Build breakage from large moves → use phased PRs and temporary adapters.
- Feature flag drift → centralize feature forwarding in the workspace; keep `mcworld` in `geist-io` and forward.

## Acceptance Criteria

- `cargo build --workspace` succeeds.
- App renders the same scene before/after refactors.
- No engine crate depends on `raylib`.
- `mcworld` feature works via `geist-io`.

## Appendix: Completed Phase Details

- Phase 1
  - `geist-blocks` and `geist-world` created; worldgen parity retained; shims in root.

- Phase 2
  - `geist-chunk` and `geist-lighting` created; mesher/light APIs preserved; borders handled.

- Phase 3
  - `geist-geom` added with `Vec3`/`Aabb`; engine code refactored (structures + mesher bbox).
  - `geist-render-raylib` gained conversion helpers for clean boundaries.

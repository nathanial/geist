# CratePlan: Splitting Geist into a Cargo Workspace

This document describes how to split the current single‑crate Geist project into a multi‑crate Cargo workspace with clear ownership boundaries, minimal coupling, and a safe incremental migration path.

Status Summary

- Completed
  - Phase 0: Workspace scaffolded; crates created under `crates/` for all planned packages.
  - Phase 1: `blocks/` moved to `geist-blocks`; `voxel.rs` and `worldgen/` moved to `geist-world`.
  - Worldgen parity restored in `geist-world`: biomes, cave carving, features, tree placement, and config loader (`load_params_from_path`).
  - Root app compiles via compatibility shims (`src/blocks/`, `src/worldgen/`, `src/voxel.rs` re-export new crates).
  - `cargo check` passes for the workspace.

- Temporary shims (to remove in Phase 7)
  - `src/blocks/mod.rs` re-exports `geist-blocks` (and preserves `crate::blocks::{registry,material,config,types}` paths).
  - `src/worldgen/mod.rs` re-exports `geist-world::worldgen`.
  - `src/voxel.rs` re-exports `geist-world::voxel`.

- Not yet done (future phases)
  - Extract chunk + lighting crates.
  - Split mesher CPU/GPU and add `geist-render-raylib` logic.
  - Slim `runtime` (move GPU bits out).
  - Extract edits, IO, and structures.
  - Final import cleanup and shim removal.

Goals

- Separate CPU “engine” from GPU/Raylib concerns.
- Keep crate APIs small and stable to enable parallel work.
- Maintain performance: avoid extra copies, keep hot paths lean.
- Incremental adoption with compiling checkpoints after each phase.

Overview of Today

- Rendering/app: `app.rs`, `camera.rs`, `player.rs`, `raycast.rs`, `shaders.rs`, `texture_cache.rs`.
- Engine/core: `voxel.rs`, `worldgen/`, `chunkbuf.rs`, `lighting.rs`, `mesher.rs`, `microgrid_tables.rs`, `structure.rs`, `edit.rs`, `runtime.rs`, `event.rs`, `gamestate.rs`.
- I/O and formats: `schem.rs`, `mcworld.rs` (feature `mcworld`).

Proposed Workspace

Create a `crates/` directory with the following crates. The dependency arrows indicate allowed directions only (acyclic).

- geist-geom
  - Responsibility: Minimal math types used in engine crates.
  - Public API: `Vec3`, `Aabb`, small helpers and conversions.
  - Depends on: nothing. No Raylib.

- geist-blocks
  - Responsibility: Block/Material/Registry types and config loading.
  - Public API: `Block`, `MaterialId`, `MaterialCatalog`, `BlockRegistry`, `BlockRegistry::load_from_paths()`.
  - Depends on: `serde`, `toml`.

- geist-world
  - Responsibility: World sizing, runtime sampling, worldgen params.
  - Public API: `World`, worldgen config I/O, sampling helpers.
  - Depends on: `geist-blocks`, `fastnoise-lite`, `serde`, `toml`.

- geist-chunk
  - Responsibility: Chunk buffer type + utilities.
  - Public API: `ChunkBuf`, `get_local/world`, `contains_world`, `generate_chunk_buffer(...)`.
  - Depends on: `geist-world`, `geist-blocks`.

- geist-lighting
  - Responsibility: In‑chunk lighting and neighbor border planes.
  - Public API: `LightGrid::compute_with_borders_buf`, `LightBorders`, `LightingStore`.
  - Depends on: `geist-chunk`, `geist-blocks`.

- geist-mesh-cpu
  - Responsibility: CPU meshing only (no Raylib types).
  - Public API: `NeighborsLoaded`, `MeshBuild`, `ChunkMeshCPU`, `build_chunk_greedy_cpu_buf(...)`, `build_voxel_body_cpu_buf(...)`.
  - Depends on: `geist-chunk`, `geist-world`, `geist-lighting`, `geist-blocks`, `geist-geom`.
  - Includes: `microgrid_tables.rs` and any meshing helpers.

- geist-runtime
  - Responsibility: Job lanes/queues/worker pools; drains CPU results.
  - Public API: `BuildJob`, `JobOut`, `submit_*`, `drain_worker_results`, queue counters; `StructureBuildJob/StructureJobOut`.
  - Depends on: `geist-world`, `geist-lighting`, `geist-chunk`, `geist-mesh-cpu`, `geist-blocks`.
  - Excludes: Raylib/texture/shader management.

- geist-structures
  - Responsibility: Structure buffers, transforms, local edits.
  - Public API: `StructureId`, `Pose`, `Structure`, `rotate_yaw[_inv]`.
  - Depends on: `geist-blocks`, `geist-geom`.

- geist-edit
  - Responsibility: Persistent world edit store + revisions.
  - Public API: `EditStore` and helpers for affected regions.
  - Depends on: `geist-blocks`.

- geist-io
  - Responsibility: Data import/export (schematics, optional Bedrock).
  - Public API: `schem::{load_any_schematic_apply_edits, find_unsupported_blocks_in_file, count_blocks_in_file}`.
  - Depends on: `geist-blocks`, `geist-edit`; feature `mcworld` pulls `bedrock-hematite-nbt` and `zip`.

- geist-render-raylib
  - Responsibility: GPU‑facing rendering utils and resources.
  - Public API: `ChunkRender`, shader wrappers, `TextureCache`, `upload_chunk_mesh(cpu, ...) -> ChunkRender`.
  - Depends on: `raylib`, `geist-mesh-cpu`, `geist-blocks`.

- geist-app (bin)
  - Responsibility: CLI, window loop, input, HUD, orchestration.
  - Depends on: `geist-runtime`, `geist-render-raylib`, `geist-io`, `geist-structures`, `geist-edit`, `clap`, `log`.

Dependency Graph (condensed)

geist-geom
  ↓
geist-blocks → geist-world → geist-chunk → geist-lighting → geist-mesh-cpu → geist-runtime → geist-app
                                                              ↘ geist-render-raylib → geist-app
geist-edit → geist-io ─────────────────────────────────────────────────────────────→ geist-app
geist-structures ─────────────────────────────────────────────────────────────────→ geist-app

Feature Flags

- `mcworld` moves from root crate to `geist-io` as optional, and is re‑exported at the workspace root for convenience:
  - Root workspace `[features]`: `mcworld = ["geist-io/mcworld"]`.
  - Only `geist-io` lists `bedrock-hematite-nbt` and `zip` as `optional = true`.

Old → New File Mapping

- `src/blocks/*` → `crates/geist-blocks/src/*`
- `src/worldgen/*`, `src/voxel.rs` → `crates/geist-world/src/*`
- `src/chunkbuf.rs` → `crates/geist-chunk/src/lib.rs`
- `src/lighting.rs` → `crates/geist-lighting/src/lib.rs`
- `src/microgrid_tables.rs` → `crates/geist-mesh-cpu/src/microgrid_tables.rs`
- `src/mesher.rs`
  - CPU parts → `crates/geist-mesh-cpu/src/lib.rs`
  - Raylib model upload + `ChunkRender` → `crates/geist-render-raylib/src/lib.rs`
- `src/runtime.rs` → `crates/geist-runtime/src/lib.rs` (minus Raylib/texture/shader/file‑watch)
- `src/structure.rs` → `crates/geist-structures/src/lib.rs`
- `src/edit.rs` → `crates/geist-edit/src/lib.rs`
- `src/schem.rs`, `src/mcworld.rs` → `crates/geist-io/src/*`
- App‑only stays in `geist-app`: `app.rs`, `camera.rs`, `player.rs`, `raycast.rs`, `shaders.rs`, `texture_cache.rs`, `event.rs`, `gamestate.rs`, `snapshowcase.rs`.

Workspace Layout

.
├─ Cargo.toml (workspace root)
├─ crates/
│  ├─ geist-geom/
│  ├─ geist-blocks/
│  ├─ geist-world/
│  ├─ geist-chunk/
│  ├─ geist-lighting/
│  ├─ geist-mesh-cpu/
│  ├─ geist-runtime/
│  ├─ geist-structures/
│  ├─ geist-edit/
│  ├─ geist-io/
│  └─ geist-render-raylib/
└─ src/ (geist-app bin crate)

Root Cargo.toml (sketch)

```toml
[workspace]
members = [
  "crates/geist-geom",
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
  "." # geist-app
]
resolver = "2"

[workspace.package]
edition = "2024" # keep current edition

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
toml = "0.8"
fastnoise-lite = "1.1"
log = "0.4"
clap = { version = "4.5", features = ["derive"] }
raylib = "5.5.1"
# etc.

[features]
default = []
mcworld = ["geist-io/mcworld"]
```

Example crate manifest (geist-mesh-cpu)

```toml
[package]
name = "geist-mesh-cpu"
version = "0.1.0"
edition = "2021"

[dependencies]
geist-blocks = { path = "../geist-blocks" }
geist-world = { path = "../geist-world" }
geist-chunk = { path = "../geist-chunk" }
geist-lighting = { path = "../geist-lighting" }
geist-geom = { path = "../geist-geom" }
```

Incremental Migration Plan

Phase 0: Scaffold the workspace (no code moves)
- Convert root to a `[workspace]` and create empty crates with stubs.
- In each new crate, add temporary `pub use` re‑exports pointing back into the old modules to keep the binary compiling while we move code.

Phase 1: Extract Blocks + World
- Move `src/blocks/` → `geist-blocks`.
- Move `src/worldgen/` and `src/voxel.rs` → `geist-world`.
- Update imports where needed; keep re‑exports during the transition.

Done in repo:
- `geist-blocks` now contains `config`, `material`, `registry`, `types` (with re-exports in `lib.rs`).
- `geist-world` now contains `worldgen` and `voxel` with full parity features (biomes, trees, caves, features) and `load_params_from_path`.
- Root app re-exports preserve old paths; app worldgen config reload continues to work.

Phase 2: Extract Chunk + Lighting
- Move `src/chunkbuf.rs` → `geist-chunk`.
- Move `src/lighting.rs` → `geist-lighting`.
- Update users: mesher, runtime, app.

Done in repo:
- `geist-chunk` now provides `ChunkBuf` and `generate_chunk_buffer()`; root `src/chunkbuf.rs` re-exports it.
- `geist-lighting` now provides `LightGrid`, `LightBorders`, `NeighborBorders`, `LightingStore` with the same APIs (including `sample_face_local` used by mesher); root `src/lighting.rs` re-exports it.
- Root `Cargo.toml` depends on `geist-chunk` and `geist-lighting`.
- Workspace builds (`cargo check`): OK.

Notes:
- `LightGrid::sample_face_local` restored so mesher compiles unchanged.
- Neighbor border handling is preserved; dynamic emitters remain in `LightingStore`.

Phase 3: Introduce engine math (remove Raylib from engine types)
- Add `geist-geom` with `Vec3` and `Aabb` equivalents.
- Replace `raylib::Vector3/BoundingBox` usages inside CPU engine code (meshing/structure) with `geist-geom` types.
- Keep conversions inside `geist-render-raylib` when uploading meshes.

Notes:
- `geist-geom` crate already exists with initial `Vec3`/`Aabb` types; integration pending.

Phase 4: Split Mesher
- Move CPU meshing into `geist-mesh-cpu` (including `microgrid_tables.rs`).
- Add `geist-render-raylib` that owns `ChunkRender`, shader wrappers, `TextureCache`, and `upload_chunk_mesh`.
- Remove Raylib model types from the CPU path.

Proposed concrete steps:
- Extract CPU-only meshing structures (`MeshBuild`, `ChunkMeshCPU`, `NeighborsLoaded`) into `geist-mesh-cpu`.
- Keep `microgrid_tables.rs` with static tables in `geist-mesh-cpu`.
- Introduce `geist-render-raylib` for `ChunkRender`, shader wrappers, texture cache and mesh upload, plus conversions from `geist-geom`.

Phase 5: Slim Runtime
- Move GPU/texture/file‑watch logic out of `runtime.rs` into `geist-render-raylib` and/or the app.
- Keep only job submission/draining, lane counts, and result types in `geist-runtime`.

Phase 6: Extract Edits + IO + Structures
- Move `src/edit.rs` → `geist-edit`.
- Move `src/schem.rs` and `src/mcworld.rs` → `geist-io` (feature‑gated as today).
- Move `src/structure.rs` → `geist-structures` (depend on `geist-geom`).

Phase 7: Wire the App
- Update `geist-app` to depend on new crates and remove all temporary re‑exports.
- Keep CLI and HUD unchanged; adopt new crate APIs for queue sizes and debug counters where applicable.

Validation checkpoints (per phase)

- Build: `cargo check`/`cargo build --workspace` must pass.
- Smoke run: `cargo run -- run` should render world parity (biomes, trees, caves intact).
- Imports: root `src/` should reference new crates or shims only; remove shims at Phase 7.


Build/Run After Split

- Build everything: `cargo build --workspace`
- Run the app: `cargo run -p geist-app -- run [args...]`
- With Bedrock support: `cargo run -F mcworld -p geist-app -- run ...`

API and Code Style Guidelines

- Crate boundaries are hard: no Raylib types in engine crates (`blocks/world/chunk/lighting/mesh/runtime/edit/structures`).
- Keep public surfaces minimal and document them with rustdoc comments.
- Prefer `&[T]`/`&mut [T]` over owned `Vec<T>` in hot paths; return borrowable views when possible.
- Avoid cyclic deps; if two crates need shared helpers, move them to `geist-geom` or a tiny util inside the lowest layer.
- Use `#[deny(missing_docs)]` and `#[warn(clippy::all)]` in new crates after initial migration.

Risks and Mitigations

- Raylib leakage into engine crates: mitigate by introducing `geist-geom` first and converting at the edges.
- Large moves breaking the build: mitigate with phased PRs and temporary `pub use` adapters to preserve paths while callers are updated.
- Feature flag drift: centralize feature forwarding in the root workspace; keep `mcworld` only in `geist-io` and re‑export at root.

Acceptance Criteria

- The workspace builds with `cargo build --workspace`.
- `cargo run -p geist-app` renders the same world before/after the split.
- No engine crate depends on `raylib`.
- Root `mcworld` feature forwards to `geist-io/mcworld` and works as it does today.

Optional Follow‑ups

- Consider a small `geist-events` crate if `event.rs` becomes a reusable boundary, otherwise keep it in the app.
- Add crate‑level README files summarizing each module’s purpose and public API.
- Add CI jobs: `cargo fmt --all`, `cargo clippy --workspace -D warnings`, and a minimal run smoke test.

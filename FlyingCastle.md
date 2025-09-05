# Flying Castle — Design Brainstorm (code-aware)

Goals:

- A dynamically sized voxel “castle” that moves through the world (e.g., a slow loop around origin).
- Players can hop on, edit it live, and eventually steer it.
- Reuse as much of the existing chunk/mesher/runtime logic as is practical, without forcing full rebuilds while the castle moves.

This doc references concrete types and functions in the repo to anchor the ideas:
- Meshing: `mesher::build_chunk_greedy_cpu_buf`, `upload_chunk_mesh`, `NeighborsLoaded` (src/mesher.rs)
- CPU buffers: `chunkbuf::ChunkBuf` (src/chunkbuf.rs)
- Edits: `edit::EditStore` (src/edit.rs)
- Runtime workers: `runtime::{BuildJob, Runtime}` (src/runtime.rs)
- App & event loop: `app::App`, `event::Event` (src/app.rs, src/event.rs)
- Lighting: `lighting::{LightGrid, LightingStore}` (src/lighting.rs)

---

## Current Architecture Constraints

- Chunk pipeline builds world-fixed meshes with vertices already in world space. `build_chunk_greedy_cpu_buf` positions faces using `base_x = buf.cx * buf.sx` and `base_z = buf.cz * buf.sz`, then the renderer draws at `(0,0,0)`.
- Cross-chunk face culling samples neighbors via `is_occluder(...)` using
  - a `neighbors` mask (only occlude if neighbor chunk is loaded), and
  - a region edits overlay (`region_edits`) to avoid seam artifacts when neighbors have unbuilt edits.
- Lighting is computed via `LightGrid::compute_with_borders_buf(buf, store)` and requires a `LightingStore`. The mesher currently returns `None` if lighting is not provided.
- Editing uses `EditStore` keyed by world-chunk and world coordinates; the app’s sampler consults `edits > loaded buffers > world` for collisions and raycasts.

Implication for a moving castle:
- Baking absolute coordinates into mesh vertices means we would need to rebuild on every movement tick — not viable.
- We need a mesh in local coordinates and draw-time transforms (translation/yaw) to move it without rebuilds.
- The castle’s editing and collision must integrate into the app’s sampling/raycast closures alongside the world.
- For lighting, we either add a simple local lighting path or extend the mesher to work without `LightingStore`.

---

## Approach A (Recommended): Local Voxel Body with Draw Transform

Treat the castle as a voxel “structure” with its own local grid, separate from the world chunk grid. Mesh it in local coords, then draw it with a translation/rotation. No rebuild when it moves; rebuild only on edits.

Key pieces:

- Data model (new module `structure.rs`):
  - `Structure { id, dims: (sx,sy,sz), blocks: Vec<Block>, edits: StructureEditStore, pose: Pose { pos: Vector3, yaw: f32 }, dirty_rev: u64, built_rev: u64 }`
  - `StructureEditStore`: same spirit as `EditStore` but indexed by local `(lx,ly,lz)` (no chunking). Provide `snapshot_all()` and `get(lx,ly,lz)`.
  - Optional: partition very large structures into micro-chunks later (see Approach C).

- Meshing path (extend mesher without breaking chunks):
  - Add `mesher::build_voxel_body_cpu_buf(buf: &ChunkBuf, ambient: u8) -> ChunkMeshCPU` that:
    - Emits vertices in local coordinates (no `base_x/base_z` offset).
    - Uses a simple lighting model (e.g., constant ambient, plus local block emission from `Block::emission()`), skipping `LightingStore` and neighbor borders.
    - Reuses most of the greedy face stitching and `FaceMaterial` path.
  - Alternatively, parameterize `build_chunk_greedy_cpu_buf` to accept a “lighting provider” trait and a position mode: `{WorldSpace|LocalSpace}`. For initial prototyping, a separate function is simpler.

- Rendering integration:
  - Create `StructureRender { id, parts: Vec<(FaceMaterial, Model)>, bbox_local: BoundingBox }` analogous to `ChunkRender`, but vertices are local (0..sx, 0..sz).
  - Keep a `Runtime`-owned map `structures: HashMap<StructureId, StructureRender>` and draw with transform:
    - In `App::render`, after drawing chunk renders, iterate structures and call `draw_model(model, structure.pose.pos, 1.0, ...)` for each part (and apply yaw via a per-model transform if desired; otherwise stick to translation first).

- Editing integration:
  - Raycast: run two raycasts and choose the closer hit.
    - World raycast (current code): builds sampler `edits > chunk buf > world` and calls `raycast_first_hit_with_face`.
    - Structure raycast: transform `(origin, dir)` into structure-local space: `local_origin = world_origin - structure.pos; local_dir = rotate_inverse_yaw(dir)`; then call `raycast_first_hit_with_face` with a `is_solid_local(lx,ly,lz)` closure sampling `StructureEditStore` overlay then `blocks`.
    - Compare travel distance/steps and select the nearer. Emit `Event::StructureBlockPlaced/Removed { id, lx,ly,lz, block }` or the existing world events.
  - On a structure edit:
    - Update `StructureEditStore`, bump `dirty_rev`, and queue a structure rebuild job (see next).

- Build/runtime integration:
  - Add a lightweight `StructureBuildJob { id, rev, chunkbuf_like: ChunkBuf }` sent to the worker threads or a separate single-threaded queue.
    - Create a `ChunkBuf` from the structure’s current local `blocks` (dims arbitrary), set `cx=0, cz=0` to keep local origin for meshing.
    - Apply `StructureEditStore::snapshot_all()` to the local `ChunkBuf` before meshing (same pattern as `chunk_edits`).
    - Call `build_voxel_body_cpu_buf` and then `upload_chunk_mesh`.
    - Store `StructureRender` in `Runtime.structures` on completion and update `built_rev`.

- Collision integration:
  - In `App::handle_event::MovementRequested`, extend the collision sampler closure to test the castle first:
    - Transform `(wx,wy,wz)` into structure-local; if inside bounds, sample `StructureEditStore > structure.blocks` and return that block if solid.
    - Otherwise fall back to the existing world sampler.
  - Optional (for boarding stability): add “platform velocity” when the walker stands on the castle floor:
    - Detect if walker’s last resolved collision normal is +Y and contact was with a structure cell.
    - Add the castle’s per-tick displacement to the walker’s position/velocity to avoid drift.

- Movement and steering:
  - Start with a deterministic path (slow circle): in `App::step`, update `structure.pose.pos.xz` by a small angle every tick.
  - For steering, add input → emit `Event::StructureSteer { id, yaw_delta, thrust }` and integrate velocity with a simple damping model.

- Lighting options (minimal viable first):
  - Phase 1: constant ambient brightness (e.g., 180), plus `Block::emission()` for glowstones/beacons.
  - Phase 2: local flood-fill within the structure, but still independent from world’s `LightingStore`.
  - Phase 3 (optional, complex): sample world `LightingStore` at the castle’s projected footprint for external influence; probably not worth it initially.

Pros:
- Zero rebuild cost on movement; only edits trigger a rebuild.
- Isolated, incremental changes to mesher and runtime; no impact to world chunk streaming.
- Clean mental model: world is static chunk grid; castle is a dynamic, transformed voxel body.

Cons:
- Lighting is separate/simplified vs. the world.
- Cross-body occlusion with the world is not considered (castle always renders its exterior faces). That’s acceptable for a first pass.

---

## Approach B: Virtual World Chunks (reuse existing pipeline end-to-end)

Map the castle volume onto reserved “virtual” chunk coordinates and reuse the exact world chunk path (jobs, lighting, region edits), then draw its meshes with a translation.

Sketch:
- Create faux `(cx,cz)` for the castle (e.g., negative space or a separate space) and build with `build_chunk_greedy_cpu_buf` using the global `LightingStore`.
- To move the castle, you’d normally need to rebuild (vertices are world-space). To avoid rebuilds, you’d need to change the pipeline so meshes are authored in local coords and drawn with transforms — which brings you back to Approach A.

Pros:
- Maximum reuse of existing logic (lighting, neighbor masks, region edits), if you accept rebuild-on-move.

Cons:
- Rebuild on every movement tick is a non-starter.
- Mixing “virtual” coordinates into neighbor/lighting systems will be fragile.

Conclusion: Not recommended unless the castle is stationary.

---

## Approach C: Micro‑Chunked Structure + Region Edits Overlay

If castles become large, partition the castle into a small local chunk grid (e.g., 16×16×16 micro-chunks). Each micro-chunk:
- Has its own `ChunkBuf` and mesh in local coords.
- Uses a local neighbor mask (all true within the structure), and a local “region edits” overlay (within the castle only) to avoid seams.
- Render by drawing N models with the same structure transform.

This scales better for huge objects and helps cull hidden faces inside the castle, at the cost of added complexity.

---

## Implementation Plan (Phased)

Phase 1: Prototype (no steering, constant ambient)
- Add `Structure` with fixed dims (e.g., 32×24×32), `StructureEditStore`, and one example castle asset (even procedural).
- Add `build_voxel_body_cpu_buf` with simple ambient+emission lighting; return `ChunkMeshCPU` with local coords (no `base_x/base_z` offset).
- Add runtime path to build and cache `StructureRender` and draw at `structure.pose.pos`.
- Extend raycast and collision samplers to include structure sample path; add `Event::StructureBlockPlaced/Removed`.
- Move the castle in a slow circle in `App::step`.

Phase 2: Interactivity
- Add steering events and velocity integration.
- Add platform velocity so the player rides the deck smoothly.
- Add save/load for the structure’s edits.

Phase 3: Polish
- Micro-chunking for big castles; local region-edit overlay.
- Better lighting (local flood fill), leaf/alpha handling matches world materials.
- Optional world-castle occlusion interaction (complex; likely skip).

---

## Concrete Integration Points (by file)

- src/mesher.rs
  - Add `build_voxel_body_cpu_buf` (clone greedy pipeline, drop `LightingStore` dependency, emit local-space vertices).
  - Optionally refactor a shared greedy core parametrized by a lighting provider.

- src/runtime.rs
  - Add `structures: HashMap<Id, StructureRender>` and a simple `StructureBuildJob` queue (can reuse worker threads; jobs are similar to `BuildJob` without world/region edits).

- src/app.rs
  - Keep a `Vec<Structure>` in `App` or `GameState` with pose and edits.
  - In `render`, draw `Runtime.structures` at `pose.pos` (and later yaw via transform).
  - In `handle_event`:
    - Extend raycast path to test structure in local space.
    - Add `Event::StructureBlockPlaced/Removed` handling to mutate edits and queue rebuild.
    - Update movement in `Event::Tick` or `step` by animating `pose.pos`.
  - In `MovementRequested`, extend collision sampler to test structure before world.

- src/edit.rs
  - Mirror a minimal `StructureEditStore` (or generalize `EditStore` to support non-chunked spaces behind a trait).

- src/raycast.rs
  - Reuse as-is; call it twice (world vs structure) with separate `is_solid` closures and choose the closer.

- src/lighting.rs
  - Not needed for Phase 1 if we do ambient+emission; optional later for local flood-fill.

---

## Open Questions / Risks

- Lighting parity: World uses neighbor-aware lighting and beacon direction planes; the structure will start simpler. Acceptable visually?
- Player boarding stability: Platform velocity handling is needed for a smooth ride.
- Edits while moving: With local-space edits, this is straightforward; ensure raycast target transforms are correct.
- Performance: One structure is trivial; many structures may need micro-chunking + frustum culling.

---

## Nice-to-haves

- Physics-ish collisions between multiple moving voxel bodies (out of scope for now).
- Simple GUI to toggle ride/steer modes, save/load castle designs.
- LOD or impostors for far-away structures.

---

## TL;DR

- Use a separate “voxel body” for the castle, meshed in local space and drawn with a transform. Rebuild only on edits, not movement.
- Extend raycast/collision to sample the castle by transforming the query into its local grid.
- Start with ambient+emission lighting; revisit lighting later.
- For very large castles, split into micro-chunks and apply a local region-edit overlay.


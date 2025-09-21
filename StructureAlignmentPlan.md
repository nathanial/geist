# Structure Alignment Plan

Goal: bring dynamic structures (sun body, orbital schem platforms, moving builds) up to parity with streamed chunk lighting/meshing so visuals and lighting stay consistent everywhere.

## Open Questions
1. Should structures exchange light with the main world grid (i.e., update `LightingStore` and neighbor borders), or stay self-contained?
   Answer: yes
2. How frequently can we afford to recompute lighting for moving/orbiting structures—every pose change, on a timer, or only near the player?
   Answer: Let's try recomputing on every pose change. If that is too onerous, we can throttle the update rate.
3. Do we need to preserve a lightweight ambient-only rendering mode for tooling/debug, or can we fully migrate to the parity pipeline?
   Answer: No need to preserve anything, let's consolidate to just the parity pipeline.

## Phase 1 – Foundation & Decisions
- **Audit findings**
  - Struct meshing uses `build_voxel_body_cpu_buf` with a hard-coded ambient value of 96 and no neighbor occlusion, confirming the expected ambient-only path (`crates/geist-runtime/src/lib.rs`, `crates/geist-mesh-cpu/src/build.rs`).
  - `StructureBuildCompleted` uploads meshes but never installs light atlases; `structure_renders` store `light_tex: None`, so structures skip per-voxel lighting in the draw path (`src/app/events.rs`).
  - Shaders are rebound (fog/leaves/water) the same way as chunks but operate without light uniforms, so they fall back to default tinting.
- **Parity gaps vs. chunk pipeline**
  - No skylight/blocklight propagation; structures lack `LightGrid`s, border updates, and biome tint sampling (chunks inject all of these in `BuildChunkJobCompleted`).
  - No column-profile caching or chunk-buffer reuse, so structures rebuild from scratch each job.
  - Lighting atlases, neighbor awareness, and hot-reload hooks are absent for structures.
- **Decision status**
  - Awaiting answers to the open questions above to finalize requirements for lighting exchange scope, relight cadence, and ambient-only fallback.

## Phase 2 – Mesher Parity
- Replace `build_voxel_body_cpu_buf` with a WCC-backed path (or shared helper) so structures reuse chunk face culling, micro-grid occupancy, and material lookup logic.
- Introduce pseudo-chunk coordinates for structures if needed to satisfy mesher assumptions.
- Ensure leaves/water/fog tagging matches chunk meshes so shader selection is consistent.

## Phase 3 – Lighting Computation
- Integrate `compute_light_with_borders_buf` (or equivalent) for structure buffers, seeding skylight/blocklight from world altitude and internal emitters.
- Decide refresh cadence per structure class (static platform vs. moving body) based on Phase 1 answers.
- Optionally sample nearby chunk borders when structures hug terrain to avoid seams.

## Phase 4 – Rendering Integration
- Have `StructureBuildCompleted` upload light atlases via `update_chunk_light_texture`, mirroring chunk logic, and store the textures on `structure_renders`.
- Bind fog/leaves/water shaders for structures using those textures; keep a fallback path while migrating.
- If structures share lighting with the world, emit any light-border updates or other synchronization events.

## Phase 5 – Performance, Tooling, Rollout
- Budget rebuild cadence, cache results (e.g., reuse light grids for repeated orbit angles), and add perf counters for structure jobs.
- Provide debug tooling (light volume overlays, toggles) and document workflows in `AGENTS.md` / CLI helpers.
- Land changes behind a feature flag, validate with `cargo test --workspace` and targeted perf runs, then remove the ambient-only path once parity is confirmed.

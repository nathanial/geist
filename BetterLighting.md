# Better Lighting — Decoupling Plan

Goal: decouple lighting from meshing so lighting changes never trigger remeshes or geometry uploads. Geometry should depend only on occupancy/material. Lighting should be an overlay that can be updated independently and efficiently.

## Current State (What’s Coupled)

- Mesh build path samples lighting during meshing:
  - `crates/geist-mesh-cpu/src/build.rs::build_chunk_wcc_cpu_buf` computes a `LightGrid` and passes it into the mesher.
  - `crates/geist-mesh-cpu/src/wcc.rs::WccMesher` samples light (`light_bin`) while toggling faces and stores a combined key `(MaterialId, u8 light)` in face grids.
  - Emission (`emit_plane_mask!`) extracts `(mid, l)` and writes per‑vertex `rgba` into `MeshBuild.col`.
  - Thin pass (`emit_box_*`) also samples light and bakes it into `rgba`.
- Renderer uploads vertex colors as the lighting term: `crates/geist-render-raylib/src/lib.rs::upload_chunk_mesh` copies `mb.col` into the GPU mesh colors buffer.
- Runtime: lighting updates (borders) trigger a full chunk rebuild job:
  - `Event::LightBordersUpdated` → `ChunkRebuildRequested { cause: LightingBorder }` → worker recomputes both geometry and baked colors.

Implications:
- Any lighting change re-meshes chunks and re-uploads full geometry and color buffers.
- Face‑grid keys include light, conflating “which face exists” with “how bright it is.”

## Decoupling Strategy Overview

Two phased options, complementary:

1) Phase 1 — No‑remesh lighting updates (CPU colors only):
   - Keep current shading model (vertex colors) but move lighting computation out of meshing. Colors become a separate, updatable buffer aligned to the existing geometry layout. Light changes only recompute colors and update the GPU color buffer, not geometry.

2) Phase 2 — Shader‑sampled lighting (GPU lightfield):
   - Stop baking lighting into vertices entirely. Upload per‑chunk light data (3D texture or SSBO). The shader samples lighting using world/chunk coords and face normal. Lighting changes only update the lightfield resource; geometry remains unchanged and un-touched.

We can ship Phase 1 quickly to remove remeshes on light changes, then migrate to Phase 2 for cleaner architecture and lower CPU cost.

## Phase 1 — Updatable Vertex Colors (No Remesh)

Key ideas:
- Geometry emission no longer depends on light. The mesher records only material in face grids; lighting is computed later.
- On lighting events, recompute per‑vertex colors for the already uploaded meshes and update the GPU colors buffer in place.

Changes by area:

- Mesher (geometry‑only):
  - `crates/geist-mesh-cpu/src/wcc.rs`
    - Change `KeyTable` and grid keys to store only `MaterialId` (remove `u8 light` from key). Light is not part of merge/emit decisions.
    - `light_bin` remains as a helper for lighting overlays but is not used inside `toggle_*` or parity grids.
  - `emit_plane_mask!` and thin emitters (`emit_box_*`) should produce geometry with a placeholder color (e.g., white) or omit color if we prefer shader‑only down the road.
  - Output stays deterministic: per‑face quads ordered by axis/scan so we have a stable vertex order per material.

- Renderer (color buffer updates):
  - Store per‑part mesh layout metadata in `ChunkRender` (vertex count per material, offsets). We already chunk split by material; we further record total vertices per GPU Mesh slice.
  - Add a function to update the color buffer of an uploaded mesh without re-uploading positions/indices:
    - Use Raylib FFI (`UpdateMeshBuffer`) to update buffer index 3 (colors) or re-upload just the colors array for that model part. If FFI is awkward, fallback to re-create the GPU Mesh but copy only the color array, not geometry, for minimal diff.

- Runtime (lighting updates ≠ rebuilds):
  - New event: `ChunkLightingUpdateRequested { cx, cz }` scheduled from both `LightBordersUpdated` and emitter add/remove events within the visible radius.
  - Light update job lane: compute a fresh `LightGrid` for the chunk using the authoritative `ChunkBuf` stored in `GameState::chunks[(cx,cz)].buf`. No meshing.
  - Walk the existing geometry layout deterministically and recompute colors using the same face‑aware sampler currently used in meshing (`LightGrid::sample_face_local_s2`). Emit a dense `Vec<u8>` for colors aligned with the uploaded vertex order per material.
  - On completion, call renderer color‑update for the chunk’s GPU meshes.
  - Keep `LightingStore::update_borders_mask` logic to debounce border change fanout; it now gates light‑update jobs, not rebuilds.

Notes:
- This removes remeshes on lighting changes. CPU still computes colors O(Nverts) for affected chunks, but geometry is untouched, and GPU upload is limited to colors.
- If we keep a visual minimum (e.g., `VISUAL_LIGHT_MIN`) apply it in the color compute stage.

## Phase 2 — Shader‑Sampled Lighting (GPU Lightfield)

Key ideas:
- Upload the chunk’s lightfield to the GPU (per‑chunk 3D texture or SSBO with addressing) and sample it in the fragment shader. Lighting updates only update the lightfield resource; vertex colors become constant.

Data representation options:
- RG8 3D texture: R = block light, G = skylight. Size = chunk dims (coarse voxel grid). For a 32×256×32 chunk, ~262k texels (≈512 KB at 2 bytes/texel). Manageable per visible chunk.
- If we need micro S=2 fidelity, consider: (a) shader logic to choose between inside/neighbor voxel based on face normal + micro cell index, or (b) encode a single combined brightness per coarse cell and accept approximation for micro shapes.

Shader sampling (per fragment):
- Inputs: world position (already in vertex), face normal, chunk base (cx*size), and scale.
- Compute local voxel coords and sample rules mirroring `sample_face_local_s2`:
  - For face F on cell at (x,y,z), sample the voxel on the visible side of F, with bias for skylight/block attenuation.
  - For micro S=2 faces, derive micro subcell from fractional part (e.g., `fract(coord * 2.0)`) to disambiguate which subcell is visible and choose inside/neighbor sample accordingly.
- Combine R/G to RGB multiplier; apply visual minimum if desired inside shader.

Renderer changes:
- Add a `ChunkLightTexture` per loaded chunk (create/destroy with chunk streams). Allocate with +1 border texels on X− and Z− sides so seam sampling matches neighbors; populate those from `LightBorders` to ensure continuity.
- Extend `FogShader`/`LeavesShader` (and water) to bind the 3D light texture and per‑chunk transform uniforms. Replace usage of vertex color with sampled light.

Runtime changes:
- Replace Phase 1 light‑update jobs with upload to the chunk’s 3D light texture. No CPU walk over geometry. On `LightBordersUpdated`, update the border texels for both owner and neighbor as needed; on emitter changes, recompute light for the chunk (and affected neighbors if necessary) and upload texture subregions.

Pros/Cons:
- Pros: Lighting fully decoupled, very low CPU cost per change, no geometry traffic. Paves the way for dynamic effects and materials.
- Cons: Requires custom shaders and managing 3D textures per chunk; careful seam handling; slight divergence from exact S=2 micro sampling unless we mirror it in shader logic.

## Migration Plan

1) Prepare geometry‑only mesher:
   - Remove light from face keys in `WccMesher` (material‑only keys). Ensure baked `rgba` is temporary or constant.
   - Confirm deterministic vertex order per material (already true with per‑face emission loops).

2) Phase 1 implementation:
   - Add `ChunkLightingUpdateRequested/Completed` events and a dedicated job lane.
   - Implement CPU color recompute using `LightGrid::sample_face_local_s2` over the current chunk’s `ChunkBuf` and geometry layout. Persist per‑part vertex counts/offsets in `ChunkRender`.
   - Expose a renderer entry to update mesh color buffers (via Raylib FFI `UpdateMeshBuffer` or minimal re-upload path).
   - Swap event fanout: `LightBordersUpdated` schedules light update jobs (not rebuilds). Emitter add/remove also schedules light updates for visible chunks within a radius.
   - Keep LightBorders maintenance unchanged; still used to avoid unnecessary updates and to seed seam continuity in Phase 2.

3) Optional: Ship Phase 1 (user‑visible result):
   - Lighting changes no longer trigger remeshes. Geometry is stable; only colors change.

4) Phase 2 implementation (behind a feature flag):
   - Add per‑chunk 3D light textures; upload from `LightGrid` and update on changes. Include a 1‑voxel ring for X−/Z− to stitch seams with `LightBorders`.
   - Update shaders to sample the 3D lightfield; remove reliance on vertex colors. Keep a fallback path to Phase 1 colors while iterating.
   - Optimize updates: compute and upload subregions or only borders when applicable.

5) Clean‑up:
   - Once stable, drop CPU color recompute path and stop generating color arrays in `MeshBuild`.
   - Remove light sampling code from `crates/geist-mesh-cpu` entirely; the mesher becomes geometry‑only.

## Risks and Mitigations

- Vertex order stability (Phase 1):
  - Risk: any future change in emission order would desync color updates.
  - Mitigation: codify and test the emission order (axis → plane index → scan order), and store per‑material vertex counts/offsets in `ChunkRender` as the single source of truth for update ranges.

- Raylib color buffer updates:
  - Risk: Rust binding may not expose `UpdateMeshBuffer` ergonomically.
  - Mitigation: use `raylib::ffi::UpdateMeshBuffer` directly; if needed, rebuild only the color array for the model part while reusing positions/indices.

- Seam continuity (Phase 2):
  - Risk: sampling across chunk boundaries shows seams.
  - Mitigation: maintain a 1‑voxel border in each chunk’s light texture populated from `LightBorders`; treat owner planes consistently so both sides sample identical values at borders.

- Micro S=2 fidelity in shader:
  - Risk: shader approximation doesn’t perfectly match CPU `sample_face_local_s2` in edge cases.
  - Mitigation: port the face‑aware sampling rules into shader logic using normal and fractional local coords; add tests/images to verify parity with CPU in representative cases.

## Concrete Touch List (by file)

- Meshing (geometry‑only):
  - `crates/geist-mesh-cpu/src/wcc.rs`: remove light from `KeyTable` and `toggle_*` keys; do not call `light_bin` inside toggles. `emit_into` uses only `MaterialId`; set color to constant for now.
  - `crates/geist-mesh-cpu/src/emit.rs`: make `emit_box_*` accept an external color provider; for Phase 1 simply pass white.
  - `crates/geist-mesh-cpu/src/build.rs`: continue computing `LightGrid` for Phase 1 updates and for `LightBorders`; but meshing itself no longer needs it.

- Renderer:
  - `crates/geist-render-raylib/src/lib.rs`: extend `ChunkRender` to store per‑part vertex counts/offsets; add `update_chunk_colors(cx,cz, part, &[u8])` that updates GPU colors via FFI.
  - Shaders (Phase 2): extend Fog/Leaves/Water to sample a bound 3D light texture and drop dependence on vertex colors.

- Runtime:
  - `src/event.rs`: add `ChunkLightingUpdateRequested/Completed` events.
  - `src/app.rs`: replace `LightBordersUpdated` → rebuild with a light‑update scheduling path; add a light update job lane. On completion, call renderer color‑update (Phase 1) or texture upload (Phase 2).
  - `src/gamestate.rs`: keep `ChunkBuf` for light recomputes; no change to mesh counters.

## Acceptance Criteria

- Lighting changes (emitters and border updates) do not enqueue chunk remesh jobs.
- Geometry uploads remain unchanged when only lighting changes.
- Visual lighting updates propagate correctly across chunk seams without cracks or mismatches.
- Phase 1: colors update in place with no new geometry.
- Phase 2: shaders render lighting from a per‑chunk lightfield; vertex colors are constant or unused.

## Stretch Goals

- Batched light texture atlas for multiple chunks to reduce binds.
- GPU light blending for dynamic day/night cycles without CPU recompute.
- Material flags to opt‑in to custom light response (e.g., emissive, translucent).


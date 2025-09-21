# New Light Plan

## Executive Summary
We currently represent daylight as a static skylight scalar that flood-fills each chunk from the sky, stored in the green channel of the three-channel light atlas (`crates/geist-lighting/src/lib.rs`, `crates/geist-lighting/src/micro.rs`). The renderer simply multiplies that scalar by `skyLightScale` to simulate a day/night cycle (`assets/shaders/voxel_fog_textured.fs`, `src/app/render/frame.rs`). Replacing this with a single, directional sun would deliver stronger cues for time-of-day, surface relief, and underground darkness. This document explores the scope of that change, sketches candidate approaches, and recommends a staged plan that adds directional sun lighting without losing the efficiency of our existing light propagation.

## Current Lighting Snapshot
- **Light data layout**: Each chunk uploads a 3D light volume baked into a 2D texture atlas (`LightGrid`). Channels: block light (R), skylight (G), beacons (B). Skylight is a flood-fill seeded from open sky in each chunk worker.
- **Propagation pipeline**: Workers run an S=2 micro-voxel BFS (2× resolution per axis) that honors partial occupancy, fences, ladders, etc. (`crates/geist-lighting/src/micro.rs`). A coarse fallback BFS runs when micro lighting is disabled.
- **Day/night modulation**: The viewer toggles a global `skyLightScale` uniform each frame to dim or brighten the baked skylight equally everywhere (`src/app/render/frame.rs:62-98`). No directionality or per-pixel normal interaction exists today.
- **World assumptions**: The flood fill assumes light travels straight down. Overhangs and caves darken correctly, but anything that should be shadowed by a hill to the west at sunset still receives full skylight until night.

## Goals For A Directional Sun
- **Directional shading**: Lit surfaces should brighten when their normal faces the sun and fall into shadow when blocked.
- **Temporal coherence**: The sun should traverse a predictable arc (reuse the existing `day_length_sec` cycle) without wholesale recomputation of every chunk each frame.
- **Interior darkness**: Underground spaces should stay dark even if the sun is low on the horizon; we cannot regress cave ambience.
- **Compatibility**: Preserve block light propagation and existing emissive gameplay logic. Avoid breaking the `LightGrid` consumer APIs during the transition if possible.
- **Performance**: Minimize full-volume lighting rebuilds. Target amortized updates tied to chunk streaming or slow sun motion (seconds between significant shadow changes), not per-frame recomputation.

## Constraints & Observations
- **Chunk-local precomputation**: Lighting builds happen off-thread per chunk. Anything we add should fit into that pipeline or a similar background job in `geist-runtime`.
- **Existing data channels**: We have one 8-bit channel free if we fold beacons into block light or repurpose skylight. Alternatively, expand the light atlas format (larger textures, more VRAM).
- **Sun movement**: A directional light that changes azimuth/elevation every frame invalidates static flood-fill assumptions. We need either slow updates or a fast visibility test per pixel.
- **Camera range**: The far plane reaches ~10 km (`src/app/render/frame.rs:34`). Cascaded shadow maps that cover that extent would be expensive without frustum slicing.
- **World features**: Tall vertical features and micro geometry (stairs, slabs) already rely on the micro BFS for correct occlusion; we should reuse that occupancy data where possible.
- **Skylight leakage regression**: Since moving from tall column chunks to cubic chunks we seed skylight independently per chunk. Underground layers that load before their "ceiling" neighbor now assume open-to-sky columns and flood-fill caves. Once the ceiling chunk finishes, we do not retroactively clear the leaked light. The new design has to include an occlusion handshake (e.g., defer skylight seeding until the column profile confirms sky exposure, or mark sealed columns dirty when an above chunk materializes) so caves stay dark regardless of load order.

## Option A — Anisotropic Skylight Field
**Idea**: Extend the light volume so each voxel stores sun visibility along a small set of basis directions (e.g. +Y, two diagonal samples). During shading, pick the visibility closest to the current sun direction and modulate with a Lambertian dot product.

**Pros**
- Reuses existing voxel pipeline; no GPU shadow maps needed.
- Supports low sun angles by dedicating more basis samples near the horizon.

**Cons**
- Light builds must recompute whenever basis samples change (i.e. time-of-day crosses a breakpoint).
- Memory multiplies by number of basis directions (3–5× larger light textures) unless we compress aggressively.
- Still approximate—shadows only as accurate as selected bases, causing popping when the sun crosses to a new basis vector.

## Option B — Cascaded Shadow Mapping (CSM)
**Idea**: Keep voxel block light for emissives but replace skylight entirely in the shader. Render depth maps from the sun’s perspective (multiple cascades) each frame and compute direct light per fragment on the GPU.

**Pros**
- High-quality directional shadows with smooth motion and penumbra tricks (PCF, VSM).
- No chunk-side rebuilds when the sun moves.

**Cons**
- Requires large changes to our renderer (`geist-render-raylib`) or a custom backend; Raylib’s GL wrapper provides limited hooks.
- VRAM and draw-call overhead: need to render terrain into the shadow map using the same geometry buffers (meshes) before the main pass.
- Needs stable world-to-light transforms for enormous view distances; aliasing risk is high without cascade stabilization.
- Underground darkness would rely on the absence of direct light; we’d need an ambient term to mimic current flood-lit skylight or caves may glow from residual block light.

## Option C — Hybrid Column Horizon + Local Fill (Recommended)
**Idea**: Keep the micro BFS to provide block light and coarse ambient skylight, but layer a directional “sun visibility” field derived from per-column horizon maps. Each column stores the highest elevation angle that blocks the sun for each azimuth sector. During gameplay, interpolate visibility for the current sun azimuth and adjust per-surface brightness in the shader.

**Implementation Sketch**
1. **Per-column preprocessing**
   - When a chunk column (x,z) finishes building, compute horizon samples (e.g. every 10°) by marching along the terrain heightmap plus micro-occupancy for nearby occluders. Persist into the `ChunkColumnProfile` introduced in Phase 5 (`crates/geist-runtime/src/column_cache.rs`).
   - For underground voxels where the column is sealed above, tag them as “sun occluded” immediately.
2. **Directional visibility field**
   - Store a single byte per macro voxel indicating whether the current sun angle clears the horizon. Update lazily when the sun enters a new azimuth bin or when the column profile invalidates.
   - Reuse the existing green channel for “sun direct” intensity; keep a small ambient floor to soften interior lighting.
3. **Shader adjustments**
   - Pass the sun direction and color as new uniforms.
   - Compute `max(0, dot(normal, sun_dir)) * sun_visibility` and mix with block light + ambient skylight. Optionally blend in a warm rim at low angles.
4. **Temporal updates**
   - Quantize the sun path to ~32 azimuth steps and 8 elevation slices. When the sun crosses a boundary, schedule background jobs to refresh the affected columns (similar cost to current flood-fill rebuilds but amortized over time-of-day).

**Pros**
- Reuses chunk column cache; no per-frame GPU shadow renders.
- Supports low-angle shadows with controllable resolution (azimuth bins).
- Keeps micro BFS for complex geometry—columns can consult the detailed occupancy within a neighborhood.
- Ambient skylight still available for caves; simply clamp to a low baseline.

**Cons**
- Shadows are columnar—fine-grained occluders (trees, statues) may look soft or delayed until the horizon map samples them.
- Requires careful invalidation: edits that raise terrain or place tall structures must recompute nearby horizons.
- Sun visibility updates are discrete; need temporal smoothing to hide bin transitions.

## Data Flow Proposal (Hybrid)
1. **Sun Path Model** (`src/app/render/frame.rs`)
   - Promote the existing phase calculation into a shared `SunState { azimuth, elevation, color }` service so lighting workers and renderer read the same values.
2. **Column Horizon Builder** (`crates/geist-runtime/src/column_cache.rs`)
   - Extend `ChunkColumnProfile` to cache `horizon[azimuth_bin] = max_elevation_deg` and `occluder_height[azimuth_bin]` within a configurable radius (e.g. 64 m).
3. **Light Worker Update** (`crates/geist-lighting/src/lib.rs`)
   - During skylight propagation, fetch the current sun azimuth bin, test against the stored horizon, and set `sun_visibility` (0 or attenuated factor) per voxel column before BFS. Underground voxels keep 0 and rely on block light.
   - Maintain ambient skylight as a secondary field (e.g. 48/255) so caves aren’t pitch black.
4. **Renderer** (`crates/geist-render-raylib/src/lib.rs` + shaders)
   - Add uniforms for sun direction/intensity.
   - Adjust fragment shaders (`assets/shaders/voxel_fog_*.fs`) to compute lambertian shading from normals, modulated by `sun_visibility` from the light atlas. Preserve block light sum for emissive sources.
5. **Edits & Hot Reload**
   - When terrain edits occur, mark affected horizon bins dirty and enqueue recomputation tasks (in the same job pool as lighting).

## Transition Strategy
1. **Phase 1**: Introduce `SunState` and drive existing `skyLightScale` from it to keep behavior identical while plumbing data through runtime and renderer.
2. **Phase 2**: Teach chunk lighting to output both ambient skylight and placeholder sun visibility (currently all 255) using the existing atlas format to validate plumbing.
3. **Phase 3**: Implement horizon sampling and binning offline; experiment with 16 azimuth × 4 elevation grid. Add smoothing/interpolation logic.
4. **Phase 4**: Replace shader lighting path to use normals and the new visibility field. Ship behind a debug toggle for comparison.
5. **Phase 5**: Iterate on quality (bin count, smoothing, ambient floor) and profile rebuild costs over a full day cycle.

## Risks & Mitigations
- **Performance spikes when sun updates**: Mitigate by staggering column refresh jobs across frames and caching intermediate results per column radius.
- **Shadow aliasing from coarse bins**: Fade transitions between bins using trilinear interpolation, and bias horizons slightly upward to avoid leaking sun into cliffs.
- **Edit churn**: Large edits (schematics, TNT) could dirty many columns. Use a capped flood of horizon recompute jobs and fall back to vertical flood-fill skylight temporarily.
- **Shader divergence**: New lambertian math may desaturate textures. Keep an adjustable ambient term and tone-map to preserve art direction.

## Open Questions
- How far should horizon sampling reach? Is a 3–4 chunk radius enough to catch mountains casting long shadows?
- Can we store horizon data compactly in `ChunkColumnProfile` without blowing cache size (currently shared by terrain streaming)?
- Do we still need beacon (B channel) once the sun uses that slot? If not, where do we redirect beacon lighting?
- Should underground ambient depend on biome/time (e.g., moonlight glow)?
- Would a lightweight screen-space ambient occlusion pass pair well with the new directional sun to recover small-scale detail?

## Next Steps
1. Stop the current skylight leak by deferring per-column seeding until the column profile (or neighbor chunk) certifies open sky, and enqueue re-lighting when an above chunk seals a column.
2. Prototype `SunState` and wire it through runtime + renderer to replace the ad hoc `sky_scale` calculation.
3. Spike a CPU-only horizon sampler on a test world; measure build time per column and accuracy at dawn/dusk.
4. Define the atlas format for ambient + sun visibility + block light, and confirm GPU memory impact at current chunk budgets.
5. Update shaders with a compile-time toggle to switch between legacy skylight and experimental sun for side-by-side profiling.
6. After validation, schedule a follow-up plan for beacon channel migration and gameplay tuning (night brightness, torch range).

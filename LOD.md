LOD for Chunk Rendering in Geist

- Goal: render distant terrain cheaply while keeping near detail crisp.
- Scope: CPU mesher (`crates/geist-mesh-cpu`), runtime scheduling (`crates/geist-runtime`), and draw path (`crates/geist-render-raylib` + `src/app.rs`).
- Constraint: preserve watertight seams and consistent lighting across chunk boundaries.

Current Pipeline (Summary)
- Streaming: `App::flush_intents` schedules meshing jobs per chunk by ring distance; no LOD differentiation yet. View radius defaults to 12 chunks, with frustum culling at draw time.
- CPU meshing: `build_chunk_wcc_cpu_buf_with_light` builds one mesh per chunk via `ParityMesher` (WCC v3). Micro-occupancy is S=2 half-steps (2× upsample per axis) merged into greedy quads. Thin dynamic shapes (pane, fence, carpet) are emitted in a second pass.
- Lighting: worker computes a per-chunk `LightGrid`, main thread packs a ring-extended atlas; shaders sample per-voxel via the atlas (fallback to a brightness floor if texture is absent).
- Rendering: per-chunk per-material `Model` uploaded to raylib; opaque drawn first, water later. Frustum culling on chunk AABBs; no distance-based simplification.

Where GPU Time Goes (Typical)
- Vertex/triangle count is proportional to visible surface area. Micro-occupancy (S=2) and thin shapes increase triangles substantially.
- Draw calls scale with materials × chunks; distant rings amplify both even after frustum culling.
- Per-fragment lighting is relatively cheap; the big wins are fewer vertices and fewer models to draw.

LOD Strategy Overview
- LOD0 (near): current behavior (S=2 microgrid + thin shapes + full lighting). Highest fidelity.
- LOD1 (mid): disable micro detail and thin shapes; emit only full-cube occupancy (S=1). Keep lighting, but optionally skip the light atlas to save a texture bind at extreme mid.
- LOD2 (far): coarse “super-voxel” meshing by downsampling whole blocks in 2×2×2 (or 4×4×4) groups into full cubes. Optionally omit lighting atlas (shader falls back to `visualLightMin`).
- Optional LOD3 (horizon): height-only impostor for terrain-like worlds, or even a single tinted AABB per chunk. Only worth it if LOD2 still overdraws too much.

Why these work with the existing code
- Parity mesher already supports S=1 in its grids and emitters (benches use S=1 for uniform solids). Using S=1 naturally drops micro-occupancy detail; greedy merging produces large quads over flat terrain.
- Thin shapes are emitted by a separate pass in `build.rs`; skipping that pass is trivial and eliminates a large triangle/draw-call tail.
- Shaders gracefully handle missing `lightTex` by using a floor (`visualLightMin`). That allows far LODs without per-chunk light textures.

Key Design Details
- Distance rings: reuse existing ring bucketing in `App::flush_intents`. Map Chebyshev ring to an LOD tier. Example: rings 0–4 → LOD0; 5–8 → LOD1; 9+ → LOD2.
- Seam correctness: keep the same half-open seam ownership. For LOD1 (S=1), parity planes still toggle against neighbor occupancy. Lighting borders are already macro (voxel) planes; they remain compatible. Mixed LOD seams will be watertight because ownership and clipping are unchanged.
- Materials: LOD1/2 keep the per-material partitioning, so upload/render code stays unchanged. Expect far fewer quads → fewer vertices/draws.
- Lighting: LOD0 uses per-chunk light atlas. LOD1 can choose atlas on/off (tunable). LOD2 should omit atlas to avoid bandwidth and texture churn; distant chunks look fine with fog + visual floor.

Minimal Engine Changes (incremental plan)
1) Mesher options API
   - Add `MeshingOptions { micro_steps: usize, include_thin: bool, include_water: bool }`.
   - Thread through `build_chunk_wcc_cpu_buf_with_light`, `build_chunk_into_sink_with_light`, and `ParityMesher::new`.
   - Behavior:
     - LOD0 → `{ micro_steps: 2, include_thin: true, include_water: true }`.
     - LOD1 → `{ micro_steps: 1, include_thin: false, include_water: true }`.
     - LOD2 → `{ micro_steps: 1, include_thin: false, include_water: false }` (optional: keep water if desired).

2) Handle S!=2 in micro occupancy
   - Current code assumes S=2 when `variant.occupancy` is present. For `micro_steps==1`, treat any nonzero micro occupancy as a full cube (or ignore occupancy entirely and only use `is_solid`).
   - Implementation: in `ParityMesher::build_occupancy`, branch on `self.s`. If `s==1`, skip `variant.occupancy` path and rely on full-cube solids; this matches the LOD goal of dropping micro detail.

3) Skip thin pass by flag
   - Guard the pane/fence/carpet emission loop in `build.rs` behind `include_thin`.

4) Runtime and scheduling
   - Add `lod: LODLevel` to `BuildJob` (and into job hash) so uploads don’t race between different LODs.
   - In `App::flush_intents`, compute LOD per (cx,cz) from `dist_bucket` and select lane + options:
     - LOD0: current path (edit lane).
     - LOD1: bg lane with LOD1 options; optionally request light-only later if we want an atlas.
     - LOD2: bg lane with LOD2 options; no light atlas request.
   - On `BuildChunkJobCompleted`, keep the highest-fidelity mesh available: replacing LOD2 with LOD1, then LOD0 as chunks move inward.

5) Rendering policy
   - If `cr.light_tex` is None, shaders already use `visualLightMin`; fog still applies. No shader changes are required.
   - Optionally tag far LOD materials with a simpler shader (skip light sampling) to shave a little ALU, but the gain is marginal versus vertex count.

6) Coarse super-voxel (optional, for LOD2+)
   - For larger gains, pre-downsample per-chunk blocks to a `ChunkBuf` with `sx/2, sy/2, sz/2` (or 4×) using a simple aggregator (e.g., “solid if any solid in the cell”, pick a dominant material).
   - Run `ParityMesher` with `s=1` against that synthetic buffer but emit geometry in world units scaled by 2 (or 4). This produces very low-poly far meshes that still respect seams and clipping.
   - Implementation fits in the existing mesher crate by adding a helper to build a temporary coarse `ChunkBuf` and forwarding to the same builders.

Expected Wins
- Triangle count: LOD1 removes micro detail and thin shapes; large planes greedy-merge → very few quads over flat terrain. LOD2 reduces planes further by grouping blocks.
- Draw calls: fewer per-chunk quads means fewer vertices copied; material count drops when thin-shape materials disappear; water can be disabled beyond a ring.
- Bandwidth: skipping the light atlas for LOD2 reduces texture updates/binds.

Compatibility and Edge Cases
- Seam safety across mixed LODs: preserved by parity ownership and clipping logic; faces still meet on −X/−Z and Y is chunk-local.
- Lighting continuity: near chunks keep atlases; far chunks use brightness floor + fog, which hides detail loss. If needed, keep atlas for LOD1 to smooth the transition.
- Edits and promotion: when a chunk moves inward, schedule a rebuild at the higher LOD; job_id must include LOD to avoid stale overwrite.
- Water: disabling it at far LOD removes a costly transparent pass; fog largely masks the difference. Keep it for oceans if needed.

Rollout Plan (safe steps)
- Phase A (low risk):
  - Add `MeshingOptions` and hook `include_thin=false` + `s=1` for a debug toggle from a hotkey to measure savings.
  - Add a simple ring→LOD mapping in `App::flush_intents` that always rebuilds inward with higher LOD.
- Phase B (quality):
  - Optional super-voxel downsampling for LOD2.
  - Optional keep-light-atlas for LOD1 only.
- Phase C (polish):
  - Hysteresis on ring thresholds to avoid flicker as camera hovers on boundaries.
  - Per-material policy: always keep leaves at LOD1 if necessary for silhouette.

Touchpoints (files to change)
- `crates/geist-mesh-cpu/src/build.rs`: accept `MeshingOptions`; pass to `ParityMesher`; skip thin pass by flag.
- `crates/geist-mesh-cpu/src/parity.rs`: support `s=1` LOD by bypassing per-variant micro occupancy; leave full-cube path unchanged.
- `crates/geist-runtime/src/lib.rs`: add LOD to `BuildJob`, job hash, and worker meshing call.
- `src/app.rs`: map ring distance→LOD, request builds with options, prefer highest LOD on upload, optionally drop light-atlas requests for LOD2.
- (Optional) add a coarse-downsample helper in `geist-chunk` or `geist-mesh-cpu`.

Tuning Defaults (starting point)
- Rings 0–4: LOD0 (S=2, thin on, water on, light atlas on).
- Rings 5–8: LOD1 (S=1, thin off, water on, light atlas on).
- Rings 9+: LOD2 (S=1, thin off, water off, no light atlas). Increase fog density slightly with distance if desired.

Notes
- The existing greedy plane emission and seam ownership are ideal for LOD: they already minimize geometry and ensure watertightness. LOD mainly becomes a question of “what not to emit.”
- If GPU becomes bind-call limited rather than vertex limited, consider material atlasing to reduce models per chunk, but that’s a larger investment and not required for first LOD.


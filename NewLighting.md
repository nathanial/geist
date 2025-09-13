**Motivation**
- Faces look darker than expected in some cases, especially near slabs/stairs (S=2 occupancy), panes/fences, and at chunk seams.
- The root cause is a mismatch between how meshing decides face visibility (WCC at S=2) and how lighting samples/propagates brightness (voxel grid with heuristics for S=2).

**Current Behavior (Summary)**
- Meshing: crates/geist-mesh-cpu uses WCC at S=2 for full cubes and micro-occupancy; thin dynamics (pane/fence/gate/carpet) are emitted via a thin‑box pass. Emission is per boundary cell (no greedy merge), keyed by `(MaterialId, LightBin)`.
- Lighting: crates/geist-lighting computes a per‑voxel LightGrid with three channels: `skylight`, `block_light`, and `beacon` (+ direction). Propagation is BFS-like and “face-aware” at S=2 via `can_cross_face_s2(...)`. Per-face sampling uses `sample_face_local_s2(...)` to approximate micro openings.
- Shading: The mesher encodes a per-vertex grayscale light in `MeshBuild.col`. Shaders multiply texture by vertex color and apply fog.

**Symptoms Observed**
- Dark faces next to micro shapes where the WCC mesher exposes a face, but lighting underestimates neighbor contribution.
- Faces behind glass panes/fences appear too dark; block lights do not seem to pass.
- Occasional chunk seam darkening until neighbor light borders arrive and a rebuild occurs.

**Likely Mismatches**
- Mixed samplers: Some thin-shape code still calls `sample_face_local(...)` (S=1 semantics) instead of `sample_face_local_s2(...)`, causing under-lighting on micro-affected faces.
- Sampling granularity: WCC toggles faces at S=2 micro cells, but light sampling takes a single bin per block face and reuses it for all micro face‑cells in that block. This can visibly under/over-light merged quads that cross micro detail.
- Propagation gate vs occlusion gate: `can_cross_face_s2(...)` defines plane “sealed” if either side covers a micro cell (`a || b`). WCC seam occluder checks for neighbors consider only the neighbor’s occupancy for sealing that micro cell. Subtle differences can show up in edge cases.
- Data flags: Many transparent/thin blocks (e.g., `glass_pane`, `fence`, `fence_gate`, `carpet`) default to `propagates_light = false`, so block light BFS refuses to enter those voxels even if geometry is visually open, producing darker results than expected.
- Seam timing: When a chunk is built before its neighbor’s light borders exist, `neighbor_light_max(...)` falls back to conservative values; a later “borders updated” rebuild fixes it, but the transient frame is dark.

**Concrete Code Pointers**
- Meshing WCC: `crates/geist-mesh-cpu/src/lib.rs`
- Light sampling in WCC: `WccMesher::light_bin(...)` → `LightGrid::sample_face_local_s2(...)`
- Mixed sampler usage in thin shapes: search for `sample_face_local(` calls in the thin‑box pass
- Light propagation gates: `crates/geist-lighting/src/lib.rs` `can_cross_face_s2`, `block_light_passable`, `skylight_transparent`
- Neighbor fallback: `LightGrid::neighbor_light_max(...)` and `LightingStore::update_borders(...)`

**Short-Term Improvements (Low Risk)**
- Unify samplers: Replace all remaining `sample_face_local(...)` calls in the mesher with `sample_face_local_s2(...)` so thin shapes respect micro openings. This directly targets the “faces too dark next to panes/slabs” symptom.
- Data fixes: Mark transparent/thin blocks as light‑passable where appropriate.
  - `glass_pane`, `fence`, `fence_gate`, possibly `carpet`: set `propagates_light = true` in `assets/voxels/blocks.toml`.
  - Optionally define a profile to override just block‑light passability without changing skylight behavior.
- Seam rebuild policy: If a chunk is built without any neighbor borders yet, enqueue an automatic rebuild once borders arrive (already partly done). Add a short-circuit to skip uploading the first mesh for that chunk until at least one lateral neighbor’s borders exist, if stalling a frame is acceptable.

**Medium-Term Options (Better Consistency)**
- Per‑micro face sampling tied to WCC
  - During WCC emit, compute light per plane‑cell instead of once per block face. Extend `LightGrid` with a micro‑aware sampler: `sample_face_micro(buf, reg, x, y, z, face, i0, i1)` where `(i0,i1)` are micro offsets on the plane (0..S-1).
  - This removes averaging artifacts across micro details and prevents over‑merging quads with visibly different brightness.
- Align “sealed plane” logic
  - Make mesher’s seam occluder checks and lighting’s `can_cross_face_s2(...)` share the same predicate (ideally one utility in `geist-lighting`). Today propagation considers a plane sealed if either side covers a micro cell; the mesher checks just the neighbor in some paths. A single authoritative test avoids edge cases.
- Border-aware face sampling
  - For plane cells on −X/−Z chunk borders, sample neighbor brightness from `LightBorders` at matching micro indices when the neighbor buffer is out of bounds, not just the max at block resolution. This reduces border darkening for micro detail.

**Long-Term Direction (Most Correct)**
- Micro‑voxel light field (S=2)
  - Store light per micro‑cell (2× resolution per axis → 8× voxels). Propagation uses micro adjacency with the same plane‑open predicate used by WCC.
  - Sampling becomes trivial: face‑cell light is just the max of the two micro voxels separated by that plane cell (local and neighbor). No special heuristics.
  - Memory/perf tradeoff: For 32×256×32 chunks, 262,144 voxels; at S=2 micro that’s 2,097,152 cells. With 2–3 channels (skylight, block, beacon) as `u8`, this is 2–6 MB/chunk worst case. Techniques to mitigate:
    - Store only a single channel or compress skylight (RLE per column).
    - Quantize further (nibbles for non-beacon), or use per‑plane sparse storage.
    - Compute on demand for build jobs; avoid persisting full grids in memory.
- Hybrid micro grid
  - Keep voxel lighting at S=1 but add per‑plane micro masks and per‑plane micro light samples only for exposed faces (the WCC frontier). This captures most visual benefit with far less memory than a full micro‑grid.

**Blend/Quantization Improvements**
- Introduce a small number of light bins (e.g., 16) with gamma-aware mapping to improve gradient smoothness without inflating key diversity.
- Consider folding simple AO into the per‑face bin to stabilize merges, or compute AO as a separate, cheap term based on occupancy around the face cell (S=2 neighborhood).

**Proposed Rollout Plan**
- Step 1: Replace `sample_face_local(...)` with `sample_face_local_s2(...)` in thin‑shape emitters; set `propagates_light = true` for glass panes and other open thin blocks; verify dark‑face regressions.
- Step 2: Add a micro‑aware face sampler and use it in WCC emit to compute per‑plane‑cell light bins; gate merges by this finer bin to avoid over‑merging across brightness seams.
- Step 3: Unify the sealed‑plane predicate and expose it from `geist-lighting` so both lighting and mesher use the same function.
- Step 4 (optional): Implement micro‑voxel lighting for a test world size and compare memory/time vs visual uplift. If too heavy, try the hybrid per‑plane micro sampler/border approach instead.

**Validation Checklist**
- Compare side‑by‑side captures before/after on:
  - Stair stacks and slab terraces (S=2 shapes).
  - Glass pane corridors with emissive lights behind.
  - Chunk seams with and without neighbors loaded.
- Ensure no increase in visible cracks or duplicate faces (WCC property should hold).
- Track merge counts; expect slightly fewer over‑merged quads when bins refine per micro face.

**Open Questions**
- Should thin dynamics be routed through WCC at S=2 (or S=4) instead of a separate pass to fully unify occlusion and lighting? Cost vs benefit needs measurement.
- Do we want glass to block skylight but pass block light, or should both pass? Current flags allow this to be configured per block.
- Is the minimum brightness floor (`VISUAL_LIGHT_MIN`) still desired once face sampling is more correct? It may be reduced.

**TL;DR**
- Quick wins: use the S=2 sampler everywhere, fix `propagates_light` for open thin blocks, and ensure a rebuild after borders arrive.
- Better match: compute light per WCC micro face cell and unify the “sealed” predicate across mesher and lighting.
- Ultimate: micro‑voxel lighting at S=2 for exact consistency with WCC.

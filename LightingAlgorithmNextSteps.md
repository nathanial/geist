# Lighting Algorithm — Current State and Next Steps

This note captures the status of the Micro S=2 lighting work, what’s in place now, and the concrete steps to take it to a robust, WCC‑aligned solution across seams and faces.

## TL;DR
- Implemented Micro S=2 lighting with micro BFS and per‑micro seam exchange.
- Face shading now samples the exact two micro voxels across each WCC plane cell (no heuristics), including across chunk seams.
- Skylight dropoff increased: per‑micro step attenuation is 32 (sharper falloff), block light uses 16.
- Committed to Micro S=2 only (legacy mode removed; no toggle).
- Shared sealed‑plane predicate implemented for S=2 and used by mesher + lighting.

---

## Current State

### Engine selection
- File: `crates/geist-lighting/src/lib.rs`
  - Mode toggle removed. `compute_light_with_borders_buf(...)` always uses the Micro S=2 engine.
  - App hotkey `M` and `Event::LightingModeToggled` removed.

### Micro S=2 lighting engine
- File: `crates/geist-lighting/src/micro.rs`
- Grid: Micro resolution with dims `(mxs, mys, mzs) = (2*sx, 2*sy, 2*sz)`.
- Occupancy: A micro voxel is solid if the macro block is a full cube, or if its occupancy bit (S=2) is set.
- Seeds:
  - Skylight: “open‑above” scan per micro column (top→down) within the chunk.
  - Neighbor seams: prefers per‑micro seam planes from neighbors; falls back to upsampling coarse planes.
  - Block emitters: seed interior air micro voxels (two per axis) of the macro cell.
- Propagation:
  - BFS with integer attenuation per micro step.
  - Skylight attenuation: 32 per micro step.
  - Block light attenuation: 16 per micro step.
  - Steps are blocked by solid micro voxels (occupancy/full cube).
- Borders (seam exchange):
  - Publishes per‑micro planes (`MicroBorders`) for −X/−Y/−Z ownership to `LightingStore`.
  - Neighbor retrieval (`get_neighbor_micro_borders`) maps +X/+Z from neighbors’ −X/−Z respectively. (Vertical not chunked here; Y planes are carried but unused.)
- Downsample to macro:
  - Produces a `LightGrid` by max over each 2×2×2 block for `skylight` and `block_light` (legacy‐compatible fields).
  - Attaches micro arrays (`m_sky`, `m_blk`) and neighbor micro planes to the `LightGrid` for face shading and seam sampling.

### Face shading (WCC‑aligned)
- File: `crates/geist-lighting/src/lib.rs` → `LightGrid::sample_face_local_s2(...)`
- If micro arrays are present:
  - For each of the 4 plane micro cells on the face, read the two micro voxels across the plane (local + neighbor side) and take the max.
  - When neighbor is out of bounds, sample from the attached neighbor micro seam plane.
  - Take the max across the 4 cells. Finally, max with legacy beacon level at the macro cell (micro beacons pending).
- If micro arrays are absent (fallback path), uses an S=2‑aware sampler that now consults the shared micro helpers.

### Seam propagation and events
- Coarse `LightBorders` are still emitted and used by the app to trigger `LightBordersUpdated` cascades (neighbor rebuilds). Micro borders ride alongside in `LightingStore` and are consumed by the Micro S=2 engine.
- This means micro‑level seam improvements work transparently without changing the app’s event graph.

### Attenuation defaults
- Legacy skylight: 32 per macro step (unchanged).
- Micro skylight: 16 per micro step (stronger dropoff than the previous scaffold).
- Micro block light: 16 per micro step.

### Toggle & UX
- No runtime toggle; Micro S=2 is the only lighting path.

---

## Known Gaps / Limitations
- Micro beacons not implemented:
  - No micro directional propagation or micro beacon border planes.
  - Faces still include coarse beacon via `beacon_light` at macro resolution.
- Vertical neighbors are not wired (if vertical chunking is introduced in the future):
  - The store supports Y micro planes; seam retrieval currently leaves them None.
- Shared sealed‑plane predicate is not formalized as a single function/API:
  - We consistently gate via micro occupancy/full cubes, but a single predicate shared with the mesher reduces drift.
- Performance tuning:
  - BFS uses basic queues; not yet bucketed (Dial’s algorithm) nor tiled/banded.
  - Micro arrays are byte‑per‑voxel; no nibble‑packing yet.
- Coarse fallback is used when neighbor micro planes are missing:
  - Works fine visually and converges as neighbors arrive, but it’s an approximation until micro planes are present.

---

## Next Steps (Prioritized)

1) Micro beacon lighting
- Add directional micro BFS and per‑micro beacon border planes.
- Mirror current material beam params (straight/turn/vertical costs) at micro scale.
- Attach micro beacon arrays and planes to `LightGrid` for shading where needed.

2) Shared sealed‑plane predicate
- DONE: Added `geist_blocks::micro::{micro_cell_solid_s2, micro_face_cell_open_s2}`.
- Mesher now uses `micro_cell_solid_s2` for local/neighbor micro plane decisions; lighting uses it for S=2 gating and sampling fallback.
- `can_cross_face_s2` now uses `micro_face_cell_open_s2` across the four plane micro cells.

3) Performance + memory
- Switch micro BFS to bucketed queues (Dial’s algorithm) for integer levels.
- Reuse scratch arenas between chunks; consider banding the Y dimension (process 64 micro‑Y at a time) to reduce memory peaks.
- Nibble‑pack micro arrays (`sky`/`blk` → two nibbles in 1 byte) for ~2× reduction.

4) Seam robustness & events
- Option A: Piggyback on existing coarse `LightBordersUpdated` (current).
- Option B: Add a `MicroBordersUpdated` event and cascade neighbor rebuilds specifically for micro changes (optional if A is sufficient).
- Add seam unit tests that assert converged equality across chunk pairs for micro planes.

5) Attenuation and calibration
- Expose attenuation constants via config to tune micro skylight (32) and micro block (16).
- Validate visual parity vs. legacy in open areas; ensure expected darker interiors under overhangs.

6) Vertical seams (optional)
- If/when vertical chunking is introduced, wire Y micro border exchange and sampling (the plumbing is scaffolded).
- Until then, top skylight uses in‑chunk “open‑above” seeding and works well.

---

## Validation Plan

Scenes
- Slab/stair stacks and thin panes along seams (ensure no peeking/leaks).
- Overhang skylight: verify stronger dropoff with micro skylight=32.
- Emissives in corridors at seams; compare legacy vs. micro.
- Tree canopies/leaves: skylight occlusion by micro occupancy.

Metrics
- Build time per chunk (legacy vs. micro), memory peak during build.
- Seam consistency: no cracks or double lighting; equality of stitched planes.
- Visual deltas and user‐perceived shading quality.

---

- Engine dispatcher:
  - `crates/geist-lighting/src/lib.rs`: `compute_light_with_borders_buf(...)` always dispatches to Micro S=2.
- Shared S=2 micro helpers:
  - `crates/geist-blocks/src/micro.rs`: `micro_cell_solid_s2`, `micro_face_cell_open_s2`.
- Micro engine:
  - `crates/geist-lighting/src/micro.rs`: micro BFS, seam seeding, border export, LightGrid micro attachment.
- Face shading:
  - `crates/geist-lighting/src/lib.rs`: `LightGrid::sample_face_local_s2(...)` (micro plane‐cell sampling).
- Seam borders:
  - Coarse: `LightBorders::from_grid` unchanged (still drives neighbor cascades).
  - Micro: `MicroBorders` in `LightingStore`; `get_neighbor_micro_borders` / `update_micro_borders`.
- App:
  - `src/app.rs`: key `M` toggles lighting mode and schedules relight; `LightBordersUpdated` cascades rebuilds.

---

## Risks & Gotchas
- Predicate drift: without a shared `micro_face_open(...)`, mesher vs. lighting could diverge over time — fix with a shared function and tests.
- Missing neighbor micro planes: temporary coarse fallback is fine, but plan to re‑enqueue on neighbor arrival (current coarse cascade suffices).
- Memory spikes: micro arrays are large; mitigate via packing and banding.
- Beacons: ensure micro implementation matches gameplay expectations (directional behavior and costs).

---

## Milestones
1. Micro beacons + planes + shading hook
2. Shared sealed‑plane predicate API + tests
3. Bucketed BFS + scratch reuse + (optional) nibble packing
4. Seam unit tests (coarse + micro) and targeted perf/visual validation
5. Optional: vertical micro seams (if vertical chunking is added)

This plan takes the Micro S=2 path from functional to robust, aligning lighting semantics exactly with the WCC S=2 mesher and removing remaining approximations.

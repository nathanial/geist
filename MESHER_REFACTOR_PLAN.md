Mesher Parity Refactor Plan (WCC v3)

Context
- Current mesher (WCC v2) builds face parity by toggling per-cell spans via toggle_{x,y,z} while scanning blocks and seams.
- It then greedy-emits rectangles per plane from parity/material grids.
- Recent optimizations improved latency, but complexity grew and a change caused missing faces, so we reverted it.
- Seed and scan dominate runtime; toggling is branchy and touches parity grids many times.

Goal
Replace the toggle-based parity builder with a cache-friendly, linear pass that derives face parity by XOR’ing a dense micro-occupancy bitfield along axes. Preserve visual output (watertight, seam-free), determinism, and existing public APIs.

Progress (Sept 2025)
- Implemented dual-occupancy and dual-face-grid meshing in ParityMesher:
  - Solids occupancy (`occs`) contains only full cubes and micro shapes; water is excluded.
  - Water occupancy (`occs_water`) is maintained separately with its own -X/-Z seam layers.
  - Parity/material computation produces two grid sets: `grids` (solids) and `grids_water` (water).
  - Water faces are emitted only where water borders air (skip water–solid boundaries). Solids emit where solids border non-solids (including water).
  - Emission order: solids first (opaque), then water (transparent). Renderer unchanged; it already draws water in a later pass.
- Corrected ordering in occupancy and seam seeding so water is handled before the full-cube branch. This prevents water from being marked as solid and restores terrain faces under water.
- Scratch reuse: added separate thread‑local pools for water grids/occ; no allocator churn at steady state.
- Performance impact: single scan maintained; parity/emission adds a small water pass. Early measurements show similar scan/seed times and a minor emit increase proportional to water surface area.

Remaining Work
- Fuse solids+water parity into a single per‑axis loop to shave a small constant from parity time.
- Unit tests for water semantics and seam stitching across chunk borders (ensure only water–air faces, and solid faces appear under water).
- Property tests comparing total rect area vs v2 for non‑water scenes (regression guard).
- Optional feature flag to toggle v3 path off/on during rollout (if needed).
- Benchmarks capturing scan/seed/emit breakdown with and without significant water coverage.

Overview
1. Build a dense S-resolution occupancy bitset O[x,y,z] for the chunk interior.
2. Populate seam occupancy for a one-cell overscan on -X and -Z using neighbor world+edits.
3. Compute face parity and ownership in one pass via XOR along axes to produce PX/PY/PZ and orientation OX/OY/OZ.
4. Compute face material IDs KX/KY/KZ using the owner side’s block and precomputed material caches.
5. Greedy-emit planes as we do today (no algorithmic change there).
6. Keep thin-shape pass (pane/fence/carpet) separate as today.

Data Structures
- Micro occupancy bitset Occ: dimensions (S*sx, S*sy, S*sz), 1 bit per micro voxel.
  - Example at S=2 over 32×256×32: 64×512×64 = 2,097,152 bits ≈ 256 KiB.
- Seam occupancy buffers: one micro layer for -X and -Z; only the outer layer is needed.
- Face grids (reuse existing): parity PX/PY/PZ, orientation OX/OY/OZ, material KX/KY/KZ.
- Thread-local scratch: pre-allocated Occ and face grids to avoid reallocation across chunk jobs in a worker thread.

Algorithm
1) Build occupancy Occ
- For each block cell (x,y,z):
  - If air → continue.
  - If full cube (or AxisCube) → set all S×S×S micro voxels to 1.
  - Else if micro occupancy variant present → set the micro boxes derived from the 8‑bit occupancy mask.
  - Else if water → do not mark occupancy here; water handled via parity filtering below.
- Memory layout: linearized XYZ (X-fastest recommended) for cache-friendly XOR passes.

2) Seam occupancy
- -X boundary: for each (y,z), sample neighbor at (base_x−1, y, base_z+z) into the virtual micro layer at ix=−1:
  - Full cube → 1 for both micro cells.
  - Micro shapes → bit from neighbor’s 8‑bit occupancy mask (mx=1).
  - Water → treat as 0 (to preserve “water only vs air”).
- -Z boundary: analogous with neighbor at (x, y, base_z−1) and mz=1.

3) Parity and ownership via XOR
- For X faces: PX[ix,iy,iz] = Occ[ix−1,iy,iz] XOR Occ[ix,iy,iz]
- Owner OX[ix,iy,iz] = Occ[ix,iy,iz] (true means +X side, else −X)
- Analogous for Y/Z.
- Use 0 for out-of-range except the virtual seam layers at -X/-Z.

4) Material assignment (KX/KY/KZ)
- For parity=1, pick material from the owner’s block and face role (PosX/NegX etc.).
- Use precomputed caches material_for_cached for top/bottom/side.
- Water: keep a separate IsWater bitset from the scan; when owner is water and neighbor is not air, skip the face to match current behavior.

5) Greedy emission
- Keep current greedy merge per plane using PX/PY/PZ, OX/OY/OZ, and KX/KY/KZ.

Feature Gating and Rollout
- Add cargo feature parity_mesher (off by default).
- Keep both paths: v2 (toggle-based) default, v3 (parity) behind feature flag.
- Optional CLI flag to switch at runtime for A/B testing.

Status: v3 water/solid split is implemented and enabled along the v3 path; gating is still optional/TBD depending on rollout strategy.

Implementation Plan (Incremental)
1. Skeleton + scratch
   - Introduce MesherScratchV3 with Occ, optional IsWater, seam layers; add thread-local pool.
   - Wire v3 builder under cfg(feature = "parity_mesher") with logging only.
2. Occupancy build
   - Implement build_occupancy_s(buf, reg) to fill Occ and IsWater; log ms_occ_build.
3. Seam layers
   - Implement seed_seam_layers(world, edits); log ms_seam_layer.
4. Parity + orientation + materials
   - Axis XOR passes to fill PX/PY/PZ, OX/OY/OZ, KX/KY/KZ; log ms_parity and ms_material.
5. Emit integration
   - Plug v3 FaceGrids into existing greedy emit; validate output in dev scenes.
6. Water semantics
   - Integrate IsWater to allow only water/air faces; validate no occlusion behind water.
7. Bench + compare
   - Side-by-side perf vs v2; capture scan/seed/emit and totals; assess memory churn.
8. Cleanup and default
   - After sign-off, consider making v3 default and keep v2 as fallback feature.

Complexity and Memory
- Occ ≈ 256 KiB at S=2 for 32×256×32.
- FaceGrids sizes unchanged from v2.
- Thread-local scratch avoids reallocation pressure across many chunks.
- Passes are linear and branch-light; friendly to prefetch/vectorization.

Pseudocode (X axis)
for ix in 0..S*sx+1:
  for iy in 0..S*sy:
    for iz in 0..S*sz:
      a = occ(ix-1,iy,iz)    // 0 for ix==0; use seam layer for -X
      b = occ(ix,iy,iz)
      p = a ^ b
      px[idx_x(ix,iy,iz)] = p
      if p:
        owner_pos = (b == 1)
        ox.set(idx, owner_pos)
        (bx,by,bz,face) = owner_pos ? (ix,iy,iz, PosX) : (ix-1,iy,iz, NegX)
        (wx,wy,wz) = micro_to_world_block(bx,by,bz)
        here = buf.get_local(wx,wy,wz)
        // Water skip: if here is water and neighbor not air, continue
        kx[idx] = material_for_cached(here, face)

Test Plan
- Unit tests: micro occupancy expansion; parity for tiny patterns; seam correctness; water faces only against air.
- Property tests: random 8×8×8 chunks compare v2 vs v3 rect counts/areas per axis.
- Visual: regression scenes mixing micro shapes, water, tall terrain.

Risks and Mitigations
- Off-by-ones at boundaries → dedicated seam tests for ix==0/iz==0.
- Water semantics → separate IsWater; explicit skip rules.
- Material ownership ambiguities → owner side defined by occ==1; test coverage.
- Memory use → thread-local scratch reuse.

Perf Expectations
- Significant reductions in seed_ms and scan_ms vs v2 by eliminating per-cell toggles and hashing overhead.
- Emit remains low due to greedy merge and coherent K maps.

Acceptance Criteria
- No cracks or duplicate faces; visual parity with v2.
- Total meshing time improved; seed+scan reduced ≥30% on typical scenes.
- Stable under high load without allocator churn.

Rollback Plan
- Feature-flag guarded; switch back to v2 quickly if issues appear.
- Keep v2 code path until v3 fully validated.

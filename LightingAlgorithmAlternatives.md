# Micro‑Voxel Lighting (S=2) — WCC‑Aligned, Non‑Heuristic

## Motivation
- Ensure lighting semantics match the WCC S=2 mesher exactly, eliminating quadrant sampling bias, leaks through sealed micro cells, and seam inconsistencies.
- Provide a model with clear mathematical guarantees: propagation is a shortest‑path (or monotone attenuation) problem on a finite graph with watertight gates defined by geometry.

## High‑Level Overview
- Grid: Replace voxel‑center light with an S=2 micro‑voxel grid (2× each axis → 8× voxels). For a 32×256×32 chunk, micro dims are 64×512×64 (≈2.1M cells).
- Graph: Each micro voxel is a node. Edges exist only across open micro faces determined by the same sealed‑plane predicate the WCC mesher uses (watertight by construction).
- Sources: Skylight injects from the top boundary via micro +Y faces open to sky; emissive blocks inject via their exposed faces or designated micro voxels.
- Solve: Run integer bucketed BFS (or Dijkstra if weights differ) per channel. Result is deterministic and order‑independent.
- Shading: For each WCC face cell (2×2 per macro face), sample the two adjacent micro voxels across that plane cell and combine (typically max). No neighborhood heuristics.

## Guarantees
- Soundness: Light never crosses sealed faces (identical gates as mesher).
- Completeness: All and only micro voxels connected to a source through open faces receive light.
- Determinism: Monotone, order‑independent solution (least fixed point of the propagation operator).
- Seam correctness: Half‑open ownership and micro border exchange make chunk‑local results equal to a global solve.

## Data Model
- Micro occupancy: Boolean per micro voxel (air vs solid) or per micro face open/closed. Derive from the same S=2 shape sampling the mesher uses. Prefer a shared function to avoid drift.
- Light channels: At minimum `sky` and `emissive` (block light). Optional RGB per emissive if needed by art style.
- Representation: Integer light level L in [0..15] or [0..31]. Use nibble packing to control memory if needed.
- Layout: Flattened 3D array for micro light, e.g., `idx = ((y * Zm) + z) * Xm + x` with `Xm=2*X`, `Ym=2*Y`, `Zm=2*Z`. Keep a matching layout for occupancy if stored.

Memory notes
- Raw micro light (1 byte/voxel/channel) at 64×512×64 ≈ 2.1 MB/channel. With two channels (sky+emissive) ≈ 4.2 MB/chunk.
- Nibble packing (4 bits/channel) allows `sky`+`emissive` in 1 byte → ≈ 2.1 MB/chunk.
- Ephemeral compute: Allocate during build, then persist only border micro planes (for neighbor exchange) + any gameplay field you still need.

## Sealed‑Plane Predicate (Shared With WCC)
- Single source of truth: Implement `micro_face_open(a_voxel, b_voxel, face)` using the exact S=2 occupancy the mesher uses to prove watertightness.
- Properties required for correctness:
  - Antisymmetry: `open(a→b) == open(b→a)`.
  - Locality: Decision depends only on geometry of the two incident macro blocks at the micro interface.
  - Consistency: If all four micro plane cells on a macro face are closed, the mesher must emit no face there; lighting must also block.

## Borders and Seam Ownership
- Half‑open rule: This chunk owns −X, −Y, −Z boundary faces; +X, +Y, +Z are owned by the neighbor. Only the owner writes to a seam plane.
- Exchange format: For each of 6 faces, provide the micro border plane of light for each channel (size: `Ym×Zm` on X faces, `Xm×Zm` on Y faces, `Xm×Ym` on Z faces).
- Build order independence:
  - When building a chunk, read available neighbor border planes for +X/+Y/+Z as seeds. If missing, treat as zero for emissive; for skylight at world top, inject full sky via a virtual +Y plane.
  - After compute, publish −X/−Y/−Z planes for neighbors. Optionally schedule a cheap “seam relax” pass when new neighbor data arrives.

## Propagation Algorithm
- Use an integer bucketed BFS (Dial’s algorithm) per channel with non‑increasing levels:
  - Typical rule: Neighbor receives `max(nei, src - cost)`, with `cost = 1` per micro step. For distance‑based falloff use anisotropic costs (then prefer 0–31 range).
  - Because updates are monotone and bounded, a bucketed queue over light levels converges quickly and deterministically.

Pseudocode (single channel)
```
init all L = 0
queue buckets[0..Lmax]

// Seed skylight from +Y border and interior sources
for each seed s: set L[s] = max(L[s], seed_val); push s to bucket[seed_val]

for level from Lmax down to 1:
  while bucket[level] not empty:
    v = pop()
    if L[v] != level: continue // stale entry
    for each dir in 6:
      if not micro_face_open(v, v+dir, dir): continue
      n = v+dir
      new = level - cost(dir) // usually 1
      if new > L[n]: L[n] = new; push n to bucket[new]
```

Parallelization
- Process buckets per level to avoid race conditions; or use lock‑free multi‑producer queues per level. Micro occupancy is read‑only, L is updated via monotone writes guarded by the level check above.

## Sources
- Skylight: Seeds live on micro voxels whose +Y face is open to the external sky. Practically:
  - Read +Y neighbor border plane if present and enqueue those voxels just below the plane where the +Y micro face is open.
  - For the topmost world chunk, synthesize a “sky plane” with level = `Lmax`.
- Emissive blocks: Prefer surface emission for correctness with WCC:
  - For each exposed micro face cell of an emissive block, inject into the adjacent air micro voxel with the block’s emission level.
  - Optionally also seed the block’s interior micro voxels if you want center‑emission behavior.

## Shading of WCC Faces
- For each emitted WCC face micro cell (the 2×2 grid on a macro face):
  - Identify the two adjacent micro voxels separated by that plane cell (call them `A` and `B`).
  - Face light = `combine(L[A], L[B])` with `combine = max` for non‑directional light.
  - This guarantees no peeking across sealed micro cells and aligns 1:1 with merge cells.

## Integration With Current Codebase
- Current state (observed):
  - Voxel‑center light grid with BFS and S=2‑aware gates like `can_cross_face_s2`/`skylight_transparent_s2`.
  - Face shading uses `LightGrid::sample_face_local_s2` with multi‑sample neighborhood (recently made symmetric to remove bias).
  - Border data exists for skylight (`sk_xn/xp/zn/zp`) at voxel resolution; Y borders handled specially.
- Changes required:
  - Add micro occupancy provider shared with WCC (S=2). Either compute on demand from block shapes or precompute a packed bitset per chunk.
  - Introduce micro light storage (ephemeral scratch) and border micro planes per face per channel.
  - Replace voxel‑center BFS with micro‑voxel BFS for `sky` and `emissive`.
  - Replace `sample_face_local_s2` usage for WCC faces with the two‑voxel max described above.
  - Unify sealed‑plane predicate used by mesher and lighting; deprecate heuristic sampling paths.

## Gotchas and How To Avoid Them
- Predicate drift: If lighting and mesher use different S=2 openness logic, cracks/leaks reappear. Fix by moving the predicate to a shared module and testing it.
- Border mis‑ownership: Writing/reading the wrong seam side causes double‑counting or gaps. Enforce −X/−Y/−Z ownership with unit tests over chunk pairs.
- Missing neighbor data: Build might run without some neighbors. Treat missing +X/+Y/+Z planes as zeros (and sky for +Y at world top) and schedule a seam‑relax when neighbors arrive.
- Memory spikes: 2.1 MB/channel scratch can be heavy. Use nibble packing, arena reuse, or tile the chunk vertically (process bands of, say, 64 micro Y at a time) with streaming borders between bands.
- Performance regressions: BFS on 2.1M cells is fast if most are solid. Ensure early culling by never enqueuing solid voxels and using tight cache‑friendly layout.
- Non‑manifold micro shapes: WCC must guarantee watertightness at S=2. Add assertions that every closed macro face implies all four plane cells are closed and vice versa.

## Step‑By‑Step Migration Plan
- Step 1: Extract the sealed‑plane predicate into a shared `wcc::s2::micro_face_open()` used by both mesher and lighting.
- Step 2: Provide a micro occupancy view for a chunk: API to test micro voxel solid/air and micro face openness, backed by per‑block S=2 tables.
- Step 3: Implement bucketed BFS over the micro grid for `sky` and `emissive`. Accept neighbor micro border planes as optional inputs; emit −X/−Y/−Z planes as outputs.
- Step 4: Replace WCC face shading path to read exactly the two adjacent micro voxels per plane cell and combine.
- Step 5: Delete heuristic multi‑sample codepaths for WCC surfaces. Keep a legacy path only for non‑WCC meshes if needed.
- Step 6: Add tests: slab/stair stacks, thin panes, overhang skylight, emissives in corridors, and staggered chunk seam cases.

## Validation
- Scenes: slab/stair stacks, pane corridors with emissives, tree canopies (leaf skylight occlusion), cliff overhangs, and chunk seams with staggered loads.
- Metrics: build time per chunk, peak memory, merge stability, seam consistency (no cracks/doubles), visual diffs vs current.

## Summary
- Micro‑voxel lighting at S=2 is the fully non‑heuristic, theoretically clean approach aligned with WCC. It removes directional bias and leakiness by construction, yields deterministic results across seams, and integrates cleanly with WCC face shading. The main tradeoff is memory/compute, which can be mitigated via nibble packing and ephemeral allocation during builds.

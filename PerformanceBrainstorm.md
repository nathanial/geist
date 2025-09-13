Performance Brainstorm: Speeding Up compute_light_with_borders_buf_micro

Goal
- Reduce the runtime of `compute_light_with_borders_buf_micro` by 3–10x so meshing dominates again during chunk builds.
- Keep visual quality close to current Micro S=2 behavior, but consider controlled approximations where impact is minimal.

Context (today)
- Domain size (typical): 32×256×32 macro → 64×512×64 micro ≈ 2.1M micro voxels.
- Current algorithm: micro BFS with per-step attenuation, dial queue (16 buckets), occupancy bitset, boundary-only skylight seeding, seam pruning + GenCtx reuse. Still the top hotspot.

Strategy Buckets (ordered by likely ROI)

1) Bitset Wavefront Propagation (SIMD/bit-parallel BFS)
- Idea: Represent the frontier as bitsets (one bit per micro voxel). Instead of pushing neighbors individually, advance a whole wave by bitwise shifts + masks in the six directions.
- Mechanics:
  - Maintain a `level` array (u8 max per cell) and for each wave value `L`, have a frontier bitset `F_L` of voxels set to `L` this iteration.
  - For each axis: compute neighbor candidates by shifting `F_L` by ±1 cell in that axis and AND with “not-solid & (level < L-atten)” masks; OR them into next frontier `F_{L-atten}`.
  - Repeat until frontiers empty; update `level` with `L-atten` for all newly reached bits in a single pass.
- Pros: Massive reduction in per-cell branching and queue traffic; leverages CPU word (u64) parallelism, amenable to AVX2/NEON.
- Cons: Complexity (boundaries between 64-bit words, spill masks), memory for multiple frontiers.
- Est. Speedup: 2–5x on BFS core.

2) Parallel Per-Bucket Processing (Rayon)
- Idea: Process each bucket’s frontier in parallel (all items have same “distance class”). Use per-thread output buffers/frontiers, then merge.
- Mechanics:
  - For queue-based BFS: iterate pop loop per-bucket; drain current bucket into a slice; `par_iter()` that slice to generate next buckets (thread-local), then concatenate.
  - For bitset BFS: per-level bitsets can be split by word-range for `par_chunks_exact_mut`.
- Pros: Scales with cores, limited synchronization (per-bucket phases).
- Cons: Threading overhead, careful merges; but BFS is big enough to amortize.
- Est. Speedup: 2–4x on 8 cores.

3) Macro+Micro Hybrid Solver (coarse BFS with micro face-gating)
- Idea: Return to macro-resolution BFS for propagation, but decide whether crossing a face is allowed using S=2 micro occupancy (as we do today). The interior of macro cells doesn’t need a full micro BFS fill.
- Mechanics:
  - Maintain macro `block_light`/`skylight` grids; for a step from A→B, check if any of the 4 micro face cells is open. Attenuation per macro step becomes `MICRO_ATTEN` (1 or 2 micro steps depending on face’s openness/topology).
  - For render-time sampling of micro (e.g., face lighting), rely on micro border planes and local sampling rather than full micro array.
- Pros: Order-of-magnitude work reduction (64×512×64 → 32×256×32 graph). Visual difference small in most shapes; correctness preserved at seams via micro planes.
- Cons: Loses intra-macro micro gradients; may affect very thin‐feature lighting.
- Est. Speedup: 3–6x overall; biggest “pragmatic” win.

4) ROI-Limited Micro BFS (focus where micro matters)
- Idea: Run micro BFS only near non-full-cube blocks and near seams; elsewhere, use macro shortcuts (skylight column fill, solid interiors).
- Mechanics:
  - Build a mask: micro cells within K micro steps of any non-full-cube macro cell or seam plane; everything else uses macro-level fill.
  - BFS only inside ROI; for outside ROI, set light analytically (e.g., skylight 255 for open-above, block light decays via macro BFS).
- Pros: Preserves micro detail where it matters; reduces total micro cells touched.
- Cons: Needs a robust ROI estimator; correctness at ROI borders must be handled.
- Est. Speedup: 2–4x depending on terrain.

5) Tile-Based BFS with Local Frontiers
- Idea: Split the micro grid into 8×8×8 tiles; do BFS per-tile using compact local queues/bitsets; exchange tile boundary frontiers per level.
- Mechanics:
  - Improves cache locality and enables parallel per-tile processing in waves.
  - Each tile maintains a mini frontier; global orchestrator ticks levels and exchanges halos.
- Pros: Better cache, natural parallelism; pairs well with bitset wavefronts.
- Cons: More complex orchestration; need deterministic merges.
- Est. Speedup: 1.5–3x on top of other wins.

6) GPU Compute Shader Offload (wgpu)
- Idea: Implement the micro light propagation as 3D compute passes on GPU.
- Mechanics:
  - Ping-pong 3D textures for light values; per-discrete step kernel applies neighbor relaxations with attenuation and solidity masks. Iterate until convergence or fixed step budget.
  - Upload occupancy and seeds; download final arrays and planes.
- Pros: Potentially 10x+ speedup; frees CPU for meshing.
- Cons: Significant engineering; GPU availability/permissions; async orchestration. Precision/ordering must match CPU path or be “close enough.”
- Est. Speedup: 5–15x BFS core.

7) Skylight Specialized Solver (no BFS for open sky)
- Idea: Separate skylight into 2 parts: column fill (255 until blocked) and local lateral spread under overhangs/caves; constrain lateral skylight BFS to a small band below overhang ceilings.
- Mechanics:
  - Precompute per-(x,z) overhang height; only allow lateral skylight propagation for y ∈ [overhang_y - W, overhang_y + H], with small windows W/H.
- Pros: Skylight becomes almost free on open terrain.
- Cons: Needs careful handling around complex cave systems; window size affects quality.
- Est. Speedup: 2–3x on skylight share (scene-dependent).

8) Far-Field Attenuation Shortcuts
- Idea: For omni block lights, after D micro steps, contribution falls below threshold. Prune BFS beyond a radius per-seed (especially for low emitters).
- Mechanics:
  - Track “max reach” by level; abort propagation once v < MIN_VISIBLE.
- Pros: Trims tails; ideal for small emitters.
- Cons: Already partially implicit; formalizing yields clearer bounds.
- Est. Speedup: small-to-medium; minimal complexity.

9) Data Layout + SIMD Tightening
- Idea: Ensure arrays are cache- and SIMD-friendly.
- Mechanics:
  - Z-order or Y-major if it matches access patterns; align to 64 bytes; remove iterator overhead (manual `u8::max`); add `#[inline(always)]` to tiny helpers.
- Pros: “Free” perf; small increments that add up.
- Cons: Diminishing returns; interacts with other approaches.
- Est. Speedup: 1.1–1.3x.

10) Seam Batching and Caching
- Idea: Compute lighting for 2×2 chunk quads at once to avoid `world.block_at_runtime` on borders entirely.
- Mechanics:
  - Stage boundary blocks and micro planes into a shared cache; run seeding and gating across the quad without world callbacks.
- Pros: Removes expensive per-line world lookups; smooths cache access.
- Cons: Scheduler changes; memory management for staging.
- Est. Speedup: 1.2–1.8x in generation-heavy paths.

11) Incremental/Temporal Reuse (when possible)
- Idea: If chunk generation is steady and neighbor borders repeat (e.g., same biomes), cache and reuse lighting artifacts.
- Mechanics:
  - Hash chunk inputs (registry revision, worldgen params, seam planes); memoize micro borders and/or macro fields.
- Pros: Huge wins for repeated content or similar tiles.
- Cons: Cache invalidation, memory trade-offs; scene-dependent.
- Est. Speedup: Scene-dependent; potentially very large.

Prioritized Roadmap (pragmatic)
- Phase A (1–2 weeks)
  1. Parallel per-bucket processing (Rayon) on current queue path.
  2. Downsample tightening + more seam pruning (per-line micro seed precheck).
  3. Prototype macro+micro hybrid solver behind a feature flag; benchmark correctness/quality vs speed.

- Phase B (2–4 weeks)
  4. Bitset wavefront prototype for skylight only (simpler), then generalize to block light.
  5. Tile-based BFS (8×8×8) with per-tile queues; optional parallel.

- Phase C (4–6+ weeks)
  6. GPU compute prototype (wgpu) for skylight, then block light; add fallback.
  7. ROI-limited micro BFS + macro shortcuts for non-ROI.

Validation Plan
- Add Criterion benches for: flat, normal worldgen, cave-heavy synthetic, and neighbor-heavy seam cases.
- Add counters: nodes popped, pushes skipped (no improvement), solid checks performed, seeds processed per seam.
- Profile with cargo-flamegraph + perf on Linux; compare “BFS core” vs “seam seed/gate” vs “skylight seeding.”

Risk/Quality Notes
- Hybrid solver and ROI-limited BFS introduce approximation; we should gate by feature flag, add A/B tests (existing unit tests + visual comparisons).
- Bitset wavefronts must be precise about boundaries and avoid tearing; start with skylight (simpler monotonic rules), then omni.
- GPU path must be an optional backend with identical or near-identical results; deterministic order is less important than convergence and equivalence.

Rough Impact Estimates (cumulative if combined)
- Bitset wavefronts: 2–5x BFS
- Parallel buckets (8 cores): 2–4x
- Hybrid macro+micro: 3–6x overall (with small quality tradeoffs)
- Skylight specialization: 2–3x of skylight share
- Combined (CPU path): realistic 4–8x; with GPU, 8–15x

## Appendix — Hybrid Solver Post‑Mortem (Issues & Lessons)

Context
- We prototyped a Macro+Micro Hybrid Solver: macro‑grid BFS using S=2 micro face‑gating, micro neighbor planes for seams, and micro planes synthesized by upsampling macro fields.
- The intent was to keep seam/micro face correctness while dramatically reducing BFS work.

Symptoms Observed (real scenes)
- Dark X/Z faces next to thin geometry (e.g., carpet), despite correct mesh geometry.
- Carpet blocking skylight incorrectly (or not blocking when it should) under different revisions.
- Stairs next to full cubes causing unexpected darkening, sensitive to orientation/top/bottom state.
- Intermittent “lightless faces” near seams; behavior depended on whether coarse vs micro planes seeded.
- Unit tests passed, but visual regressions still appeared in complex arrangements.

Primary Causes
- Mixed semantics across phases:
  - Seeding vs propagation used different rules (coarse blocks_skylight vs micro S=2 gates), causing non‑monotonic behavior.
  - Vertical vs lateral moves treated inconsistently; thin full‑plane blockers (carpet) should block vertical skylight but allow lateral where the micro plane is open.
- Loss of micro detail:
  - Upsampling macro results to synthesize micro arrays removed intra‑macro gradients; face sampling relying on micro neighbors magnified edge cases.
- Gating logic drift:
  - S=2 face openness is “plane not sealed” (any of the four micro cells open). We mixed that with coarse passability checks in some paths.
  - Target transparency checks (skylight) were sometimes coarse, sometimes S=2, breaking equivalence with the micro BFS.
- Seeding details matter:
  - Coarse neighbor seeds need precise attenuation to match micro semantics across the boundary and first interior step.
  - Over‑eager queuing at edges created subtle priority/ordering differences.

Lessons Learned
- Maintain a single, precise contract for each axis of variation:
  - Plane crossing gate = micro_face_cell_open_s2 (not sealed).
  - Target skylight passability = S=2: full cubes block; shapes with occ8 are transparent for skylight BFS if the crossing plane is open.
  - Target block‑light passability = propagates_light.
  - Vertical vs lateral skylight: use the same S=2 transparency; let the face gate decide enterability. Full‑plane thin blocks (carpet) end up blocking vertical by lacking an open Y‑plane micro cell.
- Keep seeding and propagation consistent with the same gates and attenuation. Do not special‑case one without the other.
- Be wary of synthesizing micro arrays from macro fields; sampling logic expects real micro structure. If synthesis is required, bound its use (e.g., sampling fallback only, not as a source of truth).
- Tests must cover thin‑shape edge cases:
  - Carpet under a full cube; carpet next to a full cube; mixed seams.
  - Stairs (all facings, top/bottom) next to full cubes and in stacks.
  - Seams with both coarse and micro planes present; override precedence.
  - Visual “goldens” for lighting around thin geometry.
- Add debug tooling: render micro plane openness overlays and “why blocked” annotations.

Safer Path Forward
- Prefer Bitset Wavefront BFS (SIMD) at micro scale for skylight, keeping exact semantics, and layer performance optimizations around it (tiling, parallel buckets).
- If adopting Hybrid again:
  - Keep pure S=2 rules for both crossing and target checks.
  - Treat coarse planes only as seeds (with carefully matched attenuation), never as gates.
  - Avoid synthesizing full micro arrays; instead, compute and cache only the boundary planes from local micro passes or from neighbor planes.
  - Introduce feature flags and A/B switch to compare visuals easily.
- Consider ROI‑limited micro BFS near thin shapes and seams while using macro shortcuts elsewhere; define ROI deterministically to preserve semantics.

Revert Plan (now)
- Return to the previous Micro S=2 BFS path (compute_light_with_borders_buf_micro) to restore lighting correctness.
- Retain safe performance wins (e.g., seam gating improvements, occupancy bitsets, queue tightening) if they don’t alter semantics.

Checklist Before Next Attempt
- Codify the S=2 lighting contract in tests (unit + scenario goldens).
- Add per‑axis gates and target transparency helpers used uniformly by seeding and propagation.
- Instrument with counters (gates passed/failed, seeds enqueue, neighbor overrides) and enable on hotkeys.
- Provide a runtime toggle (Micro vs Hybrid) and a visual overlay of micro face openness.
- Benchmark both correctness and speed on curated scenes (flat, caves, stairs fields, carpets, mixed seams).

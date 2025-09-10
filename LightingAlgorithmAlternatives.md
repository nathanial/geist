# Lighting Algorithm Alternatives (WCC‑Friendly)

## Motivation
- Current voxel BFS + heuristic face sampling introduces directional bias, seam timing artifacts, and micro‑shape inconsistencies.
- The Watertight Cubical Complex (WCC) mesher works at S=2 micro resolution and emits only the world’s surface. An ideal lighting method should align to that frontier, avoid approximations, and have clean chunk‑seam behavior.

## Goals
- Visual correctness on WCC faces (no quadrant bias, no leaks through sealed micro cells).
- Seam‑safe: half‑open plane ownership, deterministic neighbor exchange.
- Reasonable memory and compute for 32×256×32 chunks.
- Works for skylight and block lights; optional directional beams.

## Option A — WCC‑Frontier Flood (Surface BFS)
Description
- Treat WCC plane micro‑cells (2×2 per voxel face) as the graph. Edges connect adjacent plane cells on the same plane and around edges where geometry allows. Light sources “inject” onto surface cells they touch (sky on top surfaces, blocks on their incident faces). Propagate along surface cells with attenuation (per step or per metric distance).

How it integrates with WCC
- Perfectly: the mesher already iterates plane micro‑cells to merge quads. Shade directly from the surface scalar on each plane cell; merges remain stable because lighting is computed at the same resolution as the merge grid.

Seams
- Use half‑open rule. For −X/−Z boundary planes owned by this chunk, optionally ingest neighbor surface values via micro border planes. No duplication on +X/+Z. Deterministic and crack‑free.

Pros
- Alignment with geometry eliminates sampler artifacts entirely (no peeking across closed micro cells).
- Minimal memory: compute on demand per build only for visible frontier; no volumetric arrays needed.
- Naturally supports variable attenuation profiles per light type.

Cons
- Propagation is limited to surfaces; interior volumetric light is not stored. This is fine for rendering (hidden interiors are not emitted), but gameplay systems that require interior light might still need a coarse field.
- Implementation needs a clean, shared “plane‑cell open” predicate and consistent edge connectivity rules.

Complexity
- Moderate. Build/visit surface cell graph while WCC emits or in a prepass. Use a small queue per face plane. Cost scales with exposed area, not volume.

## Option B — Micro‑Voxel Lighting (S=2)
Description
- Store light per micro voxel (2× on each axis → 8× voxels). BFS propagates through voxel faces using the same open/closed predicate as WCC micro occupancy. Face shading reads the two voxels across the face cell and takes the max.

How it integrates with WCC
- Exact semantics match. No sampling heuristics; every plane cell is the frontier between two micro voxels.

Seams
- Exchange micro border planes per neighbor; same half‑open ownership as WCC to prevent double counting.

Pros
- Highest fidelity and simplest conceptual model; eliminates all the current artifacts.

Cons
- Memory/time overhead: ~8× the voxel count; for three channels this can be 2–6 MB per chunk unless compressed or computed ephemerally. Needs careful scheduling.

Complexity
- Higher. Requires changes to storage, propagation, neighbor exchange, and possibly streaming.

## Option C — Dual‑Grid BFS (Voxel Centers + Plane Gates)
Description
- Keep a voxel‑center light grid, but edges between voxels are allowed only where the specific plane micro‑cell is open (same predicate WCC uses). For face shading, evaluate light as max(local, neighbor) but only if the corresponding plane cell is open.

How it integrates with WCC
- Good. Uses WCC’s sealed‑plane logic; face shading consults the exact plane cell openness.

Seams
- Same neighbor border planes as today, but micro‑aware for skylight/block light.

Pros
- Close to current design; lower memory than micro‑voxel grid; removes most leaks because the gate matches WCC.

Cons
- Still volumetric; stores light where it may not be needed (hidden interiors). Some residual approximation when multiple micro cells exist across a face and one side dominates.

Complexity
- Moderate. Needs a unified sealed‑plane function and a face‑cell gate in both propagation and sampling.

## Option D — Column Skylight + Layer Lateral Pass
Description
- For skylight only: compute vertical visibility per (x,z) column with micro occupancy. Then, per Y‑layer, run a 2D lateral flood constrained by micro openness to spread skylight horizontally with controlled attenuation.

How it integrates with WCC
- WCC tops receive sky values exactly; lateral spread remains layer‑local and micro‑aware.

Seams
- Exchange per‑layer skylight micro border stripes. Deterministic with half‑open rule.

Pros
- Memory light; removes odd “side bleed” patterns while keeping expected skylight look. Keeps block light system unchanged.

Cons
- More special‑case logic (skylight path differs from block light). Still an approximation vs A/B.

Complexity
- Low–moderate.

## Option E — Ray‑Based Face Shading (Skylight‑Only)
Description
- For each WCC face cell, cast a handful of discrete rays upward/outward through micro occupancy to test sky visibility and accumulate brightness. No BFS field; shade faces directly.

How it integrates with WCC
- Natural: rays run through the same micro grid WCC uses; no volumetric storage.

Seams
- Rays traversing chunk borders can use neighbor micro border occupancy/light or a limited horizon.

Pros
- Eliminates BFS artifacts; computation limited to visible faces; trivially parallel.

Cons
- Stochastic artifacts unless rays are carefully chosen; harder to match non‑skylight sources; caching needed for stability.

Complexity
- Moderate.

## Option F — Logical Light + Surface AO
Description
- Keep a strict, simple logical light (e.g., vertical‑only skylight, basic block‑light BFS) and add a cheap, deterministic ambient occlusion term based on WCC micro neighborhoods to provide contact‑shadow cues.

How it integrates with WCC
- AO computed per face cell while merging; stable and crack‑free.

Seams
- AO is geometric; no neighbor exchange needed. Logical light retains current seam path with micro border planes.

Pros
- Easy to implement; removes many perceptual artifacts without heavy lighting changes.

Cons
- Not physically motivated for light transport; still needs clean logical light for emissives/skylight.

Complexity
- Low.

## Cross‑Cutting Requirements
- Unified sealed‑plane predicate: a single function that, given two adjacent voxels and a face index, decides openness at S=2. Both mesher and lighting must use it.
- Micro border planes: for any solution needing neighbor exchange, send per‑face micro planes (skylight, block light, and optional direction) with consistent half‑open indexing.
- Deterministic ownership: do not compute or own +X/+Z faces locally; stitch via neighbor planes only (already matches WCC seam rule).

## Recommendations
Short term (low risk)
- Adopt Option C improvements on top of the current system:
  - Use the unified sealed‑plane predicate for both propagation and sampling gates.
  - Make border sampling micro‑aware (read neighbor skylight/block micro planes, not block‑wide max).
  - Keep the 8‑sample symmetric neighborhood only as a fallback when precise per‑cell data isn’t available.

Medium term (best WCC synergy with small memory)
- Implement Option A (WCC‑Frontier Flood) for skylight first, optionally for block lights later. Shade directly from surface values; drop heuristic neighbor sampling entirely for WCC faces.

Long term (maximum correctness)
- Evaluate Option B (micro‑voxel lighting) with compression or on‑demand computation restricted to a band near surfaces. Compare visuals and cost against the surface BFS.

## Validation Plan
- Test scenes: slab/stair stacks, pane corridors with emissives, tree canopies (leaves block skylight), cliff overhangs, and chunk seams with staggered neighbor loads.
- Metrics: build time per chunk, memory peak during build, surface quad merge stability, seam consistency (no cracks/double faces), and visual deltas.


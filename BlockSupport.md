# Block Support Plan

This document outlines an incremental plan to support the rest of the block types present in `.schem` files, including non‑cubic shapes, with attention to rendering, meshing, collision, and import mapping.

## Goals
- Import `.schem` content faithfully enough to preserve silhouettes and key materials.
- Keep the meshing fast and memory‑efficient; avoid degrading greedy meshing for cubes.
- Add support in small, verifiable slices; always re-run `--schem-report` to measure progress.

## Phasing Overview
- Phase 1: Expand cubic coverage (done incrementally).
- Phase 2: Add simple partial cubes (slabs, smooth variants, top/bottom faces).
- Phase 3: Add directional partials (stairs, logs with axis, pillars) and per-face UVs.
- Phase 4: Add connected partials (walls, fences, iron bars, glass panes).
- Phase 5: Add flat/plantlike quads (grass, flowers, crops, carrots, vines).
- Phase 6: Add special opaque models (bookshelf done, crafting table, furnaces, etc.).
- Phase 7: Add transparent solids (glass, stained glass) with alpha sorting.
- Phase 8: Liquids (static water/lava surfaces).
- Phase 9: Redstone (visuals only), doors/trapdoors/gates (static pose), rails (flat).

Each phase includes import mapping, meshing, textures, and minimal collision semantics.

## Technical Approach

### 1) Data Model Evolution
- Keep Block enum lean for major materials and families. For non-cubic/directional shapes, avoid exploding the enum.
- Introduce a data-driven shape layer:
  - VoxelShape: Cube, Slab(Top/Bottom), Stairs(orientation), Wall(segments), Fence(segments), Pane(segments), CrossQuad, Pillar(axis), etc.
  - ShapeMaterial: references FaceMaterials (existing) or texture paths.
- Store per-voxel a compact descriptor: ShapeKind + Material + small state bits (e.g., 1 byte flags).
  - Backward compat: cubes continue through current greedy path.

### 2) Meshing Pipeline
- Preserve greedy meshing for full cubes only (current path).
- Add a secondary “special-shapes” mesher pass:
  - Iterates voxels that are not full cubes.
  - Emits pre-baked triangles/quads per shape with proper UVs and occlusion (skip faces touching same material/shape if applicable).
  - Batches geometry by FaceMaterial to limit draw calls, identical to cubes.
- Occlusion rules:
  - For special-shapes, test per-face occlusion vs adjacent cubes; for partials, preserve expected visibility.
- Draw ordering:
  - Maintain two passes: opaque (cubes + opaque partials) then alpha (leaves, glass, panes, plants).

### 3) Import Mapping (`schem.rs`)
- Keep a pure mapping function: `palette_id + states -> (ShapeKind, Material)`.
- Parse a minimal subset of state attributes:
  - logs/pillars: `axis=x|y|z`
  - slabs: `type=top|bottom|double`
  - stairs: `facing=...`, `half=top|bottom`, `shape=straight|inner|outer` (start with straight only)
  - walls/fences/panes: four directional booleans (connectivity)
  - doors/trapdoors: `facing`, `half`, `open` -> collapsed to a single representative static mesh for now
- Short-term pragmatic defaults: map unknown states to reasonable fallback (e.g., straight stairs, closed doors).

### 4) Textures and Materials
- Reuse existing textures where available (`assets/blocks/*`).
- Add FaceMaterials for:
  - Terracotta colors: map `<color>_terracotta -> hardened_clay_stained_<color>.png`.
  - End stone / end stone bricks (end_stone.png; custom brick if provided later).
  - Deepslate/tuff/basalt (add when textures are chosen).
- For multi-face materials (sandstone, quartz): top/bottom/side already patterned.

### 5) Collision Semantics
- Phase-in collision detail:
  - Slabs: half-height AABB (top/bottom).
  - Stairs: start as full AABB, then refine to L‑shaped later.
  - Walls/fences/panes: start as full AABB; refine to pillar + thin arms.
  - Plants/panes/glass: no collision or thin collision depending on gameplay needs.
- Keep player experience smooth; err toward full AABB initially to avoid falling through.

### 6) Transparency and Sorting
- Add alpha bucket for glass/panes/plant quads. Draw after opaques.
- Use alpha-tested textures where available (avoid semi‑transparent except glass).
- Depth write on alpha-tested quads OK; for glass, consider depth-sorted draw or simple ordering.

### 7) Connected Models (Walls/Fences/Panes)
- Connectivity requires neighbor sampling within the chunk and across boundaries.
- Implement local connectivity first; for chunk borders, use neighbor mask to avoid popping (similar to current occlusion’s neighbor rules).
- Mesh generation: start from a central post; append arms per connected side.

### 8) Doors/Trapdoors/Gates (Static)
- For import: pick a closed, axis-aligned state ignoring open/hinge to preserve look.
- Mesh: two quads per face rectangle or a thin slab; later optional animation support.

### 9) Plants and Flat Quads
- CrossQuad for tall grass, flowers, saplings.
- Vine-like: single quads attached to faces according to state; start simplified.
- Crops: flat quads near ground; ignore growth stage or map to a single stage.

### 10) Liquids
- Visual only: render a top face at y+0.9 with animated UV optional; side faces optional.
- Skip flow direction for now; later, compute slope to neighbors for slanted top.

### 11) Redstone Components (Visual Only)
- Comparator/repeater/redstone wire: flat quads with overlays; ignore functional logic.
- Lamps/lanterns/torches: billboard or small column quads; ignore light emission differences initially.

## Implementation Steps (Actionable)

1) Terracotta and End Stone (Quick Win)
- Map all `<color>_terracotta` -> FaceMaterial using existing `hardened_clay_stained_*.png`.
- Add `EndStone` + optional `EndStoneBricks` if texture provided.
- Acceptance: schem-report drops by ~10–30 types.

2) Logs/Pillars Axis
- Parse `axis` in logs and quartz pillars; store as Pillar(axis) shape.
- Mesher: generate side faces per axis; reuse top/bottom materials already present.
- Acceptance: visual alignment of logs/pillars matches schem.

3) Slabs
- Parse `type=top|bottom|double`.
- Add Slab shape with half-height AABB, occlusion against adjacent full cubes.
- Acceptance: double -> full cube, top/bottom render as half blocks.

4) Stairs (Straight Only)
- Parse `facing` and `half` (ignore inner/outer at first).
- Add Stairs shape with a simple L prism and basic occlusion.
- Acceptance: recognizable stairs; schem-report excludes stairs.

5) Walls/Fences/Panes (Connectivity Lite)
- Parse 4-direction connections; mesh central post plus thin arms per direction.
- Start with cobblestone walls, oak fences, glass panes; reuse textures.
- Acceptance: connected appearances match neighbors within chunk; borders okay.

6) Plants/Flat Quads
- Add CrossQuad and SingleFace shapes; map grass/flowers/crops/vines simplistically.
- Acceptance: vegetation visible; no collision.

7) Glass and Alpha Pass
- Move glass/panes to alpha bucket, ensure draw after opaques.
- Acceptance: glass visible without fighting z; leaves remain in alpha bucket.

8) Doors/Trapdoors/Gates (Static Closed)
- Map to closed state meshes; basic quads; collision full or thin as desired.
- Acceptance: door-like appearance in structures; no interactivity.

9) Liquids (Visual Top Faces)
- Render top surface quads; color via texture; no flow.
- Acceptance: water/lava volumes recognizable; no gameplay effects.

10) Special Blocks
- Furnaces/crafting tables/barrels: add specific FaceMaterials using existing textures.
- Acceptance: those blocks no longer reported as unsupported.

## Tooling & Config
- Add `assets/blockmap.toml` (optional): map `minecraft:id[states]` -> `{ shape, material, params }` to reduce code churn.
- Keep `--schem-report` as primary validation; add `--schem-report --counts` to show per-id counts.

## Performance & Quality
- Batch special-shape geometry by material alongside cubes to minimize draw calls.
- Keep neighbor queries bounded; leverage existing neighbor mask to avoid cross-chunk stalls.
- Validate memory impact; ensure special-shape buffers free with chunk unload like cube meshes.

## Acceptance Criteria per Phase
- Report trend: unsupported ids count decreases after each slice.
- No crash/regressions in chunk streaming; frame time stays stable.
- Visual inspection for a few known structures (stairs, fences, vegetation, panes, glass, doors).

## Open Questions
- How accurate should collisions be for non-cubic shapes vs. development speed?
- Do we want separate material/shader for transparent vs alpha-tested quads (glass vs leaves)?
- Should we adopt Minecraft model JSONs for certain blocks to avoid hand‑meshing? (Potential future work.)

## Maintenance
- Keep additions small and well-scoped; re-run `--schem-report` and capture the delta in PR notes.
- Prefer data-driven mappings where possible; keep hardcoded fallbacks minimal.


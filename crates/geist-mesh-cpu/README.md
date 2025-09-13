# geist-mesh-cpu — WCC Meshing (CPU)

This crate builds chunk meshes on the CPU using a Watertight Cubical Complex (WCC) approach. Instead of emitting faces per voxel, it computes the boundary of solids with parity toggles, greedily merges coplanar faces, and stitches chunk seams without cracks.

Highlights
- Watertight by construction: interior faces cancel via XOR parity; only boundaries remain.
- Greedy rectangle merging per plane dramatically reduces triangle count.
- Half‑open seam rule on X/Z: emit on −X/−Z planes; +X/+Z are owned by neighbors. Y is local to a chunk.
- Micro occupancy at S=2: slabs/stairs (and similar) are 2×2×2 “micro boxes” processed by the same WCC toggler.
- Thin dynamics (pane/fence/carpet) are emitted in a lightweight secondary box pass.

Key types
- `ChunkMeshCPU`: output mesh per material; buffers include positions, normals, uvs, indices, and vertex colors.
- `WccMesher`: accumulates parity/material keys on axis‑aligned face grids and emits merged quads.

Algorithm sketch
1) Scale by S (default 2). For each solid volume (full cubes or micro boxes from occupancy), toggle six face planes in integer grid space:
   - +X/−X at x1/x0 across y0..y1, z0..z1, etc.
   - Toggling flips a parity bit; when parity becomes true, store a compact key (material id + light).
2) For each plane family (−X, ±Y, −Z), build a 2‑D mask of keys and greedily merge rectangles; emit one quad per rect.
3) Seam ownership: do not emit on +X/+Z; for −X/−Z planes, seed/toggle using neighbor world blocks so faces match across chunk boundaries.

Lighting (current behavior)
- Per‑face light is sampled from the lighting grid; a small visual minimum prevents pitch‑black faces. For S=2 shapes, a face‑aware sampler approximates micro visibility.

Entry point
- `build_chunk_wcc_cpu_buf(buf, lighting, world, edits, cx, cz, reg) -> Option<(ChunkMeshCPU, Option<LightBorders>)>`
  - Builds a chunk mesh with WCC (S=2 for micro occupancy), then emits thin dynamic shapes.
  - Returns the mesh and optional light borders to feed neighbor lighting.

Seam policy
- Block types may opt out of occluding identical neighbors via `dont_occlude_same` (respected here).
- Additional seam fields (e.g., `dont_project_fixups`) exist in the registry but are not used by this crate.

Implementation notes
- Micro occupancy tables are generated on demand and cached (`occ8_to_boxes`, `empty4_to_rects`).
- Key constants live in `src/constants.rs` (e.g., `MICROGRID_STEPS = 2`, `OPAQUE_ALPHA`, bitset word math).

Tests
- See `crates/geist-mesh-cpu/tests/wcc.rs`:
  - Random solids parity/area checks
  - Seam stitching on -X/-Z (no duplicate faces on shared planes)
  - Greedy merges reduce triangles for slabs/micro shapes

Notes / Limitations
- Thin dynamics are emitted via a secondary pass (not through WCC grids) for simplicity.
- Micro occupancy uses S=2; higher S would require more memory and tuning.

Future work
- Integrate micro‑voxel lighting for S=2 shapes (see the scaffolding in `geist-lighting/src/micro.rs`).

## Algorithm Glossary (Plain English)

- Voxel: a single cube in the world grid with a block type/state.
- Chunk: a fixed-size 3D region of voxels processed as a unit.
- Axis‑Aligned: faces parallel to X, Y, or Z axes (no rotation).
- Face: one of six directions: +X, −X, +Y, −Y, +Z, −Z.
- Face‑Aligned Rectangle (Quad): a rectangle lying in a single face plane used for rendering.

- Watertight: a mesh with no cracks or holes along shared edges.
- Watertight Cubical Complex (WCC): computes the boundary of solids in a grid so only outside faces remain, guaranteeing watertightness.
- Boundary of Solids: faces that separate solid cells from empty cells.
- Parity Toggle / XOR Parity: flipping a bit when a solid spans a face location; even flips cancel, leaving only boundary faces.

- Plane Family (−X, ±Y, −Z): we emit in three groups of planes. We own −X and −Z; Y is chunk‑local.
- Half‑Open Seam Rule (X/Z): emit faces on −X/−Z borders; +X/+Z belong to neighbors; prevents duplicates.
- Seeding From Neighbors: read the neighbor’s world data and toggle our −X/−Z planes so seams match exactly.

- Greedy Rectangle Merging: merge adjacent same‑key cells on a plane into larger rectangles to reduce quads.
- 2D Mask (of Keys): per‑plane grid of optional values; each value encodes material + light.
- Compact Key: a small code that pairs a material id with a light level for merge decisions.

- Micro Occupancy: sub‑voxel geometry represented on a finer grid inside a voxel.
- S (Micro Scale): number of subdivisions per axis (here S=2 → 2×2×2 inside a voxel).
- 2×2×2 Micro Boxes: half‑step AABBs that approximate micro shapes (e.g., slabs/stairs) at S=2.
- `occ8_to_boxes`: converts an 8‑bit micro occupancy mask to micro boxes.
- `empty4_to_rects`: converts a 4‑bit emptiness mask on a boundary plane to rectangles.

- Lighting Grid: stores light samples used to shade emitted faces.
- Face‑Aware Sampling (S=2): choose light consistent with which side of a micro face is visible.
- Visual Minimum: clamp to a small light floor to avoid pitch‑black faces.
- Light Borders: per‑edge light data exported so neighbors can match lighting across seams.

- Bitset Word Math: store bits in 64‑bit words; index with shifts/masks for fast toggles/tests.
- AABB (Axis‑Aligned Bounding Box): min/max corner box aligned to axes.
- Clipping to Chunk Interior: trim emitted rects/boxes to the chunk’s X/Z and Y bounds.

- Thin Dynamics: slender shapes (pane/fence/carpet) emitted as thin boxes in a secondary pass.
- Occlusion: a neighbor covers a face so it shouldn’t be emitted.
- `dont_occlude_same`: block‑type flag; identical touching blocks may not occlude each other.

## How It Fits Together (2D Example)

Below is a 2D XY slice to illustrate the flow; Z behaves similarly per plane.

Step 1: Start with solids (#) and empty (.) cells

    y↑
    3 | . # # .
    2 | . # # .
    1 | . . # .
    0 | . . . .
       +-------→ x

Step 2: Compute boundary via parity

- Consider the grid edges between cells. For every solid span crossing an edge, flip a bit.
- Shared edges between two solids flip twice (cancel). Edges between solid/empty flip once (remain).
- The remaining 1‑bits trace the closed boundary around the solids (watertight outline).

Step 3: Build a 2D mask of keys and merge greedily

- Rasterize boundary faces into a mask; each mask cell gets a key (material+light) or remains empty.
- Merge adjacent same‑key cells into rectangles:

    Before merging (letters = material+light keys):
      A A . .
      A A B .
      . . B .
      . . . .

    After greedy merging:
      [ A A ] . .   → one 2×2 rectangle for A, one 2×2 rectangle for B
      [ A A ] B B
      . . [ B B ]
      . . . .

Step 4: Emit quads and clip to chunk

- Each rectangle becomes one quad with the correct face normal and UVs.
- Quads are clipped to the chunk interior on −X/−Z seams (and fully for Y bounds).

Seams (X/Z half‑open policy)

- On −X/−Z borders, we seed toggles using the neighbor’s world so the shared plane matches.
- We do not emit on +X/+Z; the neighbor will own those faces. This avoids duplicates and cracks.

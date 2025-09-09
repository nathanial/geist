# geist-mesh-cpu — WCC Mesher (CPU)

This crate builds chunk meshes on the CPU using a Watertight Cubical Complex (WCC) approach. It replaces per‑voxel face emission with a robust boundary‑of‑solids algorithm that merges faces greedily and stitches chunk seams without cracks.

Highlights
- Watertight by construction: interior faces cancel via XOR parity; only boundary faces remain.
- Greedy rectangle merge per plane reduces triangle count significantly.
- Half‑open seam rule: emit faces on −X/−Y/−Z planes only; +X/+Y/+Z are owned by neighbors.
- S=2 micro occupancy: slabs and stairs are represented as 2×2×2 micro boxes routed through the same WCC toggler.
- Thin dynamics (pane/fence/gate/carpet) emitted via light‑box pass for practicality.

Key types
- `ChunkMeshCPU`: output mesh parts keyed by material; each part stores positions, normals, uvs, indices, and vertex colors.
- `WccMesher`: accumulates parity on axis‑aligned face grids and emits merged quads.
- `NeighborsLoaded`: indicates which neighbor chunks are present so seams can be stitched deterministically.

Algorithm sketch
1) Scale by S (default 2). For each solid volume (full cubes, or micro boxes from occupancy), toggle six face planes in integer grid space:
   - +X/−X at x1/x0 across y0..y1, z0..z1, etc.
   - Toggling flips a parity bit and sets a material/light “key” when parity becomes true.
2) For each allowed plane (−X, all Y, −Z), build a 2‑D mask of keys and run greedy merging to emit quads with correct normals and UVs.
3) Seam ownership: do not emit on +X/+Z planes. Additionally, for −X/−Z boundary planes, incorporate neighbor faces and drop faces occluded by loaded neighbors (prevents cracks and duplicates).

Lighting (current behavior)
- The mesher samples per‑face light from the lighting grid. With S=2 shapes, it uses a face‑aware sampler to approximate micro visibility while we migrate towards micro‑voxel lighting.

Entry point
- `build_chunk_wcc_cpu_buf(buf, lighting, world, edits, neighbors, cx, cz, reg) -> Option<(ChunkMeshCPU, Option<LightBorders>)>`
  - Builds a chunk mesh using WCC (S=2 for occupancy), then emits thin dynamic shapes.
  - Returns the CPU mesh and optional lighting border planes to feed neighbor chunks.

Seam policy
- Blocks can opt out of mutual occlusion at seams (e.g., panes) via registry seam flags (`dont_occlude_same`, `dont_project_fixups`).

Tests
- See `crates/geist-mesh-cpu/tests/wcc.rs`:
  - Random solids parity area check
  - Seam plane stitching (no triangles exactly on shared plane)
  - Greedy merge reduces triangles on slabs

Notes / Limitations
- Thin dynamics are currently emitted via a secondary pass (not through WCC grids) for simplicity.
- Micro occupancy is S=2; higher S would need additional memory and tuning.

Future work
- Switch lighting to a micro‑voxel field (see `MicroVoxelLighting.md`) to make shading fully consistent with WCC at S=2.


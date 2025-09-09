# MesherPlan — Watertight Cubical Complex (WCC)

Goal: Replace per-face occlusion + greedy quads with a robust boundary-of-solids mesher based on XOR parity on faces, then greedy rectangle merging. Must be crack-free at chunk seams, match or improve full-cube output, and be extendable to micro-shapes and thin dynamics.

- Feature flag: `Rastergeist.mesh.wcc = true` (design). Current implementation uses env var `RASTERGEIST_MESH_WCC=1` to enable the WCC path.
- Target crates: `geist-mesh-cpu` (implementation), `geist-lighting` (sampling), `geist-world`/`geist-chunk` (inputs)
- Entry point: `build_chunk_wcc_cpu_buf(...) -> (ChunkMeshCPU, Option<LightBorders>)`

---

## Phases

1) Phase 1 — Full cubes (S=1) [COMPLETED]
- WCC at voxel resolution S=1 for full-cube solids.
- Parity accumulation + greedy rect emission; half-open seam rule.

2) Phase 2 — Micro grid (S=2) [COMPLETED]
- Integrated 2×2×2 micro occupancy via scaled integer coords.
- Micro boxes go through the same WCC toggler; micro projection fixups removed under WCC.

3) Phase 3 — Thin dynamics [COMPLETED (Hybrid)]
- Practical hybrid: panes/fences/gates/carpets are emitted via a thin‑box pass alongside WCC output. This avoids the memory overhead that a large uniform S (or Sy=16) would introduce.
- Optional future upgrade: route thin shapes through WCC using a common S (e.g., S=4) or axis-dependent scales (e.g., Sy=16 for carpets) with on‑the‑fly plane masks.

---

## Data Structures

Key = `(MaterialId, LightBin)`

Face grids sized for scale `S` and chunk dims `(sx, sy, sz)`:
- X faces: `(S*sx + 1) × (S*sy) × (S*sz)`
- Y faces: `(S*sx) × (S*sy + 1) × (S*sz)`
- Z faces: `(S*sx) × (S*sy) × (S*sz + 1)`

Represent parity as bitsets; optionally store an orientation bit (true for the positive face) and key indices (u16) in parallel arrays pointing into a compact table of distinct `(MaterialId, LightBin)` pairs.

Indexers: `idx_x(ix, iy, iz)`, `idx_y(ix, iy, iz)`, `idx_z(ix, iy, iz)` computed by row-major strides per axis.

---

## Ownership / Seam Rule (Half-open)

To avoid cracks/duplicates at chunk borders:
- Emit on internal planes and on −X, −Y, −Z boundary planes only.
- Do not emit on +X (ix == S*sx), +Y (iy == S*sy), +Z (iz == S*sz) planes; neighbors own those as their negative planes.
- Parity accumulation still toggles all six faces; the rule applies at emit time.

---

## Boundary-of-solids (∂ operator via XOR)

For each solid box `[x0,x1)×[y0,y1)×[z0,z1)` in scaled integer coordinates:
- Toggle ranges on six planes:
  - +X at `ix=x1`, span `y0..y1`, `z0..z1`
  - −X at `ix=x0`, span `y0..y1`, `z0..z1`
  - +Y at `iy=y1`, span `x0..x1`, `z0..z1`
  - −Y at `iy=y0`, span `x0..x1`, `z0..z1`
  - +Z at `iz=z1`, span `x0..x1`, `y0..y1`
  - −Z at `iz=z0`, span `x0..x1`, `y0..y1`

On toggle:
- `parity ^= true` on the addressed cell
- If `parity == true`, set its `key = Some((material_for_face, light_bin))`
- If `parity == false`, clear the key (interior faces cancel)

Material and light must reflect the solid side of that face (use face direction).

Notes:
- Order of toggles does not matter; interior faces cancel.
- Consider row-wise memxor for spans to accelerate toggling.

---

## Inputs (Boxes to Toggle)

Phase 1 (S=1): Full cubes only
- For each voxel `(x,y,z)` that is “full solid”, toggle the unit box:
  - `[x*S, (x+1)*S) × [y*S, (y+1)*S) × [z*S, (z+1)*S)`

Phase 2 (S=2): Micro occupancy
- For each micro box from `occ8_to_boxes(occ)` where micro coords are in `{0,1,2}` at S=2:
  - `min = (x*S + micro.x0, y*S + micro.y0, z*S + micro.z0)`
  - `max = (x*S + micro.x1, y*S + micro.y1, z*S + micro.z1)`

Dynamic thin shapes (Phase 3)
- Either choose a common S (e.g., S=4) and route boxes through WCC or keep the legacy thin pass temporarily.

---

## Lighting & Material Keys (Merge Compatibility)

- Material: use existing registry’s per-face method, e.g. `material_for_cached(face.role(), block.state)`.
- Light: sample as currently done per face; then quantize to N bins (e.g., 16–32). Clamp using `VISUAL_LIGHT_MIN`.
- Merge key: `(MaterialId, LightBin)`. If AO is baked into vertex color, fold it into `LightBin` to avoid over-merging.

Trait hints:
- `trait FaceLight { fn for_face(&self, face: Face) -> u8; }`

---

## Emission (Grids → Quads)

For each axis and each allowed plane (respect half-open rule):
1) Build a 2-D `mask: Vec<Option<Key>>` using grid `key`s where `parity == true`.
   - X-planes: width = `S*sz`, height = `S*sy` (u along Z, v along Y)
   - Y-planes: width = `S*sx`, height = `S*sz` (u along X, v along Z)
   - Z-planes: width = `S*sx`, height = `S*sy` (u along X, v along Y)
2) Run `greedy_rects(width, height, &mut mask, emit)`.
3) In `emit`, map plane coord and rect `(u0,v0,w,h)` back to world, scaling by `1/S`, then call `add_face_rect(axis_face, origin, u_size, v_size, ...)` with existing UV orientation rules.

Thin shapes (Phase 3, hybrid):
- Emit thin volumes (pane/fence/gate/carpet) via a legacy thin‑box pass using shape rules and occluder checks, and append results to the same MeshBuild. These do not use the WCC parity grids and therefore do not require additional face grids or large S.

---

## Integration Points

- `geist-mesh-cpu`:
  - Add `build_chunk_wcc_cpu_buf(...) -> (ChunkMeshCPU, Option<LightBorders>)`
  - Add `WccMesher` (struct + methods) and bitset/key-table utilities.
  - Keep `build_chunk_greedy_cpu_buf` intact; WCC selected by feature flag.

- `geist-runtime` job selection:
  - Plumb config `Rastergeist.mesh.wcc` (bool) into the meshing job (design). Current implementation uses an env var `RASTERGEIST_MESH_WCC` to choose WCC vs legacy greedy.

- Inputs:
  - Use `ChunkBuf` and shape/solid queries from `geist-chunk`/`geist-blocks` to decide “full solid” and to get micro occupancy for Phase 2.
  - Use `LightGrid` (or equivalent) from `geist-lighting` for sampling.

- Outputs:
  - Build `ChunkMeshCPU` parts keyed by material; preserve current upload path via `geist-render-raylib`.

---

## Correctness & Tests

Invariants (assert where cheap):
- Internal shared faces cancel: after accumulation, any interior face has `parity == false`.
- Ownership: emitted faces never appear on +X/+Y/+Z planes.
- Face count sanity: X-face total equals count of `(solid(x-1) XOR solid(x))` across the volume; similarly for Y/Z.

Unit tests (crates/geist-mesh-cpu/tests/wcc.rs):
1) Random binary chunks (full cubes): WCC face counts match naive boundary counts per axis.
2) Seam stitching: two adjacent chunks with same pattern → stitch outputs and verify no duplicates/holes on shared plane.
3) Merge stability: checkerboard patterns → ensure greedy reduces count and produces no T-junctions.
4) Thin-shape seams (hybrid): connectors crossing chunk borders (pane/fence/gate/carpet) should not produce duplicate faces or gaps.

Visual regression:
- Spin camera around terraced terrain; compare silhouette and check for cracks vs legacy mesher.

---

## Performance Notes

- S=1 face grids on 32×256×32 chunks are small (on the order of a few thousand cells per axis).
- S=2 roughly doubles each dimension; still modest.
- Use bitsets for parity; compact key indices (`u16` is sufficient) into a small `(MaterialId, LightBin)` table.
- Toggle spans row-wise; consider `memxor` for whole rows.

Note on keys at S=2 (memory-friendly):
- To keep memory modest at higher S, avoid a global per-face-cell key array. Instead, when traversing a plane for emission, compute `(MaterialId, LightBin)` on-the-fly for only the cells on that plane to build the 2-D mask. You can also store a compact per-plane key table (u16 indices) rather than a crate-wide table. This keeps peak RAM close to the mask size and avoids scaling issues as S grows. The current implementation uses a compact global key table with u16 indices; moving to per-plane tables is a drop-in improvement if needed.

---

## Edge Cases & Policies

- Transparent solids (e.g., glass):
  - If drawn in the opaque pass, include them in WCC with translucent materials; otherwise, exclude and draw in a separate transparent pass.
- “Don’t occlude same” flags: no longer needed for seams; parity removes interior faces by design.
- Prevent over-merging: keep `LightBin` (and AO if folded) in the merge key.

Thin shapes (hybrid) seam policy:
- Thin shapes are not routed through WCC parity; they rely on occluder checks to avoid duplicate faces. This is robust because their thin volumes do not create shared coplanar interior faces across chunk borders. If needed, unify them under WCC using axis-dependent scales.

---

## Minimal Rust Stubs (Sketch)

```rust
pub fn build_chunk_wcc_cpu_buf(
    buf: &ChunkBuf,
    lights: &LightGrid,
    registry: &BlockRegistry,
    base_x: i32,
    base_z: i32,
) -> (ChunkMeshCPU, Option<LightBorders>) {
    let S = 2usize; // Phase 2 default (S=1 for full-cube-only)
    let mut mesher = WccMesher::new(buf, lights, registry, S, base_x, base_z);

    // Full cubes
    for z in 0..buf.sz() {
        for y in 0..buf.sy() {
            for x in 0..buf.sx() {
                let b = buf.get(x, y, z);
                if registry.is_full_cube(b) {
                    mesher.add_cube(x, y, z, b);
                }
            }
        }
    }

    // Micro occupancy (S=2): add half-step boxes via occ8_to_boxes

    let parts = mesher.emit();
    (ChunkMeshCPU { parts, ..Default::default() }, None)
}
```

```rust
struct WccMesher<'a> {
    S: usize,
    sx: usize, sy: usize, sz: usize,
    grids: FaceGrids,
    reg: &'a BlockRegistry,
    light: &'a LightGrid,
    base_x: i32, base_z: i32,
}
```

---

## Acceptance Criteria

- Geometry equivalence for full-cube worlds vs legacy mesher (silhouette identical, quad partitioning can differ).
- Seam correctness: no cracks/overlaps at chunk borders under randomized fills.
- Performance within ~±10% of current path for full cubes; micro path acceptable at S=2.
- Code simplification: micro fixups removable in Phase 2.

---

## Common Pitfalls

- Emitting faces on +X/+Y/+Z planes (violates ownership).
- Deriving material/light from the wrong side of a face; always use the solid side.
- Forgetting to scale sizes by `1/S` when mapping rectangles to world units.
- Over-merging across different light/material bins.

---

## Optional Upgrades

- Replace simple greedy with maximal-rectangle cover per plane for fewer quads.
- Incremental updates: toggle boxes on edits and re-greedy only affected planes.
- Unify thin shapes under WCC at a higher global micro scale (e.g., S=4) or axis-dependent (Sy=16 for carpets), implemented with on‑the‑fly plane masks to keep memory modest.

---

## Current Implementation Summary

- WCC mesher exists in `geist-mesh-cpu::build_chunk_wcc_cpu_buf`.
- S=2 WCC covers full cubes and micro-grid occupancy; half-open ownership rule enforced at emission.
- Thin dynamics (pane/fence/gate/carpet) are emitted via a thin-box pass (legacy logic) appended to the WCC output.
- Env flag `RASTERGEIST_MESH_WCC=1` enables WCC; otherwise the legacy greedy mesher runs.

---

## Deliverables

- New `build_chunk_wcc_cpu_buf` behind `Rastergeist.mesh.wcc` feature flag.
- Unit tests for parity and seams.
- Before/after screenshots on cube-only scene; micro scene once Phase 2 lands.
- Bench numbers vs current mesher.

---

## Task Checklist

- [ ] Add bitset + key-table utilities for face grids
- [ ] Implement WccMesher toggles for six faces
- [ ] Emit masks per plane and run greedy merge
- [ ] Integrate feature flag and selection path
- [ ] Add Phase 1 unit tests (parity, seams, merge stability)
- [ ] Visual regression capture for terraced terrain
- [ ] Bench and compare vs legacy mesher
- [ ] Phase 2: add S=2 micro boxes path; remove micro fixups

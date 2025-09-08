**Overview**
- Goal: Make meshing maximally table/data‑driven to simplify logic, remove shape‑specific branches, and enable adding new shapes/policies without touching mesher code.
- Constraints: Keep current visuals and performance (or better), align with existing modules (`mesher.rs`, `meshing_core.rs`, `meshutil.rs`, `blocks/registry.rs`).

**Current State (Quick Read)**
- Greedy meshing lives in `meshing_core::build_mesh_core` and is already table-ish per face plane.
- `mesher.rs` unifies box emission via `emit_box_faces`/`emit_box_generic`, and uses a 2×2×2 micro‑grid for slabs/stairs with generic neighbor fixups.
- Materials are precomputed per state/role (`BlockType::material_for_cached`).
- Occlusion uses a computed 6‑bit mask via `occlusion_mask_for` and `occludes_face`.

This is a good base; we can push more into tables to reduce branching and repeated merging logic.

**Design Principles**
- Separate policy from execution: data describes what to emit; mesher only iterates and applies closures for occlusion and lighting.
- Prefer static tables over per‑frame conditionals for small state spaces (2×2×2 occupancy, 2×2 boundaries, 6 faces).
- Precompute per‑block/state artifacts in the registry: materials, occlusion masks, shape variants, and occupancy.
- Keep world vs local meshing unified via closures for occlusion and light sampling.

**Core Tables To Introduce/Strengthen**
- Faces/Neighbors (exists):
  - `Face`, `ALL_FACES`, `SIDE_NEIGHBORS` (already present). Consider adding per‑face metadata: neighbor delta, plane axes (u,v), and `flip_v` defaults in a single table.
- Occlusion Mask (improve):
  - Precompute `occlusion_mask: u8` per block state in `BlockType` (6 bits ordered by `Face::index`). Replace `occlusion_mask_for` with a simple lookup.
- Micro‑Grid Occupancy → Boxes (new static table):
  - For 2×2×2, map 8‑bit occupancy to a small list of AABBs in half‑cell units. There are 256 masks; each expands to ≤4 boxes (2 layers × ≤2 rects).
  - Store as compact structs: `(x0,y0,z0,x1,y1,z1)` in {0,1,2} half‑steps. At runtime, translate by block origin.
- Boundary Emptiness (2×2) → Neighbor Fixup Rects (new static table):
  - For each 4‑bit emptiness mask on a boundary layer, precompute 0–2 rectangles to draw on the neighbor face. There are only 16 cases; encode rects as `(u0,v0,du,dv)` in half‑steps.
  - Provide one table per side orientation (±X, ±Z, ±Y) or a generic table plus per‑face axis remap.
- Lighting Policy (small table):
  - Encode per‑face ambient bias for local meshes (current `face_light`) as a 6‑entry table.
  - For neighbor fixups, encode whether to sample top/bottom halves (height==2 → max of halves) as flags; the sampling closure remains, but branching disappears.
- Material Lookup (exists):
  - Continue using `pre_mat_top/bottom/side`; expose `face_material_cached(face, state)` helper to remove role mapping in the mesher.

**Shape Descriptors (Per BlockType)**
- Introduce a precomputed per‑state “shape variant” that the mesher can consume directly:
  - `grid: MicroGridSpec` (currently fixed `2×2×2`).
  - `occupancy: u8` (2×2×2 occupancy mask) or `None` for full cubes handled by greedy core.
  - `occlusion_mask: u8` (6 bits, overrides default solid/air logic).
  - `seam_policy: SeamPolicy` (how to treat neighbor seams: allow fixups on ±X/±Z/±Y; optionally glass‑to‑glass rules later).
  - `face_flip_v: [bool;6]` (optional per‑face V flip; default false).
  - `material_selector: uses precomputed arrays (no code).`

Populate variants at registry build time from the existing `Shape` + state props (e.g., slab top/bottom, stairs facing/half). For cubes and axis‑cubes, set `occupancy=None` so the greedy core handles them.

**Refactor Plan (Phased)**
- Phase 1: Extract Static Tables (no behavior changes)
  - Add `microgrid_tables.rs` with:
    - `OCC8_TO_BOXES: [[u8; MAX*6]; 256]` or a compact `SmallVec` style encoding for up to 4 AABBs, each stored as six half‑step coords.
    - `EMPTY4_TO_RECTS: [[u8; MAX*4]; 16]` mapping 4‑bit emptiness to up to 2 rects `(u0,v0,du,dv)`.
  - Replace `microgrid_boxes` and the in‑function coalescing in `emit_neighbor_fixups_micro_generic` with lookups + simple loops, using per‑face axis remapping tables.
  - Precompute a 6‑entry `LOCAL_FACE_LIGHT[6]` and use it instead of `match`.

- Phase 2: Precompute Occlusion Masks
  - In `BlockType`, add `pre_occ_mask: Vec<u8>` sized to the state space.
  - Fill it during `from_configs` using shape + props (cubes → 0x3F, slabs/stairs → current logic, others default sensible).
  - Replace `occlusion_mask_for` with `ty.pre_occ_mask[state & (len-1)]` behind a helper.

- Phase 3: Shape Variants
  - Add `ShapeVariant` and `pre_shape_variants: Vec<ShapeVariant>` to `BlockType`.
  - For `Shape::Cube` and `Shape::AxisCube`, set `occupancy=None` and keep using greedy core (best for large planes).
  - For `Shape::Slab`/`Shape::Stairs` (and future micro‑grid shapes), compute `occupancy`, `occlusion_mask`, and `seam_policy` per state and store them in the variant.
  - Expose `fn variant(&self, state) -> &ShapeVariant` on `BlockType`.

- Phase 4: Unify Special‑Shape Emission via Tables
  - In world and local meshers, replace the `match` over shape with a single branch:
    - Get variant. If `occupancy=None` → skip (handled by greedy core).
    - Else: lookup `boxes = OCC8_TO_BOXES[occupancy]` and emit with `emit_box_generic` using closures for occlusion and light.
    - If `seam_policy` requests fixups, compute boundary masks and lookup `EMPTY4_TO_RECTS[...]` per neighbor face to emit fixups.
  - Materials come from precomputed arrays; use a per‑face `face_material_cached` helper to retrieve `MaterialId`.

- Phase 5: Optional Seam/Transparency Policies
  - Add a small `SeamPolicy` enum/table to control whether a neighbor with the same block/shape/material suppresses faces (e.g., glass panes, leaves), and whether to project fixups.
  - Gate `is_occluder` with these policies to reduce popping along seams for transparent/partial shapes.

- Phase 6: Future Extensions (kept data‑driven)
  - Support higher micro‑grid resolutions (e.g., 4×4×4) by making the occupancy table pluggable per shape; start with 2×2×2 to avoid bloat.
  - Add new shapes expressible on a micro‑grid (panes, fences, posts, thin walls) by just defining occupancy + seam policy transforms from state.
  - Optional CTM/UV transforms as small per‑face tables applied at emit time.

**Implementation Notes**
- Table Encoding:
  - Use compact arrays to avoid allocations in static tables. Example: first byte = count, followed by packed rects/boxes.
  - Provide iterators to decode tables ergonomically at call sites.
- Axis Remapping:
  - Predefine a per‑face mapping of (u,v) axes for rects and how half‑steps map to world (x,y,z) for each face.
- Closures Stay Small:
  - Keep world/local differences as closures: `occludes(face)`, `sample_light(face)`. Everything else comes from tables.

**Milestones & Deliverables**
- M1: Static tables added; `microgrid_boxes` and neighbor coalescing replaced by lookups. No visual diffs. Small code deletion in `mesher.rs`.
- M2: Precomputed `pre_occ_mask` used everywhere; drop `occlusion_mask_for` logic. Minor speedup.
- M3: Shape variants precomputed; mesher special‑shape branch reduced to a single generic path. Significant code size reduction.
- M4: Optional seam/transparency policies implemented; culling correctness across similar blocks improved.

**Success Criteria**
- Mesher hot path contains no shape‑specific logic for slabs/stairs; adding a shape only touches registry precompute and tables.
- Fewer branches and loops inside `mesher.rs`; more in `meshutil.rs`/`microgrid_tables.rs` static data.
- Equal or fewer draw calls/triangles; equal or better FPS.

**Appendix: Table Schemas (Compact Encodings)**
- OCC8_TO_BOXES[256] → [u8]
  - Format: `[count, x0,y0,z0,x1,y1,z1, ...]` with values in {0,1,2} half‑steps, count ≤ 4.
- EMPTY4_TO_RECTS[16] → [u8]
  - Format: `[count, u0,v0,du,dv, ...]` with values in {0,1,2}, count ≤ 2.
- LOCAL_FACE_LIGHT[6] → [i16]
  - Example: `[+40, -60, 0, 0, 0, 0]` added to ambient and clamped.
- FACE_META[6]
  - For each `Face`: `(dx,dy,dz), (u_axis,v_axis), default_flip_v`.


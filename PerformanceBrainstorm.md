# Mesher Performance Brainstorm

Goal: Reduce average per‑chunk meshing time from ~400ms to ~4ms (~100x). Below is a prioritized set of ideas with rough impact, trade‑offs, and implementation notes. The current mesher (crates/geist-mesh-cpu) builds a WCC grid at S=2, toggles parity/material across all faces, seeds seams, then emits greedy rectangles per axis. Most time is spent in: per‑voxel face toggling at micro scale, large temporary grids/allocations, and the plane sweep with per‑cell checks.

## Highest Impact (Algorithmic)
- Split macro vs micro passes (S=1 for cubes, S=2 only where needed)
  - Mesh full cubes with classic greedy slicing at S=1 using block‑level occlusion. Run S=2 only where variants have micro occupancy (slabs/stairs/etc.).
  - Why: Today, full cubes are meshed at S=2 which multiplies grid sizes, parity toggles, and sweep work by ~4x. Most blocks are full cubes. Expect 4–10x speedup on typical worlds.
  - Avoiding the overhead we saw last time:
    - Single classification pass, no double scans:
      - First, iterate voxels once to classify each into `Cube | Water | Micro(occ) | Thin | Air` and build a compact `micro_list: Vec<(x,y,z,occ:u8)>`. Also set per‑plane flags (`has_micro_x/y/z[plane_id]`).
      - This eliminates a second full chunk scan previously used to discover micro voxels per pass.
    - Exclusive responsibility per pass (no double work):
      - Macro S=1 pass skips any cell whose negative or positive neighbor is `Micro` (macro leaves those faces for the micro pass). This avoids generating faces twice and avoids later de‑dup/merge overhead.
      - Micro S=2 pass handles only faces involving at least one micro voxel (micro|micro, micro|cube, micro|air). Macro continues to handle cube|cube faces only.
    - Localized S=2 work, not full‑grid S=2:
      - Do not allocate a full S=2 `FaceGrids`. Instead, for each axis plane that has micro, build a temporary sub‑plane grid just over the bounding box of micro regions in that plane, or emit directly from each micro voxel using small per‑voxel masks.
      - Represent micro detail per voxel with its 8‑bit occupancy; for micro|cube boundaries, treat the cube side as fully solid at S=2. This avoids global S=2 arrays and the “multiple planes” blow‑up.
    - One seam pass for both:
      - Seed seams once using world/edits. During macro pass: treat neighbor full cubes directly. During micro pass: refine only where micro is present at the seam. Don’t run two seam passes.
    - Unified emission buffers, no combiner stage:
      - Both passes write directly into a shared `Vec<Option<MeshBuild>>` indexed by `MaterialId` (thread‑local if parallel). No per‑pass HashMap and no merging step that caused churn previously.
    - Greedy sweeps tailored to each pass:
      - Macro: classic S=1 greedy over plane‑wide bitsets; skip ranges flagged as “micro‑adjacent”.
      - Micro: sweep only sub‑planes that contain micro. For per‑cell state, store a tiny `u8` submask (2 bits per micro‑row/col) and the material/orientation per subcell; the greedy merge compares `(present_submask, material+orientation)` without expanding to a full S=2 plane buffer.
    - Index math and allocation minimization:
      - Reuse plane‑local `visited`/bitsets across both passes; pool buffers per worker. Pre‑reserve per‑material buffers once.
      - Hoist index strides to avoid repeated multiplies in inner loops.
    - Deterministic ownership rules at boundaries:
      - Define that if either side is micro, micro pass owns that face; macro never emits it. Prevents duplicate quads and post‑dedupe cost.
    - Pseudo‑pipeline:
      1) Classify voxels; build `micro_list` and per‑plane micro flags.
      2) Macro S=1 masks and greedy emission, skipping micro‑adjacent cells.
      3) For each axis plane with micro: build minimal sub‑plane masks from `micro_list` and emit S=2 quads.
      4) Done — a single seam seed step and a single set of emission buffers.

- Derive boundary masks via occupancy XOR instead of parity toggles
  - Build a 1‑bit occupancy grid (S=1 for cubes, S=2 where needed) and compute face masks as occupancy XOR of neighbor cells per axis. Set orientation from which side is occupied and material from that side.
  - Why: Avoids toggling all six faces for every voxel (and the resulting double‑work cancelled by parity). Expect 2–5x depending on density. Pairs well with the split macro/micro pass.

- Parallelize the plane sweeps and/or mask construction (Rayon)
  - Emit per‑axis planes in parallel using thread‑local `builds` and merge at the end. Also parallelize boundary mask construction across slices.
  - Why: On 8 cores, 4–8x speedup is realistic. Combined with the two ideas above, can get >20x.

- Sub‑chunk meshing + change detection (16x16x16)
  - Segment chunks vertically (e.g., 16‑tall). Mesh only dirty sub‑chunks and stitch across internal seams. Track dirty flags from edits/worldgen.
  - Why: In edits/streaming scenarios, most rebuilds touch a fraction of the chunk. Reduces work by 4–16x when changes are localized.

- GPU meshing path (compute shader)
  - Upload occupancy/material grids, generate triangle buffers on GPU (one thread per face cell), and stream directly into a device buffer.
  - Why: Offloads massively parallel work; 10–100x speedups are common. Highest complexity and engine integration cost.

## High Value (Structural/Seams)
- Seam seeding fast‑path for full cubes
  - In `seed_neighbor_seams`, if neighbor is a full cube, toggle the entire s×s seam span at once instead of per‑micro test (`micro_cell_solid_s2`). If neighbor is air, skip; only micro‑test when neighbor has micro occupancy.
  - Why: Reduces seam work by ~4x and avoids thousands of function calls.

- Skip interior faces pre‑check for cubes
  - Before toggling, check block neighbor occlusion mask; if both sides occlude, skip toggling that face entirely. Keep parity flow only where necessary (mixed materials/thin shapes).
  - Why: Eliminates a large portion of parity toggles on solid interiors. 1.5–3x on dense terrain.

- Two‑way occlusion rule at micro scale
  - When building micro occupancy, if both adjacent micro cells occlude each other for the relevant face, avoid emitting that boundary entirely (can be folded into XOR mask step).
  - Why: Further shrinks masks in mixed micro regions.

## Medium Impact (Data Structures)
- Replace `HashMap<MaterialId, MeshBuild>` with `Vec<Option<MeshBuild>>`
  - Index directly by `MaterialId.0` with capacity = number of materials. Remove hashing and rehash costs; improves cache locality on writes.
  - Why: 1.2–2x on emission heavy scenes; trivial change across `emit.rs` and merging steps.

- Pre‑reserve mesh buffers
  - Estimate a conservative upper bound for vertices/indices per material and call `reserve_exact` once. For example, bound by exposed faces from a quick pre‑scan of boundary masks (or a heuristic like 6 faces per block at 5–10%).
  - Why: Avoids repeated reallocations and memcpy during emission. Measurable wins, especially on large chunks.

- Reuse and pool large temporaries
  - Pool `FaceGrids` and plane‑local `visited` buffers per worker thread. Use generation counters to mark/clear instead of re‑alloc/zero.
  - Why: Cuts alloc/zero overhead and improves cache locality. Helpful even if algorithms change.

- Pack orientation with material
  - Store orientation bit with `MaterialId` (e.g., highest bit of u16 or a separate `u8` key array) to cut an indirection in sweep comparisons.
  - Why: Fewer memory touches per cell during greedy rectangle growth.

## Medium Impact (Sweeps and Indexing)
- Eliminate per‑cell multiplies in `idx_*`
  - Hoist `base` index for a row and increment by constant strides when scanning `u` and `v`. Inline helpers; prefer pointer‑like arithmetic over repeated `(a*b + c)*d + e`.
  - Why: Reduces ALU pressure inside tight loops; small but broad win.

- Reuse `visited` across the plane
  - Allocate once per plane and reset with a `fill(false)` or generation marker. Avoids `vec![false; width*height]` per plane iteration.
  - Why: Cuts allocations and memory zeroing. Pairs well with pooling.

- Row‑wise run detection
  - For each row, scan to build runs of identical `(present, material+orientation)` and then attempt vertical merging by checking only run boundaries for the next rows.
  - Why: Fewer random checks and earlier skipping over long uniform stretches.

- Bitset‑aware skipping for empty spans
  - Iterate over `u64` words of the parity bitset and skip whole 64‑cell spans when zero. Use `trailing_zeros` to jump between set bits.
  - Why: Greatly reduces CPU work on sparse planes.

## Lower Effort Wins (Quality/Heuristics)
- Configurable micro scale
  - Allow S=1 meshing mode for performance builds; auto‑switch to S=2 only when micro shapes are present in view. Expose sliders in debug UI.
  - Why: Quick way to trade detail for speed; can be worth 2–4x.

- Early chunk sparsity checks
  - Track per‑column or per‑subchunk occupancy flags during worldgen; skip meshing of completely empty tiles/subchunks.
  - Why: Saves full passes on airy regions or caves.

## Emission and Materials
- Emission combiner and dedupe
  - When merging thread‑local builds, coalesce materials with small counts early (e.g., panes/fences) to reduce hashmap churn and small vector overhead.
  - Why: Micro‑optimization that keeps per‑material buffers long‑lived.

- Texture/UV derivation shortcuts
  - Current `add_quad_uv` flips V and recomputes orientation per quad. Cache per‑face UV generation logic; avoid branching where possible.
  - Why: Minor but hot path.

## Incremental Meshing (Edits)
- Delta meshing around edits
  - On block edits, re‑mesh only a padded neighborhood (e.g., ±1 voxel at S=1 and S=2 around micro shapes). Keep a small patch mesh builder and stitch into existing chunk mesh.
  - Why: For interactive edits, turn 400ms full rebuilds into 1–5ms local patches.

- Neighbor‑aware patching
  - Track which sub‑chunk borders changed and rebuild only affected neighbor planes rather than whole sub‑chunks.
  - Why: Reduces work further in sparse edits.

## GPU/Advanced Paths
- GPU face emission from mask textures
  - Upload 3 axis mask textures (or SSBO bitmasks) and a material key texture; run a compute pass that performs the same greedy merge (or triangle emission per cell) in parallel. Output to a device‑local VBO.
  - Why: Orders‑of‑magnitude throughput; keep CPU for culling/streaming.

- Indirect draw via per‑face instancing (prototype)
  - As a stepping stone, try per‑face instancing with frustum + occlusion culling on GPU. Not as efficient as merging but easy to validate.
  - Why: Quick prototype to compare ROI without full compute pipeline.

## Specific Hotspots and Fix Ideas (from code)
- Large grids at S=2
  - `FaceGrids::new` allocates multi‑million‑entry arrays per chunk (kx/ky/kz) plus bitsets. Avoid when possible (macro/micro split), and pool when unavoidable.

- Plane sweeps allocate `visited` per plane
  - Reuse a single buffer per plane and clear via generation counters. Consider a bitset instead of `Vec<bool>`.

- Seam seeding loops call `micro_cell_solid_s2` per micro cell
  - Add fast paths for full neighbor cubes and air.

- HashMap for `builds`
  - Replace with indexable `Vec<Option<MeshBuild>>` keyed by `MaterialId`; pre‑reserve capacities.

- Index math in tight loops
  - Hoist and increment instead of recomputing `idx_*` every cell.

## Rough Roadmap (to approach 100x)
1) Quick structural wins (1–2 weeks):
   - Replace builds HashMap with Vec.
   - Pool FaceGrids and reuse visited buffers.
   - Seam fast‑paths for full cubes.
   - Indexing micro‑opts and bitset row skips.

2) Algorithmic shift (2–3 weeks):
   - Macro/micro split (S=1 cubes + S=2 only where needed).
   - XOR boundary mask derivation replacing toggle parity.
   - Parallel sweeps with Rayon and thread‑local builds.

3) Sub‑chunk + incremental (2 weeks):
   - 16x16x16 sub‑chunk meshing and dirty tracking.
   - Delta rebuilds around edits.

4) GPU path (spike 1–2 weeks; full integration longer):
   - Prototype compute‑generated mesh from mask textures.

Combined effect target: 4–10x (phase 1) × 3–6x (phase 2) × 2–4x (phase 3 incremental/parallel) → plausible path to ~24–240x depending on content and cores.

## Measurement Plan
- Use existing Criterion benches in `crates/geist-mesh-cpu/benches/wcc.rs`:
  - Track: build_chunk_wcc_normal_dims, wcc_toggle_emit_normal_dims, plus new benches for macro/micro split and XOR masks.
  - Add counters: number of boundary cells, rectangles emitted, allocations.
  - Ensure lighting time (`t_light_ms`) is measured separately from meshing (`t_mesh_ms`) in runtime workers.

## Risks & Notes
- Micro vs macro stitching: Ensure no cracks where S=1 and S=2 outputs meet; favor overlaps and consistent seam rules.
- Material/orientation fidelity: Packing or shortcuts must not lose orientation or face‑role material selection.
- Memory spikes: Pre‑reserving too much can hurt; use measured bounds.
- GPU path complexity: Requires robust device buffer management and fallback.

# Micro‑Voxel Lighting (S=2) — Implementation Plan

Goal
- Make lighting match the WCC mesher’s geometry at micro resolution (S=2), so slabs, stairs, and other micro‑shapes shade correctly without special cases. Keep behavior identical for S=1.

Outcomes
- Correct skylight/block/beam lighting through micro openings; no “full‑cube” darkness beside slabs/stairs.
- Seam‑consistent results across chunk borders.
- Backward‑compatible fallback by deriving the legacy voxel LightGrid from the micro grid where needed.

Scope and Phasing
1) Data model + passability (micro grid)
2) Propagation (skylight + omni + beam) on micro cells
3) Border exchange at micro resolution
4) Shading integration (face sampling from micro grid)
5) Performance + memory tuning; derivation to voxel grid

---

1) Data Model (S=2 micro grid)
- For a chunk of size `(sx, sy, sz)` and scale `S=2`, the micro grid dims are `(mx, my, mz) = (S*sx, S*sy, S*sz)`.
- Fields (u8 per micro cell unless noted):
  - `micro_skylight[mx * my * mz]`
  - `micro_block[mx * my * mz]` (omni)
  - `micro_beacon[mx * my * mz]`, and `micro_beacon_dir[mx * my * mz]` (directional hint)
- Occupancy / passability:
  - A micro cell is passable iff the corresponding block’s S=2 occupancy bit is empty.
  - Full cubes: all micro cells are blocked.
  - Slabs/stairs: occupancy comes from `BlockRegistry::variant(state).occupancy` (occ8) → expanded to S=2 box fills.
  - Thin dynamics (pane/fence/gate/carpet): initially continue to use legacy `propagates_light` (passable) or treat as thin occluders; optional follow‑up adds micro shapes for them if needed.

2) Propagation (BFS on micro cells)
- Seed collection:
  - Skylight: for each micro column (ux, uz) in `[0..mx)×[0..mz)`, scan from `my-1` down:
    - Set `level=255` for the highest passable micro cell; enqueue; continue propagation via BFS.
  - Omni emitters: for each emitting block at voxel (x,y,z), seed all passable micro cells within that voxel with the block’s emission (or seed the immediate passable neighbors around the block faces; both behave well at S=2). Attenuation as today (configurable per block).
  - Beacons: same as omni, but push with direction‑aware costs (straight/turn/vertical) into the micro grid.
- BFS step:
  - 6‑connected neighbors; reject out‑of‑bounds; reject target if not passable.
  - Attenuation:
    - Skylight: `level-1`
    - Omni: `level - attenuation`
    - Beacon: `level - cost(dir, step_dir)`
  - Write if new level > stored; enqueue.

3) Chunk Border Exchange (micro planes)
- Export/import border planes at micro resolution:
  - X- sides: `(my × mz)` planes (neg_x, pos_x)
  - Z- sides: `(my × mx)` planes (neg_z, pos_z)
  - Y- sides optional (for editor structures or tall stacks), `(mz × mx)`
- For each field (skylight, block, beacon, beacon_dir), exchange planes with neighbors similar to the current LightBorders, but sized for micro grids.
- Seeding from neighbors: subtract a small cost before enqueuing at the border (same policy as today), with direction hints for beacon.

4) Shading Integration (face sampling)
- For a meshed face (WCC rectangle), compute light from the micro grid on the face’s far side:
  - Map face origin and extents (in world units) to micro coordinates.
  - Sample a small set of micro cells that overlap the face (e.g., the corresponding 2×2 cells for S=2, or a 2×N/ M×2 stencil for larger faces) and take max or a simple average.
  - Apply the visual minimum clamp (existing `VISUAL_LIGHT_MIN`).
- For S=1 (full cubes), the micro grid degenerates to the legacy behavior when downsampled.

5) Performance/Memory + Legacy Derivation
- Memory at S=2 (32×256×32): micro dims 64×512×64 ≈ 2.1M cells. Three u8 fields ≈ 6.3 MB per chunk worst‑case. In practice, we can:
  - Quantize to fewer bits (e.g., 6‑bit) or use run‑length per plane if needed.
  - Only retain skylight + block for most scenes; keep beacon optional or compressed.
  - Use in‑place BFS queues (ring buffers) and avoid re‑allocations.
- Derive legacy voxel LightGrid on demand by taking `max` over the corresponding 2×2×2 micro block cells (or per face via 2×2 micro faces), so existing consumers keep working.

API Changes
- Add `MicroLightGrid` with builders:
  - `MicroLightGrid::compute_with_borders(buf, store, reg, S=2)`
  - `MicroLightBorders` exchange structure analogous to `LightBorders`, but micro sized.
  - `fn downsample_to_voxel(&self) -> LightGrid` (legacy grid for compatibility)
- Mesher uses `MicroLightGrid` for face sampling. For now we can pass both grids; migrate callers gradually.

Migration Plan (incremental)
1) Add micro occupancy helpers (already in blocks/mesher) + micro dims plumbing.
2) Implement `MicroLightGrid` (skylight first), export/import micro skylight planes, replace mesher shading to sample micro skylight.
3) Add omni/beam micro BFS and border exchange. Downsample to legacy `LightGrid` where that is still needed.
4) Switch mesher completely to micro face sampling for all light kinds; remove ad‑hoc neighbor peeks.
5) Optimize memory/CPU; profile; add config toggles.

Validation
- Unit tests:
  - S=1 parity with existing LightGrid.
  - S=2 slab/stairs: faces beside micro openings receive light; borders stitch without seams.
  - Multi‑chunk scenes: no light discontinuity across seams.
- Visual: regressions on terraced terrain, slab/stairs clusters, pane/fence scenes.

Risks / Alternatives
- Memory at S=2 is higher than voxel lighting. If needed, consider:
  - On‑demand micro tiles per Y‑slice.
  - Compression of zero regions.
  - Skipping beacon in micro unless present.


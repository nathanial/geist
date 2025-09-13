True Minecraft‑Style Two‑Phase Lighting — Architectural Plan

Goals
- Correct removal: decrease (darken) first, then increase (re‑light), including across chunk seams.
- Bounded work: budgeted updates like `doLightUpdates(maxUpdateCount, doSkylight, skipEdgeLightPropagation)`.
- Off‑thread execution: lighting tasks run on worker pools, serialized per region/section.
- First‑class edges: defer cross‑chunk work when neighbors aren’t resident, complete later with epochs/ordering.
- Decouple from meshing: lighting changes do not require geometry remeshes.

Scope of Changes (high level)
- Introduce a persistent light engine with per‑section light arrays and explicit decrease/increase queues.
- Replace on‑build recomputation in `geist-lighting` (current micro BFS) with queued, persistent updates.
- Separate lighting runtime from meshing runtime; keep shared executor infrastructure.
- Publish seam planes from persistent data with epochs; neighbors ignore stale planes.
- Update event model and scheduling to feed lighting tasks rather than remeshing work.

Key Code to Replace or Rework
- crates/geist-mesh-cpu/src/build.rs
  - Current: calls `compute_light_with_borders_buf(...)` inside mesh build and derives `LightBorders` from that transient compute.
  - Replace: meshing must only read light from the persistent lighting store (CPU light buffer) via a query API. Remove lighting computation from meshing path. Stop deriving `LightBorders` here.

- crates/geist-lighting/src/lib.rs and crates/geist-lighting/src/micro.rs
  - Current: builds a fresh `LightGrid` (macro + optional micro) per chunk, bucketed BFS, and returns new planes. No persistent light between builds.
  - Replace with:
    - A new `engine/` module: persistent light layers per section (block and skylight; beacon as optional third channel), plus per‑section epochs.
    - Explicit queues: `DecreaseQueue` and `IncreaseQueue` per region/section, with algorithms mirroring Minecraft (see Algorithm section).
    - Section graph + cross‑chunk edges with “skip edge” deferral and per‑face plane epochs.
    - APIs for: enqueue emitters add/remove, enqueue skylight occluder changes, process budgeted updates, and publish updated planes.
    - Keep or re‑implement S=2 face sampling on top of the persistent macro grid (micro is used for gating/passability, not for storing light).

- src/app.rs
  - Current: reacts to `LightBordersUpdated` with ad‑hoc neighbor scheduling; calls `compute_light...` via runtime build jobs.
  - Replace:
    - Replace decrease/rebuild hacks with clean lighting events: `LightTaskScheduled`, `LightSectionUpdated`, `LightPlanesUpdated`.
    - Stop invoking any lighting compute in the app; instead, enqueue lighting tasks into the lighting runtime. Meshing reacts only when geometry truly changed, or (temporary) when we still need to re‑bake per‑vertex light.

- src/event.rs
  - Add events:
    - `LightingDoWork { budget_updates: u32 }` (optional periodic tick to drain lighting queues).
    - `LightSectionUpdated { cx, cz, sec_y, channel, epoch }`.
    - `LightPlanesUpdated { cx, cz, face_mask, epoch }` (replacement for `LightBordersUpdated`, includes epoch and bitmask for all applicable channels).
  - Deprecate/adjust:
    - Keep `LightEmitterAdded/Removed` and block edits, but they now enqueue decrease/increase tasks instead of forcing chunk rebuilds.

- crates/geist-runtime/src/lib.rs
  - Current: three lanes (edit/light/bg) all carry `BuildJob` (meshing) payloads.
  - Replace/extend:
    - Add a dedicated `LightTask` queue and workers distinct from mesh builds.
    - Provide `submit_light_task(...)` and a result channel (`LightTaskOut`) that carries section updates and plane deltas.
    - Maintain budget/serialization per region/section (e.g., key on (cx, cz, sec_y)).


Algorithm (True Two‑Phase)

Terminology
- Section: 16×16×16 macro voxels (Y‑segmented). If our chunks are tall monoliths today, we still partition into logical sections inside each chunk: sy/16 sections along Y. (Fallback: one section per chunk if we must ship sooner.)
- Channel: block light vs skylight (and optionally beacon/beam as a third channel with its own rules).
- Planes: per‑face, per‑channel border planes (macro resolution), published with monotonically increasing `epoch`.

Data Structures
- Persistent per‑section arrays:
  - `block_light[16][16][16]: u8`
  - `skylight[16][16][16]: u8`
  - Optional: `beacon_light[...]` and `beacon_dir[...]` (directional, or treat as separate pass later).
- Per‑section metadata:
  - `epoch_block`, `epoch_sky` (u32): bump when section data changes.
  - Dirty flags (by channel).
  - Compact priority (distance ring) for scheduling.
- Per‑chunk planes (published):
  - `planes_block_xn/xp/zn/zp/yn/yp` (u8 arrays) + `epoch_plane_*` (u32 per face)
  - `planes_sky_*` similarly.

Queues
- For each region (e.g., chunk) maintain two queues per channel:
  - `DecreaseQueue`: items `(pos, prev_level)`
  - `IncreaseQueue`: items `(pos)`
- A global light‑task scheduler drains queues with a budget (`maxUpdateCount`), producing section diffs and plane diffs. Cross‑region work is re‑enqueued on neighbor when necessary.

Phase 1 — Decrease (Darken)
- Seed: On emitter removal/downgrade (block light) or on skylight occluder placement/removal (affects skylight columns), enqueue the affected position(s) with the previous recorded light level into `DecreaseQueue` for the owning section.
- BFS (bounded): While budget allows, pop from `DecreaseQueue`. For each neighbor:
  - Recompute “expected via this path” using the channel’s attenuation/cost (omni: −1 per step, skylight: −1 per step or 0 vertical, etc.) and passability (S=2 gating vs macro full‑cube rule). If neighbor’s current level equals exactly the value contributed through this path, then lower it (usually to the best of remaining neighbors or 0) and enqueue that neighbor into `DecreaseQueue`.
  - If neighbor’s level is higher because another route sustains it, do not lower; instead, enqueue that neighbor into `IncreaseQueue` (boundary for re‑light).
- Edges: If a step crosses a section/chunk boundary and neighbor isn’t resident, record a pending edge task (skip edge). When neighbor becomes resident/present, continue the decrease from the saved frontier.
- Output: Section changes (cells lowered) and per‑face plane changes; bump epochs accordingly.

Phase 2 — Increase (Re‑light)
- Seed: (a) all boundary cells recorded during Decrease; (b) all active emitters in or near the region (within range) and (c) skylight sources (open‑above columns and neighbor seam seeds) — the union yields initial frontier for `IncreaseQueue`.
- BFS (bounded): Standard flood (dial queue/radix buckets) subject to passability and costs; write when `new > old`.
- Edges: Same deferred cross‑section handling; enqueue neighbor section tasks when needed.
- Output: Section increases and plane changes; bump epochs.

Skylight specifics
- Skylight seeds from “open‑above” columns per column/section. Occluder placement/removal triggers local decreases along the column and across neighbors via boundary propagation.
- Vertical handling can be simplified by precomputing per‑column open height per section.

Beacons (optional split)
- Treat beacon as separate directional pass with its own persistent grid; or fold into block light with higher cost for turns. Recommendation: separate channel for clarity (stage 2).

Cross‑Chunk Seams
- Ownership: Keep canonical owners as today (e.g., +X/+Z), but any changed face publishes with a strictly increasing `epoch`.
- Neighbors: On plane update, a neighbor section enqueues an Increase or Decrease continuation (depending on delta sign). When neighbor not resident: store pending edge task keyed by (cx,cz,face,epoch) and re‑check on load.
- Epoch guard: On receipt, ignore planes with `epoch < current_epoch` to avoid reintroducing stale states.

Concurrency & Budgets
- Lighting worker lane(s) drain per‑section queues with a budget: `maxUpdateCount` per tick burst.
- Per‑region serialization: never run two lighting updates concurrently for the same section.
- Priorities: (Edit > Lights triggered by border > Background maintenance), distance‑gated.

Decoupling From Meshing
- Introduce a per‑chunk CPU light buffer (macro, optionally with a small micro assist for shading). Mesh sampling reads from this buffer via the `LightingStore` API.
- Optionally upload to GPU (SSBO/texture) so geometry stays static while light changes.
- Mesh rebuilds are only for geometry/topology edits — no rebuilds on lighting‑only changes.

Integration Plan (Phased)

Phase 0 — Prep (small)
- Add per‑face plane epochs into `LightingStore` and propagate in `LightPlanesUpdated`.
- Update consumers to ignore stale planes (keep current compute in place temporarily).

Phase 1 — Lighting Engine Skeleton
- Add `crates/geist-lighting/src/engine/`:
  - `section.rs`: structs for `LightSection`, per‑channel grids, epochs, dirty flags.
  - `queues.rs`: `DecreaseQueue`, `IncreaseQueue`, radix buckets, item types.
  - `engine.rs`: API for enqueue events, process with budget, cross‑section edge handling, plane publishing.
- Modify `LightingStore` to host sections and planes (persistent). Provide `get_section_mut/get_section` APIs and `publish_planes(chunk)`.
- Add `LightTask` to `crates/geist-runtime`: queues, workers, and result channel (`LightTaskOut` carries section diffs + plane diffs + epochs).

Phase 2 — Event & Scheduling Rewire
- src/event.rs: add `LightTaskScheduled`, `LightSectionUpdated`, `LightPlanesUpdated` with epoch.
- src/app.rs: on `BlockPlaced/Removed` and `LightEmitterAdded/Removed`:
  - Do not rebuild chunks for lighting; instead, call `LightingStore.enqueue_*` to seed decrease/increase and emit `LightTaskScheduled`.
  - Add a tick‑driven `LightingDoWork { budget }` to drain lighting tasks every frame.
- Light border changes: propagate via `LightPlanesUpdated` to neighbors (which enqueue tasks) — not by scheduling chunk rebuilds.

Phase 3 — Meshing Decouple
- crates/geist-mesh-cpu/src/build.rs:
  - Remove `compute_light_with_borders_buf(...)` call and delete return plumbing of `LightBorders`.
  - Meshers query light through `LightingStore.sample_face_local_s2(...)`, which reads persistent buffers.
- Temporary: If keeping per‑vertex colors, keep using `sample_face_local_s2`; for GPU light, add shader sampling.

Phase 4 — Remove Legacy Lighting Compute
- Delete or gate (feature flag) `compute_light_with_borders_buf` and the micro BFS in `crates/geist-lighting/src/micro.rs`.
- Keep micro passability helpers and S=2 face sampling helpers; they now read persistent macro light.

Phase 5 — Beacon Split (optional)
- Add directional channel and pass; separate queues as needed. Compose in sampling.

File‑Level Changes (Concrete)
- Remove/replace:
  - crates/geist-mesh-cpu/src/build.rs
    - Remove: calls to `compute_light_with_borders_buf`, `LightBorders::from_grid`.
    - Replace with: sampling via `LightingStore` and no `light_borders` in the return tuple.
  - crates/geist-lighting/src/lib.rs
    - Remove: `compute_light_with_borders_buf` as authoritative pathway.
    - Add: `engine` module exports; sampling APIs read persistent layers.
  - crates/geist-lighting/src/micro.rs
    - Remove: full‑volume micro BFS; keep only helpers for S=2 passability and face sampling.
  - crates/geist-runtime/src/lib.rs
    - Add: `LightTask` lane and workers; `submit_light_task`, `drain_light_results`.
  - src/event.rs
    - Add: new lighting events (task scheduled, section/planes updated, optional periodic budget tick).
    - Adjust: `LightBordersUpdated` → `LightPlanesUpdated` with epochs.
  - src/app.rs
    - Remove: decrease‑phase hacks and lighting recompute triggers.
    - Add: enqueue lighting tasks, process results (mark planes updated, optionally trigger tiny GPU light buffer updates). Remove light‑only chunk rebuilds.

Detailed Decrease/Increase Logic (Pseudo‑Code)

// For block light, omni with unit step cost (or MICRO_BLOCK_ATTENUATION if we
// keep micro step semantics; below shows macro unit steps for clarity)

decrease(seed_pos, prev_level):
  queue_dec.push((seed_pos, prev_level))
  while budget and queue_dec not empty:
    (p, level_through_removed_path) = queue_dec.pop()
    for n in neighbors(p):
      if not passable(p, n): continue
      expected = level_through_removed_path - step_cost(p, n)
      if expected <= 0: continue
      if light[n] == expected:
        // n was solely sustained by the removed path -> lower
        new_n = max( best_neighbor_except_removed(n), 0 )
        if new_n < light[n]:
          light[n] = new_n
          queue_dec.push((n, new_n))
      else:
        // sustained from another route; boundary for re‑light
        queue_inc.push(n)

increase(seed_set):
  for s in seed_set: queue_inc.push(s)
  while budget and queue_inc not empty:
    p = queue_inc.pop()
    for n in neighbors(p):
      if not passable(p, n): continue
      cand = light[p] - step_cost(p, n)
      if cand > light[n]:
        light[n] = cand
        queue_inc.push(n)

Skylight: similar but with skylight transparency, open‑above seeds, and vertical/atten rules; occluders seed decreases along columns + lateral neighbors.

Passability & S=2
- Use existing `micro_face_cell_open_s2`/`is_full_cube` helpers to decide passability at macro face granularity. Micro occupancy allows thin shapes to not block; full cubes block.

Planes & Epochs
- After each section update, compute affected faces (min/max extents touched) and refresh those faces’ plane arrays and `epoch_plane_*`.
- Emit `LightPlanesUpdated { cx, cz, face_mask, epoch }`.
- Consumers ignore any plane with stale epoch.

Sampling API (for meshing)
- `LightingStore.sample_face_local_s2(buf, reg, x, y, z, face) -> u8` reads persistent macro light and (if present) an optional micro overlay; no recompute.
- Optional: provide a flat function `sample_block_sky_at(cx, cz, lx, ly, lz)` for non‑face sampling clients.

Testing
- Unit tests:
  - Emitter remove near seams (all four faces). Verify neighbor sections darken after decrease and re‑light from remaining sources.
  - Skylight occluder placement/removal: vertical columns + lateral reprop.
  - Plane epoch ordering: older planes ignored.
- Property tests: random toggles with a per‑tick budget; assert convergence to expected field.
- Soak: burst edits near seams; ensure no long‑term oscillation; stable performance.

Migration Strategy
- 1: Introduce engine alongside current compute; gate old compute behind a feature flag.
- 2: Switch meshing to sample persistent light; keep old compute only for fallback.
- 3: Turn on engine as default; remove old compute.
- 4: Optionally add GPU light buffers and beacon channel split.

Performance Considerations
- Memory: 2 channels × 16×16×16 per section × sections per chunk; easily within budget even for large worlds.
- Work: bounded by budget per tick; tasks are small (local) except for large occluder changes, which spread but remain processed incrementally.
- Parallelism: sections update independently; serialize per section to avoid races.

Open Questions (decide during implementation)
- Section granularity: keep 16×16×16 or align to our chunk dimensions temporarily (single section per chunk) for first shipment.
- Beacon channel: separate now vs later.
- GPU light buffer adoption timing vs shipping CPU‑only first.

References / Inspiration
- Minecraft LightingProvider.doLightUpdates(maxUpdateCount, doSkylight, skipEdgeLightPropagation)
- ServerLightingProvider + ThreadedAnvilChunkStorage + TaskExecutor (off‑thread execution)
- Spottedleaf’s Starlight notes on separate decrease/increase queues and batching


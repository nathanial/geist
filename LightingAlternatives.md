Lighting Alternatives (Brainstorm)

Context
- Current micro S=2 lighting solves addition well (bucketed BFS), but removal and seam convergence can stall or “stick”.
- Emission is sampled from block defs and runtime emitters; planes exchange across seams; meshing bakes light into vertex colors.
- Symptoms we’re seeing:
  - Glowstone near seams leaves residual light after removal (stuck brightness on one or both sides).
  - Over‑eager neighbor rebuilds cascade; when damped, light can lag or persist.

Goals
- Robust add/remove with bounded scope and predictable convergence.
- Seam‑stable first draw; minimal remeshes; avoid ping‑pong.
- Keep memory reasonable; keep code testable (unit tests for seams and removals).

Key Failure Modes Today
- Monotonic max: light fields that only grow unless we explicitly reflow a region after removal.
- Provenance loss: we store only max values, not which source contributed; removal needs either provenance or local recompute.
- Seams: neighbors can be out of sync; “owner vs dependent” handshakes reduce but don’t eliminate partial states.
- Coupling: geometry remesh ties to lighting changes; even small light edits induce full remeshes.

Minecraft Model (Decrease/Increase BFS)
- Removal/downgrade uses two phases:
  - Phase 1 — Decrease (darken): enqueue the removed position with its previous level and breadth‑first walk outward. Lower a neighbor only when its current value exactly matches what it would have received from the removed path; otherwise, leave it and mark that boundary for phase 2.
  - Phase 2 — Increase (re‑light): from the still‑lit boundaries and any other active emitters/skylight, run a normal flood to refill valid light. This prevents over‑darkening and avoids full region recomputes.
- Skylight and occluders follow the same pattern: placing/removing opaque blocks triggers a decrease along the affected columns, then an increase to re‑propagate skylight.
- Scheduling/budgeting:
  - Updates are processed off the main game thread on worker/executor pools (e.g., ServerLightingProvider with TaskExecutor/ChunkTaskPrioritySystem), effectively serialized per region via task queues.
  - Work is batched with budgets similar to `doLightUpdates(maxCount, doSkylight, skipEdgeLightPropagation)`; cross‑section/edge work can be deferred until neighbors are resident (skip‑edge propagation) and finished later.

Alternatives (with pros/cons)

1) Emitter‑Centric Incremental Lighting with Counters
- Idea: Every light write carries a contribution id (emitter id) or a small counter of contributors. Removal decrements and reflows only where counts drop to zero.
- Data:
  - For each macro cell (and optionally micro planes), store:
    - light value (u8)
    - contrib count (u8) OR a compact bitset over a small, rolling emitter pool per chunk
    - epoch (u16) per cell to extinguish stale writes without full clears
- Ops:
  - Add: BFS/Dijkstra from emitter; when a target cell’s (new_value > old_value) write succeeds, increment count and stamp epoch.
  - Remove: re‑emitters in a local radius with “ownership” scan; for cells where count becomes zero, recompute max from remaining neighbors (limited region) and push decreases.
- Pros: Precise removal without global recompute.
- Cons: Memory overhead (counts/ids); complexity in merging across seams; emitter pool management.
- Notes: Pool can be capped (e.g., 8–16 local active emitters per chunk); overflow falls back to region recompute.

2) Region‑Bound Local Recompute on Edit (Lazy Retract)
- Idea: When an emitter is added/removed/changed, recompute a bounded region (R in macro cells; or K micro steps) from scratch using current world + neighbor planes.
- Data: None extra beyond current fields; keep a dirty AABB queue per chunk.
- Ops:
  - Compute dirty AABB: conservative bounds: sphere with radius based on max range/attenuation; expand one more cell to capture boundary conditions.
  - Within AABB: reset lighting (to zero) and re‑run micro BFS seeded by emitters that lie inside or on AABB border, and by seam planes clipped to AABB faces.
  - Publish updated planes if AABB touches chunk border.
- Pros: Simple; no provenance; deterministic; handles both add/remove.
- Cons: Recomputes some light unnecessarily; needs careful AABB union/coalesce; spikes when lots of edits close together.
- Notes: Combine with per‑frame budget and queue coalescing; degrade to larger box recompute if too many overlapping dirty requests.

3) Canonical Seam “Halo BFS” (Owner Synthesizes Neighbor)
- Idea: Each chunk’s solve extends a thin halo (1–2 macro cells, or K micro steps) outside its boundary using world sampling. Published seam planes are thus neighbor‑independent and stable on first draw.
- Data: No persistent extra; needs world sampler at lighting time.
- Ops:
  - During lighting, allow BFS to step across the seam by a small number of steps using world/block sampling instead of neighbor planes.
  - Generate seam planes from the interior + halo; neighbors sample those planes directly, never needing neighbor recomputes for light‑only changes.
- Pros: Stable first mesh; eliminates light order dependence.
- Cons: Slightly costlier lighting; requires careful gate at vertical bounds and micro occupancy across halo cells.
- Notes: This pairs well with 2) for removal inside the chunk.

4) Two‑Grid Ping‑Pong with Epochs (Idempotent Convergence)
- Idea: Maintain two buffers (A,B) with per‑cell epochs. Edits write epoch+1; propagation reads only from current epoch; at the end, swap. Removals naturally converge to new max because nothing reads stale epochs.
- Data: two u8 grids + u16 epoch grid (or single u8 epoch delta per chunk revision).
- Ops:
  - For a lighting step, set epoch = rev; BFS reads and writes only that epoch; old epoch values are ignored by samplers.
  - Seam planes carry epochs; neighbors ignore older planes.
- Pros: Poison‑pill for stale light; no explicit removal path.
- Cons: Requires re‑lighting the whole chunk (or large subregion) for every edit; mitigable with 2) region bounds.

5) Distance‑Field/Dijkstra with Parent Pointers (Selective Retract)
- Idea: Store parent direction (one nibble) and source id/tag for each lit cell. On removal, walk the frontier where parent pointed to the removed source until alternative neighbor beats it.
- Data: For each cell: light (u8), parent dir (3 bits), source tag (8–16 bits bounded).
- Ops:
  - Add: Dijkstra (dial queue); set parent for each improved cell.
  - Remove: start from removed source’s cells; follow parent chains, decrement; when neighbor provides ≥ old level, stop.
- Pros: Retract limited to actual dependency tree; less recompute.
- Cons: Memory and complexity; merging trees across seams needs stable source tags.

6) Decouple Lighting from Meshing (GPU Light Texture/SSBO)
- Idea: Keep geometry static. Upload a small light texture/SSBO per chunk (macro or micro). When lighting changes, update only this buffer; fragment shader samples light.
- Data: per‑chunk light atlas or SSBO; optional seam padding for sampling.
- Ops:
  - On lighting change: no remesh; just update light buffer.
  - On seam change: update small border regions; neighbors sample seamlessly.
- Pros: Eliminates remesh churn; isolates lighting correctness from geometry.
- Cons: Requires shader plumbing and texture updates; careful handling of thin shapes (sample face light vs center).
- Notes: Combine with 3) to reduce dependency on neighbor presence.

7) Beam/Directional Source Pass (Beacons) as Separate Channel
- Idea: Treat beacons as a separate directed pass with its own data (dir + level). Don’t fold into omni grid; compose at sample time.
- Data: optional per‑cell “best beacon” (level + 2‑bit cardioid dir) or per‑face beam planes.
- Ops:
  - Run a low‑cost directional Dijkstra for beams; store per‑cell dir+level; downsample to planes for seams.
  - Compose final face light as max(omni, skylight, beacon).
- Pros: Cleaner semantics; easier to debug; avoids beacon artifacts in omni removal.
- Cons: More memory; modest compute.

8) Seam Protocol Refinements
- Ownership: Only owners publish their positive faces (+X/+Z); dependents never publish back.
- Planes:
  - Plane epochs: planes carry a monotonically increasing revision; neighbors ignore older.
  - Plane deltas: optionally publish run‑length compressed deltas when below a threshold to reduce jitter.
  - Finalize barrier: dependent chunk meshes only after both owners have published at least once; no timeouts.
- Neighbor targeting:
  - Per‑face change mask in events; schedule only dependent neighbors; avoid −X/−Z ping‑pong.
  - Post‑finalize, allow light‑only updates to skip geometry remesh when 6) is in place.

9) Scheduling & Budgeting
- Intent queue prioritization by cause (Edit > Light > StreamLoad) and ring distance.
- Budgets per lane (edit/light/bg) based on worker counts; skip far‑ring light updates.
- Coalesce multiple dirty regions per chunk before scheduling a lighting job; favor finalize passes over speculative remeshes.
 - Mirror Minecraft’s knobs: a per‑tick `maxUpdateCount` budget and a flag to defer cross‑chunk edges (our analogue to `skipEdgeLightPropagation`) until neighbor chunks/sections are ready.

11) Two‑Phase Decrease/Increase BFS (Minecraft‑style)
- Idea: On removal/downgrade, first run a targeted darkening pass that retracts only light that depended on the removed path; then run a normal propagation from remaining sources and the darkening boundary to restore persistent light.
- Data: Two work queues per channel (decrease, increase). Optional bitmask/epoch per cell to avoid duplicate enqueues within a job. No per‑cell provenance required.
- Ops:
  - Decrease: seed with the removed position and previous value; for each popped cell, if a neighbor’s value equals the value it would have had via this path, lower it and enqueue; if not, push that neighbor into the increase queue (boundary).
  - Increase: seed with boundary cells plus any nearby emitters/skylight; run standard BFS/Dijkstra to refill.
  - Borders: enqueue cross‑chunk edges as “edge work”; if neighbor section isn’t loaded, tag it and complete once available (our skip‑edge analogue).
- Pros: Precisely removes stale light without over‑darkening; cheap compared to full region recompute; matches well‑known behavior and expectations.
- Cons: Slightly more complex control flow than a single flood; needs careful seam handling to avoid ping‑pong.
- Notes: Applies to both block light and skylight; can be capped per tick via a budget like `doLightUpdates` and scheduled off‑thread.

10) Data Packing & Performance Notes
- Nibble packing (4‑bit) for macro light grids is feasible; micro planes remain u8.
- Dial queue (radix buckets) performs well for uniform attenuations; use Dijkstra only when costs vary (beams).
- Parallel frontier expansion is OK if merges are serialized; region recompute can be parallel by tiles.

Recommended Path (practical, staged)
- Stage 1: Two‑phase decrease/increase BFS (Alternative 11)
  - Implement Minecraft‑style darken‑then‑re‑light for block light and skylight, with separate decrease/increase queues and a per‑tick `maxUpdateCount` budget.
  - Treat cross‑chunk edges as first‑class: defer edge work until neighbors are resident and finish it deterministically (skip‑edge analogue).
  - Keep canonical seam ownership + finalize barrier and per‑face change masks.
- Stage 2: Seam halo (Alternative 3)
  - Extend solve by a thin halo using world sampling; derive planes from halo.
  - This stabilizes first draw and reduces neighbor dependency.
- Stage 3: Decouple lighting from meshing (Alternative 6)
  - Upload per‑chunk light buffers; remove geometry remesh on light changes.
  - Mesh only on geometry edits or topology‑affecting shape changes.
- Stage 4: Beacon pass split (Alternative 7)
  - Separate directed beams; simpler reasoning and fewer cross‑effects.
- Stage 5: (Optional) Emitter provenance or parent pointers (Alternative 1 or 5)
  - Use where precise retract is critical (heavy edit tools), else rely on 2).

Edge Cases and Fixes for “Stuck” Light
- Plane epochs + neighbor ignore: if a neighbor holds a brighter but older plane, it must ignore it; epochs solve this.
- Two‑phase decrease then increase: guarantees stale light is removed, and remaining light flows back from valid sources without over‑darkening.
- Region recompute on removal (fallback): guarantees local decreases even without provenance; keep as a safety net for complex edits.
- Out‑of‑bounds owners: treat as “ready” in finalize, or synthesize halo planes; prevents dependent waiting at world edges.
- Vertical seams: if/when vertically chunked, mirror ownership and plane exchange for +Y as needed.

Testing Plan
- Unit tests:
  - Add/remove omni emitter near each seam; assert neighbor planes lower accordingly.
  - Removal darken‑then‑re‑light: after downgrading/removing a source, verify the darkening frontier retracts only dependent cells and relight restores brightness from other sources.
  - Skylight occluder edits: place/remove opaque blocks in columns; confirm decrease followed by correct skylight re‑propagation across chunk borders.
  - Cross‑chunk removal with both owners missing or present; ensure finalize barrier triggers once.
  - Beacon beams across corners; verify direction preserved.
- Property tests:
  - Random emitter toggles with two‑phase BFS under a budget; assert no cell exceeds expected max and the system converges to the baseline.
- Soak tests:
  - Repeated add/remove near seams while camera orbits; track rebuild counts; should settle with no long‑term cascades.

Migration Notes
- Start with 11) in the current micro engine: implement the decrease/increase queues and cross‑chunk edge deferral with a per‑tick budget.
- Add plane epoching without changing consumers; wire masks/events later.
- 6) can be prototyped behind a feature flag; keep CPU fallback.

References
- Spottedleaf, “The future of the Starlight mod” (decrease/increase overview)
- Yarn docs: `LightingProvider#doLightUpdates`, `ServerLightingProvider`, `ThreadedAnvilChunkStorage`, `TaskExecutor`, `ChunkTaskPrioritySystem`

Open Questions
- How large should the region radius be? Use min(max_range, attenuation threshold to drop below VISUAL_LIGHT_MIN) with micro step units.
- How to compress plane deltas efficiently? RLE or bitplane for non‑zeros is probably enough.
- Where to keep emitter ids if we adopt provenance? Chunk‑local pool vs. world‑global small IDs.

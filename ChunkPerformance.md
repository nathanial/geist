# Chunk Build Performance: Reducing Rebuilds and Making Rebuilds Cheaper

This doc summarizes what we are seeing in the event stream and proposes
changes to reduce the number of chunk rebuilds we do, and to make each
rebuild faster so spikes don’t matter.

Observed session (example)
- EnsureChunkLoaded: 841 processed
- ChunkRebuildRequested: 1430 processed
- BuildChunkJobRequested: 2266 processed
- LightBordersUpdated: 1573 processed

The ratios suggest we are rebuilding many chunks more than once after load,
with extra rebuilds driven by light propagation, edits, and “stale” job
completions that requeue newer work. A non-trivial portion of these jobs also
regenerates the chunk’s block buffer from worldgen, even when geometry didn’t
change, which is expensive.

Goals
- Fewer rebuilds per chunk: coalesce and de-duplicate.
- Prioritize what matters: build visible/nearby first.
- Make rebuilds cheaper: avoid worldgen when possible, reuse memory, and mesh
  only what changed.


## Reality check: current implementation

Code paths were reviewed to anchor proposals against what actually runs today.
Key observations with file/function pointers:
- Scheduling:
  - `EnsureChunkLoaded` enqueues `BuildChunkJobRequested` and adds `(cx,cz)` to `gs.pending` (src/app.rs, event handler).
  - `ChunkRebuildRequested` enqueues `BuildChunkJobRequested` immediately if the chunk is loaded and not `pending` (src/app.rs). There is no per‑frame coalescing or visibility gating yet.
- Inflight/stale handling:
  - `BuildChunkJobCompleted` re‑enqueues immediately when `rev < cur_rev` without checking for a newer inflight request (src/app.rs). This can duplicate work while `pending` is still set, because the `BuildChunkJobRequested` handler submits directly to workers and does not consult `pending`.
- Worker behavior:
  - Each job regenerates the full `ChunkBuf` from worldgen via `generate_chunk_buffer`, applies chunk/region edits, then meshes (src/runtime.rs worker loop). There is no reuse of a prior buffer or lighting‑only path.
- Light borders fan‑out:
  - On `LightBordersUpdated`, neighbor rebuilds are emitted immediately for loaded and not‑pending neighbors (src/app.rs). No per‑frame fan‑in, so multiple border updates within a tick can schedule multiple neighbor rebuilds.
- Determinism:
  - Results are drained once per tick, sorted by `job_id`, then converted to `BuildChunkJobCompleted` events (src/app.rs). Good.
- Worldgen hot‑reload:
  - If `rebuild_on_worldgen` is enabled, all loaded chunks are scheduled for rebuild at the next `step()` (src/app.rs). There is no progressive scheduling.
- GPU upload:
  - A new Raylib `Model` is created and uploaded on every completion; there is no in‑place mesh buffer update or model pooling (src/mesher.rs `upload_chunk_mesh`).
- Logging:
  - `BuildChunkJobRequested`/`Completed` log at `info` level per event (src/app.rs), which can add overhead during rebuild storms.

These match the symptoms captured in the counters (high `BuildChunkJobRequested`, many stale re‑queues, border‑driven rebuilds).


## Reduce the number of rebuilds

1) Coalesce and dedupe rebuild triggers
- Debounce within a frame: Collect rebuild requests (`ChunkRebuildRequested`) in
  a set, then schedule a single `BuildChunkJobRequested` per chunk at the end of
  the frame. This prevents multiple requests for the same chunk from edits,
  lighting, and neighbor border changes from spawning multiple build jobs.
- Use a “scheduled or inflight” map: If a chunk is already scheduled or inflight,
  drop subsequent `ChunkRebuildRequested` for that chunk. Today we use
  `gs.pending` to avoid duplicate build requests in some paths, but requests can
  still accumulate in the queue before we check `pending`. A front-of-queue set
  (or a `rebuild_scheduled` set) prevents event queue growth and overcounting.

2) Avoid re-enqueueing on stale completions when a newer build is already inflight
- In `BuildChunkJobCompleted` we currently re-enqueue if `rev < cur_rev`:
  we should only re-enqueue if there isn’t a newer build already scheduled or inflight.
  Use an `inflight_rev: HashMap<(cx,cz), u64>` to track the newest rev we sent to
  workers. If a stale job completes and `inflight_rev.get(&(cx,cz)) >= cur_rev`,
  skip re-enqueue.

Implementation details (fits the current code):
- Add `gs.inflight_rev: HashMap<(i32,i32), u64>`.
- When emitting `BuildChunkJobRequested`, set `inflight_rev[(cx,cz)] = rev`.
- In `BuildChunkJobCompleted` stale branch, re‑enqueue only if
  `!gs.pending.contains(&(cx,cz)) && inflight_rev.get(&(cx,cz)).copied().unwrap_or(0) < cur_rev`.
  Otherwise, drop the stale result.
- Clear `inflight_rev` entry when the latest completion for that `(cx,cz)` is accepted.

3) Tame LightBordersUpdated fan-out
- Border events often cause neighbor rebuilds that then trigger more border
  events, and so on. Collect border changes per-frame into a set, then on frame
  end, schedule at most 1 rebuild per affected neighbor. If a neighbor is not
  visible (see 5), delay its rebuild until visible.

4) Coalesce edits into a per-frame flush
- We already snapshot edits and region edits. When a burst of block edits occur
  (e.g., placements/removals in quick succession), coalesce into one per-chunk
  rebuild at frame end.

5) Visibility-based gating (on-demand rebuild)
- If a chunk is dirty but outside the frustum, don’t build immediately. Mark
  dirty and rebuild only when it becomes visible or reaches a distance/age
  threshold. This directly cuts builds that aren’t seen and reduces spikes
  during fast camera moves.

Implementation notes:
- Add `dirty_chunks: HashSet<(i32,i32)>` and only flush to jobs for chunks that intersect the current frustum (expanded by a small margin) or exceed an age/distance threshold.
- The renderer already computes a frustum (src/app.rs). Reuse the same bounds check against each chunk’s `ChunkRender.bbox`.

6) Worldgen hot-reload rebuild policy
- Today, worldgen hot-reload schedules rebuilds for all loaded chunks. Offer
  policies:
  - Manual: show a toast “Worldgen changed — press R to rebuild loaded chunks”.
  - Progressive: rebuild closest N per frame until done.
  - Off by default; still applies to newly loaded chunks.


## Make each rebuild faster

1) Reuse existing ChunkBuf; avoid worldgen unless necessary
- Worker currently does `generate_chunk_buffer(&world, cx, cz, &reg)` on every
  build, then applies edits, then meshes. For many rebuilds (lighting-only,
  neighbor border changes), world contents didn’t change.
- Strategy:
  - Keep the previous `ChunkBuf` as authoritative while loaded.
  - Only regenerate from worldgen when (a) the chunk wasn’t generated yet,
    or (b) worldgen config changed. Otherwise, clone or shallow-copy the
    existing buffer and apply edits.
  - Optionally introduce a lightweight “apply region edits” path that doesn’t
    require full buffer copy.

Clarifications for this codebase:
- `ChunkBuf::clone()` clones the `Vec<Block>` (full copy). That’s still far cheaper than per‑voxel worldgen in most cases and enables an immediate “reuse + apply delta edits” path.
- For lighting‑only rebuilds, skip even the edit application and mesh straight from the last accepted `ChunkBuf`.
- Long‑term: use a per‑worker scratch `Vec<Block>` and `Vec::clone_from` to reuse capacity and avoid reallocations.

2) Partial remesh by sub-chunk section
- Track dirty ranges (e.g., 16×16×16 sections). If the edit/light change is
  localized, only remesh those sections and stitch borders. For lighting-only
  changes, recompute per-vertex/face lighting where needed without geometry
  rebuild. Emit meshes per section and rebind the changed sections.

Notes on fit:
- World is chunked only in X/Z today; Y is a single slab. Sectioning can be an internal mesher concept: generate 16³ section meshes and aggregate at upload time. Start with “geometry unchanged, lighting‑only recolor” as the lowest‑risk step.

3) Improve greedy meshing and batching
- Ensure greedy meshing merges across long runs, per material and face.
- Maintain material-sorted batches to minimize draw calls (we already do this to
  some extent). Verify we’re not over-splitting meshes per material.

Extra checks:
- Pre‑reserve mesh buffers per chunk using the previous build’s sizes to avoid repeated reallocations (`MeshBuild` fields in src/mesher.rs).

4) Memory and allocation hygiene
- Use thread-local arenas/Vec pools for vertex/index buffers; reserve capacity
  based on past chunk sizes to prevent repeated reallocations.
- Reuse `FastNoiseLite`/noise contexts (already done in worldgen path per-worker
  ctx; extend this where feasible).

Additionally:
- Pool Raylib `Model`/`Mesh` objects per chunk and update buffers in place where the API allows, instead of re‑creating models every rebuild.

5) Parallelism and prioritization
- We already launch workers equal to available cores. Introduce a priority queue:
  near-visible chunks first, then other dirty chunks. Let distant/hidden chunks
  trickle through.

Concrete hook:
- Replace direct `ChunkRebuildRequested -> BuildChunkJobRequested` with a per‑frame scheduler that sorts by priority (visibility, distance to camera, cause).

6) Mesh LOD for distance
- For distant chunks, generate simplified meshes (e.g., decimate surfaces or
  collapse small features). This reduces CPU time and draw cost. Switch to full
  meshes as camera approaches.

7) Separate lighting updates from geometry
- Many rebuilds originate from lighting changes. If geometry is unchanged,
  recompute lightmaps/vertex lighting and reuse indices/positions. The mesher
  can support a “lighting-only” path that re-emits colors without rebuilding
  faces.

Concrete path:
- Keep the last accepted CPU mesh around (already produced in `JobOut`). For lighting‑only rebuilds, re‑run lighting, resample per‑face/vertex light, and only update the color buffer when uploading to GPU.

## Additional quick wins in this repo

- Lower log level for high‑volume events:
  - Demote `BuildChunkJobRequested`/`Completed` logs from `info` to `debug` (src/app.rs). This meaningfully reduces overhead during rebuild bursts.
- Coalesce `ChunkRebuildRequested` per frame:
  - Maintain `requested_rebuilds: HashSet<(cx,cz)>` and flush once per `step()` after draining worker results.
- Guard duplicate re‑requests by `job_id` when possible:
  - If a rebuild for the same `(cx,cz,rev,neighbors_mask)` was already enqueued this frame, drop subsequent requests.



## Quick wins (order to implement)

1) Dedupe stale-completion re-enqueues
- Add `inflight_rev` per chunk. On stale completion (`rev < cur_rev`), requeue
  only if `!pending.contains(chunk)` AND `inflight_rev.get(chunk) < cur_rev`.
- Expectation: cuts a big chunk of extra `BuildChunkJobRequested` spam.

2) Debounce rebuild requests per frame
- Maintain `requested_rebuilds: HashSet<(cx,cz)>` while handling events.
- At the end of `step()`, for each in the set, if not pending/inflight and
  (visible or close enough), emit one `BuildChunkJobRequested` and clear.

3) Reuse ChunkBuf on rebuilds
- Worker path: if a `ChunkBuf` exists for (cx,cz) and worldgen hasn’t changed,
  start from that buffer instead of regenerating from world.
- If edits exist, apply them; if only lighting changed, skip even that.
- Expectation: removes worldgen compute from most rebuilds, slashing CPU time.

4) Progressive worldgen rebuilds
- When worldgen config flips, schedule rebuilds in distance rings (or N per
  frame) instead of all at once, with a hotkey for manual trigger.

5) Border-change fan-in
- Accumulate neighbor rebuilds from `LightBordersUpdated` and issue one per
  neighbor per frame. Combine with visibility gating to further reduce work.


## Metrics to watch
- Per-chunk build count: `BuildChunkJobRequested / EnsureChunkLoaded` should be
  close to 1–1.5 under normal camera motion.
- Queue depth and age: queued count and oldest age (ticks) to ensure smoothness.
- Stale completion rate: number of stale completions re-enqueued vs. dropped.
- Rebuild cause breakdown: edits, streaming, lighting, worldgen.
- Time-to-first-draw for newly loaded chunks.

Instrumenting in current code:
- Add counters on the `App` side for: deduped `ChunkRebuildRequested` per frame, dropped stale completions (due to `inflight_rev`), and visibility‑gated deferrals.
- Track last mesh sizes per chunk to drive `reserve()` sizes for `MeshBuild`.


## Implementation sketch (minimal patches)

- Track inflight revs:
  - App: `inflight_rev[(cx,cz)] = rev` when issuing BuildChunkJobRequested.
  - On completion: if stale, only requeue if `inflight_rev.get(chunk) < cur_rev`.
- Debounce rebuilds:
  - App: collect `ChunkRebuildRequested` into a set; flush once per frame with
    pending/inflight checks + visibility filter.
- Reuse buffers:
  - Runtime worker: pass an optional existing `ChunkBuf` (or a handle to fetch
    it) so worker doesn’t regenerate from worldgen unless flagged.
  - On worldgen dirty, mark buffers invalid and revert to worldgen-only for the
    next build of each affected chunk.

Fit to current modules (low‑risk edits):
- src/gamestate.rs: add `inflight_rev`, `dirty_chunks` (and optionally `last_mesh_sizes`).
- src/app.rs:
  - Update `EnsureChunkLoaded`/`ChunkRebuildRequested` to push into a `requested_rebuilds` set.
  - At end of `step()`, flush the set in priority order, writing `gs.inflight_rev` when emitting `BuildChunkJobRequested`.
  - In `BuildChunkJobCompleted`, apply the stale re‑enqueue gating noted above.
  - Demote noisy logs to `debug`.
- src/runtime.rs:
  - Extend `BuildJob` with an optional prior `ChunkBuf` and a `mode` enum: `Full`, `FromPrevBuf`, `LightingOnly`.
  - Worker: if `FromPrevBuf`, start from the provided buffer (clone blocks Vec, apply edits), else fall back to worldgen. If `LightingOnly`, skip block mutation and rebuild only lighting/mesh colors.
- src/mesher.rs:
  - Add an upload path that updates only mesh color buffers when geometry is unchanged.

These are focused, low-risk changes that should immediately reduce the counts
you’re observing and cut worst-case CPU.


## Longer-term
- Sectioned (16³) partial remesh + light-only pass.
- Mesh LOD for distance.
- GPU-assisted meshing for high-end targets, if CPU still burns.

Exploratory ideas specific to this engine:
- Share simple “solid border bitmasks” alongside `LightBorders` so the mesher can decide border occlusion without sampling neighbor worldgen per face (reduces cross‑chunk `world.block_at_runtime` calls).
- Progressive worldgen rebuilds upon config change: batch N per frame closest to camera with backpressure to keep frame time stable; provide an on‑demand hotkey for “rebuild all loaded now”.


## Bottom line
Cut redundant rebuilds with dedupe/debounce + visibility gating, then make the
remaining rebuilds cheaper by reusing `ChunkBuf` and avoiding worldgen in the
common path. These should bring `BuildChunkJobRequested` much closer to
`EnsureChunkLoaded` while keeping the scene responsive during heavy edits and
lighting updates.

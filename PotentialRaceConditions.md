Potential Race Conditions: Revisions, Pending, and In‑Flight

Summary
- Symptom reported: After an initial successful edit, subsequent edits to the same chunk sometimes never appear. This tends to happen if an edit is made while a previous build is still pending/in‑flight.
- Root cause candidate: The stale‑completion requeue logic now gates on `!pending`, which can suppress the only path that schedules a build for the new revision. The chunk can remain stuck in `pending=true` with no active job for the latest `rev`.

Key Concepts in Current Code
- `rev` (per chunk): Latest requested change in `EditStore` (bumped by `bump_region_around`).
- `built_rev` (per chunk): Rev that was last accepted (via `mark_built` and in `ChunkEntry.built_rev`).
- `pending: HashSet<(cx,cz)>`: Tracks chunks for which we have an outstanding build request/job.
- `inflight_rev: HashMap<(cx,cz), u64>`: New addition used to record the newest rev we’ve submitted to workers for that chunk.
- Event flow:
  1) Edit → `EditStore.set` + `bump_region_around` → `ChunkRebuildRequested` for affected chunks (only if loaded).
  2) `ChunkRebuildRequested` → if chunk is loaded and NOT `pending`, emit `BuildChunkJobRequested` and set `pending=true`, record `inflight_rev=rev`.
  3) Worker result drained → emit `BuildChunkJobCompleted`.
  4) On completion: If `rev < cur_rev` treat as stale.

Where The Race Manifests
Timeline (one chunk):
1) T0: Chunk A has `built_rev = 10`, `pending = false`.
2) T1: User makes an edit → `rev = 11`. We emit `BuildChunkJobRequested(rev=11)`, set `pending = true`, `inflight_rev = 11`.
3) T2: Before `rev=11` job completes, user makes another edit → `EditStore.bump_region_around` sets `rev = 12`. We emit `ChunkRebuildRequested`, but handler sees `pending = true` and returns early (by design), deferring the reschedule to stale‑completion handling.
4) T3: The `rev=11` job completes → `BuildChunkJobCompleted(rev=11)` arrives. `cur_rev = 12` so this completion is stale.
5) Stale branch (current logic) checks:
   - `inflight = inflight_rev.get(A).unwrap_or(0)`
   - if `!pending && inflight < cur_rev` then requeue latest.
   - Problem: `pending` is still true (for the job that just completed). Because of `!pending`, we do NOT requeue.
   - We return early from the stale branch without clearing `pending` and without scheduling a job for `rev=12`.
6) Result: The chunk stays `pending=true` with no active job for `rev=12`. Subsequent `ChunkRebuildRequested` events are dropped by the `pending` guard, so edits never show up.

Why this used to “work once”
- Before adding the `!pending` condition, the stale branch always re‑enqueued the latest `rev` unconditionally. So step (5) would schedule `rev=12` and the build would proceed.
- With `!pending`, the only path that schedules a new job while a prior one is in flight has been blocked.

Secondary Risk Areas (less likely cause of the reported symptom)
- Job sorting and dual completions in same tick: We sort `JobOut` by `job_id` before emitting `BuildChunkJobCompleted`. If two completions for the same chunk arrive in one tick (e.g., a stale and a newer), the stale one could be processed after the newer one depending on hash ordering. Our stale check uses `cur_rev` (authoritative), so the newer accept path would set `built_rev` and clear `pending`; a later stale would be dropped harmlessly. This is unlikely to cause “stuck pending,” but worth monitoring.
- `inflight_rev` lifecycle: We set it on request and clear it only on accepted completion. For stale completion requeues, we do set it to the new `cur_rev` when re‑emitting (good). The observed issue arises when we fail to re‑emit.
- `prev_buf` reuse: The worker starts from the prior buffer and reapplies edit snapshots. That is idempotent and should not suppress geometry changes. Not a direct cause of “never shows” symptoms.

Concrete Repro (most deterministic)
1) Make an edit; observe `BuildChunkJobRequested` for the chunk; pending becomes true.
2) While the job is still inflight (e.g., immediately place/remove another block), generate another edit on the same chunk.
3) Observe logs when the first (`rev=N`) completion arrives:
   - `BuildChunkJobCompleted (rev=N)`, `cur_rev=N+1`.
   - If the stale branch logs “drop” due to `pending=true`, and no new `BuildChunkJobRequested` appears, the chunk will remain `pending` and the new edit will never build.

Minimal Fix Direction (do not apply yet; for discussion)
- In stale branch of `BuildChunkJobCompleted`, remove the `!pending` guard. The decision to requeue should depend only on `inflight_rev < cur_rev`.
  - Rationale: `pending` reflects that some job was in flight (possibly the stale one); it does not necessarily imply a job for the latest `cur_rev` exists. The correct dedupe is “only schedule if we haven’t already scheduled the latest rev,” which is exactly what `inflight_rev` captures.
- Pseudocode:
  - If `rev < cur_rev` and `inflight_rev.get(chunk) < cur_rev`: emit `BuildChunkJobRequested` for `cur_rev`; set `inflight_rev[(cx,cz)] = cur_rev`; keep `pending=true`; return.
- Optional: Add diagnostic logs/counters for this path: when we drop stale completions, when we requeue on stale, and the values of `pending`, `rev`, `cur_rev`, `inflight_rev`.

Additional Observations
- `ChunkRebuildRequested` is intentionally a no‑op when `pending=true`. That design relies on stale‑completion requeue to carry newer revisions forward while the pipeline is busy. Any gating added to stale requeue must be carefully chosen to not suppress that responsibility.
- A more robust scheduling model would be to collect rebuild intents per frame and coalesce by latest `rev`, independent of `pending`; but that’s outside the immediate scope.

Actionable Debugging Aids (non‑functional)
- Add temporary `debug!` logs on stale branch showing `(cx,cz)`, `rev`, `cur_rev`, `inflight`, `pending`, plus whether we requeued or dropped.
- Track a per‑chunk “stuck pending for > X ms/ticks” metric to surface this state directly in the overlay.

Bottom Line
- The reported “edit never appears after first time” matches a race where the stale completion is the only opportunity to schedule the new `rev`, but a `!pending` guard prevents it. Pending remains set and the chunk never rebuilds again. The fix is to base stale‑completion requeue solely on `inflight_rev` vs. `cur_rev`, not on `pending`.


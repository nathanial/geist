Event Prioritization: Options, Trade‑offs, and a Practical Plan

Problem
- While terrain is streaming or a bulk rebuild is underway (e.g., hot‑reload of worldgen), user edits and lighting updates can be delayed because they sit behind low‑value work for far‑away chunks.
- We want near‑camera edits/lighting/loads to “cut the line” so the world feels responsive, without sacrificing correctness.

Current Behavior (quick recap)
- The App handles events and immediately calls `submit_build_job` for each `BuildChunkJobRequested`. Runtime’s dispatcher forwards jobs FIFO to workers.
- No prioritization exists; arrival order wins. If many far chunks are requested first, they can monopolize worker capacity for a while.
- Correctness is protected by revision checks on completion (accept only latest `rev`) and deterministic acceptance ordering within a tick. Arrival order does not change final state, only latency.

Correctness Considerations
- Reordering job submission does not affect the final world state:
  - `BuildChunkJobCompleted` is revision‑checked; older results are dropped or cause a requeue of the latest `rev`.
  - Meshing across chunk seams is robust: we avoid cross‑seam culling without a loaded neighbor, so drawing earlier/later doesn’t create holes (worst case: temporary overdraw).
  - Lighting borders propagate via idempotent events; reordering only affects when neighbors update, not what they become.
- Therefore, it’s safe to prioritize near‑camera work for latency without risking consistency.

Prioritization Dimensions
- Cause: Edit > LightingBorder > StreamLoad (view radius) > WorldgenHotReload bulk.
- Distance: Near camera first; use frustum + distance buckets to avoid sqrt.
- Visibility: Chunks outside the frustum are lower priority (or deferred).
- Age/Fairness: Prevent starvation for background tasks by aging priority upward over time.

Design Options

1) App‑Level Priority Scheduler (coalescer + capped submit)
- Idea: Buffer rebuild intents per frame (or tick), rank by priority, and submit only the top K to Runtime each frame.
- Pieces:
  - Coalesce by `(cx,cz)`: keep latest `rev`, strongest cause (Edit beats StreamLoad), most recent timestamp.
  - Compute a priority key per chunk: `(cause_score, distance_bucket, -age_bucket)`.
  - Submit up to `K = worker_count * factor` per frame; leave the rest queued for the next frame.
  - Always allow “immediate” submit for ultra‑critical actions (direct user edits) if desired.
- Pros: Minimal surgery; naturally coalesces storms; easy to tune N per frame; integrates with `inflight_rev` gating.
- Cons: Still FIFO inside Runtime; no preemption once a job is running.

2) Runtime‑Level Priority Dispatcher
- Idea: Replace the FIFO MPSC with a priority heap (or multiple queues) inside Runtime. Dispatcher always picks highest priority job next.
- Shapes:
  - Single priority heap guarded by a mutex + condvar; workers pull from it.
  - Two (or three) queues: high/normal/low. Dispatch to any idle worker, always drain high first.
- Pros: True prioritization across all producers; less pressure on the App to sequence perfectly.
- Cons: Larger refactor (queue structure, locking); must still coalesce at the App or risk flooding Runtime with redundant items.

3) Capacity Reservation (lane splitting)
- Idea: Dedicate a subset of worker threads to high‑priority jobs (edits/near‑camera), with the remainder handling background streaming.
- Variants:
  - Static: e.g., 2 workers reserved high‑prio; others for background.
  - Dynamic: adapt reserved count based on observed high‑prio backlog.
- Pros: Simple mental model; bounded latency for important work.
- Cons: Risk of underutilization if no high‑prio work exists; requires dual queues.

4) Rate‑Limit Background Work (budgeting)
- Idea: Keep current queuing but enforce budgets on low‑priority producers:
  - StreamLoad: at most N submissions per frame (walk a ring around camera progressively).
  - Worldgen hot‑reload: rebuild N per frame (closest first) or manual trigger.
- Pros: Easiest to layer in; reduces the chance of long backlogs.
- Cons: Doesn’t solve head‑of‑line blocking if background items are already inflight; still FIFO among queued jobs.

5) Soft Preemption via Backpressure
- Idea: Avoid sending low‑prio jobs to Runtime if high‑prio demand is present and worker backlog is above a threshold.
- Pros: Keeps the queue lean and focused on the important work.
- Cons: Requires visibility into current backlog; incomplete if other sources already filled the queue.

Recommended Path (Incremental, Low Risk)
Step A: App‑Level Coalescing + Priority Submit
- Buffer `ChunkRebuildRequested`/`EnsureChunkLoaded` intents per frame in a map keyed by `(cx,cz)` with fields `{rev, cause, last_requested_tick}`.
- At the end of `step()`, compute priority and submit up to K:
  - cause_score: Edit=0, LightingBorder=1, StreamLoad=2, HotReload=3
  - distance_bucket: 0 (in‑frustum and within R1), 1 (within R2), 2 (others)
  - age_bonus: subtract 1 if waiting > T ticks to avoid starvation
  - tie‑break: smaller distance, newer rev
- Before submit, apply `inflight_rev` gating:
  - If `inflight_rev[(cx,cz)] >= rev`, skip (already sent or a newer is inflight).
  - Else set `inflight_rev[(cx,cz)] = rev` and submit job.

Step B: Rate‑Limit Background Sources
- StreamLoad: Trickle new rings (or N chunks per frame) toward view radius; prioritize closest ring not yet loaded.
- Worldgen hot‑reload: N per frame or manual hotkey; log remaining count.

Step C: Capacity Reservation (optional short hop)
- Reserve 1–2 workers for high‑prio only; they pull from a small high‑prio queue populated by the App coalescer.
- Remaining workers consume normal queue.

Step D: Runtime Priority (longer term)
- Move the priority heap into Runtime and make `submit_build_job` accept a priority key. Replace FIFO dispatcher with a priority‑aware one. Keep App‑side coalescing.

Edge Cases and Consistency
- Neighbor occlusion: Building a chunk without neighbors might temporarily render more faces; later neighbor builds will reduce them. Correctness is unaffected; only temporary overdraw.
- Lighting borders: Prioritize near‑camera border updates so light continuity looks good where the player can see; far borders can defer without logical inconsistencies.
- Edits during heavy streaming: Edits should always come first (cause score = 0). With `inflight_rev` gating, repeated edits to the same chunk naturally coalesce to the latest rev.

Scheduling Examples

Priority key (lower is higher):
  P = (cause_score, distance_bucket, starvation_boost)
where:
  cause_score ∈ {0=Edit, 1=Light, 2=StreamLoad, 3=HotReload}
  distance_bucket ∈ {0=in frustum and d<=R1, 1=d<=R2, 2=else}
  starvation_boost ∈ {0 normally, -1 if age>T, -2 if age>2T}

Coalescer state per chunk (pseudo):
```
struct Intent { rev: u64, cause: Cause, last_tick: u64 }
intents: HashMap<(i32,i32), Intent>

fn record_intent(cx,cz, cause) {
  let rev = edits.get_rev(cx,cz)
  let e = intents.entry((cx,cz)).or_insert(Intent{rev, cause, last_tick: now})
  if rev > e.rev { e.rev = rev }
  if cause > e.cause { /* keep strongest (Edit strongest) */ } else { e.cause = cause }
  e.last_tick = now
}

fn flush() {
  // build list, compute priorities, sort ascending, take K
  for each (cx,cz,intent) by priority {
    if inflight_rev[(cx,cz)] >= intent.rev { continue }
    if not visible and cause >= StreamLoad and too_far { continue }
    inflight_rev[(cx,cz)] = intent.rev
    submit_build_job(...)
  }
  intents.clear()
}
```

Metrics to Watch
- Edit latency: time from BlockPlaced/Removed to accepted BuildChunkJobCompleted.
- High‑prio backlog size and age (edits, lighting) vs. low‑prio backlog.
- Worker utilization split across priority classes.
- Starvation counter for background items (max/avg wait time).

Open Questions
- How aggressively to age background tasks? (Prevent permanent starvation without hurting edit latency.)
- Should we allow a trickle of background jobs every frame (e.g., at least 1) regardless of high‑prio load?
- Do we want a separate class for “near‑camera but off‑frustum” (e.g., just behind the player) to smooth rapid turns?

Bottom Line
- The safest and most impactful first step is App‑side coalescing plus priority submit with a per‑frame cap, combined with rate limiting for background sources. This preserves correctness (thanks to revision checks) and materially improves the responsiveness of edits/lighting near the player. Capacity reservation and a Runtime‑level priority queue are natural follow‑ups if we need more control.


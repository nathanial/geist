Edit Prioritization: Options, Trade‑offs, and How To Implement

Context
- Current state: App coalesces intents and prioritizes by cause → distance ring → age. This governs submission order only. Runtime still dispatches jobs FIFO per worker.
- Problem: After sprinting (large StreamLoad backlog), a new Edit job can sit behind already enqueued work on workers. Priority at submission time cannot “skip” work already queued.
- Constraints: Keep world correctness (revision checks on completion), avoid starvation, preserve responsiveness near the player.

Objectives
- Minimize edit‑to‑accept latency (p95) even under heavy streaming.
- Ensure lighting/edit rebuilds near the player outrank background streaming.
- Avoid wasted CPU/GPU work on out‑of‑radius chunks.

Key Causes of Delay
- FIFO worker queues: Jobs submitted earlier (StreamLoad) block later high‑prio edits.
- “Loaded” prerequisite: Edits on not‑yet‑loaded chunks require a high‑prio load first; if that load is queued behind others, the edit waits.
- Lighting fan‑out: LightBordersUpdated can burst many rebuilds; already queued jobs run first.

Safety/Correctness
- Reordering submission/execution is safe: completion handlers accept only latest rev; lighting borders are idempotent. Changes affect latency, not final state.

Option A — App‑Only Tweaks (low risk, incremental)
- Description: Keep Runtime as FIFO, improve App scheduling behavior.
- Tactics:
  - Immediate edit submit: bypass per‑frame cap for Edit intents; submit ASAP (still suffers from worker FIFO backlog).
  - Separate caps by class: e.g., allow up to M edits + L lighting + N stream loads per frame.
  - High‑prio load for edited chunks: when edit targets unloaded chunk, record a high‑prio StreamLoad intent for that chunk (cause=EditLoad) so load + subsequent edit get ranked ahead of normal StreamLoad.
  - Continue pruning and “still desired” gating (already implemented) to limit background noise.
- Pros: Minimal code churn; low risk.
- Cons: Can’t preempt already queued work; worst‑case edit latency still poor under deep backlogs.
- Implementation:
  - App only (src/app.rs), extend cause taxonomy and cap logic; send edit‑related loads immediately.

Option B — Dual Queues + Reserved Workers (medium complexity)
- Description: Add a small high‑priority lane serviced by dedicated workers; background lane for StreamLoad.
- Design:
  - Two queues in Runtime: `job_tx_hi` (edits + lighting + edit‑related loads) and `job_tx_bg` (StreamLoad, hot‑reload).
  - Spawn `W_hi` reserved workers draining high‑prio queue only; `W_bg` workers drain background queue.
  - Dispatcher: App routes BuildJob by cause to appropriate queue.
  - Backpressure: If high‑prio queue is empty, reserved workers may opportunistically pull from background (optional) or sleep.
  - Fairness: Keep App caps; high‑prio not capped or lightly capped.
- Pros: Edits truly bypass StreamLoad backlog without a full priority heap; predictable latency.
- Cons: Capacity partitioning risk (idle high‑prio workers if no edits; or insufficient high‑prio capacity during storms).
- Implementation notes:
  - src/runtime.rs: add two channels (mpsc), spin `W_hi` and `W_bg` loops reading different receivers.
  - src/app.rs: tag BuildJob cause; route to `submit_build_job_hi` or `_bg`.
  - Consider making `W_hi = max(1, N/4)` and clamp.

Option C — Central Priority Queue in Runtime (higher complexity, best control)
- Description: Replace per‑worker channels + dispatcher with a single synchronized priority queue (binary heap) from which workers pop highest‑priority jobs.
- Design:
  - Shared `Arc<(Mutex<BinaryHeap<Job>>, Condvar)>` or `parking_lot` equivalents.
  - Job carries priority key: `(cause, distance_ring, -age, tie_breaker)`.
  - Workers: loop `pop_highest()`, block on condvar if empty.
  - Ingest path: `submit_build_job(job)` pushes with priority and notifies condvar.
  - Dedup/cancel: Maintain a `HashMap<(cx,cz), JobMeta>`; on enqueue, if a job for same chunk exists and is not yet started, replace with newer rev (drop older).
- Pros: Edits and near jobs always execute first; can coalesce/cancel redundant background jobs; maximum responsiveness.
- Cons: More refactor, locking contention possible under very high throughput, careful design needed to avoid priority inversion/starvation.
- Implementation notes:
  - src/runtime.rs: remove per‑worker queues; write central queue + worker loops.
  - Job struct gains `kind: JobKind { Edit, Light, StreamLoad, HotReload, EditLoad }`, `priority: u64` or tuple.
  - App can still do coalescing; Runtime should also dedup late (last line of defense).
  - Fairness: add aging so background jobs eventually climb priority.

Option D — Hybrid: Central Queue + High‑Prio Reservation (robust, moderate)
- Description: Central priority queue plus a small guaranteed share for high‑prio (or min share for background).
- Design:
  - Workers can be identical; reservation enforced by queue policy (e.g., every Kth pop must be background if background backlog > threshold).
- Pros: Combines best of both worlds; avoids pathological starvation; edits stay snappy.
- Cons: Slightly more complex queue policy.

Option E — Background Job Sizing / Throttling (complementary)
- Description: Make background work smaller/cheaper so it doesn’t block as long.
- Ideas:
  - StreamLoad LOD: far chunks build simplified meshes; refine when closer (less CPU per job).
  - Rate limits: keep or reduce StreamLoad cap; maintain a trickle.
  - Cancel out‑of‑radius jobs before they start: requires central queue to drop queued items when no longer desired.
- Pros: Improves perceived responsiveness without invasive changes.
- Cons: Doesn’t guarantee preemption; best paired with B/C.

Recommended Path (pragmatic, phased)
- Phase 1 (quick win): Dual queues with reserved workers
  - Route Edit/Light/EditLoad to high‑prio queue; StreamLoad/HotReload to background.
  - Reserve `W_hi = 1..2` threads (configurable); allow opportunistic pull from bg when high‑prio empty.
  - Keep App coalescing and gating (already in place) to reduce waste.
  - Metrics: edit latency p50/p95, high‑prio queue depth, bg queue depth, worker utilization split.

- Phase 2 (robust): Central priority queue
  - Replace channels with heap + condvar; add dedup map to coalesce queued jobs per chunk.
  - Keep App coalescing, but don’t rely on it exclusively.
  - Add simple fairness aging to avoid bg starvation.

- Phase 3 (optional): Hybrid reservation
  - Enforce a small min share for background or max share for high‑prio to avoid extremes.

Implementation Sketches
- Dual queues (Runtime):
  - Add in Runtime:
    - `job_tx_hi: mpsc::Sender<BuildJob>` / `job_rx_hi: mpsc::Receiver<BuildJob>`
    - `job_tx_bg: mpsc::Sender<BuildJob>` / `job_rx_bg: mpsc::Receiver<BuildJob>`
    - Spawn `W_hi` workers reading `job_rx_hi` only; `W_bg` reading `job_rx_bg`.
  - API:
    - `submit_build_job_hi(job)` and `submit_build_job_bg(job)`.
  - App routing:
    - Map `RebuildCause::Edit` → hi, `LightingBorder` → hi, `StreamLoad` → bg; add `EditLoad` for unloaded edits → hi.

- Central priority queue (Runtime):
  - Data:
    - `struct PrioQueue { heap: BinaryHeap<PJob>, by_chunk: HashMap<(i32,i32), PJobRef> }`
    - `PJob { key: (cause, ring, age_boost, tie), job: BuildJob }`
  - Worker loop:
    - Acquire lock → pop highest → release → execute → repeat.
  - Submit path:
    - Lock → if `by_chunk` has entry and not started: replace with newer rev; else push new; signal condvar.
  - Cancellation:
    - On ViewCenterChanged or unload: lock → remove entries for chunks outside desired.
  - Fairness:
    - Age increment per tick (or compute on the fly from enqueue time), add to priority key.

Measurement & Telemetry
- Edit latency: emit timestamps at intent record, job submit, worker start (optional), completion accept.
- Queue depths: hi/bg (or central heap size) per frame.
- Dropped/canceled jobs: count how many StreamLoad were pruned or completions dropped due to out‑of‑radius.
- Worker utilization: % time on hi vs bg; average job time by cause.

Risks & Mitigations
- Lock contention in central queue: keep critical sections small; use BinaryHeap + simple keys; avoid heavy logging while locked.
- Starvation: add aging and/or reservation.
- Complexity creep: implement in phases; keep App coalescing to reduce queue volume.

Decision Checklist
- Do we need true “skip the line” behavior (edits must preempt already queued work)? If yes → Dual queues or Central queue.
- Are we comfortable with reserved capacity trade‑offs (possible underutilization)? If yes → Dual queues (fastest path).
- Do we want dedup/cancellation in Runtime to reduce waste? If yes → Central queue (or hybrid).

Bottom Line
- The App‑side priority improved submission order but cannot preempt already queued work. To make edits reliably fast under load, we should introduce Runtime‑level prioritization. Start with dual queues + reserved workers (quick, impactful), then move to a central priority queue with dedup/cancellation for full control if needed.


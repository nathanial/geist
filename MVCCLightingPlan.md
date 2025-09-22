# MVCC Lighting Plan

## Motivation

Incremental lighting currently relies on asynchronous, best-effort propagation. Races between geometry edits, seam publication, and worker scheduling allow stale skylight to linger. A multi-version concurrency control (MVCC) lighting pipeline eliminates these races by treating lighting updates as transactions that commit atomically, guaranteeing readers always see a coherent snapshot.

## Core Principles

1. **Transactional edits** – Each lighting-relevant mutation (player edit, structure placement, worldgen change) becomes an `EditTxn` targeting a bounded set of chunks.
2. **Immutable snapshots** – Lighting workers operate on frozen copies of geometry, seam inputs, and prior light fields so work cannot observe in-flight changes.
3. **Optimistic validation** – Workers run without locks; the commit phase checks for conflicting updates and retries when necessary.
4. **Atomic commits** – Chunks only adopt lighting results when every affected face passes validation, ensuring no partial state leaks to the world.
5. **Versioned reads** – Consumers (rendering, AI, gameplay) read by `(chunk_id, rev)`; they either see pre-edit or post-edit lighting, never a mix.

## System Architecture

### Transaction Coordinator
- Collects block edits into `EditTxn` structs keyed by chunk coordinates.
- Assigns monotonically increasing `txn_id` and schedules work over a serialized intent queue.
- Applies conflict policy: merge compatible edits, split large diffs, or serialize high-contention areas.

### Chunk Registry
- Maintains per-chunk `geometry_rev`, `lighting_rev`, and the `committed_snapshot` pointer.
- Tracks outstanding `txn_id`s touching the chunk to detect conflicts at commit time.

### Worker Pipeline
1. Coordinator snapshots target chunks: block data, seam planes, and neighbor metadata are copied into immutable buffers tagged with `snapshot_rev`.
2. Snapshot plus edit diff travels to a worker thread that reruns lighting (skylight, block light, structures) in isolation.
3. Worker produces new light grids, seam planes, and an audit trail (affected voxels, metrics) tied to `txn_id`.

### Commit Phase
- Acquire chunk locks in deterministic order (e.g., Morton-sorted) to avoid deadlock.
- Validate invariants: current `geometry_rev`/`lighting_rev` must match the snapshot values recorded before work began and no newer txn has committed overlapping faces.
- If validation passes, apply block edits, swap in new lighting/seams, bump rev counters, and release locks.
- If validation fails, drop the attempt, enqueue a retry with the latest snapshot, and optionally backoff to prevent thrash.

### Seam Store
- Stores seam planes alongside the `txn_id` and `lighting_rev` that produced them.
- Readers specify desired `lighting_rev`; store returns either the exact match or indicates the data is outdated, prompting them to wait or subscribe to commit events.

### Read Path
- Gameplay systems request light data via `LightSnapshotHandle { chunk, lighting_rev }`.
- Handles resolve to atomically consistent data; stale handles invalidate automatically when revs advance, prompting a fresh query.
- Rendering threads can double-buffer handles to avoid stalls.

## Engine Integration

### Edit Sources
- Player/world edits push directly into the coordinator via `begin_edit(chunk_set) -> EditTxnHandle`.
- Structure builders batch placements into a single txn to guarantee cohesive lighting.
- World streaming prefetches lighting by scheduling txns when loading new regions; unload waits until outstanding txns finish.

### Undo/Redo
- Because commits are atomic, undo is modeled as a compensating txn that restores previous blocks/light, producing deterministic replay.

### Persistence
- Save games store `lighting_rev` per chunk and the most recent seam data. Outstanding transactions serialize as journals so we can replay them on load.

## Observability & Tooling

- Metrics: txn latency, retry rate, conflict hot spots, commit throughput, retry backoff distribution.
- Logs: conflict causes (geometry changed, overlapping txn, chunk unloaded mid-flight).
- Debug UI: per-chunk state showing `latest_commit_rev`, pending txns, retry counters.
- CLI: `lighting txn-status --chunk x y z` to inspect history and pending work.

## Testing Strategy

1. **Unit tests** for coordinator merging, conflict detection, and commit validation logic.
2. **Simulation tests** that issue randomized edit storms across chunk seams and assert no reader surfaces mixed lighting states.
3. **Latency benchmarks** to measure txn completion under varying contention (single-player build vs. large structure import).
4. **Rollback tests** forcing worker failures to confirm retries converge and no partial lighting escapes.

## Performance Considerations

- **Snapshot cost**: minimize copies via copy-on-write chunk storage or arena-backed voxel representations.
- **Lock contention**: partition world into lock domains; large txns acquire coarse locks, small edits stay fine-grained.
- **Throughput**: allow multiple non-overlapping txns to commit concurrently; coordinator uses spatial indexing to detect disjoint sets.
- **Memory**: maintain bounded snapshot pools with LRU eviction; throttle txn queue when memory pressure rises.

## Pros & Cons

**Pros**
- Strong consistency; stale lighting cannot persist past a commit boundary.
- Deterministic audit trail for debugging and regression reproduction.
- Natural foundation for undo/redo and editor tooling.
- Easier reasoning about invariants; lighting is either pre- or post-edit.

**Cons**
- Added latency when conflicts force retries, especially in high-activity areas.
- Increased memory footprint from snapshots and per-txn metadata.
- Coordinator and lock manager represent new complex systems that must be highly reliable.
- Integration requires touching most runtime subsystems (scheduling, rendering, persistence).

## Risk Mitigation

- Start with single-chunk txns to validate plumbing before supporting multi-chunk edits.
- Feature flag gate; allow fallback to current incremental pipeline during rollout.
- Implement watchdog timers and exponential backoff to prevent livelock under contention.
- Provide extensive tracing to diagnose slow or thrashing txns quickly.

## Rollout Plan

1. **Prototype** coordinator & single-chunk txns in a branch; run headless regression harness.
2. **Incremental adoption**: enable MVCC for editor/build tools first, keep live gameplay on legacy path.
3. **Hybrid mode**: allow both pipelines, with MVCC authoritative for interior chunks while surface skylight still uses legacy propagation.
4. **Full cutover** once contention metrics and soak tests meet targets; remove legacy queueing after stabilization.

## Open Questions

- What lock granularity balances throughput vs. complexity (chunk, column, micro-plane)?
- Can we leverage GPU compute for snapshot bakes without breaking MVCC guarantees?
- How to prioritize txns under constant stream of edits (e.g., griefing scenarios) without starving low-priority maintenance jobs?

## Conclusion

An MVCC-driven lighting pipeline trades algorithmic complexity for airtight consistency. By elevating lighting to a transactional system with immutable snapshots and atomic commits, we eliminate seam races, enable tooling-friendly workflows, and gain deterministic behavior. The cost is additional latency, memory, and engineering effort, but for worlds where visual correctness is paramount, the approach offers a path to truly bulletproof lighting.

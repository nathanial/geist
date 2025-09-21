# Bulletproof Lighting Plan

## Objectives

1. **Deterministic darkness** – sealing any space must remove skylight within one simulation tick, regardless of chunk seams or worker timing.
2. **Seam integrity** – neighbor lighting can only consume seam data that reflects the latest geometry.
3. **Idempotent processing** – edits issued in rapid succession cannot strand stale light; jobs may run twice but never zero times.
4. **Observability** – we need metrics, logs, and tests that fail loudly when lighting drifts.

## Why We Still Fail Today

- The chunk that owns a seam publishes its micro planes *after* lower chunks already rebuilt, so they hold onto stale skylight.
- Runtime tracks just a single “inflight job” flag per chunk. When the surface finishes and tries to trigger the cave, the scheduler drops the request because it thinks work is already pending.
- Seam planes carry no revision metadata. Consumers cannot tell whether they integrated the latest update.
- Tests exercise lighting in isolation but never push the full edit → rebuild → notification loop across chunk boundaries.

## Strategy Overview

We refactor the lighting pipeline around **versioned state**, **explicit dependencies**, and **reliable scheduling**.

1. **Version Everything**
   - Introduce `geometry_rev` (incremented whenever blocks change) and `lighting_rev` (incremented whenever a chunk’s lighting job finishes).
   - Seam planes and light grids store the producing `lighting_rev`.
   - Neighbor chunks record the highest rev they have consumed per face.

2. **Dependency-Aware Scheduler**
   - When chunk A finishes lighting at rev *n*, it emits `LightingUpdated { chunk: A, rev: n }`.
   - Runtime enqueues each neighbor with intent `LightingDependency { source: A, required_rev: n }`.
   - The scheduler lane for lighting dependencies is FIFO, never distance-gated, and deduplicates by `(chunk, required_rev)`.

3. **Result Filtering**
   - Lighting workers return `(geometry_rev_seen, lighting_rev_produced, seam_revs)`.
   - Runtime accepts the result only if `geometry_rev_seen` equals the current chunk `geometry_rev`.
   - If a lighting job finishes too early (because geometry changed afterward), it is discarded and a new job is queued.

4. **Atomic Seam Updates**
   - `LightingStore::update_micro_borders` stores `(plane_data, lighting_rev)`.
   - `plane_changed` becomes `old.rev != new.rev || old.data != new.data`.
   - Neighbor queries expose both bytes and rev so consumers can confirm freshness.

5. **Observability**
   - Metrics: seam updates per tick, dependency queue depth, stale lighting discards, time delta between seam update and neighbor rebuild.
   - Logs: warnings when a chunk samples seam revs older than its current geometry.
   - Debug overlay: show `(geometry_rev, lighting_rev, last_dependency_rev)` per chunk.

## Refactor Phases

### Phase 0 – Plumbing

1. Add rev fields to chunk state, lighting store entries, job structs.
2. Update serialization/load paths to initialize revs (start at 0).

### Phase 1 – Worker Contract

1. Extend `BuildJob` with `geometry_rev` snapshot and neighbor seam revs.
2. Lighting worker copies seam revs into result and increments `lighting_rev` atomically on success.
3. Runtime discards results where `job.geometry_rev` != `chunk.geometry_rev`.

### Phase 2 – Seam Storage

1. Switch micro border maps to `(Arc<[u8]>, u64 rev)` tuples.
2. Provide helper `neighbor_micro_borders_with_rev` returning slices + revs.

### Phase 3 – Scheduler Overhaul

1. Introduce `IntentCause::LightingDependency` with high priority and own queue limit.
2. On `LightingUpdated`, enqueue all six neighbors with `(required_rev = lighting_rev)`.
3. When scheduling a chunk, merge dependency intents: keep the max `required_rev` and mark older queued jobs obsolete.
4. Allow lighting jobs even if an edit job is in flight; runtime reconciles by comparing revs.

### Phase 4 – Integration Tests

1. Build a headless harness `tests/integration/skylight_seal.rs`:
   - Load stacked chunks.
   - Open/close skylight 100 times.
   - Wait for dependency queue to drain each time.
   - Assert interior light always matches expected state.
2. Add randomized multi-chunk edit storm test to detect cross-seam races.

### Phase 5 – Observability

1. Emit metrics via existing debug HUD and tracing exporters.
2. Log stale result discards with chunk coord, old/new revs.
3. Add CLI command (or hotkey overlay) to display dependency backlog.

### Phase 6 – Cleanup

1. Remove legacy “skip vertical neighbor” logic entirely.
2. Document invariants in `docs/lighting.md`.
3. Provide migration notes for structure lighting to adopt the same dependency pipeline.

## Risk Mitigation

- Keep a feature flag (`lighting_revamp`) to toggle new pipeline on/off.
- Maintain old scheduling as fallback until integration tests pass in CI and staging.
- Use nightly soak tests to monitor metrics before enabling by default.

## Open Questions

1. Should we store revs per face or per chunk? Proposed: per face for accuracy.
2. Do we need back-pressure on dependency queue? Proposed: cap length but never drop; instead, defer edit processing if dependency queue is over budget.
3. How to handle worldgen hot reload resetting geometry? Proposed: bump `geometry_rev` globally.

## Conclusion

Fixing the skylight regression long-term means treating lighting as a versioned, dependency-driven pipeline rather than a best-effort queue. Once the runtime enforces “no chunk consumes stale seam data” and lighting jobs are idempotent, the race that keeps caves lit disappears. The refactor is sizable, but it gives us deterministic, testable lighting behavior that future content updates can rely on.


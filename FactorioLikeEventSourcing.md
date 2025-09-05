# Factorio‑Like Event Sourcing Plan for Geist

This document proposes an event‑sourced, tick‑driven architecture for Geist’s voxel world. The goal is to mediate all gameplay logic through a deterministic event queue, enabling replay, debugging, and consistent behavior across runs while still fanning work out to worker threads.

## Goals & Principles

- Deterministic core loop: A single authoritative queue of events processed by tick in sequence. No direct state mutation outside event handlers.
- Time as ticks: All time advances by integer ticks. Any “do later” logic enqueues events for future ticks.
- Event‑only mutations: Systems do not mutate state directly; they publish events and handle events.
- Reproducible & debuggable: Ability to record, replay, and bisect logic via the event log and checkpoints.
- Parallel but deterministic: Heavy work fans out to workers, but the main thread applies results deterministically.
- Minimal invasive migration: Phase in the queue while preserving current behavior (chunk loading, editing, meshing, lighting, player movement).

Non‑goals (for this cutover)
- Networking/multiplayer and rollback netcode.
- Persisted event log/snapshots (can be added later without changing core loop).

## Snapshot of Current Code (as‑is)

- Main loop (`src/main.rs`) owns:
  - Input polling (camera/walker, mouse, hotkeys).
  - Streaming set: `loaded` meshes, `loaded_bufs` chunk buffers, `pending` jobs; neighbor checks.
  - Worker threads: `BuildJob` generation and `JobOut` intake for meshing.
  - Edits: `edit::EditStore` with per‑chunk rev tracking; triggers rebuilds.
  - Lighting: `lighting::LightingStore` emitters + neighbor borders; light computed inside meshing.
- Meshing (`src/mesher.rs`): Greedy mesh + per‑face light sampling using `LightGrid::compute_with_borders_buf` + `LightingStore` borders. Emits updated border planes back to store.
- Lighting (`src/lighting.rs`): Emitter/bookkeeping, neighbor border planes, and lightgrid computation invoked by mesher.
- World (`src/voxel.rs`): Procedural world function `block_at` (deterministic given seed).
- Player (`src/player.rs`): Walker physics using a sampler over loaded chunk buffers (and ignores unloaded regions).

This is already close to a data‑flow with revision checks, but today the main loop and systems mutate state directly. We’ll route these via events.

## Core Architecture

1) Event types: A domain `Event` enum carrying structured payloads. All state transitions happen via handling one event at a time.

2) Event envelope: `EventEnvelope { id, tick, kind, source, causal }` with:
   - `id`: monotonically increasing or UUID (plus stable `job_id` for worker work).
   - `tick`: scheduled tick to process.
   - `source`: which code path produced it.
   - `causal`: optional parent event id for tracing.

3) Scheduler: A priority queue (min‑heap by `tick`) of events; per‑tick FIFO for same‑tick ordering. If no events at current tick, sleep or fast‑forward to next event.

4) One authoritative GameState: A single struct holding all logical state and derived caches. Event handlers are the only code allowed to mutate it.

5) Systems as functions: “Systems” become modules with handler functions `fn handle(gs: &mut GameState, ev: &EventEnvelope, ctx: &mut Ctx)` that mutate only `GameState` and emit new events via `ctx.emit(...)`.

6) Worker fan‑out: A `Meshing` handler emits `BuildChunkJobRequested` to workers via a side‑effect `Runtime`. Results are reintroduced as events (`BuildChunkJobCompleted`) in deterministic order (sorted by `job_id`) before modifying `GameState`.

7) Persistence (later phase): Append‑only event log with periodic snapshots of `GameState`. Replaying events reconstructs state.

8) Bootstrap: On start, enqueue spawn/setup events (player spawn, initial `ViewCenterChanged`) and `EnsureChunkLoaded` for the initial view radius.

9) Separation of concerns: `GameState` is fully serializable and independent of Raylib. `Runtime` holds OS/graphics/thread resources only.

## Event Model (MVP)

Event enum sketch (Rust):

```
enum Event {
  // Time & housekeeping
  Tick,                             // internal sentinel; optional

  // Input → intent
  InputCaptured { actions: Vec<InputAction> },
  MovementRequested { entity: EntityId, yaw: f32, dt_ms: u32 },

  // Player world interactions
  RaycastEditRequested { place: bool, block: Block, max_dist: f32 },
  BlockEditApplied { wx: i32, wy: i32, wz: i32, new: Block, old: Block },

  // Lighting
  LightEmitterAdded { wx: i32, wy: i32, wz: i32, level: u8, is_beacon: bool },
  LightEmitterRemoved { wx: i32, wy: i32, wz: i32 },
  LightBordersUpdated { cx: i32, cz: i32 },

  // Streaming & meshing
  ViewCenterChanged { ccx: i32, ccz: i32 },
  EnsureChunkLoaded { cx: i32, cz: i32 },          // enqueue load+mesh if missing
  EnsureChunkUnloaded { cx: i32, cz: i32 },        // drop GPU + buffers
  BuildChunkJobRequested { cx: i32, cz: i32, rev: u64, neighbors: NeighborsLoaded, job_id: u64 },
  BuildChunkJobCompleted { job_id: u64, cx: i32, cz: i32, rev: u64, cpu: ChunkMeshCPU, buf: ChunkBuf, borders_changed: bool },

  // Rebuild scheduling
  ChunkRebuildRequested { cx: i32, cz: i32, cause: RebuildCause },
}
```

Notes:
- Input actions get normalized into MovementRequested / RaycastEditRequested events for the next tick.
- Block edits always emit BlockEditApplied after validating and performing the change in the GameState’s edit overlay; affected chunks then translate into ChunkRebuildRequested events.
- Worker requests get a stable `job_id` (e.g., hash of `(cx,cz,rev,neighbors)`), allowing deterministic intake ordering.

## Tick & Queue Semantics

- Tick rate: Keep 60 ticks/s (aligned to current `set_target_fps(60)`). One logical tick may process multiple events until budget is exhausted; excess events roll to next tick.
- Deterministic order: Within a tick, process events in FIFO order by enqueue time; when bridging from threads, buffer all “completed” job results for that tick and emit `BuildChunkJobCompleted` events sorted by `job_id`.
- Future scheduling: Any delayed behaviors use `tick + delta` scheduling.
- Idle: If the queue has no events at current tick, either sleep until next vsync or advance to the next event’s scheduled tick.

Fixed‑timestep recommendation
- Use a fixed accumulator: accumulate real time; while `accum >= tick_dt`, run one tick and subtract `tick_dt`. Render every frame using the latest `GameState`.
- Set `tick_dt = 1/60s`. If we get > 5 ticks behind, clamp to avoid spiral of death and process at most `MAX_TICKS_PER_FRAME` (e.g., 2–3), then carry over the rest.

Per‑tick processing budget
- Configure `MAX_EVENTS_PER_TICK` (e.g., 10_000) and `MAX_TICK_TIME_MS` (e.g., 4 ms). If exceeded, spill the remaining events to next tick to maintain frame pacing.

## Systems and Responsibilities

All handlers mutate only `GameState`.

1) Input
   - Consumes OS input and emits `InputCaptured` with a canonical list.
   - Translates to `MovementRequested` and `RaycastEditRequested` for tick N+1.

2) Player
   - On `MovementRequested`: updates `GameState.player` via walker physics using a sampler that reads `GameState.chunks` + `GameState.edits`.
   - Emits `ViewCenterChanged` if center chunk changes.

3) Streaming
   - On `ViewCenterChanged`: diff desired set vs `GameState.streaming.loaded`; emit `EnsureChunkLoaded` / `EnsureChunkUnloaded`.
   - On `EnsureChunkLoaded`: if already loaded or pending do nothing; else compute `NeighborsLoaded`, read current `rev`, emit `BuildChunkJobRequested`.
   - On `EnsureChunkUnloaded`: remove `ChunkState` and GPU model refs from `GameState`.

4) Edit
   - On `RaycastEditRequested`: raycast (edits > loaded_bufs > world); emit `BlockEditApplied`.
   - On `BlockEditApplied`:
     - Update `GameState.edits` overlay and revision maps.
     - If emission changed, emit `LightEmitterAdded` or `LightEmitterRemoved`.
     - Emit `ChunkRebuildRequested` for the edited chunk.

5) Lighting
   - On `LightEmitterAdded/Removed`: update `GameState.lighting.emitters` map.
   - On `LightBordersUpdated`: if borders changed and neighbors are loaded, emit neighbor `ChunkRebuildRequested`.

6) Meshing
   - On `ChunkRebuildRequested`: compute `NeighborsLoaded`, read `rev`, emit `BuildChunkJobRequested` (dedupe by `job_id`).
   - On `BuildChunkJobRequested`: post work to worker chosen by stable strategy (`job_id % N`) via `Runtime`.
   - Intake worker results from `Runtime` into a main‑thread buffer; at end of tick, convert to `BuildChunkJobCompleted` events sorted by `job_id`.
   - On `BuildChunkJobCompleted`: drop if stale; else update `GameState.chunks[cx,cz]` (buf, built_rev), update GPU model refs, mark `pending=false`, and emit `LightBordersUpdated` when borders changed.

7) Render
   - Uses read‑only `GameState` to draw via `Runtime` (shaders/textures live in `Runtime`, not in the authoritative state).

## Deterministic Multithreading

- Stable job ids: `job_id = hash(cx, cz, rev, neighbors_mask)`; any re‑enqueue for the same inputs yields the same id.
- Stable worker assignment: `worker_index = job_id % N` to reduce nondeterministic contention.
- Deterministic acceptance: Within each tick, sort completed results by `job_id` before emitting `BuildChunkJobCompleted`. Handlers must be idempotent and revision‑checked, so arrival order cannot change state.
- No data races: Worker threads never mutate `GameState`; they only return pure results (CPU mesh + buf + flags).

Stable hashing
- Implement a local 64‑bit FNV‑1a or xxhash with a fixed seed for `job_id` to avoid runtime‑randomized hashers.

Cross‑thread ingress
- Runtime collects worker `JobOut` results in a `crossbeam::SegQueue`/channel. The main thread drains this queue only at the boundary between processing and rendering and turns them into sorted completion events.

## Light Propagation Strategy

- Keep light propagation as an implicit computation inside `LightGrid::compute_with_borders_buf` for now.
- Keep emitter and border planes inside `GameState.lighting` and update them via events.
- When a chunk’s light borders change, emit neighbor rebuild requests as events.

Optional alternative (later)
- Promote light propagation to explicit events (finer control and pacing), but this is not required for determinism given current implementation.

## Persistence & Replay (Phase 3+)

- Event log: Append events with stable serialization format.
- Snapshots: Periodically serialize stores (`StreamingStore`, `EditStore`, `LightingStore`, `PlayerStore`) for fast load.
- Deterministic replay: Rebuild exact state from snapshots + event log. Render side effects can be gated or disabled in replay mode for speed.

## Cutover Plan (Big‑Bang)

Commit to the event‑sourced loop with a single authoritative `GameState`. Remove legacy mutation paths during the same branch.

1) Core scaffolding
- Add `Event`, `EventEnvelope`, `EventQueue`, `EventScheduler`.
- Define `GameState` (chunks, edits, lighting, revisions, player, jobs) and `Runtime` (threads, raylib handles, textures, channels).
- Add `App` that owns `GameState`, queue, and `Runtime` with `app.tick()` and `app.render()`.

2) Deterministic worker infra
- Move worker threads/channels into `Runtime`.
- Introduce stable `job_id = hash(cx,cz,rev,neighbors)` and stable worker assignment `job_id % N`.
- Buffer worker results per frame; at end of tick, sort by `job_id` and emit `BuildChunkJobCompleted` events.

3) Event handlers (single pass)
- Input → `InputCaptured` → `MovementRequested` / `RaycastEditRequested`.
- Player → updates `GameState.player`, emits `ViewCenterChanged` on center change.
- Streaming → `EnsureChunkLoaded/Unloaded`, `BuildChunkJobRequested`.
- Edit → `BlockEditApplied`, bumps revs, emits light add/remove and `ChunkRebuildRequested`.
- Lighting → manage emitters and react to `LightBordersUpdated` by queuing neighbor rebuilds.
- Meshing → request jobs, ingest completions, update `GameState` chunk entries (buf, built_rev, gpu handle id), emit `LightBordersUpdated`.

4) Main loop rewrite
- Replace direct mutations in `src/main.rs` with: poll input, enqueue input events, process the event queue for current tick, flush worker completions to events, render from `GameState`.

5) Remove legacy code paths
- Delete/inline any direct writes to `loaded`, `loaded_bufs`, `pending`, and direct `LightingStore`/`EditStore` mutations outside handlers.
- Keep mesher/lighting computation modules; they remain pure helpers called by handlers/workers.

6) Determinism guardrails & instrumentation
- Sort job completions; avoid order‑dependent HashMap iteration when outcomes differ; use stable key order if needed.
- Add per‑tick metrics: events processed, queue depth, jobs started/completed, dropped stale results.

7) Validation
- Manual test matrix: chunk streaming (move across boundaries), edits (remove/place, emissive blocks), dynamic emitters (L/K), neighbor rebuilds, underground lighting, wireframe toggle, bounds debug overlay.
- Determinism spot‑check: record sequence of applied events/job_ids on two successive runs; verify identical ordering.

8) Bootstrap sequence
- On startup enqueue:
  - `InputCaptured` (empty) for tick 0 to initialize subsystems.
  - `MovementRequested` for player spawn yaw/position.
  - `ViewCenterChanged` for initial center; then `EnsureChunkLoaded` for the view radius.

Later work (post‑cutover)
- Event log + snapshots for replay.
- Multiplayer/lockstep input shaping (if desired later).

Phase 5 — Cleanups & Guarantees
- Remove remaining direct mutations; make systems read‑only except through event handlers.
- Add invariants/metrics and debugging UI (queue depth, per‑tick timings, last N events).

## Data Structures (Initial Sketch)

Authoritative logical state contained in one struct:

```
struct GameState {
  tick: u64,

  // World config
  world: World, // chunks_x/z, chunk_size, seed

  // Streaming
  view_center: (i32, i32),
  view_radius: i32,
  loaded: HashMap<(i32,i32), ChunkState>,   // includes buf, built_rev, gpu_model_id (opaque), bbox
  pending: HashSet<(i32,i32)>,

  // Edits & revisions
  edits: HashMap<(i32,i32), HashMap<(i32,i32,i32), Block>>, // per-chunk overlay
  rev_latest: HashMap<(i32,i32), u64>,
  rev_built: HashMap<(i32,i32), u64>,
  rev_counter: u64,

  // Lighting
  emitters: HashMap<(i32,i32), Vec<(usize,usize,usize,u8,bool)>>,
  light_borders: HashMap<(i32,i32), LightBorders>,

  // Jobs
  next_job_id: u64,
  jobs_pending: HashMap<u64, BuildJob>,

  // Player
  player: PlayerState,
}

struct ChunkState {
  cx: i32,
  cz: i32,
  buf: Option<ChunkBuf>,
  gpu_model_id: Option<u64>, // handle managed by Runtime
  built_rev: u64,
  bbox: BoundingBox,
}

struct PlayerState { pos: Vector3, vel: Vector3, yaw: f32, on_ground: bool, walk_mode: bool }

struct App {
  gs: GameState,
  queue: EventQueue,
  runtime: Runtime, // threads, raylib handles, texture cache, channel endpoints
}
```

Runtime holds non‑authoritative, side‑effect state (threadpool, channels, raylib handles, textures). All mutations that matter for replay live in `GameState` only.

APIs (sketch)
```
impl EventQueue {
  fn emit_now(&mut self, id: u64, kind: Event);
  fn emit_at(&mut self, id: u64, tick: u64, kind: Event);
  fn emit_after(&mut self, id: u64, delta: u64, kind: Event) { self.emit_at(id, self.now()+delta, kind) }
}

impl App {
  fn tick(&mut self) {
    let cur = self.gs.tick;
    // 1) Drain worker ingress → completion events (sorted by job_id)
    for ev in self.runtime.drain_worker_events_sorted() { self.queue.emit_now(self.next_id(), ev); }
    // 2) Process events scheduled for this tick up to budget
    let mut processed = 0;
    while let Some(env) = self.queue.pop_ready(cur) {
      self.dispatch(&env); // mutates gs via handlers; may emit more events
      processed += 1;
      if processed >= MAX_EVENTS_PER_TICK { break; }
    }
    self.gs.tick += 1;
  }
}
```

## Example Flows

1) Edit block
- Input → `RaycastEditRequested(place=true, block=Stone)`
- EditSystem: raycast with sampler; if valid, `BlockEditApplied { old, new }`
- EditSystem: set edit in `EditStore`, bump revs, emitters add/remove, then `ChunkRebuildRequested` for the direct chunk.
- MeshingSystem: translate to `BuildChunkJobRequested` (job_id stable), worker runs, intake `BuildChunkJobCompleted`, apply buf + upload mesh, mark built rev, emit `LightBordersUpdated` if changed.
- LightingSystem: on borders update, emit neighbor `ChunkRebuildRequested` as needed.

2) Movement and streaming
- Input → `MovementRequested(entity=player, yaw, dt)`
- PlayerSystem: update `PlayerStore` using sampler; if center chunk changed, `ViewCenterChanged`.
- StreamingSystem: diff desired set, emit `EnsureChunkLoaded`/`EnsureChunkUnloaded`, and `BuildChunkJobRequested` for newly needed chunks.
- MeshingSystem: submit jobs; intake results deterministically.

## Determinism Notes & Risks

- Worker arrival order: solved by buffering and sorting `BuildChunkJobCompleted` by `job_id` within a tick.
- Floating point: Physics and lighting use fixed code paths; keep neighbor iteration orders stable (already fixed arrays). Avoid non‑deterministic iteration over hash maps when it affects outcomes.
- Time: No wall‑clock in core logic; all time originates from tick duration inputs. Normalize `dt` for movement into integer sub‑steps if necessary.
- Randomness: World gen is seeded; any new randomness must derive from deterministic seeds.

HashMap iteration
- Never rely on `HashMap` iteration order for logic. When ordering matters (e.g., selecting neighbors to rebuild), sort the keys.

Frame pacing
- Budget events/ticks per frame; render uses last committed state. Avoid per‑frame direct mutations.

Error handling & backpressure
- If event storms occur (queue grows too large), drop redundant events by coalescing (e.g., multiple `ChunkRebuildRequested` for same `(cx,cz)` and same/latest `rev`).

Diagnostics
- Keep a ring buffer of the last N applied events (ids, kinds, payload summaries) and expose a simple overlay or log print for debugging.

## Acceptance Criteria

- No direct state mutation outside event handlers; all world changes arise from events.
- Streaming/meshing driven entirely by events; deterministic worker intake used (sorted by `job_id`).
- Edits and lighting changes occur via events; neighbor rebuilds are event‑driven.
- Player movement is event‑driven and updates `GameState.player` only via handlers; streaming responds to `ViewCenterChanged`.
- Main loop uses the event queue; legacy mutation paths removed.

## Next Steps

1) Implement `Event`, `EventEnvelope`, `EventQueue`, `Runtime`, and `GameState`.
2) Move worker threads/channels into `Runtime`; implement deterministic job intake.
3) Port streaming/meshing, edits/lighting, and movement to event handlers in one pass.
4) Replace `main.rs` loop with `App` loop using the event queue and render from `GameState`.
5) Remove legacy mutation paths and validate with the test matrix.

Open questions to align on before coding:
- Tick budgeting: Process N events per tick vs process‑all? Recommend a budget (e.g., timeboxed to 2–4 ms) and spillover to next tick.
- Persistence priority: Do we want the log/snapshot in Phase 2 instead?
- Input determinism: For multiplayer/lockstep later, do we want to quantize input into fixed action packets now?

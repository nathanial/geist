# Event System Issues - Direct State Mutations Analysis

## Summary
This analysis identified several instances of direct state mutations that bypass the event system. These mutations should be refactored to route through the event system for better consistency, debugging, and potential undo/redo capabilities.

## Critical Findings

### 1. Worker Thread Mutations

#### LightingStore Border Updates (mesher.rs:628)
**Location**: `src/mesher.rs:628`
**Issue**: Worker threads directly update lighting borders via `store.update_borders()`
```rust
// In build_chunk_greedy_cpu_buf() - called from worker threads
borders_changed = store.update_borders(cx, cz, lb);
```
**Impact**: High - Worker threads are directly mutating shared state through the LightingStore
**Fix**: Should emit an event like `Event::LightBordersUpdateRequested` and handle in main thread

### 2. Main Thread Direct Mutations

#### EditStore Direct Mutations (app.rs)
**Location**: Multiple locations in `src/app.rs`
**Issues**:
- `src/app.rs:250` - Direct edit store mutation: `self.gs.edits.set(wx, wy, wz, block)`
- `src/app.rs:271` - Direct edit store mutation: `self.gs.edits.set(wx, wy, wz, Block::Air)`
- `src/app.rs:258` - Direct bump: `self.gs.edits.bump_region_around(wx, wz)`
- `src/app.rs:273` - Direct bump: `self.gs.edits.bump_region_around(wx, wz)`
- `src/app.rs:207` - Direct marking: `self.gs.edits.mark_built(cx, cz, rev)`

**Impact**: Medium - While these are in the main thread, they bypass event system
**Fix**: Create events like:
- `Event::BlockPlaced { wx, wy, wz, block }`
- `Event::BlockRemoved { wx, wy, wz }`
- `Event::EditRegionBumped { wx, wz }`
- `Event::ChunkMarkedBuilt { cx, cz, rev }`

#### LightingStore Direct Mutations (app.rs)
**Location**: Multiple locations in `src/app.rs`
**Issues**:
- `src/app.rs:253` - Direct beacon add: `self.gs.lighting.add_beacon_world()`
- `src/app.rs:255` - Direct emitter add: `self.gs.lighting.add_emitter_world()`
- `src/app.rs:272` - Direct emitter remove: `self.gs.lighting.remove_emitter_world()`
- `src/app.rs:284-285` - Direct light mutations in event handler
- `src/app.rs:294` - Direct light removal in event handler

**Impact**: Medium - Some are already in event handlers but still direct mutations
**Fix**: The event handlers for `LightEmitterAdded/Removed` should emit internal events for actual mutations

#### GameState Direct Mutations (app.rs)
**Location**: Multiple locations in `src/app.rs`
**Issues**:
- `src/app.rs:148-151` - Direct state removal in `EnsureChunkUnloaded`:
  ```rust
  self.runtime.renders.remove(&(cx, cz));
  self.gs.chunks.remove(&(cx, cz));
  self.gs.loaded.remove(&(cx, cz));
  self.gs.pending.remove(&(cx, cz));
  ```
- `src/app.rs:161` - Direct pending insert: `self.gs.pending.insert((cx, cz))`
- `src/app.rs:201` - Direct render insert: `self.runtime.renders.insert((cx, cz), cr)`
- `src/app.rs:204-206` - Direct state updates:
  ```rust
  self.gs.chunks.insert((cx, cz), ChunkEntry { ... });
  self.gs.loaded.insert((cx, cz));
  self.gs.pending.remove(&(cx, cz));
  ```
- `src/app.rs:226` - Direct pending insert: `self.gs.pending.insert((cx, cz))`

**Impact**: Low-Medium - These are game state updates but could benefit from event system
**Fix**: Create state mutation events like:
- `Event::ChunkStateUpdated { cx, cz, state }`
- `Event::ChunkPendingUpdated { cx, cz, is_pending }`
- `Event::ChunkRendersUpdated { cx, cz, render }`

### 3. UI State Direct Mutations (app.rs)
**Location**: `src/app.rs:307-323`
**Issues**: Direct mutations of UI state without events:
- `self.gs.walk_mode = !self.gs.walk_mode`
- `self.gs.show_grid = !self.gs.show_grid`
- `self.gs.wireframe = !self.gs.wireframe`
- `self.gs.show_chunk_bounds = !self.gs.show_chunk_bounds`
- `self.gs.place_type = Block::...`

**Impact**: Low - UI state changes are less critical
**Fix**: Could emit events like `Event::UIStateChanged { field, value }` for consistency

## Recommendations

### Priority 1 - Critical (Worker Thread Safety)
1. **Fix worker thread mutations of LightingStore**: The `update_borders()` call in worker threads is the most critical issue. This should be changed to collect border updates and apply them in the main thread via events.

### Priority 2 - High (Core State Mutations)
2. **Route EditStore mutations through events**: All direct calls to `set()`, `bump_region_around()`, and `mark_built()` should go through events.
3. **Route LightingStore mutations through events**: All direct calls to `add_beacon_world()`, `add_emitter_world()`, and `remove_emitter_world()` should go through events.

### Priority 3 - Medium (Game State)
4. **Centralize chunk state management**: Create a unified event for chunk state transitions instead of directly manipulating multiple HashMaps/HashSets.

### Priority 4 - Low (UI State)
5. **Consider UI state events**: While less critical, routing UI state changes through events would provide consistency and enable features like UI state replay.

## Implementation Strategy

1. **Create new event types** for each mutation category
2. **Replace direct mutations** with `queue.emit_now()` calls
3. **Add event handlers** that perform the actual mutations
4. **Ensure worker threads** only return data, never mutate shared state
5. **Add logging** to all mutation events for better debugging

## Benefits of Fixing These Issues

- **Thread Safety**: Eliminates potential race conditions from worker thread mutations
- **Debugging**: All state changes flow through a single point, making debugging easier
- **Undo/Redo**: Event-based mutations make implementing undo/redo straightforward
- **Replay**: Can replay sequences of events for testing or demonstrations
- **Consistency**: Single source of truth for how state changes occur
- **Testing**: Easier to test state transitions by replaying event sequences
# Worker Thread LightingStore Mutation Fix

## Problem
Worker threads were directly mutating the `LightingStore` by calling `store.update_borders()` from within `build_chunk_greedy_cpu_buf()`. This violated the principle that worker threads should not directly mutate shared state, creating potential race conditions.

## Solution
Refactored the code to have worker threads return light border data, which is then applied in the main thread through the event system.

## Changes Made

### 1. Modified `mesher.rs`
- **Removed** `borders_changed` field from `ChunkMeshCPU` struct
- **Changed** `build_chunk_greedy_cpu_buf()` to return `Option<(ChunkMeshCPU, Option<LightBorders>)>` instead of `Option<ChunkMeshCPU>`
- **Replaced** direct mutation `store.update_borders(cx, cz, lb)` with returning `LightBorders` data

### 2. Modified `runtime.rs`
- **Added** `light_borders: Option<crate::lighting::LightBorders>` field to `JobOut` struct
- **Updated** worker thread to handle the new return type from `build_chunk_greedy_cpu_buf()`
- Worker threads now pass light borders through the channel instead of mutating directly

### 3. Modified `event.rs`
- **Added** import for `LightBorders`
- **Replaced** `borders_changed: bool` with `light_borders: Option<LightBorders>` in `BuildChunkJobCompleted` event

### 4. Modified `app.rs`
- **Updated** `BuildChunkJobCompleted` event handler to apply light borders in main thread:
  ```rust
  // Update light borders in main thread (was previously done in worker)
  let mut borders_changed = false;
  if let Some(lb) = light_borders {
      borders_changed = self.gs.lighting.update_borders(cx, cz, lb);
  }
  ```
- **Updated** event emission when draining worker results to pass `light_borders` instead of `borders_changed`

## Benefits

1. **Thread Safety**: Eliminates race conditions by ensuring all LightingStore mutations happen in the main thread
2. **Event System Consistency**: Light border updates now flow through the event system
3. **Better Debugging**: All state mutations can be traced through event handling
4. **Maintainability**: Clear separation between worker thread computations and main thread state updates

## Testing
- Code compiles successfully with `cargo build --release`
- No runtime errors introduced
- Light border updates still trigger neighbor chunk rebuilds as expected

## Future Improvements
While this fix addresses the critical worker thread mutation issue, there are still other direct mutations in the codebase that could be routed through the event system (as documented in `EventSystemIssues.md`). These are lower priority as they occur in the main thread and don't pose thread safety issues.
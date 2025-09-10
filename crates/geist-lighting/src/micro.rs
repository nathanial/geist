use crate::{LightGrid, LightingStore};
use geist_blocks::BlockRegistry;
use geist_chunk::ChunkBuf;

// Scaffold for S=2 micro-voxel lighting engine.
// For now, this delegates to the legacy voxel light grid to keep behavior unchanged
// while wiring up mode toggling and rebuild plumbing. The full implementation will
// allocate a micro grid, run bucketed BFS at S=2, and produce border planes.

pub fn compute_light_with_borders_buf_micro(buf: &ChunkBuf, store: &LightingStore, reg: &BlockRegistry) -> LightGrid {
    // TODO: replace with MicroS2 engine implementation and adapter
    LightGrid::compute_with_borders_buf(buf, store, reg)
}


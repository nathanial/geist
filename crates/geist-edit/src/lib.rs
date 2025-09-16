//! Persistent world edits and revisions.
#![forbid(unsafe_code)]

use geist_blocks::types::Block;
use std::collections::HashMap;

/// Chunk-aware persistent edit store with simple change tracking.
pub struct EditStore {
    sx: i32,
    sy: i32,
    sz: i32,
    // Map per-chunk: key=(cx,cy,cz) -> map of world coords -> Block
    inner: HashMap<(i32, i32, i32), HashMap<(i32, i32, i32), Block>>,
    // Change-tracking
    rev: HashMap<(i32, i32, i32), u64>, // latest requested change affecting chunk
    built: HashMap<(i32, i32, i32), u64>, // last built rev for chunk
    counter: u64,
}

impl EditStore {
    pub fn new(sx: i32, sy: i32, sz: i32) -> Self {
        Self {
            sx,
            sy,
            sz,
            inner: HashMap::new(),
            rev: HashMap::new(),
            built: HashMap::new(),
            counter: 0,
        }
    }

    #[inline]
    fn chunk_key(&self, wx: i32, wy: i32, wz: i32) -> (i32, i32, i32) {
        (
            wx.div_euclid(self.sx),
            wy.div_euclid(self.sy),
            wz.div_euclid(self.sz),
        )
    }

    pub fn get(&self, wx: i32, wy: i32, wz: i32) -> Option<Block> {
        let k = self.chunk_key(wx, wy, wz);
        self.inner
            .get(&k)
            .and_then(|m| m.get(&(wx, wy, wz)).copied())
    }

    pub fn set(&mut self, wx: i32, wy: i32, wz: i32, b: Block) {
        let k = self.chunk_key(wx, wy, wz);
        let entry = self.inner.entry(k).or_default();
        entry.insert((wx, wy, wz), b);
    }

    /// Snapshot of all edits for a specific chunk
    pub fn snapshot_for_chunk(&self, cx: i32, cy: i32, cz: i32) -> Vec<((i32, i32, i32), Block)> {
        if let Some(m) = self.inner.get(&(cx, cy, cz)) {
            return m.iter().map(|(k, v)| (*k, *v)).collect();
        }
        Vec::new()
    }

    /// Snapshot of all edits across a chunk region (inclusive radius in chunk units)
    pub fn snapshot_for_region(
        &self,
        cx: i32,
        cy: i32,
        cz: i32,
        radius_xz: i32,
        radius_y: i32,
    ) -> Vec<((i32, i32, i32), Block)> {
        let mut out = Vec::new();
        for dy in -radius_y..=radius_y {
            for dz in -radius_xz..=radius_xz {
                for dx in -radius_xz..=radius_xz {
                    let k = (cx + dx, cy + dy, cz + dz);
                    if let Some(m) = self.inner.get(&k) {
                        for (k2, v) in m.iter() {
                            out.push((*k2, *v));
                        }
                    }
                }
            }
        }
        out
    }

    /// Change-tracking: mark the chunk containing (wx,wz) and any immediate neighbors
    /// if the edit touches a border. Returns a new monotonically increasing stamp.
    pub fn bump_region_around(&mut self, wx: i32, wy: i32, wz: i32) -> u64 {
        self.counter = self.counter.wrapping_add(1).max(1);
        let stamp = self.counter;
        let (cx, cy, cz) = self.chunk_key(wx, wy, wz);
        // Only bump the chunk that was directly edited and its immediate neighbors
        // if the edit is near a chunk boundary (within 1 block of edge)
        let x0 = cx * self.sx;
        let y0 = cy * self.sy;
        let z0 = cz * self.sz;
        let lx = wx - x0;
        let ly = wy - y0;
        let lz = wz - z0;

        // Always bump the current chunk
        self.rev.insert((cx, cy, cz), stamp);

        let mut offsets_x = vec![0];
        let mut offsets_y = vec![0];
        let mut offsets_z = vec![0];
        if lx == 0 {
            offsets_x.push(-1);
        }
        if lx == self.sx - 1 {
            offsets_x.push(1);
        }
        if ly == 0 {
            offsets_y.push(-1);
        }
        if ly == self.sy - 1 {
            offsets_y.push(1);
        }
        if lz == 0 {
            offsets_z.push(-1);
        }
        if lz == self.sz - 1 {
            offsets_z.push(1);
        }

        for dx in offsets_x {
            for dy in &offsets_y {
                for dz in &offsets_z {
                    if dx == 0 && *dy == 0 && *dz == 0 {
                        continue;
                    }
                    self.rev.insert((cx + dx, cy + *dy, cz + *dz), stamp);
                }
            }
        }
        stamp
    }

    /// Get list of chunks affected by an edit at world position
    pub fn get_affected_chunks(&self, wx: i32, wy: i32, wz: i32) -> Vec<(i32, i32, i32)> {
        let mut affected: Vec<(i32, i32, i32)> = Vec::new();
        let (cx, cy, cz) = self.chunk_key(wx, wy, wz);
        let x0 = cx * self.sx;
        let y0 = cy * self.sy;
        let z0 = cz * self.sz;
        let lx = wx - x0;
        let ly = wy - y0;
        let lz = wz - z0;

        // Always include current chunk
        affected.push((cx, cy, cz));

        let mut offsets_x = vec![0];
        let mut offsets_y = vec![0];
        let mut offsets_z = vec![0];
        if lx == 0 {
            offsets_x.push(-1);
        }
        if lx == self.sx - 1 {
            offsets_x.push(1);
        }
        if ly == 0 {
            offsets_y.push(-1);
        }
        if ly == self.sy - 1 {
            offsets_y.push(1);
        }
        if lz == 0 {
            offsets_z.push(-1);
        }
        if lz == self.sz - 1 {
            offsets_z.push(1);
        }

        for dx in offsets_x {
            for dy in &offsets_y {
                for dz in &offsets_z {
                    if dx == 0 && *dy == 0 && *dz == 0 {
                        continue;
                    }
                    let key = (cx + dx, cy + *dy, cz + *dz);
                    if !affected.contains(&key) {
                        affected.push(key);
                    }
                }
            }
        }

        affected
    }

    pub fn get_rev(&self, cx: i32, cy: i32, cz: i32) -> u64 {
        self.rev.get(&(cx, cy, cz)).copied().unwrap_or(0)
    }

    pub fn mark_built(&mut self, cx: i32, cy: i32, cz: i32, rev: u64) {
        // Only update if this is a newer revision
        let e = self.built.entry((cx, cy, cz)).or_insert(0);
        if rev > *e {
            *e = rev;
        }
    }

    /// Check if a chunk needs rebuilding
    #[allow(dead_code)]
    pub fn needs_rebuild(&self, cx: i32, cy: i32, cz: i32) -> bool {
        let current_rev = self.get_rev(cx, cy, cz);
        let built_rev = self.get_built_rev(cx, cy, cz);
        current_rev > built_rev
    }

    #[allow(dead_code)]
    pub fn get_built_rev(&self, cx: i32, cy: i32, cz: i32) -> u64 {
        self.built.get(&(cx, cy, cz)).copied().unwrap_or(0)
    }
}

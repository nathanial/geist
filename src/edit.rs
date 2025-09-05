use crate::voxel::Block;
use std::collections::HashMap;

pub struct EditStore {
    sx: i32,
    sz: i32,
    // Map per-chunk: key=(cx,cz) -> map of world coords -> Block
    inner: HashMap<(i32, i32), HashMap<(i32, i32, i32), Block>>,
    // Change-tracking
    rev: HashMap<(i32, i32), u64>, // latest requested change affecting chunk
    built: HashMap<(i32, i32), u64>, // last built rev for chunk
    counter: u64,
}

impl EditStore {
    pub fn new(sx: i32, _sy: i32, sz: i32) -> Self {
        Self {
            sx,
            sz,
            inner: HashMap::new(),
            rev: HashMap::new(),
            built: HashMap::new(),
            counter: 0,
        }
    }

    #[inline]
    fn chunk_key(&self, wx: i32, wz: i32) -> (i32, i32) {
        (wx.div_euclid(self.sx), wz.div_euclid(self.sz))
    }

    pub fn get(&self, wx: i32, wy: i32, wz: i32) -> Option<Block> {
        let k = self.chunk_key(wx, wz);
        self.inner
            .get(&k)
            .and_then(|m| m.get(&(wx, wy, wz)).copied())
    }

    pub fn set(&mut self, wx: i32, wy: i32, wz: i32, b: Block) {
        let k = self.chunk_key(wx, wz);
        let entry = self.inner.entry(k).or_insert_with(HashMap::new);
        entry.insert((wx, wy, wz), b);
    }

    // Snapshot of all edits for a specific chunk
    pub fn snapshot_for_chunk(&self, cx: i32, cz: i32) -> Vec<((i32, i32, i32), Block)> {
        if let Some(m) = self.inner.get(&(cx, cz)) {
            return m.iter().map(|(k, v)| (*k, *v)).collect();
        }
        Vec::new()
    }

    // Snapshot of all edits across a chunk region (inclusive radius in chunk units)
    pub fn snapshot_for_region(
        &self,
        cx: i32,
        cz: i32,
        radius: i32,
    ) -> Vec<((i32, i32, i32), Block)> {
        let mut out = Vec::new();
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                let k = (cx + dx, cz + dz);
                if let Some(m) = self.inner.get(&k) {
                    for (k2, v) in m.iter() {
                        out.push((*k2, *v));
                    }
                }
            }
        }
        out
    }

    // Change-tracking API
    pub fn bump_region_around(&mut self, wx: i32, wz: i32) -> u64 {
        self.counter = self.counter.wrapping_add(1).max(1);
        let stamp = self.counter;
        let (cx, cz) = self.chunk_key(wx, wz);
        // Only bump the chunk that was directly edited and its immediate neighbors
        // if the edit is near a chunk boundary (within 1 block of edge)
        let x0 = cx * self.sx;
        let z0 = cz * self.sz;
        let lx = wx - x0;
        let lz = wz - z0;

        // Always bump the current chunk
        self.rev.insert((cx, cz), stamp);

        // Check if near chunk boundaries and bump neighbors accordingly
        if lx == 0 { self.rev.insert((cx - 1, cz), stamp); }
        if lx == self.sx - 1 { self.rev.insert((cx + 1, cz), stamp); }
        if lz == 0 { self.rev.insert((cx, cz - 1), stamp); }
        if lz == self.sz - 1 { self.rev.insert((cx, cz + 1), stamp); }

        // Check corners
        if lx == 0 && lz == 0 { self.rev.insert((cx - 1, cz - 1), stamp); }
        if lx == 0 && lz == self.sz - 1 { self.rev.insert((cx - 1, cz + 1), stamp); }
        if lx == self.sx - 1 && lz == 0 { self.rev.insert((cx + 1, cz - 1), stamp); }
        if lx == self.sx - 1 && lz == self.sz - 1 { self.rev.insert((cx + 1, cz + 1), stamp); }
        stamp
    }

    // Get list of chunks affected by an edit at world position
    pub fn get_affected_chunks(&self, wx: i32, wz: i32) -> Vec<(i32, i32)> {
        let mut affected = Vec::new();
        let (cx, cz) = self.chunk_key(wx, wz);
        let x0 = cx * self.sx;
        let z0 = cz * self.sz;
        let lx = wx - x0;
        let lz = wz - z0;

        // Always include current chunk
        affected.push((cx, cz));

        // Check if near chunk boundaries
        if lx == 0 {
            affected.push((cx - 1, cz));
        }
        if lx == self.sx - 1 {
            affected.push((cx + 1, cz));
        }
        if lz == 0 {
            affected.push((cx, cz - 1));
        }
        if lz == self.sz - 1 {
            affected.push((cx, cz + 1));
        }

        // Check corners
        if lx == 0 && lz == 0 {
            affected.push((cx - 1, cz - 1));
        }
        if lx == 0 && lz == self.sz - 1 {
            affected.push((cx - 1, cz + 1));
        }
        if lx == self.sx - 1 && lz == 0 {
            affected.push((cx + 1, cz - 1));
        }
        if lx == self.sx - 1 && lz == self.sz - 1 {
            affected.push((cx + 1, cz + 1));
        }

        affected
    }

    pub fn get_rev(&self, cx: i32, cz: i32) -> u64 {
        self.rev.get(&(cx, cz)).copied().unwrap_or(0)
    }

    pub fn mark_built(&mut self, cx: i32, cz: i32, rev: u64) {
        // Only update if this is a newer revision
        let e = self.built.entry((cx, cz)).or_insert(0);
        if rev > *e { *e = rev; }
    }

    // Check if a chunk needs rebuilding
    pub fn needs_rebuild(&self, cx: i32, cz: i32) -> bool {
        let current_rev = self.get_rev(cx, cz);
        let built_rev = self.get_built_rev(cx, cz);
        current_rev > built_rev
    }

    pub fn get_built_rev(&self, cx: i32, cz: i32) -> u64 {
        self.built.get(&(cx, cz)).copied().unwrap_or(0)
    }
}

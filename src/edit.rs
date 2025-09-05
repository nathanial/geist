use std::collections::HashMap;
use std::sync::{RwLock};
use std::sync::atomic::{AtomicU64, Ordering};
use crate::voxel::Block;

pub struct EditStore {
    sx: i32,
    sy: i32,
    sz: i32,
    // Map per-chunk: key=(cx,cz) -> map of world coords -> Block
    inner: RwLock<HashMap<(i32,i32), HashMap<(i32,i32,i32), Block>>>,
    // Change-tracking
    rev: RwLock<HashMap<(i32,i32), u64>>,       // latest requested change affecting chunk
    built: RwLock<HashMap<(i32,i32), u64>>,     // last built rev for chunk
    counter: AtomicU64,
}

impl EditStore {
    pub fn new(sx: i32, sy: i32, sz: i32) -> Self {
        Self { sx, sy, sz, inner: RwLock::new(HashMap::new()), rev: RwLock::new(HashMap::new()), built: RwLock::new(HashMap::new()), counter: AtomicU64::new(0) }
    }

    #[inline]
    fn chunk_key(&self, wx: i32, wz: i32) -> (i32,i32) {
        (wx.div_euclid(self.sx), wz.div_euclid(self.sz))
    }

    pub fn get(&self, wx: i32, wy: i32, wz: i32) -> Option<Block> {
        let k = self.chunk_key(wx, wz);
        let g = self.inner.read().ok()?;
        g.get(&k).and_then(|m| m.get(&(wx,wy,wz)).copied())
    }

    pub fn set(&self, wx: i32, wy: i32, wz: i32, b: Block) {
        let k = self.chunk_key(wx, wz);
        if let Ok(mut g) = self.inner.write() {
            let entry = g.entry(k).or_insert_with(HashMap::new);
            entry.insert((wx,wy,wz), b);
        }
    }

    pub fn remove(&self, wx: i32, wy: i32, wz: i32) {
        let k = self.chunk_key(wx, wz);
        if let Ok(mut g) = self.inner.write() {
            if let Some(map) = g.get_mut(&k) {
                map.remove(&(wx,wy,wz));
                if map.is_empty() { g.remove(&k); }
            }
        }
    }

    // Snapshot of all edits for a specific chunk
    pub fn snapshot_for_chunk(&self, cx: i32, cz: i32) -> Vec<((i32,i32,i32), Block)> {
        if let Ok(g) = self.inner.read() {
            if let Some(m) = g.get(&(cx,cz)) {
                return m.iter().map(|(k,v)| (*k, *v)).collect();
            }
        }
        Vec::new()
    }

    // Snapshot of all edits across a chunk region (inclusive radius in chunk units)
    pub fn snapshot_for_region(&self, cx: i32, cz: i32, radius: i32) -> Vec<((i32,i32,i32), Block)> {
        let mut out = Vec::new();
        if let Ok(g) = self.inner.read() {
            for dz in -radius..=radius { for dx in -radius..=radius {
                let k = (cx + dx, cz + dz);
                if let Some(m) = g.get(&k) { for (k2,v) in m.iter() { out.push((*k2, *v)); } }
            }}
        }
        out
    }

    // Change-tracking API
    pub fn bump_region_around(&self, wx: i32, wz: i32) -> u64 {
        let stamp = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        let (cx, cz) = self.chunk_key(wx, wz);
        if let Ok(mut r) = self.rev.write() {
            for dz in -1..=1 { for dx in -1..=1 { r.insert((cx+dx, cz+dz), stamp); } }
        }
        stamp
    }

    pub fn get_rev(&self, cx: i32, cz: i32) -> u64 {
        self.rev.read().ok().and_then(|m| m.get(&(cx,cz)).copied()).unwrap_or(0)
    }

    pub fn mark_built(&self, cx: i32, cz: i32, rev: u64) {
        if let Ok(mut b) = self.built.write() {
            let e = b.entry((cx,cz)).or_insert(0);
            if rev > *e { *e = rev; }
        }
    }

    pub fn get_built_rev(&self, cx: i32, cz: i32) -> u64 {
        self.built.read().ok().and_then(|m| m.get(&(cx,cz)).copied()).unwrap_or(0)
    }
}

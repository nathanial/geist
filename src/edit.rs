use std::collections::HashMap;
use std::sync::RwLock;
use crate::voxel::Block;

pub struct EditStore {
    sx: i32,
    sy: i32,
    sz: i32,
    // Map per-chunk: key=(cx,cz) -> map of world coords -> Block
    inner: RwLock<HashMap<(i32,i32), HashMap<(i32,i32,i32), Block>>>,
}

impl EditStore {
    pub fn new(sx: i32, sy: i32, sz: i32) -> Self {
        Self { sx, sy, sz, inner: RwLock::new(HashMap::new()) }
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
}

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use geist_world::ChunkCoord;
use geist_world::voxel::generation::ChunkColumnProfile;

#[derive(Clone, Copy, Debug, Default)]
pub struct ChunkColumnCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub entries: usize,
}

pub struct ChunkColumnCache {
    entries: RwLock<HashMap<ChunkCoord, Arc<ChunkColumnProfile>>>,
    order: Mutex<VecDeque<ChunkCoord>>,
    capacity: usize,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

impl ChunkColumnCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            order: Mutex::new(VecDeque::new()),
            capacity,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }

    pub fn get(&self, coord: ChunkCoord, expected_rev: u32) -> Option<Arc<ChunkColumnProfile>> {
        if let Some(profile) = self.lookup(&coord) {
            if profile.worldgen_rev == expected_rev {
                self.hits.fetch_add(1, Ordering::Relaxed);
                self.touch(&coord);
                return Some(profile);
            }
            self.remove_entry(&coord);
        }
        self.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    pub fn insert(&self, profile: Arc<ChunkColumnProfile>) {
        let coord = profile.coord;
        {
            let mut entries = self.entries.write().unwrap();
            entries.insert(coord, profile);
        }
        self.remove_from_order(&coord);
        {
            let mut order = self.order.lock().unwrap();
            order.push_back(coord);
        }
        self.enforce_capacity();
    }

    pub fn clear(&self) {
        let removed = {
            let mut entries = self.entries.write().unwrap();
            let len = entries.len() as u64;
            entries.clear();
            len
        };
        if removed > 0 {
            self.evictions.fetch_add(removed, Ordering::Relaxed);
        }
        self.order.lock().unwrap().clear();
    }

    pub fn stats(&self) -> ChunkColumnCacheStats {
        ChunkColumnCacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            entries: self.entries.read().map(|m| m.len()).unwrap_or(0),
        }
    }

    fn lookup(&self, coord: &ChunkCoord) -> Option<Arc<ChunkColumnProfile>> {
        self.entries
            .read()
            .ok()
            .and_then(|map| map.get(coord).cloned())
    }

    fn remove_entry(&self, coord: &ChunkCoord) {
        let removed = {
            let mut entries = self.entries.write().unwrap();
            entries.remove(coord)
        };
        if removed.is_some() {
            self.evictions.fetch_add(1, Ordering::Relaxed);
        }
        self.remove_from_order(coord);
    }

    fn touch(&self, coord: &ChunkCoord) {
        let mut order = self.order.lock().unwrap();
        if let Some(pos) = order.iter().position(|c| c == coord) {
            if let Some(entry) = order.remove(pos) {
                order.push_back(entry);
            }
        }
    }

    fn remove_from_order(&self, coord: &ChunkCoord) {
        let mut order = self.order.lock().unwrap();
        if let Some(pos) = order.iter().position(|c| c == coord) {
            order.remove(pos);
        }
    }

    fn enforce_capacity(&self) {
        let mut victims: Vec<ChunkCoord> = Vec::new();
        {
            let mut order = self.order.lock().unwrap();
            while order.len() > self.capacity {
                if let Some(old) = order.pop_front() {
                    victims.push(old);
                }
            }
        }
        if victims.is_empty() {
            return;
        }
        let mut entries = self.entries.write().unwrap();
        for coord in victims {
            if entries.remove(&coord).is_some() {
                self.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

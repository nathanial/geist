use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

#[derive(Clone, Debug, Eq)]
pub struct TileKey {
    pub base_x: i32,
    pub base_z: i32,
    pub size_x: usize,
    pub size_z: usize,
}

impl TileKey {
    #[inline]
    pub fn new(base_x: i32, base_z: i32, size_x: usize, size_z: usize) -> Self {
        Self {
            base_x,
            base_z,
            size_x,
            size_z,
        }
    }
}

impl PartialEq for TileKey {
    fn eq(&self, other: &Self) -> bool {
        self.base_x == other.base_x
            && self.base_z == other.base_z
            && self.size_x == other.size_x
            && self.size_z == other.size_z
    }
}

impl Hash for TileKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.base_x.hash(state);
        self.base_z.hash(state);
        self.size_x.hash(state);
        self.size_z.hash(state);
    }
}

#[derive(Debug)]
pub struct TerrainTile {
    key: TileKey,
    pub worldgen_rev: u32,
    heights: Arc<[i32]>,
    pub compute_time_us: u32,
    pub columns: u32,
    pub reuse_count: AtomicU64,
}

impl TerrainTile {
    pub fn new(
        key: TileKey,
        worldgen_rev: u32,
        heights: Vec<i32>,
        compute_time_us: u32,
        columns: u32,
    ) -> Arc<Self> {
        Arc::new(Self {
            key,
            worldgen_rev,
            heights: heights.into(),
            compute_time_us,
            columns,
            reuse_count: AtomicU64::new(0),
        })
    }

    #[inline]
    pub fn key(&self) -> &TileKey {
        &self.key
    }

    #[inline]
    pub fn matches(&self, key: &TileKey) -> bool {
        &self.key == key
    }

    #[inline]
    pub fn height(&self, wx: i32, wz: i32) -> Option<i32> {
        let dx = wx - self.key.base_x;
        let dz = wz - self.key.base_z;
        if dx < 0 || dz < 0 {
            return None;
        }
        let (dx, dz) = (dx as usize, dz as usize);
        if dx >= self.key.size_x || dz >= self.key.size_z {
            return None;
        }
        let idx = dz * self.key.size_x + dx;
        self.heights.get(idx).copied()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TerrainTileCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub entries: usize,
}

pub struct TerrainTileCache {
    entries: RwLock<HashMap<TileKey, Arc<TerrainTile>>>,
    order: Mutex<VecDeque<TileKey>>,
    capacity: usize,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

impl TerrainTileCache {
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

    pub fn get(&self, key: &TileKey, expected_rev: u32) -> Option<Arc<TerrainTile>> {
        if let Some(tile) = self.lookup(key) {
            if tile.worldgen_rev == expected_rev {
                self.hits.fetch_add(1, Ordering::Relaxed);
                tile.reuse_count.fetch_add(1, Ordering::Relaxed);
                self.touch_key(key);
                return Some(tile);
            }
            self.remove_entry(key);
        }
        self.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    pub fn insert(&self, tile: Arc<TerrainTile>) {
        let key = tile.key().clone();
        {
            let mut entries = self.entries.write().unwrap();
            entries.insert(key.clone(), tile);
        }
        self.remove_from_order(&key);
        {
            let mut order = self.order.lock().unwrap();
            order.push_back(key.clone());
        }
        self.enforce_capacity();
    }

    pub fn snapshot(&self) -> TerrainTileCacheStats {
        TerrainTileCacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            entries: self.entries.read().map(|m| m.len()).unwrap_or(0),
        }
    }

    pub fn invalidate_all(&self) {
        let evicted = {
            let mut entries = self.entries.write().unwrap();
            let len = entries.len() as u64;
            entries.clear();
            len
        };
        if evicted > 0 {
            self.evictions.fetch_add(evicted, Ordering::Relaxed);
        }
        let mut order = self.order.lock().unwrap();
        order.clear();
    }

    fn lookup(&self, key: &TileKey) -> Option<Arc<TerrainTile>> {
        self.entries
            .read()
            .ok()
            .and_then(|map| map.get(key).cloned())
    }

    fn remove_entry(&self, key: &TileKey) {
        let removed = {
            let mut entries = self.entries.write().unwrap();
            entries.remove(key)
        };
        if removed.is_some() {
            self.evictions.fetch_add(1, Ordering::Relaxed);
        }
        self.remove_from_order(key);
    }

    fn touch_key(&self, key: &TileKey) {
        let mut order = self.order.lock().unwrap();
        if let Some(pos) = order.iter().position(|k| k == key) {
            if let Some(entry) = order.remove(pos) {
                order.push_back(entry);
            }
        }
    }

    fn remove_from_order(&self, key: &TileKey) {
        let mut order = self.order.lock().unwrap();
        if let Some(pos) = order.iter().position(|k| k == key) {
            order.remove(pos);
        }
    }

    fn enforce_capacity(&self) {
        let mut victims: Vec<TileKey> = Vec::new();
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
        for key in victims {
            if entries.remove(&key).is_some() {
                self.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

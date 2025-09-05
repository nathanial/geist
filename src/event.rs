use std::collections::{BTreeMap, VecDeque};

use crate::chunkbuf::ChunkBuf;
use crate::lighting::LightBorders;
use crate::mesher::{ChunkMeshCPU, NeighborsLoaded};
use crate::voxel::Block;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RebuildCause {
    Edit,
    LightingBorder,
    StreamLoad,
}

pub enum Event {
    // Time housekeeping
    Tick,

    // Input-derived intents
    MovementRequested { dt_ms: u32, yaw: f32, walk_mode: bool },
    RaycastEditRequested { place: bool, block: Block },

    // Player/view
    ViewCenterChanged { ccx: i32, ccz: i32 },

    // Streaming & meshing
    EnsureChunkLoaded { cx: i32, cz: i32 },
    EnsureChunkUnloaded { cx: i32, cz: i32 },
    ChunkRebuildRequested { cx: i32, cz: i32, cause: RebuildCause },
    BuildChunkJobRequested {
        cx: i32,
        cz: i32,
        neighbors: NeighborsLoaded,
        rev: u64,
        job_id: u64,
    },
    BuildChunkJobCompleted {
        cx: i32,
        cz: i32,
        rev: u64,
        cpu: ChunkMeshCPU,
        buf: ChunkBuf,
        light_borders: Option<LightBorders>,
        job_id: u64,
    },

    // Lighting
    LightEmitterAdded { wx: i32, wy: i32, wz: i32, level: u8, is_beacon: bool },
    LightEmitterRemoved { wx: i32, wy: i32, wz: i32 },
    LightBordersUpdated { cx: i32, cz: i32 },
}

pub struct EventEnvelope {
    pub id: u64,
    pub tick: u64,
    pub kind: Event,
}

pub struct EventQueue {
    // map of tick -> FIFO queue of events
    by_tick: BTreeMap<u64, VecDeque<EventEnvelope>>,
    pub now: u64,
    next_id: u64,
}

impl Default for EventQueue {
    fn default() -> Self {
        Self { by_tick: BTreeMap::new(), now: 0, next_id: 1 }
    }
}

impl EventQueue {
    pub fn new() -> Self { Self::default() }

    #[inline]
    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    pub fn emit_now(&mut self, kind: Event) -> u64 {
        let id = self.alloc_id();
        let env = EventEnvelope { id, tick: self.now, kind };
        self.by_tick.entry(self.now).or_default().push_back(env);
        id
    }

    pub fn emit_at(&mut self, tick: u64, kind: Event) -> u64 {
        let id = self.alloc_id();
        let env = EventEnvelope { id, tick, kind };
        self.by_tick.entry(tick).or_default().push_back(env);
        id
    }

    pub fn emit_after(&mut self, delta: u64, kind: Event) -> u64 {
        self.emit_at(self.now + delta, kind)
    }

    pub fn pop_ready(&mut self) -> Option<EventEnvelope> {
        // Get or create current tick bucket
        if let Some((_, q)) = self.by_tick.range_mut(self.now..=self.now).next() {
            if let Some(env) = q.pop_front() {
                if q.is_empty() {
                    // allow cleanup on next step
                }
                return Some(env);
            }
        }
        None
    }

    pub fn advance_tick(&mut self) {
        // clean empty current bucket
        if let Some((tick, q)) = self.by_tick.range(self.now..=self.now).next() {
            if q.is_empty() {
                let key = *tick;
                self.by_tick.remove(&key);
            }
        }
        self.now = self.now.wrapping_add(1);
    }
}

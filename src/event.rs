use std::collections::{BTreeMap, VecDeque};

use crate::chunkbuf::ChunkBuf;
use crate::lighting::LightBorders;
use crate::mesher::{ChunkMeshCPU, NeighborsLoaded};
use crate::structure::StructureId;
use crate::blocks::Block;
use raylib::prelude::Vector3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RebuildCause {
    Edit,
    LightingBorder,
    StreamLoad,
}

#[allow(clippy::large_enum_variant)]
pub enum Event {
    // Time housekeeping
    #[allow(dead_code)]
    Tick,

    // Input-derived intents
    WalkModeToggled,
    GridToggled,
    WireframeToggled,
    ChunkBoundsToggled,
    FrustumCullingToggled,
    BiomeLabelToggled,
    PlaceTypeSelected {
        block: Block,
    },
    MovementRequested {
        dt_ms: u32,
        yaw: f32,
        walk_mode: bool,
    },
    RaycastEditRequested {
        place: bool,
        block: Block,
    },
    BlockPlaced {
        wx: i32,
        wy: i32,
        wz: i32,
        block: Block,
    },
    BlockRemoved {
        wx: i32,
        wy: i32,
        wz: i32,
    },

    // Player/view
    ViewCenterChanged {
        ccx: i32,
        ccz: i32,
    },

    // Streaming & meshing
    EnsureChunkLoaded {
        cx: i32,
        cz: i32,
    },
    EnsureChunkUnloaded {
        cx: i32,
        cz: i32,
    },
    ChunkRebuildRequested {
        cx: i32,
        cz: i32,
        cause: RebuildCause,
    },
    BuildChunkJobRequested {
        cx: i32,
        cz: i32,
        neighbors: NeighborsLoaded,
        rev: u64,
        job_id: u64,
        // Scheduling hint: which cause triggered this build
        cause: RebuildCause,
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

    // Structures
    StructureBuildRequested {
        id: StructureId,
        rev: u64,
    },
    StructureBuildCompleted {
        id: StructureId,
        rev: u64,
        cpu: ChunkMeshCPU,
    },
    // Structure transform updates (pose/motion)
    StructurePoseUpdated {
        id: StructureId,
        pos: Vector3,
        yaw_deg: f32,
        delta: Vector3,
    },
    StructureBlockPlaced {
        id: StructureId,
        lx: i32,
        ly: i32,
        lz: i32,
        block: Block,
    },
    StructureBlockRemoved {
        id: StructureId,
        lx: i32,
        ly: i32,
        lz: i32,
    },

    // Player ↔ structure attachment lifecycle
    PlayerAttachedToStructure {
        id: StructureId,
        local_offset: Vector3,
    },
    PlayerDetachedFromStructure {
        id: StructureId,
    },

    // Lighting
    LightEmitterAdded {
        wx: i32,
        wy: i32,
        wz: i32,
        level: u8,
        is_beacon: bool,
    },
    LightEmitterRemoved {
        wx: i32,
        wy: i32,
        wz: i32,
    },
    LightBordersUpdated {
        cx: i32,
        cz: i32,
    },
}

pub struct EventEnvelope {
    #[allow(dead_code)]
    pub id: u64,
    #[allow(dead_code)]
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
        Self {
            by_tick: BTreeMap::new(),
            now: 0,
            next_id: 1,
        }
    }
}

impl EventQueue {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    pub fn emit_now(&mut self, kind: Event) -> u64 {
        let id = self.alloc_id();
        let env = EventEnvelope {
            id,
            tick: self.now,
            kind,
        };
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

    // Debug: count events that are in past ticks (< now). These will never be processed.
    pub fn count_stale_events(&self) -> usize {
        let mut total = 0usize;
        for (tick, q) in &self.by_tick {
            if *tick < self.now {
                total += q.len();
            }
        }
        total
    }

    // Debug: list per-tick counts for stale buckets (< now)
    pub fn stale_summary(&self) -> Vec<(u64, usize)> {
        let mut v = Vec::new();
        for (tick, q) in &self.by_tick {
            if *tick < self.now {
                v.push((*tick, q.len()));
            }
        }
        v
    }

    // Debug helpers: count queued events by kind across all future ticks (including current)
    pub fn queued_counts(
        &self,
    ) -> (usize, std::collections::BTreeMap<&'static str, usize>) {
        let mut total: usize = 0;
        let mut by: std::collections::BTreeMap<&'static str, usize> = Default::default();
        for q in self.by_tick.values() {
            for env in q {
                total += 1;
                let label: &'static str = match &env.kind {
                    Event::Tick => "Tick",
                    Event::WalkModeToggled => "WalkModeToggled",
                    Event::GridToggled => "GridToggled",
                    Event::WireframeToggled => "WireframeToggled",
                    Event::ChunkBoundsToggled => "ChunkBoundsToggled",
                    Event::FrustumCullingToggled => "FrustumCullingToggled",
                    Event::BiomeLabelToggled => "BiomeLabelToggled",
                    Event::PlaceTypeSelected { .. } => "PlaceTypeSelected",
                    Event::MovementRequested { .. } => "MovementRequested",
                    Event::RaycastEditRequested { .. } => "RaycastEditRequested",
                    Event::BlockPlaced { .. } => "BlockPlaced",
                    Event::BlockRemoved { .. } => "BlockRemoved",
                    Event::ViewCenterChanged { .. } => "ViewCenterChanged",
                    Event::EnsureChunkLoaded { .. } => "EnsureChunkLoaded",
                    Event::EnsureChunkUnloaded { .. } => "EnsureChunkUnloaded",
                    Event::ChunkRebuildRequested { .. } => "ChunkRebuildRequested",
                    Event::BuildChunkJobRequested { .. } => "BuildChunkJobRequested",
                    Event::BuildChunkJobCompleted { .. } => "BuildChunkJobCompleted",
                    Event::StructureBuildRequested { .. } => "StructureBuildRequested",
                    Event::StructureBuildCompleted { .. } => "StructureBuildCompleted",
                    Event::StructurePoseUpdated { .. } => "StructurePoseUpdated",
                    Event::StructureBlockPlaced { .. } => "StructureBlockPlaced",
                    Event::StructureBlockRemoved { .. } => "StructureBlockRemoved",
                    Event::PlayerAttachedToStructure { .. } => "PlayerAttachedToStructure",
                    Event::PlayerDetachedFromStructure { .. } => "PlayerDetachedFromStructure",
                    Event::LightEmitterAdded { .. } => "LightEmitterAdded",
                    Event::LightEmitterRemoved { .. } => "LightEmitterRemoved",
                    Event::LightBordersUpdated { .. } => "LightBordersUpdated",
                };
                *by.entry(label).or_insert(0) += 1;
            }
        }
        (total, by)
    }
}

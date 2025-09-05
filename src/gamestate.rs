use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::chunkbuf::ChunkBuf;
use crate::edit::EditStore;
use crate::lighting::LightingStore;
use crate::player::Walker;
use crate::voxel::{Block, World};
use crate::structure::{Structure, StructureId};

pub struct ChunkEntry {
    pub buf: Option<ChunkBuf>,
    pub built_rev: u64,
}

pub struct GameState {
    pub tick: u64,
    pub world: Arc<World>,

    // Streaming
    pub view_radius_chunks: i32,
    pub center_chunk: (i32, i32),
    pub loaded: HashSet<(i32, i32)>,
    pub pending: HashSet<(i32, i32)>,
    pub chunks: HashMap<(i32, i32), ChunkEntry>,

    // Edits + lighting (authoritative overlays)
    pub edits: EditStore,
    pub lighting: Arc<LightingStore>,

    // Player
    pub walker: Walker,
    pub walk_mode: bool,

    // UI/options
    pub place_type: Block,
    pub show_grid: bool,
    pub wireframe: bool,
    pub show_chunk_bounds: bool,

    // Dynamic voxel bodies (e.g., flying castle)
    pub structures: HashMap<StructureId, Structure>,
    pub ground_attach: Option<GroundAttach>,
}

#[derive(Clone, Copy)]
pub struct GroundAttach {
    pub id: StructureId,
    pub grace: u8,
    pub local_offset: raylib::prelude::Vector3,
}

impl GameState {
    pub fn new(
        world: Arc<World>,
        edits: EditStore,
        lighting: Arc<LightingStore>,
        spawn_eye: raylib::prelude::Vector3,
    ) -> Self {
        use raylib::prelude::*;
        let mut walker = Walker::new(Vector3::new(spawn_eye.x, spawn_eye.y - 1.60, spawn_eye.z));
        walker.yaw = -45.0;
        Self {
            tick: 0,
            center_chunk: (i32::MIN, i32::MIN),
            view_radius_chunks: 6,
            loaded: HashSet::new(),
            pending: HashSet::new(),
            chunks: HashMap::new(),
            edits,
            lighting,
            walker,
            walk_mode: true,
            world,
            place_type: Block::Stone,
            show_grid: true,
            wireframe: false,
            show_chunk_bounds: false,
            structures: HashMap::new(),
            ground_attach: None,
        }
    }
}

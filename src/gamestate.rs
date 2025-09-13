use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::player::Walker;
use geist_blocks::types::Block;
use geist_chunk::ChunkBuf;
use geist_edit::EditStore;
use geist_lighting::LightingStore;
use geist_structures::{Structure, StructureId};
use geist_world::voxel::World;

pub struct ChunkEntry {
    pub buf: Option<ChunkBuf>,
    #[allow(dead_code)]
    pub built_rev: u64,
}

#[derive(Default, Clone, Copy)]
pub struct FinalizeState {
    pub owner_x_ready: bool, // left owner (cx-1,cz) published +X
    pub owner_z_ready: bool, // front owner (cx,cz-1) published +Z
    pub finalize_requested: bool,
    pub finalized: bool,
}

pub struct GameState {
    pub tick: u64,
    pub world: Arc<World>,

    // Streaming
    pub view_radius_chunks: i32,
    pub center_chunk: (i32, i32),
    pub loaded: HashSet<(i32, i32)>,
    pub chunks: HashMap<(i32, i32), ChunkEntry>,
    // How many times each chunk has completed meshing (by (cx, cz))
    pub mesh_counts: HashMap<(i32, i32), u32>,
    // Track newest rev sent to workers per chunk to avoid redundant requeues
    pub inflight_rev: HashMap<(i32, i32), u64>,
    // Finalization tracking per chunk (no-timeout finalize after both owners publish)
    pub finalize: HashMap<(i32, i32), FinalizeState>,

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
    pub frustum_culling_enabled: bool,
    pub show_biome_label: bool,

    // Dynamic voxel bodies (e.g., flying castle)
    pub structures: HashMap<StructureId, Structure>,
    pub ground_attach: Option<GroundAttach>,
    // Control: global speed for moving structures (units/sec)
    pub structure_speed: f32,
    // Control: vertical speed for moving structures (units/sec)
    pub structure_elev_speed: f32,
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
            view_radius_chunks: 10,
            loaded: HashSet::new(),
            chunks: HashMap::new(),
            mesh_counts: HashMap::new(),
            inflight_rev: HashMap::new(),
            finalize: HashMap::new(),
            edits,
            lighting,
            walker,
            walk_mode: true,
            world,
            place_type: Block { id: 0, state: 0 },
            show_grid: true,
            wireframe: false,
            show_chunk_bounds: false,
            frustum_culling_enabled: true,
            show_biome_label: true,
            structures: HashMap::new(),
            ground_attach: None,
            structure_speed: 0.0,
            structure_elev_speed: 0.0,
        }
    }
}

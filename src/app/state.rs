use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use geist_blocks::{Block, BlockRegistry};
use geist_render_raylib::{ChunkRender, FogShader, LeavesShader, TextureCache, WaterShader};
use geist_runtime::Runtime;
use geist_structures::StructureId;
use geist_world::ChunkCoord;
use raylib::prelude::RenderTexture2D;

use crate::camera::FlyCamera;
use crate::event::EventQueue;
use crate::gamestate::GameState;

pub struct App {
    pub gs: GameState,
    pub queue: EventQueue,
    pub runtime: Runtime,
    pub cam: FlyCamera,
    pub debug_stats: DebugStats,
    pub(crate) hotbar: Vec<Block>,
    pub leaves_shader: Option<LeavesShader>,
    pub fog_shader: Option<FogShader>,
    pub water_shader: Option<WaterShader>,
    pub tex_cache: TextureCache,
    pub renders: HashMap<ChunkCoord, ChunkRender>,
    pub structure_renders: HashMap<StructureId, ChunkRender>,
    pub minimap_rt: Option<RenderTexture2D>,
    pub reg: Arc<BlockRegistry>,
    pub(crate) evt_processed_total: usize,
    pub(crate) evt_processed_by: HashMap<String, usize>,
    pub(crate) intents: HashMap<ChunkCoord, IntentEntry>,
    pub(crate) perf_remove_start: HashMap<ChunkCoord, VecDeque<Instant>>,
    pub(crate) perf_mesh_ms: VecDeque<u32>,
    pub(crate) perf_light_ms: VecDeque<u32>,
    pub(crate) perf_total_ms: VecDeque<u32>,
    pub(crate) perf_remove_ms: VecDeque<u32>,
    pub(crate) tex_event_rx: Receiver<String>,
    pub(crate) worldgen_event_rx: Receiver<()>,
    pub(crate) world_config_path: String,
    pub rebuild_on_worldgen: bool,
    pub(crate) worldgen_dirty: bool,
    pub assets_root: PathBuf,
    pub(crate) reg_event_rx: Receiver<()>,
    pub(crate) shader_event_rx: Receiver<()>,
}

#[derive(Default)]
pub struct DebugStats {
    pub total_vertices: usize,
    pub total_triangles: usize,
    pub chunks_rendered: usize,
    pub chunks_culled: usize,
    pub structures_rendered: usize,
    pub structures_culled: usize,
    pub draw_calls: usize,
    pub queued_events_total: usize,
    pub queued_events_by: Vec<(String, usize)>,
    pub intents_size: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum IntentCause {
    Edit = 0,
    Light = 1,
    StreamLoad = 2,
    #[allow(dead_code)]
    HotReload = 3,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct IntentEntry {
    pub(crate) rev: u64,
    pub(crate) cause: IntentCause,
    pub(crate) last_tick: u64,
}

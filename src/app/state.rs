use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use geist_blocks::{Block, BlockRegistry};
use geist_render_raylib::{ChunkRender, FogShader, LeavesShader, TextureCache, WaterShader};
use geist_runtime::Runtime;
use geist_structures::StructureId;
use geist_world::{ChunkCoord, TERRAIN_STAGE_COUNT};
use raylib::prelude::{MouseButton, RenderTexture2D, Vector2, Vector3};

use crate::camera::FlyCamera;
use crate::event::EventQueue;
use crate::gamestate::GameState;

pub(crate) const STREAM_LOAD_SHELLS: i32 = 1;
pub(crate) const STREAM_EVICT_SHELLS: i32 = 2;

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
    pub minimap_zoom: f32,
    pub minimap_yaw: f32,
    pub minimap_pitch: f32,
    pub minimap_pan: Vector3,
    pub minimap_ui_rect: Option<(i32, i32, i32, i32)>,
    pub minimap_drag_button: Option<MouseButton>,
    pub minimap_drag_pan: bool,
    pub minimap_last_cursor: Option<Vector2>,
    pub event_histogram_pos: Vector2,
    pub event_histogram_dragging: bool,
    pub event_histogram_drag_offset: Vector2,
    pub event_histogram_rect: Option<(i32, i32, i32, i32)>,
    pub event_histogram_titlebar_rect: Option<(i32, i32, i32, i32)>,
    pub event_histogram_resize_rect: Option<(i32, i32, i32, i32)>,
    pub event_histogram_resizing: bool,
    pub event_histogram_resize_origin: Vector2,
    pub event_histogram_resize_start: (i32, i32),
    pub event_histogram_manual_size: Option<(i32, i32)>,
    pub event_histogram_min_size: (i32, i32),
    pub event_histogram_size: (i32, i32),
    pub intent_histogram_pos: Vector2,
    pub intent_histogram_dragging: bool,
    pub intent_histogram_drag_offset: Vector2,
    pub intent_histogram_rect: Option<(i32, i32, i32, i32)>,
    pub intent_histogram_titlebar_rect: Option<(i32, i32, i32, i32)>,
    pub intent_histogram_resize_rect: Option<(i32, i32, i32, i32)>,
    pub intent_histogram_resizing: bool,
    pub intent_histogram_resize_origin: Vector2,
    pub intent_histogram_resize_start: (i32, i32),
    pub intent_histogram_manual_size: Option<(i32, i32)>,
    pub intent_histogram_min_size: (i32, i32),
    pub intent_histogram_size: (i32, i32),
    pub terrain_histogram_pos: Vector2,
    pub terrain_histogram_dragging: bool,
    pub terrain_histogram_drag_offset: Vector2,
    pub terrain_histogram_rect: Option<(i32, i32, i32, i32)>,
    pub terrain_histogram_titlebar_rect: Option<(i32, i32, i32, i32)>,
    pub terrain_histogram_resize_rect: Option<(i32, i32, i32, i32)>,
    pub terrain_histogram_resizing: bool,
    pub terrain_histogram_resize_origin: Vector2,
    pub terrain_histogram_resize_start: (i32, i32),
    pub terrain_histogram_manual_size: Option<(i32, i32)>,
    pub terrain_histogram_min_size: (i32, i32),
    pub terrain_histogram_size: (i32, i32),
    pub reg: Arc<BlockRegistry>,
    pub(crate) evt_processed_total: usize,
    pub(crate) evt_processed_by: HashMap<String, usize>,
    pub(crate) intents: HashMap<ChunkCoord, IntentEntry>,
    pub(crate) perf_remove_start: HashMap<ChunkCoord, VecDeque<Instant>>,
    pub(crate) perf_mesh_ms: VecDeque<u32>,
    pub(crate) perf_light_ms: VecDeque<u32>,
    pub(crate) perf_total_ms: VecDeque<u32>,
    pub(crate) perf_remove_ms: VecDeque<u32>,
    pub(crate) perf_gen_ms: VecDeque<u32>,
    pub(crate) terrain_stage_us: [VecDeque<u32>; TERRAIN_STAGE_COUNT],
    pub(crate) terrain_stage_calls: [VecDeque<u32>; TERRAIN_STAGE_COUNT],
    pub(crate) terrain_height_tile_us: VecDeque<u32>,
    pub(crate) terrain_height_tile_reused: VecDeque<u32>,
    pub(crate) terrain_cache_hits: VecDeque<u32>,
    pub(crate) terrain_cache_misses: VecDeque<u32>,
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
    pub intents_by_cause: Vec<(String, usize)>,
    pub intents_by_radius: Vec<(String, usize)>,
    pub loaded_chunks: usize,
    pub chunk_resident_total: usize,
    pub chunk_resident_nonempty: usize,
    pub chunk_unique_cx: usize,
    pub chunk_unique_cy: usize,
    pub chunk_unique_cz: usize,
    pub render_cache_chunks: usize,
    pub lighting_border_chunks: usize,
    pub lighting_emitter_chunks: usize,
    pub lighting_micro_chunks: usize,
    pub edit_chunk_entries: usize,
    pub edit_block_edits: usize,
    pub edit_rev_entries: usize,
    pub edit_built_entries: usize,
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

impl App {
    #[inline]
    pub(crate) fn stream_base_radius(&self) -> i32 {
        self.gs.view_radius_chunks.max(0)
    }

    #[inline]
    pub(crate) fn stream_load_radius(&self) -> i32 {
        let load_shells = STREAM_LOAD_SHELLS.max(0);
        self.stream_base_radius().saturating_add(load_shells)
    }

    #[inline]
    pub(crate) fn stream_evict_radius(&self) -> i32 {
        let load_shells = STREAM_LOAD_SHELLS.max(0);
        let evict_shells = STREAM_EVICT_SHELLS.max(load_shells);
        self.stream_base_radius().saturating_add(evict_shells)
    }
}

use std::collections::HashMap;
use std::sync::Arc;

use crate::player::Walker;
use geist_blocks::types::Block;
use geist_chunk::{ChunkBuf, ChunkOccupancy};
use geist_edit::EditStore;
use geist_geom::Vec3;
use geist_lighting::LightingStore;
use geist_structures::{Structure, StructureId};
use geist_world::voxel::{ChunkCoord, World, generation::ChunkColumnProfile};
use log::warn;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkLifecycle {
    Loading,
    Ready,
}

impl ChunkLifecycle {
    #[inline]
    pub fn is_ready(self) -> bool {
        matches!(self, ChunkLifecycle::Ready)
    }
}

pub struct ChunkEntry {
    pub buf: Option<ChunkBuf>,
    occupancy: Option<ChunkOccupancy>,
    pub built_rev: u64,
    pub lifecycle: ChunkLifecycle,
    pub lighting_ready: bool,
    pub mesh_ready: bool,
    pub column_profile: Option<Arc<ChunkColumnProfile>>,
    pub column_profile_blob: Option<Vec<u8>>,
}

impl ChunkEntry {
    #[inline]
    pub fn loading() -> Self {
        Self {
            buf: None,
            occupancy: None,
            built_rev: 0,
            lifecycle: ChunkLifecycle::Loading,
            lighting_ready: false,
            mesh_ready: false,
            column_profile: None,
            column_profile_blob: None,
        }
    }

    #[inline]
    pub fn is_ready(&self) -> bool {
        self.lifecycle.is_ready()
    }

    #[inline]
    pub fn occupancy_or_empty(&self) -> ChunkOccupancy {
        self.occupancy.unwrap_or(ChunkOccupancy::Empty)
    }

    #[inline]
    pub fn has_blocks(&self) -> bool {
        self.occupancy
            .map(ChunkOccupancy::has_blocks)
            .unwrap_or(false)
    }

    #[inline]
    pub fn set_ready(
        &mut self,
        occ: ChunkOccupancy,
        buf: Option<ChunkBuf>,
        built_rev: u64,
        column_profile: Option<Arc<ChunkColumnProfile>>,
    ) {
        self.occupancy = Some(occ);
        self.buf = buf;
        self.built_rev = built_rev;
        self.lifecycle = ChunkLifecycle::Ready;
        self.column_profile_blob = column_profile.as_ref().map(|profile| profile.to_bytes());
        self.column_profile = column_profile;
    }
}

#[derive(Default)]
pub struct ChunkInventory {
    slots: HashMap<ChunkCoord, ChunkEntry>,
}

impl ChunkInventory {
    #[inline]
    pub fn ready_len(&self) -> usize {
        self.slots.values().filter(|entry| entry.is_ready()).count()
    }

    #[inline]
    pub fn is_ready(&self, coord: ChunkCoord) -> bool {
        self.slots
            .get(&coord)
            .map(|entry| entry.is_ready())
            .unwrap_or(false)
    }

    #[inline]
    pub fn get(&self, coord: &ChunkCoord) -> Option<&ChunkEntry> {
        self.slots.get(coord).filter(|entry| entry.is_ready())
    }

    #[inline]
    pub fn get_any_mut(&mut self, coord: &ChunkCoord) -> Option<&mut ChunkEntry> {
        self.slots.get_mut(coord)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&ChunkCoord, &ChunkEntry)> {
        self.slots.iter().filter(|(_, entry)| entry.is_ready())
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&ChunkCoord, &mut ChunkEntry)> {
        self.slots.iter_mut().filter(|(_, entry)| entry.is_ready())
    }

    pub fn coords_any(&self) -> impl Iterator<Item = ChunkCoord> + '_ {
        self.slots.keys().copied()
    }

    #[inline]
    pub fn mark_loading(&mut self, coord: ChunkCoord) -> &mut ChunkEntry {
        self.slots
            .entry(coord)
            .and_modify(|entry| {
                entry.lifecycle = ChunkLifecycle::Loading;
                entry.lighting_ready = false;
                entry.mesh_ready = false;
                entry.occupancy = None;
                entry.buf = None;
            })
            .or_insert_with(ChunkEntry::loading)
    }

    #[inline]
    pub fn mark_ready(
        &mut self,
        coord: ChunkCoord,
        occupancy: ChunkOccupancy,
        buf: Option<ChunkBuf>,
        built_rev: u64,
        column_profile: Option<Arc<ChunkColumnProfile>>,
    ) -> &mut ChunkEntry {
        let entry = self.slots.entry(coord).or_insert_with(ChunkEntry::loading);
        entry.set_ready(occupancy, buf, built_rev, column_profile);
        entry
    }

    pub fn column_profile(&mut self, coord: &ChunkCoord) -> Option<Arc<ChunkColumnProfile>> {
        let entry = self.slots.get_mut(coord)?;
        if let Some(profile) = entry.column_profile.as_ref() {
            return Some(Arc::clone(profile));
        }
        let blob = entry.column_profile_blob.as_ref()?;
        match ChunkColumnProfile::from_bytes(blob) {
            Ok(profile) => {
                let arc = Arc::new(profile);
                entry.column_profile = Some(Arc::clone(&arc));
                Some(arc)
            }
            Err(err) => {
                warn!(
                    "failed to deserialize column profile for chunk ({},{},{}): {}",
                    coord.cx, coord.cy, coord.cz, err
                );
                entry.column_profile_blob = None;
                None
            }
        }
    }

    pub fn clear_column_profile(&mut self, coord: &ChunkCoord) {
        if let Some(entry) = self.slots.get_mut(coord) {
            entry.column_profile = None;
            entry.column_profile_blob = None;
        }
    }

    #[inline]
    pub fn mark_missing(&mut self, coord: ChunkCoord) {
        self.slots.remove(&coord);
    }

    pub fn ready_coords(&self) -> impl Iterator<Item = ChunkCoord> + '_ {
        self.slots
            .iter()
            .filter_map(|(coord, entry)| entry.is_ready().then_some(*coord))
    }

    #[inline]
    pub fn mesh_ready(&self, coord: ChunkCoord) -> bool {
        self.slots
            .get(&coord)
            .map(|entry| entry.mesh_ready && entry.is_ready())
            .unwrap_or(false)
    }
}

#[derive(Default, Clone, Copy)]
pub struct FinalizeState {
    pub owner_neg_x_ready: bool, // neighbor (cx-1,cy,cz) published +X
    pub owner_neg_y_ready: bool, // neighbor (cx,cy-1,cz) published +Y
    pub owner_neg_z_ready: bool, // neighbor (cx,cy,cz-1) published +Z
    pub finalize_requested: bool,
    pub finalized: bool,
}

pub struct GameState {
    pub tick: u64,
    pub world: Arc<World>,

    // Streaming
    pub view_radius_chunks: i32,
    pub center_chunk: ChunkCoord,
    pub chunks: ChunkInventory,
    // How many times each chunk has completed meshing (by chunk coordinate)
    pub mesh_counts: HashMap<ChunkCoord, u32>,
    // How many times each chunk has completed a light-only recompute (no mesh)
    pub light_counts: HashMap<ChunkCoord, u32>,
    // Track newest rev sent to workers per chunk to avoid redundant requeues
    pub inflight_rev: HashMap<ChunkCoord, u64>,
    // Finalization tracking per chunk (no-timeout finalize after both owners publish)
    pub finalize: HashMap<ChunkCoord, FinalizeState>,

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
    pub show_debug_overlay: bool,

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
    /// Structure-local feet position recorded at the moment of attachment.
    pub local_offset: Vec3,
    /// Pose snapshot used to translate between structure-local and world space.
    pub pose_pos: Vec3,
    pub pose_yaw_deg: f32,
    /// Optional structure-local velocity inherited while attached (Phase 2 will populate).
    pub local_velocity: Option<Vec3>,
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
            center_chunk: ChunkCoord::new(i32::MIN, i32::MIN, i32::MIN),
            view_radius_chunks: 8,
            chunks: ChunkInventory::default(),
            mesh_counts: HashMap::new(),
            light_counts: HashMap::new(),
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
            show_debug_overlay: true,
            structures: HashMap::new(),
            ground_attach: None,
            structure_speed: 0.0,
            structure_elev_speed: 0.0,
        }
    }
}

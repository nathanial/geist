use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use fastnoise_lite::{FastNoiseLite, NoiseType};
use geist_blocks::registry::BlockRegistry;
use geist_blocks::types::Block as RtBlock;

use crate::worldgen::WorldGenParams;

use super::{CHUNK_SIZE, GenCtx};

pub struct World {
    pub chunk_size_x: usize,
    pub chunk_size_y: usize,
    pub chunk_size_z: usize,
    pub chunks_x: usize,
    pub chunks_y_hint: usize,
    pub chunks_z: usize,
    pub seed: i32,
    pub mode: WorldGenMode,
    pub gen_params: Arc<RwLock<Arc<WorldGenParams>>>,
    block_id_cache: RwLock<HashMap<String, u16>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorldGenMode {
    Normal,
    Flat { thickness: i32 },
}

impl World {
    pub fn new(
        chunks_x: usize,
        chunks_y_hint: usize,
        chunks_z: usize,
        seed: i32,
        mode: WorldGenMode,
    ) -> Self {
        Self {
            chunk_size_x: CHUNK_SIZE,
            chunk_size_y: CHUNK_SIZE,
            chunk_size_z: CHUNK_SIZE,
            chunks_x,
            chunks_y_hint,
            chunks_z,
            seed,
            mode,
            gen_params: Arc::new(RwLock::new(Arc::new(WorldGenParams::default()))),
            block_id_cache: RwLock::new(HashMap::new()),
        }
    }

    #[inline]
    pub fn world_size_x(&self) -> usize {
        self.chunk_size_x * self.chunks_x
    }

    #[inline]
    pub fn world_size_z(&self) -> usize {
        self.chunk_size_z * self.chunks_z
    }

    #[inline]
    pub fn chunk_stack_hint(&self) -> usize {
        self.chunks_y_hint
    }

    #[inline]
    pub fn world_height_hint(&self) -> usize {
        self.chunk_size_y * self.chunks_y_hint
    }

    pub(crate) fn resolve_block_id(&self, reg: &BlockRegistry, name: &str) -> u16 {
        if let Ok(cache) = self.block_id_cache.read() {
            if let Some(id) = cache.get(name) {
                return *id;
            }
        }

        let id = match reg.id_by_name(name) {
            Some(id) => id,
            None if name == "air" => 0,
            None => self.resolve_block_id(reg, "air"),
        };

        if let Ok(mut cache) = self.block_id_cache.write() {
            cache.entry(name.to_string()).or_insert(id);
        }
        id
    }

    pub(crate) fn air_block(&self, reg: &BlockRegistry) -> RtBlock {
        RtBlock {
            id: self.resolve_block_id(reg, "air"),
            state: 0,
        }
    }

    pub fn make_gen_ctx(&self) -> GenCtx {
        let params = {
            let guard = self.gen_params.read().unwrap();
            Arc::clone(&*guard)
        };
        let mut terrain = FastNoiseLite::with_seed(self.seed);
        terrain.set_noise_type(Some(NoiseType::OpenSimplex2));
        terrain.set_frequency(Some(params.height_frequency));
        let mut warp = FastNoiseLite::with_seed(self.seed ^ 99_173);
        warp.set_noise_type(Some(NoiseType::OpenSimplex2));
        warp.set_frequency(Some(0.012));
        let mut tunnel = FastNoiseLite::with_seed(self.seed ^ 41_337);
        tunnel.set_noise_type(Some(NoiseType::OpenSimplex2));
        tunnel.set_frequency(Some(0.017));
        let (temp2d, moist2d) = if let Some(b) = params.biomes.as_ref() {
            let b = &**b;
            let mut t = FastNoiseLite::with_seed(self.seed ^ 0x1203_5F31);
            t.set_noise_type(Some(NoiseType::OpenSimplex2));
            t.set_frequency(Some(b.temp_freq));
            let mut m = FastNoiseLite::with_seed(((self.seed as u32) ^ 0x92E3_A1B2u32) as i32);
            m.set_noise_type(Some(NoiseType::OpenSimplex2));
            m.set_frequency(Some(b.moisture_freq));
            (Some(t), Some(m))
        } else {
            (None, None)
        };
        GenCtx {
            terrain,
            warp,
            tunnel,
            params,
            temp2d,
            moist2d,
        }
    }

    pub fn update_worldgen_params(&self, params: WorldGenParams) {
        if let Ok(mut guard) = self.gen_params.write() {
            *guard = Arc::new(params);
        }
        if let Ok(mut ids) = self.block_id_cache.write() {
            ids.clear();
        }
    }

    #[inline]
    pub fn is_flat(&self) -> bool {
        matches!(self.mode, WorldGenMode::Flat { .. })
    }

    pub fn biome_at(&self, wx: i32, wz: i32) -> Option<crate::worldgen::BiomeDefParam> {
        let params = {
            let guard = self.gen_params.read().ok()?;
            Arc::clone(&*guard)
        };
        let biomes = params.biomes.as_ref()?.clone();
        let b = &*biomes;
        let mut t = FastNoiseLite::with_seed(self.seed ^ 0x1203_5F31);
        t.set_noise_type(Some(NoiseType::OpenSimplex2));
        t.set_frequency(Some(b.temp_freq));
        let mut m = FastNoiseLite::with_seed(((self.seed as u32) ^ 0x92E3_A1B2u32) as i32);
        m.set_noise_type(Some(NoiseType::OpenSimplex2));
        m.set_frequency(Some(b.moisture_freq));
        let sx = if b.scale_x == 0.0 { 1.0 } else { b.scale_x };
        let sz = if b.scale_z == 0.0 { 1.0 } else { b.scale_z };
        let x = wx as f32 * sx;
        let z = wz as f32 * sz;
        let temp = (t.get_noise_2d(x, z) * 0.5 + 0.5).clamp(0.0, 1.0);
        let moist = (m.get_noise_2d(x, z) * 0.5 + 0.5).clamp(0.0, 1.0);
        for def in &b.defs {
            if temp >= def.temp_min
                && temp < def.temp_max
                && moist >= def.moisture_min
                && moist < def.moisture_max
            {
                return Some(def.clone());
            }
        }
        None
    }
}

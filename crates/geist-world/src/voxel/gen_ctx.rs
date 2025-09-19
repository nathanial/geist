use std::sync::Arc;

use fastnoise_lite::FastNoiseLite;

use crate::worldgen::WorldGenParams;

pub struct GenCtx {
    pub terrain: FastNoiseLite,
    pub warp: FastNoiseLite,
    pub tunnel: FastNoiseLite,
    pub params: Arc<WorldGenParams>,
    pub temp2d: Option<FastNoiseLite>,
    pub moist2d: Option<FastNoiseLite>,
}

use std::time::Instant;

use super::super::gen_ctx::TerrainStage;
use super::column_sampler::ColumnSampler;

pub(super) fn apply_water_fill<'p>(
    sampler: &mut ColumnSampler<'_, 'p>,
    y: i32,
    water_level: i32,
    base: &mut &'p str,
) {
    sampler.profiler_mut().begin_stage(TerrainStage::Water);
    let stage_start = Instant::now();
    if *base == "air" && sampler.params.water_enable && y <= water_level {
        *base = "water";
    }
    sampler
        .profiler_mut()
        .record_stage_duration(TerrainStage::Water, stage_start.elapsed());
}

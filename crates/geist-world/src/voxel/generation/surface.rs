use std::time::Instant;

use super::super::gen_ctx::TerrainStage;
use super::column_sampler::ColumnSampler;

pub(super) fn select_surface_block<'p>(
    sampler: &mut ColumnSampler<'_, 'p>,
    x: i32,
    y: i32,
    z: i32,
    height: i32,
) -> &'p str {
    sampler.profiler_mut().begin_stage(TerrainStage::Surface);
    let stage_start = Instant::now();
    let block = if y >= height {
        "air"
    } else if y == height - 1 {
        sampler.top_block_for_column(x, z, height)
    } else if y + sampler.params.topsoil_thickness >= height {
        sampler.params.sub_near.as_str()
    } else {
        sampler.params.sub_deep.as_str()
    };
    sampler
        .profiler_mut()
        .record_stage_duration(TerrainStage::Surface, stage_start.elapsed());
    block
}

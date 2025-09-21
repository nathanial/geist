use raylib::prelude::Color;
use std::collections::VecDeque;

use super::super::{
    App, ContentLayout, DisplayLine, GeistDraw, WindowFrame, WindowTheme, draw_lines, format_count,
};

pub(crate) struct RuntimeStatsView {
    lines: Vec<DisplayLine>,
    subtitle: Option<String>,
}

impl RuntimeStatsView {
    const MIN_WIDTH: i32 = 420;

    pub(crate) fn new(app: &App) -> Self {
        let mut lines = Vec::new();
        lines.push(
            DisplayLine::new(
                format!(
                    "Processed events: {}",
                    format_count(app.evt_processed_total)
                ),
                18,
                Color::new(230, 238, 255, 255),
            )
            .with_line_height(24),
        );
        lines.push(DisplayLine::new(
            format!(
                "Intents queued: {}",
                format_count(app.debug_stats.intents_size)
            ),
            16,
            Color::new(204, 216, 236, 255),
        ));
        lines.push(DisplayLine::new(
            "Lighting mode: FullMicro".to_string(),
            15,
            Color::new(176, 192, 214, 255),
        ));

        let (q_e, if_e, q_l, if_l, q_b, if_b) = app.runtime.queue_debug_counts();
        lines.push(
            DisplayLine::new("Runtime queues", 17, Color::new(214, 226, 246, 255))
                .with_line_height(22),
        );
        lines.push(
            DisplayLine::new(
                format!("Edit: queued {} | inflight {}", q_e, if_e),
                15,
                Color::new(186, 200, 222, 255),
            )
            .with_indent(18),
        );
        lines.push(
            DisplayLine::new(
                format!("Light: queued {} | inflight {}", q_l, if_l),
                15,
                Color::new(186, 200, 222, 255),
            )
            .with_indent(18),
        );
        lines.push(
            DisplayLine::new(
                format!("Background: queued {} | inflight {}", q_b, if_b),
                15,
                Color::new(186, 200, 222, 255),
            )
            .with_indent(18),
        );

        lines.push(
            DisplayLine::new("Chunk residency", 17, Color::new(214, 226, 246, 255))
                .with_line_height(22),
        );
        lines.push(
            DisplayLine::new(
                format!(
                    "Loaded {} | active {} | nonempty {}",
                    format_count(app.debug_stats.loaded_chunks),
                    format_count(app.debug_stats.chunk_resident_total),
                    format_count(app.debug_stats.chunk_resident_nonempty)
                ),
                15,
                Color::new(188, 202, 226, 255),
            )
            .with_indent(18),
        );
        lines.push(
            DisplayLine::new(
                format!(
                    "Unique axes: x {} y {} z {}",
                    format_count(app.debug_stats.chunk_unique_cx),
                    format_count(app.debug_stats.chunk_unique_cy),
                    format_count(app.debug_stats.chunk_unique_cz)
                ),
                15,
                Color::new(188, 202, 226, 255),
            )
            .with_indent(18),
        );
        lines.push(
            DisplayLine::new(
                format!(
                    "GPU renders cached: {}",
                    format_count(app.debug_stats.render_cache_chunks)
                ),
                15,
                Color::new(188, 202, 226, 255),
            )
            .with_indent(18),
        );

        lines.push(
            DisplayLine::new("Lighting store", 17, Color::new(214, 226, 246, 255))
                .with_line_height(22),
        );
        lines.push(
            DisplayLine::new(
                format!(
                    "Borders {} | Emitters {} | Micro {}",
                    format_count(app.debug_stats.lighting_border_chunks),
                    format_count(app.debug_stats.lighting_emitter_chunks),
                    format_count(app.debug_stats.lighting_micro_chunks)
                ),
                15,
                Color::new(180, 196, 222, 255),
            )
            .with_indent(18),
        );

        lines.push(
            DisplayLine::new("Edit store", 17, Color::new(214, 226, 246, 255)).with_line_height(22),
        );
        lines.push(
            DisplayLine::new(
                format!(
                    "Chunks {} | Blocks {} | Rev {} | Built {}",
                    format_count(app.debug_stats.edit_chunk_entries),
                    format_count(app.debug_stats.edit_block_edits),
                    format_count(app.debug_stats.edit_rev_entries),
                    format_count(app.debug_stats.edit_built_entries)
                ),
                15,
                Color::new(180, 196, 222, 255),
            )
            .with_indent(18),
        );

        lines.push(
            DisplayLine::new("Perf (ms)", 17, Color::new(214, 226, 246, 255)).with_line_height(22),
        );
        let summary = |q: &VecDeque<u32>| -> (usize, u32, u32) {
            let n = q.len();
            if n == 0 {
                return (0, 0, 0);
            }
            let sum: u64 = q.iter().map(|&v| v as u64).sum();
            let avg = ((sum as f32) / (n as f32)).round() as u32;
            let mut values: Vec<u32> = q.iter().copied().collect();
            values.sort_unstable();
            let idx = ((n as f32) * 0.95).ceil().max(1.0) as usize - 1;
            let p95 = values[idx.min(n - 1)];
            (n, avg, p95)
        };
        let (n_mesh, avg_mesh, p95_mesh) = summary(&app.perf_mesh_ms);
        let (n_light, avg_light, p95_light) = summary(&app.perf_light_ms);
        let (n_total, avg_total, p95_total) = summary(&app.perf_total_ms);
        let (n_rr, avg_rr, p95_rr) = summary(&app.perf_remove_ms);
        let (n_gen, avg_gen, p95_gen) = summary(&app.perf_gen_ms);
        let last_gen = app.perf_gen_ms.back().copied().unwrap_or(0);

        let perf_lines = [
            (
                "Mesh",
                avg_mesh,
                p95_mesh,
                n_mesh,
                Some(app.perf_mesh_ms.back().copied().unwrap_or(0)),
            ),
            (
                "Light",
                avg_light,
                p95_light,
                n_light,
                Some(app.perf_light_ms.back().copied().unwrap_or(0)),
            ),
            ("Total", avg_total, p95_total, n_total, None),
            ("Remove->Render", avg_rr, p95_rr, n_rr, None),
            ("Load", avg_gen, p95_gen, n_gen, Some(last_gen)),
        ];

        for (label, avg, p95, n, last) in perf_lines {
            let text = if let Some(last_ms) = last {
                format!(
                    "{}: last {} | avg {} | p95 {} | n {}",
                    label, last_ms, avg, p95, n
                )
            } else {
                format!("{}: avg {} | p95 {} | n {}", label, avg, p95, n)
            };
            lines.push(DisplayLine::new(text, 15, Color::new(172, 190, 218, 255)).with_indent(18));
        }

        let total_queue = q_e + q_l + q_b;
        let subtitle = Some(format!(
            "queues {} | inflight {}",
            total_queue,
            if_e + if_l + if_b
        ));

        Self { lines, subtitle }
    }

    pub(crate) fn min_size(&self, theme: &WindowTheme) -> (i32, i32) {
        let height: i32 = self.lines.iter().map(|line| line.line_height).sum();
        let min_height = theme.titlebar_height + height + theme.padding_y * 2;
        let h = min_height.max(theme.titlebar_height + theme.padding_y * 2 + 220);
        let w = theme.padding_x * 2 + Self::MIN_WIDTH;
        (w, h)
    }

    pub(crate) fn subtitle(&self) -> Option<&str> {
        self.subtitle.as_deref()
    }

    pub(crate) fn draw(&self, d: &mut GeistDraw, frame: &WindowFrame) -> ContentLayout {
        draw_lines(d, &self.lines, frame)
    }
}

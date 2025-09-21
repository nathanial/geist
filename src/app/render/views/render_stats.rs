use raylib::prelude::Color;

use super::super::{
    App, ContentLayout, DisplayLine, GeistDraw, WindowFrame, WindowTheme, draw_lines, format_count,
};

pub(crate) struct RenderStatsView {
    lines: Vec<DisplayLine>,
    subtitle: Option<String>,
}

impl RenderStatsView {
    const MIN_WIDTH: i32 = 340;

    pub(crate) fn new(app: &App, fps: u32) -> Self {
        let mut lines = Vec::new();
        lines.push(
            DisplayLine::new(format!("FPS: {}", fps), 20, Color::new(236, 244, 255, 255))
                .with_line_height(26),
        );
        lines.push(DisplayLine::new(
            format!("Vertices: {}", format_count(app.debug_stats.total_vertices)),
            16,
            Color::new(206, 220, 240, 255),
        ));
        lines.push(DisplayLine::new(
            format!(
                "Triangles: {}",
                format_count(app.debug_stats.total_triangles)
            ),
            16,
            Color::new(206, 220, 240, 255),
        ));
        lines.push(DisplayLine::new(
            format!(
                "Chunks rendered: {} (culled {})",
                format_count(app.debug_stats.chunks_rendered),
                format_count(app.debug_stats.chunks_culled)
            ),
            16,
            Color::new(190, 204, 226, 255),
        ));
        lines.push(DisplayLine::new(
            format!(
                "Structures rendered: {} (culled {})",
                format_count(app.debug_stats.structures_rendered),
                format_count(app.debug_stats.structures_culled)
            ),
            16,
            Color::new(190, 204, 226, 255),
        ));
        lines.push(DisplayLine::new(
            format!("Draw calls: {}", format_count(app.debug_stats.draw_calls)),
            16,
            Color::new(206, 220, 240, 255),
        ));
        let center = app.gs.center_chunk;
        lines.push(DisplayLine::new(
            format!(
                "Center chunk: ({}, {}, {})",
                center.cx, center.cy, center.cz
            ),
            16,
            Color::new(188, 198, 214, 255),
        ));
        if app.gs.show_biome_label {
            let wx = app.cam.position.x.floor() as i32;
            let wz = app.cam.position.z.floor() as i32;
            if let Some(biome) = app.gs.world.biome_at(wx, wz) {
                lines.push(DisplayLine::new(
                    format!("Biome: {}", biome.name),
                    16,
                    Color::new(188, 198, 214, 255),
                ));
            }
        }

        Self {
            lines,
            subtitle: Some(format!("fps {}", fps)),
        }
    }

    pub(crate) fn min_size(&self, theme: &WindowTheme) -> (i32, i32) {
        let height: i32 = self.lines.iter().map(|line| line.line_height).sum();
        let min_height = theme.titlebar_height + height + theme.padding_y * 2;
        let h = min_height.max(theme.titlebar_height + theme.padding_y * 2 + 160);
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

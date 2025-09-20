use raylib::prelude::*;
use std::collections::{HashSet, VecDeque};

use super::{App, DebugStats, IRect, WindowChrome, WindowFrame, WindowId, WindowTheme};

pub(super) const MINIMAP_MIN_CONTENT_SIDE: i32 = 200;
pub(super) const MINIMAP_MAX_CONTENT_SIDE: i32 = 420;
pub(super) const MINIMAP_BORDER_PX: i32 = 10;
use crate::raycast;
use geist_blocks::Block;
use geist_chunk::ChunkOccupancy;
use geist_geom::Vec3;
use geist_render_raylib::conv::{vec3_from_rl, vec3_to_rl};
use geist_structures::{StructureId, rotate_yaw_inv};
use geist_world::{ChunkCoord, TERRAIN_STAGE_COUNT, TERRAIN_STAGE_LABELS};

fn format_count(count: usize) -> String {
    match count {
        0..=999 => count.to_string(),
        1_000..=9_999 => format!("{:.1}k", count as f32 / 1_000.0),
        10_000..=999_999 => format!("{}k", count / 1_000),
        1_000_000..=9_999_999 => format!("{:.1}M", count as f32 / 1_000_000.0),
        _ => format!("{}M", count / 1_000_000),
    }
}

#[derive(Default, Debug, Clone, Copy)]
struct ContentLayout {
    available_height: i32,
    used_height: i32,
    overflow_rows: usize,
    overflow_items: usize,
}

impl ContentLayout {
    fn new(available_height: i32) -> Self {
        Self {
            available_height,
            ..Default::default()
        }
    }

    fn add_rows(&mut self, rows: usize, row_height: i32) {
        self.used_height += (rows as i32) * row_height;
    }

    fn add_custom(&mut self, height: i32) {
        self.used_height += height.max(0);
    }

    fn mark_overflow(&mut self, rows: usize, items: usize) {
        self.overflow_rows += rows;
        self.overflow_items += items;
    }

    fn overflow(&self) -> bool {
        self.used_height > self.available_height || self.overflow_items > 0
    }
}

#[derive(Debug, Clone)]
struct DisplayLine {
    text: String,
    color: Color,
    font: i32,
    line_height: i32,
    indent: i32,
}

impl DisplayLine {
    fn new(text: impl Into<String>, font: i32, color: Color) -> Self {
        let font = font.max(1);
        Self {
            text: text.into(),
            color,
            font,
            line_height: font + 4,
            indent: 0,
        }
    }

    fn with_indent(mut self, indent: i32) -> Self {
        self.indent = indent.max(0);
        self
    }

    fn with_line_height(mut self, line_height: i32) -> Self {
        self.line_height = line_height.max(self.font);
        self
    }
}

fn draw_lines(
    d: &mut RaylibDrawHandle,
    lines: &[DisplayLine],
    frame: &WindowFrame,
) -> ContentLayout {
    let content = frame.content;
    let mut layout = ContentLayout::new(content.h);
    if content.h <= 0 {
        return layout;
    }
    let mut y = content.y;
    for (idx, line) in lines.iter().enumerate() {
        let next_y = y + line.line_height;
        layout.add_custom(line.line_height);
        if next_y - content.y > content.h {
            let remaining = lines.len().saturating_sub(idx);
            if remaining > 0 {
                layout.mark_overflow(remaining, remaining);
            }
            break;
        }
        if !line.text.is_empty() {
            d.draw_text(
                &line.text,
                content.x + line.indent,
                y,
                line.font,
                line.color,
            );
        }
        y = next_y;
    }
    layout
}

struct RenderStatsView {
    lines: Vec<DisplayLine>,
    subtitle: Option<String>,
}

impl RenderStatsView {
    const MIN_WIDTH: i32 = 340;

    fn new(app: &App, fps: u32) -> Self {
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

    fn min_size(&self, theme: &WindowTheme) -> (i32, i32) {
        let height: i32 = self.lines.iter().map(|line| line.line_height).sum();
        let min_height = theme.titlebar_height + height + theme.padding_y * 2;
        let h = min_height.max(theme.titlebar_height + theme.padding_y * 2 + 160);
        let w = theme.padding_x * 2 + Self::MIN_WIDTH;
        (w, h)
    }

    fn subtitle(&self) -> Option<&str> {
        self.subtitle.as_deref()
    }

    fn draw(&self, d: &mut RaylibDrawHandle, frame: &WindowFrame) -> ContentLayout {
        draw_lines(d, &self.lines, frame)
    }
}

struct RuntimeStatsView {
    lines: Vec<DisplayLine>,
    subtitle: Option<String>,
}

impl RuntimeStatsView {
    const MIN_WIDTH: i32 = 420;

    fn new(app: &App) -> Self {
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

    fn min_size(&self, theme: &WindowTheme) -> (i32, i32) {
        let height: i32 = self.lines.iter().map(|line| line.line_height).sum();
        let min_height = theme.titlebar_height + height + theme.padding_y * 2;
        let h = min_height.max(theme.titlebar_height + theme.padding_y * 2 + 220);
        let w = theme.padding_x * 2 + Self::MIN_WIDTH;
        (w, h)
    }

    fn subtitle(&self) -> Option<&str> {
        self.subtitle.as_deref()
    }

    fn draw(&self, d: &mut RaylibDrawHandle, frame: &WindowFrame) -> ContentLayout {
        draw_lines(d, &self.lines, frame)
    }
}

struct AttachmentDebugView {
    lines: Vec<DisplayLine>,
}

impl AttachmentDebugView {
    const MIN_WIDTH: i32 = 520;

    fn new(app: &App) -> Self {
        let mut lines = Vec::new();
        if let Some(att) = app.gs.ground_attach {
            lines.push(
                DisplayLine::new(
                    format!("Attached to structure ID {}", att.id),
                    16,
                    Color::GREEN,
                )
                .with_line_height(20),
            );
            lines.push(
                DisplayLine::new(
                    format!("Grace period: {}", att.grace),
                    15,
                    Color::new(156, 212, 178, 255),
                )
                .with_indent(18),
            );
        } else {
            lines.push(DisplayLine::new("Not attached", 16, Color::ORANGE).with_line_height(20));
        }

        let pos = app.gs.walker.pos;
        lines.push(DisplayLine::new(
            format!("Walker pos: ({:.2}, {:.2}, {:.2})", pos.x, pos.y, pos.z),
            15,
            Color::new(210, 220, 238, 255),
        ));
        lines.push(DisplayLine::new(
            format!("On ground: {}", app.gs.walker.on_ground),
            15,
            Color::new(210, 220, 238, 255),
        ));

        if app.gs.structures.is_empty() {
            lines.push(DisplayLine::new(
                "No active structures",
                15,
                Color::new(160, 172, 190, 255),
            ));
        }

        for (id, st) in &app.gs.structures {
            let on_structure = app.is_feet_on_structure(st, app.gs.walker.pos);
            let color = if on_structure {
                Color::GREEN
            } else {
                Color::GRAY
            };
            lines.push(
                DisplayLine::new(
                    format!(
                        "Structure {}: on={} pos=({:.1},{:.1},{:.1}) delta=({:.3},{:.3},{:.3})",
                        id,
                        on_structure,
                        st.pose.pos.x,
                        st.pose.pos.y,
                        st.pose.pos.z,
                        st.last_delta.x,
                        st.last_delta.y,
                        st.last_delta.z
                    ),
                    15,
                    color,
                )
                .with_line_height(20),
            );

            let walker = vec3_from_rl(app.gs.walker.pos);
            let diff = Vec3 {
                x: walker.x - st.pose.pos.x,
                y: walker.y - st.pose.pos.y,
                z: walker.z - st.pose.pos.z,
            };
            let local = rotate_yaw_inv(diff, st.pose.yaw_deg);
            let test_y = local.y - 0.08;
            let lx = local.x.floor() as i32;
            let ly = test_y.floor() as i32;
            let lz = local.z.floor() as i32;
            lines.push(
                DisplayLine::new(
                    format!(
                        "Local: ({:.2}, {:.2}, {:.2}) test_y {:.2} -> grid ({}, {}, {})",
                        local.x, local.y, local.z, test_y, lx, ly, lz
                    ),
                    14,
                    color,
                )
                .with_indent(20)
                .with_line_height(18),
            );

            let in_bounds = lx >= 0
                && ly >= 0
                && lz >= 0
                && (lx as usize) < st.sx
                && (ly as usize) < st.sy
                && (lz as usize) < st.sz;

            let (block_at_pos, block_solid) = if in_bounds {
                if let Some(b) = st.edits.get(lx, ly, lz) {
                    (
                        format!("id:{} state:{} (edit)", b.id, b.state),
                        app.reg
                            .get(b.id)
                            .map(|ty| ty.is_solid(b.state))
                            .unwrap_or(false),
                    )
                } else {
                    let idx = st.idx(lx as usize, ly as usize, lz as usize);
                    let b = st.blocks[idx];
                    (
                        format!("id:{} state:{}", b.id, b.state),
                        app.reg
                            .get(b.id)
                            .map(|ty| ty.is_solid(b.state))
                            .unwrap_or(false),
                    )
                }
            } else {
                ("out of bounds".to_string(), false)
            };

            lines.push(
                DisplayLine::new(
                    format!(
                        "Bounds: 0..{} x 0..{} x 0..{} | in bounds {}",
                        st.sx, st.sy, st.sz, in_bounds
                    ),
                    14,
                    color,
                )
                .with_indent(20)
                .with_line_height(18),
            );
            lines.push(
                DisplayLine::new(
                    format!(
                        "Block at ({},{},{}): {} | solid {}",
                        lx, ly, lz, block_at_pos, block_solid
                    ),
                    14,
                    color,
                )
                .with_indent(20)
                .with_line_height(18),
            );

            if ly > 0 {
                let by = ly - 1;
                let (block_below, solid_below) = if lx >= 0
                    && by >= 0
                    && lz >= 0
                    && (lx as usize) < st.sx
                    && (by as usize) < st.sy
                    && (lz as usize) < st.sz
                {
                    if let Some(b) = st.edits.get(lx, by, lz) {
                        (
                            format!("id:{} state:{} (edit)", b.id, b.state),
                            app.reg
                                .get(b.id)
                                .map(|ty| ty.is_solid(b.state))
                                .unwrap_or(false),
                        )
                    } else {
                        let idx = st.idx(lx as usize, by as usize, lz as usize);
                        let b = st.blocks[idx];
                        (
                            format!("id:{} state:{}", b.id, b.state),
                            app.reg
                                .get(b.id)
                                .map(|ty| ty.is_solid(b.state))
                                .unwrap_or(false),
                        )
                    }
                } else {
                    ("out of bounds".to_string(), false)
                };
                lines.push(
                    DisplayLine::new(
                        format!(
                            "Block below ({},{},{}): {} | solid {}",
                            lx, by, lz, block_below, solid_below
                        ),
                        14,
                        color,
                    )
                    .with_indent(20)
                    .with_line_height(18),
                );
            }

            let deck_y = (st.sy as f32 * 0.33) as i32;
            lines.push(
                DisplayLine::new(
                    format!("Deck Y level: {} (expect solid blocks)", deck_y),
                    14,
                    Color::BLUE,
                )
                .with_indent(20)
                .with_line_height(18),
            );

            if lx >= 0 && lz >= 0 && (lx as usize) < st.sx && (lz as usize) < st.sz {
                if deck_y >= 0 && (deck_y as usize) < st.sy {
                    let deck_idx = st.idx(lx as usize, deck_y as usize, lz as usize);
                    let deck_block = st.blocks[deck_idx];
                    lines.push(
                        DisplayLine::new(
                            format!("Block at deck ({},{},{}): {:?}", lx, deck_y, lz, deck_block),
                            14,
                            Color::MAGENTA,
                        )
                        .with_indent(20)
                        .with_line_height(18),
                    );
                }
            }
        }

        Self { lines }
    }

    fn min_size(&self, theme: &WindowTheme) -> (i32, i32) {
        let height: i32 = self.lines.iter().map(|line| line.line_height).sum();
        let min_height = theme.titlebar_height + height + theme.padding_y * 2;
        let h = min_height.max(theme.titlebar_height + theme.padding_y * 2 + 240);
        let w = theme.padding_x * 2 + Self::MIN_WIDTH;
        (w, h)
    }

    fn draw(&self, d: &mut RaylibDrawHandle, frame: &WindowFrame) -> ContentLayout {
        draw_lines(d, &self.lines, frame)
    }
}

struct EventHistogramView<'a> {
    total: usize,
    entries: &'a [(String, usize)],
}

struct IntentHistogramView<'a> {
    total: usize,
    by_cause: &'a [(String, usize)],
    by_radius: &'a [(String, usize)],
}

struct TerrainHistogramView<'a> {
    stage_us: &'a [VecDeque<u32>; TERRAIN_STAGE_COUNT],
    stage_calls: &'a [VecDeque<u32>; TERRAIN_STAGE_COUNT],
    height_tile_us: &'a VecDeque<u32>,
    height_tile_reused: &'a VecDeque<u32>,
    cache_hits: &'a VecDeque<u32>,
    cache_misses: &'a VecDeque<u32>,
}

impl<'a> TerrainHistogramView<'a> {
    const ROW_HEIGHT: i32 = 26;
    const LABEL_WIDTH: i32 = 200;
    const GAP_X: i32 = 14;
    const MIN_BAR_WIDTH: i32 = 280;
    const SUMMARY_CARD_HEIGHT: i32 = 78;
    const SUMMARY_GAP: i32 = 18;
    const DEFAULT_MIN_HEIGHT: i32 = 360;
    const ROW_FONT: i32 = 16;
    const SUBTITLE_FONT: i32 = 16;
    const CARD_HEADER_FONT: i32 = 18;

    fn new(
        stage_us: &'a [VecDeque<u32>; TERRAIN_STAGE_COUNT],
        stage_calls: &'a [VecDeque<u32>; TERRAIN_STAGE_COUNT],
        height_tile_us: &'a VecDeque<u32>,
        height_tile_reused: &'a VecDeque<u32>,
        cache_hits: &'a VecDeque<u32>,
        cache_misses: &'a VecDeque<u32>,
    ) -> Self {
        Self {
            stage_us,
            stage_calls,
            height_tile_us,
            height_tile_reused,
            cache_hits,
            cache_misses,
        }
    }

    fn sample_window(&self) -> usize {
        self.stage_us
            .get(0)
            .map(|q| q.len())
            .unwrap_or_default()
            .max(self.height_tile_us.len())
    }

    fn min_size(&self, theme: &WindowTheme) -> (i32, i32) {
        let stage_rows = TERRAIN_STAGE_COUNT as i32;
        let mut min_height = theme.titlebar_height
            + theme.padding_y * 2
            + Self::SUMMARY_CARD_HEIGHT
            + Self::SUMMARY_GAP
            + stage_rows * Self::ROW_HEIGHT;
        min_height = min_height.max(Self::DEFAULT_MIN_HEIGHT);
        let min_width = theme.padding_x * 2 + Self::LABEL_WIDTH + Self::GAP_X + Self::MIN_BAR_WIDTH;
        (min_width, min_height)
    }

    fn subtitle(&self) -> Option<String> {
        let window = self.sample_window();
        if window == 0 {
            None
        } else {
            Some(format!("{} samples", window))
        }
    }

    fn draw(
        &self,
        d: &mut RaylibDrawHandle,
        frame: &WindowFrame,
        theme: &WindowTheme,
    ) -> Option<ContentLayout> {
        let window = self.sample_window();
        if window == 0 {
            return None;
        }

        let content = frame.content;
        let pad_x = theme.padding_x;

        let last_tile_us = self.height_tile_us.back().copied().unwrap_or(0);
        let last_tile_text = if self.height_tile_reused.back().copied().unwrap_or(0) == 1 {
            "reused".to_string()
        } else {
            format!("{:.2}ms", last_tile_us as f32 / 1000.0)
        };

        let tile_builds: Vec<u32> = self
            .height_tile_us
            .iter()
            .copied()
            .filter(|&v| v > 0)
            .collect();
        let (tile_avg_ms, tile_p95_ms) = if tile_builds.is_empty() {
            (0.0, 0.0)
        } else {
            let mut sorted = tile_builds.clone();
            sorted.sort_unstable();
            let sum: u64 = sorted.iter().map(|&v| v as u64).sum();
            let avg = (sum as f32 / sorted.len() as f32) / 1000.0;
            let p95_idx =
                ((sorted.len() as f32 * 0.95).ceil().max(1.0) as usize - 1).min(sorted.len() - 1);
            let p95 = sorted[p95_idx] as f32 / 1000.0;
            (avg, p95)
        };

        let reuse_total: u32 = self.height_tile_reused.iter().copied().sum();
        let reuse_ratio = if self.height_tile_reused.is_empty() {
            0.0
        } else {
            (reuse_total as f32 / self.height_tile_reused.len() as f32) * 100.0
        };

        let total_hits: u64 = self.cache_hits.iter().map(|&v| v as u64).sum();
        let total_miss: u64 = self.cache_misses.iter().map(|&v| v as u64).sum();
        let total_cache = total_hits + total_miss;
        let avg_cache_rate = if total_cache == 0 {
            0.0
        } else {
            (total_hits as f64 / total_cache as f64 * 100.0) as f32
        };
        let last_hits = self.cache_hits.back().copied().unwrap_or(0);
        let last_miss = self.cache_misses.back().copied().unwrap_or(0);
        let last_cache = last_hits + last_miss;
        let last_cache_rate = if last_cache == 0 {
            0.0
        } else {
            (last_hits as f32 / last_cache as f32) * 100.0
        };

        #[derive(Clone, Copy)]
        struct StageRowData {
            avg_ms: f32,
            p95_ms: f32,
            last_ms: f32,
            avg_calls: f32,
            last_calls: u32,
        }

        let mut rows = [StageRowData {
            avg_ms: 0.0,
            p95_ms: 0.0,
            last_ms: 0.0,
            avg_calls: 0.0,
            last_calls: 0,
        }; TERRAIN_STAGE_COUNT];
        let mut max_span_ms = 0.0_f32;
        for idx in 0..TERRAIN_STAGE_COUNT {
            let durations = &self.stage_us[idx];
            if durations.is_empty() {
                continue;
            }
            let mut sorted: Vec<u32> = durations.iter().copied().collect();
            sorted.sort_unstable();
            let sum: u64 = durations.iter().map(|&v| v as u64).sum();
            let avg_ms = (sum as f32 / durations.len() as f32) / 1000.0;
            let p95_idx = ((durations.len() as f32 * 0.95).ceil().max(1.0) as usize - 1)
                .min(durations.len() - 1);
            let p95_ms = sorted[p95_idx] as f32 / 1000.0;
            let last_ms = durations.back().copied().unwrap_or(0) as f32 / 1000.0;
            let calls = &self.stage_calls[idx];
            let (avg_calls, last_calls) = if calls.is_empty() {
                (0.0, 0)
            } else {
                let sum: u64 = calls.iter().map(|&v| v as u64).sum();
                let avg = sum as f32 / calls.len() as f32;
                let last = calls.back().copied().unwrap_or(0);
                (avg, last)
            };
            rows[idx] = StageRowData {
                avg_ms,
                p95_ms,
                last_ms,
                avg_calls,
                last_calls,
            };
            max_span_ms = max_span_ms.max(p95_ms.max(avg_ms));
        }

        let mut layout = ContentLayout::new(content.h);
        let mut cursor_y = content.y;

        let card_gap = 14;
        let card_width = ((frame.outer.w - pad_x * 2 - card_gap) / 2).max(160);
        let card_height = Self::SUMMARY_CARD_HEIGHT;
        let card_bg = Color::new(24, 32, 44, 235);
        let card_outline = Color::new(52, 68, 84, 200);
        let accent_tile = Color::new(118, 202, 255, 220);
        let accent_cache = Color::new(124, 220, 184, 220);

        let card1_x = content.x;
        let card2_x = card1_x + card_width + card_gap;

        d.draw_rectangle(card1_x, cursor_y, card_width, card_height, card_bg);
        d.draw_rectangle_lines(card1_x, cursor_y, card_width, card_height, card_outline);
        d.draw_rectangle(card1_x, cursor_y, card_width, 2, accent_tile);
        let text_x = card1_x + 12;
        let mut text_y = cursor_y + 10;
        d.draw_text(
            "Height Tiles",
            text_x,
            text_y,
            Self::CARD_HEADER_FONT,
            Color::new(228, 238, 255, 255),
        );
        text_y += Self::CARD_HEADER_FONT + 4;
        d.draw_text(
            &format!("last: {}", last_tile_text),
            text_x,
            text_y,
            Self::SUBTITLE_FONT,
            Color::new(210, 220, 238, 255),
        );
        text_y += Self::SUBTITLE_FONT + 2;
        d.draw_text(
            &format!("avg: {:.2}ms   p95: {:.2}ms", tile_avg_ms, tile_p95_ms),
            text_x,
            text_y,
            Self::SUBTITLE_FONT,
            Color::new(198, 208, 230, 255),
        );
        text_y += Self::SUBTITLE_FONT + 2;
        d.draw_text(
            &format!("reuse: {:.0}%", reuse_ratio),
            text_x,
            text_y,
            Self::SUBTITLE_FONT,
            Color::new(200, 214, 242, 255),
        );
        text_y += Self::SUBTITLE_FONT + 4;
        let reuse_bar_x = text_x;
        let reuse_bar_width = card_width - 24;
        let reuse_bar_height = 10;
        d.draw_rectangle(
            reuse_bar_x,
            text_y,
            reuse_bar_width,
            reuse_bar_height,
            Color::new(18, 24, 34, 255),
        );
        let reuse_fill =
            ((reuse_ratio / 100.0).clamp(0.0, 1.0) * reuse_bar_width as f32).round() as i32;
        if reuse_fill > 0 {
            d.draw_rectangle(
                reuse_bar_x,
                text_y,
                reuse_fill.max(2),
                reuse_bar_height,
                accent_tile,
            );
        }

        d.draw_rectangle(card2_x, cursor_y, card_width, card_height, card_bg);
        d.draw_rectangle_lines(card2_x, cursor_y, card_width, card_height, card_outline);
        d.draw_rectangle(card2_x, cursor_y, card_width, 2, accent_cache);
        let text_x2 = card2_x + 12;
        let mut text_y2 = cursor_y + 10;
        d.draw_text(
            "Height Cache",
            text_x2,
            text_y2,
            Self::CARD_HEADER_FONT,
            Color::new(224, 244, 236, 255),
        );
        text_y2 += Self::CARD_HEADER_FONT + 4;
        d.draw_text(
            &format!(
                "avg hit: {:.0}%   last: {:.0}%",
                avg_cache_rate, last_cache_rate
            ),
            text_x2,
            text_y2,
            Self::SUBTITLE_FONT,
            Color::new(204, 220, 214, 255),
        );
        text_y2 += Self::SUBTITLE_FONT + 2;
        d.draw_text(
            &format!(
                "hits: {}   total: {}",
                format_count(total_hits as usize),
                format_count(total_cache as usize)
            ),
            text_x2,
            text_y2,
            Self::SUBTITLE_FONT,
            Color::new(190, 206, 204, 255),
        );
        text_y2 += Self::SUBTITLE_FONT + 4;
        let cache_bar_x = text_x2;
        let cache_bar_width = card_width - 24;
        let cache_bar_height = 10;
        d.draw_rectangle(
            cache_bar_x,
            text_y2,
            cache_bar_width,
            cache_bar_height,
            Color::new(18, 26, 34, 255),
        );
        let cache_fill =
            ((last_cache_rate / 100.0).clamp(0.0, 1.0) * cache_bar_width as f32).round() as i32;
        if cache_fill > 0 {
            d.draw_rectangle(
                cache_bar_x,
                text_y2,
                cache_fill.max(2),
                cache_bar_height,
                accent_cache,
            );
        }

        cursor_y += card_height + Self::SUMMARY_GAP;
        layout.add_custom(card_height + Self::SUMMARY_GAP);

        let zebra_width = frame.outer.w - pad_x * 2;
        let bar_x = content.x + Self::LABEL_WIDTH + Self::GAP_X;
        let bar_width = (frame.outer.w - (pad_x * 2 + Self::LABEL_WIDTH + Self::GAP_X)).max(160);
        let bar_height = (Self::ROW_HEIGHT - 10).max(8);
        for (idx, row) in rows.iter().enumerate() {
            let row_top = cursor_y + (idx as i32) * Self::ROW_HEIGHT;
            if idx % 2 == 0 {
                d.draw_rectangle(
                    content.x,
                    row_top,
                    zebra_width,
                    Self::ROW_HEIGHT,
                    Color::new(24, 32, 46, 110),
                );
            }
            let label = TERRAIN_STAGE_LABELS[idx];
            let label_y = row_top + (Self::ROW_HEIGHT - Self::ROW_FONT) / 2;
            d.draw_text(
                label,
                content.x,
                label_y,
                Self::ROW_FONT,
                Color::new(230, 236, 250, 255),
            );

            d.draw_rectangle(
                bar_x,
                row_top + (Self::ROW_HEIGHT - bar_height) / 2,
                bar_width,
                bar_height,
                Color::new(22, 28, 40, 220),
            );

            let span_ratio = if max_span_ms <= 0.0 {
                0.0
            } else {
                row.p95_ms / max_span_ms
            };
            let span_fill = (span_ratio * bar_width as f32).round() as i32;
            if span_fill > 0 {
                d.draw_rectangle(
                    bar_x,
                    row_top + (Self::ROW_HEIGHT - bar_height) / 2,
                    span_fill.max(2),
                    bar_height,
                    Color::new(96, 178, 244, 220),
                );
            }

            let avg_call_text = format!("avg {:.1} | last {}", row.avg_calls, row.last_calls);
            let latency_text = format!(
                "avg {:.2}ms  p95 {:.2}ms  last {:.2}ms",
                row.avg_ms, row.p95_ms, row.last_ms
            );
            let metrics_y = row_top + Self::ROW_HEIGHT - Self::ROW_FONT - 2;
            d.draw_text(
                &latency_text,
                bar_x,
                metrics_y,
                Self::ROW_FONT,
                Color::new(210, 220, 238, 255),
            );
            let avg_w = d.measure_text(&avg_call_text, Self::ROW_FONT);
            d.draw_text(
                &avg_call_text,
                bar_x + bar_width - avg_w,
                metrics_y,
                Self::ROW_FONT,
                Color::new(190, 204, 224, 255),
            );
        }

        layout.add_rows(TERRAIN_STAGE_COUNT, Self::ROW_HEIGHT);
        Some(layout)
    }
}

impl<'a> IntentHistogramView<'a> {
    const ROW_HEIGHT: i32 = 26;
    const SECTION_HEADER_HEIGHT: i32 = 24;
    const SECTION_GAP: i32 = 14;
    const LABEL_WIDTH_CAUSE: i32 = 230;
    const LABEL_WIDTH_RADIUS: i32 = 190;
    const GAP_X: i32 = 12;
    const MIN_BAR_WIDTH: i32 = 220;
    const DEFAULT_MIN_HEIGHT: i32 = 240;
    const SECTION_FONT: i32 = 18;
    const ROW_FONT: i32 = 16;
    const MAX_CAUSE_ROWS: usize = 4;
    const MAX_RADIUS_ROWS: usize = 8;

    fn new(stats: &'a DebugStats) -> Self {
        Self {
            total: stats.intents_size,
            by_cause: &stats.intents_by_cause,
            by_radius: &stats.intents_by_radius,
        }
    }

    fn min_size(&self, theme: &WindowTheme) -> (i32, i32) {
        let cause_len = self.by_cause.len();
        let radius_len = self.by_radius.len();
        let cause_rows = if cause_len == 0 {
            1
        } else {
            cause_len.min(Self::MAX_CAUSE_ROWS)
        };
        let cause_summary = if cause_len > cause_rows { 1 } else { 0 };
        let radius_rows = if radius_len == 0 {
            1
        } else {
            radius_len.min(Self::MAX_RADIUS_ROWS)
        };
        let radius_summary = if radius_len > radius_rows { 1 } else { 0 };

        let mut min_height = theme.titlebar_height
            + theme.padding_y * 2
            + Self::SECTION_HEADER_HEIGHT * 2
            + Self::SECTION_GAP
            + ((cause_rows + cause_summary + radius_rows + radius_summary) as i32)
                * Self::ROW_HEIGHT;
        min_height = min_height.max(Self::DEFAULT_MIN_HEIGHT);

        let label_max = Self::LABEL_WIDTH_CAUSE.max(Self::LABEL_WIDTH_RADIUS);
        let min_width = theme.padding_x * 2 + label_max + Self::GAP_X + Self::MIN_BAR_WIDTH;
        (min_width, min_height)
    }

    fn subtitle(&self) -> Option<String> {
        Some(format!("{} pending", self.total))
    }

    fn draw(
        &self,
        d: &mut RaylibDrawHandle,
        frame: &WindowFrame,
        _theme: &WindowTheme,
    ) -> ContentLayout {
        let content = frame.content;
        let mut cursor_y = content.y;
        let cause_bar_x = content.x + Self::LABEL_WIDTH_CAUSE + Self::GAP_X;
        let cause_bar_width =
            (content.w - (Self::LABEL_WIDTH_CAUSE + Self::GAP_X)).max(Self::MIN_BAR_WIDTH);
        let radius_bar_x = content.x + Self::LABEL_WIDTH_RADIUS + Self::GAP_X;
        let radius_bar_width =
            (content.w - (Self::LABEL_WIDTH_RADIUS + Self::GAP_X)).max(Self::MIN_BAR_WIDTH);
        let mut layout = ContentLayout::new(content.h);

        d.draw_text(
            "By Cause",
            content.x,
            cursor_y,
            Self::SECTION_FONT,
            Color::new(238, 228, 252, 255),
        );
        cursor_y += Self::SECTION_HEADER_HEIGHT;

        if self.by_cause.is_empty() {
            let msg = if self.total == 0 {
                "No pending intents"
            } else {
                "No cause data"
            };
            let msg_y = cursor_y + (Self::ROW_HEIGHT - Self::ROW_FONT) / 2;
            d.draw_text(
                msg,
                content.x,
                msg_y,
                Self::ROW_FONT,
                Color::new(210, 200, 226, 255),
            );
            cursor_y += Self::ROW_HEIGHT;
            layout.add_rows(1, Self::ROW_HEIGHT);
        } else {
            let display_rows = self.by_cause.len().min(Self::MAX_CAUSE_ROWS);
            let remainder = self.by_cause.len().saturating_sub(display_rows);
            let max_count = self
                .by_cause
                .iter()
                .take(display_rows)
                .map(|(_, count)| *count)
                .max()
                .unwrap_or(1) as f32;
            for (idx, (label, count)) in self.by_cause.iter().take(display_rows).enumerate() {
                let row_top = cursor_y + (idx as i32) * Self::ROW_HEIGHT;
                if idx % 2 == 0 {
                    d.draw_rectangle(
                        content.x - 6,
                        row_top,
                        content.w + 12,
                        Self::ROW_HEIGHT,
                        Color::new(30, 26, 52, 110),
                    );
                }
                let label_y = row_top + (Self::ROW_HEIGHT - Self::ROW_FONT) / 2;
                d.draw_text(
                    label,
                    content.x,
                    label_y,
                    Self::ROW_FONT,
                    Color::new(232, 226, 248, 255),
                );

                let bar_height = (Self::ROW_HEIGHT - 10).max(6);
                let bar_top = row_top + (Self::ROW_HEIGHT - bar_height) / 2;
                d.draw_rectangle(
                    cause_bar_x,
                    bar_top,
                    cause_bar_width,
                    bar_height,
                    Color::new(30, 34, 60, 210),
                );
                let ratio = if max_count <= 0.0 {
                    0.0
                } else {
                    (*count as f32) / max_count
                };
                let fill_width = (ratio * cause_bar_width as f32).round() as i32;
                if fill_width > 0 {
                    let fill = fill_width.max(2).min(cause_bar_width);
                    let fill_color = match idx {
                        0 => Color::new(124, 214, 224, 230),
                        1 => Color::new(108, 198, 208, 222),
                        2 => Color::new(96, 186, 196, 218),
                        _ => Color::new(82, 170, 182, 212),
                    };
                    d.draw_rectangle(cause_bar_x, bar_top, fill, bar_height, fill_color);
                }
                let count_text = format!("{}", count);
                let count_w = d.measure_text(&count_text, Self::ROW_FONT);
                let count_y = row_top + (Self::ROW_HEIGHT - Self::ROW_FONT) / 2;
                d.draw_text(
                    &count_text,
                    cause_bar_x + cause_bar_width - count_w,
                    count_y,
                    Self::ROW_FONT,
                    Color::new(240, 234, 252, 255),
                );
            }
            cursor_y += (self.by_cause.len().min(Self::MAX_CAUSE_ROWS) as i32) * Self::ROW_HEIGHT;
            layout.add_rows(
                self.by_cause.len().min(Self::MAX_CAUSE_ROWS),
                Self::ROW_HEIGHT,
            );
            if remainder > 0 {
                let summary = format!("… {} more causes", remainder);
                let summary_y = cursor_y + (Self::ROW_HEIGHT - Self::ROW_FONT) / 2;
                d.draw_text(
                    &summary,
                    content.x,
                    summary_y,
                    Self::ROW_FONT,
                    Color::new(206, 196, 224, 255),
                );
                cursor_y += Self::ROW_HEIGHT;
                layout.add_rows(1, Self::ROW_HEIGHT);
                layout.mark_overflow(1, remainder);
            }
        }

        cursor_y += Self::SECTION_GAP;
        d.draw_text(
            "By Radius",
            content.x,
            cursor_y,
            Self::SECTION_FONT,
            Color::new(238, 228, 252, 255),
        );
        cursor_y += Self::SECTION_HEADER_HEIGHT;

        if self.by_radius.is_empty() {
            let msg = if self.total == 0 {
                "No pending intents"
            } else {
                "No radius data"
            };
            let msg_y = cursor_y + (Self::ROW_HEIGHT - Self::ROW_FONT) / 2;
            d.draw_text(
                msg,
                content.x,
                msg_y,
                Self::ROW_FONT,
                Color::new(210, 200, 226, 255),
            );
            layout.add_rows(1, Self::ROW_HEIGHT);
        } else {
            let display_rows = self.by_radius.len().min(Self::MAX_RADIUS_ROWS);
            let remainder = self.by_radius.len().saturating_sub(display_rows);
            let max_radius = self
                .by_radius
                .iter()
                .take(display_rows)
                .map(|(_, count)| *count)
                .max()
                .unwrap_or(1) as f32;
            for (idx, (label, count)) in self.by_radius.iter().take(display_rows).enumerate() {
                let row_top = cursor_y + (idx as i32) * Self::ROW_HEIGHT;
                if idx % 2 == 0 {
                    d.draw_rectangle(
                        content.x - 6,
                        row_top,
                        content.w + 12,
                        Self::ROW_HEIGHT,
                        Color::new(30, 26, 52, 110),
                    );
                }
                let label_y = row_top + (Self::ROW_HEIGHT - Self::ROW_FONT) / 2;
                d.draw_text(
                    label,
                    content.x,
                    label_y,
                    Self::ROW_FONT,
                    Color::new(232, 226, 248, 255),
                );
                let bar_height = (Self::ROW_HEIGHT - 10).max(6);
                let bar_top = row_top + (Self::ROW_HEIGHT - bar_height) / 2;
                d.draw_rectangle(
                    radius_bar_x,
                    bar_top,
                    radius_bar_width,
                    bar_height,
                    Color::new(32, 28, 58, 210),
                );
                let ratio = if max_radius <= 0.0 {
                    0.0
                } else {
                    (*count as f32) / max_radius
                };
                let fill_width = (ratio * radius_bar_width as f32).round() as i32;
                if fill_width > 0 {
                    let fill = fill_width.max(2).min(radius_bar_width);
                    let fill_color = match idx {
                        0 => Color::new(120, 198, 255, 230),
                        1 => Color::new(104, 184, 248, 220),
                        2 => Color::new(92, 168, 238, 215),
                        _ => Color::new(80, 152, 226, 210),
                    };
                    d.draw_rectangle(radius_bar_x, bar_top, fill, bar_height, fill_color);
                }
                let count_text = format!("{}", count);
                let count_w = d.measure_text(&count_text, Self::ROW_FONT);
                let count_y = row_top + (Self::ROW_HEIGHT - Self::ROW_FONT) / 2;
                d.draw_text(
                    &count_text,
                    radius_bar_x + radius_bar_width - count_w,
                    count_y,
                    Self::ROW_FONT,
                    Color::new(236, 234, 252, 255),
                );
            }
            cursor_y += (self.by_radius.len().min(Self::MAX_RADIUS_ROWS) as i32) * Self::ROW_HEIGHT;
            layout.add_rows(
                self.by_radius.len().min(Self::MAX_RADIUS_ROWS),
                Self::ROW_HEIGHT,
            );
            if remainder > 0 {
                let summary = format!("… {} more rings", remainder);
                let summary_y = cursor_y + (Self::ROW_HEIGHT - Self::ROW_FONT) / 2;
                d.draw_text(
                    &summary,
                    content.x,
                    summary_y,
                    Self::ROW_FONT,
                    Color::new(204, 198, 224, 255),
                );
                layout.add_rows(1, Self::ROW_HEIGHT);
                layout.mark_overflow(1, remainder);
            }
        }

        layout
    }
}

impl<'a> EventHistogramView<'a> {
    const MAX_ROWS: usize = 12;
    const ROW_HEIGHT: i32 = 26;
    const LABEL_WIDTH: i32 = 220;
    const BAR_MIN_WIDTH: i32 = 220;
    const GAP_X: i32 = 12;
    const DEFAULT_MIN_HEIGHT: i32 = 220;

    fn new(stats: &'a DebugStats) -> Self {
        Self {
            total: stats.queued_events_total,
            entries: &stats.queued_events_by,
        }
    }

    fn min_size(&self, theme: &WindowTheme) -> (i32, i32) {
        let base_rows = self.entries.len().min(Self::MAX_ROWS).max(1);
        let remainder = self.entries.len().saturating_sub(base_rows);
        let mut min_height = theme.titlebar_height + theme.padding_y * 2;
        min_height += (base_rows as i32) * Self::ROW_HEIGHT;
        if remainder > 0 {
            min_height += Self::ROW_HEIGHT;
        }
        min_height = min_height.max(Self::DEFAULT_MIN_HEIGHT);
        let min_width = theme.padding_x * 2 + Self::LABEL_WIDTH + Self::GAP_X + Self::BAR_MIN_WIDTH;
        (min_width, min_height)
    }

    fn subtitle(&self) -> Option<String> {
        Some(format!("{} pending", self.total))
    }

    fn draw(
        &self,
        d: &mut RaylibDrawHandle,
        frame: &WindowFrame,
        _theme: &WindowTheme,
    ) -> ContentLayout {
        let content = frame.content;
        let mut cursor_y = content.y;
        let bar_x = content.x + Self::LABEL_WIDTH + Self::GAP_X;
        let bar_width = (content.w - (Self::LABEL_WIDTH + Self::GAP_X)).max(Self::BAR_MIN_WIDTH);
        let mut layout = ContentLayout::new(content.h);

        let rows_fit = if content.h <= 0 {
            1_usize
        } else {
            (content.h / Self::ROW_HEIGHT).max(1) as usize
        };

        let mut display_count = self.entries.len().min(rows_fit);
        let mut remainder = self.entries.len().saturating_sub(display_count);
        if remainder > 0 && display_count + 1 > rows_fit {
            if display_count > 0 {
                display_count -= 1;
            }
            remainder = self.entries.len().saturating_sub(display_count);
        }

        if self.entries.is_empty() {
            let msg = "No queued events";
            let msg_y = cursor_y + (Self::ROW_HEIGHT - 16) / 2;
            d.draw_text(msg, content.x, msg_y, 16, Color::new(192, 198, 216, 255));
            cursor_y += Self::ROW_HEIGHT;
            layout.add_rows(1, Self::ROW_HEIGHT);
        } else {
            let max_count = self
                .entries
                .iter()
                .take(display_count.max(1))
                .map(|(_, count)| *count)
                .max()
                .unwrap_or(1) as f32;
            for (idx, (label, count)) in self.entries.iter().take(display_count).enumerate() {
                let row_top = cursor_y + (idx as i32) * Self::ROW_HEIGHT;
                if idx % 2 == 0 {
                    d.draw_rectangle(
                        content.x - 6,
                        row_top,
                        content.w + 12,
                        Self::ROW_HEIGHT,
                        Color::new(26, 30, 44, 120),
                    );
                }
                let label_y = row_top + (Self::ROW_HEIGHT - 16) / 2;
                let label_color = if idx == 0 {
                    Color::new(238, 244, 255, 255)
                } else {
                    Color::new(212, 220, 240, 255)
                };
                d.draw_text(label, content.x, label_y, 16, label_color);

                let bar_height = (Self::ROW_HEIGHT - 10).max(6);
                let bar_top = row_top + (Self::ROW_HEIGHT - bar_height) / 2;
                d.draw_rectangle(
                    bar_x,
                    bar_top,
                    bar_width,
                    bar_height,
                    Color::new(30, 38, 54, 210),
                );

                let ratio = if max_count <= 0.0 {
                    0.0
                } else {
                    (*count as f32) / max_count
                };
                let fill_width = (ratio * bar_width as f32).round() as i32;
                if fill_width > 0 {
                    let fill = fill_width.max(2).min(bar_width);
                    let fill_color = match idx {
                        0 => Color::new(118, 202, 255, 230),
                        1 => Color::new(96, 186, 250, 220),
                        2 => Color::new(82, 170, 240, 215),
                        _ => Color::new(68, 152, 222, 210),
                    };
                    d.draw_rectangle(bar_x, bar_top, fill, bar_height, fill_color);
                }

                let count_text = format_count(*count);
                let count_w = d.measure_text(&count_text, 16);
                let count_y = row_top + (Self::ROW_HEIGHT - 16) / 2;
                d.draw_text(
                    &count_text,
                    bar_x + bar_width - count_w,
                    count_y,
                    16,
                    Color::new(234, 238, 252, 255),
                );
            }
            cursor_y += (display_count as i32) * Self::ROW_HEIGHT;
            layout.add_rows(display_count.max(1), Self::ROW_HEIGHT);
        }

        if remainder > 0 {
            let summary = format!("… {} more types", remainder);
            let summary_y = cursor_y + (Self::ROW_HEIGHT - 16) / 2;
            d.draw_text(
                &summary,
                content.x,
                summary_y,
                16,
                Color::new(188, 196, 214, 255),
            );
            layout.add_rows(1, Self::ROW_HEIGHT);
            layout.mark_overflow(1, remainder);
        }

        layout
    }
}

impl App {
    pub fn render(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread) {
        // Preserve queued-events snapshot captured during step() before processing,
        // then reset per-frame stats for rendering accumulation.
        let prev_q_total = self.debug_stats.queued_events_total;
        let prev_q_by = self.debug_stats.queued_events_by.clone();
        let prev_intents = self.debug_stats.intents_size;
        let prev_intents_by_cause = self.debug_stats.intents_by_cause.clone();
        let prev_intents_by_radius = self.debug_stats.intents_by_radius.clone();
        self.debug_stats = DebugStats::default();
        self.debug_stats.queued_events_total = prev_q_total;
        self.debug_stats.queued_events_by = prev_q_by;
        self.debug_stats.intents_size = prev_intents;
        self.debug_stats.intents_by_cause = prev_intents_by_cause;
        self.debug_stats.intents_by_radius = prev_intents_by_radius;

        // Snapshot current residency metrics for debug overlay
        self.debug_stats.loaded_chunks = self.gs.chunks.ready_len();
        let mut unique_cx: HashSet<i32> = HashSet::new();
        let mut unique_cy: HashSet<i32> = HashSet::new();
        let mut unique_cz: HashSet<i32> = HashSet::new();
        let mut nonempty = 0usize;
        for (coord, entry) in self.gs.chunks.iter() {
            unique_cx.insert(coord.cx);
            unique_cy.insert(coord.cy);
            unique_cz.insert(coord.cz);
            if entry.has_blocks() {
                nonempty += 1;
            }
        }
        self.debug_stats.chunk_resident_total = self.gs.chunks.ready_len();
        self.debug_stats.chunk_resident_nonempty = nonempty;
        self.debug_stats.chunk_unique_cx = unique_cx.len();
        self.debug_stats.chunk_unique_cy = unique_cy.len();
        self.debug_stats.chunk_unique_cz = unique_cz.len();
        self.debug_stats.render_cache_chunks = self.renders.len();

        let light_stats = self.gs.lighting.stats();
        self.debug_stats.lighting_border_chunks = light_stats.border_chunks;
        self.debug_stats.lighting_emitter_chunks = light_stats.emitter_chunks;
        self.debug_stats.lighting_micro_chunks = light_stats.micro_chunks;

        let edit_stats = self.gs.edits.stats();
        self.debug_stats.edit_chunk_entries = edit_stats.chunk_entries;
        self.debug_stats.edit_block_edits = edit_stats.block_edits;
        self.debug_stats.edit_rev_entries = edit_stats.rev_entries;
        self.debug_stats.edit_built_entries = edit_stats.built_entries;

        // Calculate frustum for culling
        let screen_width = rl.get_screen_width() as f32;
        let screen_height = rl.get_screen_height() as f32;
        let aspect_ratio = screen_width / screen_height;
        let frustum = self.cam.calculate_frustum(aspect_ratio, 0.1, 10000.0); // Increased far plane

        // Time-of-day sky color (used for clear background and surface fog)
        let time_now = rl.get_time() as f32;
        let day_length_sec = 60.0_f32; // ~4 minutes per full cycle
        let phase = (time_now / day_length_sec) * std::f32::consts::TAU; // 0..2pi
        let sky_scale = 0.5 * (1.0 + phase.sin()); // 0..1 (0 = midnight, 1 = noon)
        let day_sky = [210.0 / 255.0, 221.0 / 255.0, 235.0 / 255.0];
        let night_sky = [10.0 / 255.0, 12.0 / 255.0, 20.0 / 255.0];
        let t_gamma = sky_scale.powf(1.5);
        // Base sky from night->day blend
        let base_sky = [
            night_sky[0] + (day_sky[0] - night_sky[0]) * t_gamma,
            night_sky[1] + (day_sky[1] - night_sky[1]) * t_gamma,
            night_sky[2] + (day_sky[2] - night_sky[2]) * t_gamma,
        ];
        // Dawn/Dusk warm tint: peak near sunrise/sunset, minimal at noon/midnight
        let warm_tint = [1.0, 0.63, 0.32];
        let twilight = phase.cos().abs().powf(3.0); // 0 at noon/midnight, 1 at dawn/dusk
        // Scale warmth by how bright the sky is to avoid over-saturating at night
        let warm_strength = (0.35 * twilight * sky_scale).clamp(0.0, 0.5);
        let surface_sky = [
            base_sky[0] * (1.0 - warm_strength) + warm_tint[0] * warm_strength,
            base_sky[1] * (1.0 - warm_strength) + warm_tint[1] * warm_strength,
            base_sky[2] * (1.0 - warm_strength) + warm_tint[2] * warm_strength,
        ];

        let camera3d = self.cam.to_camera3d();
        self.minimap_ui_rect = None;

        let screen_dims = (screen_width as i32, screen_height as i32);
        let overlay_theme = *self.overlay_windows.theme();
        let minimap_min_size = (
            overlay_theme.padding_x * 2 + MINIMAP_MIN_CONTENT_SIDE,
            overlay_theme.titlebar_height + overlay_theme.padding_y * 2 + MINIMAP_MIN_CONTENT_SIDE,
        );
        let mut minimap_plan: Option<(WindowFrame, i32, i32)> = None;
        if self.gs.show_debug_overlay {
            if let Some(window) = self.overlay_windows.get_mut(WindowId::Minimap) {
                window.set_min_size(minimap_min_size);
                let frame = window.layout(screen_dims, &overlay_theme);
                let content = frame.content;
                let available_side = content.w.min(content.h).max(0);
                let outer_side =
                    available_side.min(MINIMAP_MAX_CONTENT_SIDE + MINIMAP_BORDER_PX * 2);
                let map_side = (outer_side - MINIMAP_BORDER_PX * 2).max(0);
                minimap_plan = Some((frame, outer_side, map_side));
                if map_side > 0 {
                    self.render_minimap_to_texture(rl, thread, map_side);
                } else {
                    self.render_minimap_to_texture(rl, thread, 0);
                }
            } else {
                self.render_minimap_to_texture(rl, thread, 0);
            }
        } else {
            self.render_minimap_to_texture(rl, thread, 0);
        }

        let mut d = rl.begin_drawing(thread);
        // Skybox: clear background to time-of-day sky color
        d.clear_background(Color::new(
            (surface_sky[0] * 255.0) as u8,
            (surface_sky[1] * 255.0) as u8,
            (surface_sky[2] * 255.0) as u8,
            255,
        ));
        // Ensure the depth buffer is cleared every frame to avoid ghost silhouettes when moving
        unsafe {
            raylib::ffi::rlClearScreenBuffers();
        }
        {
            let mut d3 = d.begin_mode3D(camera3d);
            if self.gs.show_grid {
                d3.draw_grid(64, 1.0);
            }

            // Determine if camera is underwater (used for fog + water + leaves)
            let p_cam = self.cam.position;
            let wx = p_cam.x.floor() as i32;
            let wy = p_cam.y.floor() as i32;
            let wz = p_cam.z.floor() as i32;
            let b_cam = if let Some(edit) = self.gs.edits.get(wx, wy, wz) {
                edit
            } else {
                // Prefer loaded chunk buffers before falling back to worldgen
                let sx = self.gs.world.chunk_size_x as i32;
                let sy = self.gs.world.chunk_size_y as i32;
                let sz = self.gs.world.chunk_size_z as i32;
                let cx = wx.div_euclid(sx);
                let cy = wy.div_euclid(sy);
                let cz = wz.div_euclid(sz);
                let coord = ChunkCoord::new(cx, cy, cz);
                if let Some(cent) = self.gs.chunks.get(&coord) {
                    match (cent.occupancy_or_empty(), cent.buf.as_ref()) {
                        (ChunkOccupancy::Empty, _) => Block::AIR,
                        (_, Some(buf)) => buf.get_world(wx, wy, wz).unwrap_or(Block::AIR),
                        (_, None) => self.gs.world.block_at_runtime(&self.reg, wx, wy, wz),
                    }
                } else {
                    self.gs.world.block_at_runtime(&self.reg, wx, wy, wz)
                }
            };
            let underwater = self
                .reg
                .get(b_cam.id)
                .map(|ty| ty.name == "water")
                .unwrap_or(false);

            // Update shader uniforms
            let cave_fog = [0.0, 0.0, 0.0];
            // Underwater tint: soft blue-green
            let water_fog = [0.16, 0.32, 0.45];
            let world_h = self.gs.world.world_height_hint() as f32;
            let underground_thr = 0.30_f32 * world_h;
            let underground = self.cam.position.y < underground_thr;
            let fog_color = if underwater {
                water_fog
            } else if underground {
                cave_fog
            } else {
                surface_sky
            };
            // Fog ranges: denser underwater
            let fog_start = if underwater { 4.0 } else { 512.0 * 0.1 };
            let fog_end = if underwater { 48.0 } else { 512.0 * 0.8 };
            if let Some(ref mut ls) = self.leaves_shader {
                ls.update_frame_uniforms(
                    self.cam.position,
                    fog_color,
                    fog_start,
                    fog_end,
                    time_now,
                    underwater,
                    sky_scale,
                );
            }
            if let Some(ref mut fs) = self.fog_shader {
                fs.update_frame_uniforms(
                    self.cam.position,
                    fog_color,
                    fog_start,
                    fog_end,
                    time_now,
                    underwater,
                    sky_scale,
                );
            }
            if let Some(ref mut ws) = self.water_shader {
                ws.update_frame_uniforms(
                    self.cam.position,
                    fog_color,
                    fog_start,
                    fog_end,
                    time_now,
                    underwater,
                    sky_scale,
                );
            }

            // First pass: draw opaque parts and gather visible chunks for transparent pass
            let mut visible_chunks: Vec<(ChunkCoord, f32)> = Vec::new();
            for (ckey, cr) in self.renders.iter() {
                // Check if chunk is within frustum
                if self.gs.frustum_culling_enabled && !frustum.contains_bounding_box(&cr.bbox) {
                    self.debug_stats.chunks_culled += 1;
                    continue;
                }

                self.debug_stats.chunks_rendered += 1;
                // Record for transparent pass (sort by distance from camera)
                let center = (cr.bbox.min + cr.bbox.max) * 0.5;
                let dx = center.x - self.cam.position.x;
                let dy = center.y - self.cam.position.y;
                let dz = center.z - self.cam.position.z;
                let dist2 = dx * dx + dy * dy + dz * dz;
                visible_chunks.push((*ckey, dist2));
                // Precompute per-chunk lighting parameters
                let origin = cr.origin;
                let vis_min = 18.0f32 / 255.0f32;
                let (dims_some, grid_some) = if let Some(ref lt) = cr.light_tex {
                    ((lt.sx, lt.sy, lt.sz), (lt.grid_cols, lt.grid_rows))
                } else {
                    ((0, 0, 0), (0, 0))
                };
                // Set biome-based leaf palette per chunk if available
                if let Some(ref mut ls) = self.leaves_shader {
                    if let Some(t) = cr.leaf_tint {
                        let p0 = t;
                        let p1 = [t[0] * 0.85, t[1] * 0.85, t[2] * 0.85];
                        let p2 = [t[0] * 0.7, t[1] * 0.7, t[2] * 0.7];
                        let p3 = [t[0] * 0.5, t[1] * 0.5, t[2] * 0.5];
                        ls.set_autumn_palette(p0, p1, p2, p3, 1.0);
                    } else {
                        // Default greenish palette
                        ls.set_autumn_palette(
                            [0.32, 0.55, 0.25],
                            [0.28, 0.48, 0.22],
                            [0.20, 0.40, 0.18],
                            [0.12, 0.28, 0.10],
                            1.0,
                        );
                    }
                }
                for part in &cr.parts {
                    // Get mesh stats from the model
                    unsafe {
                        let mesh = &*part.model.meshes;
                        self.debug_stats.total_vertices += mesh.vertexCount as usize;
                        self.debug_stats.total_triangles += mesh.triangleCount as usize;
                    }
                    let tag = self
                        .reg
                        .materials
                        .get(part.mid)
                        .and_then(|m| m.render_tag.as_deref());
                    if tag != Some("water") {
                        // Bind only the shader used by this part, right before draw
                        match tag {
                            Some("leaves") => {
                                if let Some(ref mut ls) = self.leaves_shader {
                                    if let Some(ref lt) = cr.light_tex {
                                        ls.update_chunk_uniforms(
                                            thread, &lt.tex, dims_some, grid_some, origin, vis_min,
                                        );
                                    } else {
                                        ls.update_chunk_uniforms_no_tex(
                                            thread, dims_some, grid_some, origin, vis_min,
                                        );
                                    }
                                }
                            }
                            _ => {
                                if let Some(ref mut fs) = self.fog_shader {
                                    if let Some(ref lt) = cr.light_tex {
                                        fs.update_chunk_uniforms(
                                            thread, &lt.tex, dims_some, grid_some, origin, vis_min,
                                        );
                                    } else {
                                        fs.update_chunk_uniforms_no_tex(
                                            thread, dims_some, grid_some, origin, vis_min,
                                        );
                                    }
                                }
                            }
                        }
                        self.debug_stats.draw_calls += 1;
                        if self.gs.wireframe {
                            d3.draw_model_wires(&part.model, Vector3::zero(), 1.0, Color::WHITE);
                        } else {
                            d3.draw_model(&part.model, Vector3::zero(), 1.0, Color::WHITE);
                        }
                    }
                }
            }

            // Draw structures with transform (translation + yaw)
            let mut visible_structs: Vec<(StructureId, f32)> = Vec::new();
            for (id, cr) in &self.structure_renders {
                if let Some(st) = self.gs.structures.get(id) {
                    // Translate bounding box to structure position for frustum check
                    let translated_bbox = raylib::core::math::BoundingBox {
                        min: cr.bbox.min + vec3_to_rl(st.pose.pos),
                        max: cr.bbox.max + vec3_to_rl(st.pose.pos),
                    };

                    // Check if structure is within frustum
                    if self.gs.frustum_culling_enabled
                        && !frustum.contains_bounding_box(&translated_bbox)
                    {
                        self.debug_stats.structures_culled += 1;
                        continue;
                    }

                    self.debug_stats.structures_rendered += 1;
                    // Record for transparent pass
                    let center = (translated_bbox.min + translated_bbox.max) * 0.5;
                    let dx = center.x - self.cam.position.x;
                    let dy = center.y - self.cam.position.y;
                    let dz = center.z - self.cam.position.z;
                    let dist2 = dx * dx + dy * dy + dz * dz;
                    visible_structs.push((*id, dist2));
                    for part in &cr.parts {
                        // Get mesh stats from the model
                        unsafe {
                            let mesh = &*part.model.meshes;
                            self.debug_stats.total_vertices += mesh.vertexCount as usize;
                            self.debug_stats.total_triangles += mesh.triangleCount as usize;
                        }
                        // Only draw opaque parts in first pass (water is transparent)
                        let tag = self
                            .reg
                            .materials
                            .get(part.mid)
                            .and_then(|m| m.render_tag.as_deref());
                        if tag != Some("water") {
                            self.debug_stats.draw_calls += 1;
                            d3.draw_model(&part.model, vec3_to_rl(st.pose.pos), 1.0, Color::WHITE);
                        }
                    }
                }
            }

            // Transparent pass: draw water parts back-to-front (blend on, depth write off)
            unsafe {
                // Keep depth test enabled but stop writing depth for transparent surfaces
                raylib::ffi::rlDisableDepthMask();
            }
            visible_chunks
                .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            for (ckey, _) in visible_chunks {
                if let Some(cr) = self.renders.get(&ckey) {
                    if self.gs.frustum_culling_enabled && !frustum.contains_bounding_box(&cr.bbox) {
                        continue;
                    }
                    // Precompute per-chunk lighting parameters
                    let origin = cr.origin;
                    let vis_min = 18.0f32 / 255.0f32;
                    let (dims_some, grid_some) = if let Some(ref lt) = cr.light_tex {
                        ((lt.sx, lt.sy, lt.sz), (lt.grid_cols, lt.grid_rows))
                    } else {
                        ((0, 0, 0), (0, 0))
                    };
                    for part in &cr.parts {
                        let tag = self
                            .reg
                            .materials
                            .get(part.mid)
                            .and_then(|m| m.render_tag.as_deref());
                        if tag == Some("water") {
                            // Bind only the shader used by this part, right before draw
                            if let Some(ref mut ws) = self.water_shader {
                                if let Some(ref lt) = cr.light_tex {
                                    ws.update_chunk_uniforms(
                                        thread, &lt.tex, dims_some, grid_some, origin, vis_min,
                                    );
                                } else {
                                    ws.update_chunk_uniforms_no_tex(
                                        thread, dims_some, grid_some, origin, vis_min,
                                    );
                                }
                            }
                            self.debug_stats.draw_calls += 1;
                            unsafe {
                                raylib::ffi::rlDisableBackfaceCulling();
                            }
                            d3.draw_model(&part.model, Vector3::zero(), 1.0, Color::WHITE);
                            unsafe {
                                raylib::ffi::rlEnableBackfaceCulling();
                            }
                        }
                    }
                }
            }

            // Transparent pass for structures (back-to-front)
            visible_structs
                .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            for (sid, _) in visible_structs {
                if let Some(cr) = self.structure_renders.get(&sid) {
                    if let Some(st) = self.gs.structures.get(&sid) {
                        let translated_bbox = raylib::core::math::BoundingBox {
                            min: cr.bbox.min + vec3_to_rl(st.pose.pos),
                            max: cr.bbox.max + vec3_to_rl(st.pose.pos),
                        };
                        if self.gs.frustum_culling_enabled
                            && !frustum.contains_bounding_box(&translated_bbox)
                        {
                            continue;
                        }
                        for part in &cr.parts {
                            let tag = self
                                .reg
                                .materials
                                .get(part.mid)
                                .and_then(|m| m.render_tag.as_deref());
                            if tag == Some("water") {
                                self.debug_stats.draw_calls += 1;
                                unsafe {
                                    raylib::ffi::rlDisableBackfaceCulling();
                                }
                                d3.draw_model(
                                    &part.model,
                                    vec3_to_rl(st.pose.pos),
                                    1.0,
                                    Color::WHITE,
                                );
                                unsafe {
                                    raylib::ffi::rlEnableBackfaceCulling();
                                }
                            }
                        }
                    }
                }
            }
            unsafe {
                // Restore depth writes
                raylib::ffi::rlEnableDepthMask();
            }

            // Raycast highlight: show where a placed block would go (world only for now)
            // Sample order: edits > loaded chunk buffers > world
            let org = self.cam.position;
            let dir = self.cam.forward();
            let sx = self.gs.world.chunk_size_x as i32;
            let sy = self.gs.world.chunk_size_y as i32;
            let sz = self.gs.world.chunk_size_z as i32;
            let sampler = |wx: i32, wy: i32, wz: i32| -> Block {
                if let Some(b) = self.gs.edits.get(wx, wy, wz) {
                    return b;
                }
                let cx = wx.div_euclid(sx);
                let cy = wy.div_euclid(sy);
                let cz = wz.div_euclid(sz);
                let coord = ChunkCoord::new(cx, cy, cz);
                if let Some(cent) = self.gs.chunks.get(&coord) {
                    match (cent.occupancy_or_empty(), cent.buf.as_ref()) {
                        (ChunkOccupancy::Empty, _) => return Block::AIR,
                        (_, Some(buf)) => {
                            return buf.get_world(wx, wy, wz).unwrap_or(Block::AIR);
                        }
                        (_, None) => {}
                    }
                }
                self.gs.world.block_at_runtime(&self.reg, wx, wy, wz)
            };
            let is_solid = |wx: i32, wy: i32, wz: i32| -> bool {
                let b = sampler(wx, wy, wz);
                self.reg
                    .get(b.id)
                    .map(|ty| ty.is_solid(b.state))
                    .unwrap_or(false)
            };
            if let Some(hit) = raycast::raycast_first_hit_with_face(org, dir, 5.0, is_solid) {
                // Outline only the struck face of the solid block (bx,by,bz)
                let (bx, by, bz) = (hit.bx, hit.by, hit.bz);
                let (x0, y0, z0) = (bx as f32, by as f32, bz as f32);
                let (x1, y1, z1) = (x0 + 1.0, y0 + 1.0, z0 + 1.0);
                let eps = 0.002f32;
                if hit.nx != 0 {
                    let xf = if hit.nx > 0 { x1 } else { x0 } + (hit.nx as f32) * eps;
                    let p1 = Vector3::new(xf, y0, z0);
                    let p2 = Vector3::new(xf, y1, z0);
                    let p3 = Vector3::new(xf, y1, z1);
                    let p4 = Vector3::new(xf, y0, z1);
                    d3.draw_line_3D(p1, p2, Color::YELLOW);
                    d3.draw_line_3D(p2, p3, Color::YELLOW);
                    d3.draw_line_3D(p3, p4, Color::YELLOW);
                    d3.draw_line_3D(p4, p1, Color::YELLOW);
                } else if hit.ny != 0 {
                    let yf = if hit.ny > 0 { y1 } else { y0 } + (hit.ny as f32) * eps;
                    let p1 = Vector3::new(x0, yf, z0);
                    let p2 = Vector3::new(x1, yf, z0);
                    let p3 = Vector3::new(x1, yf, z1);
                    let p4 = Vector3::new(x0, yf, z1);
                    d3.draw_line_3D(p1, p2, Color::YELLOW);
                    d3.draw_line_3D(p2, p3, Color::YELLOW);
                    d3.draw_line_3D(p3, p4, Color::YELLOW);
                    d3.draw_line_3D(p4, p1, Color::YELLOW);
                } else if hit.nz != 0 {
                    let zf = if hit.nz > 0 { z1 } else { z0 } + (hit.nz as f32) * eps;
                    let p1 = Vector3::new(x0, y0, zf);
                    let p2 = Vector3::new(x1, y0, zf);
                    let p3 = Vector3::new(x1, y1, zf);
                    let p4 = Vector3::new(x0, y1, zf);
                    d3.draw_line_3D(p1, p2, Color::YELLOW);
                    d3.draw_line_3D(p2, p3, Color::YELLOW);
                    d3.draw_line_3D(p3, p4, Color::YELLOW);
                    d3.draw_line_3D(p4, p1, Color::YELLOW);
                }
            }

            if self.gs.show_chunk_bounds {
                let center_chunk = self.gs.center_chunk;
                for cr in self.renders.values() {
                    let min = cr.bbox.min;
                    let max = cr.bbox.max;
                    let center = Vector3::new(
                        (min.x + max.x) * 0.5,
                        (min.y + max.y) * 0.5,
                        (min.z + max.z) * 0.5,
                    );
                    let size = Vector3::new(
                        (max.x - min.x).abs(),
                        (max.y - min.y).abs(),
                        (max.z - min.z).abs(),
                    );
                    let dy = cr.coord.cy - center_chunk.cy;
                    let abs_dy = dy.abs();
                    let alpha = (220 - (abs_dy.min(4) * 30)).clamp(90, 220) as u8;
                    let mut col = if dy > 0 {
                        Color::new(72, 144, 255, alpha)
                    } else if dy < 0 {
                        Color::new(255, 140, 88, alpha)
                    } else {
                        Color::new(255, 64, 32, alpha)
                    };
                    if cr.coord == center_chunk {
                        col = Color::YELLOW;
                    }
                    d3.draw_cube_wires(center, size.x, size.y, size.z, col);
                }
            }
        }

        if self.gs.show_debug_overlay {
            let fps = d.get_fps();

            if self.overlay_windows.get(WindowId::RenderStats).is_some() {
                let view = RenderStatsView::new(self, fps);
                if let Some(window) = self.overlay_windows.get_mut(WindowId::RenderStats) {
                    window.set_min_size(view.min_size(&overlay_theme));
                    let frame = window.layout(screen_dims, &overlay_theme);
                    let hover = self
                        .overlay_hover
                        .and_then(|(id, region)| (id == WindowId::RenderStats).then_some(region));
                    WindowChrome::draw(
                        &mut d,
                        &overlay_theme,
                        &frame,
                        "Frame Stats",
                        view.subtitle(),
                        hover,
                    );
                    let layout = view.draw(&mut d, &frame);
                    self.draw_overflow_hint(&mut d, &frame, layout);
                }
            }

            if self.overlay_windows.get(WindowId::RuntimeStats).is_some() {
                let view = RuntimeStatsView::new(self);
                if let Some(window) = self.overlay_windows.get_mut(WindowId::RuntimeStats) {
                    window.set_min_size(view.min_size(&overlay_theme));
                    let frame = window.layout(screen_dims, &overlay_theme);
                    let hover = self
                        .overlay_hover
                        .and_then(|(id, region)| (id == WindowId::RuntimeStats).then_some(region));
                    WindowChrome::draw(
                        &mut d,
                        &overlay_theme,
                        &frame,
                        "Runtime Stats",
                        view.subtitle(),
                        hover,
                    );
                    let layout = view.draw(&mut d, &frame);
                    self.draw_overflow_hint(&mut d, &frame, layout);
                }
            }

            if self
                .overlay_windows
                .get(WindowId::AttachmentDebug)
                .is_some()
            {
                let view = AttachmentDebugView::new(self);
                if let Some(window) = self.overlay_windows.get_mut(WindowId::AttachmentDebug) {
                    window.set_min_size(view.min_size(&overlay_theme));
                    let frame = window.layout(screen_dims, &overlay_theme);
                    let hover = self.overlay_hover.and_then(|(id, region)| {
                        (id == WindowId::AttachmentDebug).then_some(region)
                    });
                    WindowChrome::draw(
                        &mut d,
                        &overlay_theme,
                        &frame,
                        "Attachment Debug",
                        None,
                        hover,
                    );
                    let layout = view.draw(&mut d, &frame);
                    self.draw_overflow_hint(&mut d, &frame, layout);
                }
            }

            if let Some(window) = self.overlay_windows.get_mut(WindowId::EventHistogram) {
                let view = EventHistogramView::new(&self.debug_stats);
                window.set_min_size(view.min_size(&overlay_theme));
                let frame = window.layout(screen_dims, &overlay_theme);
                let hover = self
                    .overlay_hover
                    .and_then(|(id, region)| (id == WindowId::EventHistogram).then_some(region));
                let subtitle = view.subtitle();
                WindowChrome::draw(
                    &mut d,
                    &overlay_theme,
                    &frame,
                    "Event Queue",
                    subtitle.as_deref(),
                    hover,
                );
                let layout = view.draw(&mut d, &frame, &overlay_theme);
                self.draw_overflow_hint(&mut d, &frame, layout);
            }

            if let Some(window) = self.overlay_windows.get_mut(WindowId::IntentHistogram) {
                let view = IntentHistogramView::new(&self.debug_stats);
                window.set_min_size(view.min_size(&overlay_theme));
                let frame = window.layout(screen_dims, &overlay_theme);
                let hover = self
                    .overlay_hover
                    .and_then(|(id, region)| (id == WindowId::IntentHistogram).then_some(region));
                let subtitle = view.subtitle();
                WindowChrome::draw(
                    &mut d,
                    &overlay_theme,
                    &frame,
                    "Intent Queue",
                    subtitle.as_deref(),
                    hover,
                );
                let layout = view.draw(&mut d, &frame, &overlay_theme);
                self.draw_overflow_hint(&mut d, &frame, layout);
            }

            let terrain_view = TerrainHistogramView::new(
                &self.terrain_stage_us,
                &self.terrain_stage_calls,
                &self.terrain_height_tile_us,
                &self.terrain_height_tile_reused,
                &self.terrain_cache_hits,
                &self.terrain_cache_misses,
            );
            if let Some(window) = self.overlay_windows.get_mut(WindowId::TerrainHistogram) {
                window.set_min_size(terrain_view.min_size(&overlay_theme));
                let frame = window.layout(screen_dims, &overlay_theme);
                let hover = self
                    .overlay_hover
                    .and_then(|(id, region)| (id == WindowId::TerrainHistogram).then_some(region));
                let subtitle = terrain_view.subtitle();
                WindowChrome::draw(
                    &mut d,
                    &overlay_theme,
                    &frame,
                    "Terrain Pipeline",
                    subtitle.as_deref(),
                    hover,
                );
                if let Some(layout) = terrain_view.draw(&mut d, &frame, &overlay_theme) {
                    self.draw_overflow_hint(&mut d, &frame, layout);
                }
            }

            if let Some((frame, outer_side, map_side)) = minimap_plan {
                let hover = self
                    .overlay_hover
                    .and_then(|(id, region)| (id == WindowId::Minimap).then_some(region));
                let subtitle = Some(format!(
                    "radius {} chunks",
                    self.gs.view_radius_chunks.max(0)
                ));

                WindowChrome::draw(
                    &mut d,
                    &overlay_theme,
                    &frame,
                    "Minimap",
                    subtitle.as_deref(),
                    hover,
                );

                let content = frame.content;
                if map_side > 0 {
                    let frame_x = content.x + (content.w - outer_side) / 2;
                    let frame_y = content.y + (content.h - outer_side) / 2;
                    let frame_rect = IRect::new(frame_x, frame_y, outer_side, outer_side);
                    let map_rect = IRect::new(
                        frame_rect.x + MINIMAP_BORDER_PX,
                        frame_rect.y + MINIMAP_BORDER_PX,
                        map_side,
                        map_side,
                    );

                    d.draw_rectangle(
                        frame_rect.x,
                        frame_rect.y,
                        frame_rect.w,
                        frame_rect.h,
                        Color::new(12, 18, 28, 210),
                    );
                    d.draw_rectangle_lines(
                        frame_rect.x,
                        frame_rect.y,
                        frame_rect.w,
                        frame_rect.h,
                        Color::new(86, 108, 152, 210),
                    );

                    if let Some(ref minimap_rt) = self.minimap_rt {
                        let tex = minimap_rt.texture().clone();
                        let src =
                            Rectangle::new(0.0, 0.0, tex.width() as f32, -(tex.height() as f32));
                        let dest = Rectangle::new(
                            map_rect.x as f32,
                            map_rect.y as f32,
                            map_rect.w as f32,
                            map_rect.h as f32,
                        );
                        d.draw_texture_pro(
                            tex,
                            src,
                            dest,
                            Vector2::new(0.0, 0.0),
                            0.0,
                            Color::WHITE,
                        );
                        self.minimap_ui_rect =
                            Some((frame_rect.x, frame_rect.y, frame_rect.w, frame_rect.h));

                        let label = format!("Loaded {} chunks", self.gs.chunks.ready_len());
                        let label_fs = 18;
                        let label_x = map_rect.x + 14;
                        let label_y = map_rect.y + 14;
                        d.draw_text(
                            &label,
                            label_x + 1,
                            label_y + 1,
                            label_fs,
                            Color::new(0, 0, 0, 220),
                        );
                        d.draw_text(&label, label_x, label_y, label_fs, Color::WHITE);

                        let legend = ["Scroll: zoom", "LMB drag: orbit", "Shift+Drag/RMB: pan"];
                        let legend_fs = 14;
                        let legend_total_h = (legend.len() as i32) * (legend_fs + 2);
                        let mut legend_y = map_rect.y + map_rect.h - legend_total_h - 12;
                        let legend_min_y = map_rect.y + 14;
                        if legend_y < legend_min_y {
                            legend_y = legend_min_y;
                        }
                        for line in legend.iter() {
                            d.draw_text(
                                line,
                                map_rect.x + 14 + 1,
                                legend_y + 1,
                                legend_fs,
                                Color::new(0, 0, 0, 200),
                            );
                            d.draw_text(
                                line,
                                map_rect.x + 14,
                                legend_y,
                                legend_fs,
                                Color::new(220, 220, 240, 240),
                            );
                            legend_y += legend_fs + 2;
                        }
                    } else {
                        self.minimap_ui_rect = None;
                        d.draw_rectangle(
                            map_rect.x,
                            map_rect.y,
                            map_rect.w,
                            map_rect.h,
                            Color::new(18, 24, 34, 220),
                        );
                        let msg = "Minimap unavailable";
                        let msg_fs = 18;
                        let msg_w = d.measure_text(msg, msg_fs);
                        let msg_x = map_rect.x + (map_rect.w - msg_w) / 2;
                        let msg_y = map_rect.y + (map_rect.h - msg_fs) / 2;
                        d.draw_text(msg, msg_x + 1, msg_y + 1, msg_fs, Color::new(0, 0, 0, 220));
                        d.draw_text(msg, msg_x, msg_y, msg_fs, Color::new(220, 220, 240, 240));
                    }
                } else {
                    self.minimap_ui_rect = None;
                    let msg = "Expand the window to view the minimap";
                    let msg_fs = 16;
                    let msg_w = d.measure_text(msg, msg_fs);
                    let msg_x = content.x + (content.w - msg_w) / 2;
                    let msg_y = content.y + (content.h - msg_fs) / 2;
                    d.draw_text(msg, msg_x + 1, msg_y + 1, msg_fs, Color::new(0, 0, 0, 180));
                    d.draw_text(msg, msg_x, msg_y, msg_fs, Color::new(218, 228, 248, 230));
                }
            }
        } // end debug overlay

        // HUD
        let hud_mode = if self.gs.walk_mode { "Walk" } else { "Fly" };
        let hud = format!(
            "{}: Tab capture, WASD{} move{}, V toggle mode, F wireframe, G grid, B bounds, C culling, H biome label, F3 debug overlay, L add light, K remove light | Place: {:?} (1-7) | Castle vX={:.1} (-/= adj, 0 stop) vY={:.1} ([/] adj, \\ stop)",
            hud_mode,
            if self.gs.walk_mode { "" } else { "+QE" },
            if self.gs.walk_mode {
                ", Space jump, Shift run"
            } else {
                ""
            },
            self.gs.place_type,
            self.gs.structure_speed,
            self.gs.structure_elev_speed,
        );
        d.draw_text(&hud, 12, 12, 18, Color::DARKGRAY);

        // Biome label moved to debug overlay above
        if !self.gs.show_debug_overlay {
            return;
        }
    }

    fn draw_overflow_hint(
        &self,
        d: &mut RaylibDrawHandle,
        frame: &WindowFrame,
        layout: ContentLayout,
    ) {
        if !layout.overflow() {
            return;
        }
        let font_size = 14;
        let text = if layout.overflow_items > 0 {
            format!("⋯ {} more", layout.overflow_items)
        } else {
            "⋯".to_string()
        };
        let content = frame.content;
        if content.w <= 0 || content.h <= 0 {
            return;
        }
        let text_w = d.measure_text(&text, font_size);
        let pad = 6;
        let box_w = text_w + pad * 2;
        let box_h = font_size + pad * 2;
        let x = content.x + content.w - box_w;
        let y = content.y + content.h - box_h;
        d.draw_rectangle(x, y, box_w, box_h, Color::new(12, 18, 28, 210));
        d.draw_rectangle_lines(x, y, box_w, box_h, Color::new(48, 64, 92, 220));
        d.draw_text(
            &text,
            x + pad,
            y + pad,
            font_size,
            Color::new(224, 234, 252, 255),
        );
    }
}

impl App {
    pub(super) fn minimap_side_px(view_radius_chunks: i32) -> i32 {
        if view_radius_chunks < 0 {
            return 0;
        }
        let radius = view_radius_chunks as f32;
        let side = 220.0 + radius * 16.0;
        let min_side = MINIMAP_MIN_CONTENT_SIDE as f32;
        let max_side = MINIMAP_MAX_CONTENT_SIDE as f32;
        side.clamp(min_side, max_side) as i32
    }

    fn render_minimap_to_texture(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        side_px: i32,
    ) {
        if side_px <= 0 {
            self.minimap_rt = None;
            return;
        }

        let radius = self.gs.view_radius_chunks.max(0);
        let center = self.gs.center_chunk;
        let spacing = 1.15_f32;
        let cube = 0.88_f32;
        let radius_f = radius.max(1) as f32;
        let zoom = self.minimap_zoom.clamp(0.3, 8.0);
        let yaw = self.minimap_yaw;
        let pitch = self.minimap_pitch.clamp(0.05, 1.5);

        #[derive(Clone, Copy)]
        struct MiniCell {
            pos: Vector3,
            color: Color,
            border: Color,
            has_above: bool,
            has_below: bool,
            is_center: bool,
        }

        let mut cells: Vec<MiniCell> = Vec::new();
        let to_u8 = |v: f32| -> u8 { v.clamp(0.0, 255.0) as u8 };

        for dy in -radius..=radius {
            for dz in -radius..=radius {
                for dx in -radius..=radius {
                    let dist_sq = dx * dx + dy * dy + dz * dz;
                    if dist_sq > radius * radius {
                        continue;
                    }
                    let coord = center.offset(dx, dy, dz);
                    let entry = self.gs.chunks.get(&coord);
                    let known_empty = entry
                        .map(|c| c.occupancy_or_empty().is_empty())
                        .unwrap_or(false);
                    let is_ready = self.gs.chunks.is_ready(coord);
                    let is_loaded = is_ready && !known_empty;
                    let is_center = dx == 0 && dy == 0 && dz == 0;
                    if known_empty && !is_center {
                        continue;
                    }
                    if !is_ready && !is_center {
                        continue;
                    }
                    let mesh_c = *self.gs.mesh_counts.get(&coord).unwrap_or(&0);
                    let light_c = *self.gs.light_counts.get(&coord).unwrap_or(&0);
                    let mesh_heat = (mesh_c.min(16) as f32) / 16.0;
                    let light_heat = (light_c.min(16) as f32) / 16.0;
                    let dist_norm = if radius == 0 {
                        0.0
                    } else {
                        (dist_sq as f32).sqrt() / radius_f
                    };
                    let mut r = 55.0 + 130.0 * light_heat;
                    let mut g = 110.0 + 120.0 * mesh_heat;
                    let mut b = 140.0 + 80.0 * (1.0 - mesh_heat);
                    if dy > 0 {
                        b += 45.0;
                        g += 10.0;
                    } else if dy < 0 {
                        r += 50.0;
                        g -= 15.0;
                    }
                    let fade = 0.4 + 0.6 * (1.0 - dist_norm * 0.7);
                    r *= fade;
                    g *= fade;
                    b *= fade;
                    let alpha = if is_loaded { 230.0 } else { 130.0 };
                    let above_has_blocks = self
                        .gs
                        .chunks
                        .get(&coord.offset(0, 1, 0))
                        .map(|c| c.has_blocks())
                        .unwrap_or(false);
                    let below_has_blocks = self
                        .gs
                        .chunks
                        .get(&coord.offset(0, -1, 0))
                        .map(|c| c.has_blocks())
                        .unwrap_or(false);
                    let has_above = is_loaded && above_has_blocks;
                    let has_below = is_loaded && below_has_blocks;
                    let pos = Vector3::new(
                        dx as f32 * spacing,
                        dy as f32 * spacing,
                        dz as f32 * spacing,
                    );
                    cells.push(MiniCell {
                        pos,
                        color: Color::new(to_u8(r), to_u8(g), to_u8(b), to_u8(alpha)),
                        border: if is_loaded {
                            Color::new(220, 220, 240, 160)
                        } else {
                            Color::new(120, 120, 130, 120)
                        },
                        has_above,
                        has_below,
                        is_center,
                    });
                }
            }
        }

        if cells.is_empty() {
            cells.push(MiniCell {
                pos: Vector3::zero(),
                color: Color::new(70, 70, 90, 160),
                border: Color::new(180, 180, 200, 120),
                has_above: false,
                has_below: false,
                is_center: true,
            });
        }

        let needs_new = match self.minimap_rt {
            Some(ref rt) => rt.width() != side_px || rt.height() != side_px,
            None => true,
        };
        if needs_new {
            let side_u = side_px as u32;
            match rl.load_render_texture(thread, side_u, side_u) {
                Ok(rt) => self.minimap_rt = Some(rt),
                Err(e) => {
                    log::warn!("Failed to allocate minimap render texture: {}", e);
                    self.minimap_rt = None;
                    return;
                }
            }
        }

        let Some(minimap_rt) = self.minimap_rt.as_mut() else {
            return;
        };

        let max_pan = (radius as f32 + 1.0) * spacing;
        self.minimap_pan.x = self.minimap_pan.x.clamp(-max_pan, max_pan);
        self.minimap_pan.y = self.minimap_pan.y.clamp(-max_pan, max_pan);
        self.minimap_pan.z = self.minimap_pan.z.clamp(-max_pan, max_pan);
        let target = self.minimap_pan;

        {
            let mut td = rl.begin_texture_mode(thread, minimap_rt);
            td.clear_background(Color::new(0, 0, 0, 0));

            let orbit_base = (radius as f32 + 1.5).max(1.5) * spacing * 2.4 + 4.0;
            let orbit = (orbit_base / zoom).clamp(2.0, 160.0);
            let dir = Vector3::new(
                orbit * yaw.cos() * pitch.cos(),
                orbit * pitch.sin(),
                orbit * yaw.sin() * pitch.cos(),
            );
            let cam_pos = Vector3::new(target.x + dir.x, target.y + dir.y, target.z + dir.z);
            let up = Vector3::new(0.0, 1.0, 0.0);
            let camera = Camera3D::perspective(
                cam_pos,
                target,
                up,
                (35.0 / zoom.powf(0.25)).clamp(18.0, 55.0),
            );

            {
                let mut d3 = td.begin_mode3D(camera);
                let sphere_r = if radius == 0 {
                    spacing
                } else {
                    radius as f32 * spacing + cube * 0.6
                };
                d3.draw_sphere_wires(
                    Vector3::new(0.0, 0.0, 0.0),
                    sphere_r,
                    16,
                    16,
                    Color::new(120, 130, 165, 40),
                );
                for cell in &cells {
                    d3.draw_cube(cell.pos, cube, cube, cube, cell.color);
                    d3.draw_cube_wires(cell.pos, cube, cube, cube, cell.border);
                    if cell.has_above {
                        let top = Vector3::new(cell.pos.x, cell.pos.y + cube * 0.5, cell.pos.z);
                        let tip = Vector3::new(cell.pos.x, cell.pos.y + spacing * 0.5, cell.pos.z);
                        d3.draw_line_3D(top, tip, Color::new(64, 128, 255, 160));
                    }
                    if cell.has_below {
                        let bottom = Vector3::new(cell.pos.x, cell.pos.y - cube * 0.5, cell.pos.z);
                        let tip = Vector3::new(cell.pos.x, cell.pos.y - spacing * 0.5, cell.pos.z);
                        d3.draw_line_3D(bottom, tip, Color::new(255, 140, 88, 160));
                    }
                    if cell.is_center {
                        d3.draw_cube_wires(
                            cell.pos,
                            cube + 0.12,
                            cube + 0.12,
                            cube + 0.12,
                            Color::YELLOW,
                        );
                    }
                }
            }

            let center_px = side_px / 2;
            let cross = side_px as f32 * 0.45;
            td.draw_circle_lines(center_px, center_px, cross, Color::new(255, 255, 255, 36));
            let cross_i = cross as i32;
            td.draw_line(
                center_px - cross_i,
                center_px,
                center_px + cross_i,
                center_px,
                Color::new(255, 255, 255, 24),
            );
            td.draw_line(
                center_px,
                center_px - cross_i,
                center_px,
                center_px + cross_i,
                Color::new(255, 255, 255, 24),
            );
            td.draw_text(
                &format!("cy {}", center.cy),
                8,
                side_px - 26,
                16,
                Color::new(220, 220, 255, 220),
            );
        }
    }
}

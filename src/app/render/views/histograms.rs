use raylib::prelude::*;
use std::collections::VecDeque;

use super::super::format_count;
use super::super::{ContentLayout, DebugStats, GeistDraw, WindowFrame, WindowTheme};
use geist_world::{TERRAIN_STAGE_COUNT, TERRAIN_STAGE_LABELS};

pub(crate) struct EventHistogramView<'a> {
    total: usize,
    entries: &'a [(String, usize)],
}

pub(crate) struct IntentHistogramView<'a> {
    total: usize,
    by_cause: &'a [(String, usize)],
    by_radius: &'a [(String, usize)],
}

pub(crate) struct TerrainHistogramView<'a> {
    stage_us: &'a [VecDeque<u32>; TERRAIN_STAGE_COUNT],
    stage_calls: &'a [VecDeque<u32>; TERRAIN_STAGE_COUNT],
    height_tile_us: &'a VecDeque<u32>,
    height_tile_reused: &'a VecDeque<u32>,
    cache_hits: &'a VecDeque<u32>,
    cache_misses: &'a VecDeque<u32>,
    tile_cache_hits: &'a VecDeque<u32>,
    tile_cache_misses: &'a VecDeque<u32>,
    tile_cache_evictions: &'a VecDeque<u32>,
    tile_cache_entries: &'a VecDeque<u32>,
    chunk_total_us: &'a VecDeque<u32>,
    chunk_fill_us: &'a VecDeque<u32>,
    chunk_feature_us: &'a VecDeque<u32>,
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

    pub(crate) fn new(
        stage_us: &'a [VecDeque<u32>; TERRAIN_STAGE_COUNT],
        stage_calls: &'a [VecDeque<u32>; TERRAIN_STAGE_COUNT],
        height_tile_us: &'a VecDeque<u32>,
        height_tile_reused: &'a VecDeque<u32>,
        cache_hits: &'a VecDeque<u32>,
        cache_misses: &'a VecDeque<u32>,
        tile_cache_hits: &'a VecDeque<u32>,
        tile_cache_misses: &'a VecDeque<u32>,
        tile_cache_evictions: &'a VecDeque<u32>,
        tile_cache_entries: &'a VecDeque<u32>,
        chunk_total_us: &'a VecDeque<u32>,
        chunk_fill_us: &'a VecDeque<u32>,
        chunk_feature_us: &'a VecDeque<u32>,
    ) -> Self {
        Self {
            stage_us,
            stage_calls,
            height_tile_us,
            height_tile_reused,
            cache_hits,
            cache_misses,
            tile_cache_hits,
            tile_cache_misses,
            tile_cache_evictions,
            tile_cache_entries,
            chunk_total_us,
            chunk_fill_us,
            chunk_feature_us,
        }
    }

    fn sample_window(&self) -> usize {
        self.stage_us
            .get(0)
            .map(|q| q.len())
            .unwrap_or_default()
            .max(self.height_tile_us.len())
            .max(self.chunk_total_us.len())
    }

    pub(crate) fn min_size(&self, theme: &WindowTheme) -> (i32, i32) {
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

    pub(crate) fn subtitle(&self) -> Option<String> {
        let window = self.sample_window();
        if window == 0 {
            None
        } else {
            Some(format!("{} samples", window))
        }
    }

    pub(crate) fn draw(
        &self,
        d: &mut GeistDraw,
        frame: &WindowFrame,
        theme: &WindowTheme,
    ) -> Option<ContentLayout> {
        let window = self.sample_window();
        if window == 0 {
            return None;
        }

        let content = frame.content;
        let pad_x = theme.padding_x;

        fn avg_p95(values: &[u32]) -> (f32, f32) {
            if values.is_empty() {
                return (0.0, 0.0);
            }
            let mut sorted = values.to_vec();
            sorted.sort_unstable();
            let sum: u64 = values.iter().map(|&v| v as u64).sum();
            let avg = (sum as f32 / values.len() as f32) / 1000.0;
            let p95_idx =
                ((values.len() as f32 * 0.95).ceil().max(1.0) as usize - 1).min(values.len() - 1);
            let p95 = sorted[p95_idx] as f32 / 1000.0;
            (avg, p95)
        }

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
        let tile_cache_hits_last = self.tile_cache_hits.back().copied().unwrap_or(0);
        let tile_cache_misses_last = self.tile_cache_misses.back().copied().unwrap_or(0);
        let tile_cache_total_last = tile_cache_hits_last.saturating_add(tile_cache_misses_last);
        let tile_cache_rate_last = if tile_cache_total_last == 0 {
            0.0
        } else {
            (tile_cache_hits_last as f32 / tile_cache_total_last as f32) * 100.0
        };
        let tile_cache_evictions_last = self.tile_cache_evictions.back().copied().unwrap_or(0);
        let tile_cache_entries_last = self.tile_cache_entries.back().copied().unwrap_or(0);

        let chunk_total_samples: Vec<u32> = self
            .chunk_total_us
            .iter()
            .copied()
            .filter(|&v| v > 0)
            .collect();
        let chunk_fill_samples: Vec<u32> = self
            .chunk_fill_us
            .iter()
            .copied()
            .filter(|&v| v > 0)
            .collect();
        let chunk_feature_samples: Vec<u32> = self
            .chunk_feature_us
            .iter()
            .copied()
            .filter(|&v| v > 0)
            .collect();
        let (chunk_avg_ms, chunk_p95_ms) = avg_p95(&chunk_total_samples);
        let (fill_avg_ms, fill_p95_ms) = avg_p95(&chunk_fill_samples);
        let (feature_avg_ms, feature_p95_ms) = avg_p95(&chunk_feature_samples);
        let last_chunk_us = self.chunk_total_us.back().copied().unwrap_or(0);
        let last_fill_us = self.chunk_fill_us.back().copied().unwrap_or(0);
        let last_feature_us = self.chunk_feature_us.back().copied().unwrap_or(0);
        let chunk_last_label = if last_chunk_us == 0 {
            "last: cached".to_string()
        } else {
            format!(
                "last: {:.2}ms (fill {:.2}ms)",
                last_chunk_us as f32 / 1000.0,
                last_fill_us as f32 / 1000.0,
            )
        };
        let feature_share_last = if last_chunk_us == 0 {
            0.0
        } else {
            (last_feature_us as f32 / last_chunk_us as f32 * 100.0).clamp(0.0, 100.0)
        };
        let mut feature_share_sum = 0.0_f32;
        let mut feature_share_count = 0.0_f32;
        for (&total, &feature) in self.chunk_total_us.iter().zip(self.chunk_feature_us.iter()) {
            if total == 0 {
                continue;
            }
            feature_share_sum += feature as f32 / total as f32;
            feature_share_count += 1.0;
        }
        let feature_share_avg = if feature_share_count > 0.0 {
            (feature_share_sum / feature_share_count * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
        let chunk_avg_label = if chunk_avg_ms == 0.0 && chunk_p95_ms == 0.0 {
            "avg: --   p95: --".to_string()
        } else {
            format!("avg: {:.2}ms   p95: {:.2}ms", chunk_avg_ms, chunk_p95_ms)
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

        max_span_ms = max_span_ms
            .max(chunk_p95_ms.max(chunk_avg_ms))
            .max(fill_p95_ms.max(fill_avg_ms))
            .max(feature_p95_ms.max(feature_avg_ms));

        let mut table_rows: Vec<(&str, StageRowData)> = Vec::with_capacity(TERRAIN_STAGE_COUNT + 3);
        table_rows.push((
            "Chunk Total",
            StageRowData {
                avg_ms: chunk_avg_ms,
                p95_ms: chunk_p95_ms,
                last_ms: last_chunk_us as f32 / 1000.0,
                avg_calls: 0.0,
                last_calls: 0,
            },
        ));
        table_rows.push((
            "Voxel Fill",
            StageRowData {
                avg_ms: fill_avg_ms,
                p95_ms: fill_p95_ms,
                last_ms: last_fill_us as f32 / 1000.0,
                avg_calls: 0.0,
                last_calls: 0,
            },
        ));
        table_rows.push((
            "Features",
            StageRowData {
                avg_ms: feature_avg_ms,
                p95_ms: feature_p95_ms,
                last_ms: last_feature_us as f32 / 1000.0,
                avg_calls: 0.0,
                last_calls: 0,
            },
        ));
        for idx in 0..TERRAIN_STAGE_COUNT {
            table_rows.push((TERRAIN_STAGE_LABELS[idx], rows[idx]));
        }

        let mut layout = ContentLayout::new(content.h);
        let mut cursor_y = content.y;

        let card_gap = 14;
        let card_count = 3;
        let card_width =
            ((frame.outer.w - pad_x * 2 - card_gap * (card_count - 1)) / card_count).max(160);
        let card_height = Self::SUMMARY_CARD_HEIGHT;
        let card_bg = Color::new(24, 32, 44, 235);
        let card_outline = Color::new(52, 68, 84, 200);
        let accent_tile = Color::new(118, 202, 255, 220);
        let accent_cache = Color::new(124, 220, 184, 220);
        let accent_chunk = Color::new(248, 192, 132, 220);

        let card1_x = content.x;
        let card2_x = card1_x + card_width + card_gap;
        let card3_x = card2_x + card_width + card_gap;

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
        text_y2 += Self::SUBTITLE_FONT + 2;
        d.draw_text(
            &format!(
                "tile hit: {:.0}%   entries: {}",
                tile_cache_rate_last,
                format_count(tile_cache_entries_last as usize)
            ),
            text_x2,
            text_y2,
            Self::SUBTITLE_FONT,
            Color::new(186, 206, 198, 255),
        );
        text_y2 += Self::SUBTITLE_FONT + 2;
        d.draw_text(
            &format!(
                "tile totals: {} hits  {} miss  {} evict",
                format_count(tile_cache_hits_last as usize),
                format_count(tile_cache_misses_last as usize),
                format_count(tile_cache_evictions_last as usize)
            ),
            text_x2,
            text_y2,
            Self::SUBTITLE_FONT,
            Color::new(176, 196, 186, 255),
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

        d.draw_rectangle(card3_x, cursor_y, card_width, card_height, card_bg);
        d.draw_rectangle_lines(card3_x, cursor_y, card_width, card_height, card_outline);
        d.draw_rectangle(card3_x, cursor_y, card_width, 2, accent_chunk);
        let text_x3 = card3_x + 12;
        let mut text_y3 = cursor_y + 10;
        d.draw_text(
            "Chunk Build",
            text_x3,
            text_y3,
            Self::CARD_HEADER_FONT,
            Color::new(250, 236, 220, 255),
        );
        text_y3 += Self::CARD_HEADER_FONT + 4;
        d.draw_text(
            &chunk_last_label,
            text_x3,
            text_y3,
            Self::SUBTITLE_FONT,
            Color::new(246, 230, 210, 255),
        );
        text_y3 += Self::SUBTITLE_FONT + 2;
        d.draw_text(
            &chunk_avg_label,
            text_x3,
            text_y3,
            Self::SUBTITLE_FONT,
            Color::new(236, 222, 206, 255),
        );
        text_y3 += Self::SUBTITLE_FONT + 2;
        d.draw_text(
            &format!(
                "features: last {:.0}%   avg {:.0}%",
                feature_share_last, feature_share_avg
            ),
            text_x3,
            text_y3,
            Self::SUBTITLE_FONT,
            Color::new(234, 214, 194, 255),
        );
        text_y3 += Self::SUBTITLE_FONT + 4;
        let chunk_bar_x = text_x3;
        let chunk_bar_width = card_width - 24;
        let chunk_bar_height = 10;
        d.draw_rectangle(
            chunk_bar_x,
            text_y3,
            chunk_bar_width,
            chunk_bar_height,
            Color::new(28, 26, 20, 255),
        );
        let chunk_fill =
            ((feature_share_last / 100.0).clamp(0.0, 1.0) * chunk_bar_width as f32).round() as i32;
        if chunk_fill > 0 {
            d.draw_rectangle(
                chunk_bar_x,
                text_y3,
                chunk_fill.max(2),
                chunk_bar_height,
                accent_chunk,
            );
        }

        cursor_y += card_height + Self::SUMMARY_GAP;
        layout.add_custom(card_height + Self::SUMMARY_GAP);

        let zebra_width = frame.outer.w - pad_x * 2;
        let bar_x = content.x + Self::LABEL_WIDTH + Self::GAP_X;
        let bar_width = (frame.outer.w - (pad_x * 2 + Self::LABEL_WIDTH + Self::GAP_X)).max(160);
        let bar_height = (Self::ROW_HEIGHT - 10).max(8);
        for (idx, (label, row)) in table_rows.iter().enumerate() {
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

            let avg_call_text = match *label {
                "Chunk Total" => format!("samples {}", self.chunk_total_us.len()),
                "Voxel Fill" => format!("samples {}", self.chunk_fill_us.len()),
                "Features" => format!("samples {}", self.chunk_feature_us.len()),
                _ => format!("avg {:.1} | last {}", row.avg_calls, row.last_calls),
            };
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

        layout.add_rows(table_rows.len(), Self::ROW_HEIGHT);
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

    pub(crate) fn new(stats: &'a DebugStats) -> Self {
        Self {
            total: stats.intents_size,
            by_cause: &stats.intents_by_cause,
            by_radius: &stats.intents_by_radius,
        }
    }

    pub(crate) fn min_size(&self, theme: &WindowTheme) -> (i32, i32) {
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

    pub(crate) fn subtitle(&self) -> Option<String> {
        Some(format!("{} pending", self.total))
    }

    pub(crate) fn draw(
        &self,
        d: &mut GeistDraw,
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

    pub(crate) fn new(stats: &'a DebugStats) -> Self {
        Self {
            total: stats.queued_events_total,
            entries: &stats.queued_events_by,
        }
    }

    pub(crate) fn min_size(&self, theme: &WindowTheme) -> (i32, i32) {
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

    pub(crate) fn subtitle(&self) -> Option<String> {
        Some(format!("{} pending", self.total))
    }

    pub(crate) fn draw(
        &self,
        d: &mut GeistDraw,
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

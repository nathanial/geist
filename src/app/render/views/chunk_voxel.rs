use raylib::prelude::Color;
use std::collections::HashMap;

use super::super::{
    App, ContentLayout, DisplayLine, GeistDraw, WindowFrame, WindowTheme, draw_lines, format_count,
};
use geist_blocks::Block;

pub(crate) struct ChunkVoxelView {
    lines: Vec<DisplayLine>,
    subtitle: Option<String>,
}

impl ChunkVoxelView {
    const MIN_WIDTH: i32 = 420;

    pub(crate) fn new(app: &App) -> Self {
        let mut lines = Vec::new();
        let center = app.gs.center_chunk;
        lines.push(
            DisplayLine::new(
                format!("Chunk ({}, {}, {}) voxels", center.cx, center.cy, center.cz),
                20,
                Color::new(236, 244, 255, 255),
            )
            .with_line_height(26),
        );

        let subtitle = match app.gs.chunks.get(&center) {
            None => {
                lines.push(
                    DisplayLine::new(
                        "Chunk is not yet loaded; waiting for streaming",
                        16,
                        Color::new(210, 220, 240, 255),
                    )
                    .with_line_height(22),
                );
                Some("stream pending".to_string())
            }
            Some(entry) => {
                if let Some(buf) = entry.buf.as_ref() {
                    let mut counts: HashMap<(u16, u16), usize> = HashMap::new();
                    for block in &buf.blocks {
                        if *block == Block::AIR {
                            continue;
                        }
                        *counts.entry((block.id, block.state)).or_default() += 1;
                    }
                    let mut total: usize = counts.values().copied().sum();
                    let overrides = app
                        .gs
                        .edits
                        .snapshot_for_chunk(center.cx, center.cy, center.cz);
                    let mut overrides_applied = 0usize;
                    if !overrides.is_empty() {
                        let base_x = center.cx * buf.sx as i32;
                        let base_y = center.cy * buf.sy as i32;
                        let base_z = center.cz * buf.sz as i32;
                        for ((wx, wy, wz), override_block) in overrides.iter().copied() {
                            let lx = wx - base_x;
                            let ly = wy - base_y;
                            let lz = wz - base_z;
                            if lx >= 0
                                && ly >= 0
                                && lz >= 0
                                && lx < buf.sx as i32
                                && ly < buf.sy as i32
                                && lz < buf.sz as i32
                            {
                                let old_block =
                                    buf.get_local(lx as usize, ly as usize, lz as usize);
                                if old_block != Block::AIR {
                                    if let Some(count) =
                                        counts.get_mut(&(old_block.id, old_block.state))
                                    {
                                        if *count > 0 {
                                            *count -= 1;
                                            total = total.saturating_sub(1);
                                        }
                                    }
                                }
                            }
                            if override_block != Block::AIR {
                                *counts
                                    .entry((override_block.id, override_block.state))
                                    .or_default() += 1;
                                total += 1;
                            }
                            overrides_applied += 1;
                        }
                        counts.retain(|_, count| *count > 0);
                    }

                    if total == 0 {
                        lines.push(
                            DisplayLine::new(
                                "Chunk contains only air",
                                16,
                                Color::new(210, 220, 240, 255),
                            )
                            .with_line_height(22),
                        );
                        if overrides_applied > 0 {
                            lines.push(
                                DisplayLine::new(
                                    format!(
                                        "{} edit overrides currently applied",
                                        overrides_applied
                                    ),
                                    14,
                                    Color::new(200, 188, 228, 255),
                                )
                                .with_line_height(20),
                            );
                        }
                        Some("air only".to_string())
                    } else {
                        let mut per_block_totals: HashMap<u16, usize> = HashMap::new();
                        let mut variant_counts: HashMap<u16, usize> = HashMap::new();
                        for ((block_id, _state), count) in &counts {
                            if *count == 0 {
                                continue;
                            }
                            *per_block_totals.entry(*block_id).or_default() += *count;
                            variant_counts
                                .entry(*block_id)
                                .and_modify(|v| *v += 1)
                                .or_insert(1);
                        }

                        let unique_types = per_block_totals.len();
                        lines.push(
                            DisplayLine::new(
                                format!(
                                    "{} blocks across {} block types",
                                    format_count(total),
                                    unique_types
                                ),
                                16,
                                Color::new(206, 220, 240, 255),
                            )
                            .with_line_height(22),
                        );
                        if overrides_applied > 0 {
                            lines.push(
                                DisplayLine::new(
                                    format!(
                                        "{} edit overrides currently applied",
                                        overrides_applied
                                    ),
                                    14,
                                    Color::new(200, 188, 228, 255),
                                )
                                .with_line_height(20),
                            );
                        }

                        struct BlockEntry {
                            label: String,
                            known: bool,
                            count: usize,
                            variant_count: usize,
                        }

                        let mut blocks: Vec<BlockEntry> = per_block_totals
                            .into_iter()
                            .map(|(block_id, count)| {
                                let (label, known) = match app.reg.get(block_id) {
                                    Some(ty) => (ty.name.clone(), true),
                                    None => (format!("id {}", block_id), false),
                                };
                                BlockEntry {
                                    label,
                                    known,
                                    count,
                                    variant_count: variant_counts
                                        .get(&block_id)
                                        .copied()
                                        .unwrap_or(0),
                                }
                            })
                            .collect();

                        blocks.sort_by(|a, b| {
                            b.count.cmp(&a.count).then_with(|| a.label.cmp(&b.label))
                        });

                        const MAX_ROWS: usize = 80;
                        let mut truncated = 0usize;
                        if blocks.len() > MAX_ROWS {
                            truncated = blocks.len() - MAX_ROWS;
                            blocks.truncate(MAX_ROWS);
                        }

                        for (idx, entry) in blocks.iter().enumerate() {
                            let percent = (entry.count as f64 * 100.0) / (total as f64);
                            let color = if entry.known {
                                if idx == 0 {
                                    Color::new(255, 224, 178, 255)
                                } else {
                                    Color::new(206, 220, 240, 255)
                                }
                            } else {
                                Color::new(255, 182, 182, 255)
                            };
                            let state_suffix = if entry.variant_count > 1 {
                                format!(" ({} states)", entry.variant_count)
                            } else {
                                String::new()
                            };
                            lines.push(
                                DisplayLine::new(
                                    format!(
                                        "{:>5.1}% {} – {}{}",
                                        percent,
                                        format_count(entry.count),
                                        entry.label,
                                        state_suffix
                                    ),
                                    16,
                                    color,
                                )
                                .with_line_height(22),
                            );
                        }

                        if truncated > 0 {
                            lines.push(
                                DisplayLine::new(
                                    format!("… {} more block types", truncated),
                                    13,
                                    Color::new(178, 192, 214, 255),
                                )
                                .with_line_height(18),
                            );
                        }

                        Some(format!(
                            "types: {} blocks: {}",
                            unique_types,
                            format_count(total)
                        ))
                    }
                } else {
                    lines.push(
                        DisplayLine::new(
                            "Chunk mesh buffer not resident; voxel data unavailable",
                            16,
                            Color::new(214, 200, 214, 255),
                        )
                        .with_line_height(22),
                    );
                    if !entry.has_blocks() {
                        lines.push(
                            DisplayLine::new(
                                "Chunk was generated as empty",
                                14,
                                Color::new(200, 210, 226, 255),
                            )
                            .with_line_height(20),
                        );
                    }
                    Some("buffer unavailable".to_string())
                }
            }
        };

        Self { lines, subtitle }
    }

    pub(crate) fn min_size(&self, theme: &WindowTheme) -> (i32, i32) {
        let h = theme.titlebar_height + theme.padding_y * 2 + 280;
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

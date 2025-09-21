use raylib::prelude::Color;

use super::super::{
    App, ContentLayout, DisplayLine, GeistDraw, WindowFrame, WindowTheme, draw_lines,
};
use crate::app::{anchor_world_position, anchor_world_velocity, structure_world_to_local};
use crate::gamestate::WalkerAnchor;
use geist_render_raylib::conv::vec3_from_rl;

pub(crate) struct AttachmentDebugView {
    lines: Vec<DisplayLine>,
}

impl AttachmentDebugView {
    const MIN_WIDTH: i32 = 520;

    pub(crate) fn new(app: &App) -> Self {
        let mut lines = Vec::new();
        if let WalkerAnchor::Structure(anchor) = app.gs.anchor {
            lines.push(
                DisplayLine::new(
                    format!("Attached to structure ID {}", anchor.id),
                    16,
                    Color::GREEN,
                )
                .with_line_height(20),
            );
            lines.push(
                DisplayLine::new(
                    format!("Grace period: {}", anchor.grace),
                    15,
                    Color::new(156, 212, 178, 255),
                )
                .with_indent(18),
            );
            lines.push(
                DisplayLine::new(
                    format!(
                        "Local offset: ({:.2}, {:.2}, {:.2})",
                        anchor.local_pos.x, anchor.local_pos.y, anchor.local_pos.z
                    ),
                    15,
                    Color::new(156, 212, 178, 255),
                )
                .with_indent(18),
            );
            lines.push(
                DisplayLine::new(
                    format!(
                        "Local velocity: ({:.2}, {:.2}, {:.2})",
                        anchor.local_vel.x, anchor.local_vel.y, anchor.local_vel.z
                    ),
                    15,
                    Color::new(156, 212, 178, 255),
                )
                .with_indent(18),
            );
            lines.push(
                DisplayLine::new(
                    format!("Yaw offset: {:.1}°", anchor.yaw_offset),
                    15,
                    Color::new(156, 212, 178, 255),
                )
                .with_indent(18),
            );
            if let Some(st) = app.gs.structures.get(&anchor.id) {
                let inferred_world = anchor_world_position(&anchor, st);
                lines.push(
                    DisplayLine::new(
                        format!(
                            "Frame→world: ({:.2}, {:.2}, {:.2})",
                            inferred_world.x, inferred_world.y, inferred_world.z
                        ),
                        15,
                        Color::new(156, 212, 178, 255),
                    )
                    .with_indent(18),
                );
                lines.push(
                    DisplayLine::new(
                        format!(
                            "World velocity: ({:.2}, {:.2}, {:.2})",
                            st.last_velocity.x, st.last_velocity.y, st.last_velocity.z
                        ),
                        15,
                        Color::new(156, 212, 178, 255),
                    )
                    .with_indent(18),
                );
                lines.push(
                    DisplayLine::new(
                        format!("World yaw: {:.1}°", anchor.world_yaw(st)),
                        15,
                        Color::new(156, 212, 178, 255),
                    )
                    .with_indent(18),
                );
                let handoff = anchor_world_velocity(&anchor, st);
                lines.push(
                    DisplayLine::new(
                        format!(
                            "Detach handoff vel: ({:.2}, {:.2}, {:.2})",
                            handoff.x, handoff.y, handoff.z
                        ),
                        15,
                        Color::new(156, 212, 178, 255),
                    )
                    .with_indent(18),
                );
            } else {
                lines.push(
                    DisplayLine::new(
                        "Structure reference missing",
                        15,
                        Color::new(214, 160, 160, 255),
                    )
                    .with_indent(18),
                );
            }
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
            let local = structure_world_to_local(walker, st.pose.pos, st.pose.yaw_deg);
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

    pub(crate) fn min_size(&self, theme: &WindowTheme) -> (i32, i32) {
        let height: i32 = self.lines.iter().map(|line| line.line_height).sum();
        let min_height = theme.titlebar_height + height + theme.padding_y * 2;
        let h = min_height.max(theme.titlebar_height + theme.padding_y * 2 + 240);
        let w = theme.padding_x * 2 + Self::MIN_WIDTH;
        (w, h)
    }

    pub(crate) fn draw(&self, d: &mut GeistDraw, frame: &WindowFrame) -> ContentLayout {
        draw_lines(d, &self.lines, frame)
    }
}

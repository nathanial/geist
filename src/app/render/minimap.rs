use raylib::prelude::*;

use super::App;

pub(crate) const MINIMAP_MIN_CONTENT_SIDE: i32 = 200;
pub(crate) const MINIMAP_MAX_CONTENT_SIDE: i32 = 420;
pub(crate) const MINIMAP_BORDER_PX: i32 = 10;

impl App {
    pub(crate) fn minimap_side_px(view_radius_chunks: i32) -> i32 {
        if view_radius_chunks < 0 {
            return 0;
        }
        let radius = view_radius_chunks as f32;
        let side = 220.0 + radius * 16.0;
        let min_side = MINIMAP_MIN_CONTENT_SIDE as f32;
        let max_side = MINIMAP_MAX_CONTENT_SIDE as f32;
        side.clamp(min_side, max_side) as i32
    }

    pub(super) fn render_minimap_to_texture(
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
            let text = format!("cy {}", center.cy);
            let color = Color::new(220, 220, 255, 220);
            let pos = Vector2::new(8.0, (side_px - 26) as f32);
            if let Some(font) = self.ui_font.as_ref() {
                let size = 16.0_f32;
                let spacing = size / font.base_size().max(1) as f32;
                td.draw_text_ex(&**font, &text, pos, size, spacing, color);
            } else {
                td.draw_text(&text, pos.x as i32, pos.y as i32, 16, color);
            }
        }
    }
}

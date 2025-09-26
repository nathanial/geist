use raylib::prelude::*;

use super::super::{App, GeistDraw};
use crate::app::DayLightSample;
use crate::camera::Frustum;
use crate::raycast;
use geist_blocks::Block;
use geist_chunk::ChunkOccupancy;
use geist_render_raylib::conv::vec3_to_rl;
use geist_structures::StructureId;
use geist_world::ChunkCoord;

pub(super) fn surface_color(surface_sky: [f32; 3]) -> Color {
    Color::new(
        (surface_sky[0] * 255.0) as u8,
        (surface_sky[1] * 255.0) as u8,
        (surface_sky[2] * 255.0) as u8,
        255,
    )
}

pub(super) fn sun_tint_color(sample: DayLightSample) -> Color {
    let warm = [1.0, 0.84, 0.42];
    let ember = [1.0, 0.58, 0.28];
    let twilight = sample.phase.cos().abs().clamp(0.0, 1.0);
    let ember_mix = twilight.powf(1.5);
    let warm_mix = 1.0 - ember_mix;
    let base = [
        warm[0] * warm_mix + ember[0] * ember_mix,
        warm[1] * warm_mix + ember[1] * ember_mix,
        warm[2] * warm_mix + ember[2] * ember_mix,
    ];
    let visibility = if sample.sun_visible { 1.0 } else { 0.15 };
    let brightness = ((0.2 + 0.8 * sample.sky_scale) * visibility).clamp(0.0, 1.0);
    let r = (base[0] * brightness * 255.0).clamp(0.0, 255.0) as u8;
    let g = (base[1] * brightness * 255.0).clamp(0.0, 255.0) as u8;
    let b = (base[2] * brightness * 255.0).clamp(0.0, 255.0) as u8;
    Color::new(r, g, b, 255)
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_world_scene(
        &mut self,
        d: &mut GeistDraw,
        thread: &RaylibThread,
        camera3d: Camera3D,
        frustum: &Frustum,
        time_now: f32,
        sky_scale: f32,
        surface_sky: [f32; 3],
        sun_id: Option<StructureId>,
        sun_tint: Color,
    ) {
        let mut d3 = d.begin_mode3D(camera3d);
        if self.gs.show_grid {
            d3.draw_grid(64, 1.0);
        }

        let p_cam = self.cam.position;
        let wx = p_cam.x.floor() as i32;
        let wy = p_cam.y.floor() as i32;
        let wz = p_cam.z.floor() as i32;
        let b_cam = if let Some(edit) = self.gs.edits.get(wx, wy, wz) {
            edit
        } else {
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

        let cave_fog = [0.0, 0.0, 0.0];
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
        let fog_start = if underwater { 4.0 } else { 64.0 as f32 };
        let fog_end = if underwater {
            48.0
        } else {
            64.0 * self.gs.view_radius_chunks as f32
        };
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

        let mut visible_chunks: Vec<(ChunkCoord, f32)> = Vec::new();
        for (ckey, cr) in self.renders.iter() {
            if self.gs.frustum_culling_enabled && !frustum.contains_bounding_box(&cr.bbox) {
                self.debug_stats.chunks_culled += 1;
                continue;
            }

            self.debug_stats.chunks_rendered += 1;
            let center = (cr.bbox.min + cr.bbox.max) * 0.5;
            let dx = center.x - self.cam.position.x;
            let dy = center.y - self.cam.position.y;
            let dz = center.z - self.cam.position.z;
            let dist2 = dx * dx + dy * dy + dz * dz;
            visible_chunks.push((*ckey, dist2));
            let origin = cr.origin;
            let vis_min = 18.0f32 / 255.0f32;
            let (dims_some, grid_some) = if let Some(ref lt) = cr.light_tex {
                ((lt.sx, lt.sy, lt.sz), (lt.grid_cols, lt.grid_rows))
            } else {
                ((0, 0, 0), (0, 0))
            };
            if let Some(ref mut ls) = self.leaves_shader {
                if let Some(t) = cr.leaf_tint {
                    let p0 = t;
                    let p1 = [t[0] * 0.85, t[1] * 0.85, t[2] * 0.85];
                    let p2 = [t[0] * 0.7, t[1] * 0.7, t[2] * 0.7];
                    let p3 = [t[0] * 0.5, t[1] * 0.5, t[2] * 0.5];
                    ls.set_autumn_palette(p0, p1, p2, p3, 1.0);
                } else {
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

        let mut visible_structs: Vec<(StructureId, f32)> = Vec::new();
        for (id, cr) in &self.structure_renders {
            if let Some(st) = self.gs.structures.get(id) {
                let translated_bbox = raylib::core::math::BoundingBox {
                    min: cr.bbox.min + vec3_to_rl(st.pose.pos),
                    max: cr.bbox.max + vec3_to_rl(st.pose.pos),
                };

                if self.gs.frustum_culling_enabled
                    && !frustum.contains_bounding_box(&translated_bbox)
                {
                    self.debug_stats.structures_culled += 1;
                    continue;
                }

                self.debug_stats.structures_rendered += 1;
                let center = (translated_bbox.min + translated_bbox.max) * 0.5;
                let dx = center.x - self.cam.position.x;
                let dy = center.y - self.cam.position.y;
                let dz = center.z - self.cam.position.z;
                let dist2 = dx * dx + dy * dy + dz * dz;
                visible_structs.push((*id, dist2));
                let origin_world = [
                    cr.origin[0] + st.pose.pos.x,
                    cr.origin[1] + st.pose.pos.y,
                    cr.origin[2] + st.pose.pos.z,
                ];
                let vis_min = 18.0f32 / 255.0f32;
                let (dims_some, grid_some) = if let Some(ref lt) = cr.light_tex {
                    ((lt.sx, lt.sy, lt.sz), (lt.grid_cols, lt.grid_rows))
                } else {
                    ((0, 0, 0), (0, 0))
                };
                for part in &cr.parts {
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
                        match tag {
                            Some("leaves") => {
                                if let Some(ref mut ls) = self.leaves_shader {
                                    if let Some(ref lt) = cr.light_tex {
                                        ls.update_chunk_uniforms(
                                            thread,
                                            &lt.tex,
                                            dims_some,
                                            grid_some,
                                            origin_world,
                                            vis_min,
                                        );
                                    } else {
                                        ls.update_chunk_uniforms_no_tex(
                                            thread,
                                            dims_some,
                                            grid_some,
                                            origin_world,
                                            vis_min,
                                        );
                                    }
                                }
                            }
                            _ => {
                                if let Some(ref mut fs) = self.fog_shader {
                                    if let Some(ref lt) = cr.light_tex {
                                        fs.update_chunk_uniforms(
                                            thread,
                                            &lt.tex,
                                            dims_some,
                                            grid_some,
                                            origin_world,
                                            vis_min,
                                        );
                                    } else {
                                        fs.update_chunk_uniforms_no_tex(
                                            thread,
                                            dims_some,
                                            grid_some,
                                            origin_world,
                                            vis_min,
                                        );
                                    }
                                }
                            }
                        }
                        self.debug_stats.draw_calls += 1;
                        let tint = if Some(*id) == sun_id {
                            sun_tint
                        } else {
                            Color::WHITE
                        };
                        d3.draw_model(&part.model, vec3_to_rl(st.pose.pos), 1.0, tint);
                    }
                }
            }
        }

        unsafe {
            raylib::ffi::rlDisableDepthMask();
        }
        visible_chunks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for (ckey, _) in &visible_chunks {
            if let Some(cr) = self.renders.get(ckey) {
                if self.gs.frustum_culling_enabled && !frustum.contains_bounding_box(&cr.bbox) {
                    continue;
                }
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

        visible_structs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for (sid, _) in &visible_structs {
            if let Some(cr) = self.structure_renders.get(sid) {
                if let Some(st) = self.gs.structures.get(sid) {
                    let translated_bbox = raylib::core::math::BoundingBox {
                        min: cr.bbox.min + vec3_to_rl(st.pose.pos),
                        max: cr.bbox.max + vec3_to_rl(st.pose.pos),
                    };
                    if self.gs.frustum_culling_enabled
                        && !frustum.contains_bounding_box(&translated_bbox)
                    {
                        continue;
                    }
                    let origin_world = [
                        cr.origin[0] + st.pose.pos.x,
                        cr.origin[1] + st.pose.pos.y,
                        cr.origin[2] + st.pose.pos.z,
                    ];
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
                            if let Some(ref mut ws) = self.water_shader {
                                if let Some(ref lt) = cr.light_tex {
                                    ws.update_chunk_uniforms(
                                        thread,
                                        &lt.tex,
                                        dims_some,
                                        grid_some,
                                        origin_world,
                                        vis_min,
                                    );
                                } else {
                                    ws.update_chunk_uniforms_no_tex(
                                        thread,
                                        dims_some,
                                        grid_some,
                                        origin_world,
                                        vis_min,
                                    );
                                }
                            }
                            self.debug_stats.draw_calls += 1;
                            unsafe {
                                raylib::ffi::rlDisableBackfaceCulling();
                            }
                            let tint = if Some(*sid) == sun_id {
                                sun_tint
                            } else {
                                Color::WHITE
                            };
                            d3.draw_model(&part.model, vec3_to_rl(st.pose.pos), 1.0, tint);
                            unsafe {
                                raylib::ffi::rlEnableBackfaceCulling();
                            }
                        }
                    }
                }
            }
        }
        unsafe {
            raylib::ffi::rlEnableDepthMask();
        }

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
}

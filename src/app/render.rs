use raylib::prelude::*;
use std::collections::HashSet;

use super::{App, DebugStats};
use crate::raycast;
use geist_blocks::Block;
use geist_chunk::ChunkOccupancy;
use geist_geom::Vec3;
use geist_render_raylib::conv::{vec3_from_rl, vec3_to_rl};
use geist_structures::{StructureId, rotate_yaw_inv};
use geist_world::ChunkCoord;

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
        let minimap_side_px = App::minimap_side_px(self.gs.view_radius_chunks);
        self.render_minimap_to_texture(rl, thread, minimap_side_px);
        self.minimap_ui_rect = None;
        self.event_histogram_rect = None;
        self.intent_histogram_rect = None;
        self.height_histogram_rect = None;
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
            // Debug overlay (lower left)
            let fps = d.get_fps();
            let mut debug_text = format!(
                "FPS: {}\nVertices: {}\nTriangles: {}\nChunks: {} (culled: {})\nStructures: {} (culled: {})\nDraw Calls: {}",
                fps,
                self.debug_stats.total_vertices,
                self.debug_stats.total_triangles,
                self.debug_stats.chunks_rendered,
                self.debug_stats.chunks_culled,
                self.debug_stats.structures_rendered,
                self.debug_stats.structures_culled,
                self.debug_stats.draw_calls
            );
            let mut text_lines = 6; // Base number of lines in debug text
            let center_chunk = self.gs.center_chunk;
            debug_text.push_str(&format!(
                "\nCenter chunk: ({}, {}, {})",
                center_chunk.cx, center_chunk.cy, center_chunk.cz
            ));
            text_lines += 1;
            if self.gs.show_biome_label {
                let wx = self.cam.position.x.floor() as i32;
                let wz = self.cam.position.z.floor() as i32;
                if let Some(biome) = self.gs.world.biome_at(wx, wz) {
                    debug_text.push_str(&format!("\nBiome: {}", biome.name));
                    text_lines += 1;
                }
            }
            // (moved event stats to right-side overlay)
            let screen_height = d.get_screen_height();
            let line_height = 22; // Approximate height per line with font size 20
            let y_pos = screen_height - (text_lines * line_height) - 10; // 10px margin from bottom
            d.draw_text(&debug_text, 10, y_pos, 20, Color::WHITE);
            d.draw_text(&debug_text, 11, y_pos + 1, 20, Color::BLACK); // Shadow for readability

            // Right-side overlay (reduced to avoid jitter):
            // - No queued events line or subtype lists
            // - Keep processed total, intents, runtime queues, and perf summary
            let mut right_text = String::new();
            right_text.push_str(&format!(
                "Processed Events (session): {}",
                self.evt_processed_total
            ));
            right_text.push_str(&format!("\nIntents: {}", self.debug_stats.intents_size));
            // Show lighting mode (fixed)
            right_text.push_str("\nLighting: FullMicro");
            // Runtime queue debug (vertical layout)
            let (q_e, if_e, q_l, if_l, q_b, if_b) = self.runtime.queue_debug_counts();
            right_text.push_str("\nRuntime Queues:");
            right_text.push_str(&format!("\n  Edit  - q={} inflight={}", q_e, if_e));
            right_text.push_str(&format!("\n  Light - q={} inflight={}", q_l, if_l));
            right_text.push_str(&format!("\n  BG    - q={} inflight={}", q_b, if_b));

            right_text.push_str("\nChunks:");
            right_text.push_str(&format!(
                "\n  Loaded={} active={} nonempty={}",
                self.debug_stats.loaded_chunks,
                self.debug_stats.chunk_resident_total,
                self.debug_stats.chunk_resident_nonempty
            ));
            right_text.push_str(&format!(
                "\n  Axes  x={} y={} z={}",
                self.debug_stats.chunk_unique_cx,
                self.debug_stats.chunk_unique_cy,
                self.debug_stats.chunk_unique_cz
            ));
            right_text.push_str(&format!(
                "\n  GPU renders={}",
                self.debug_stats.render_cache_chunks
            ));

            right_text.push_str("\nLighting Store:");
            right_text.push_str(&format!(
                "\n  Borders={} Emitters={} Micro={}",
                self.debug_stats.lighting_border_chunks,
                self.debug_stats.lighting_emitter_chunks,
                self.debug_stats.lighting_micro_chunks
            ));

            right_text.push_str("\nEdit Store:");
            right_text.push_str(&format!(
                "\n  Chunks={} Blocks={} Rev={} Built={}",
                self.debug_stats.edit_chunk_entries,
                self.debug_stats.edit_block_edits,
                self.debug_stats.edit_rev_entries,
                self.debug_stats.edit_built_entries
            ));

            // Perf summary (rolling window average and p95)
            let stats = |q: &std::collections::VecDeque<u32>| -> (usize, u32, u32) {
                let n = q.len();
                if n == 0 {
                    return (0, 0, 0);
                }
                let sum: u64 = q.iter().map(|&v| v as u64).sum();
                let avg = ((sum as f32) / (n as f32)).round() as u32;
                let mut v: Vec<u32> = q.iter().copied().collect();
                v.sort_unstable();
                let idx = ((n as f32) * 0.95).ceil().max(1.0) as usize - 1;
                let p95 = v[idx.min(n - 1)];
                (n, avg, p95)
            };
            let (n_mesh, avg_mesh, p95_mesh) = stats(&self.perf_mesh_ms);
            let (n_light, avg_light, p95_light) = stats(&self.perf_light_ms);
            let (n_total, avg_total, p95_total) = stats(&self.perf_total_ms);
            let (n_rr, avg_rr, p95_rr) = stats(&self.perf_remove_ms);
            let (n_gen, avg_gen, p95_gen) = stats(&self.perf_gen_ms);
            let last_gen = self.perf_gen_ms.back().copied().unwrap_or(0);
            right_text.push_str("\nPerf (ms):");
            right_text.push_str(&format!(
                "\n  Mesh   avg={} p95={} n={}",
                avg_mesh, p95_mesh, n_mesh
            ));
            right_text.push_str(&format!(
                "\n  Light  avg={} p95={} n={}",
                avg_light, p95_light, n_light
            ));
            right_text.push_str(&format!(
                "\n  Total  avg={} p95={} n={}",
                avg_total, p95_total, n_total
            ));
            right_text.push_str(&format!(
                "\n  Remove->Render avg={} p95={} n={}",
                avg_rr, p95_rr, n_rr
            ));
            right_text.push_str(&format!(
                "\n  Load  last={} avg={} p95={} n={}",
                last_gen, avg_gen, p95_gen, n_gen
            ));

            let screen_width = d.get_screen_width();
            let font_size = 20;
            // Fixed panel width template samples
            let panel_templates = [
                "Processed Events (session): 1,000,000",
                "Intents: 1,000,000",
                "Lighting: FullMicro",
                "Runtime Queues:",
                "  Edit  - q=1,000,000 inflight=1,000,000",
                "  Light - q=1,000,000 inflight=1,000,000",
                "  BG    - q=1,000,000 inflight=1,000,000",
                "Chunks:",
                "  Loaded=1,000,000 active=1,000,000 nonempty=1,000,000",
                "  Axes  x=1,000,000 y=1,000,000 z=1,000,000",
                "  GPU renders=1,000,000",
                "Lighting Store:",
                "  Borders=1,000,000 Emitters=1,000,000 Micro=1,000,000",
                "Edit Store:",
                "  Chunks=1,000,000 Blocks=1,000,000 Rev=1,000,000 Built=1,000,000",
                "Perf (ms):",
                "  Mesh   avg=9,999 p95=9,999 n=9,999",
                "  Light  avg=9,999 p95=9,999 n=9,999",
                "  Total  avg=9,999 p95=9,999 n=9,999",
                "  Remove->Render avg=9,999 p95=9,999 n=9,999",
            ];
            let mut panel_w = 0;
            for t in panel_templates.iter() {
                let w = d.measure_text(t, font_size);
                if w > panel_w {
                    panel_w = w;
                }
            }
            // Small padding so text doesn't hug the edge
            panel_w += 8;
            let margin = 10;
            let rx = screen_width - panel_w - margin;
            // Align bottom similar to left overlay
            let lines = right_text.split('\n').count();
            let ry = screen_height - (lines as i32 * line_height) - 10;
            d.draw_text(&right_text, rx, ry, font_size, Color::WHITE);
            d.draw_text(&right_text, rx + 1, ry + 1, font_size, Color::BLACK);

            self.draw_event_histogram(&mut d);
            self.draw_intent_histogram(&mut d);
            self.draw_height_histogram(&mut d);

            // Minimap (bottom-right): draw the 3D chunk sphere render texture
            if minimap_side_px > 0 {
                if let Some(ref minimap_rt) = self.minimap_rt {
                    let pad: i32 = 8;
                    let map_side = minimap_side_px;
                    let map_w = map_side + pad * 2;
                    let map_h = map_side + pad * 2;
                    let margin: i32 = 10;
                    let scr_w: i32 = screen_width as i32;
                    let scr_h: i32 = screen_height as i32;
                    let mx = scr_w - map_w - margin;
                    let mut my = ry - map_h - 8; // 8px spacing above the right panel
                    if my < margin {
                        my = scr_h - map_h - margin;
                    }
                    d.draw_rectangle(mx, my, map_w, map_h, Color::new(0, 0, 0, 150));
                    d.draw_rectangle_lines(mx, my, map_w, map_h, Color::new(255, 255, 255, 40));
                    self.minimap_ui_rect = Some((mx, my, map_w, map_h));
                    let tex = minimap_rt.texture().clone();
                    let src = Rectangle::new(0.0, 0.0, tex.width() as f32, -(tex.height() as f32));
                    let dest = Rectangle::new(
                        (mx + pad) as f32,
                        (my + pad) as f32,
                        map_side as f32,
                        map_side as f32,
                    );
                    d.draw_texture_pro(tex, src, dest, Vector2::new(0.0, 0.0), 0.0, Color::WHITE);
                    let label = format!(
                        "Sphere r {} | Loaded {}",
                        self.gs.view_radius_chunks,
                        self.gs.chunks.ready_len()
                    );
                    let label_fs = 18;
                    let label_w = d.measure_text(&label, label_fs);
                    let mut label_x = mx + map_w - label_w - pad;
                    if label_x < mx + pad {
                        label_x = mx + pad;
                    }
                    let mut label_y = my - label_fs - 4;
                    if label_y < margin {
                        label_y = (my + map_h + 4).min(scr_h - label_fs - margin);
                    }
                    d.draw_text(&label, label_x + 1, label_y + 1, label_fs, Color::BLACK);
                    d.draw_text(&label, label_x, label_y, label_fs, Color::WHITE);

                    let legend = ["Scroll: zoom", "LMB drag: orbit", "Shift+Drag/RMB: pan"];
                    let legend_fs = 14;
                    let legend_h = (legend.len() as i32) * (legend_fs + 2);
                    let mut legend_y = my + map_h - pad - legend_h;
                    if legend_y < my + pad {
                        legend_y = my + pad;
                    }
                    for line in legend.iter() {
                        d.draw_text(
                            line,
                            mx + pad + 1,
                            legend_y + 1,
                            legend_fs,
                            Color::new(0, 0, 0, 200),
                        );
                        d.draw_text(
                            line,
                            mx + pad,
                            legend_y,
                            legend_fs,
                            Color::new(220, 220, 240, 240),
                        );
                        legend_y += legend_fs + 2;
                    }
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
        if self.gs.show_debug_overlay {
            d.draw_fps(12, 36);
        }

        // Biome label moved to debug overlay above
        if !self.gs.show_debug_overlay {
            return;
        }

        // Debug overlay for attachment status
        let mut debug_y = 60;
        d.draw_text("=== ATTACHMENT DEBUG ===", 12, debug_y, 16, Color::RED);
        debug_y += 20;

        // Show attachment status
        if let Some(att) = self.gs.ground_attach {
            d.draw_text(
                &format!("ATTACHED to structure ID: {}", att.id),
                12,
                debug_y,
                16,
                Color::GREEN,
            );
            debug_y += 18;
            d.draw_text(
                &format!("  Grace period: {}", att.grace),
                12,
                debug_y,
                16,
                Color::GREEN,
            );
            debug_y += 18;
            d.draw_text(
                &format!(
                    "  Local offset: ({:.2}, {:.2}, {:.2})",
                    att.local_offset.x, att.local_offset.y, att.local_offset.z
                ),
                12,
                debug_y,
                16,
                Color::GREEN,
            );
            debug_y += 18;
        } else {
            d.draw_text("NOT ATTACHED", 12, debug_y, 16, Color::ORANGE);
            debug_y += 18;
        }

        // Show walker position
        d.draw_text(
            &format!(
                "Walker pos: ({:.2}, {:.2}, {:.2})",
                self.gs.walker.pos.x, self.gs.walker.pos.y, self.gs.walker.pos.z
            ),
            12,
            debug_y,
            16,
            Color::DARKGRAY,
        );
        debug_y += 18;

        // Show on_ground status
        d.draw_text(
            &format!("On ground: {}", self.gs.walker.on_ground),
            12,
            debug_y,
            16,
            Color::DARKGRAY,
        );
        debug_y += 18;

        // Check each structure and show detection status
        for (id, st) in &self.gs.structures {
            let on_structure = self.is_feet_on_structure(st, self.gs.walker.pos);
            let color = if on_structure {
                Color::GREEN
            } else {
                Color::GRAY
            };
            d.draw_text(
                &format!(
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
                12,
                debug_y,
                16,
                color,
            );
            debug_y += 18;

            // Show detailed detection info
            let p = vec3_from_rl(self.gs.walker.pos);
            let diff = Vec3 {
                x: p.x - st.pose.pos.x,
                y: p.y - st.pose.pos.y,
                z: p.z - st.pose.pos.z,
            };
            let local = rotate_yaw_inv(diff, st.pose.yaw_deg);
            let test_y = local.y - 0.08;
            let lx = local.x.floor() as i32;
            let ly = test_y.floor() as i32;
            let lz = local.z.floor() as i32;

            d.draw_text(
                &format!(
                    "  Local: ({:.2}, {:.2}, {:.2}) Test Y: {:.2} -> Grid: ({}, {}, {})",
                    local.x, local.y, local.z, test_y, lx, ly, lz
                ),
                12,
                debug_y,
                14,
                color,
            );
            debug_y += 16;

            // Check if we're in bounds
            let in_bounds = lx >= 0
                && ly >= 0
                && lz >= 0
                && (lx as usize) < st.sx
                && (ly as usize) < st.sy
                && (lz as usize) < st.sz;

            // Get the actual block at this position (direct sample)
            let (block_at_pos, block_solid) = if in_bounds {
                // Check edits first
                if let Some(b) = st.edits.get(lx, ly, lz) {
                    (
                        format!("id:{} state:{} (edit)", b.id, b.state),
                        self.reg
                            .get(b.id)
                            .map(|ty| ty.is_solid(b.state))
                            .unwrap_or(false),
                    )
                } else {
                    // Check base blocks
                    let idx = st.idx(lx as usize, ly as usize, lz as usize);
                    let b = st.blocks[idx];
                    (
                        format!("id:{} state:{}", b.id, b.state),
                        self.reg
                            .get(b.id)
                            .map(|ty| ty.is_solid(b.state))
                            .unwrap_or(false),
                    )
                }
            } else {
                ("out of bounds".to_string(), false)
            };

            d.draw_text(
                &format!(
                    "  Bounds: 0..{} x 0..{} x 0..{} | In bounds: {}",
                    st.sx, st.sy, st.sz, in_bounds
                ),
                12,
                debug_y,
                14,
                color,
            );
            debug_y += 16;

            d.draw_text(
                &format!(
                    "  Block at ({},{},{}): {} | Solid: {}",
                    lx, ly, lz, block_at_pos, block_solid
                ),
                12,
                debug_y,
                14,
                color,
            );
            debug_y += 16;

            // Also show the block one cell below the sample (helps diagnose edge cases)
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
                            self.reg
                                .get(b.id)
                                .map(|ty| ty.is_solid(b.state))
                                .unwrap_or(false),
                        )
                    } else {
                        let idx = st.idx(lx as usize, by as usize, lz as usize);
                        let b = st.blocks[idx];
                        (
                            format!("id:{} state:{}", b.id, b.state),
                            self.reg
                                .get(b.id)
                                .map(|ty| ty.is_solid(b.state))
                                .unwrap_or(false),
                        )
                    }
                } else {
                    ("out of bounds".to_string(), false)
                };
                d.draw_text(
                    &format!(
                        "  Block at below ({},{},{}): {} | Solid: {}",
                        lx, by, lz, block_below, solid_below
                    ),
                    12,
                    debug_y,
                    14,
                    color,
                );
                debug_y += 16;
            }

            // Show deck info and check what's at deck level
            let deck_y = (st.sy as f32 * 0.33) as i32;
            d.draw_text(
                &format!("  Deck Y level: {} (expecting solid blocks here)", deck_y),
                12,
                debug_y,
                14,
                Color::BLUE,
            );
            debug_y += 16;

            // Debug: Check what's actually at the deck level at player's X,Z
            if lx >= 0 && lz >= 0 && (lx as usize) < st.sx && (lz as usize) < st.sz {
                let deck_idx = st.idx(lx as usize, deck_y as usize, lz as usize);
                let deck_block = st.blocks[deck_idx];
                d.draw_text(
                    &format!(
                        "  Block at deck level ({},{},{}): {:?}",
                        lx, deck_y, lz, deck_block
                    ),
                    12,
                    debug_y,
                    14,
                    Color::MAGENTA,
                );
                debug_y += 16;
            }
        }
    }
}

impl App {
    fn minimap_side_px(view_radius_chunks: i32) -> i32 {
        if view_radius_chunks < 0 {
            return 0;
        }
        let radius = view_radius_chunks as f32;
        let side = 220.0 + radius * 16.0;
        side.clamp(180.0, 420.0) as i32
    }

    fn draw_event_histogram(&mut self, d: &mut RaylibDrawHandle) {
        const MAX_ROWS: usize = 12;
        const PADDING_X: i32 = 14;
        const PADDING_Y: i32 = 12;
        const HEADER_HEIGHT: i32 = 28;
        const ROW_HEIGHT: i32 = 22;
        const LABEL_WIDTH: i32 = 210;
        const BAR_WIDTH: i32 = 220;
        const GAP_X: i32 = 12;
        const TITLE_FONT: i32 = 20;
        const ROW_FONT: i32 = 18;

        let total = self.debug_stats.queued_events_total;
        let all = &self.debug_stats.queued_events_by;
        let (entries, remainder) = if all.len() > MAX_ROWS {
            (&all[..MAX_ROWS], all.len() - MAX_ROWS)
        } else {
            (all.as_slice(), 0)
        };

        let bar_rows = entries.len().max(1) as i32;
        let base_height = PADDING_Y * 2 + HEADER_HEIGHT + bar_rows * ROW_HEIGHT;
        let height = if remainder > 0 {
            base_height + ROW_HEIGHT
        } else {
            base_height + 8
        };
        let width = PADDING_X * 2 + LABEL_WIDTH + GAP_X + BAR_WIDTH;

        let screen_w = d.get_screen_width();
        let screen_h = d.get_screen_height();
        let mut x = self.event_histogram_pos.x.round() as i32;
        let mut y = self.event_histogram_pos.y.round() as i32;
        let max_x = (screen_w - width - 10).max(10);
        let max_y = (screen_h - height - 10).max(10);
        x = x.clamp(10, max_x);
        y = y.clamp(10, max_y);
        self.event_histogram_pos = Vector2::new(x as f32, y as f32);
        self.event_histogram_rect = Some((x, y, width, height));
        self.event_histogram_size = (width, height);

        let bg_color = Color::new(16, 20, 32, 220);
        let border_color = Color::new(200, 215, 240, 140);
        d.draw_rectangle(x, y, width, height, bg_color);
        d.draw_rectangle_lines(x, y, width, height, border_color);

        let title = format!("Event Queue — {} pending", total);
        d.draw_text(
            &title,
            x + PADDING_X + 1,
            y + PADDING_Y + 1,
            TITLE_FONT,
            Color::BLACK,
        );
        d.draw_text(
            &title,
            x + PADDING_X,
            y + PADDING_Y,
            TITLE_FONT,
            Color::WHITE,
        );

        let bar_origin_y = y + PADDING_Y + HEADER_HEIGHT;
        if entries.is_empty() {
            let msg = "No queued events".to_owned();
            d.draw_text(
                &msg,
                x + PADDING_X + 1,
                bar_origin_y + 1,
                ROW_FONT,
                Color::BLACK,
            );
            d.draw_text(
                &msg,
                x + PADDING_X,
                bar_origin_y,
                ROW_FONT,
                Color::new(210, 215, 230, 255),
            );
        } else {
            let max_count = entries.iter().map(|entry| entry.1).max().unwrap_or(0);
            let max_count = if max_count == 0 {
                1.0
            } else {
                max_count as f32
            };
            for (idx, entry) in entries.iter().enumerate() {
                let row_top = bar_origin_y + (idx as i32) * ROW_HEIGHT;
                let label = entry.0.as_str();
                let count = entry.1;
                let label_shadow = Color::new(0, 0, 0, 180);
                let label_color = if idx == 0 {
                    Color::new(240, 240, 255, 255)
                } else {
                    Color::new(220, 225, 240, 255)
                };
                d.draw_text(
                    label,
                    x + PADDING_X + 1,
                    row_top + 1,
                    ROW_FONT,
                    label_shadow,
                );
                d.draw_text(label, x + PADDING_X, row_top, ROW_FONT, label_color);

                let bar_x = x + PADDING_X + LABEL_WIDTH + GAP_X;
                let bar_height = ROW_HEIGHT - 8;
                let bar_top = row_top + (ROW_HEIGHT - bar_height) / 2;
                d.draw_rectangle(
                    bar_x,
                    bar_top,
                    BAR_WIDTH,
                    bar_height,
                    Color::new(34, 44, 58, 160),
                );

                let ratio = (count as f32) / max_count;
                let fill = (ratio * BAR_WIDTH as f32).round() as i32;
                if fill > 0 {
                    let fill_width = fill.max(2).min(BAR_WIDTH);
                    let fill_color = match idx {
                        0 => Color::new(118, 202, 255, 230),
                        1 => Color::new(96, 186, 250, 220),
                        2 => Color::new(82, 170, 240, 215),
                        _ => Color::new(68, 152, 222, 210),
                    };
                    d.draw_rectangle(bar_x, bar_top, fill_width, bar_height, fill_color);
                }

                let count_text = Self::format_count(count);
                let count_w = d.measure_text(&count_text, ROW_FONT);
                let count_x = bar_x + BAR_WIDTH - count_w;
                let count_y = row_top;
                d.draw_text(
                    &count_text,
                    count_x + 1,
                    count_y + 1,
                    ROW_FONT,
                    Color::BLACK,
                );
                d.draw_text(
                    &count_text,
                    count_x,
                    count_y,
                    ROW_FONT,
                    Color::new(236, 236, 248, 255),
                );
            }
        }

        if remainder > 0 {
            let summary = format!("… {} more types", remainder);
            let summary_y = bar_origin_y + bar_rows * ROW_HEIGHT;
            d.draw_text(
                &summary,
                x + PADDING_X + 1,
                summary_y + 1,
                ROW_FONT,
                Color::BLACK,
            );
            d.draw_text(
                &summary,
                x + PADDING_X,
                summary_y,
                ROW_FONT,
                Color::new(190, 195, 215, 255),
            );
        }
    }

    fn draw_intent_histogram(&mut self, d: &mut RaylibDrawHandle) {
        const MAX_CAUSE_ROWS: usize = 4;
        const MAX_RADIUS_ROWS: usize = 8;
        const PADDING_X: i32 = 14;
        const PADDING_Y: i32 = 12;
        const HEADER_HEIGHT: i32 = 28;
        const SECTION_HEADER_HEIGHT: i32 = 24;
        const SECTION_GAP: i32 = 12;
        const ROW_HEIGHT: i32 = 22;
        const LABEL_WIDTH: i32 = 210;
        const BAR_WIDTH: i32 = 220;
        const GAP_X: i32 = 12;
        const TITLE_FONT: i32 = 20;
        const SECTION_FONT: i32 = 18;
        const ROW_FONT: i32 = 18;

        let total = self.debug_stats.intents_size;
        let cause_entries = &self.debug_stats.intents_by_cause;
        let radius_entries = &self.debug_stats.intents_by_radius;

        let cause_display_count = cause_entries.len().min(MAX_CAUSE_ROWS);
        let cause_remainder = cause_entries.len().saturating_sub(cause_display_count);
        let cause_rows = cause_display_count.max(1) as i32;

        let radius_display_count = radius_entries.len().min(MAX_RADIUS_ROWS);
        let radius_remainder = radius_entries.len().saturating_sub(radius_display_count);
        let radius_rows = radius_display_count.max(1) as i32;

        let mut height = PADDING_Y * 2 + HEADER_HEIGHT;
        height += SECTION_HEADER_HEIGHT + cause_rows * ROW_HEIGHT;
        if cause_remainder > 0 {
            height += ROW_HEIGHT;
        }
        height += SECTION_GAP;
        height += SECTION_HEADER_HEIGHT + radius_rows * ROW_HEIGHT;
        if radius_remainder > 0 {
            height += ROW_HEIGHT;
        }
        height += 8;

        let width = PADDING_X * 2 + LABEL_WIDTH + GAP_X + BAR_WIDTH;

        let screen_w = d.get_screen_width();
        let screen_h = d.get_screen_height();
        let mut x = self.intent_histogram_pos.x.round() as i32;
        let mut y = self.intent_histogram_pos.y.round() as i32;
        let max_x = (screen_w - width - 10).max(10);
        let max_y = (screen_h - height - 10).max(10);
        x = x.clamp(10, max_x);
        y = y.clamp(10, max_y);
        self.intent_histogram_pos = Vector2::new(x as f32, y as f32);
        self.intent_histogram_rect = Some((x, y, width, height));
        self.intent_histogram_size = (width, height);

        let bg_color = Color::new(24, 18, 32, 220);
        let border_color = Color::new(210, 200, 235, 150);
        d.draw_rectangle(x, y, width, height, bg_color);
        d.draw_rectangle_lines(x, y, width, height, border_color);

        let title = format!("Intents Backlog — {} pending", total);
        d.draw_text(
            &title,
            x + PADDING_X + 1,
            y + PADDING_Y + 1,
            TITLE_FONT,
            Color::BLACK,
        );
        d.draw_text(
            &title,
            x + PADDING_X,
            y + PADDING_Y,
            TITLE_FONT,
            Color::WHITE,
        );

        let mut cursor_y = y + PADDING_Y + HEADER_HEIGHT;

        d.draw_text(
            "By Cause",
            x + PADDING_X,
            cursor_y,
            SECTION_FONT,
            Color::new(230, 230, 245, 255),
        );
        cursor_y += SECTION_HEADER_HEIGHT;

        if cause_display_count == 0 {
            let msg = if total == 0 {
                "No pending intents"
            } else {
                "No cause data"
            };
            d.draw_text(
                msg,
                x + PADDING_X,
                cursor_y,
                ROW_FONT,
                Color::new(210, 215, 230, 255),
            );
            cursor_y += ROW_HEIGHT;
        } else {
            let max_count = cause_entries
                .iter()
                .take(MAX_CAUSE_ROWS)
                .map(|(_, count)| *count)
                .max()
                .unwrap_or(1) as f32;
            for (idx, (label, count)) in cause_entries.iter().take(MAX_CAUSE_ROWS).enumerate() {
                let row_top = cursor_y + (idx as i32) * ROW_HEIGHT;
                let label = label.as_str();
                let count = *count;
                let bar_x = x + PADDING_X + LABEL_WIDTH + GAP_X;
                let bar_height = ROW_HEIGHT - 8;
                let bar_top = row_top + (ROW_HEIGHT - bar_height) / 2;
                d.draw_text(
                    label,
                    x + PADDING_X,
                    row_top,
                    ROW_FONT,
                    Color::new(220, 225, 240, 255),
                );
                d.draw_rectangle(
                    bar_x,
                    bar_top,
                    BAR_WIDTH,
                    bar_height,
                    Color::new(40, 40, 68, 160),
                );
                let ratio = (count as f32) / max_count;
                let fill = (ratio * BAR_WIDTH as f32).round() as i32;
                if fill > 0 {
                    let fill_width = fill.max(2).min(BAR_WIDTH);
                    let fill_color = match idx {
                        0 => Color::new(232, 140, 254, 230),
                        1 => Color::new(212, 116, 244, 220),
                        _ => Color::new(196, 96, 232, 210),
                    };
                    d.draw_rectangle(bar_x, bar_top, fill_width, bar_height, fill_color);
                }
                let count_text = Self::format_count(count);
                let count_w = d.measure_text(&count_text, ROW_FONT);
                d.draw_text(
                    &count_text,
                    bar_x + BAR_WIDTH - count_w,
                    row_top,
                    ROW_FONT,
                    Color::new(240, 240, 255, 255),
                );
            }
            cursor_y += cause_rows * ROW_HEIGHT;
        }

        if cause_remainder > 0 {
            let summary = format!("… {} more causes", cause_remainder);
            d.draw_text(
                &summary,
                x + PADDING_X,
                cursor_y,
                ROW_FONT,
                Color::new(200, 205, 220, 255),
            );
            cursor_y += ROW_HEIGHT;
        }

        cursor_y += SECTION_GAP;
        d.draw_text(
            "By Radius (chunks)",
            x + PADDING_X,
            cursor_y,
            SECTION_FONT,
            Color::new(230, 230, 245, 255),
        );
        cursor_y += SECTION_HEADER_HEIGHT;

        if radius_display_count == 0 {
            let msg = if total == 0 {
                "No pending intents"
            } else {
                "No radius data"
            };
            d.draw_text(
                msg,
                x + PADDING_X,
                cursor_y,
                ROW_FONT,
                Color::new(210, 215, 230, 255),
            );
            cursor_y += ROW_HEIGHT;
        } else {
            let max_count = radius_entries
                .iter()
                .take(MAX_RADIUS_ROWS)
                .map(|(_, count)| *count)
                .max()
                .unwrap_or(1) as f32;
            for (idx, (label, count)) in radius_entries.iter().take(MAX_RADIUS_ROWS).enumerate() {
                let row_top = cursor_y + (idx as i32) * ROW_HEIGHT;
                let label = label.as_str();
                let count = *count;
                let bar_x = x + PADDING_X + LABEL_WIDTH + GAP_X;
                let bar_height = ROW_HEIGHT - 8;
                let bar_top = row_top + (ROW_HEIGHT - bar_height) / 2;
                d.draw_text(
                    label,
                    x + PADDING_X,
                    row_top,
                    ROW_FONT,
                    Color::new(210, 225, 240, 255),
                );
                d.draw_rectangle(
                    bar_x,
                    bar_top,
                    BAR_WIDTH,
                    bar_height,
                    Color::new(32, 52, 72, 150),
                );
                let ratio = (count as f32) / max_count;
                let fill = (ratio * BAR_WIDTH as f32).round() as i32;
                if fill > 0 {
                    let fill_width = fill.max(2).min(BAR_WIDTH);
                    let fill_color = match idx {
                        0 => Color::new(118, 202, 255, 230),
                        1 => Color::new(96, 186, 250, 220),
                        2 => Color::new(82, 170, 240, 215),
                        _ => Color::new(68, 152, 222, 210),
                    };
                    d.draw_rectangle(bar_x, bar_top, fill_width, bar_height, fill_color);
                }
                let count_text = Self::format_count(count);
                let count_w = d.measure_text(&count_text, ROW_FONT);
                d.draw_text(
                    &count_text,
                    bar_x + BAR_WIDTH - count_w,
                    row_top,
                    ROW_FONT,
                    Color::new(236, 240, 250, 255),
                );
            }
            cursor_y += radius_rows * ROW_HEIGHT;
        }

        if radius_remainder > 0 {
            let summary = format!("… {} more radii", radius_remainder);
            d.draw_text(
                &summary,
                x + PADDING_X,
                cursor_y,
                ROW_FONT,
                Color::new(200, 205, 220, 255),
            );
        }
    }

    fn draw_height_histogram(&mut self, d: &mut RaylibDrawHandle) {
        if !self.gs.show_debug_overlay {
            return;
        }

        const PADDING_X: i32 = 14;
        const PADDING_Y: i32 = 12;
        const HEADER_HEIGHT: i32 = 28;
        const SUMMARY_LINE_HEIGHT: i32 = 18;
        const ROW_HEIGHT: i32 = 22;
        const LABEL_WIDTH: i32 = 150;
        const BAR_WIDTH: i32 = 220;
        const GAP_X: i32 = 12;
        const TITLE_FONT: i32 = 20;
        const SUMMARY_FONT: i32 = 16;
        const ROW_FONT: i32 = 18;
        const MAX_BINS: usize = 12;

        let window = self.height_tile_us.len();
        if window == 0 {
            return;
        }

        let reuse_count = self.height_tile_us.iter().filter(|&&v| v == 0).count();
        let durations_ms: Vec<f32> = self
            .height_tile_us
            .iter()
            .copied()
            .filter(|&v| v > 0)
            .map(|v| v as f32 / 1000.0)
            .collect();
        let build_count = durations_ms.len();

        let screen_w = d.get_screen_width();
        let screen_h = d.get_screen_height();
        let mut x = self.height_histogram_pos.x.round() as i32;
        let mut y = self.height_histogram_pos.y.round() as i32;

        let title = format!(
            "Height Tiles — builds: {} reuse: {} (window {})",
            build_count, reuse_count, window
        );

        let width = PADDING_X * 2 + LABEL_WIDTH + GAP_X + BAR_WIDTH;

        let format_ms = |ms: f32| -> String {
            if ms >= 10.0 {
                format!("{:.0}", ms)
            } else if ms >= 1.0 {
                format!("{:.1}", ms)
            } else {
                format!("{:.2}", ms)
            }
        };

        if build_count == 0 {
            let height = PADDING_Y * 2 + HEADER_HEIGHT + ROW_HEIGHT + 8;
            let max_x = (screen_w - width - 10).max(10);
            let max_y = (screen_h - height - 10).max(10);
            x = x.clamp(10, max_x);
            y = y.clamp(10, max_y);
            self.height_histogram_pos = Vector2::new(x as f32, y as f32);
            self.height_histogram_rect = Some((x, y, width, height));
            self.height_histogram_size = (width, height);

            let bg_color = Color::new(20, 22, 36, 220);
            let border_color = Color::new(200, 210, 240, 140);
            d.draw_rectangle(x, y, width, height, bg_color);
            d.draw_rectangle_lines(x, y, width, height, border_color);
            d.draw_text(
                &title,
                x + PADDING_X + 1,
                y + PADDING_Y + 1,
                TITLE_FONT,
                Color::BLACK,
            );
            d.draw_text(
                &title,
                x + PADDING_X,
                y + PADDING_Y,
                TITLE_FONT,
                Color::WHITE,
            );
            let msg = "No tile builds yet";
            d.draw_text(
                msg,
                x + PADDING_X,
                y + PADDING_Y + HEADER_HEIGHT,
                ROW_FONT,
                Color::new(210, 215, 230, 255),
            );
            return;
        }

        let mut sorted = durations_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let avg_ms = sorted.iter().copied().sum::<f32>() / build_count as f32;
        let p95_idx = ((build_count as f32) * 0.95).ceil().max(1.0) as usize - 1;
        let p95_ms = sorted[p95_idx.min(build_count - 1)];
        let max_ms = *sorted.last().unwrap();
        let last_ms = self
            .height_tile_us
            .iter()
            .rev()
            .find(|&&v| v > 0)
            .map(|&v| v as f32 / 1000.0)
            .unwrap_or(0.0);

        let bin_count = MAX_BINS.min(build_count.max(4));
        let mut bins = vec![0usize; bin_count];
        let max_edge = max_ms.max(0.1);
        let bin_width = (max_edge / bin_count as f32).max(0.1);

        for ms in durations_ms.iter().copied() {
            let mut idx = (ms / bin_width).floor() as usize;
            if idx >= bin_count {
                idx = bin_count - 1;
            }
            bins[idx] += 1;
        }

        let summary_height = SUMMARY_LINE_HEIGHT * 2;
        let bar_rows = bin_count as i32;
        let height = PADDING_Y * 2 + HEADER_HEIGHT + summary_height + bar_rows * ROW_HEIGHT + 8;
        let max_x = (screen_w - width - 10).max(10);
        let max_y = (screen_h - height - 10).max(10);
        x = x.clamp(10, max_x);
        y = y.clamp(10, max_y);
        self.height_histogram_pos = Vector2::new(x as f32, y as f32);
        self.height_histogram_rect = Some((x, y, width, height));
        self.height_histogram_size = (width, height);

        let bg_color = Color::new(20, 22, 36, 220);
        let border_color = Color::new(200, 210, 240, 140);
        d.draw_rectangle(x, y, width, height, bg_color);
        d.draw_rectangle_lines(x, y, width, height, border_color);

        d.draw_text(
            &title,
            x + PADDING_X + 1,
            y + PADDING_Y + 1,
            TITLE_FONT,
            Color::BLACK,
        );
        d.draw_text(
            &title,
            x + PADDING_X,
            y + PADDING_Y,
            TITLE_FONT,
            Color::WHITE,
        );

        let summary_y = y + PADDING_Y + HEADER_HEIGHT;
        let summary_text = format!(
            "Avg {}ms   P95 {}ms   Max {}ms   Last {}ms",
            format_ms(avg_ms),
            format_ms(p95_ms),
            format_ms(max_ms),
            format_ms(last_ms)
        );
        d.draw_text(
            &summary_text,
            x + PADDING_X,
            summary_y,
            SUMMARY_FONT,
            Color::new(215, 220, 240, 255),
        );

        let bin_text = format!("Bins: {}  Width ≈ {}ms", bin_count, format_ms(bin_width));
        d.draw_text(
            &bin_text,
            x + PADDING_X,
            summary_y + SUMMARY_LINE_HEIGHT,
            SUMMARY_FONT,
            Color::new(195, 200, 225, 255),
        );

        let bar_origin_y = summary_y + summary_height;
        let max_count = bins.iter().copied().max().unwrap_or(1) as f32;
        for (idx, count) in bins.iter().enumerate() {
            let row_top = bar_origin_y + (idx as i32) * ROW_HEIGHT;
            let start = idx as f32 * bin_width;
            let end = if idx == bin_count - 1 {
                max_edge
            } else {
                (idx as f32 + 1.0) * bin_width
            };
            let label = if idx == bin_count - 1 {
                format!("≥{}ms", format_ms(start))
            } else {
                format!("{}-{}ms", format_ms(start), format_ms(end))
            };
            d.draw_text(
                &label,
                x + PADDING_X,
                row_top,
                ROW_FONT,
                Color::new(220, 225, 240, 255),
            );
            d.draw_rectangle(
                x + PADDING_X + LABEL_WIDTH + GAP_X,
                row_top + 3,
                BAR_WIDTH,
                ROW_HEIGHT - 6,
                Color::new(40, 40, 68, 160),
            );
            if *count > 0 {
                let ratio = (*count as f32) / max_count;
                let fill = (ratio * BAR_WIDTH as f32).round() as i32;
                let fill_width = fill.max(2).min(BAR_WIDTH);
                let fill_color = match idx {
                    0 => Color::new(120, 200, 255, 230),
                    1 => Color::new(100, 180, 245, 220),
                    _ => Color::new(80, 160, 235, 210),
                };
                d.draw_rectangle(
                    x + PADDING_X + LABEL_WIDTH + GAP_X,
                    row_top + 3,
                    fill_width,
                    ROW_HEIGHT - 6,
                    fill_color,
                );
            }
            let count_text = format!("{}", count);
            let count_w = d.measure_text(&count_text, ROW_FONT);
            d.draw_text(
                &count_text,
                x + PADDING_X + LABEL_WIDTH + GAP_X + BAR_WIDTH - count_w,
                row_top,
                ROW_FONT,
                Color::new(240, 240, 255, 255),
            );
        }
    }

    fn format_count(mut value: usize) -> String {
        if value < 1_000 {
            return value.to_string();
        }
        let mut parts: Vec<u16> = Vec::new();
        while value >= 1_000 {
            parts.push((value % 1_000) as u16);
            value /= 1_000;
        }
        let mut out = value.to_string();
        while let Some(part) = parts.pop() {
            out.push('_');
            out.push_str(&format!("{:03}", part));
        }
        out
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

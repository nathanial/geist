use std::collections::HashSet;

use super::{
    App, AttachmentDebugView, ChunkVoxelView, ContentLayout, DebugOverlayTab, DebugStats,
    DiagnosticsTab, EventHistogramView, GeistDraw, HitRegion, IRect, IntentHistogramView,
    RenderStatsView, RuntimeStatsView, TabDefinition, TabStrip, TerrainHistogramView, WindowChrome,
    WindowFrame, WindowId,
};
use raylib::prelude::*;

use super::{MINIMAP_BORDER_PX, MINIMAP_MAX_CONTENT_SIDE, MINIMAP_MIN_CONTENT_SIDE};
use crate::raycast;
use geist_blocks::Block;
use geist_chunk::ChunkOccupancy;
use geist_render_raylib::conv::vec3_to_rl;
use geist_structures::StructureId;
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
        let sample = self.day_sample;
        let sky_scale = sample.sky_scale;
        let surface_sky = sample.surface_sky;
        let sun_id = self.sun.as_ref().map(|s| s.id);
        let sun_tint = {
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
            let brightness = ((0.2 + 0.8 * sky_scale) * visibility).clamp(0.0, 1.0);
            let r = (base[0] * brightness * 255.0).clamp(0.0, 255.0) as u8;
            let g = (base[1] * brightness * 255.0).clamp(0.0, 255.0) as u8;
            let b = (base[2] * brightness * 255.0).clamp(0.0, 255.0) as u8;
            Color::new(r, g, b, 255)
        };

        let camera3d = self.cam.to_camera3d();
        self.minimap_ui_rect = None;

        let screen_dims = (screen_width as i32, screen_height as i32);
        let overlay_theme = *self.overlay_windows.theme();
        let minimap_min_size = (
            overlay_theme.padding_x * 2 + MINIMAP_MIN_CONTENT_SIDE,
            overlay_theme.titlebar_height + overlay_theme.padding_y * 2 + MINIMAP_MIN_CONTENT_SIDE,
        );
        let mut minimap_render_side = 0;
        if self.gs.show_debug_overlay {
            if let Some(window) = self.overlay_windows.get_mut(WindowId::Minimap) {
                window.set_min_size(minimap_min_size);
                let frame = window.layout(screen_dims, &overlay_theme);
                let content = frame.content;
                let available_side = content.w.min(content.h).max(0);
                let outer_side =
                    available_side.min(MINIMAP_MAX_CONTENT_SIDE + MINIMAP_BORDER_PX * 2);
                minimap_render_side = (outer_side - MINIMAP_BORDER_PX * 2).max(0);
            }
        }
        if minimap_render_side > 0 {
            self.render_minimap_to_texture(rl, thread, minimap_render_side);
        } else {
            self.render_minimap_to_texture(rl, thread, 0);
        }

        let cursor_position = rl.get_mouse_position();
        let mouse_left_pressed = rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT);

        let font_for_frame = self.ui_font.clone();
        let mut d = GeistDraw::new(rl.begin_drawing(thread), font_for_frame);
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
                                d3.draw_model(
                                    &part.model,
                                    vec3_to_rl(st.pose.pos),
                                    1.0,
                                    if Some(sid) == sun_id {
                                        sun_tint
                                    } else {
                                        Color::WHITE
                                    },
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
            let ordered_ids = self.overlay_windows.ordered_ids();
            let mut minimap_drawn = false;

            for id in ordered_ids {
                let hover = self
                    .overlay_hover
                    .as_ref()
                    .and_then(|(hid, region)| (*hid == id).then_some(*region));

                match id {
                    WindowId::DiagnosticsTabs => {
                        let is_focused = self.overlay_windows.is_focused(id);
                        let frame_view = RenderStatsView::new(self, fps);
                        let runtime_view = RuntimeStatsView::new(self);
                        let attachment_view = AttachmentDebugView::new(self);

                        if let Some(window) = self.overlay_windows.get_mut(id) {
                            let frame_min = frame_view.min_size(&overlay_theme);
                            let runtime_min = runtime_view.min_size(&overlay_theme);
                            let attachment_min = attachment_view.min_size(&overlay_theme);
                            let min_width = frame_min.0.max(runtime_min.0).max(attachment_min.0);
                            let tab_extra =
                                overlay_theme.tab_height + overlay_theme.tab_content_spacing;
                            let min_height =
                                frame_min.1.max(runtime_min.1).max(attachment_min.1) + tab_extra;
                            window.set_min_size((min_width, min_height));
                            let frame = window.layout(screen_dims, &overlay_theme);

                            let tab_definitions = [
                                TabDefinition::new(DiagnosticsTab::FrameStats.title()),
                                TabDefinition::new(DiagnosticsTab::RuntimeStats.title()),
                                TabDefinition::new(DiagnosticsTab::AttachmentDebug.title()),
                            ];
                            let tab_layout =
                                TabStrip::layout(&d, &overlay_theme, &frame, &tab_definitions);
                            let hovered_tab = tab_layout.hovered(cursor_position);
                            if mouse_left_pressed
                                && hovered_tab.is_some()
                                && matches!(hover, Some(HitRegion::Content))
                                && !window.is_dragging()
                                && !window.is_resizing()
                            {
                                if let Some(index) = hovered_tab {
                                    let next_tab = DiagnosticsTab::from_index(index);
                                    if next_tab != self.overlay_diagnostics_tab {
                                        self.overlay_diagnostics_tab = next_tab;
                                    }
                                }
                            }

                            let selected_tab = self.overlay_diagnostics_tab;
                            let selected_index = selected_tab.as_index();

                            let frame_subtitle = frame_view.subtitle();
                            let runtime_subtitle = runtime_view.subtitle();
                            let attachment_subtitle: Option<&str> = None;
                            let subtitle = match selected_tab {
                                DiagnosticsTab::FrameStats => frame_subtitle,
                                DiagnosticsTab::RuntimeStats => runtime_subtitle,
                                DiagnosticsTab::AttachmentDebug => attachment_subtitle,
                            };

                            let window_state = window.state();
                            let is_pinned = window.is_pinned();

                            WindowChrome::draw(
                                &mut d,
                                &overlay_theme,
                                &frame,
                                "Diagnostics",
                                subtitle,
                                hover,
                                window_state,
                                is_focused,
                                is_pinned,
                            );

                            TabStrip::draw(
                                &mut d,
                                &overlay_theme,
                                &tab_layout,
                                selected_index,
                                hovered_tab,
                            );

                            let tab_content_area = tab_layout.content_rect();
                            window.update_content_viewport(tab_content_area);
                            let mut tab_content_frame = *window.frame();
                            tab_content_frame.content = tab_content_area;

                            let layout = match selected_tab {
                                DiagnosticsTab::FrameStats => {
                                    frame_view.draw(&mut d, &tab_content_frame)
                                }
                                DiagnosticsTab::RuntimeStats => {
                                    runtime_view.draw(&mut d, &tab_content_frame)
                                }
                                DiagnosticsTab::AttachmentDebug => {
                                    attachment_view.draw(&mut d, &tab_content_frame)
                                }
                            };

                            window.set_content_extent((
                                tab_content_frame.content.w,
                                layout.used_height,
                            ));

                            self.draw_overflow_hint(&mut d, &tab_content_frame, layout);
                        }
                    }

                    WindowId::DebugTabs => {
                        let is_focused = self.overlay_windows.is_focused(id);
                        if let Some(window) = self.overlay_windows.get_mut(id) {
                            let event_view = EventHistogramView::new(&self.debug_stats);
                            let intent_view = IntentHistogramView::new(&self.debug_stats);
                            let terrain_view = TerrainHistogramView::new(
                                &self.terrain_stage_us,
                                &self.terrain_stage_calls,
                                &self.terrain_height_tile_us,
                                &self.terrain_height_tile_reused,
                                &self.terrain_cache_hits,
                                &self.terrain_cache_misses,
                                &self.terrain_tile_cache_hits,
                                &self.terrain_tile_cache_misses,
                                &self.terrain_tile_cache_evictions,
                                &self.terrain_tile_cache_entries,
                                &self.terrain_chunk_total_us,
                                &self.terrain_chunk_fill_us,
                                &self.terrain_chunk_feature_us,
                            );

                            let event_min = event_view.min_size(&overlay_theme);
                            let intent_min = intent_view.min_size(&overlay_theme);
                            let terrain_min = terrain_view.min_size(&overlay_theme);
                            let min_width = event_min.0.max(intent_min.0).max(terrain_min.0);
                            let tab_extra =
                                overlay_theme.tab_height + overlay_theme.tab_content_spacing;
                            let min_height =
                                event_min.1.max(intent_min.1).max(terrain_min.1) + tab_extra;
                            window.set_min_size((min_width, min_height));
                            let frame = window.layout(screen_dims, &overlay_theme);

                            let tab_definitions = [
                                TabDefinition::new(DebugOverlayTab::EventQueue.title()),
                                TabDefinition::new(DebugOverlayTab::IntentQueue.title()),
                                TabDefinition::new(DebugOverlayTab::TerrainPipeline.title()),
                            ];
                            let tab_layout =
                                TabStrip::layout(&d, &overlay_theme, &frame, &tab_definitions);
                            let hovered_tab = tab_layout.hovered(cursor_position);
                            if mouse_left_pressed
                                && hovered_tab.is_some()
                                && matches!(hover, Some(HitRegion::Content))
                                && !window.is_dragging()
                                && !window.is_resizing()
                            {
                                if let Some(index) = hovered_tab {
                                    let next_tab = DebugOverlayTab::from_index(index);
                                    if next_tab != self.overlay_debug_tab {
                                        self.overlay_debug_tab = next_tab;
                                    }
                                }
                            }

                            let selected_tab = self.overlay_debug_tab;
                            let selected_index = selected_tab.as_index();

                            let event_subtitle = event_view.subtitle();
                            let intent_subtitle = intent_view.subtitle();
                            let terrain_subtitle = terrain_view.subtitle();
                            let subtitle = match selected_tab {
                                DebugOverlayTab::EventQueue => event_subtitle.as_deref(),
                                DebugOverlayTab::IntentQueue => intent_subtitle.as_deref(),
                                DebugOverlayTab::TerrainPipeline => terrain_subtitle.as_deref(),
                            };

                            let window_state = window.state();
                            let is_pinned = window.is_pinned();

                            WindowChrome::draw(
                                &mut d,
                                &overlay_theme,
                                &frame,
                                "Queues & Pipelines",
                                subtitle,
                                hover,
                                window_state,
                                is_focused,
                                is_pinned,
                            );

                            TabStrip::draw(
                                &mut d,
                                &overlay_theme,
                                &tab_layout,
                                selected_index,
                                hovered_tab,
                            );

                            let tab_content_area = tab_layout.content_rect();
                            window.update_content_viewport(tab_content_area);
                            let mut tab_content_frame = *window.frame();
                            tab_content_frame.content = tab_content_area;

                            let maybe_layout = match selected_tab {
                                DebugOverlayTab::EventQueue => {
                                    let layout =
                                        event_view.draw(&mut d, &tab_content_frame, &overlay_theme);
                                    Some(layout)
                                }
                                DebugOverlayTab::IntentQueue => {
                                    let layout = intent_view.draw(
                                        &mut d,
                                        &tab_content_frame,
                                        &overlay_theme,
                                    );
                                    Some(layout)
                                }
                                DebugOverlayTab::TerrainPipeline => {
                                    terrain_view.draw(&mut d, &tab_content_frame, &overlay_theme)
                                }
                            };

                            if let Some(layout) = maybe_layout {
                                window.set_content_extent((
                                    tab_content_frame.content.w,
                                    layout.used_height,
                                ));
                                self.draw_overflow_hint(&mut d, &tab_content_frame, layout);
                            } else {
                                window.set_content_extent((
                                    tab_content_frame.content.w,
                                    tab_content_frame.content.h,
                                ));
                            }
                        }
                    }
                    WindowId::ChunkVoxels => {
                        let is_focused = self.overlay_windows.is_focused(id);
                        let view = ChunkVoxelView::new(self);
                        if let Some(window) = self.overlay_windows.get_mut(id) {
                            window.set_min_size(view.min_size(&overlay_theme));
                            let frame = window.layout(screen_dims, &overlay_theme);
                            let window_state = window.state();
                            let is_pinned = window.is_pinned();

                            WindowChrome::draw(
                                &mut d,
                                &overlay_theme,
                                &frame,
                                "Chunk Voxels",
                                view.subtitle(),
                                hover,
                                window_state,
                                is_focused,
                                is_pinned,
                            );

                            let content = frame.content;
                            window.update_content_viewport(content);
                            let mut content_frame = *window.frame();
                            content_frame.content = content;
                            let layout = view.draw(&mut d, &content_frame);
                            window
                                .set_content_extent((content_frame.content.w, layout.used_height));
                            self.draw_overflow_hint(&mut d, &content_frame, layout);
                        }
                    }
                    WindowId::Minimap => {
                        minimap_drawn = true;
                        let is_focused = self.overlay_windows.is_focused(id);
                        if let Some(window) = self.overlay_windows.get_mut(id) {
                            let minimap_min_size = (
                                overlay_theme.padding_x * 2 + MINIMAP_MIN_CONTENT_SIDE,
                                overlay_theme.titlebar_height
                                    + overlay_theme.padding_y * 2
                                    + MINIMAP_MIN_CONTENT_SIDE,
                            );
                            window.set_min_size(minimap_min_size);
                            let frame = window.layout(screen_dims, &overlay_theme);
                            let subtitle = Some(format!(
                                "radius {} chunks",
                                self.gs.view_radius_chunks.max(0)
                            ));

                            let window_state = window.state();
                            let is_pinned = window.is_pinned();
                            WindowChrome::draw(
                                &mut d,
                                &overlay_theme,
                                &frame,
                                "Minimap",
                                subtitle.as_deref(),
                                hover,
                                window_state,
                                is_focused,
                                is_pinned,
                            );

                            window.set_content_extent((frame.content.w, frame.content.h));

                            let content = frame.content;
                            let available_side = content.w.min(content.h).max(0);
                            let outer_side = available_side
                                .min(MINIMAP_MAX_CONTENT_SIDE + MINIMAP_BORDER_PX * 2);
                            let map_side = (outer_side - MINIMAP_BORDER_PX * 2).max(0);

                            if map_side > 0 {
                                let frame_rect = IRect::new(
                                    content.x + (content.w - outer_side) / 2,
                                    content.y + (content.h - outer_side) / 2,
                                    outer_side,
                                    outer_side,
                                );
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
                                    let src = Rectangle::new(
                                        0.0,
                                        0.0,
                                        tex.width() as f32,
                                        -(tex.height() as f32),
                                    );
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
                                    self.minimap_ui_rect = Some((
                                        frame_rect.x,
                                        frame_rect.y,
                                        frame_rect.w,
                                        frame_rect.h,
                                    ));

                                    let label =
                                        format!("Loaded {} chunks", self.gs.chunks.ready_len());
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

                                    let legend =
                                        ["Scroll: zoom", "LMB drag: orbit", "Shift+Drag/RMB: pan"];
                                    let legend_fs = 14;
                                    let legend_total_h = (legend.len() as i32) * (legend_fs + 2);
                                    let mut legend_y =
                                        map_rect.y + map_rect.h - legend_total_h - 12;
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
                                    d.draw_text(
                                        msg,
                                        msg_x + 1,
                                        msg_y + 1,
                                        msg_fs,
                                        Color::new(0, 0, 0, 220),
                                    );
                                    d.draw_text(
                                        msg,
                                        msg_x,
                                        msg_y,
                                        msg_fs,
                                        Color::new(220, 220, 240, 240),
                                    );
                                }
                            } else {
                                self.minimap_ui_rect = None;
                                let msg = "Expand the window to view the minimap";
                                let msg_fs = 16;
                                let msg_w = d.measure_text(msg, msg_fs);
                                let msg_x = content.x + (content.w - msg_w) / 2;
                                let msg_y = content.y + (content.h - msg_fs) / 2;
                                d.draw_text(
                                    msg,
                                    msg_x + 1,
                                    msg_y + 1,
                                    msg_fs,
                                    Color::new(0, 0, 0, 180),
                                );
                                d.draw_text(
                                    msg,
                                    msg_x,
                                    msg_y,
                                    msg_fs,
                                    Color::new(218, 228, 248, 230),
                                );
                            }
                        }
                    }
                }
            }

            if !minimap_drawn {
                self.minimap_ui_rect = None;
            }
        } else {
            self.minimap_ui_rect = None;
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

    fn draw_overflow_hint(&self, d: &mut GeistDraw, frame: &WindowFrame, layout: ContentLayout) {
        if !layout.overflow() {
            return;
        }
        if frame.scroll.content_size.1 > frame.scroll.viewport_size.1 {
            // Scrollbar handles overflow; skip hint.
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

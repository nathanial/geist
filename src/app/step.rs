use geist_blocks::Block;
use geist_geom::Vec3;
use geist_render_raylib::conv::vec3_to_rl;
use geist_runtime::JobOut;
use geist_world::ChunkCoord;
use raylib::prelude::*;
use std::collections::BTreeMap;

use super::App;
use crate::event::{Event, RebuildCause};

impl App {
    pub fn step(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread, dt: f32) {
        // Shader hot-reload
        if self.shader_event_rx.try_iter().next().is_some() {
            // Attempt to reload both shaders; fall back to previous if load fails
            if let Some(ls) =
                geist_render_raylib::LeavesShader::load_with_base(rl, thread, &self.assets_root)
            {
                self.leaves_shader = Some(ls);
            }
            if let Some(fs) =
                geist_render_raylib::FogShader::load_with_base(rl, thread, &self.assets_root)
            {
                self.fog_shader = Some(fs);
            }
            if let Some(ws) =
                geist_render_raylib::WaterShader::load_with_base(rl, thread, &self.assets_root)
            {
                self.water_shader = Some(ws);
            }
            // Rebind shaders on all existing models
            let rebind = |parts: &mut Vec<geist_render_raylib::ChunkPart>| {
                for part in parts.iter_mut() {
                    if let Some(mat) = part.model.materials_mut().get_mut(0) {
                        let tag = self
                            .reg
                            .materials
                            .get(part.mid)
                            .and_then(|m| m.render_tag.as_deref());
                        if tag == Some("leaves") {
                            if let Some(ref ls) = self.leaves_shader {
                                let dest = mat.shader_mut();
                                let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                let src_ptr: *const raylib::ffi::Shader = ls.shader.as_ref();
                                unsafe { std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1) };
                            }
                        } else if tag == Some("water") {
                            if let Some(ref ws) = self.water_shader {
                                let dest = mat.shader_mut();
                                let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                                let src_ptr: *const raylib::ffi::Shader = ws.shader.as_ref();
                                unsafe { std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1) };
                            }
                        } else if let Some(ref fs) = self.fog_shader {
                            let dest = mat.shader_mut();
                            let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                            let src_ptr: *const raylib::ffi::Shader = fs.shader.as_ref();
                            unsafe { std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1) };
                        }
                    }
                }
            };
            for (_k, cr) in self.renders.iter_mut() {
                rebind(&mut cr.parts);
            }
            for (_id, cr) in self.structure_renders.iter_mut() {
                rebind(&mut cr.parts);
            }
            log::info!("Reloaded shaders and rebound on existing models");
        }
        // Registry hot-reload (materials/blocks)
        if self.reg_event_rx.try_iter().next().is_some() {
            let mats = crate::assets::materials_path(&self.assets_root);
            let blks = crate::assets::blocks_path(&self.assets_root);
            match geist_blocks::BlockRegistry::load_from_paths(&mats, &blks) {
                Ok(mut newreg) => {
                    for m in &mut newreg.materials.materials {
                        for p in &mut m.texture_candidates {
                            if p.is_relative() {
                                *p = self.assets_root.join(&p);
                            }
                        }
                    }
                    self.reg = std::sync::Arc::new(newreg);
                    self.tex_cache.map.clear();
                    let keys: Vec<ChunkCoord> = self.renders.keys().copied().collect();
                    for coord in keys {
                        self.queue.emit_now(Event::ChunkRebuildRequested {
                            cx: coord.cx,
                            cy: coord.cy,
                            cz: coord.cz,
                            cause: RebuildCause::HotReload,
                        });
                    }
                    for (id, st) in self.gs.structures.iter() {
                        let next_rev = st.built_rev.wrapping_add(1);
                        self.queue.emit_now(Event::StructureBuildRequested {
                            id: *id,
                            rev: next_rev,
                        });
                    }
                    log::info!("Reloaded voxel registry and scheduled rebuilds");
                }
                Err(e) => log::warn!("Registry reload failed: {}", e),
            }
        }
        // Handle worldgen hot-reload
        // Always invalidate previous CPU buffers on change; optionally schedule rebuilds
        if self.take_worldgen_dirty() {
            let keys: Vec<ChunkCoord> = self.gs.chunks.ready_coords().collect();
            let total_chunks = self.gs.chunks.ready_len();
            for (_coord, ent) in self.gs.chunks.iter_mut() {
                ent.buf = None; // prevent reuse across worldgen param changes
            }
            if self.rebuild_on_worldgen {
                for coord in &keys {
                    self.queue.emit_now(Event::ChunkRebuildRequested {
                        cx: coord.cx,
                        cy: coord.cy,
                        cz: coord.cz,
                        cause: RebuildCause::HotReload,
                    });
                }
                log::info!(
                    "Scheduled rebuild of {} loaded chunks due to worldgen change",
                    keys.len()
                );
            } else {
                log::info!(
                    "Worldgen changed; invalidated {} chunk buffers (rebuild on demand)",
                    total_chunks
                );
            }
        }
        // Input handling → emit events
        if rl.is_key_pressed(KeyboardKey::KEY_V) {
            self.queue.emit_now(Event::WalkModeToggled);
        }
        if self.gs.walk_mode {
            self.cam.update_look_only(rl, dt);
        } else {
            self.cam.update(rl, dt);
        }

        if rl.is_key_pressed(KeyboardKey::KEY_G) {
            self.queue.emit_now(Event::GridToggled);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_F) {
            self.queue.emit_now(Event::WireframeToggled);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_B) {
            self.queue.emit_now(Event::ChunkBoundsToggled);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_C) {
            self.queue.emit_now(Event::FrustumCullingToggled);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_H) {
            self.queue.emit_now(Event::BiomeLabelToggled);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_F3) {
            self.queue.emit_now(Event::DebugOverlayToggled);
        }
        // Hotbar selection: if config present, use it; else fallback to legacy mapping
        if !self.hotbar.is_empty() {
            let keys = [
                KeyboardKey::KEY_ONE,
                KeyboardKey::KEY_TWO,
                KeyboardKey::KEY_THREE,
                KeyboardKey::KEY_FOUR,
                KeyboardKey::KEY_FIVE,
                KeyboardKey::KEY_SIX,
                KeyboardKey::KEY_SEVEN,
                KeyboardKey::KEY_EIGHT,
                KeyboardKey::KEY_NINE,
            ];
            for (i, key) in keys.iter().enumerate() {
                if i < self.hotbar.len() && rl.is_key_pressed(*key) {
                    self.queue.emit_now(Event::PlaceTypeSelected {
                        block: self.hotbar[i],
                    });
                }
            }
        } else {
            let id_of = |name: &str| self.reg.id_by_name(name).unwrap_or(0);
            if rl.is_key_pressed(KeyboardKey::KEY_ONE) {
                self.queue.emit_now(Event::PlaceTypeSelected {
                    block: Block {
                        id: id_of("dirt"),
                        state: 0,
                    },
                });
            }
            if rl.is_key_pressed(KeyboardKey::KEY_TWO) {
                self.queue.emit_now(Event::PlaceTypeSelected {
                    block: Block {
                        id: id_of("stone"),
                        state: 0,
                    },
                });
            }
            if rl.is_key_pressed(KeyboardKey::KEY_THREE) {
                self.queue.emit_now(Event::PlaceTypeSelected {
                    block: Block {
                        id: id_of("sand"),
                        state: 0,
                    },
                });
            }
            if rl.is_key_pressed(KeyboardKey::KEY_FOUR) {
                self.queue.emit_now(Event::PlaceTypeSelected {
                    block: Block {
                        id: id_of("grass"),
                        state: 0,
                    },
                });
            }
            if rl.is_key_pressed(KeyboardKey::KEY_FIVE) {
                self.queue.emit_now(Event::PlaceTypeSelected {
                    block: Block {
                        id: id_of("snow"),
                        state: 0,
                    },
                });
            }
            if rl.is_key_pressed(KeyboardKey::KEY_SIX) {
                self.queue.emit_now(Event::PlaceTypeSelected {
                    block: Block {
                        id: id_of("glowstone"),
                        state: 0,
                    },
                });
            }
            if rl.is_key_pressed(KeyboardKey::KEY_SEVEN) {
                self.queue.emit_now(Event::PlaceTypeSelected {
                    block: Block {
                        id: id_of("beacon"),
                        state: 0,
                    },
                });
            }
        }

        // Minimap interactions (zoom/orbit/pan)
        let mut minimap_hovered = false;
        if !self.gs.show_debug_overlay {
            self.minimap_drag_button = None;
            self.minimap_last_cursor = None;
        }
        if self.gs.show_debug_overlay {
            if let Some((mx, my, mw, mh)) = self.minimap_ui_rect {
                let mouse = rl.get_mouse_position();
                if mouse.x >= mx as f32
                    && mouse.x <= (mx + mw) as f32
                    && mouse.y >= my as f32
                    && mouse.y <= (my + mh) as f32
                {
                    minimap_hovered = true;
                    let wheel = rl.get_mouse_wheel_move();
                    if wheel.abs() > f32::EPSILON {
                        let factor = 1.0 + wheel * 0.18;
                        self.minimap_zoom = (self.minimap_zoom * factor).clamp(0.35, 6.0);
                    }
                    if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                        self.minimap_drag_button = Some(MouseButton::MOUSE_BUTTON_LEFT);
                        self.minimap_drag_pan = rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
                            || rl.is_key_down(KeyboardKey::KEY_RIGHT_SHIFT);
                        self.minimap_last_cursor = Some(mouse);
                    }
                    if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT) {
                        self.minimap_drag_button = Some(MouseButton::MOUSE_BUTTON_RIGHT);
                        self.minimap_drag_pan = true;
                        self.minimap_last_cursor = Some(mouse);
                    }
                }
            }
        }

        if let Some(button) = self.minimap_drag_button {
            if !rl.is_mouse_button_down(button) {
                self.minimap_drag_button = None;
                self.minimap_last_cursor = None;
            } else if let Some(prev) = self.minimap_last_cursor {
                let mouse = rl.get_mouse_position();
                let dx = mouse.x - prev.x;
                let dy = mouse.y - prev.y;
                if dx.abs() > f32::EPSILON || dy.abs() > f32::EPSILON {
                    if self.minimap_drag_pan {
                        let pan_scale = 0.01 * self.minimap_zoom.max(0.4);
                        self.minimap_pan.x -= dx * pan_scale;
                        self.minimap_pan.z += dy * pan_scale;
                    } else {
                        let yaw_speed = 0.010;
                        let pitch_speed = 0.010;
                        self.minimap_yaw += dx * yaw_speed;
                        self.minimap_pitch =
                            (self.minimap_pitch - dy * pitch_speed).clamp(0.12, 1.45);
                        let tau = std::f32::consts::TAU;
                        if self.minimap_yaw > std::f32::consts::PI {
                            self.minimap_yaw -= tau;
                        } else if self.minimap_yaw < -std::f32::consts::PI {
                            self.minimap_yaw += tau;
                        }
                    }
                    self.minimap_last_cursor = Some(mouse);
                }
            }
        } else if !minimap_hovered {
            self.minimap_last_cursor = None;
        }

        let mut event_hist_hovered = false;
        if !self.gs.show_debug_overlay {
            self.event_histogram_dragging = false;
        } else if let Some((hx, hy, hw, hh)) = self.event_histogram_rect {
            let mouse = rl.get_mouse_position();
            if mouse.x >= hx as f32
                && mouse.x <= (hx + hw) as f32
                && mouse.y >= hy as f32
                && mouse.y <= (hy + hh) as f32
            {
                event_hist_hovered = true;
                if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                    self.event_histogram_dragging = true;
                    self.event_histogram_drag_offset =
                        Vector2::new(mouse.x - hx as f32, mouse.y - hy as f32);
                }
            }
        }

        if self.event_histogram_dragging {
            if !rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) {
                self.event_histogram_dragging = false;
            } else {
                let mouse = rl.get_mouse_position();
                let (win_w, win_h) = self.event_histogram_size;
                let pad = 10.0_f32;
                let screen_w = rl.get_screen_width() as f32;
                let screen_h = rl.get_screen_height() as f32;
                let mut new_x = mouse.x - self.event_histogram_drag_offset.x;
                let mut new_y = mouse.y - self.event_histogram_drag_offset.y;
                let max_x = (screen_w - win_w as f32 - pad).max(pad);
                let max_y = (screen_h - win_h as f32 - pad).max(pad);
                new_x = new_x.clamp(pad, max_x);
                new_y = new_y.clamp(pad, max_y);
                self.event_histogram_pos = Vector2::new(new_x, new_y);
            }
        }

        let mut height_hist_hovered = false;
        if !self.gs.show_debug_overlay {
            self.height_histogram_dragging = false;
        } else if let Some((hx, hy, hw, hh)) = self.height_histogram_rect {
            let mouse = rl.get_mouse_position();
            if mouse.x >= hx as f32
                && mouse.x <= (hx + hw) as f32
                && mouse.y >= hy as f32
                && mouse.y <= (hy + hh) as f32
            {
                height_hist_hovered = true;
                if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                    self.height_histogram_dragging = true;
                    self.height_histogram_drag_offset =
                        Vector2::new(mouse.x - hx as f32, mouse.y - hy as f32);
                }
            }
        }

        if self.height_histogram_dragging {
            if !rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) {
                self.height_histogram_dragging = false;
            } else {
                let mouse = rl.get_mouse_position();
                let (win_w, win_h) = self.height_histogram_size;
                let pad = 10.0_f32;
                let screen_w = rl.get_screen_width() as f32;
                let screen_h = rl.get_screen_height() as f32;
                let mut new_x = mouse.x - self.height_histogram_drag_offset.x;
                let mut new_y = mouse.y - self.height_histogram_drag_offset.y;
                let max_x = (screen_w - win_w as f32 - pad).max(pad);
                let max_y = (screen_h - win_h as f32 - pad).max(pad);
                new_x = new_x.clamp(pad, max_x);
                new_y = new_y.clamp(pad, max_y);
                self.height_histogram_pos = Vector2::new(new_x, new_y);
            }
        } else if !height_hist_hovered {
            self.height_histogram_drag_offset = Vector2::new(0.0, 0.0);
        }

        let mut intent_hist_hovered = false;
        if !self.gs.show_debug_overlay {
            self.intent_histogram_dragging = false;
        } else if let Some((hx, hy, hw, hh)) = self.intent_histogram_rect {
            let mouse = rl.get_mouse_position();
            if mouse.x >= hx as f32
                && mouse.x <= (hx + hw) as f32
                && mouse.y >= hy as f32
                && mouse.y <= (hy + hh) as f32
            {
                intent_hist_hovered = true;
                if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                    self.intent_histogram_dragging = true;
                    self.intent_histogram_drag_offset =
                        Vector2::new(mouse.x - hx as f32, mouse.y - hy as f32);
                }
            }
        }

        if self.intent_histogram_dragging {
            if !rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) {
                self.intent_histogram_dragging = false;
            } else {
                let mouse = rl.get_mouse_position();
                let (win_w, win_h) = self.intent_histogram_size;
                let pad = 10.0_f32;
                let screen_w = rl.get_screen_width() as f32;
                let screen_h = rl.get_screen_height() as f32;
                let mut new_x = mouse.x - self.intent_histogram_drag_offset.x;
                let mut new_y = mouse.y - self.intent_histogram_drag_offset.y;
                let max_x = (screen_w - win_w as f32 - pad).max(pad);
                let max_y = (screen_h - win_h as f32 - pad).max(pad);
                new_x = new_x.clamp(pad, max_x);
                new_y = new_y.clamp(pad, max_y);
                self.intent_histogram_pos = Vector2::new(new_x, new_y);
            }
        }

        let block_minimap_input = minimap_hovered || self.minimap_drag_button.is_some();
        let block_hist_input = event_hist_hovered || self.event_histogram_dragging;
        let block_intent_input = intent_hist_hovered || self.intent_histogram_dragging;
        let block_height_input = height_hist_hovered || self.height_histogram_dragging;
        let block_ui_input =
            block_minimap_input || block_hist_input || block_intent_input || block_height_input;

        // Structure speed controls (horizontal X)
        if rl.is_key_pressed(KeyboardKey::KEY_MINUS) {
            self.gs.structure_speed = (self.gs.structure_speed - 1.0).max(0.0);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_EQUAL) {
            self.gs.structure_speed = (self.gs.structure_speed + 1.0).min(64.0);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_ZERO) {
            self.gs.structure_speed = 0.0;
        }

        // Structure elevation controls (vertical Y)
        if rl.is_key_pressed(KeyboardKey::KEY_LEFT_BRACKET) {
            self.gs.structure_elev_speed = (self.gs.structure_elev_speed - 1.0).max(-64.0);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_RIGHT_BRACKET) {
            self.gs.structure_elev_speed = (self.gs.structure_elev_speed + 1.0).min(64.0);
        }
        if rl.is_key_pressed(KeyboardKey::KEY_BACKSLASH) {
            self.gs.structure_elev_speed = 0.0;
        }

        // Light emitters via hotkeys
        if rl.is_key_pressed(KeyboardKey::KEY_L) {
            let fwd = self.cam.forward();
            let p = self.cam.position + fwd * 4.0;
            let wx = p.x.floor() as i32;
            let wy = p.y.floor() as i32;
            let wz = p.z.floor() as i32;
            self.queue.emit_now(Event::LightEmitterAdded {
                wx,
                wy,
                wz,
                level: 255,
                is_beacon: false,
            });
        }
        if rl.is_key_pressed(KeyboardKey::KEY_K) {
            let fwd = self.cam.forward();
            let p = self.cam.position + fwd * 4.0;
            let wx = p.x.floor() as i32;
            let wy = p.y.floor() as i32;
            let wz = p.z.floor() as i32;
            self.queue
                .emit_now(Event::LightEmitterRemoved { wx, wy, wz });
        }

        // Lighting mode cycling removed; FullMicro is the only supported mode.

        // Mouse edit intents
        let want_edit = !block_ui_input
            && (rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT)
                || rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT));
        if want_edit {
            let place = rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT);
            let block = self.gs.place_type;
            self.queue
                .emit_now(Event::RaycastEditRequested { place, block });
        }

        // Update structure poses: translate along +X and vertical Y with adjustable speeds
        let step_dx = self.gs.structure_speed * dt.max(0.0);
        let step_dy = self.gs.structure_elev_speed * dt.max(0.0);
        for (id, st) in self.gs.structures.iter() {
            let prev = st.pose.pos;
            let newp = Vec3 {
                x: prev.x + step_dx,
                y: prev.y + step_dy,
                z: prev.z,
            };
            let delta = Vector3::new(newp.x - prev.x, newp.y - prev.y, newp.z - prev.z);
            // Keep yaw fixed so collisions match visuals
            let yaw = 0.0_f32;
            self.queue.emit_now(Event::StructurePoseUpdated {
                id: *id,
                pos: vec3_to_rl(newp),
                yaw_deg: yaw,
                delta,
            });
        }

        // Movement intent for this tick (dt→ms)
        let dt_ms = (dt.max(0.0) * 1000.0) as u32;
        self.queue.emit_now(Event::MovementRequested {
            dt_ms,
            yaw: self.cam.yaw,
            walk_mode: self.gs.walk_mode,
        });

        // Drain worker results, sort deterministically by job_id, and emit completion events for this tick
        let mut results: Vec<JobOut> = self.runtime.drain_worker_results();
        results.sort_by_key(|r| r.job_id);
        for r in results {
            // Record perf samples into rolling windows
            match r.kind {
                geist_runtime::JobKind::Light => {
                    Self::perf_push(&mut self.perf_light_ms, r.t_light_ms);
                    Self::perf_push(&mut self.perf_total_ms, r.t_total_ms);
                }
                geist_runtime::JobKind::Edit | geist_runtime::JobKind::Bg => {
                    Self::perf_push(&mut self.perf_mesh_ms, r.t_mesh_ms);
                    Self::perf_push(&mut self.perf_light_ms, r.t_light_ms);
                    Self::perf_push(&mut self.perf_total_ms, r.t_total_ms);
                }
            }
            if r.t_gen_ms > 0 {
                Self::perf_push(&mut self.perf_gen_ms, r.t_gen_ms);
            }
            if r.height_tile_stats.duration_us > 0 || r.height_tile_stats.reused {
                Self::perf_push(&mut self.height_tile_us, r.height_tile_stats.duration_us);
            }
            // Perf logging per job
            match r.kind {
                geist_runtime::JobKind::Light => {
                    log::info!(
                        target: "perf",
                        "light_ms={} total_ms={} gen_ms={} apply_ms={} cx={} cz={} rev={} job_id={}",
                        r.t_light_ms,
                        r.t_total_ms,
                        r.t_gen_ms,
                        r.t_apply_ms,
                        r.cx,
                        r.cz,
                        r.rev,
                        r.job_id
                    );
                }
                geist_runtime::JobKind::Edit | geist_runtime::JobKind::Bg => {
                    log::info!(
                        target: "perf",
                        "mesh_ms={} light_ms={} total_ms={} gen_ms={} apply_ms={} kind={:?} cx={} cy={} cz={} rev={} job_id={}",
                        r.t_mesh_ms,
                        r.t_light_ms,
                        r.t_total_ms,
                        r.t_gen_ms,
                        r.t_apply_ms,
                        r.kind,
                        r.cx,
                        r.cy,
                        r.cz,
                        r.rev,
                        r.job_id
                    );
                }
            }
            if r.occupancy.is_empty() {
                self.queue.emit_now(Event::BuildChunkJobCompleted {
                    cx: r.cx,
                    cy: r.cy,
                    cz: r.cz,
                    rev: r.rev,
                    occupancy: r.occupancy,
                    cpu: None,
                    buf: None,
                    light_borders: None,
                    light_grid: None,
                    job_id: r.job_id,
                });
            } else if let Some(cpu) = r.cpu {
                if let Some(buf) = r.buf {
                    // For mesh builds, pass through the grid; pack atlas later during event handling
                    self.queue.emit_now(Event::BuildChunkJobCompleted {
                        cx: r.cx,
                        cy: r.cy,
                        cz: r.cz,
                        rev: r.rev,
                        occupancy: r.occupancy,
                        cpu: Some(cpu),
                        buf: Some(buf),
                        light_borders: r.light_borders,
                        light_grid: r.light_grid,
                        job_id: r.job_id,
                    });
                } else {
                    log::warn!(
                        "build job {:?} missing buffer despite non-empty occupancy",
                        ChunkCoord::new(r.cx, r.cy, r.cz)
                    );
                }
            } else if let Some(lg) = r.light_grid {
                // If macro light borders were computed on the light-only lane, update them here
                // and notify neighbors on changes so they can refresh their seam rings.
                if let Some(lb) = r.light_borders {
                    let coord = ChunkCoord::new(r.cx, r.cy, r.cz);
                    let (changed, mask) = self.gs.lighting.update_borders_mask(coord, lb);
                    if changed {
                        self.queue.emit_now(Event::LightBordersUpdated {
                            cx: r.cx,
                            cy: r.cy,
                            cz: r.cz,
                            xn_changed: mask.xn,
                            xp_changed: mask.xp,
                            yn_changed: mask.yn,
                            yp_changed: mask.yp,
                            zn_changed: mask.zn,
                            zp_changed: mask.zp,
                        });
                    }
                }
                self.queue.emit_now(Event::ChunkLightingRecomputed {
                    cx: r.cx,
                    cy: r.cy,
                    cz: r.cz,
                    rev: r.rev,
                    light_grid: lg,
                    job_id: r.job_id,
                });
            }
        }

        // Drain structure worker results
        for r in self.runtime.drain_structure_results() {
            self.queue.emit_now(Event::StructureBuildCompleted {
                id: r.id,
                rev: r.rev,
                cpu: r.cpu,
            });
        }

        // Snapshot queued events before processing (for debug overlay)
        {
            let (total, by) = self.queue.queued_counts();
            self.debug_stats.queued_events_total = total;
            // Sort for stable presentation
            let mut pairs: Vec<(String, usize)> =
                by.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
            pairs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            self.debug_stats.queued_events_by = pairs;
        }

        // Process events scheduled for this tick with a budget
        let mut processed = 0usize;
        let max_events = 20_000usize;
        let label_of = |ev: &Event| -> &'static str {
            match ev {
                Event::Tick => "Tick",
                Event::WalkModeToggled => "WalkModeToggled",
                Event::GridToggled => "GridToggled",
                Event::WireframeToggled => "WireframeToggled",
                Event::ChunkBoundsToggled => "ChunkBoundsToggled",
                Event::FrustumCullingToggled => "FrustumCullingToggled",
                Event::BiomeLabelToggled => "BiomeLabelToggled",
                Event::DebugOverlayToggled => "DebugOverlayToggled",
                Event::PlaceTypeSelected { .. } => "PlaceTypeSelected",
                Event::MovementRequested { .. } => "MovementRequested",
                Event::RaycastEditRequested { .. } => "RaycastEditRequested",
                Event::BlockPlaced { .. } => "BlockPlaced",
                Event::BlockRemoved { .. } => "BlockRemoved",
                Event::ViewCenterChanged { .. } => "ViewCenterChanged",
                Event::EnsureChunkLoaded { .. } => "EnsureChunkLoaded",
                Event::EnsureChunkUnloaded { .. } => "EnsureChunkUnloaded",
                Event::ChunkRebuildRequested { .. } => "ChunkRebuildRequested",
                Event::BuildChunkJobRequested { .. } => "BuildChunkJobRequested",
                Event::BuildChunkJobCompleted { .. } => "BuildChunkJobCompleted",
                Event::ChunkLightingRecomputed { .. } => "ChunkLightingRecomputed",
                Event::StructureBuildRequested { .. } => "StructureBuildRequested",
                Event::StructureBuildCompleted { .. } => "StructureBuildCompleted",
                Event::StructurePoseUpdated { .. } => "StructurePoseUpdated",
                Event::StructureBlockPlaced { .. } => "StructureBlockPlaced",
                Event::StructureBlockRemoved { .. } => "StructureBlockRemoved",
                Event::PlayerAttachedToStructure { .. } => "PlayerAttachedToStructure",
                Event::PlayerDetachedFromStructure { .. } => "PlayerDetachedFromStructure",
                Event::LightEmitterAdded { .. } => "LightEmitterAdded",
                Event::LightEmitterRemoved { .. } => "LightEmitterRemoved",
                Event::LightBordersUpdated { .. } => "LightBordersUpdated",
            }
        };
        while let Some(env) = self.queue.pop_ready() {
            // Tally processed stats (session-wide)
            let label = label_of(&env.kind).to_string();
            self.evt_processed_total = self.evt_processed_total.saturating_add(1);
            *self.evt_processed_by.entry(label).or_insert(0) += 1;
            self.handle_event(rl, thread, env);
            processed += 1;
            if processed >= max_events {
                break;
            }
        }
        // After handling events for this tick, flush prioritized intents.
        self.flush_intents();
        // Snapshot current intents backlog for debug overlay
        self.debug_stats.intents_size = self.intents.len();
        if self.intents.is_empty() {
            self.debug_stats.intents_by_cause.clear();
            self.debug_stats.intents_by_radius.clear();
        } else {
            let mut cause_counts = [0usize; 4];
            for entry in self.intents.values() {
                let idx = entry.cause as usize;
                if idx < cause_counts.len() {
                    cause_counts[idx] = cause_counts[idx].saturating_add(1);
                }
            }
            let mut by_cause: Vec<(String, usize)> = Vec::new();
            for (idx, count) in cause_counts.into_iter().enumerate() {
                if count == 0 {
                    continue;
                }
                let label = match idx {
                    0 => "Edit",
                    1 => "Light",
                    2 => "StreamLoad",
                    3 => "HotReload",
                    _ => "Other",
                };
                by_cause.push((label.to_string(), count));
            }
            by_cause.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            self.debug_stats.intents_by_cause = by_cause;

            let center = self.gs.center_chunk;
            let mut radius_counts: BTreeMap<i32, usize> = BTreeMap::new();
            for key in self.intents.keys() {
                let dist_sq = center.distance_sq(*key);
                let radius = (dist_sq as f64).sqrt().floor() as i32;
                let entry = radius_counts.entry(radius).or_insert(0);
                *entry = entry.saturating_add(1);
            }
            let mut radius_rows: Vec<(String, usize)> = Vec::with_capacity(radius_counts.len());
            for (radius, count) in radius_counts {
                radius_rows.push((format!("r={}", radius), count));
            }
            self.debug_stats.intents_by_radius = radius_rows;
        }
        self.gs.tick = self.gs.tick.wrapping_add(1);
        self.queue.advance_tick();
        // Sanity check: events left in past ticks will never be processed; warn if detected
        let stale = self.queue.count_stale_events();
        if stale > 0 {
            let mut details = String::new();
            for (t, n) in self.queue.stale_summary() {
                use std::fmt::Write as _;
                let _ = write!(&mut details, "[t={} n={}] ", t, n);
            }
            log::error!(
                target: "events",
                "Detected {} stale event(s) in past tick buckets; details: {}",
                stale,
                details
            );
        }
    }
}

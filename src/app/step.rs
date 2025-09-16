use geist_blocks::Block;
use geist_geom::Vec3;
use geist_render_raylib::conv::vec3_to_rl;
use geist_runtime::JobOut;
use geist_world::ChunkCoord;
use raylib::prelude::*;

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
                            cause: RebuildCause::StreamLoad,
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
            let keys: Vec<ChunkCoord> = self.renders.keys().copied().collect();
            for coord in keys.iter().copied() {
                if let Some(ent) = self.gs.chunks.get_mut(&coord) {
                    ent.buf = None; // prevent reuse across worldgen param changes
                }
            }
            if self.rebuild_on_worldgen {
                for coord in keys.iter().copied() {
                    self.queue.emit_now(Event::ChunkRebuildRequested {
                        cx: coord.cx,
                        cy: coord.cy,
                        cz: coord.cz,
                        cause: RebuildCause::StreamLoad,
                    });
                }
                log::info!(
                    "Scheduled rebuild of {} loaded chunks due to worldgen change",
                    keys.len()
                );
            } else {
                log::info!(
                    "Worldgen changed; invalidated {} chunk buffers (rebuild on demand)",
                    keys.len()
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
        let want_edit = rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT)
            || rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT);
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
            if let Some(cpu) = r.cpu {
                // For mesh builds, pass through the grid; pack atlas later during event handling
                self.queue.emit_now(Event::BuildChunkJobCompleted {
                    cx: r.cx,
                    cy: r.cy,
                    cz: r.cz,
                    rev: r.rev,
                    cpu,
                    buf: r.buf,
                    light_borders: r.light_borders,
                    light_grid: r.light_grid,
                    job_id: r.job_id,
                });
            } else if let Some(lg) = r.light_grid {
                // If macro light borders were computed on the light-only lane, update them here
                // and notify neighbors on changes so they can refresh their seam rings.
                if let Some(lb) = r.light_borders {
                    let (changed, mask) = self.gs.lighting.update_borders_mask(r.cx, r.cz, lb);
                    if changed {
                        self.queue.emit_now(Event::LightBordersUpdated {
                            cx: r.cx,
                            cy: r.cy,
                            cz: r.cz,
                            xn_changed: mask.xn,
                            xp_changed: mask.xp,
                            yn_changed: false,
                            yp_changed: false,
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

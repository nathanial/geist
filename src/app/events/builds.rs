use super::{App, lighting};
use crate::event::{Event, RebuildCause};
use geist_chunk::{ChunkBuf, ChunkOccupancy};
use geist_lighting::{LightBorders, LightGrid, pack_light_grid_atlas_with_neighbors};
use geist_mesh_cpu::{ChunkMeshCPU, NeighborsLoaded};
use geist_render_raylib::{update_chunk_light_texture, upload_chunk_mesh};
use geist_runtime::{BuildJob, StructureBuildJob};
use geist_structures::StructureId;
use geist_world::ChunkCoord;
use geist_world::voxel::generation::ChunkColumnProfile;
use hashbrown::HashMap;
use raylib::prelude::*;
use std::sync::Arc;

impl App {
    pub(super) fn handle_build_chunk_job_requested(
        &mut self,
        coord: ChunkCoord,
        neighbors: NeighborsLoaded,
        rev: u64,
        job_id: u64,
        cause: RebuildCause,
    ) {
        let cx = coord.cx;
        let cy = coord.cy;
        let cz = coord.cz;
        let chunk_edits = self.gs.edits.snapshot_for_chunk(cx, cy, cz);
        let region_edits = self
            .gs
            .edits
            .snapshot_for_region(cx, cy, cz, 1, 1)
            .into_iter()
            .collect::<HashMap<_, _>>();
        let expected_rev = self.gs.world.current_worldgen_rev();
        let mut column_profile = self.runtime.column_cache().get(coord, expected_rev);
        if column_profile.is_none() {
            column_profile = self.gs.chunks.column_profile(&coord);
        }
        let prev_buf = self
            .gs
            .chunks
            .get(&coord)
            .and_then(|c| if c.has_blocks() { c.buf.as_ref() } else { None })
            .cloned();
        let job = BuildJob {
            cx,
            cy,
            cz,
            neighbors,
            rev,
            job_id,
            chunk_edits,
            region_edits,
            prev_buf,
            reg: self.reg.clone(),
            column_profile,
        };
        match cause {
            RebuildCause::Edit => {
                self.runtime.submit_build_job_edit(job);
            }
            RebuildCause::LightingBorder => {
                self.runtime.submit_build_job_light(job);
            }
            RebuildCause::StreamLoad | RebuildCause::HotReload => {
                self.runtime.submit_build_job_bg(job);
            }
        }
    }

    pub(super) fn handle_structure_build_requested(&mut self, id: StructureId, rev: u64) {
        if let Some(st) = self.gs.structures.get(&id) {
            let job = StructureBuildJob {
                id,
                rev,
                sx: st.sx,
                sy: st.sy,
                sz: st.sz,
                base_blocks: st.blocks.clone(),
                edits: st.edits.snapshot_all(),
                reg: self.reg.clone(),
            };
            self.runtime.submit_structure_build_job(job);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn handle_structure_build_completed(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        id: StructureId,
        rev: u64,
        cpu: ChunkMeshCPU,
        light_grid: LightGrid,
        light_borders: LightBorders,
    ) {
        if let Some(mut cr) =
            upload_chunk_mesh(rl, thread, cpu, &mut self.tex_cache, &self.reg.materials)
        {
            for part in &mut cr.parts {
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
                            unsafe {
                                std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1);
                            }
                        }
                    } else if tag == Some("water") {
                        if let Some(ref ws) = self.water_shader {
                            let dest = mat.shader_mut();
                            let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                            let src_ptr: *const raylib::ffi::Shader = ws.shader.as_ref();
                            unsafe {
                                std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1);
                            }
                        }
                    } else if let Some(ref fs) = self.fog_shader {
                        let dest = mat.shader_mut();
                        let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                        let src_ptr: *const raylib::ffi::Shader = fs.shader.as_ref();
                        unsafe {
                            std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1);
                        }
                    }
                }
            }
            let atlas = {
                let nb = lighting::structure_neighbor_borders(&light_borders);
                pack_light_grid_atlas_with_neighbors(&light_grid, &nb)
            };
            update_chunk_light_texture(rl, thread, &mut cr, &atlas);
            self.structure_renders.insert(id, cr);
        }
        self.structure_lights.insert(id, light_grid);
        self.structure_light_borders.insert(id, light_borders);
        if let Some(st) = self.gs.structures.get_mut(&id) {
            st.built_rev = rev;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn handle_build_chunk_job_completed(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        coord: ChunkCoord,
        rev: u64,
        occupancy: ChunkOccupancy,
        cpu: Option<ChunkMeshCPU>,
        buf: Option<ChunkBuf>,
        light_borders: Option<LightBorders>,
        light_grid: Option<LightGrid>,
        column_profile: Option<Arc<ChunkColumnProfile>>,
    ) {
        let cur_rev = self.gs.edits.get_rev(coord.cx, coord.cy, coord.cz);
        if rev < cur_rev {
            let inflight = self.gs.inflight_rev.get(&coord).copied().unwrap_or(0);
            if inflight < cur_rev {
                let neighbors = self.neighbor_mask(coord);
                let job_id = Self::job_hash(coord, cur_rev, neighbors);
                self.queue.emit_now(Event::BuildChunkJobRequested {
                    cx: coord.cx,
                    cy: coord.cy,
                    cz: coord.cz,
                    neighbors,
                    rev: cur_rev,
                    job_id,
                    cause: RebuildCause::Edit,
                });
                self.gs.inflight_rev.insert(coord, cur_rev);
            }
            return;
        }
        let center = self.gs.center_chunk;
        let dist_sq = center.distance_sq(coord);
        let keep_r = self.stream_evict_radius();
        let keep_sq = i64::from(keep_r) * i64::from(keep_r);
        if dist_sq > keep_sq {
            self.gs.inflight_rev.remove(&coord);
            return;
        }

        if let Some(profile) = column_profile.as_ref() {
            self.runtime.column_cache().insert(Arc::clone(profile));
        } else {
            self.gs.chunks.clear_column_profile(&coord);
        }

        if occupancy.is_empty() {
            self.renders.remove(&coord);
            self.gs.lighting.clear_chunk(coord);
            let entry =
                self.gs
                    .chunks
                    .mark_ready(coord, occupancy, None, rev, column_profile.clone());
            entry.lighting_ready = true;
            entry.mesh_ready = false;
            self.gs.inflight_rev.remove(&coord);
            self.gs.edits.mark_built(coord.cx, coord.cy, coord.cz, rev);
            self.gs.mesh_counts.remove(&coord);
            self.gs.light_counts.remove(&coord);
            self.mark_empty_chunk_ready(coord);
            return;
        }

        let cpu = match cpu {
            Some(cpu) => cpu,
            None => {
                log::warn!(
                    "populated chunk build missing mesh output at ({},{},{}) rev={}",
                    coord.cx,
                    coord.cy,
                    coord.cz,
                    rev
                );
                self.gs.inflight_rev.remove(&coord);
                return;
            }
        };
        let buf = match buf {
            Some(buf) => buf,
            None => {
                log::warn!(
                    "populated chunk build missing buffer at ({},{},{}) rev={}",
                    coord.cx,
                    coord.cy,
                    coord.cz,
                    rev
                );
                self.gs.inflight_rev.remove(&coord);
                return;
            }
        };
        if let Some(mut cr) =
            upload_chunk_mesh(rl, thread, cpu, &mut self.tex_cache, &self.reg.materials)
        {
            let sx = self.gs.world.chunk_size_x as i32;
            let sz = self.gs.world.chunk_size_z as i32;
            let wx = coord.cx * sx + sx / 2;
            let wz = coord.cz * sz + sz / 2;
            if let Some(b) = self.gs.world.biome_at(wx, wz) {
                if let Some(t) = b.leaf_tint {
                    cr.leaf_tint = Some(t);
                }
            }
            for part in &mut cr.parts {
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
                            unsafe {
                                std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1);
                            }
                        }
                    } else if tag == Some("water") {
                        if let Some(ref ws) = self.water_shader {
                            let dest = mat.shader_mut();
                            let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                            let src_ptr: *const raylib::ffi::Shader = ws.shader.as_ref();
                            unsafe {
                                std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1);
                            }
                        }
                    } else if let Some(ref fs) = self.fog_shader {
                        let dest = mat.shader_mut();
                        let dest_ptr: *mut raylib::ffi::Shader = dest.as_mut();
                        let src_ptr: *const raylib::ffi::Shader = fs.shader.as_ref();
                        unsafe {
                            std::ptr::copy_nonoverlapping(src_ptr, dest_ptr, 1);
                        }
                    }
                }
            }
            self.renders.insert(coord, cr);
            if let Some(ref lg) = light_grid {
                let nb = self.gs.lighting.get_neighbor_borders(coord);
                let atlas = pack_light_grid_atlas_with_neighbors(lg, &nb);
                self.validate_chunk_light_atlas(coord, &atlas);
                if let Some(cr) = self.renders.get_mut(&coord) {
                    update_chunk_light_texture(rl, thread, cr, &atlas);
                }
            }
        }
        let entry =
            self.gs
                .chunks
                .mark_ready(coord, occupancy, Some(buf), rev, column_profile.clone());
        entry.mesh_ready = true;
        entry.lighting_ready = light_grid.is_some();
        self.gs.inflight_rev.remove(&coord);
        self.gs.edits.mark_built(coord.cx, coord.cy, coord.cz, rev);
        *self.gs.mesh_counts.entry(coord).or_insert(0) += 1;
        if let Some(q) = self.perf_remove_start.get_mut(&coord) {
            if let Some(t0) = q.pop_front() {
                let dt_ms_u32 = t0.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
                Self::perf_push(&mut self.perf_remove_ms, dt_ms_u32);
                log::info!(
                    target: "perf",
                    "remove_to_render_ms={} cx={} cy={} cz={} rev={}",
                    dt_ms_u32,
                    coord.cx,
                    coord.cy,
                    coord.cz,
                    rev
                );
            }
            if q.is_empty() {
                self.perf_remove_start.remove(&coord);
            }
        }
        let mut notify_mask = geist_lighting::BorderChangeMask::default();
        if let Some(lb) = light_borders {
            let (changed, mask) = self.gs.lighting.update_borders_mask(coord, lb);
            if changed {
                notify_mask = mask;
            }
        }
        if let Some(ref lg) = light_grid {
            if lg.micro_change.any() {
                if !notify_mask.any() {
                    notify_mask = lg.micro_change;
                } else {
                    notify_mask.or_with(&lg.micro_change);
                }
            }
        }
        if notify_mask.any() {
            self.queue.emit_now(Event::LightBordersUpdated {
                cx: coord.cx,
                cy: coord.cy,
                cz: coord.cz,
                xn_changed: notify_mask.xn,
                xp_changed: notify_mask.xp,
                yn_changed: notify_mask.yn,
                yp_changed: notify_mask.yp,
                zn_changed: notify_mask.zn,
                zp_changed: notify_mask.zp,
            });
        }
        if let Some(st) = self.gs.finalize.get(&coord).copied() {
            if st.owner_neg_x_ready
                && st.owner_neg_y_ready
                && st.owner_neg_z_ready
                && !st.finalized
                && !st.finalize_requested
            {
                self.try_schedule_finalize(coord);
            }
        }
        if let Some(st) = self.gs.finalize.get_mut(&coord) {
            if st.finalize_requested {
                st.finalize_requested = false;
                st.finalized = true;
            }
        }
    }
}

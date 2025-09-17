use std::collections::VecDeque;

use super::App;
use super::state::{IntentCause, IntentEntry};
use crate::event::{Event, RebuildCause};
use crate::gamestate::FinalizeState;
use geist_lighting::LightAtlas;
use geist_mesh_cpu::NeighborsLoaded;
use geist_world::ChunkCoord;

// Scheduling/queue tuning knobs
// Increase per-frame submissions and per-lane queue headroom so workers stay busier.
const JOB_FRAME_CAP_MULT: usize = 4; // was 2
const LANE_QUEUE_EXTRA: usize = 3; // was 1 (target = workers + extra)
const PERF_WIN_CAP: usize = 200; // rolling window size for perf stats

impl App {
    #[inline]
    pub(super) fn perf_push(q: &mut VecDeque<u32>, v: u32) {
        q.push_back(v);
        if q.len() > PERF_WIN_CAP {
            q.pop_front();
        }
    }

    pub(super) fn validate_chunk_light_atlas(&self, coord: ChunkCoord, atlas: &LightAtlas) {
        let cx = coord.cx;
        let cy = coord.cy;
        let cz = coord.cz;
        // Compare atlas border rings against LightingStore neighbor planes; panic on mismatch.
        let nb = self.gs.lighting.get_neighbor_borders(coord);
        let width = atlas.width;
        let grid_cols = atlas.grid_cols;
        let tile_w = atlas.sx; // extended: sx + 2
        let tile_h = atlas.sz; // extended: sz + 2
        let inner_sx = tile_w.saturating_sub(2);
        let inner_sz = tile_h.saturating_sub(2);
        let data = &atlas.data;
        let at = |x: usize, y: usize| -> (u8, u8, u8) {
            let di = (y * width + x) * 4;
            (data[di + 0], data[di + 1], data[di + 2])
        };
        let total_slices = atlas.sy;
        let inner_sy = total_slices.saturating_sub(2);
        for slice in 0..total_slices {
            let tx = slice % grid_cols;
            let ty = slice / grid_cols;
            let ox = tx * tile_w;
            let oy = ty * tile_h;
            if slice == 0 {
                if let (Some(ref blk), Some(ref sky), Some(ref bcn)) =
                    (nb.yn.as_ref(), nb.sk_yn.as_ref(), nb.bcn_yn.as_ref())
                {
                    for z in 0..inner_sz {
                        for x in 0..inner_sx {
                            let (r, g, b) = at(ox + 1 + x, oy + 1 + z);
                            let ii = z * inner_sx + x;
                            let er = blk.get(ii).cloned().unwrap_or(0);
                            let eg = sky.get(ii).cloned().unwrap_or(0);
                            let eb = bcn.get(ii).cloned().unwrap_or(0);
                            if r != er || g != eg || b != eb {
                                panic!(
                                    "Light atlas -Y plane mismatch at chunk ({},{},{}) z={} x={} got=({},{},{}) exp=({},{},{})",
                                    cx, cy, cz, z, x, r, g, b, er, eg, eb
                                );
                            }
                        }
                    }
                }
                continue;
            }
            if slice == inner_sy + 1 {
                if let (Some(ref blk), Some(ref sky), Some(ref bcn)) =
                    (nb.yp.as_ref(), nb.sk_yp.as_ref(), nb.bcn_yp.as_ref())
                {
                    for z in 0..inner_sz {
                        for x in 0..inner_sx {
                            let (r, g, b) = at(ox + 1 + x, oy + 1 + z);
                            let ii = z * inner_sx + x;
                            let er = blk.get(ii).cloned().unwrap_or(0);
                            let eg = sky.get(ii).cloned().unwrap_or(0);
                            let eb = bcn.get(ii).cloned().unwrap_or(0);
                            if r != er || g != eg || b != eb {
                                panic!(
                                    "Light atlas +Y plane mismatch at chunk ({},{},{}) z={} x={} got=({},{},{}) exp=({},{},{})",
                                    cx, cy, cz, z, x, r, g, b, er, eg, eb
                                );
                            }
                        }
                    }
                }
                continue;
            }
            let y = slice - 1;
            if let (Some(ref blk), Some(ref sky), Some(ref bcn)) =
                (nb.xn.as_ref(), nb.sk_xn.as_ref(), nb.bcn_xn.as_ref())
            {
                for z in 0..inner_sz {
                    let (r, g, b) = at(ox + 0, oy + 1 + z);
                    let ii = y * inner_sz + z;
                    let er = blk.get(ii).cloned().unwrap_or(0);
                    let eg = sky.get(ii).cloned().unwrap_or(0);
                    let eb = bcn.get(ii).cloned().unwrap_or(0);
                    if r != er || g != eg || b != eb {
                        panic!(
                            "Light atlas -X ring mismatch at chunk ({},{},{}) slice y={} z={} got=({},{},{}) exp=({},{},{})",
                            cx, cy, cz, y, z, r, g, b, er, eg, eb
                        );
                    }
                }
            }
            if let (Some(ref blk), Some(ref sky), Some(ref bcn)) =
                (nb.xp.as_ref(), nb.sk_xp.as_ref(), nb.bcn_xp.as_ref())
            {
                for z in 0..inner_sz {
                    let (r, g, b) = at(ox + (inner_sx + 1), oy + 1 + z);
                    let ii = y * inner_sz + z;
                    let er = blk.get(ii).cloned().unwrap_or(0);
                    let eg = sky.get(ii).cloned().unwrap_or(0);
                    let eb = bcn.get(ii).cloned().unwrap_or(0);
                    if r != er || g != eg || b != eb {
                        panic!(
                            "Light atlas +X ring mismatch at chunk ({},{},{}) slice y={} z={} got=({},{},{}) exp=({},{},{})",
                            cx, cy, cz, y, z, r, g, b, er, eg, eb
                        );
                    }
                }
            }
            if let (Some(ref blk), Some(ref sky), Some(ref bcn)) =
                (nb.zn.as_ref(), nb.sk_zn.as_ref(), nb.bcn_zn.as_ref())
            {
                for x in 0..inner_sx {
                    let (r, g, b) = at(ox + 1 + x, oy + 0);
                    let ii = y * inner_sx + x;
                    let er = blk.get(ii).cloned().unwrap_or(0);
                    let eg = sky.get(ii).cloned().unwrap_or(0);
                    let eb = bcn.get(ii).cloned().unwrap_or(0);
                    if r != er || g != eg || b != eb {
                        panic!(
                            "Light atlas -Z ring mismatch at chunk ({},{},{}) slice y={} x={} got=({},{},{}) exp=({},{},{})",
                            cx, cy, cz, y, x, r, g, b, er, eg, eb
                        );
                    }
                }
            }
            if let (Some(ref blk), Some(ref sky), Some(ref bcn)) =
                (nb.zp.as_ref(), nb.sk_zp.as_ref(), nb.bcn_zp.as_ref())
            {
                for x in 0..inner_sx {
                    let (r, g, b) = at(ox + 1 + x, oy + (inner_sz + 1));
                    let ii = y * inner_sx + x;
                    let er = blk.get(ii).cloned().unwrap_or(0);
                    let eg = sky.get(ii).cloned().unwrap_or(0);
                    let eb = bcn.get(ii).cloned().unwrap_or(0);
                    if r != er || g != eg || b != eb {
                        panic!(
                            "Light atlas +Z ring mismatch at chunk ({},{},{}) slice y={} x={} got=({},{},{}) exp=({},{},{})",
                            cx, cy, cz, y, x, r, g, b, er, eg, eb
                        );
                    }
                }
            }
        }
    }

    pub(super) fn try_schedule_finalize(&mut self, coord: ChunkCoord) {
        let st = self
            .gs
            .finalize
            .entry(coord)
            .or_insert(FinalizeState::default());
        if st.finalized || st.finalize_requested {
            return;
        }
        if !(st.owner_neg_x_ready && st.owner_neg_y_ready && st.owner_neg_z_ready) {
            return;
        }
        if !self.gs.chunks.mesh_ready(coord) {
            return;
        }
        if self.gs.inflight_rev.contains_key(&coord) {
            return;
        }
        self.queue.emit_now(Event::ChunkRebuildRequested {
            cx: coord.cx,
            cy: coord.cy,
            cz: coord.cz,
            cause: RebuildCause::LightingBorder,
        });
        st.finalize_requested = true;
    }

    pub(super) fn record_intent(&mut self, coord: ChunkCoord, cause: IntentCause) {
        let cur_rev = self.gs.edits.get_rev(coord.cx, coord.cy, coord.cz);
        let now = self.gs.tick;
        self.intents
            .entry(coord)
            .and_modify(|e| {
                if cur_rev > e.rev {
                    e.rev = cur_rev;
                }
                if cause < e.cause {
                    e.cause = cause;
                }
                e.last_tick = now;
            })
            .or_insert(IntentEntry {
                rev: cur_rev,
                cause,
                last_tick: now,
            });
    }

    pub(super) fn flush_intents(&mut self) {
        if self.intents.is_empty() {
            return;
        }
        let ccx = (self.cam.position.x / self.gs.world.chunk_size_x as f32).floor() as i32;
        let ccy = (self.cam.position.y / self.gs.world.chunk_size_y as f32).floor() as i32;
        let ccz = (self.cam.position.z / self.gs.world.chunk_size_z as f32).floor() as i32;
        let now = self.gs.tick;
        let mut items: Vec<(ChunkCoord, IntentEntry, i64, i32)> =
            Vec::with_capacity(self.intents.len());
        for (&key, &ent) in self.intents.iter() {
            let cx = key.cx;
            let cz = key.cz;
            let dx = i64::from(cx - ccx);
            let dy = i64::from(key.cy - ccy);
            let dz = i64::from(cz - ccz);
            let dist_bucket: i64 = dx * dx + dy * dy + dz * dz;
            let age = now.saturating_sub(ent.last_tick);
            let age_boost: i32 = if age > 180 {
                -2
            } else if age > 60 {
                -1
            } else {
                0
            };
            items.push((key, ent, dist_bucket, age_boost));
        }
        items.sort_by(|a, b| {
            a.1.cause
                .cmp(&b.1.cause)
                .then(a.2.cmp(&b.2))
                .then(a.3.cmp(&b.3))
        });

        let worker_n = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8);
        let cap = (worker_n * JOB_FRAME_CAP_MULT).max(8);
        let mut submitted = 0usize;
        let mut submitted_keys: Vec<ChunkCoord> = Vec::new();

        let (q_e, if_e, q_l, if_l, q_b, if_b) = self.runtime.queue_debug_counts();
        let target_edit = self.runtime.w_edit.max(1) + LANE_QUEUE_EXTRA;
        let target_light = self.runtime.w_light.max(1) + LANE_QUEUE_EXTRA;
        let target_bg = self.runtime.w_bg.max(1) + LANE_QUEUE_EXTRA;
        let mut budget_edit = target_edit.saturating_sub(q_e + if_e);
        let mut budget_light = target_light.saturating_sub(q_l + if_l);
        let mut budget_bg = target_bg.saturating_sub(q_b + if_b);
        let r = self.gs.view_radius_chunks.max(0);
        let gate_light_sq = {
            let rr = r + 1;
            i64::from(rr) * i64::from(rr)
        };
        let gate_stream_sq = i64::from(r) * i64::from(r);

        for (key, ent, dist_bucket, _ab) in items.into_iter() {
            if submitted >= cap {
                break;
            }
            let cx = key.cx;
            let cz = key.cz;
            if self
                .gs
                .inflight_rev
                .get(&key)
                .map(|v| *v >= ent.rev)
                .unwrap_or(false)
            {
                continue;
            }
            let neighbors = self.neighbor_mask(key);
            let rev = ent.rev;
            let job_id = Self::job_hash(key, rev, neighbors);
            let is_ready = self.gs.chunks.is_ready(key);
            match ent.cause {
                IntentCause::Edit => {
                    if budget_edit == 0 {
                        continue;
                    }
                }
                IntentCause::Light => {
                    if dist_bucket > gate_light_sq {
                        continue;
                    }
                    if budget_light == 0 {
                        continue;
                    }
                }
                IntentCause::StreamLoad => {
                    if is_ready {
                        continue;
                    }
                    if dist_bucket > gate_stream_sq {
                        continue;
                    }
                    if budget_bg == 0 {
                        continue;
                    }
                }
                IntentCause::HotReload => {
                    if dist_bucket > gate_stream_sq {
                        continue;
                    }
                    if budget_bg == 0 {
                        continue;
                    }
                }
            }
            let cause = match ent.cause {
                IntentCause::Edit => RebuildCause::Edit,
                IntentCause::Light => RebuildCause::LightingBorder,
                IntentCause::StreamLoad => RebuildCause::StreamLoad,
                IntentCause::HotReload => RebuildCause::HotReload,
            };
            self.queue.emit_after(
                1,
                Event::BuildChunkJobRequested {
                    cx,
                    cy: key.cy,
                    cz,
                    neighbors,
                    rev,
                    job_id,
                    cause,
                },
            );
            self.gs.inflight_rev.insert(key, rev);
            submitted_keys.push(key);
            submitted += 1;
            match ent.cause {
                IntentCause::Edit => {
                    budget_edit = budget_edit.saturating_sub(1);
                }
                IntentCause::Light => {
                    budget_light = budget_light.saturating_sub(1);
                }
                IntentCause::StreamLoad | IntentCause::HotReload => {
                    budget_bg = budget_bg.saturating_sub(1);
                }
            }
        }
        for k in submitted_keys {
            self.intents.remove(&k);
        }
    }

    pub(super) fn neighbor_mask(&self, coord: ChunkCoord) -> NeighborsLoaded {
        NeighborsLoaded {
            neg_x: self.gs.chunks.mesh_ready(coord.offset(-1, 0, 0)),
            pos_x: self.gs.chunks.mesh_ready(coord.offset(1, 0, 0)),
            neg_y: self.gs.chunks.mesh_ready(coord.offset(0, -1, 0)),
            pos_y: self.gs.chunks.mesh_ready(coord.offset(0, 1, 0)),
            neg_z: self.gs.chunks.mesh_ready(coord.offset(0, 0, -1)),
            pos_z: self.gs.chunks.mesh_ready(coord.offset(0, 0, 1)),
        }
    }

    pub(super) fn job_hash(coord: ChunkCoord, rev: u64, n: NeighborsLoaded) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut write = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x100000001b3);
        };
        write(coord.cx as u64);
        write(coord.cy as u64);
        write(coord.cz as u64);
        write(rev);
        let mask = (n.neg_x as u64)
            | ((n.pos_x as u64) << 1)
            | ((n.neg_y as u64) << 2)
            | ((n.pos_y as u64) << 3)
            | ((n.neg_z as u64) << 4)
            | ((n.pos_z as u64) << 5);
        write(mask);
        h
    }
}

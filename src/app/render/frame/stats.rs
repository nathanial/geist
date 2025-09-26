use std::collections::HashSet;

use super::super::{App, DebugStats};

impl App {
    pub(super) fn reset_render_debug_stats(&mut self) {
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
    }

    pub(super) fn update_chunk_debug_stats(&mut self) {
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
    }

    pub(super) fn update_lighting_debug_stats(&mut self) {
        let light_stats = self.gs.lighting.stats();
        self.debug_stats.lighting_border_chunks = light_stats.border_chunks;
        self.debug_stats.lighting_emitter_chunks = light_stats.emitter_chunks;
        self.debug_stats.lighting_micro_chunks = light_stats.micro_chunks;
    }

    pub(super) fn update_edit_debug_stats(&mut self) {
        let edit_stats = self.gs.edits.stats();
        self.debug_stats.edit_chunk_entries = edit_stats.chunk_entries;
        self.debug_stats.edit_block_edits = edit_stats.block_edits;
        self.debug_stats.edit_rev_entries = edit_stats.rev_entries;
        self.debug_stats.edit_built_entries = edit_stats.built_entries;
    }
}

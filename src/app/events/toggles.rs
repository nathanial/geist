use super::App;
use geist_blocks::Block;
use raylib::prelude::Vector3;

impl App {
    pub(super) fn handle_walk_mode_toggled(&mut self) {
        let new_mode = !self.gs.walk_mode;
        self.gs.walk_mode = new_mode;
        if new_mode {
            self.gs.walker.yaw = self.cam.yaw;
            let mut p = self.cam.position;
            p.y -= self.gs.walker.eye_height;
            p.y = p.y.max(0.0);
            self.gs.walker.pos = p;
            self.gs.walker.vel = Vector3::zero();
            self.gs.walker.on_ground = false;
            self.cam.position = self.gs.walker.eye_position();
        }
    }

    pub(super) fn handle_grid_toggle(&mut self) {
        self.gs.show_grid = !self.gs.show_grid;
    }

    pub(super) fn handle_wireframe_toggle(&mut self) {
        self.gs.wireframe = !self.gs.wireframe;
    }

    pub(super) fn handle_chunk_bounds_toggle(&mut self) {
        self.gs.show_chunk_bounds = !self.gs.show_chunk_bounds;
    }

    pub(super) fn handle_frustum_culling_toggle(&mut self) {
        self.gs.frustum_culling_enabled = !self.gs.frustum_culling_enabled;
    }

    pub(super) fn handle_biome_label_toggle(&mut self) {
        self.gs.show_biome_label = !self.gs.show_biome_label;
    }

    pub(super) fn handle_debug_overlay_toggle(&mut self) {
        self.gs.show_debug_overlay = !self.gs.show_debug_overlay;
    }

    pub(super) fn handle_place_type_selected(&mut self, block: Block) {
        self.gs.place_type = block;
    }
}

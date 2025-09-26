use raylib::prelude::*;

use super::App;
use super::GeistDraw;

mod hud;
mod overlay;
mod stats;
mod world;

impl App {
    pub fn render(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread) {
        self.reset_render_debug_stats();
        self.update_chunk_debug_stats();
        self.update_lighting_debug_stats();
        self.update_edit_debug_stats();

        let screen_width = rl.get_screen_width() as f32;
        let screen_height = rl.get_screen_height() as f32;
        let aspect_ratio = screen_width / screen_height;
        let frustum = self.cam.calculate_frustum(aspect_ratio, 0.1, 10000.0);

        let time_now = rl.get_time() as f32;
        let sample = self.day_sample;
        let sky_scale = sample.sky_scale;
        let surface_sky = sample.surface_sky;
        let sun_id = self.sun.as_ref().map(|s| s.id);
        let sun_tint = world::sun_tint_color(sample);

        let camera3d = self.cam.to_camera3d();
        self.minimap_ui_rect = None;

        let screen_dims = (screen_width as i32, screen_height as i32);
        let overlay_theme = *self.overlay_windows.theme();
        let minimap_render_side = self.prepare_minimap_render_side(screen_dims, overlay_theme);
        self.render_minimap_to_texture(rl, thread, minimap_render_side);

        let cursor_position = rl.get_mouse_position();
        let mouse_left_pressed = rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT);

        let font_for_frame = self.ui_font.clone();
        let mut d = GeistDraw::new(rl.begin_drawing(thread), font_for_frame);
        d.clear_background(world::surface_color(surface_sky));

        unsafe {
            raylib::ffi::rlClearScreenBuffers();
        }

        self.draw_world_scene(
            &mut d,
            thread,
            camera3d,
            &frustum,
            time_now,
            sky_scale,
            surface_sky,
            sun_id,
            sun_tint,
        );

        self.draw_debug_overlay(
            &mut d,
            screen_dims,
            overlay_theme,
            cursor_position,
            mouse_left_pressed,
        );

        self.draw_hud(&mut d);

        if !self.gs.show_debug_overlay {
            return;
        }
    }
}

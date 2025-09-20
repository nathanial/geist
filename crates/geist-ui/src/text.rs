use raylib::prelude::{Color, RaylibDraw, RaylibDrawHandle};

/// Measurement interface so UI layout can compute text bounds consistently.
pub trait UiTextMeasure {
    fn ui_measure_text(&self, text: &str, font_size: i32) -> i32;
}

/// Drawing interface that honors the same font overrides used for measurement.
pub trait UiTextRenderer: UiTextMeasure {
    fn ui_draw_text(&mut self, text: &str, x: i32, y: i32, font_size: i32, color: Color);
}

impl UiTextMeasure for RaylibDrawHandle<'_> {
    fn ui_measure_text(&self, text: &str, font_size: i32) -> i32 {
        self.measure_text(text, font_size)
    }
}

impl<T: UiTextMeasure + ?Sized> UiTextMeasure for &T {
    fn ui_measure_text(&self, text: &str, font_size: i32) -> i32 {
        (*self).ui_measure_text(text, font_size)
    }
}

impl UiTextRenderer for RaylibDrawHandle<'_> {
    fn ui_draw_text(&mut self, text: &str, x: i32, y: i32, font_size: i32, color: Color) {
        self.draw_text(text, x, y, font_size, color);
    }
}

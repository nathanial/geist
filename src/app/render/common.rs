use raylib::core::drawing::RaylibDraw;
use raylib::core::text::RaylibFont;
use raylib::prelude::*;
use std::sync::Arc;

use super::{UiTextMeasure, UiTextRenderer, WindowFrame};

pub(crate) fn format_count(count: usize) -> String {
    match count {
        0..=999 => count.to_string(),
        1_000..=9_999 => format!("{:.1}k", count as f32 / 1_000.0),
        10_000..=999_999 => format!("{}k", count / 1_000),
        1_000_000..=9_999_999 => format!("{:.1}M", count as f32 / 1_000_000.0),
        _ => format!("{}M", count / 1_000_000),
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub(crate) struct ContentLayout {
    pub(crate) available_height: i32,
    pub(crate) used_height: i32,
    pub(crate) overflow_rows: usize,
    pub(crate) overflow_items: usize,
}

impl ContentLayout {
    pub(crate) fn new(available_height: i32) -> Self {
        Self {
            available_height,
            ..Default::default()
        }
    }

    pub(crate) fn add_rows(&mut self, rows: usize, row_height: i32) {
        self.used_height += (rows as i32) * row_height;
    }

    pub(crate) fn add_custom(&mut self, height: i32) {
        self.used_height += height.max(0);
    }

    pub(crate) fn mark_overflow(&mut self, rows: usize, items: usize) {
        self.overflow_rows += rows;
        self.overflow_items += items;
    }

    pub(crate) fn overflow(&self) -> bool {
        self.used_height > self.available_height || self.overflow_items > 0
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DisplayLine {
    pub(crate) text: String,
    pub(crate) color: Color,
    pub(crate) font: i32,
    pub(crate) line_height: i32,
    pub(crate) indent: i32,
}

impl DisplayLine {
    pub(crate) fn new(text: impl Into<String>, font: i32, color: Color) -> Self {
        let font = font.max(1);
        Self {
            text: text.into(),
            color,
            font,
            line_height: font + 4,
            indent: 0,
        }
    }

    pub(crate) fn with_indent(mut self, indent: i32) -> Self {
        self.indent = indent.max(0);
        self
    }

    pub(crate) fn with_line_height(mut self, line_height: i32) -> Self {
        self.line_height = line_height.max(self.font);
        self
    }
}

pub(crate) struct GeistDraw<'a> {
    pub(crate) inner: RaylibDrawHandle<'a>,
    pub(crate) font: Option<Arc<Font>>,
}

impl<'a> GeistDraw<'a> {
    pub(crate) fn new(inner: RaylibDrawHandle<'a>, font: Option<Arc<Font>>) -> Self {
        Self { inner, font }
    }

    pub(crate) fn draw_text(&mut self, text: &str, x: i32, y: i32, font_size: i32, color: Color) {
        if let Some(ref font) = self.font {
            let fs = font_size.max(1) as f32;
            let spacing = self.letter_spacing(font, fs);
            let position = Vector2::new(x as f32, y as f32);
            self.inner
                .draw_text_ex(&**font, text, position, fs, spacing, color);
        } else {
            self.inner.draw_text(text, x, y, font_size, color);
        }
    }

    pub(crate) fn measure_text(&self, text: &str, font_size: i32) -> i32 {
        if let Some(ref font) = self.font {
            let fs = font_size.max(1) as f32;
            let spacing = self.letter_spacing(font, fs);
            let size = font.measure_text(text, fs, spacing);
            size.x.round() as i32
        } else {
            self.inner.measure_text(text, font_size)
        }
    }

    fn letter_spacing(&self, font: &Font, font_size: f32) -> f32 {
        let base = font.base_size().max(1) as f32;
        font_size / base
    }
}

impl<'a> std::ops::Deref for GeistDraw<'a> {
    type Target = RaylibDrawHandle<'a>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<'a> std::ops::DerefMut for GeistDraw<'a> {
    fn deref_mut(&mut self) -> &mut RaylibDrawHandle<'a> {
        &mut self.inner
    }
}

impl<'a> RaylibDraw for GeistDraw<'a> {}

impl UiTextMeasure for GeistDraw<'_> {
    fn ui_measure_text(&self, text: &str, font_size: i32) -> i32 {
        self.measure_text(text, font_size)
    }
}

impl UiTextRenderer for GeistDraw<'_> {
    fn ui_draw_text(&mut self, text: &str, x: i32, y: i32, font_size: i32, color: Color) {
        self.draw_text(text, x, y, font_size, color);
    }
}

pub(crate) fn draw_lines(
    d: &mut GeistDraw,
    lines: &[DisplayLine],
    frame: &WindowFrame,
) -> ContentLayout {
    let content = frame.content;
    let mut layout = ContentLayout::new(content.h);
    if content.h <= 0 || content.w <= 0 {
        return layout;
    }
    let offset_y = frame.scroll.offset.y.max(0.0).round() as i32;
    let mut y = content.y - offset_y;
    {
        let mut scoped = d.begin_scissor_mode(content.x, content.y, content.w, content.h);
        for (idx, line) in lines.iter().enumerate() {
            let next_y = y + line.line_height;
            layout.add_custom(line.line_height);
            if next_y > content.y && y < content.y + content.h {
                if !line.text.is_empty() {
                    scoped.draw_text(
                        &line.text,
                        content.x + line.indent,
                        y,
                        line.font,
                        line.color,
                    );
                }
            }
            if next_y >= content.y + content.h {
                let remaining = lines.len().saturating_sub(idx + 1);
                if remaining > 0 {
                    layout.mark_overflow(remaining, remaining);
                }
                break;
            }
            y = next_y;
        }
    }
    layout
}

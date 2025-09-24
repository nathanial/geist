use raylib::prelude::Color;

pub fn blend_color(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    Color::new(
        ((a.r as f32) * inv + (b.r as f32) * t).round() as u8,
        ((a.g as f32) * inv + (b.g as f32) * t).round() as u8,
        ((a.b as f32) * inv + (b.b as f32) * t).round() as u8,
        ((a.a as f32) * inv + (b.a as f32) * t).round() as u8,
    )
}

pub fn scale_alpha(color: Color, factor: f32) -> Color {
    let factor = factor.clamp(0.0, 1.0);
    Color::new(
        color.r,
        color.g,
        color.b,
        ((color.a as f32) * factor).round() as u8,
    )
}

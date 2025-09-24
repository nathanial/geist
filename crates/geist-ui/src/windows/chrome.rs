use raylib::prelude::RaylibDraw;

use crate::text::UiTextRenderer;

use super::util::{blend_color, scale_alpha};
use super::{HitRegion, IRect, ResizeHandle, WindowButton, WindowFrame, WindowState, WindowTheme};

pub struct WindowChrome;

impl WindowChrome {
    #[allow(clippy::too_many_arguments)]
    pub fn draw<D>(
        d: &mut D,
        theme: &WindowTheme,
        frame: &WindowFrame,
        title: &str,
        subtitle: Option<&str>,
        hover_region: Option<HitRegion>,
        state: WindowState,
        is_focused: bool,
        is_pinned: bool,
    ) where
        D: RaylibDraw + UiTextRenderer,
    {
        let hovered_title = matches!(hover_region, Some(HitRegion::TitleBar));
        let hovered_button = match hover_region {
            Some(HitRegion::TitleBarButton(button)) => Some(button),
            _ => None,
        };
        let hovered_handle = match hover_region {
            Some(HitRegion::Resize(handle)) => Some(handle),
            _ => None,
        };

        let outer = frame.outer;
        let IRect {
            x,
            y,
            w: width,
            h: height,
        } = outer;

        if is_focused {
            d.draw_rectangle_lines(x - 1, y - 1, width + 2, height + 2, theme.focus_glow);
        }

        d.draw_rectangle(x + 6, y + 8, width, height, theme.frame_shadow);
        d.draw_rectangle(x, y, width, height, theme.frame_color);

        let mid = (theme.titlebar_height / 2).min(height);
        let mut title_top = if hovered_title {
            theme.title_hover_top
        } else {
            theme.title_top
        };
        let mut title_bottom = if hovered_title {
            theme.title_hover_bottom
        } else {
            theme.title_bottom
        };
        if !is_focused {
            title_top = blend_color(title_top, theme.frame_color, 0.35);
            title_bottom = blend_color(title_bottom, theme.frame_color, 0.35);
        }
        if mid > 0 {
            d.draw_rectangle(x, y, width, mid, title_top);
            d.draw_rectangle(x, y + mid, width, theme.titlebar_height - mid, title_bottom);
        }
        if height > theme.titlebar_height {
            d.draw_rectangle(
                x,
                y + theme.titlebar_height,
                width,
                height - theme.titlebar_height,
                theme.body_color,
            );
        }

        d.draw_rectangle(x, y, width, 1, theme.top_highlight);
        d.draw_rectangle(
            x,
            y + theme.titlebar_height - 1,
            width,
            1,
            theme.title_border,
        );
        let outline_color = if is_focused {
            theme.focus_outline
        } else {
            theme.outline
        };
        d.draw_rectangle_lines(x, y, width, height, outline_color);
        if width > 2 && height > 2 {
            d.draw_rectangle_lines(x + 1, y + 1, width - 2, height - 2, theme.inner_outline);
        }

        if is_pinned {
            d.draw_rectangle(x, y + 2, width, 1, theme.pinned_outline);
        }

        let title_y = y + (theme.titlebar_height - theme.title_font) / 2;
        d.ui_draw_text(
            title,
            x + theme.padding_x,
            title_y,
            theme.title_font,
            theme.title_text,
        );

        if let Some(subtitle) = subtitle {
            let subtitle_w = d.ui_measure_text(subtitle, theme.subtitle_font);
            let subtitle_y = title_y + theme.title_font - theme.subtitle_font - 2;
            let subtitle_x = x + width - theme.padding_x - subtitle_w;
            d.ui_draw_text(
                subtitle,
                subtitle_x,
                subtitle_y,
                theme.subtitle_font,
                theme.subtitle_text,
            );
        }

        for slot in frame.title_buttons.iter().flatten() {
            let is_hovered_button = hovered_button == Some(slot.button);
            let mut base_color = if slot.button == WindowButton::Pin && is_pinned {
                theme.button_active
            } else {
                theme.button_normal
            };
            if is_hovered_button {
                base_color = blend_color(base_color, theme.button_hover, 0.6);
            }

            d.draw_rectangle(
                slot.rect.x,
                slot.rect.y,
                slot.rect.w,
                slot.rect.h,
                base_color,
            );
            d.draw_rectangle_lines(
                slot.rect.x,
                slot.rect.y,
                slot.rect.w,
                slot.rect.h,
                theme.inner_outline,
            );

            let icon_color = if is_hovered_button {
                theme.button_icon_hover
            } else {
                theme.button_icon
            };

            match slot.button {
                WindowButton::Minimize => {
                    let y_line = slot.rect.y + slot.rect.h - (slot.rect.h / 4).max(4);
                    d.draw_line(
                        slot.rect.x + 4,
                        y_line,
                        slot.rect.x + slot.rect.w - 4,
                        y_line,
                        icon_color,
                    );
                }
                WindowButton::Maximize => {
                    let size = (slot.rect.w.min(slot.rect.h) - 8).max(6);
                    let offset_x = slot.rect.x + (slot.rect.w - size) / 2;
                    let offset_y = slot.rect.y + (slot.rect.h - size) / 2;
                    d.draw_rectangle_lines(offset_x, offset_y, size, size, icon_color);
                }
                WindowButton::Restore => {
                    let size = (slot.rect.w.min(slot.rect.h) - 10).max(6);
                    let inner = (size - 2).max(4);
                    let back_x = slot.rect.x + (slot.rect.w - size) / 2;
                    let back_y = slot.rect.y + (slot.rect.h - size) / 2 + 2;
                    let front_x = back_x + 3;
                    let front_y = back_y - 3;
                    d.draw_rectangle_lines(
                        back_x,
                        back_y,
                        size,
                        size,
                        scale_alpha(icon_color, 0.7),
                    );
                    d.draw_rectangle_lines(front_x, front_y, inner, inner, icon_color);
                }
                WindowButton::Pin => {
                    let cx = slot.rect.x + slot.rect.w / 2;
                    let cy = slot.rect.y + slot.rect.h / 2;
                    d.draw_circle(cx, cy - 2, 3.0, icon_color);
                    d.draw_line(cx, cy - 2, cx, cy + 4, icon_color);
                    d.draw_circle(cx, cy + 4, 2.0, icon_color);
                }
            }
        }

        let scroll = frame.scroll;
        if state != WindowState::Minimized
            && scroll.content_size.1 > scroll.viewport_size.1
            && scroll.viewport_size.1 > 0
        {
            let track_x = frame.content.x + frame.content.w - 6;
            let track_y = frame.content.y;
            let track_w = 4;
            let track_h = frame.content.h;
            if track_w > 0 && track_h > 0 {
                d.draw_rectangle(
                    track_x,
                    track_y,
                    track_w,
                    track_h,
                    scale_alpha(theme.inner_outline, 0.25),
                );
                let content_h = scroll.content_size.1.max(1) as f32;
                let viewport_h = scroll.viewport_size.1.max(1) as f32;
                let ratio = (viewport_h / content_h).clamp(0.05, 1.0);
                let handle_h = (track_h as f32 * ratio).clamp(20.0, track_h as f32);
                let max_offset = (scroll.content_size.1 - scroll.viewport_size.1).max(1) as f32;
                let offset_ratio = (scroll.offset.y / max_offset).clamp(0.0, 1.0);
                let handle_y = track_y as f32 + (track_h as f32 - handle_h) * offset_ratio;
                let handle_color = if is_focused {
                    blend_color(theme.button_hover, theme.focus_outline, 0.35)
                } else {
                    blend_color(theme.button_normal, theme.inner_outline, 0.35)
                };
                d.draw_rectangle(
                    track_x,
                    handle_y.round() as i32,
                    track_w,
                    handle_h.round().max(8.0) as i32,
                    handle_color,
                );
            }
        }

        if state == WindowState::Normal {
            for slot in frame.resize_handles.iter().flatten() {
                let highlight = hovered_handle == Some(slot.handle);
                let fill = if highlight {
                    theme.resize_fill_hover
                } else {
                    scale_alpha(theme.resize_fill, 0.25)
                };
                let outline = if highlight {
                    theme.resize_outline_hover
                } else {
                    scale_alpha(theme.resize_outline, 0.3)
                };
                d.draw_rectangle(slot.rect.x, slot.rect.y, slot.rect.w, slot.rect.h, fill);
                d.draw_rectangle_lines(slot.rect.x, slot.rect.y, slot.rect.w, slot.rect.h, outline);

                let diagonal_corner = matches!(
                    slot.handle,
                    ResizeHandle::BottomRight | ResizeHandle::TopRight
                ) || matches!(
                    slot.handle,
                    ResizeHandle::BottomLeft | ResizeHandle::TopLeft
                );

                if diagonal_corner {
                    let step = (slot.rect.w.min(slot.rect.h) / 4).max(3);
                    for i in 0..3 {
                        let offset = i * step;
                        let start_x = slot.rect.x + offset;
                        let start_y = slot.rect.y + slot.rect.h - 2;
                        let end_x = slot.rect.x + slot.rect.w - 2;
                        let end_y = slot.rect.y + offset;
                        d.draw_line(
                            start_x,
                            start_y,
                            end_x,
                            end_y,
                            scale_alpha(theme.resize_foreground, if highlight { 1.0 } else { 0.5 }),
                        );
                    }
                }
            }
        }
    }
}

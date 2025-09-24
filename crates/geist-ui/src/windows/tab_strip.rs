use raylib::prelude::{RaylibDraw, Vector2};

use crate::text::{UiTextMeasure, UiTextRenderer};

use super::{IRect, WindowFrame, WindowTheme};

#[derive(Debug, Clone)]
pub struct TabDefinition<'a> {
    pub title: &'a str,
}

impl<'a> TabDefinition<'a> {
    pub fn new(title: &'a str) -> Self {
        Self { title }
    }
}

#[derive(Debug, Clone)]
pub struct TabSlot<'a> {
    pub index: usize,
    pub title: &'a str,
    pub bounds: IRect,
    pub text_pos: Vector2,
    pub text_width: i32,
}

impl TabSlot<'_> {
    #[inline]
    pub fn contains(&self, point: Vector2) -> bool {
        self.bounds.contains(point)
    }
}

#[derive(Debug, Clone)]
pub struct TabStripLayout<'a> {
    pub strip: IRect,
    pub content: IRect,
    pub tabs: Vec<TabSlot<'a>>,
}

impl TabStripLayout<'_> {
    #[inline]
    pub fn hovered(&self, cursor: Vector2) -> Option<usize> {
        self.tabs
            .iter()
            .find(|slot| slot.contains(cursor))
            .map(|slot| slot.index)
    }

    #[inline]
    pub fn content_rect(&self) -> IRect {
        self.content
    }
}

pub struct TabStrip;

impl TabStrip {
    pub fn layout<'a, D>(
        d: &D,
        theme: &WindowTheme,
        frame: &WindowFrame,
        tabs: &'a [TabDefinition<'a>],
    ) -> TabStripLayout<'a>
    where
        D: UiTextMeasure,
    {
        let content = frame.content;
        let mut strip_height = theme.tab_height;
        if strip_height < 0 {
            strip_height = 0;
        }
        strip_height = strip_height.min(content.h.max(0));
        let strip = IRect::new(content.x, content.y, content.w, strip_height);
        let content_y =
            (content.y + strip_height + theme.tab_content_spacing).min(content.y + content.h);
        let content_h = (content.h - strip_height - theme.tab_content_spacing).max(0);
        let adjusted_content = IRect::new(content.x, content_y, content.w, content_h);

        if tabs.is_empty() || strip.w <= 0 || strip.h <= 0 {
            return TabStripLayout {
                strip,
                content: adjusted_content,
                tabs: Vec::new(),
            };
        }

        let base_font = theme.tab_font.max(1);
        let gap_total = theme.tab_gap * (tabs.len().saturating_sub(1) as i32);
        let available_width = (strip.w - theme.tab_strip_padding * 2).max(0);

        let mut widths: Vec<i32> = tabs
            .iter()
            .map(|tab| {
                let text_w = d.ui_measure_text(tab.title, base_font);
                let padded = text_w + theme.tab_padding_x * 2;
                padded.max(theme.tab_min_width)
            })
            .collect();

        let mut desired_total = widths.iter().sum::<i32>() + gap_total;
        if desired_total > available_width && available_width > 0 {
            let base_width = widths.iter().sum::<i32>();
            if base_width > 0 {
                let scale =
                    ((available_width - gap_total).max(0) as f32 / base_width as f32).min(1.0);
                if scale < 1.0 {
                    for w in widths.iter_mut() {
                        let scaled = (*w as f32 * scale).floor() as i32;
                        *w = scaled.max(theme.tab_min_width);
                    }
                }
            }
            desired_total = widths.iter().sum::<i32>() + gap_total;
            if desired_total > available_width {
                let mut overflow = desired_total - available_width;
                while overflow > 0 {
                    let mut reduced = false;
                    for w in widths.iter_mut() {
                        if *w > theme.tab_min_width {
                            *w -= 1;
                            overflow -= 1;
                            reduced = true;
                            if overflow == 0 {
                                break;
                            }
                        }
                    }
                    if !reduced {
                        break;
                    }
                }
            }
        }

        let mut tabs_layout = Vec::with_capacity(tabs.len());
        let mut x = strip.x + theme.tab_strip_padding;
        for (index, (tab, width)) in tabs.iter().zip(widths.iter()).enumerate() {
            let clamped_width = (*width).max(theme.tab_min_width);
            let bounds = IRect::new(x, strip.y, clamped_width, strip.h);
            let text_width = d.ui_measure_text(tab.title, base_font);
            let text_x = x + (clamped_width - text_width) / 2;
            let inner_height = (strip.h - theme.tab_padding_y * 2).max(base_font);
            let mut text_y = strip.y + theme.tab_padding_y + (inner_height - base_font) / 2;
            if text_y < strip.y {
                text_y = strip.y;
            }
            tabs_layout.push(TabSlot {
                index,
                title: tab.title,
                bounds,
                text_pos: Vector2::new(text_x as f32, text_y as f32),
                text_width,
            });
            x += clamped_width + theme.tab_gap;
            if x > strip.x + strip.w {
                break;
            }
        }

        TabStripLayout {
            strip,
            content: adjusted_content,
            tabs: tabs_layout,
        }
    }

    pub fn draw<D>(
        d: &mut D,
        theme: &WindowTheme,
        layout: &TabStripLayout<'_>,
        selected: usize,
        hovered: Option<usize>,
    ) where
        D: RaylibDraw + UiTextRenderer,
    {
        if layout.strip.w <= 0 || layout.strip.h <= 0 {
            return;
        }

        d.draw_rectangle(
            layout.strip.x,
            layout.strip.y,
            layout.strip.w,
            layout.strip.h,
            theme.tab_strip_background,
        );
        d.draw_rectangle(
            layout.strip.x,
            layout.strip.y + layout.strip.h - 1,
            layout.strip.w,
            1,
            theme.tab_divider,
        );

        for slot in &layout.tabs {
            let is_selected = slot.index == selected;
            let is_hovered = hovered == Some(slot.index);
            let (bg, border, text) = if is_selected {
                (
                    theme.tab_active_background,
                    theme.tab_active_border,
                    theme.tab_text_active,
                )
            } else if is_hovered {
                (
                    theme.tab_hover_background,
                    theme.tab_hover_border,
                    theme.tab_text_active,
                )
            } else {
                (
                    theme.tab_inactive_background,
                    theme.tab_inactive_border,
                    theme.tab_text_inactive,
                )
            };

            d.draw_rectangle(
                slot.bounds.x,
                slot.bounds.y,
                slot.bounds.w,
                slot.bounds.h,
                bg,
            );
            d.draw_rectangle_lines(
                slot.bounds.x,
                slot.bounds.y,
                slot.bounds.w,
                slot.bounds.h,
                border,
            );
            d.ui_draw_text(
                slot.title,
                slot.text_pos.x as i32,
                slot.text_pos.y as i32,
                theme.tab_font,
                text,
            );
        }
    }
}

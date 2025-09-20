use std::collections::HashMap;

use raylib::prelude::{Color, RaylibDraw, RaylibDrawHandle, Vector2};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowId {
    EventHistogram,
    IntentHistogram,
    TerrainHistogram,
    Minimap,
    RenderStats,
    RuntimeStats,
    AttachmentDebug,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl IRect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    #[inline]
    pub fn contains(&self, point: Vector2) -> bool {
        point.x >= self.x as f32
            && point.x <= (self.x + self.w) as f32
            && point.y >= self.y as f32
            && point.y <= (self.y + self.h) as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitRegion {
    None,
    TitleBar,
    Resize,
    Content,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WindowFrame {
    pub outer: IRect,
    pub titlebar: IRect,
    pub content: IRect,
    pub resize: IRect,
}

#[derive(Clone, Copy, Debug)]
pub struct WindowTheme {
    pub padding_x: i32,
    pub padding_y: i32,
    pub titlebar_height: i32,
    pub resize_handle: i32,
    pub screen_padding: i32,
    pub title_font: i32,
    pub subtitle_font: i32,
    pub frame_color: Color,
    pub frame_shadow: Color,
    pub title_top: Color,
    pub title_bottom: Color,
    pub title_border: Color,
    pub body_color: Color,
    pub outline: Color,
    pub inner_outline: Color,
    pub title_text: Color,
    pub subtitle_text: Color,
    pub top_highlight: Color,
    pub resize_fill: Color,
    pub resize_outline: Color,
    pub resize_foreground: Color,
    pub title_hover_top: Color,
    pub title_hover_bottom: Color,
    pub resize_fill_hover: Color,
    pub resize_outline_hover: Color,
}

impl Default for WindowTheme {
    fn default() -> Self {
        Self {
            padding_x: 18,
            padding_y: 16,
            titlebar_height: 34,
            resize_handle: 18,
            screen_padding: 10,
            title_font: 20,
            subtitle_font: 16,
            frame_color: Color::new(18, 22, 32, 235),
            frame_shadow: Color::new(6, 10, 18, 145),
            title_top: Color::new(66, 98, 154, 240),
            title_bottom: Color::new(48, 74, 116, 235),
            title_border: Color::new(24, 32, 48, 255),
            body_color: Color::new(16, 20, 30, 228),
            outline: Color::new(96, 114, 156, 200),
            inner_outline: Color::new(28, 36, 52, 210),
            title_text: Color::new(238, 244, 255, 255),
            subtitle_text: Color::new(188, 196, 214, 255),
            top_highlight: Color::new(158, 190, 242, 210),
            resize_fill: Color::new(24, 30, 44, 220),
            resize_outline: Color::new(48, 64, 92, 230),
            resize_foreground: Color::new(140, 176, 230, 220),
            title_hover_top: Color::new(82, 132, 198, 240),
            title_hover_bottom: Color::new(60, 110, 172, 235),
            resize_fill_hover: Color::new(36, 50, 72, 230),
            resize_outline_hover: Color::new(82, 108, 150, 240),
        }
    }
}

#[derive(Debug)]
pub struct OverlayWindow {
    id: WindowId,
    position: Vector2,
    size: (i32, i32),
    min_size: (i32, i32),
    manual_size: Option<(i32, i32)>,
    dragging: bool,
    drag_offset: Vector2,
    resizing: bool,
    resize_origin: Vector2,
    resize_start: (i32, i32),
    hover_region: HitRegion,
    frame: WindowFrame,
}

impl OverlayWindow {
    pub fn new(id: WindowId, position: Vector2, size: (i32, i32), min_size: (i32, i32)) -> Self {
        Self {
            id,
            position,
            size,
            min_size,
            manual_size: None,
            dragging: false,
            drag_offset: Vector2::new(0.0, 0.0),
            resizing: false,
            resize_origin: Vector2::new(0.0, 0.0),
            resize_start: size,
            hover_region: HitRegion::None,
            frame: WindowFrame::default(),
        }
    }

    pub fn id(&self) -> WindowId {
        self.id
    }

    pub fn hover_region(&self) -> HitRegion {
        self.hover_region
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    pub fn is_resizing(&self) -> bool {
        self.resizing
    }

    pub fn set_min_size(&mut self, min_size: (i32, i32)) {
        self.min_size = min_size;
        self.size.0 = self.size.0.max(min_size.0);
        self.size.1 = self.size.1.max(min_size.1);
        if let Some((w, h)) = &mut self.manual_size {
            *w = (*w).max(min_size.0);
            *h = (*h).max(min_size.1);
        }
    }

    pub fn layout(&mut self, screen_size: (i32, i32), theme: &WindowTheme) -> WindowFrame {
        let mut width = self.manual_size.map(|(w, _)| w).unwrap_or(self.size.0);
        let mut height = self.manual_size.map(|(_, h)| h).unwrap_or(self.size.1);
        width = width.max(self.min_size.0);
        height = height.max(self.min_size.1);
        if let Some((ref mut mw, ref mut mh)) = self.manual_size {
            *mw = (*mw).max(self.min_size.0);
            *mh = (*mh).max(self.min_size.1);
            width = *mw;
            height = *mh;
        }
        self.size = (width, height);

        let mut x = self.position.x.round() as i32;
        let mut y = self.position.y.round() as i32;
        let pad = theme.screen_padding;
        let max_x = (screen_size.0 - width - pad).max(pad);
        let max_y = (screen_size.1 - height - pad).max(pad);
        x = x.clamp(pad, max_x);
        y = y.clamp(pad, max_y);
        self.position = Vector2::new(x as f32, y as f32);

        let outer = IRect::new(x, y, width, height);
        let titlebar = IRect::new(x, y, width, theme.titlebar_height.min(height));
        let resize_size = theme.resize_handle.min(width).min(height);
        let resize = IRect::new(
            x + width - resize_size,
            y + height - resize_size,
            resize_size,
            resize_size,
        );
        let content_top = y + theme.titlebar_height;
        let content = IRect::new(
            x + theme.padding_x,
            content_top + theme.padding_y,
            (width - theme.padding_x * 2).max(0),
            (height - theme.titlebar_height - theme.padding_y * 2).max(0),
        );

        self.frame = WindowFrame {
            outer,
            titlebar,
            content,
            resize,
        };

        self.frame
    }

    pub fn reset_hover(&mut self) {
        self.hover_region = HitRegion::None;
    }

    pub fn update_hover(&mut self, cursor: Vector2) {
        if self.frame.resize.contains(cursor) {
            self.hover_region = HitRegion::Resize;
        } else if self.frame.titlebar.contains(cursor) {
            self.hover_region = HitRegion::TitleBar;
        } else if self.frame.outer.contains(cursor) {
            self.hover_region = HitRegion::Content;
        } else {
            self.hover_region = HitRegion::None;
        }
    }

    pub fn begin_drag(&mut self, cursor: Vector2) {
        self.dragging = true;
        self.drag_offset = Vector2::new(
            cursor.x - self.frame.titlebar.x as f32,
            cursor.y - self.frame.titlebar.y as f32,
        );
    }

    pub fn update_drag(&mut self, cursor: Vector2, screen_size: (i32, i32), theme: &WindowTheme) {
        if !self.dragging {
            return;
        }
        let pad = theme.screen_padding as f32;
        let mut new_x = cursor.x - self.drag_offset.x;
        let mut new_y = cursor.y - self.drag_offset.y;
        let (width, height) = self.size;
        let max_x = (screen_size.0 - width - theme.screen_padding) as f32;
        let max_y = (screen_size.1 - height - theme.screen_padding) as f32;
        new_x = new_x.clamp(pad, max_x.max(pad));
        new_y = new_y.clamp(pad, max_y.max(pad));
        self.position = Vector2::new(new_x, new_y);
    }

    pub fn end_drag(&mut self) {
        self.dragging = false;
    }

    pub fn begin_resize(&mut self, cursor: Vector2) {
        self.resizing = true;
        self.resize_origin = cursor;
        self.resize_start = self.size;
    }

    pub fn update_resize(&mut self, cursor: Vector2, screen_size: (i32, i32), theme: &WindowTheme) {
        if !self.resizing {
            return;
        }
        let delta_x = cursor.x - self.resize_origin.x;
        let delta_y = cursor.y - self.resize_origin.y;
        let mut new_w = (self.resize_start.0 as f32 + delta_x).round() as i32;
        let mut new_h = (self.resize_start.1 as f32 + delta_y).round() as i32;
        new_w = new_w.max(self.min_size.0);
        new_h = new_h.max(self.min_size.1);
        let pad = theme.screen_padding;
        let max_w = (screen_size.0 - pad - self.position.x.round() as i32).max(self.min_size.0);
        let max_h = (screen_size.1 - pad - self.position.y.round() as i32).max(self.min_size.1);
        new_w = new_w.min(max_w);
        new_h = new_h.min(max_h);
        self.manual_size = Some((new_w, new_h));
        self.size = (new_w, new_h);
    }

    pub fn end_resize(&mut self) {
        self.resizing = false;
    }
}

#[derive(Default)]
pub struct OverlayWindowManager {
    windows: HashMap<WindowId, OverlayWindow>,
    order: Vec<WindowId>,
    theme: WindowTheme,
}

impl OverlayWindowManager {
    pub fn new(theme: WindowTheme) -> Self {
        Self {
            windows: HashMap::new(),
            order: Vec::new(),
            theme,
        }
    }

    pub fn theme(&self) -> &WindowTheme {
        &self.theme
    }

    pub fn insert(&mut self, window: OverlayWindow) {
        let id = window.id();
        self.windows.insert(id, window);
        if !self.order.contains(&id) {
            self.order.push(id);
        }
    }

    pub fn get(&self, id: WindowId) -> Option<&OverlayWindow> {
        self.windows.get(&id)
    }

    pub fn get_mut(&mut self, id: WindowId) -> Option<&mut OverlayWindow> {
        self.windows.get_mut(&id)
    }

    pub fn ordered_ids(&self) -> Vec<WindowId> {
        self.order.clone()
    }

    pub fn ordered_ids_rev(&self) -> Vec<WindowId> {
        self.order.iter().rev().copied().collect()
    }

    pub fn bring_to_front(&mut self, id: WindowId) {
        if let Some(pos) = self.order.iter().position(|existing| *existing == id) {
            let id = self.order.remove(pos);
            self.order.push(id);
        }
    }

    pub fn handle_hover(&mut self, cursor: Vector2) -> Option<WindowId> {
        let mut hovered = None;
        let descending = self.ordered_ids_rev();
        for id in descending {
            if let Some(window) = self.windows.get_mut(&id) {
                window.update_hover(cursor);
                if window.hover_region() != HitRegion::None {
                    hovered = Some(id);
                    break;
                }
            }
        }
        // windows not hovered still need reset
        let ascending = self.ordered_ids();
        for id in ascending {
            if Some(id) != hovered {
                if let Some(window) = self.windows.get_mut(&id) {
                    window.reset_hover();
                }
            }
        }
        hovered
    }

    pub fn clamp_all(&mut self, screen_size: (i32, i32)) {
        let ordered = self.order.clone();
        for id in ordered {
            if let Some(window) = self.windows.get_mut(&id) {
                window.layout(screen_size, &self.theme);
            }
        }
    }
}

pub struct WindowChrome;

impl WindowChrome {
    pub fn draw(
        d: &mut RaylibDrawHandle,
        theme: &WindowTheme,
        frame: &WindowFrame,
        title: &str,
        subtitle: Option<&str>,
        hover_region: Option<HitRegion>,
    ) {
        let IRect {
            x,
            y,
            w: width,
            h: height,
        } = frame.outer;
        // Drop shadow for depth
        d.draw_rectangle(x + 6, y + 8, width, height, theme.frame_shadow);

        // Window body and titlebar
        d.draw_rectangle(x, y, width, height, theme.frame_color);
        let mid = (theme.titlebar_height / 2).min(height);
        let title_top = if matches!(hover_region, Some(HitRegion::TitleBar)) {
            theme.title_hover_top
        } else {
            theme.title_top
        };
        let title_bottom = if matches!(hover_region, Some(HitRegion::TitleBar)) {
            theme.title_hover_bottom
        } else {
            theme.title_bottom
        };
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

        // Border and highlights
        d.draw_rectangle(x, y, width, 1, theme.top_highlight);
        d.draw_rectangle(
            x,
            y + theme.titlebar_height - 1,
            width,
            1,
            theme.title_border,
        );
        d.draw_rectangle_lines(x, y, width, height, theme.outline);
        if width > 2 && height > 2 {
            d.draw_rectangle_lines(x + 1, y + 1, width - 2, height - 2, theme.inner_outline);
        }

        // Title
        let title_y = y + (theme.titlebar_height - theme.title_font) / 2;
        d.draw_text(
            title,
            x + theme.padding_x,
            title_y,
            theme.title_font,
            theme.title_text,
        );

        if let Some(subtitle) = subtitle {
            let subtitle_w = d.measure_text(subtitle, theme.subtitle_font);
            let subtitle_y = title_y + theme.title_font - theme.subtitle_font - 2;
            let subtitle_x = x + width - theme.padding_x - subtitle_w;
            d.draw_text(
                subtitle,
                subtitle_x,
                subtitle_y,
                theme.subtitle_font,
                theme.subtitle_text,
            );
        }

        // Resize handle cue
        let handle = frame.resize;
        let resize_fill = if matches!(hover_region, Some(HitRegion::Resize)) {
            theme.resize_fill_hover
        } else {
            theme.resize_fill
        };
        let resize_outline = if matches!(hover_region, Some(HitRegion::Resize)) {
            theme.resize_outline_hover
        } else {
            theme.resize_outline
        };
        d.draw_rectangle(handle.x, handle.y, handle.w, handle.h, resize_fill);
        for i in 0..3 {
            let offset = i * 4;
            d.draw_line(
                handle.x + offset,
                handle.y + handle.h - 2,
                handle.x + handle.w - 2,
                handle.y + offset,
                theme.resize_foreground,
            );
        }
        d.draw_rectangle_lines(handle.x, handle.y, handle.w, handle.h, resize_outline);
    }
}

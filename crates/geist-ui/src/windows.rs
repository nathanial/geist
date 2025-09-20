use std::cmp::{max, min};
use std::collections::HashMap;

use raylib::prelude::{Color, RaylibDraw, Vector2};

use crate::text::{UiTextMeasure, UiTextRenderer};

fn blend_color(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    Color::new(
        ((a.r as f32) * inv + (b.r as f32) * t).round() as u8,
        ((a.g as f32) * inv + (b.g as f32) * t).round() as u8,
        ((a.b as f32) * inv + (b.b as f32) * t).round() as u8,
        ((a.a as f32) * inv + (b.a as f32) * t).round() as u8,
    )
}

fn scale_alpha(color: Color, factor: f32) -> Color {
    let factor = factor.clamp(0.0, 1.0);
    Color::new(
        color.r,
        color.g,
        color.b,
        ((color.a as f32) * factor).round() as u8,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowId {
    DebugTabs,
    DiagnosticsTabs,
    Minimap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowState {
    Normal,
    Minimized,
    Maximized,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowButton {
    Minimize,
    Maximize,
    Restore,
    Pin,
}

impl WindowButton {
    pub const MAX_VISIBLE: usize = 3;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeHandle {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl ResizeHandle {
    pub const ALL: [Self; 8] = [
        Self::TopLeft,
        Self::Top,
        Self::TopRight,
        Self::Right,
        Self::BottomRight,
        Self::Bottom,
        Self::BottomLeft,
        Self::Left,
    ];
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
    TitleBarButton(WindowButton),
    Resize(ResizeHandle),
    Content,
}

#[derive(Clone, Copy, Debug)]
pub struct ResizeSlot {
    pub handle: ResizeHandle,
    pub rect: IRect,
}

#[derive(Clone, Copy, Debug)]
pub struct TitleBarButtonSlot {
    pub button: WindowButton,
    pub rect: IRect,
}

#[derive(Clone, Copy, Debug)]
pub struct ScrollInfo {
    pub offset: Vector2,
    pub content_size: (i32, i32),
    pub viewport_size: (i32, i32),
}

impl Default for ScrollInfo {
    fn default() -> Self {
        Self {
            offset: Vector2::new(0.0, 0.0),
            content_size: (0, 0),
            viewport_size: (0, 0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_left_edge_updates_geometry() {
        let theme = WindowTheme::default();
        let mut window = OverlayWindow::new(
            WindowId::DebugTabs,
            Vector2::new(200.0, 200.0),
            (320, 220),
            (160, 140),
        );
        let _ = window.layout((1280, 720), &theme);
        let start = Vector2::new(200.0, 240.0);
        window.begin_resize(start, ResizeHandle::Left);
        window.update_resize(Vector2::new(160.0, 240.0), (1280, 720), &theme);
        window.end_resize();
        let frame = window.layout((1280, 720), &theme);
        assert_eq!(frame.outer.x, 160);
        assert_eq!(frame.outer.w, 360);
    }

    #[test]
    fn pinned_windows_remain_on_top() {
        let theme = WindowTheme::default();
        let mut manager = OverlayWindowManager::new(theme);
        manager.insert(OverlayWindow::new(
            WindowId::DebugTabs,
            Vector2::new(50.0, 50.0),
            (240, 200),
            (120, 120),
        ));
        manager.insert(OverlayWindow::new(
            WindowId::DiagnosticsTabs,
            Vector2::new(120.0, 80.0),
            (260, 210),
            (140, 140),
        ));
        manager.insert(OverlayWindow::new(
            WindowId::Minimap,
            Vector2::new(400.0, 120.0),
            (220, 220),
            (160, 160),
        ));

        manager.bring_to_front(WindowId::DiagnosticsTabs);
        assert_eq!(manager.focused(), Some(WindowId::DiagnosticsTabs));

        if let Some(window) = manager.get_mut(WindowId::DebugTabs) {
            window.toggle_pin();
        }
        manager.update_pin_state(WindowId::DebugTabs);

        let order = manager.ordered_ids();
        assert_eq!(order.last().copied(), Some(WindowId::DebugTabs));

        manager.bring_to_front(WindowId::DiagnosticsTabs);
        let order_after = manager.ordered_ids();
        assert_eq!(order_after.last().copied(), Some(WindowId::DebugTabs));
        assert_eq!(manager.focused(), Some(WindowId::DiagnosticsTabs));
    }

    #[test]
    fn scroll_state_clamps_to_extent() {
        let theme = WindowTheme::default();
        let mut window = OverlayWindow::new(
            WindowId::DiagnosticsTabs,
            Vector2::new(80.0, 80.0),
            (300, 240),
            (180, 160),
        );
        let frame = window.layout((1024, 768), &theme);
        window.set_content_extent((frame.content.w, frame.content.h + 600));
        assert!(window.is_scrollable());

        let scrolled = window.scroll_by(Vector2::new(0.0, 180.0));
        assert!(scrolled);

        let scrolled_more = window.scroll_by(Vector2::new(0.0, 10_000.0));
        assert!(scrolled_more);

        let info = window.frame().scroll;
        let max_offset = (info.content_size.1 - info.viewport_size.1).max(0) as f32;
        let offset = window.content_offset();
        assert!((offset.y - max_offset).abs() <= 1.0);

        let back = window.scroll_by(Vector2::new(0.0, -10_000.0));
        assert!(back);
        assert!(window.content_offset().y <= 1.0);
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WindowFrame {
    pub outer: IRect,
    pub titlebar: IRect,
    pub content: IRect,
    pub resize_handles: [Option<ResizeSlot>; 8],
    pub title_buttons: [Option<TitleBarButtonSlot>; WindowButton::MAX_VISIBLE],
    pub scroll: ScrollInfo,
}

#[derive(Clone, Copy, Debug)]
struct WindowRestoreState {
    position: Vector2,
    size: (i32, i32),
    manual_size: Option<(i32, i32)>,
    state: WindowState,
}

#[derive(Clone, Copy, Debug)]
struct ScrollState {
    content_size: (i32, i32),
    viewport_size: (i32, i32),
    offset: Vector2,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self {
            content_size: (0, 0),
            viewport_size: (0, 0),
            offset: Vector2::new(0.0, 0.0),
        }
    }
}

impl ScrollState {
    fn set_content_size(&mut self, size: (i32, i32)) {
        self.content_size = (size.0.max(0), size.1.max(0));
        self.clamp();
    }

    fn set_viewport_size(&mut self, size: (i32, i32)) {
        self.viewport_size = (size.0.max(0), size.1.max(0));
        self.clamp();
    }

    fn scroll_by(&mut self, delta: Vector2) -> bool {
        if self.viewport_size.1 <= 0 && self.viewport_size.0 <= 0 {
            return false;
        }
        let mut new_offset = Vector2::new(self.offset.x + delta.x, self.offset.y + delta.y);
        let max_x = (self.content_size.0 - self.viewport_size.0).max(0) as f32;
        let max_y = (self.content_size.1 - self.viewport_size.1).max(0) as f32;
        new_offset.x = new_offset.x.clamp(0.0, max_x);
        new_offset.y = new_offset.y.clamp(0.0, max_y);
        let changed = (new_offset.x - self.offset.x).abs() > f32::EPSILON
            || (new_offset.y - self.offset.y).abs() > f32::EPSILON;
        if changed {
            self.offset = new_offset;
        }
        changed
    }

    fn clamp(&mut self) {
        let max_x = (self.content_size.0 - self.viewport_size.0).max(0) as f32;
        let max_y = (self.content_size.1 - self.viewport_size.1).max(0) as f32;
        self.offset.x = self.offset.x.clamp(0.0, max_x);
        self.offset.y = self.offset.y.clamp(0.0, max_y);
    }

    fn info(&self) -> ScrollInfo {
        ScrollInfo {
            offset: self.offset,
            content_size: self.content_size,
            viewport_size: self.viewport_size,
        }
    }

    fn is_scrollable(&self) -> bool {
        self.content_size.1 > self.viewport_size.1 || self.content_size.0 > self.viewport_size.0
    }
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
    pub tab_height: i32,
    pub tab_padding_x: i32,
    pub tab_padding_y: i32,
    pub tab_gap: i32,
    pub tab_strip_padding: i32,
    pub tab_content_spacing: i32,
    pub tab_font: i32,
    pub tab_min_width: i32,
    pub tab_strip_background: Color,
    pub tab_active_background: Color,
    pub tab_active_border: Color,
    pub tab_inactive_background: Color,
    pub tab_inactive_border: Color,
    pub tab_hover_background: Color,
    pub tab_hover_border: Color,
    pub tab_text_active: Color,
    pub tab_text_inactive: Color,
    pub tab_divider: Color,
    pub focus_outline: Color,
    pub focus_glow: Color,
    pub pinned_outline: Color,
    pub button_normal: Color,
    pub button_hover: Color,
    pub button_active: Color,
    pub button_icon: Color,
    pub button_icon_hover: Color,
    pub title_button_spacing: i32,
    pub title_button_size: i32,
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
            tab_height: 32,
            tab_padding_x: 16,
            tab_padding_y: 6,
            tab_gap: 8,
            tab_strip_padding: 10,
            tab_content_spacing: 12,
            tab_font: 18,
            tab_min_width: 120,
            tab_strip_background: Color::new(22, 28, 40, 235),
            tab_active_background: Color::new(68, 108, 176, 240),
            tab_active_border: Color::new(28, 48, 78, 255),
            tab_inactive_background: Color::new(28, 36, 54, 228),
            tab_inactive_border: Color::new(42, 56, 78, 230),
            tab_hover_background: Color::new(52, 82, 134, 235),
            tab_hover_border: Color::new(62, 90, 138, 240),
            tab_text_active: Color::new(238, 244, 255, 255),
            tab_text_inactive: Color::new(188, 200, 220, 255),
            tab_divider: Color::new(32, 44, 66, 210),
            focus_outline: Color::new(110, 180, 255, 255),
            focus_glow: Color::new(50, 110, 200, 120),
            pinned_outline: Color::new(220, 184, 64, 240),
            button_normal: Color::new(24, 34, 52, 230),
            button_hover: Color::new(44, 68, 104, 240),
            button_active: Color::new(60, 96, 148, 255),
            button_icon: Color::new(220, 230, 245, 255),
            button_icon_hover: Color::new(244, 248, 255, 255),
            title_button_spacing: 6,
            title_button_size: 20,
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
    resize_start_pos: (i32, i32),
    active_resize: Option<ResizeHandle>,
    hover_region: HitRegion,
    frame: WindowFrame,
    state: WindowState,
    restore_stack: Vec<WindowRestoreState>,
    pinned: bool,
    scroll: ScrollState,
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
            resize_start_pos: (position.x as i32, position.y as i32),
            active_resize: None,
            hover_region: HitRegion::None,
            frame: WindowFrame::default(),
            state: WindowState::Normal,
            restore_stack: Vec::new(),
            pinned: false,
            scroll: ScrollState::default(),
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

    pub fn is_scrollable(&self) -> bool {
        self.scroll.is_scrollable()
    }

    pub fn frame(&self) -> &WindowFrame {
        &self.frame
    }

    pub fn state(&self) -> WindowState {
        self.state
    }

    pub fn is_minimized(&self) -> bool {
        self.state == WindowState::Minimized
    }

    pub fn is_maximized(&self) -> bool {
        self.state == WindowState::Maximized
    }

    pub fn is_pinned(&self) -> bool {
        self.pinned
    }

    pub fn set_pinned(&mut self, pinned: bool) {
        self.pinned = pinned;
    }

    pub fn toggle_pin(&mut self) -> bool {
        self.pinned = !self.pinned;
        self.pinned
    }

    pub fn toggle_minimize(&mut self) {
        if self.state == WindowState::Minimized {
            let restored = self.restore_from_stack();
            if !restored {
                self.state = WindowState::Normal;
            }
        } else {
            self.push_restore();
            self.state = WindowState::Minimized;
            self.dragging = false;
            self.resizing = false;
            self.active_resize = None;
        }
    }

    pub fn toggle_maximize(&mut self, screen_size: (i32, i32), theme: &WindowTheme) {
        if self.state == WindowState::Maximized {
            let restored = self.restore_from_stack();
            if !restored {
                self.state = WindowState::Normal;
            }
        } else {
            self.push_restore();
            self.state = WindowState::Maximized;
            self.dragging = false;
            self.resizing = false;
            self.active_resize = None;

            let pad = theme.screen_padding as f32;
            self.position = Vector2::new(pad, pad);
            let avail_w = (screen_size.0 - theme.screen_padding * 2).max(self.min_size.0);
            let avail_h = (screen_size.1 - theme.screen_padding * 2).max(self.min_size.1);
            self.manual_size = Some((avail_w, avail_h));
            self.size = (avail_w, avail_h);
        }
    }

    pub fn set_content_extent(&mut self, size: (i32, i32)) {
        self.scroll.set_content_size(size);
        self.frame.scroll = self.scroll.info();
    }

    pub fn scroll_by(&mut self, delta: Vector2) -> bool {
        if self.state == WindowState::Minimized {
            return false;
        }
        let changed = self.scroll.scroll_by(delta);
        if changed {
            self.frame.scroll = self.scroll.info();
        }
        changed
    }

    pub fn content_offset(&self) -> Vector2 {
        self.scroll.offset
    }

    pub fn update_content_viewport(&mut self, viewport: IRect) {
        self.scroll
            .set_viewport_size((viewport.w.max(0), viewport.h.max(0)));
        self.frame.scroll = self.scroll.info();
    }

    fn push_restore(&mut self) {
        if self
            .restore_stack
            .last()
            .map(|snapshot| snapshot.state == self.state)
            .unwrap_or(false)
        {
            return;
        }
        self.restore_stack.push(self.snapshot());
    }

    fn restore_from_stack(&mut self) -> bool {
        if let Some(snapshot) = self.restore_stack.pop() {
            self.position = snapshot.position;
            self.size = snapshot.size;
            self.manual_size = snapshot.manual_size;
            self.state = snapshot.state;
            true
        } else {
            false
        }
    }

    fn snapshot(&self) -> WindowRestoreState {
        WindowRestoreState {
            position: self.position,
            size: self.size,
            manual_size: self.manual_size,
            state: self.state,
        }
    }

    fn compute_resize_handles(
        &self,
        outer: &IRect,
        theme: &WindowTheme,
    ) -> [Option<ResizeSlot>; 8] {
        let mut slots: [Option<ResizeSlot>; 8] = [None; 8];
        if outer.w <= 0 || outer.h <= 0 {
            return slots;
        }

        let handle_size = max(theme.resize_handle, 12);
        let corner_size = min(min(handle_size, outer.w), outer.h);
        let edge_thickness_raw = max(handle_size / 2, 6);
        let edge_thickness = min(min(edge_thickness_raw, outer.w), outer.h);

        let top_width = max(outer.w - corner_size * 2, 0);
        let side_height = max(outer.h - corner_size * 2, 0);

        let mut set_slot = |index: usize, handle: ResizeHandle, rect: IRect| {
            if rect.w > 0 && rect.h > 0 {
                slots[index] = Some(ResizeSlot { handle, rect });
            }
        };

        set_slot(
            0,
            ResizeHandle::TopLeft,
            IRect::new(outer.x, outer.y, corner_size, corner_size),
        );
        set_slot(
            1,
            ResizeHandle::Top,
            IRect::new(outer.x + corner_size, outer.y, top_width, edge_thickness),
        );
        set_slot(
            2,
            ResizeHandle::TopRight,
            IRect::new(
                outer.x + outer.w - corner_size,
                outer.y,
                corner_size,
                corner_size,
            ),
        );
        set_slot(
            3,
            ResizeHandle::Right,
            IRect::new(
                outer.x + outer.w - edge_thickness,
                outer.y + corner_size,
                edge_thickness,
                side_height,
            ),
        );
        set_slot(
            4,
            ResizeHandle::BottomRight,
            IRect::new(
                outer.x + outer.w - corner_size,
                outer.y + outer.h - corner_size,
                corner_size,
                corner_size,
            ),
        );
        set_slot(
            5,
            ResizeHandle::Bottom,
            IRect::new(
                outer.x + corner_size,
                outer.y + outer.h - edge_thickness,
                top_width,
                edge_thickness,
            ),
        );
        set_slot(
            6,
            ResizeHandle::BottomLeft,
            IRect::new(
                outer.x,
                outer.y + outer.h - corner_size,
                corner_size,
                corner_size,
            ),
        );
        set_slot(
            7,
            ResizeHandle::Left,
            IRect::new(outer.x, outer.y + corner_size, edge_thickness, side_height),
        );

        slots
    }

    fn compute_title_buttons(
        &self,
        outer: &IRect,
        titlebar_height: i32,
        theme: &WindowTheme,
    ) -> [Option<TitleBarButtonSlot>; WindowButton::MAX_VISIBLE] {
        let mut slots: [Option<TitleBarButtonSlot>; WindowButton::MAX_VISIBLE] =
            [None; WindowButton::MAX_VISIBLE];
        if titlebar_height <= 0 || outer.w <= 0 {
            return slots;
        }

        let max_size = max(titlebar_height - 6, 12);
        let button_size = min(theme.title_button_size.max(12), max_size);
        if button_size <= 0 {
            return slots;
        }
        let spacing = max(theme.title_button_spacing, 4);

        let mut cursor_x = outer.x + outer.w - theme.padding_x - button_size;
        let button_y = outer.y + max((titlebar_height - button_size) / 2, 0);

        let sequence = [
            WindowButton::Minimize,
            if self.state == WindowState::Maximized {
                WindowButton::Restore
            } else {
                WindowButton::Maximize
            },
            WindowButton::Pin,
        ];

        for (index, button) in sequence.into_iter().enumerate() {
            if index >= WindowButton::MAX_VISIBLE {
                break;
            }

            if cursor_x < outer.x + theme.padding_x - button_size {
                break;
            }

            let rect = IRect::new(cursor_x, button_y, button_size, button_size);
            if rect.w > 0 && rect.h > 0 {
                slots[index] = Some(TitleBarButtonSlot { button, rect });
            }

            cursor_x -= button_size + spacing;
        }

        slots
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
        let pad = theme.screen_padding;
        let (screen_w, screen_h) = screen_size;

        let mut width = self.manual_size.map(|(w, _)| w).unwrap_or(self.size.0);
        let mut height = self.manual_size.map(|(_, h)| h).unwrap_or(self.size.1);

        width = max(width, self.min_size.0);
        height = max(height, self.min_size.1);

        if let Some((ref mut mw, ref mut mh)) = self.manual_size {
            *mw = max(*mw, self.min_size.0);
            *mh = max(*mh, self.min_size.1);
            width = *mw;
            height = *mh;
        }

        match self.state {
            WindowState::Maximized => {
                width = max(screen_w - pad * 2, self.min_size.0);
                height = max(screen_h - pad * 2, self.min_size.1);
                self.manual_size = Some((width, height));
                self.position = Vector2::new(pad as f32, pad as f32);
            }
            WindowState::Minimized => {
                let min_height = max(
                    theme.titlebar_height + theme.padding_y * 2,
                    theme.titlebar_height,
                );
                height = min_height.min(max(screen_h - pad * 2, theme.titlebar_height));
            }
            WindowState::Normal => {
                // position clamped after match
            }
        }

        let mut x = self.position.x.round() as i32;
        let mut y = self.position.y.round() as i32;

        if self.state != WindowState::Maximized {
            let max_x = max(screen_w - width - pad, pad);
            let max_y = max(screen_h - height - pad, pad);
            x = x.clamp(pad, max_x);
            y = y.clamp(pad, max_y);
            self.position = Vector2::new(x as f32, y as f32);
        } else {
            x = pad;
            y = pad;
        }

        self.size = (width, height);

        let outer = IRect::new(x, y, width, height);
        let titlebar_height = min(theme.titlebar_height, height);
        let titlebar = IRect::new(x, y, width, titlebar_height);

        let content_top = min(y + titlebar_height + theme.padding_y, y + height);
        let content_height = max(height - titlebar_height - theme.padding_y * 2, 0);
        let content = IRect::new(
            x + theme.padding_x,
            content_top,
            max(width - theme.padding_x * 2, 0),
            content_height,
        );

        self.scroll.set_viewport_size((content.w, content.h));

        let resize_handles = if self.state == WindowState::Normal {
            self.compute_resize_handles(&outer, theme)
        } else {
            [None; 8]
        };

        let title_buttons = self.compute_title_buttons(&outer, titlebar_height, theme);

        self.frame = WindowFrame {
            outer,
            titlebar,
            content,
            resize_handles,
            title_buttons,
            scroll: self.scroll.info(),
        };

        self.frame
    }

    pub fn reset_hover(&mut self) {
        self.hover_region = HitRegion::None;
    }

    pub fn update_hover(&mut self, cursor: Vector2) {
        self.hover_region = HitRegion::None;

        if self.state == WindowState::Normal {
            if let Some(slot) = self
                .frame
                .resize_handles
                .iter()
                .flatten()
                .find(|slot| slot.rect.contains(cursor))
            {
                self.hover_region = HitRegion::Resize(slot.handle);
                return;
            }
        }

        if let Some(slot) = self
            .frame
            .title_buttons
            .iter()
            .flatten()
            .find(|slot| slot.rect.contains(cursor))
        {
            self.hover_region = HitRegion::TitleBarButton(slot.button);
            return;
        }

        if self.frame.titlebar.contains(cursor) {
            self.hover_region = HitRegion::TitleBar;
        } else if self.frame.outer.contains(cursor) {
            self.hover_region = HitRegion::Content;
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

    pub fn begin_resize(&mut self, cursor: Vector2, handle: ResizeHandle) {
        if self.state != WindowState::Normal {
            return;
        }
        self.resizing = true;
        self.resize_origin = cursor;
        self.resize_start = self.size;
        self.resize_start_pos = (
            self.position.x.round() as i32,
            self.position.y.round() as i32,
        );
        self.active_resize = Some(handle);
    }

    pub fn update_resize(&mut self, cursor: Vector2, screen_size: (i32, i32), theme: &WindowTheme) {
        if !self.resizing || self.state != WindowState::Normal {
            return;
        }
        let handle = match self.active_resize {
            Some(handle) => handle,
            None => return,
        };

        let delta_x = cursor.x - self.resize_origin.x;
        let delta_y = cursor.y - self.resize_origin.y;

        let mut new_x = self.resize_start_pos.0;
        let mut new_y = self.resize_start_pos.1;
        let mut new_w = self.resize_start.0;
        let mut new_h = self.resize_start.1;

        let right_edge = self.resize_start_pos.0 + self.resize_start.0;
        let bottom_edge = self.resize_start_pos.1 + self.resize_start.1;

        match handle {
            ResizeHandle::Left | ResizeHandle::TopLeft | ResizeHandle::BottomLeft => {
                new_x = (self.resize_start_pos.0 as f32 + delta_x).round() as i32;
                new_w = right_edge - new_x;
            }
            ResizeHandle::Right | ResizeHandle::TopRight | ResizeHandle::BottomRight => {
                new_w = (self.resize_start.0 as f32 + delta_x).round() as i32;
            }
            _ => {}
        }

        match handle {
            ResizeHandle::Top | ResizeHandle::TopLeft | ResizeHandle::TopRight => {
                new_y = (self.resize_start_pos.1 as f32 + delta_y).round() as i32;
                new_h = bottom_edge - new_y;
            }
            ResizeHandle::Bottom | ResizeHandle::BottomLeft | ResizeHandle::BottomRight => {
                new_h = (self.resize_start.1 as f32 + delta_y).round() as i32;
            }
            _ => {}
        }

        if new_w < self.min_size.0 {
            new_w = self.min_size.0;
            if matches!(
                handle,
                ResizeHandle::Left | ResizeHandle::TopLeft | ResizeHandle::BottomLeft
            ) {
                new_x = right_edge - new_w;
            }
        }

        if new_h < self.min_size.1 {
            new_h = self.min_size.1;
            if matches!(
                handle,
                ResizeHandle::Top | ResizeHandle::TopLeft | ResizeHandle::TopRight
            ) {
                new_y = bottom_edge - new_h;
            }
        }

        let pad = theme.screen_padding;

        if matches!(
            handle,
            ResizeHandle::Left | ResizeHandle::TopLeft | ResizeHandle::BottomLeft
        ) && new_x < pad
        {
            new_x = pad;
            new_w = right_edge - new_x;
            if new_w < self.min_size.0 {
                new_w = self.min_size.0;
                new_x = right_edge - new_w;
            }
        }

        if matches!(
            handle,
            ResizeHandle::Top | ResizeHandle::TopLeft | ResizeHandle::TopRight
        ) && new_y < pad
        {
            new_y = pad;
            new_h = bottom_edge - new_y;
            if new_h < self.min_size.1 {
                new_h = self.min_size.1;
                new_y = bottom_edge - new_h;
            }
        }

        let max_width = max(screen_size.0 - pad - new_x, self.min_size.0);
        new_w = min(new_w, max_width);
        let max_height = max(screen_size.1 - pad - new_y, self.min_size.1);
        new_h = min(new_h, max_height);

        self.position = Vector2::new(new_x as f32, new_y as f32);
        self.manual_size = Some((new_w, new_h));
        self.size = (new_w, new_h);
    }

    pub fn end_resize(&mut self) {
        self.resizing = false;
        self.active_resize = None;
    }
}

#[derive(Default)]
pub struct OverlayWindowManager {
    windows: HashMap<WindowId, OverlayWindow>,
    order: Vec<WindowId>,
    theme: WindowTheme,
    focus_stack: Vec<WindowId>,
}

impl OverlayWindowManager {
    pub fn new(theme: WindowTheme) -> Self {
        Self {
            windows: HashMap::new(),
            order: Vec::new(),
            theme,
            focus_stack: Vec::new(),
        }
    }

    pub fn theme(&self) -> &WindowTheme {
        &self.theme
    }

    pub fn insert(&mut self, window: OverlayWindow) {
        let id = window.id();
        let pinned = window.is_pinned();
        self.windows.insert(id, window);
        if !self.order.contains(&id) {
            let insert_idx = if pinned {
                self.order.len()
            } else {
                self.first_pinned_index().unwrap_or(self.order.len())
            };
            self.order.insert(insert_idx, id);
        }
        self.focus(id);
    }

    pub fn get(&self, id: WindowId) -> Option<&OverlayWindow> {
        self.windows.get(&id)
    }

    pub fn get_mut(&mut self, id: WindowId) -> Option<&mut OverlayWindow> {
        self.windows.get_mut(&id)
    }

    pub fn ordered_ids(&self) -> Vec<WindowId> {
        let mut normal = Vec::new();
        let mut pinned = Vec::new();
        for id in &self.order {
            if let Some(window) = self.windows.get(id) {
                if window.is_pinned() {
                    pinned.push(*id);
                } else {
                    normal.push(*id);
                }
            }
        }
        normal.extend(pinned);
        normal
    }

    pub fn ordered_ids_rev(&self) -> Vec<WindowId> {
        let mut ordered = self.ordered_ids();
        ordered.reverse();
        ordered
    }

    pub fn bring_to_front(&mut self, id: WindowId) {
        if let Some(pos) = self.order.iter().position(|existing| *existing == id) {
            let entry = self.order.remove(pos);
            let pinned = self
                .windows
                .get(&entry)
                .map(|w| w.is_pinned())
                .unwrap_or(false);
            if pinned {
                self.order.push(entry);
            } else {
                let insert_idx = self.first_pinned_index().unwrap_or(self.order.len());
                self.order.insert(insert_idx, entry);
            }
        }
        self.focus(id);
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

    pub fn focus(&mut self, id: WindowId) {
        self.focus_stack.retain(|existing| *existing != id);
        self.focus_stack.push(id);
    }

    pub fn clear_focus(&mut self) {
        self.focus_stack.clear();
    }

    pub fn is_focused(&self, id: WindowId) -> bool {
        self.focus_stack.last().copied() == Some(id)
    }

    pub fn focused(&self) -> Option<WindowId> {
        self.focus_stack.last().copied()
    }

    pub fn update_pin_state(&mut self, id: WindowId) {
        if let Some(pos) = self.order.iter().position(|existing| *existing == id) {
            let entry = self.order.remove(pos);
            let pinned = self
                .windows
                .get(&entry)
                .map(|w| w.is_pinned())
                .unwrap_or(false);
            if pinned {
                self.order.push(entry);
            } else {
                let insert_idx = self.first_pinned_index().unwrap_or(self.order.len());
                self.order.insert(insert_idx, entry);
            }
        }
    }

    fn first_pinned_index(&self) -> Option<usize> {
        self.order.iter().position(|id| {
            self.windows
                .get(id)
                .map(|window| window.is_pinned())
                .unwrap_or(false)
        })
    }
}

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
        let IRect {
            x,
            y,
            w: width,
            h: height,
        } = frame.outer;

        let hovered_title = matches!(hover_region, Some(HitRegion::TitleBar));
        let hovered_button = match hover_region {
            Some(HitRegion::TitleBarButton(button)) => Some(button),
            _ => None,
        };
        let hovered_handle = match hover_region {
            Some(HitRegion::Resize(handle)) => Some(handle),
            _ => None,
        };

        if is_focused {
            d.draw_rectangle_lines(x - 1, y - 1, width + 2, height + 2, theme.focus_glow);
        }

        // Drop shadow for depth
        d.draw_rectangle(x + 6, y + 8, width, height, theme.frame_shadow);

        // Window body and titlebar
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

        // Border and highlights
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

        // Title
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

        // Title bar buttons
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
                    let y_line = slot.rect.y + slot.rect.h - max(slot.rect.h / 4, 4);
                    d.draw_line(
                        slot.rect.x + 4,
                        y_line,
                        slot.rect.x + slot.rect.w - 4,
                        y_line,
                        icon_color,
                    );
                }
                WindowButton::Maximize => {
                    let size = max(slot.rect.w.min(slot.rect.h) - 8, 6);
                    let offset_x = slot.rect.x + (slot.rect.w - size) / 2;
                    let offset_y = slot.rect.y + (slot.rect.h - size) / 2;
                    d.draw_rectangle_lines(offset_x, offset_y, size, size, icon_color);
                }
                WindowButton::Restore => {
                    let size = max(slot.rect.w.min(slot.rect.h) - 10, 6);
                    let inner = max(size - 2, 4);
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

                if matches!(
                    slot.handle,
                    ResizeHandle::BottomRight | ResizeHandle::TopRight
                ) || matches!(
                    slot.handle,
                    ResizeHandle::BottomLeft | ResizeHandle::TopLeft
                ) {
                    let step = max(slot.rect.w.min(slot.rect.h) / 4, 3);
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

use std::cmp::{max, min};

use raylib::prelude::Vector2;

use super::{HitRegion, IRect, ResizeHandle, WindowButton, WindowId, WindowState, WindowTheme};

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

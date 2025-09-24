use std::collections::HashMap;

use raylib::prelude::Vector2;

use super::{HitRegion, OverlayWindow, WindowId, WindowTheme};

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

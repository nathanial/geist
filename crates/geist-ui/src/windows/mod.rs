mod chrome;
mod manager;
mod overlay;
mod tab_strip;
mod theme;
mod types;
mod util;

pub use chrome::WindowChrome;
pub use manager::OverlayWindowManager;
pub use overlay::{OverlayWindow, ResizeSlot, ScrollInfo, TitleBarButtonSlot, WindowFrame};
pub use tab_strip::{TabDefinition, TabSlot, TabStrip, TabStripLayout};
pub use theme::WindowTheme;
pub use types::{HitRegion, IRect, ResizeHandle, WindowButton, WindowId, WindowState};

#[cfg(test)]
mod tests {
    use raylib::prelude::Vector2;

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

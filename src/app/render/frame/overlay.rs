use raylib::prelude::*;

use super::super::{
    App, AttachmentDebugView, ChunkVoxelView, ContentLayout, DebugOverlayTab, DiagnosticsTab,
    EventHistogramView, GeistDraw, HitRegion, IRect, IntentHistogramView, MINIMAP_BORDER_PX,
    MINIMAP_MAX_CONTENT_SIDE, MINIMAP_MIN_CONTENT_SIDE, RenderStatsView, RuntimeStatsView,
    TabDefinition, TabStrip, TerrainHistogramView, WindowChrome, WindowFrame, WindowId,
    WindowTheme,
};

impl App {
    pub(super) fn prepare_minimap_render_side(
        &mut self,
        screen_dims: (i32, i32),
        overlay_theme: WindowTheme,
    ) -> i32 {
        if !self.gs.show_debug_overlay {
            return 0;
        }

        let mut minimap_render_side = 0;
        if let Some(window) = self.overlay_windows.get_mut(WindowId::Minimap) {
            let minimap_min_size = (
                overlay_theme.padding_x * 2 + MINIMAP_MIN_CONTENT_SIDE,
                overlay_theme.titlebar_height
                    + overlay_theme.padding_y * 2
                    + MINIMAP_MIN_CONTENT_SIDE,
            );
            window.set_min_size(minimap_min_size);
            let frame = window.layout(screen_dims, &overlay_theme);
            let content = frame.content;
            let available_side = content.w.min(content.h).max(0);
            let outer_side = available_side.min(MINIMAP_MAX_CONTENT_SIDE + MINIMAP_BORDER_PX * 2);
            minimap_render_side = (outer_side - MINIMAP_BORDER_PX * 2).max(0);
        }

        minimap_render_side
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_debug_overlay(
        &mut self,
        d: &mut GeistDraw,
        screen_dims: (i32, i32),
        overlay_theme: WindowTheme,
        cursor_position: Vector2,
        mouse_left_pressed: bool,
    ) {
        if !self.gs.show_debug_overlay {
            self.minimap_ui_rect = None;
            return;
        }

        let fps = d.get_fps();
        let ordered_ids = self.overlay_windows.ordered_ids();
        let mut minimap_drawn = false;

        for id in ordered_ids {
            let hover = self
                .overlay_hover
                .as_ref()
                .and_then(|(hid, region)| (*hid == id).then_some(*region));

            match id {
                WindowId::DiagnosticsTabs => {
                    let is_focused = self.overlay_windows.is_focused(id);
                    let frame_view = RenderStatsView::new(self, fps);
                    let runtime_view = RuntimeStatsView::new(self);
                    let attachment_view = AttachmentDebugView::new(self);

                    if let Some(window) = self.overlay_windows.get_mut(id) {
                        let frame_min = frame_view.min_size(&overlay_theme);
                        let runtime_min = runtime_view.min_size(&overlay_theme);
                        let attachment_min = attachment_view.min_size(&overlay_theme);
                        let min_width = frame_min.0.max(runtime_min.0).max(attachment_min.0);
                        let tab_extra =
                            overlay_theme.tab_height + overlay_theme.tab_content_spacing;
                        let min_height =
                            frame_min.1.max(runtime_min.1).max(attachment_min.1) + tab_extra;
                        window.set_min_size((min_width, min_height));
                        let frame = window.layout(screen_dims, &overlay_theme);

                        let tab_definitions = [
                            TabDefinition::new(DiagnosticsTab::FrameStats.title()),
                            TabDefinition::new(DiagnosticsTab::RuntimeStats.title()),
                            TabDefinition::new(DiagnosticsTab::AttachmentDebug.title()),
                        ];
                        let tab_layout =
                            TabStrip::layout(&*d, &overlay_theme, &frame, &tab_definitions);
                        let hovered_tab = tab_layout.hovered(cursor_position);
                        if mouse_left_pressed
                            && hovered_tab.is_some()
                            && matches!(hover, Some(HitRegion::Content))
                            && !window.is_dragging()
                            && !window.is_resizing()
                        {
                            if let Some(index) = hovered_tab {
                                let next_tab = DiagnosticsTab::from_index(index);
                                if next_tab != self.overlay_diagnostics_tab {
                                    self.overlay_diagnostics_tab = next_tab;
                                }
                            }
                        }

                        let selected_tab = self.overlay_diagnostics_tab;
                        let selected_index = selected_tab.as_index();

                        let frame_subtitle = frame_view.subtitle();
                        let runtime_subtitle = runtime_view.subtitle();
                        let attachment_subtitle: Option<&str> = None;
                        let subtitle = match selected_tab {
                            DiagnosticsTab::FrameStats => frame_subtitle,
                            DiagnosticsTab::RuntimeStats => runtime_subtitle,
                            DiagnosticsTab::AttachmentDebug => attachment_subtitle,
                        };

                        let window_state = window.state();
                        let is_pinned = window.is_pinned();

                        WindowChrome::draw(
                            d,
                            &overlay_theme,
                            &frame,
                            "Diagnostics",
                            subtitle,
                            hover,
                            window_state,
                            is_focused,
                            is_pinned,
                        );

                        TabStrip::draw(d, &overlay_theme, &tab_layout, selected_index, hovered_tab);

                        let tab_content_area = tab_layout.content_rect();
                        window.update_content_viewport(tab_content_area);
                        let mut tab_content_frame = *window.frame();
                        tab_content_frame.content = tab_content_area;

                        let layout = match selected_tab {
                            DiagnosticsTab::FrameStats => frame_view.draw(d, &tab_content_frame),
                            DiagnosticsTab::RuntimeStats => {
                                runtime_view.draw(d, &tab_content_frame)
                            }
                            DiagnosticsTab::AttachmentDebug => {
                                attachment_view.draw(d, &tab_content_frame)
                            }
                        };

                        window
                            .set_content_extent((tab_content_frame.content.w, layout.used_height));

                        self.draw_overflow_hint(d, &tab_content_frame, layout);
                    }
                }
                WindowId::DebugTabs => {
                    let is_focused = self.overlay_windows.is_focused(id);
                    if let Some(window) = self.overlay_windows.get_mut(id) {
                        let event_view = EventHistogramView::new(&self.debug_stats);
                        let intent_view = IntentHistogramView::new(&self.debug_stats);
                        let terrain_view = TerrainHistogramView::new(
                            &self.terrain_stage_us,
                            &self.terrain_stage_calls,
                            &self.terrain_height_tile_us,
                            &self.terrain_height_tile_reused,
                            &self.terrain_cache_hits,
                            &self.terrain_cache_misses,
                            &self.terrain_tile_cache_hits,
                            &self.terrain_tile_cache_misses,
                            &self.terrain_tile_cache_evictions,
                            &self.terrain_tile_cache_entries,
                            &self.terrain_chunk_total_us,
                            &self.terrain_chunk_fill_us,
                            &self.terrain_chunk_feature_us,
                        );

                        let event_min = event_view.min_size(&overlay_theme);
                        let intent_min = intent_view.min_size(&overlay_theme);
                        let terrain_min = terrain_view.min_size(&overlay_theme);
                        let min_width = event_min.0.max(intent_min.0).max(terrain_min.0);
                        let tab_extra =
                            overlay_theme.tab_height + overlay_theme.tab_content_spacing;
                        let min_height =
                            event_min.1.max(intent_min.1).max(terrain_min.1) + tab_extra;
                        window.set_min_size((min_width, min_height));
                        let frame = window.layout(screen_dims, &overlay_theme);

                        let tab_definitions = [
                            TabDefinition::new(DebugOverlayTab::EventQueue.title()),
                            TabDefinition::new(DebugOverlayTab::IntentQueue.title()),
                            TabDefinition::new(DebugOverlayTab::TerrainPipeline.title()),
                        ];
                        let tab_layout =
                            TabStrip::layout(&*d, &overlay_theme, &frame, &tab_definitions);
                        let hovered_tab = tab_layout.hovered(cursor_position);
                        if mouse_left_pressed
                            && hovered_tab.is_some()
                            && matches!(hover, Some(HitRegion::Content))
                            && !window.is_dragging()
                            && !window.is_resizing()
                        {
                            if let Some(index) = hovered_tab {
                                let next_tab = DebugOverlayTab::from_index(index);
                                if next_tab != self.overlay_debug_tab {
                                    self.overlay_debug_tab = next_tab;
                                }
                            }
                        }

                        let selected_tab = self.overlay_debug_tab;
                        let selected_index = selected_tab.as_index();

                        let event_subtitle = event_view.subtitle();
                        let intent_subtitle = intent_view.subtitle();
                        let terrain_subtitle = terrain_view.subtitle();
                        let subtitle = match selected_tab {
                            DebugOverlayTab::EventQueue => event_subtitle.as_deref(),
                            DebugOverlayTab::IntentQueue => intent_subtitle.as_deref(),
                            DebugOverlayTab::TerrainPipeline => terrain_subtitle.as_deref(),
                        };

                        let window_state = window.state();
                        let is_pinned = window.is_pinned();

                        WindowChrome::draw(
                            d,
                            &overlay_theme,
                            &frame,
                            "Queues & Pipelines",
                            subtitle,
                            hover,
                            window_state,
                            is_focused,
                            is_pinned,
                        );

                        TabStrip::draw(d, &overlay_theme, &tab_layout, selected_index, hovered_tab);

                        let tab_content_area = tab_layout.content_rect();
                        window.update_content_viewport(tab_content_area);
                        let mut tab_content_frame = *window.frame();
                        tab_content_frame.content = tab_content_area;

                        let maybe_layout = match selected_tab {
                            DebugOverlayTab::EventQueue => {
                                let layout = event_view.draw(d, &tab_content_frame, &overlay_theme);
                                Some(layout)
                            }
                            DebugOverlayTab::IntentQueue => {
                                let layout =
                                    intent_view.draw(d, &tab_content_frame, &overlay_theme);
                                Some(layout)
                            }
                            DebugOverlayTab::TerrainPipeline => {
                                terrain_view.draw(d, &tab_content_frame, &overlay_theme)
                            }
                        };

                        if let Some(layout) = maybe_layout {
                            window.set_content_extent((
                                tab_content_frame.content.w,
                                layout.used_height,
                            ));
                            self.draw_overflow_hint(d, &tab_content_frame, layout);
                        } else {
                            window.set_content_extent((
                                tab_content_frame.content.w,
                                tab_content_frame.content.h,
                            ));
                        }
                    }
                }
                WindowId::ChunkVoxels => {
                    let is_focused = self.overlay_windows.is_focused(id);
                    let view = ChunkVoxelView::new(self);
                    if let Some(window) = self.overlay_windows.get_mut(id) {
                        window.set_min_size(view.min_size(&overlay_theme));
                        let frame = window.layout(screen_dims, &overlay_theme);
                        let window_state = window.state();
                        let is_pinned = window.is_pinned();

                        WindowChrome::draw(
                            d,
                            &overlay_theme,
                            &frame,
                            "Chunk Voxels",
                            view.subtitle(),
                            hover,
                            window_state,
                            is_focused,
                            is_pinned,
                        );

                        let content = frame.content;
                        window.update_content_viewport(content);
                        let mut content_frame = *window.frame();
                        content_frame.content = content;
                        let layout = view.draw(d, &content_frame);
                        window.set_content_extent((content_frame.content.w, layout.used_height));
                        self.draw_overflow_hint(d, &content_frame, layout);
                    }
                }
                WindowId::Minimap => {
                    minimap_drawn = true;
                    let is_focused = self.overlay_windows.is_focused(id);
                    if let Some(window) = self.overlay_windows.get_mut(id) {
                        let minimap_min_size = (
                            overlay_theme.padding_x * 2 + MINIMAP_MIN_CONTENT_SIDE,
                            overlay_theme.titlebar_height
                                + overlay_theme.padding_y * 2
                                + MINIMAP_MIN_CONTENT_SIDE,
                        );
                        window.set_min_size(minimap_min_size);
                        let frame = window.layout(screen_dims, &overlay_theme);
                        let subtitle = Some(format!(
                            "radius {} chunks",
                            self.gs.view_radius_chunks.max(0)
                        ));

                        let window_state = window.state();
                        let is_pinned = window.is_pinned();
                        WindowChrome::draw(
                            d,
                            &overlay_theme,
                            &frame,
                            "Minimap",
                            subtitle.as_deref(),
                            hover,
                            window_state,
                            is_focused,
                            is_pinned,
                        );

                        window.set_content_extent((frame.content.w, frame.content.h));

                        let content = frame.content;
                        let available_side = content.w.min(content.h).max(0);
                        let outer_side =
                            available_side.min(MINIMAP_MAX_CONTENT_SIDE + MINIMAP_BORDER_PX * 2);
                        let map_side = (outer_side - MINIMAP_BORDER_PX * 2).max(0);

                        if map_side > 0 {
                            let frame_rect = IRect::new(
                                content.x + (content.w - outer_side) / 2,
                                content.y + (content.h - outer_side) / 2,
                                outer_side,
                                outer_side,
                            );
                            let map_rect = IRect::new(
                                frame_rect.x + MINIMAP_BORDER_PX,
                                frame_rect.y + MINIMAP_BORDER_PX,
                                map_side,
                                map_side,
                            );
                            d.draw_rectangle(
                                frame_rect.x,
                                frame_rect.y,
                                frame_rect.w,
                                frame_rect.h,
                                Color::new(12, 18, 28, 210),
                            );
                            d.draw_rectangle_lines(
                                frame_rect.x,
                                frame_rect.y,
                                frame_rect.w,
                                frame_rect.h,
                                Color::new(86, 108, 152, 210),
                            );

                            if let Some(ref minimap_rt) = self.minimap_rt {
                                let tex = minimap_rt.texture().clone();
                                let src = Rectangle::new(
                                    0.0,
                                    0.0,
                                    tex.width() as f32,
                                    -(tex.height() as f32),
                                );
                                let dest = Rectangle::new(
                                    map_rect.x as f32,
                                    map_rect.y as f32,
                                    map_rect.w as f32,
                                    map_rect.h as f32,
                                );
                                d.draw_texture_pro(
                                    tex,
                                    src,
                                    dest,
                                    Vector2::new(0.0, 0.0),
                                    0.0,
                                    Color::WHITE,
                                );
                                self.minimap_ui_rect =
                                    Some((frame_rect.x, frame_rect.y, frame_rect.w, frame_rect.h));

                                let label = format!("Loaded {} chunks", self.gs.chunks.ready_len());
                                let label_fs = 18;
                                let label_x = map_rect.x + 14;
                                let label_y = map_rect.y + 14;
                                d.draw_text(
                                    &label,
                                    label_x + 1,
                                    label_y + 1,
                                    label_fs,
                                    Color::new(0, 0, 0, 220),
                                );
                                d.draw_text(&label, label_x, label_y, label_fs, Color::WHITE);

                                let legend =
                                    ["Scroll: zoom", "LMB drag: orbit", "Shift+Drag/RMB: pan"];
                                let legend_fs = 14;
                                let legend_total_h = (legend.len() as i32) * (legend_fs + 2);
                                let mut legend_y = map_rect.y + map_rect.h - legend_total_h - 12;
                                let legend_min_y = map_rect.y + 14;
                                if legend_y < legend_min_y {
                                    legend_y = legend_min_y;
                                }
                                for line in legend.iter() {
                                    d.draw_text(
                                        line,
                                        map_rect.x + 14 + 1,
                                        legend_y + 1,
                                        legend_fs,
                                        Color::new(0, 0, 0, 200),
                                    );
                                    d.draw_text(
                                        line,
                                        map_rect.x + 14,
                                        legend_y,
                                        legend_fs,
                                        Color::new(220, 220, 240, 240),
                                    );
                                    legend_y += legend_fs + 2;
                                }
                            } else {
                                self.minimap_ui_rect = None;
                                d.draw_rectangle(
                                    map_rect.x,
                                    map_rect.y,
                                    map_rect.w,
                                    map_rect.h,
                                    Color::new(18, 24, 34, 220),
                                );
                                let msg = "Minimap unavailable";
                                let msg_fs = 18;
                                let msg_w = d.measure_text(msg, msg_fs);
                                let msg_x = map_rect.x + (map_rect.w - msg_w) / 2;
                                let msg_y = map_rect.y + (map_rect.h - msg_fs) / 2;
                                d.draw_text(
                                    msg,
                                    msg_x + 1,
                                    msg_y + 1,
                                    msg_fs,
                                    Color::new(0, 0, 0, 220),
                                );
                                d.draw_text(
                                    msg,
                                    msg_x,
                                    msg_y,
                                    msg_fs,
                                    Color::new(220, 220, 240, 240),
                                );
                            }
                        } else {
                            self.minimap_ui_rect = None;
                            let msg = "Expand the window to view the minimap";
                            let msg_fs = 16;
                            let msg_w = d.measure_text(msg, msg_fs);
                            let msg_x = content.x + (content.w - msg_w) / 2;
                            let msg_y = content.y + (content.h - msg_fs) / 2;
                            d.draw_text(
                                msg,
                                msg_x + 1,
                                msg_y + 1,
                                msg_fs,
                                Color::new(0, 0, 0, 180),
                            );
                            d.draw_text(msg, msg_x, msg_y, msg_fs, Color::new(218, 228, 248, 230));
                        }
                    }
                }
            }
        }

        if !minimap_drawn {
            self.minimap_ui_rect = None;
        }
    }

    pub(super) fn draw_overflow_hint(
        &self,
        d: &mut GeistDraw,
        frame: &WindowFrame,
        layout: ContentLayout,
    ) {
        if !layout.overflow() {
            return;
        }
        if frame.scroll.content_size.1 > frame.scroll.viewport_size.1 {
            return;
        }
        let font_size = 14;
        let text = if layout.overflow_items > 0 {
            format!("⋯ {} more", layout.overflow_items)
        } else {
            "⋯".to_string()
        };
        let content = frame.content;
        if content.w <= 0 || content.h <= 0 {
            return;
        }
        let text_w = d.measure_text(&text, font_size);
        let pad = 6;
        let box_w = text_w + pad * 2;
        let box_h = font_size + pad * 2;
        let x = content.x + content.w - box_w;
        let y = content.y + content.h - box_h;
        d.draw_rectangle(x, y, box_w, box_h, Color::new(12, 18, 28, 210));
        d.draw_rectangle_lines(x, y, box_w, box_h, Color::new(48, 64, 92, 220));
        d.draw_text(
            &text,
            x + pad,
            y + pad,
            font_size,
            Color::new(224, 234, 252, 255),
        );
    }
}

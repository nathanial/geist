# Window Overlay Refactoring Plan

## Goals
- Reduce repeated logic across event/intents/terrain histogram windows.
- Centralize styling so future overlays share a consistent look-and-feel.
- Improve testability and readability for input handling and layout math.

## Architectural Ideas
- Introduce a lightweight `OverlayWindow` struct (position, size, min size, drag+resize state) with helper methods for hit-testing titlebars/handles.
- Build a `WindowChrome` renderer that accepts a `WindowTheme` palette, draws background/border/handle, and returns interior bounds for content.
- Extract histogram-specific layout into dedicated view structs (e.g., `EventHistogramView`) that render within a provided content rectangle.
- Consider a small window manager on `App` that stores a `HashMap<WindowId, OverlayWindow>` and routes shared update/render calls.

## Input Handling
- Move shared drag/resize logic into methods on `OverlayWindow` so `step.rs` loops over windows instead of repeating blocks per window.
- Add hover state tracking for handles/titlebars to support cursor feedback and potential keyboard shortcuts (e.g., snap to edges).
- Provide a config hook to constrain windows to safe bounds (padding, DPI scaling) via a reusable clamp helper.

## Rendering and Layout
- Normalize padding, titlebar heights, and fonts through constants in a `WindowTheme` to keep windows visually aligned.
- Expose a common `ContentLayout` response struct that reports scrollable height, remainder rows, and can drive overflow affordances (scrollbars, pagination).
- Investigate precomputing gradients/border textures once per frame to avoid reissuing the same raylib draw calls per window.

## Persistence & Config
- Serialize window geometry and visibility preferences (optional) so power users can keep custom layouts between sessions.
- Surface a debug toggle to reset window positions/sizes to defaults when configuration drifts off-screen.

## Longer-Term Enhancements
- Enable optional window stacking order with simple focus handling (click brings window to front, stores z-index).
- Add keyboard navigation (arrow keys to nudge positions, modifiers for size adjustments) for accessibility.
- Prototype reusable widgets (labels, bars, cards) to eliminate bespoke text measurement logic in each window type.

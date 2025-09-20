# Geist UI

`geist-ui` hosts the viewer-facing overlay window toolkit shared across the Geist workspace. It contains the reusable primitives that manage draggable, resizable HUD windows, including layout, hit-testing, ordering, and chrome rendering.

## Features

- Window identity and z-order tracking through `OverlayWindowManager`.
- Per-window geometry helpers for layout clamping and interaction (`OverlayWindow`, `IRect`).
- Hover, drag, and resize state handling via `HitRegion` utilities.
- Themeable chrome rendering (`WindowChrome`, `WindowTheme`) built on top of `raylib`.

## Usage

Add the crate as a dependency from any workspace member that wants to construct overlay windows:

```toml
[dependencies]
geist-ui = { path = "../geist-ui" }
```

Then import the primitives you need:

```rust
use geist_ui::{OverlayWindow, OverlayWindowManager, WindowChrome, WindowTheme};
```

`OverlayWindowManager` owns the windows and should be updated each frame with cursor input and screen size. When drawing, call `WindowChrome::draw` to render the frame and then populate the interior with app-specific content.

## Development

- Run `cargo fmt` and `cargo clippy --workspace --all-targets` before sending changes for review.
- Favor additional helpers inside `crates/geist-ui` when introducing new overlay behaviors so the viewer stays lean.
- Avoid Raylib-specific state outside of rendering helpers to keep future renderer swaps straightforward.

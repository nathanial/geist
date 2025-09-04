# Geist + Raylib (Rust)

A minimal Rust project using the `raylib` crate to render a simple 3D scene (grid + cube) with a 2D overlay.

## Prerequisites

- Rust toolchain (`rustup` recommended)
- C toolchain (Xcode CLT on macOS or build-essential on Linux)
- CMake (required by `raylib-sys`)
  - macOS: `brew install cmake`
  - Ubuntu/Debian: `sudo apt-get install cmake`

## Run

```
cargo run
```

This opens an 800x450 window, draws a simple 3D grid and a cube, and overlays FPS + text.

## Project Layout

- `src/main.rs`: Window setup and simple scene rendering.
- `Cargo.toml`: Rust crate manifest with the `raylib` dependency.

## Notes

- If you encounter build errors related to CMake or Clang/LLVM, install the prerequisites above and try again.
- The `raylib` crate defaults to `bindgen`, which generates bindings at build time.

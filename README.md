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

This opens a window that renders a simple voxel terrain with a fly camera.

Controls

- `Tab`: Toggle mouse capture
- `W/A/S/D`: Move
- `Q/E`: Down/Up
- `Shift`: Sprint
- `F`: Toggle wireframe voxels
- `G`: Toggle ground grid

## Project Layout

- `src/main.rs`: App loop and voxel renderer entry point.
- `src/voxel.rs`: Minimal voxel chunk and heightmap generation.
- `src/camera.rs`: Simple WASD/mouse fly camera.
- `Cargo.toml`: Rust crate manifest with the `raylib` dependency.

## Notes

- If you encounter build errors related to CMake or Clang/LLVM, install the prerequisites above and try again.
- The `raylib` crate defaults to `bindgen`, which generates bindings at build time.

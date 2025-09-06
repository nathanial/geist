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

CLI options

- `--flat-world`: Generate a flat world consisting only of an infinite stone slab (no hills, trees, or caves).
- `--schem-report [path]`: Analyze a `.schem` file and print unsupported block IDs, then exit. Defaults to `schematics/anvilstead.schem`.
- `--schem-report --counts [path]`: Print counts per block id in the `.schem` file (helps verify if trees/terrain are in the schematic).
- `--schem-only`: Disable all terrain generation (even the slab); only the schematic contents are loaded. All `.schem` files under `schematics/` are auto‑loaded and laid out with spacing.

Highlights

- Multi‑chunk world (grid) with seamless noise generation.
- Greedy meshing per chunk (hybrid plane merging) to reduce triangles.
- Per‑face textures for grass (top/side/bottom) with corrected side orientation.

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

## Assets

- Place textures under `assets/` (already copied from the old C codebase).
- The renderer will try these paths:
  - Grass (per-face): `assets/blocks/grass_top.png` (top), `assets/blocks/grass_side.png` (sides), `assets/blocks/dirt.png` (bottom)
  - Dirt: `assets/dirt.png` or `assets/blocks/dirt.png`
  - Stone: `assets/stone.png` or `assets/blocks/stone.png`
- Greedy mesher loads textures directly per material. If a texture is missing, the corresponding faces may render untextured.

## Notes

- If you encounter build errors related to CMake or Clang/LLVM, install the prerequisites above and try again.
- The `raylib` crate defaults to `bindgen`, which generates bindings at build time.

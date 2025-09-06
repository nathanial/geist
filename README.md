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

This launches the voxel viewer with a fly camera using default world settings.

## CLI

The app now uses subcommands via Clap.

- Global
  - `--log-file [PATH]`: Write logs to a file instead of stderr. If `PATH` is omitted, defaults to `geist.log`. Honors `RUST_LOG`.

- `run`: start the viewer.
  - `--world <normal|flat|schem-only>`: World preset (default: `normal`).
  - `--flat-thickness <N>`: Thickness for `--world flat` (default: 1).
  - `--seed <N>`: World seed (default: 1337).
  - `--chunks-x <N>`, `--chunks-z <N>`: Number of chunks (default: 4x4).
  - `--chunk-size-x <N>`, `--chunk-size-y <N>`, `--chunk-size-z <N>`: Chunk dimensions (default: 32x48x32).

- `schem report [SCHEM_PATH]`: analyze a schematic file.
  - `--counts`: Show counts per block id instead of unsupported list.
  - `SCHEM_PATH`: Optional; defaults to `schematics/anvilstead.schem`.

Examples

```
# Run with defaults
cargo run -- run

# Flat world (1 layer)
cargo run -- run --world flat

# Schematic-only (no terrain)
cargo run -- run --world schem-only

# Custom sizes and seed
cargo run -- run --seed 42 --chunks-x 6 --chunks-z 6 --chunk-size-y 64

# Analyze a schematic (unsupported list)
cargo run -- schem report schematics/castle.schem

# Analyze a schematic (counts)
cargo run -- schem report schematics/castle.schem --counts

# Log to file (default path)
cargo run -- --log-file run

# Log to file (custom path)
cargo run -- --log-file logs/run.log run
```

Help

```
cargo run -- --help
cargo run -- run --help
cargo run -- schem --help
```

Migrating from legacy flags

- `--flat-world` → `run --world flat`
- `--schem-only` → `run --world schem-only`
- `--schem-report [path]` → `schem report [path]`
- `--schem-report --counts [path]` → `schem report [path] --counts`

Bedrock .mcworld (optional)

- Build with feature `mcworld` to auto-import Bedrock `.mcworld` files placed in `schematics/`.
- Command: `cargo run --features mcworld -- run --world flat`
- What’s imported: any `structures/*.mcstructure` files found inside the `.mcworld` archive are parsed via `bedrock-hematite-nbt` and stamped into the world (states are simplified; many special blocks still map to unknown or air).
- Limitations: Full Bedrock chunks (LevelDB) are not imported; only `.mcstructure` exports inside the world are supported for now.

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

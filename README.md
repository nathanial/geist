# Geist (Rust) — Voxel Viewer + Engine

A multi-crate Rust workspace that renders a voxel world using Raylib for GPU, with a clean split between CPU engine crates and a renderer crate. The root binary runs an interactive viewer with fly camera, world generation, chunk meshing, basic lighting, and schematic import tools.

## Prerequisites

- Rust toolchain (`rustup` recommended)
- C toolchain (Xcode CLT on macOS or build-essential on Linux)
- CMake (required by `raylib-sys`)
  - macOS: `brew install cmake`
  - Ubuntu/Debian: `sudo apt-get install cmake`

## Build & Run

- Build the entire workspace: `cargo build --workspace`
- Run the viewer with defaults: `cargo run`
- Or explicitly: `cargo run -- run`

The viewer launches with a fly camera using default world settings.

## CLI

The app now uses subcommands via Clap.

- Global
  - `--log-file [PATH]`: Write logs to a file instead of stderr. If `PATH` is omitted, defaults to `geist.log`. Honors `RUST_LOG`.
  - `--assets-root DIR`: Set assets root explicitly. Otherwise, `GEIST_ASSETS` env var or auto‑detection is used.

- `run`: start the viewer.
  - `--world <normal|flat|schem-only>`: World preset (default: `normal`).
  - `--flat-thickness <N>`: Thickness for `--world flat` (default: 1).
  - `--seed <N>`: World seed (default: 1337).
  - `--chunks-x <N>`, `--chunks-y <N>`, `--chunks-z <N>`: Chunk grid dimensions (default stack: 4x8x4). World height = `chunks_y * 32` voxels.

- `schem report [SCHEM_PATH]`: analyze a schematic file.
  - `--counts`: Show counts per block id instead of unsupported list.
  - `SCHEM_PATH`: Optional; defaults to `schematics/anvilstead.schem`.

Examples

```
# Run with defaults
cargo run

# Flat world (1 layer)
cargo run -- run --world flat

# Schematic-only (no terrain)
cargo run -- run --world schem-only

# Taller stack and custom seed
cargo run -- run --seed 42 --chunks-x 6 --chunks-y 10 --chunks-z 6

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

Bedrock .mcworld

- Support for Bedrock `.mcworld` import has been removed. Schematic tools remain available via `schem` subcommands.

Highlights

- Multi‑chunk world (grid) with seamless noise generation.
- Fixed 32x32x32 chunks stacked vertically; adjust `--chunks-y` to change world height.
- Per‑face meshing: one quad per boundary face cell for watertight output.
- Per‑face textures for grass (top/side/bottom) with corrected side orientation.
- Texture and worldgen config hot‑reload (see Assets).

Controls

- `Tab`: Toggle mouse capture
- `W/A/S/D`: Move
- `Q/E`: Down/Up
- `Shift`: Sprint
- `F`: Toggle wireframe voxels
- `G`: Toggle ground grid

## Project Layout

- Root binary: `src/main.rs`, `src/app.rs`, `src/camera.rs`, `src/player.rs`, etc.
- Cargo workspace managed at the repo root (`Cargo.toml`).

Engine crates (no Raylib dependency):
- `crates/geist-geom`: Minimal math types (`Vec3`, `Aabb`).
- `crates/geist-blocks`: Blocks/materials/registry/config (`BlockRegistry`, `MaterialCatalog`).
- `crates/geist-world`: World sizing, sampling, worldgen params and config I/O (`World`, `WorldGenMode`, `load_params_from_path`).
- `crates/geist-chunk`: Chunk buffer and worldgen helpers (`ChunkBuf`, `generate_chunk_buffer`).
- `crates/geist-lighting`: In‑chunk lighting, neighbor borders (`LightingStore`, `LightBorders`).
- `crates/geist-mesh-cpu`: CPU meshing (`ChunkMeshCPU`, `NeighborsLoaded`, `build_*`).
- `crates/geist-runtime`: Slim runtime with job lanes/workers and CPU results only.
- `crates/geist-structures`: Structures (`Structure`, `Pose`, helpers).
- `crates/geist-edit`: Persistent world edits + revisions (`EditStore`).
- `crates/geist-io`: Import/export tools (schematics).

Renderer crate (Raylib boundary):
- `crates/geist-render-raylib`: GPU upload, shaders, textures (`upload_chunk_mesh`, `ChunkRender`, `TextureCache`, `LeavesShader`, `FogShader`).

Dependency direction:
- `geist-geom` → `geist-blocks` → `geist-world` → `geist-chunk` → `geist-lighting` → `geist-mesh-cpu` → `geist-runtime` → app
- `geist-render-raylib` depends on `raylib`, `geist-mesh-cpu`, `geist-blocks`, and converts to/from `geist-geom`.

## Assets & Hot Reload

- Assets root
  - Default auto‑detection (searches current dir, executable dir, and crate root parents).
  - Override with `--assets-root DIR` or `GEIST_ASSETS=/abs/path/to/repo`.
  - All paths below are resolved relative to the assets root.

- Textures: `assets/blocks/`
  - Examples: `assets/blocks/grass_top.png`, `assets/blocks/grass_side.png`, `assets/blocks/dirt.png`, `assets/blocks/stone.png`.
  - Hot‑reload: changes under `assets/blocks/` are detected and updated live.

- Voxel registry:
  - Materials: `assets/voxels/materials.toml`
  - Blocks: `assets/voxels/blocks.toml`
  - Hot‑reload: edits to either file reload the registry, clear the texture cache, and rebuild all loaded chunks and structures.

- Shaders: `assets/shaders/`
  - Core: `voxel_fog_textured.vs`, `voxel_fog_textured.fs`, and `voxel_fog_leaves.fs`.
  - Hot‑reload: edits in `assets/shaders/` reload shaders and rebind them on existing models.
  - Water: `voxel_water.fs` fragment shader is used for materials tagged `render_tag = "water"`; it supports a subtle wave effect and an underwater mode.

- Worldgen config: `assets/worldgen/worldgen.toml`
  - Hot‑reload: enabled by default (`--watch-worldgen`). On change, worldgen params update; optionally triggers rebuilds (`--rebuild-on-worldgen-change`).

- Schematic palette mapping (for `schem` tools): `assets/voxels/palette_map.toml`.
  - Resolved using assets root (or auto‑detect) so tools work from any working directory.

## Notes

- If you encounter build errors related to CMake or Clang/LLVM, install the prerequisites above and try again.
- The `raylib` crate defaults to `bindgen` (generates C bindings at build time).
- Engine crates deliberately avoid any Raylib types; conversions occur only in `geist-render-raylib`.

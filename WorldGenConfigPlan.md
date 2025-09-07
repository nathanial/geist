**Worldgen Config Plan**

- Scope: Make world generation configurable and adjustable at runtime via TOML configuration and declarative rules. Avoid embedded scripting for performance.

**Current State**
- Generation pipeline: `generate_chunk_buffer` in `src/chunkbuf.rs` calls `World::block_at_runtime_with` (in `src/voxel.rs`) in a tight loop per voxel.
- Runtime knobs today: seed, chunk sizes, and `WorldGenMode::{Normal, Flat{thickness}}` via CLI.
- Hardcoded parameters exist for height map, surface thresholds, cave/carver constants, tree placement, and rare special blocks (e.g., glowstone).

**Target Approach**
- Data-driven configuration in TOML, with hot-reload. No scripting engine.
- Phased delivery:
  1) Parameterize existing generator via TOML (Phase 1).
  2) Add a small declarative rules surface for features (Phase 2).
  3) Optional biome layering via configurable temperature/moisture fields (Phase 3).

**Phase 1: Parameterized Generator (Implemented)**
- Config file: `assets/worldgen/worldgen.toml` (watch + hot‑reload; CLI flag `--watch-worldgen`).
- CLI flags:
  - `--world-config <path>` default `assets/worldgen/worldgen.toml`.
  - `--watch-worldgen` (bool, default true).
- Implementation details:
  - Add `worldgen::WorldGenParams` (serde-backed) to capture all constants currently hardcoded:
    - Height map: frequency, min_y_ratio, max_y_ratio.
    - Surface: `snow_threshold`, `sand_threshold`, `topsoil_thickness` and block names.
    - Carvers: enable, warp/tunnel fractal params, epsilons, room cell size/thresholds, soil/min_y, glow probability.
    - Trees: probability, trunk_min/max, leaf_radius (species selection remains as-is for now).
  - `World` holds an `Arc<RwLock<WorldGenParams>>` and snapshots params into `GenCtx` for low-overhead use inside tight loops. Done.
  - Initial load: read `--world-config` at startup; apply defaults if missing/invalid. Done.
  - Hot-reload: watch the config file; on change, parse and swap params, log summary. New chunks use new params; no automatic rebuild yet (future flag). Done.

**Phase 2: Declarative Feature Rules (Implemented)**
- Extend TOML with `[[features]]` blocks:
  - `when`: basic conditions compiled from TOML into an in-memory rule (no interpreter):
    - `base_in`, `base_not_in`: match current base block name.
    - `y_min`, `y_max`: absolute Y limits.
    - `below_height_offset`: require `y < height - offset`.
    - `in_carved`: require that the position is carved by tunnels/rooms.
    - `near_solid`: require adjacency to solid (noise-evaluated neighbor carve).
    - `chance`: probabilistic gate using a deterministic hash of `(x,y,z)` + seed + rule index.
  - `place`: `{ block = "..." }` replaces the base block when conditions match (first-match wins).
  - Evaluated after carvers and before tree overlay; preserves performance via simple branch checks and lazy near_solid evaluation.
  - Sample included in `assets/worldgen/worldgen.toml` to sprinkle glowstone.

**Phase 3: Biome Layering (Optional)**
- Compute temperature and moisture surfaces from configurable noises; map to biomes via thresholds.
- Allow per-biome overrides for surface materials, carvers, and tree settings.

**Example worldgen.toml (baseline mirroring current defaults)**
```
mode = "normal"          # or "flat"

[flat]
thickness = 1

[height]
frequency = 0.02
min_y_ratio = 0.15
max_y_ratio = 0.70

[surface]
snow_threshold = 0.62
sand_threshold = 0.20
topsoil_thickness = 3
top = { high = "snow", low = "sand", mid = "grass" }
subsoil = { near_surface = "dirt", deep = "stone" }

[carvers]
enable = true
y_scale = 1.6
eps_base = 0.04
eps_add = 0.08
warp_xy = 5.0
warp_y = 2.5
room_cell = 120.0
room_thr_base = 0.12
room_thr_add = 0.12
soil_min = 3.5
min_y = 2.0
glow_prob = 0.0009
tunnel = { octaves = 4, persistence = 0.55, lacunarity = 2.0, scale = 140.0 }
warp = { octaves = 3, persistence = 0.6,  lacunarity = 2.0, scale = 220.0 }

[trees]
probability = 0.02
trunk_min = 4
trunk_max = 6
leaf_radius = 2
```

**Performance Notes**
- Snapshot params into per-chunk `GenCtx` to avoid lock contention and branching in inner loops.
- Keep noise instances in `GenCtx` as today; only frequencies and thresholds derive from config.
- Worldgen hot-reload can optionally auto-rebuild loaded chunks; enabled via `--rebuild-on-worldgen-change`.

**Auto-Rebuild on Worldgen Change (Implemented)**
- CLI: `--rebuild-on-worldgen-change` (default true).
- When the config file changes and reload succeeds, schedule `ChunkRebuildRequested` for all loaded chunks with cause `StreamLoad`.
- Rebuilds happen through the existing job pipeline; no forced streaming unload/load.

**Next Steps**
- Implement and land Phase 1 (in this change).
- Iterate on Phase 2 feature rules design/implementation.
- Consider optional flag to rebuild loaded chunks when worldgen config changes.

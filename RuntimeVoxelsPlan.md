**Objective**
- Replace hardcoded voxel enums in `src/voxel.rs` with a runtime-configurable system driven by a `BlockType` interface and a registry. Enable defining blocks via config (textures, shapes, properties, states) rather than baking them into code.

**Current State (Findings)**
- **Enums baked-in:** `Block`, `MaterialKey`, `TreeSpecies`, `TerracottaColor`, plus utility enums `Axis`, `SlabHalf`, `Dir4` in `src/voxel.rs`.
- **Usage surfaces:**
  - **Meshing:** `src/mesher.rs` matches on `Block` to pick face textures and to emit special geometry for `Slab` and `Stairs`.
  - **Lighting:** `src/lighting.rs` calls `Block::emission()` and treats non-`Air` as skylight blockers.
  - **Worldgen:** `World::block_at` in `src/voxel.rs` returns `Block` variants for terrain/caves/trees.
  - **Schematic import:** `src/schem.rs` maps Minecraft palette keys and legacy IDs to `Block` variants (lots of hardcoded mappings).
  - **Gameplay/UI:** `src/app.rs` hotkeys select hardcoded blocks; `GameState.place_type: Block`; `EditStore` persists `Block` edits; player collision special-cases `Leaves`.
  - **Storage:** `ChunkBuf.blocks: Vec<Block>` holds voxels; `Structure` holds `Vec<Block>`.
- **Textures:** `FaceMaterial` in `mesher.rs` maps to concrete asset filepaths in `assets/blocks/*.png`.

**Design Goals**
- **Runtime types:** Define blocks and their properties in config (TOML/JSON), not code.
- **Stable core:** Keep core performance characteristics; keep `Block` as a compact, copyable voxel value in memory.
- **Parity:** Match current visuals and behavior using the new data-driven system.
- **Extensibility:** Allow families (e.g., planks species, terracotta colors) and stateful shapes (axis logs, slabs, stairs).

**Proposed Architecture**
- **Representation**
  - **`BlockId`/`BlockState`:**
    - `type BlockId = u16;` (0..=65535 registered kinds)
    - `type BlockState = u16;` (bitfield interpreted by the block type; expand later if needed)
  - **`Block` (voxel value):**
    - `#[derive(Copy, Clone, PartialEq, Eq, Default)] struct Block { id: BlockId, state: BlockState }`
    - `const AIR: Block = Block { id: 0, state: 0 };` (reserve id 0 for air)
    - Methods: `is_air()`, `is_solid()`, `emission()`, `debug_name()` delegate via registry/type.

- **Registry**
  - **`BlockRegistry`:** global, read-only after load; maps `BlockId` and names to `BlockType` entries.
  - Provides lookup by name (`"stone" -> BlockId`), by Minecraft key for schem import (via a separate mapper), and helpers to build `Block` from name + state map.
  - Backed by config files under `assets/voxels/` (see Config below).

- **`BlockType` Interface**
  - Methods (invoked with the voxel's `BlockState`):
    - `name(&self) -> &str`
    - `is_solid(&self, state) -> bool`
    - `blocks_skylight(&self, state) -> bool`
    - `light_emission(&self, state) -> u8`
    - `shape(&self, state) -> Shape` (see below)
    - `face_material(&self, face: Face, state) -> Option<MaterialRef>` (for cubic faces)
    - Optional fast-path: `emit_mesh(&self, ctx, state)` to draw non-cubic geometry without general dispatch.

- **Shapes**
  - `enum Shape { Cube, AxisCube { axis_from: Property }, Slab { half_from: Property }, Stairs { facing_from: Property, half_from: Property }, None }`
  - `Property` describes how to extract small enums from the `BlockState` bitfield (e.g., 2 bits for `Axis`, 2 bits for `Half`, 2 bits for `Dir4`).
  - Mesher switches on `Shape` to call stock emitters; face materials come from the type.

- **Materials & Textures**
  - Replace `FaceMaterial` enums with a runtime catalog:
    - `MaterialId(u16)` and `Material { key: String, texture_candidates: Vec<PathBuf> }`.
    - Block types reference `MaterialId` per face role (top/bottom/side) and/or material families.
  - Map catalog keys to texture file paths (e.g., `assets/blocks/*.png`) via `assets/voxels/materials.toml`; keep current paths for minimal churn.
  - Add optional `render_tag` (e.g., `"leaves"`) on materials for shader selection.

- **States/Properties**
  - Stateful families (e.g., `planks` species, `terracotta` color, `log` axis):
    - Each block type declares a compact `state_schema` (named fields with bit ranges and allowed values), e.g. `{ species: 3 bits (0..5), axis: 2 bits (0..2), half: 1 bit, facing: 2 bits }`.
    - Helpers to construct a `Block` from `{ name: "planks", state: { species: "oak" } }`.

- **Config Files (serde TOML)**
  - `assets/voxels/blocks.toml`: block type definitions
    - Example snippet:
      ```toml
      [[blocks]]
      name = "air"
      id = 0
      solid = false
      blocks_skylight = false
      propagates_light = true
      emission = 0
      shape = "cube"
      materials = { all = "unknown" }

      [[blocks]]
      name = "grass"
      solid = true
      blocks_skylight = true
      emission = 0
      shape = "cube"
      materials = { top = "grass_top", bottom = "dirt", side = "grass_side" }

      [[blocks]]
      name = "planks"
      solid = true
      shape = "cube"
      state_schema = { species = ["oak","birch","spruce","jungle","acacia","dark_oak"] }
      materials = { all = { by = "species", map.oak = "planks_oak", map.birch = "planks_birch", map.spruce = "planks_spruce", map.jungle = "planks_jungle", map.acacia = "planks_acacia", map.dark_oak = "planks_oak" } }

      [[blocks]]
      name = "log"
      solid = true
      shape = { kind = "axis_cube", axis = { from = "axis" } }
      state_schema = { species = ["oak","birch","spruce","jungle","acacia","dark_oak"], axis = ["x","y","z"] }
      materials = { top = { by = "species", map.oak = "log_oak_top", map.birch = "log_birch_top", map.spruce = "log_spruce_top", map.jungle = "log_jungle_top", map.acacia = "log_acacia_top", map.dark_oak = "log_oak_top" }, side = { by = "species", map.oak = "log_oak", map.birch = "log_birch", map.spruce = "log_spruce", map.jungle = "log_jungle", map.acacia = "log_acacia", map.dark_oak = "log_oak" } }

      [[blocks]]
      name = "slab"
      solid = true
      shape = { kind = "slab", half = { from = "half" } }
      state_schema = { half = ["bottom","top"], material = ["smooth_stone","sandstone","red_sandstone","cobblestone", "mossy_cobblestone", "stone_bricks", "end_stone_bricks", "prismarine_bricks", "granite", "diorite", "andesite", "polished_granite", "polished_diorite", "polished_andesite", "planks_oak", "planks_birch", "planks_spruce", "planks_jungle", "planks_acacia", "planks_dark_oak" ] }
      materials = { all = { by = "material" } }

      [[blocks]]
      name = "stairs"
      solid = true
      shape = { kind = "stairs", half = { from = "half" }, facing = { from = "facing" } }
      state_schema = { half = ["bottom","top"], facing = ["north","south","west","east"], material = ["planks_oak","planks_birch","planks_spruce","planks_jungle","planks_acacia","planks_dark_oak","smooth_stone","cobblestone","mossy_cobblestone","stone_bricks","quartz_block","sandstone","red_sandstone"] }
      materials = { all = { by = "material" } }
      ```
  - `assets/voxels/materials.toml`: material catalog mapping keys to texture paths
    - Example:
      ```toml
      [materials]
      unknown = ["assets/blocks/unknown.png"]
      grass_top = ["assets/blocks/grass_top.png"]
      grass_side = ["assets/blocks/grass_side.png"]
      dirt = ["assets/blocks/dirt.png"]
      # ... rest mirroring current FaceMaterial textures
      ```
  - `assets/voxels/palette_map.toml`: schematic mapping rules from Minecraft IDs/states to `{ name, state }` entries (replaces hardcoded `map_palette_key_to_block_opt`).

**Progress**
- Scaffolding: Added `src/blocks/{types.rs, material.rs, config.rs, registry.rs, mod.rs}` for runtime blocks, shapes, materials, and config loaders.
- Config assets: Created `assets/voxels/{materials.toml, blocks.toml, palette_map.toml, hotbar.toml}` with a seed set of materials and blocks (air, stone, dirt, grass, glowstone, beacon). Materials support optional `render_tag` (e.g., "leaves").
- Crate wiring: Added `toml` and `mod blocks;`. `main.rs` loads the registry at startup and passes it to `App`/`Runtime`.
- Meshing keys: `meshing_core` generalized; `mesher` now groups meshes by `MaterialId` end-to-end; `ChunkMeshCPU`/`ChunkRender` use `MaterialId` keys.
- Texture/upload path: Upload path uses `MaterialCatalog` to resolve texture candidates and a `TextureCache` keyed by file path strings; first on-disk candidate wins.
- Shader selection: `app.rs` assigns the leaves shader when the material’s `render_tag == "leaves"`; others get the fog shader.
- Runtime wiring: `Runtime` now owns an `Arc<BlockRegistry>`; worker threads pass `&reg.materials` into meshing. The app passes the registry through.
- Compatibility mapping: Removed from mesher/lighting. All materials/occlusion resolve via registry-backed runtime blocks (worldgen migration pending).
- Build status: Project compiles with the new meshing path; rendering should be unchanged for covered materials.

**Remaining Work (Prioritized)**
- Worldgen: Produce runtime blocks from registry (remove temporary runtime resolver), and drive hotbar from `assets/voxels/hotbar.toml`.
- Schematic translator: Implement `assets/voxels/palette_map.toml`-driven mapping to runtime blocks; update `schem.rs`/`mcworld.rs`.
- Occlusion: Finalize slabs/stairs occlusion using registry state after full runtime storage.
- State packing: Implement `state_schema` packing/unpacking; enable by-property material selection.
- Tests/docs: Add tests for state packing and registry lookups; update README/docs.
- Cleanup: Remove legacy enums (`Block`, `MaterialKey`, `TreeSpecies`, `TerracottaColor`, `FaceMaterial`) after parity.

**Implementation TODO**
- DONE: Core types and loaders in `src/blocks/*` with TOML parsing.
- DONE: Config files under `assets/voxels/*` (materials, blocks, palette_map, hotbar).
- DONE: Meshing groups by `MaterialId`; upload path uses `MaterialCatalog` and updated `TextureCache`.
- DONE: Shader selection via `render_tag` in `app.rs`.
- DONE: Registry-driven material resolution in mesher; uses registry for cubes and falls back to `unknown` material when unmapped. Structure mesher also uses registry.
- DONE: Expanded default pack to match current world visuals: added `sand`, `snow`, species logs (`oak/birch/spruce/jungle/acacia`), and leaves. Materials include `render_tag = "leaves"`.
- DONE: Removed FaceMaterial usage from mesher; all grouping/selection uses `MaterialId` from the registry.
- DONE: Legacy FaceMaterial usage removed from mesher; only referenced in docs.
- DONE: Skylight propagation now consults registry `blocks_skylight`.
- DONE: Shape-aware occlusion (registry-driven for cube-like blocks; legacy rules retained for slabs/stairs until registry shapes are added).
- DONE: Block-light propagation flags via registry (`propagates_light`), applied in block and beacon BFS.
- DONE: Slab/Stairs integrated as registry block types; mesher resolves their materials via registry property selectors.
- DONE: Core storage migration to runtime `Block` for chunks, meshing, lighting, edits, events, and structures; app hotkeys/raycast updated.
- NEXT: Worldgen emits runtime blocks from registry (remove temporary resolver); finalize occlusion for slabs/stairs using registry state; config-driven schematic translator and state packing; drive hotbar from config; tests/docs updates.

**Integration Notes (from code audit)**
- Mesh grouping key: Replace all uses of `FaceMaterial` as a map key in `ChunkMeshCPU`/`ChunkRender` with `MaterialId` (or `RenderKey`). Update `meshing_core` and upload paths accordingly.
- Shader selection: Use material/block metadata for shader choice. Add `render_tag` (e.g., `"leaves"`) to materials or allow a block-type override; update `app.rs` to assign the leaves shader based on this tag.
- Lighting: `LightGrid::compute_with_borders_buf(buf, store, reg)` now accepts the registry and seeds skylight through blocks with `blocks_skylight=false` (e.g., leaves). Block-light/beacon BFS uses `propagates_light`.
- Slabs/Stairs: Defined as registry block types with `state_schema`; per-face materials selected via `by = "material"`. Mesher resolves via registry selectors (no FaceMaterial).
- Occlusion by shape: Mesher consults registry types for cube-like occlusion; slabs/stairs keep precise top/bottom occlusion for now. Cross-chunk occlusion is not performed to avoid seams without neighbor access.
- Storage APIs: `generate_chunk_buffer(world, cx, cz, reg)` now requires `&BlockRegistry`. `Structure::new` requires `&BlockRegistry` to seed base blocks. Runtime/event/edit paths use runtime `Block`.
- Light propagation flags: Skylight uses `blocks_skylight`; block-light uses `propagates_light`. BFS updated to honor these flags (current default allows only air).
- Leaves collision: Keep leaves `solid=true` for collisions to match current behavior unless changed via config.
- Material resolution: Implement a resolver that maps `(block, face, state)` to `MaterialId` (for cubes) and use per-shape emitters for non-cubes; both paths feed `MaterialId` to meshing/grouping.
- Debug names: Implement `Block::debug_name()` via registry for UI/debug prints.
- Schematic translator: Move hardcoded maps in `schem.rs` to a config-driven translator; ensure `mcworld.rs` calls the same translator.
- Crates: Add `toml` to dependencies; reuse existing `serde` for config.

**Performance Considerations**
- **Memory:** New `Block` packs into 4 bytes (u16 id + u16 state) vs a large enum; reduces `ChunkBuf.blocks` memory.
- **Dispatch:** Avoid virtual calls per-voxel by:
  - Using a small `Shape` enum and type-local function pointers for fast paths.
  - Precomputing closures/func pointers in the registry for `emit_mesh` and `face_material` per type; mesher does `match shape` then calls pointers.

**Testing and Validation**
- Add a “default pack” mirroring current visuals to confirm parity:
  - Terrain generation exactness at camera-inspected spots.
  - Schematic import of `schematics/anvilstead.schem` renders as before; compare counts via `schem report --counts`.
  - Lighting screenshots: glowstone and beacon brightness patches unchanged.
- Add a small unit test for `BlockState` packing/unpacking given a `state_schema`.

**Migration Work Breakdown**
- Direct implementation per TODO above; no phased migration or temporary shims.

**Estimated Impact (files)**
- `src/voxel.rs`: replace `enum Block`, add worldgen helpers that consult registry.
- `src/mesher.rs`: swap `match Block` with `shape` + `face_material`; keep geometry emitters; return `MaterialId` instead of `FaceMaterial`.
- `src/lighting.rs`: use registry flags for skylight (`blocks_skylight`) and keep `emission()` for block-light sources.
- `src/meshing_core.rs`: group faces by `MaterialId` and carry light levels; remove `FaceMaterial` dependency.
- `src/runtime.rs`, `src/app.rs`: update upload path and shader selection to use material `render_tag`.
- `src/schem.rs`: replace mapping with config-driven translator.
- `src/chunkbuf.rs`, `src/structure.rs`, `src/edit.rs`: change to new `Block` struct.
- `src/app.rs`, `src/gamestate.rs`, `src/player.rs`: UI hotbar, debug prints, collision via `is_solid()`.
- New: `src/blocks/{mod.rs,registry.rs,types.rs,config.rs,material.rs}`.
- New assets: `assets/voxels/{blocks.toml,materials.toml,palette_map.toml,hotbar.toml}`.

**Status Summary**
- DONE: Config assets and loaders in `assets/voxels/*` (materials, blocks, palette_map, hotbar). Materials support `render_tag` (e.g., "leaves"). Air has `propagates_light = true`.
- DONE: `MaterialCatalog`, `BlockRegistry`, `BlockType` with flags (`solid`, `blocks_skylight`, `propagates_light`, `emission`), `Shape`, and `CompiledMaterials` with by-property selectors.
- DONE: Meshing groups by `MaterialId` end-to-end; upload path resolves textures via `MaterialCatalog`. `FaceMaterial` removed from pipeline.
- DONE: Lighting uses registry: `LightGrid::compute_with_borders_buf(buf, store, reg)` seeds skylight via `blocks_skylight` and propagates block/beacon light via `propagates_light`.
- DONE: Added `slab` and `stairs` to registry with `shape` + `state_schema` and per-face `by = "material"` maps.
- DONE: Storage migration started and largely complete: `ChunkBuf` holds runtime `Block { id, state }`; world/worker generates buffers via `generate_chunk_buffer(world, cx, cz, reg)`; structure meshes built from runtime blocks; events/edits pass runtime blocks.
- DONE: Shape-aware occlusion for cube-like blocks via registry; slab/stairs top/bottom occlusion via registry state.
- DONE: Compile errors resolved. Removed legacy enum matches from active meshing path; App/Player/Structure now use registry for solidity/emission and `Block::AIR` constant.
- DONE: State packing/unpacking implemented on `BlockType`; `CompiledMaterials::material_for` now resolves `by = { by, map }` using `BlockState`.
- DONE: Special-shape meshing: slabs and stairs implemented via registry-state emitters, including neighbor face restoration; materials resolved via registry.
- PARTIAL: Schematic loader now maps legacy palette output to runtime via registry bridge; full `palette_map.toml` translator still pending.
- DONE: App hotbar driven by `assets/voxels/hotbar.toml` (fallback to legacy keys when empty/invalid).

**Build Status / Recent Fixes**
- RESOLVED: Removed legacy special-shape matches from mesher; compile green.
- RESOLVED: App/Player/Structure migrated to registry (`is_solid`, `emission`); `Block::AIR` used consistently.
- RESOLVED: Removed unused legacy lighting palette helper.
- RESOLVED: Added `BlockType::state_prop_value/state_prop_is_value` and updated mesher occlusion helpers to use them.

**Remaining Work (No Shims)**
- Remove `MaterialKey` and remaining legacy enums (`TreeSpecies`, `Dir4`, etc.) from code; rely solely on registry state and names.
 
- Implement config-driven schematic translator using `assets/voxels/palette_map.toml`; drop legacy `map_palette_*` functions.
- Drive hotbar from `assets/voxels/hotbar.toml` and expose block debug names via registry.

**API Changes (Implemented)**
- `lighting::LightGrid::compute_with_borders_buf(buf, store, reg)` now requires `&BlockRegistry` and uses `blocks_skylight`/`propagates_light`. Beacon propagation tracks direction and attenuates accordingly.
- `mesher::build_chunk_greedy_cpu_buf(buf, lighting, world, edits, neighbors, cx, cz, reg)` resolves materials via registry and returns `MaterialId`-keyed parts; cross-chunk occlusion disabled to avoid seams without neighbor access.
- `mesher::build_voxel_body_cpu_buf(buf, ambient, reg)` builds local-space meshes using registry for solidity and materials.
- `ChunkBuf` stores runtime `Block { id, state }`; `generate_chunk_buffer(world, cx, cz, reg)` uses a temporary `World::block_at_runtime(reg, x,y,z)` bridge.
- `Runtime` workers own `Arc<BlockRegistry>` and pass it through meshing and texture upload paths.
- `Walker::update_with_sampler(...)` now receives `&BlockRegistry` to evaluate collisions via registry `is_solid`.
- `Structure::new(..., reg: &BlockRegistry)` requires registry for initial block IDs.
- `schem::{load_any_schematic_apply_edits, load_sponge_*, load_mcedit_*}` now require `&BlockRegistry` and emit runtime blocks.

**Immediate Tasks**
 
 
- Remove remaining legacy enums (`MaterialKey`, `TreeSpecies`, `Dir4`, etc.).
- Schematic translator: implement `palette_map.toml` mapping; remove hardcoded palette handling.
- Worldgen: emit runtime blocks directly; remove temporary bridge.

**Acceptance Criteria**
- Visual parity: Grass/dirt/stone, sand, snow render as before; logs/leaves for oak/birch/spruce/jungle/acacia match current, leaves use leaves shader.
- Meshing: Geometry batches by `MaterialId` only; no `FaceMaterial` in the build/upload path.
- Lighting: Skylight seeds through air and stops at any block with `blocks_skylight=true`; leaves block skylight per config; glowstone/beacon brightness patches unchanged.
- Config: Adding a new cube block by TOML (materials + blocks) appears in worldgen or via edits without code changes.

**Testing Steps**
- Run the app and inspect a few chunks for parity (grass tops/sides, sand at beaches, snow at peaks).
- Place each hotkey block and verify textures: dirt, stone, sand, grass, snow, glowstone, beacon.
- Place slabs and stairs of several materials; verify top/bottom/side textures and transitions (sandstone top/bottom/side, quartz top/side, planks, stone bricks, etc.).
- Inspect trees: trunk materials top/side; leaves have correct shader effect and block skylight.
- Toggle wireframe and verify greedy meshing (large stitched quads, minimal draw calls).
- Optional: Add a temporary material+block to TOML (e.g., `granite`) and place via edit to confirm pipeline.
 - Verify chunk seams: faces on chunk borders remain when neighbors are unloaded; no unintended holes at seams.

**Open Questions / Decisions**
- **Config format:** TOML is suggested for readability; JSON is acceptable; RON is another option. TOML chosen for consistency with Rust ecosystem.
- **Skylight and leaves:** Decide desired skylight behavior for leaves; set `blocks_skylight` accordingly in block configs.
- **Double slab:** Represent as separate `Block` (cube with same material) or `slab{half=top|bottom}` plus rule; simplest is mapper converts `type=double` to the material’s base cube block.
- **Material families vs separate block types:** Both supported via `state_schema.material` or multiple block definitions; start with stateful material property for slabs/stairs.

**Next Steps**
- Finalize TOML schemas and initial material key set.
- Execute the Implementation TODO items directly (no phases).

**Deliverables**
- New runtime voxel system with config-defined blocks.
- A default block pack that matches current visuals and behavior.
- Removal of hardcoded enums and palette mappings.

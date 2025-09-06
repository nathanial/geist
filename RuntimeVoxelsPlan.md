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
  - Map catalog keys to texture file paths (e.g., `assets/blocks/*.png`) via `assets/voxels/materials.toml`; reorganize assets as needed.

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

**Implementation TODO**
- Implement core types: `Block`, `BlockId`, `BlockState`, `BlockType`, `Shape`, and `BlockRegistry` with TOML loader.
- Replace storage to use runtime `Block` across `ChunkBuf`, `Structure`, `EditStore`, and helpers.
- Replace `FaceMaterial` with `MaterialCatalog`; update `TextureCache` to load by catalog keys.
- Replace mesher logic with shape-driven emitters (Cube, AxisCube, Slab, Stairs); query materials from registry.
- Replace lighting logic to use `block.is_solid()`, `blocks_skylight`, and `emission()` from registry.
- Add config files: `assets/voxels/blocks.toml`, `assets/voxels/materials.toml`, `assets/voxels/palette_map.toml`, `assets/voxels/hotbar.toml`.
- Replace schematic palette mapping with config-driven translator using `palette_map.toml`.
- Update worldgen to return runtime blocks and stateful variants (e.g., logs/leaves with species/axis).
- Update UI/hotkeys/hotbar to data-driven config; switch debug strings to `block.debug_name()`.
- Remove legacy enums and mappings: `Block`, `MaterialKey`, `TreeSpecies`, `TerracottaColor`, and `FaceMaterial`.
- Drop all compatibility shims and old code paths; proceed directly with the new system.
- Add unit tests for `BlockState` packing/unpacking and key registry lookups.
- Provide a default block pack matching current visuals.
- Update README and docs to describe the new data-driven system.

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
- `src/mesher.rs`: swap `match Block` with `shape` + `face_material`; keep geometry emitters.
- `src/lighting.rs`: delegate to `block.is_air()/is_solid()/emission()`.
- `src/schem.rs`: replace mapping with config-driven translator.
- `src/chunkbuf.rs`, `src/structure.rs`, `src/edit.rs`: change to new `Block` struct.
- `src/app.rs`, `src/gamestate.rs`, `src/player.rs`: UI hotbar, debug prints, collision via `is_solid()`.
- New: `src/blocks/{mod.rs,registry.rs,types.rs,config.rs,material.rs}`.
- New assets: `assets/voxels/{blocks.toml,materials.toml,palette_map.toml,hotbar.toml}`.

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

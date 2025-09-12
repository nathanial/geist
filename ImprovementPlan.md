Quick Wins

  - Assets root consistency
      - Add --assets-root flag and GEIST_ASSETS env var. Use it in resolve_assets_root() and
  apply everywhere assets are read (hotbar assets/voxels/hotbar.toml, shaders under assets/
  shaders/, palette map, schematics dir) so runs are independent of CWD.
      - Move resolve_assets_root() out of src/main.rs into a tiny assets module and thread it
  through App::new and render crate calls.
  - Hot-reload registry
      - Watch assets/voxels/{materials,blocks}.toml similar to textures/worldgen. On change:
  reload BlockRegistry, rebuild texture bindings for affected materials, and schedule chunk
  rebuilds of loaded chunks.
  - FXAA toggle
      - You already have FXAA shaders under assets/shaders/voxel_fxaa.* but don’t use them. Add
  an optional post-process pass (render-to-texture + full-screen quad) with a --fxaa flag and an
  in-app toggle.
  - Water shader hookup
      - Introduce WaterShader like LeavesShader/FogShader and apply to materials tagged
  render_tag = "water". Expose a uniform for “underwater” state when camera Y is below a water
  surface (see assets/shaders/voxel_water.fs).
  - CI and linting
      - GitHub Actions workflow: cargo fmt --check, cargo clippy -D warnings, and cargo test
  --workspace. For proptest, cap cases on CI (PROPTEST_CASES=64) and commit the existing
  regression files.
  - Release profile
      - In Cargo.toml, add [profile.release] with opt-level = 3, lto = "thin", codegen-units = 1.
  Consider a dev-opt profile for realistic perf during development.
  - Edition stability
      - If you don’t require 2024-specific features, consider dropping edition = "2024" to 2021
  across Cargo.toml files to avoid nightly surprises and widen contributor compatibility.
  - Logging ergonomics
      - You already target events logs. Document a couple helpful presets (e.g.,
  RUST_LOG=info,events=debug) in README. Add a --log-level flag mirror in addition to RUST_LOG.

  Medium-Term

  - Asset and shader loading robustness
      - Make shader paths absolute via the resolved assets root. Right now, LeavesShader::load
  and FogShader::load use relative paths; unify with assets root so binaries run from anywhere.
      - Add an asset preflight step at startup to validate required files and log concise
  diagnostics.
  - Ambient occlusion
      - Per-vertex AO for cubes/microshapes based on S=2 micro occupancy (cheap local lookup) and
  fold AO into the merge key as hinted in MesherPlan.md. This materially improves depth without
  real-time lighting cost.
  - Texture atlas
      - Build an atlas at load time to reduce draw calls and binding changes. Your TextureCache
  and per-material binding scheme are a good starting point; atlasing can collapse many small
  materials into a few maps. Keep “special” materials (leaves/water) in separate passes.
  - Edit persistence
      - Add (de)serialization for EditStore (simple CBOR or RON) with --save-edits/--load-edits
  CLI flags. Persist on exit and auto-load on start. This enables a lightweight creative flow
  without a full world save format.
  - Profiling and instrumentation
      - Integrate tracing + tracing-subscriber, with optional tracing-chrome or tracing-tracy
  features to profile chunk build, lighting, mesh upload paths. Gate behind feature flags so it’s
  opt-in.
  - Runtime scheduling polish
      - You already have lane budgets and intent prioritization. Add camera-velocity awareness
  to increase background budgets while moving quickly; decay intents older than N seconds; favor
  ring 0..r first for smoother perception.
  - Schematic runtime tools
      - CLI and in-app commands to load a .schem at runtime and stamp at raycast target or a
  typed coordinate. Reuse your geist_io code and palette map. Add an undo stack for edits.
  - Water and fog polish
      - Use biome tint for water as you do for leaves; add simple depth-based fog variation
  underwater.

  Longer-Term

  - Renderer backend
      - Add a WGPU path next to Raylib for better portability (Metal/Vulkan/DX12, headless
  compute, and potential WASM target). Keep geist-render-raylib intact; new crate geist-render-
  wgpu following the same “engine-only inputs, renderer-only outputs” boundary.
  - Lighting system roadmap
      - Follow your LightingAlgorithmNextSteps.md: micro beacons, nibble packing, bucketed BFS,
  seam unit tests. This will solidify lighting consistency across seams and improve performance.
  - Vertical chunking and LOD
      - If world height grows, split Y into stacked chunk columns and wire vertical micro
  borders. For far distance, add mesh LODs or simplify planes for distant rings to reduce vertex
  counts.
  - Streaming and persistence
      - Design a region file format (Anvil-like) and load/generate on demand with background IO.
  Persist generated buffers or compress chunk data for faster resume.
  - Editor UX
      - Proper “edit mode” with selection, copy/paste, rotate, and multi-block stamp tools;
  gizmos; undo/redo with EditStore revisions. Bind to a lightweight UI overlay.
  - Gameplay systems
      - Simple ECS for entities; moving platforms; interactions; block state UI. Extend geist-
  structures to parametric procedural structures with placement rules integrated into worldgen
  “features”.
  - Documentation and samples
      - Expand README with a “How it works” architecture diagram, common troubleshooting, and
  a small “modding” section for adding materials/blocks. Add a couple curated schematics in
  schematics/ demonstrating different palettes.

  Potential Issues / Tech Debt To Address

  - Shader assignment copies raw Shader structs via copy_nonoverlapping. Consider encapsulating
  this in a small helper with clear semantics or use Raylib’s intended API to avoid subtle
  lifetime pitfalls.
  - Materials and block registry reload
      - If you hot-reload materials/blocks, ensure to update existing Model materials and clear
  any incompatible model caches; drop or rebind textures in TextureCache accordingly.
  - Event queue growth
      - You’ve added intents and backpressure. Consider merging duplicate load/rebuild intents
  earlier and ensure “ring gating” can’t starve edge chunks during rapid camera moves.
  - Tests and CI fidelity
      - Add seam-convergence tests (coarse and micro) and targeted performance assertions (time
  budgets) behind a #[cfg(test)] feature so they don’t run in regular unit CI.

  If you want, I can implement a couple of these quick wins to get momentum:

  - Add --assets-root plus GEIST_ASSETS, refactor asset resolution through app and renderer.
  - Add watcher + reload for assets/voxels/{materials,blocks}.toml and schedule chunk rebuilds.
  - Integrate FXAA as a post-process with a toggle.
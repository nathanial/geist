Decoupling Lighting From Meshing — Implementation Plan

Why decouple
- Remove remesh churn: Lighting edits (emitters, skylight changes) should not rebuild geometry.
- Lower risk: We can evolve the lighting algorithm independently (true two‑phase, epochs) behind a stable buffer API.
- Performance: Small CPU/GPU buffer updates instead of full CPU mesh builds and model uploads.

Current coupling and pain points (today)
- crates/geist-mesh-cpu/src/build.rs computes lighting during mesh builds via `compute_light_with_borders_buf(...)` and bakes per‑vertex light using `sample_face_local_s2(...)`.
- src/app.rs schedules chunk rebuilds for lighting‑only events (e.g., `LightBordersUpdated`), causing cascades and churn.
- Lighting correctness is entangled with meshing order/finalize barriers.

Target architecture (decoupled)
- Persistent per‑chunk light buffers held by `LightingStore` (CPU), optionally mirrored to GPU textures/SSBOs.
- Meshers do not compute lighting; they only read from buffers (CPU fallback) or the shader samples GPU light.
- Lighting updates modify buffers and publish seam planes; geometry remains unchanged.

Deliverables
- Per‑chunk `LightBuffer` (CPU) with block + skylight (and optional beacon) channels.
- Optional GPU light texture per chunk with shader sampling.
- Event/scheduling changes to stop light‑only remeshes and instead upload/update light buffers.
- Transitional glue to populate buffers via existing compute (until true two‑phase ships).

Data model
- CPU buffer per chunk (macro resolution):
  - `struct LightBuffer { sx, sy, sz, block: Vec<u8>, sky: Vec<u8>, epoch: u32, dirty_cpu: bool, dirty_gpu: bool }`
  - Optional: `beacon: Vec<u8>` and `beacon_dir: Vec<u8>` later.
- GPU representation:
  - Option A: 2D atlas per chunk (raylib‑friendly). Pack (x,y,z) → (u = x, v = y + z*sy). Two channels: R = skylight, G = block light (UNORM8). Texture size = `sx × (sy*sz)`.
  - Option B: 3D texture (if supported) — simpler addressing, but more driver variance. Start with Option A.
  - One texture per chunk to simplify lifetime; or global atlas if desired later.

APIs (LightingStore)
- Allocation & lifecycle
  - `fn ensure_light_buffer(&self, cx, cz) -> &mut LightBuffer`
  - `fn free_light_buffer(&self, cx, cz)` on unload.
- Updates
  - `fn write_cpu_light(&self, cx, cz, block: &[u8], sky: &[u8], epoch: u32)` (replace or patch writes; mark dirty flags).
  - `fn mark_dirty(&self, cx, cz, gpu: bool)`
- GPU upload (raylib integration)
  - `fn upload_if_dirty(&self, cx, cz, rl: &mut RaylibHandle) -> Option<raylib::prelude::Texture2D>`
    - Creates/updates a `Texture2D` with packed RG8 data; stores handle in a map keyed by (cx,cz).
- Sampling (fallback / CPU)
  - Adjust `sample_face_local_s2(...)` to prefer persistent buffers when available; otherwise fallback to legacy compute (behind a dev flag during transition).

Shader changes
- Add a light texture sampler and uniforms per chunk:
  - `sampler2D u_LightTex;` with RG8 = (sky, block).
  - Uniforms for chunk dimensions and base world coords: `u_ChunkSize`, `u_ChunkBase`.
- In fragment shader(s):
  - Convert world position to chunk‑local voxel coordinate; compute atlas texcoord (nearest sampling).
  - Sample sky/block and compute final light (max of channels; compose with beacon later if split).
  - Apply VISUAL_LIGHT_MIN and tonemapping/gamma as needed.
- Wire in raylib: bind `u_LightTex` for each chunk model; update uniforms per draw.

Event & scheduling changes
- Stop emitting `ChunkRebuildRequested` for lighting‑only events:
  - src/app.rs: in handlers for `LightEmitterAdded/Removed`, skylight occluder edits, and `LightPlanesUpdated`, mark light buffers dirty and enqueue a tiny “light upload” job (or upload immediately within a small per‑frame budget).
- Add a per‑frame light upload budget (e.g., 32 chunks/frame) to avoid spikes.
- Keep mesh rebuilds for geometry/topology edits and streaming loads only.

Transitional population of light buffers (before two‑phase engine)
- Option 1 (simple):
  - After any chunk build completes, call legacy `compute_light_with_borders_buf(...)` once to refresh its `LightBuffer` and upload. No additional rebuilds for light updates.
  - For lighting edits on already built chunks, schedule a dedicated “lighting compute” job on the light lane to recompute only the light buffer using the current legacy compute (not remeshing).
- Option 2 (skip‑edges assist):
  - Use the existing skip‑edges path to recompute the edited chunk’s buffer quickly, then recompute neighbor buffers on demand (light lane).

Step‑by‑step plan
1) Storage & API
  - crates/geist-lighting/src/lib.rs
    - Add `LightBuffer` struct and maps in `LightingStore` to own per‑chunk buffers and (later) GPU handles.
    - Add API: ensure/free, write_cpu_light, mark_dirty, upload_if_dirty, and getters for shader.
  - Keep current border‑plane plumbing; buffers are orthogonal.

2) Mesh path stop computing light
  - crates/geist-mesh-cpu/src/build.rs
    - Remove calls to `compute_light_with_borders_buf(...)` and `LightBorders::from_grid`.
    - Do not return `light_borders` from mesh build results (adjust types and call sites).
    - Face light for CPU emission paths: use `LightingStore.sample_face_local_s2(...)` which now reads persistent buffers (fallback to flat ambient or legacy sampling behind a feature flag during transition).

3) Shader integration
  - Add a basic light sampling shader (or extend existing material shaders): bind per‑chunk light texture and uniforms.
  - src/app.rs
    - When creating/refreshing `ChunkRender`, attach `u_LightTex` from `LightingStore` and set per‑chunk uniforms.
    - Each frame (or when dirty), call `upload_if_dirty` for visible chunks up to a budget.

4) Event rewiring
  - src/app.rs
    - In `LightEmitterAdded/Removed`, BlockPlaced/Removed (when affecting skylight), and `LightPlanesUpdated`:
      - Schedule light buffer recompute on the light lane (legacy compute) or mark dirty if already recomputed.
      - Do not emit `ChunkRebuildRequested` for lighting‑only changes.
  - src/event.rs
    - Optional: add `LightBufferUpdated { cx, cz, epoch }` for debug/telemetry.

5) Legacy compute to buffer writer
  - crates/geist-lighting/src/lib.rs
    - Add `fn compute_light_for_buffer(buf: &ChunkBuf, store: &LightingStore, reg: &BlockRegistry, world: &World) -> (Vec<u8>, Vec<u8>)` using current algorithm.
  - Light worker consumes that and calls `LightingStore::write_cpu_light` + `mark_dirty`.

6) Clean up light‑only rebuilds
  - src/app.rs
    - Remove branches that schedule `ChunkRebuildRequested { cause: LightingBorder }` on seam changes.
    - Keep finalize barrier for first draw, but lighting updates no longer trigger remesh.

7) Optional GPU atlas management
  - Manage texture lifetime in `LightingStore` (or renderer) keyed by (cx,cz).
  - Evict on `EnsureChunkUnloaded`.

File‑level changes (concrete references)
- crates/geist-mesh-cpu/src/build.rs
  - Remove: calls to `compute_light_with_borders_buf` and `LightBorders::from_grid`; baking of per‑vertex light.
  - Replace: rely on shader sampling; for CPU fallback paths (thin emitters), call `LightingStore.sample_face_local_s2` which reads persistent buffers.
- crates/geist-lighting/src/lib.rs
  - Add LightBuffer struct + buffer maps and APIs.
  - Add `compute_light_for_buffer` (legacy‑based) used only by the light lane (transitional).
- crates/geist-runtime/src/lib.rs
  - Add a small `LightBufferJob` (cx,cz, prev_buf optional) dispatched on the light lane, returning `(cx,cz, block, sky, epoch)`.
- src/app.rs
  - Bind per‑chunk light textures to `ChunkRender` materials; update per‑frame via `upload_if_dirty` with a budget.
  - Remove light‑only `ChunkRebuildRequested` emissions and decrease‑phase hacks.
  - On unload, free light textures/buffers.
- Shaders (raylib material shaders)
  - Add `u_LightTex`, `u_ChunkSize`, `u_ChunkBase`; sample RG8 packed light.

Testing
- Visual: toggle emitters/skylight near seams; verify no remesh and light updates apply.
- Performance: stress with burst edits; track upload budget adherence and frame time.
- Regression: ensure geometry edits still rebuild and render correctly with light textures bound.

Risks & mitigations
- Shader plumbing across all materials: centralize binding logic (like how leaves/water shaders are injected) and fall back to CPU sampling if missing.
- Memory: one RG8 texture per chunk (sx × sy × sz). Mitigate with packing or lazy uploads for visible chunks only.
- Interim inconsistency while legacy compute drives buffers: accept minor lag; budgets and epochs keep it stable.

Migration to true two‑phase engine
- Once decoupled, replace the legacy buffer writer with the new engine’s section updates; buffers are just sinks for engine output.
- Keep shader and renderer unchanged.

Open questions
- Do we need GPU only (no CPU sampling) or both? Start with both; CPU path is useful for headless tests and tools.
- 2D atlas vs 3D texture: begin with 2D atlas for compatibility; revisit 3D later.


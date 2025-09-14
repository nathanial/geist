**Context**
- Goal: decouple lighting from meshing and eliminate seam artifacts while retaining performance and determinism.
- Outcome: moved light atlas ring packing to the main thread using live neighbor borders; added strict runtime validation to catch inconsistencies. Warning backlog across crates has been cleaned to zero.

**What Made This Hard**
- Asynchronous lifecycle/races
  - Workers were packing neighbor rings using LightingStore snapshots taken at job start; the main thread then updated borders, so atlas rings and store diverged by upload time.
  - Borders were updated on full builds, but light‑only jobs initially didn’t publish borders; finalize gating logic further delayed neighbors recomputing, compounding staleness.

- Split responsibilities across crates
  - Lighting (macro/micro), runtime scheduling (three lanes), mesher, render upload, and shaders each owned a part of “the seam story”. The bug sat at the boundary between lighting packer and runtime event ordering.
  - Neighbor ring ownership rules (+X/+Z vs −X/−Z) were implicit in multiple places (packer, shader clamp, event fanout), increasing cognitive load and risk of drift.

- Dual representations of lighting
  - Micro S=2 logic affects CPU lighting propagation and micro seam planes; GPU path samples a macro lightfield texture. Keeping those consistent at chunk seams without a single source of truth was delicate.

- Limited observability
  - Prior to the assertion helper, we had no direct signal that “atlas ring texel ≠ neighbor plane”. Visual symptoms (dark seams) were suggestive but not actionable. The validator immediately exposed the race and indexing.

**Architectural Concerns / Technical Debt**
- Worker‑packed atlas API still exists
  - Status: removed. `pack_light_grid_atlas(light)` has been deleted; only `pack_light_grid_atlas_with_neighbors` remains, assembling rings from authoritative borders at upload time.

- Event ordering and finalize state
  - Finalize readiness and LightBordersUpdated fanout is nuanced. Missing a single border publish or finalize mark can stall neighbor refreshes.
  - Suggestion: add a small state machine doc/tests for finalize, and a single source of truth function for “should schedule light‑only recompute for neighbor X/Z”.

- Shader coupling to ring semantics
  - Shaders required the -X/−Z rings initially; after fix they read both sides. This reliance on ring presence/indices is subtle and easy to regress.
  - Suggestion: add a shader/CPU consistency test (offline image test) for a synthetic two‑chunk scene with strong light contrast across all four faces.

- Mixed micro/macro fidelity at seams
  - Micro planes are computed and exchanged for CPU lighting, but GPU atlas encodes macro brightness only. In pathological micro‑adjacent cases at seams, visual mismatch vs CPU may still occur.
  - Suggestion: document the approximation; optionally encode a micro hint or add a better face‑aware sampling approximation in shader.

- Hard panics in runtime validation
  - `validate_chunk_light_atlas` currently panics on mismatch. Great for dev, risky for production.
  - Suggestion: guard by env var/feature flag; on mismatch, log and requeue a LIGHT job, or drop the frame instead of crashing.

- Tests and CI gaps
  - There are solid unit/property tests in `geist-lighting`, but no integration tests asserting that atlas rings equal neighbor planes for representative layouts.
  - Suggestion: add an integration test that creates two adjacent chunks with contrasting skylight/block values, feeds borders, packs atlas on main thread, and asserts ring equality; also test the shader addressing math via CPU emulation.

~ Warning backlog
  - Status: addressed. Cleaned unused imports/variables, dead code, and style warnings across crates. `cargo check` now reports zero warnings in dev profile. We kept some test-only helpers under `#[cfg(test)]` and explicitly marked a constant with `#[allow(dead_code)]` for future use to stay warning-free.

**Follow‑Up Actions (High Value)**
- Deprecate worker‑packed atlas API; mandate main‑thread packing with live borders. [Proposed next]
- Add a debug flag to turn validation into log+requeue in non‑dev builds.
- Write a small integration test harness for ring seams (X/Z, ± sides) and atlas packing.
- Add a short design note documenting seam ownership, finalize gating, and shader ring sampling assumptions.

**Potential Future Improvements**
- Transition to a shared 3D light texture per chunk or SSBO sampled in shader (already underway) to remove dependence on vertex colors entirely and simplify updates.
- Consider a “seam cache” that precomputes rings for neighbor pairs and updates both owners atomically to reduce recompute fanout.

**Summary**
- The core issue was a race between when neighbor borders were read vs when they were used. The clean fix is to assemble seam rings at upload time from the authoritative store. This is implemented, shaders now sample both ± ring sides, and a strict runtime validator catches regressions. The warning backlog has been cleared. Remaining debt is primarily around deprecating the worker‑packed atlas API, gating the validator in production, and adding integration tests plus a short design note.

**Next Highest‑Value Improvement (Proposal)**
- Deprecate and isolate worker‑side atlas packing to prevent regressions.
  - Add `#[deprecated(note = "Use pack_light_grid_atlas_with_neighbors")]` to `pack_light_grid_atlas` in `geist-lighting`, or make it `pub(crate)`.
  - Add a unit test ensuring app/runtime paths only call `pack_light_grid_atlas_with_neighbors`.
  - Optional: behind a `strict-lighting` feature, `#[deny(deprecated)]` to force call‑site migration in CI.
  - Rationale: this removes the primary footgun that can silently reintroduce seam races and requires minimal code churn with high long‑term safety.

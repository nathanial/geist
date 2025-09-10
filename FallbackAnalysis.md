**Fallback Analysis — Micro S=2 Seam Lighting**

This note analyzes persistent dark seams under the Micro S=2 lighting path and proposes clearer, deterministic fallback strategies to avoid confusion and artifacts.

**Symptoms**
- A dark band appears along chunk seams touching micro-occupied faces (slabs/stairs), even after neighboring chunks have been built.
- Darkness persists, suggesting our current “fallback to 0/None” logic or late-arriving data is not being resolved consistently.

**Where Fallbacks Exist Today**
- Shading fallback: If a neighbor micro plane is missing, `sample_face_local_s2` falls back to 0 (previously) or coarse border planes (now). Mixed code paths increase complexity and can still under-light faces.
- Compute fallback: The Micro S=2 engine seeds from coarse borders when neighbor micro planes are missing, but shading and compute fallbacks are separate and may disagree.
- Eventing: Coarse border changes trigger neighbor rebuilds; micro border availability was not guaranteed to cause neighbors to refresh until now.

**Failure Modes**
- Ambiguous “None vs 0”: Treating missing neighbor micro as literal 0 is indistinguishable from a legitimately dark neighbor, making artifacts hard to reason about and debug.
- Split responsibility: Compute and shading each do their own fallback; path-dependent differences cause inconsistent visuals.
- Attenuation mismatch: Coarse values (macro-step 32) vs micro values (micro-step 16/32) lack a clear, deterministic mapping, so coarse→micro substitutions may be too dim.
- Stale adoption: Even when neighbor micro planes become available, the other side may not re-adopt them promptly without a deterministic refresh.

**Alternatives**

- Option A — No Fallback (Block Until Ready)
  - Idea: If neighbor micro planes are missing, do not change shading; keep previous seam shading (or a neutral ‘pending’ state) until both sides publish micro planes.
  - Pros: Deterministic; no under-lighting.
  - Cons: Requires seam readiness tracking and a consistent prior state; new chunks might briefly show stale or neutral seams.

- Option B — Deterministic Coarse→Micro Upsample at Compute Time (Not in Shading)
  - Idea: When neighbor micro planes are missing, synthesize per-micro seam planes during compute using a fixed upsample mapping from coarse borders. Attach these synthesized planes to the LightGrid. Shading never does ad-hoc fallback.
  - Mapping: For each coarse face cell value V, fill its 2×2 micro cells with `V' = clamp(V - Δ, 0)`, where Δ encodes a micro-step crossing cost consistent with micro attenuation (e.g., Δ=MICRO_ATTEN per one micro step across the seam).
  - Pros: Single source of truth; deterministic; consistent attenuation; shading stays simple.
  - Cons: Slightly overestimates neighbor light until true micro planes arrive (acceptable as a visual fallback).

- Option C — Seam-Bridge Micro Synthesis (One-Cell Extension)
  - Idea: Locally synthesize the neighbor seam micro plane by running a one-cell micro BFS “over the boundary” using: (a) neighbor coarse plane as seeds, (b) micro-face openness predicate for the seam, and (c) a fixed micro attenuation.
  - Pros: More accurate than naive upsample; uses openness to prevent leaks.
  - Cons: More compute; still a fallback approximation.

- Option D — Macro-Only Neighbor, Micro Gated
  - Idea: During shading, never read neighbor micro planes. Instead, use macro neighbor light (coarse borders) but gate it by S=2 openness. This yields consistent brightness without micro dependency on the neighbor.
  - Pros: Simple; deterministic; no “None” case.
  - Cons: Loses sub-voxel accuracy across seams until neighbors’ micro data arrive; still acceptable visually.

- Option E — Min/Max Floors Instead of Zeros
  - Idea: Replace “None → 0” with a deterministic floor derived from either (a) local face-adjacent micro, (b) coarse neighbor value, or (c) a small constant when skylight is known open.
  - Pros: Eliminates black seams.
  - Cons: Heuristic; may brighten too much in truly dark cases.

- Option F — Consistency Epoch + Handshake
  - Idea: Mark each seam with an epoch. A chunk only adopts/shades with neighbor micro planes when both sides have published equal or newer epochs, reducing ‘half-updated’ states.
  - Pros: Deterministic convergence.
  - Cons: More plumbing; needs careful eventing.

- Option G — Seam Relax Pass
  - Idea: When a chunk publishes seam planes (coarse or micro), schedule a cheap seam-only recompute (or rebuild) for its neighbors to refresh adoption.
  - Pros: Converges quickly; we already trigger a neighbor rebuild on coarse changes.
  - Cons: Extra scheduling; still relies on a clear fallback rule to avoid flicker.

**Recommended Direction**
- Adopt Option B (deterministic upsample at compute-time) + Option G (seam relax).
  - Move all fallback logic into the Micro S=2 compute stage:
    - If neighbor micro planes are missing, synthesize micro seam planes from coarse borders with a fixed mapping:
      - Coarse value Vcoarse → micro cells (2×2) with `V' = max(0, Vcoarse - MICRO_ATTEN_CROSS)`, where `MICRO_ATTEN_CROSS` is 1 micro step (block=16, skylight=32 per our current constants), and apply S=2 openness to zero out sealed micro face cells.
    - Publish these synthesized planes as the chunk’s “effective neighbor micro planes” for the current build.
  - Shading then only ever reads micro planes (no per-sample fallback). Coarse values are never consulted in shading.
  - Keep the LightBordersUpdated cascade so neighbors promptly re-adopt true micro planes when available.

**Rationale**
- Single, deterministic place for fallbacks avoids codepath divergence.
- S=2 openness ensures no leaks, even with synthesized planes.
- Attenuation mapping is explicit and consistent with the micro BFS, avoiding the 0/None confusion.

**Implementation Notes**
- LightingStore / LightGrid
  - Add a compute-time “effective micro seam” layer: if neighbor micro is absent, synthesize planes and attach them (tagged as synthesized for debugging).
  - Remove shading-time fallbacks; use only the attached micro seam data.
- Micro Engine
  - During neighbor seeding, if neighbor micro planes are missing, produce the synthesized planes and use them for both propagation and seam export.
- Events
  - Keep emitting LightBordersUpdated after a build completes to trigger neighbor refreshes.
- Instrumentation
  - Add a debug overlay or logs to report seam source per face (+X, −X, +Z, −Z): real micro vs synthesized micro. Helps diagnose convergence.

**Testing**
- Seam unit tests:
  - Pair of chunks with slabs/stairs across seams; verify no dark band when one side is temporarily missing micro planes.
  - After both sides build, verify seam brightness converges whether build order is A→B or B→A.
- Attenuation mapping check:
  - Confirm Vcoarse→V' mapping matches expected micro step costs to avoid over/under brightness.

**Conclusion**
- The persistent dark band is a design smell of mixed fallbacks. Consolidating fallback logic at compute-time with a deterministic, openness-aware coarse→micro synthesis, plus a small event-driven relax pass, yields predictable, crack-free seams without shading-time ambiguity.


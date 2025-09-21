# Structure Anchor Plan

## Goal
Make walker movement truly structure-local whenever the player is anchored, so structure velocity becomes irrelevant and the player never drifts relative to the platform.

## Phase 0 – Decisions & Assumptions (Completed)
- Hybrid collision sampling: when anchored, check structure-local cells first, then fall back to world sampling if the transformed feet position exits the structure bounds.
- Camera yaw: keep camera yaw continuous; store the yaw offset relative to the structure on the anchor.
- Structure rotation: not required for the current implementation, but the anchor model will carry yaw offset so we can propagate rotation deltas later.

## Phase 1 – Anchor Foundations
- Replace `ground_attach` with `WalkerAnchor` (`World` | `Structure(StructureAnchor)`) across `GameState`, event handling, and rendering.
- Define `StructureAnchor` (id, local position, local velocity, yaw offset, grace timer) and helper methods for local↔world conversion.
- Update serialization/debug printing to report the anchor state.

## Phase 2 – Walker Integration Refactor
- Split `Walker::update_with_sampler` into `update_world_space` and `update_structure_space` variants.
- Ensure input wish directions convert into structure-local axes using the stored yaw offset.
- Factor shared collision/AABB helpers so both paths can reuse them.

## Phase 3 – Movement Pipeline Rewrite
- Rework `Event::MovementRequested` to dispatch on the anchor type.
- When anchored, run `update_structure_space`, advance the anchor’s local pose/velocity, and only derive world poses for camera/output.
- Implement attach/detach transitions: convert world pose→local on attach, local→world on detach, honour grace timers, and handle structure removal cleanly.

## Phase 4 – Rendering & Tooling
- Derive camera position and walker world pose from the anchor every frame.
- Update the attachment debug overlay to show anchor type, local pose/velocity, yaw offset, and derived world pose.
- Expose helper utilities for hybrid collision sampling based on the anchor state.

## Phase 5 – Validation & Cleanup
- Add unit tests for the new conversion helpers and anchor transitions.
- Create integration tests (or scripted harness runs) that ride moving structures, jump on/off, and verify no drift.
- Remove platform-velocity hacks and legacy teleport code, run fmt/clippy/tests, and document anchor invariants for future features.

## Deliverables Per Phase
- Each phase should leave the tree compiling and playable.
- Attach/detach functionality must remain functional after every phase.
- Final phase includes doc updates and test coverage to prevent regressions.

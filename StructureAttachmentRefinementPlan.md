# Structure Attachment Refinement Plan

Goal: remove the tug-of-war between player input and moving structures by treating attachments as motion within a structure-local frame, so riders inherit structure motion smoothly and locomote relative to that frame.

## Open Questions
1. When a player is attached to a structure, should their WASD input follow the camera yaw (current behavior) or the structure's local axes/yaw? Clarifying this will avoid surprises when the structure is rotated relative to the camera.
    Answer: It should follow the camera, but also the WASD should be local to the structure itself, so they should move and orient relative to the structure not the broader world.
2. On detach (jumping off, structure despawn), should the player keep the structure's velocity as added world momentum, or snap back to purely player-driven velocity?
    Answer: The player should keep the structure's velocity, no snap back. However, we should also distinguish between jumps which take us off the structure and jumps which keep us within the structure's local coordinate system. The player should be able to jump up and down without "coming off the structure" for example.
3. Do we need to support attachments on structures with dynamic yaw/pitch roll in the near term, or is translation-only sufficient? This informs whether we must generalize beyond the current yaw-only pose.
    Answer: Translation only for now. Currently yaw is always 0. 

## Current Findings
- Player ↔ structure attachment is tracked via `GameState::ground_attach` (structure id, grace timer, local_offset) without storing relative velocity or orientation context, so every frame we recompute world position from scratch and discard structure motion history (`src/gamestate.rs`).
- `Event::StructurePoseUpdated` warps the walker position to `att.local_offset` in structure space each tick, but does not update the walker's velocity, so any platform acceleration is immediately lost (`src/app/events.rs`).
- The movement handler (`Event::MovementRequested`) also teleports the walker to the structure-aligned position before physics, then runs world-space locomotion with `platform_velocity = None`, making the walker fight the structure whenever both are moving (`src/app/events.rs`, `src/player.rs`).
- Collision sampling already merges world chunks plus structures, so a local-frame solution must preserve that shared sampler while avoiding the current O(structures) scan per sample for unrelated bodies (`src/app/events.rs`).

## Constraints & Guardrails
- Preserve existing collision correctness with both chunks and structures, including edits and rotated poses.
- Attachment updates must stay deterministic with the event queue ordering (structure pose updates arrive before movement each tick).
- Avoid introducing frame-order dependent drift between camera, walker, and attachment transforms.
- Keep the walker API reusable for non-structure movement (free-fly / world walking) without duplicating physics code.

## Phase 1 – Attachment Frame Foundation
- Extend `GroundAttach` (or replace with `AttachmentFrame`) to store the structure pose snapshot, structure-local position, and optional structure-local velocity.
- Derive a helper that converts between structure-local and world coordinates in one place, reusing the trapezoidal transforms already available in `geist_structures`.
- Instrument debug overlay to show attachment frame data (local pos, inherited velocity) for validation.

## Phase 2 – Platform Velocity Plumbing
- Track per-structure velocity by diffing `StructurePoseUpdated` poses with the frame delta; cache it on the structure object for reuse.
- Feed that velocity into the walker via the existing `platform_velocity` hook instead of warping the walker pre-physics.
- Update `StructurePoseUpdated` to adjust the walker only when attachment drift exceeds a small tolerance, preventing constant teleport resets.

## Phase 3 – Local-Frame Locomotion
- When attached, transform the input wish vector into structure-local space (subject to Open Question #1), run movement against the structure-aligned sampler, then reproject the result back to world coordinates post-physics.
- Maintain the walker’s internal velocity in structure-local coordinates while attached, restoring world velocity on detach (based on Open Question #2).
- Optimize structure sampling to query the attached structure first, falling back to others only when necessary.

## Phase 4 – Lifecycle & Edge Cases
- Handle structure removal, detachment grace windows, and transitions between multiple structures without dropping inherited velocity.
- Ensure fall damage / jump initiation use structure-relative vertical velocity so elevators and orbiting platforms feel consistent.
- Provide hooks/tests ensuring attachment survives high-frequency pose updates (orbiting schematics, scripted motion).

## Phase 5 – Validation & Tooling
- Add unit tests for attachment math (local ↔ world conversions, velocity carry-over) and integration tests covering attach, ride, detach scenarios.
- Expand the attachment debug window with per-structure velocity readouts and drift warnings.
- Document the flow in `docs/` (or update `AGENTS.md`) so future features can reuse the attachment frame API.

## Definition of Done
- Player movement while attached feels identical regardless of structure translation, with no visible jitter when the structure accelerates.
- Detaching cleanly preserves or discards inherited momentum per the answers to the open questions.
- All automated tests pass, and manual verification on at least one orbiting and one manually-driven structure confirms smooth locomotion.

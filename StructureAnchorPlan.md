# Structure Anchor Plan

## Motivation
The current attachment workflow still marches the walker in world space and simply grafts the structure’s velocity onto the player. That leaves us chasing the platform’s motion rather than becoming part of its local frame, which is why small integration errors continue to push the player off the deck. To reach the behaviour you described (“when we’re on a structure, world velocity is irrelevant”), we need to ground the entire movement + camera pipeline in structure-local space whenever the player is anchored.

## Core Idea
Introduce an explicit anchor frame for the walker. The walker’s authoritative pose lives in structure-local coordinates whenever anchored, and only converts back to world space when we need to render or query non-structure geometry. In other words:

- **Anchor = World** → existing behaviour (walker stores world `pos`/`vel`).
- **Anchor = Structure(StructureId, LocalPose, LocalVel)** → walker stores feet and velocity purely in the structure’s grid.

All player inputs, collision tests, and velocity integration happen inside the active anchor’s coordinate system. World space becomes a derived value (`world_pose = structure_pose * local_pose`). The structure’s velocity never contaminates the player; motion comes solely from player input plus local gravity.

## Architectural Changes

1. **Anchor Abstraction**
   - Convert `GameState::ground_attach` into an enum (e.g., `WalkerAnchor::{World, Structure(StructureAnchor)}`).
   - `StructureAnchor` holds structure id, local feet position, local velocity, facing yaw relative to the structure, and metadata for grace timers.
   - Walker stores both local and world pose caches for quick readback, but mutates only the anchor-native representation.

2. **Sample Pipelines**
   - Expose lightweight sampling helpers on `Structure` that accept local integer coordinates directly.
   - When anchored, movement uses a dedicated local sampler that never leaves structure space; world chunks are consulted only if the structure exposes openings where the player can fall into the terrain (handled via optional world-overlap queries through `structure.pose` + bounds checks).

3. **Physics Integration**
   - Refactor `Walker::update_with_sampler` into two variants:
     - `update_world_space` (legacy path, used when anchor = world).
     - `update_structure_space` that consumes local inputs, integrates local velocity, and returns the updated local pose.
   - Local gravity operates in structure coordinates; conversion to world gravity happens only when we need to render the camera.

4. **Camera & Rendering**
   - Camera position = `structure.pose * walker.local_pose + eye_offset` when anchored. For UI/readouts we expose both world and local positions so debug overlays can show each frame of reference.

5. **Transitions**
   - **Attach:** capture structure id, convert current world feet into local coords via `rotate_yaw_inv`, seed the anchor with local pose/velocity.
   - **Detach (step off or jump):** convert local pose/velocity back into world values once, switch anchor to `World`, and continue integrating in world space.
   - **Structure removal/orbit updates:** if the structure disappears, flush the anchor and drop the walker into world mode at the last derived world position.

6. **Collision & Sampling Edge Cases**
   - For thin structures that hover above terrain, clamp local Y against structure bounds; if the player presses through an opening, switch anchor back to world mode mid-step.
   - For attachments spanning multiple structures (e.g., overlapping decks), pick the highest priority structure and keep a fall-back world sampler in case the local walk goes out of bounds.

## Migration Phases

1. **Foundations (Anchors & Data Flow)**
   - Define the `WalkerAnchor` enum.
   - Update `GameState`/`GroundAttach` to use the new anchor model.
   - Cache structure transforms (`pose` + inverse) for quick local↔world conversion.

2. **Walker Refactor**
   - Split the walker integrator into world vs. structure variants.
   - Ensure inputs (WASD, yaw) map correctly into local axes; yaw offset between camera and structure is kept on the anchor.

3. **Movement Pipeline Rewrite**
   - Adjust `Event::MovementRequested` to dispatch to the correct integrator based on anchor.
   - Replace the current pre/post physics teleport + velocity hacks with pure anchor updates.
   - Update collision sampling to use local samplers when anchored.

4. **Lifecycle & Edge Cases**
   - Implement attach/detach transitions, grace periods, and structure removal handling under the new model.
   - Ensure jumping upward inside a structure retains the anchor, while leaping off the edge converts back to world mode.

5. **Validation & Tooling**
   - Extend the attachment debug overlay to show anchor type, local pose/velocity, and derived world pose for sanity checks.
   - Add unit tests covering local↔world conversion and attach/detach math, plus integration tests that ride moving structures, jump on/off, and orbiting platforms.

6. **Cleanup**
   - Remove all remaining platform-velocity plumbing and world-space teleport shortcuts.
   - Document the anchor invariants in `/docs/` (or the appropriate developer guide) so future systems—like structure-local interactions—can rely on the same frame-of-reference model.

## Open Questions
1. Do we ever need to sample both structure and world collisions simultaneously while anchored (e.g., structure intersects terrain)? If so, we’ll define a hybrid sampler that checks local cells first, then world cells using the transformed coordinates.
2. Should the camera yaw automatically align with the structure’s local axes upon attach, or remain world-aligned with just a relative offset? (Recommend: keep camera yaw continuous, store `yaw_offset` on the anchor.)
3. Do structures ever rotate during attachment in the near term? If yes, we’ll need to propagate yaw deltas into the anchor’s local pose to keep the player from drifting when the platform spins.

With this plan, the walker truly lives inside the structure’s coordinate system while attached, and world velocity becomes irrelevant—exactly the behaviour you requested.

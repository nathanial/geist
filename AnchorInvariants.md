# Structure Anchor Invariants

This document records the core assumptions the movement/anchor system depends on. Future features should preserve or intentionally revise these invariants.

1. **Walker-local coordinate authority** – When `WalkerAnchor::Structure` is active, `StructureAnchor::local_pos` and `local_vel` are the source of truth. World-space pose/velocity for the walker are recomputed each frame from the anchor and the structure’s pose.
2. **Single-frame reconciliation** – Each frame `App::step` calls `sync_anchor_world_pose`, which projects the anchor back into world space and updates the camera when walk mode is active. No other code should patch the walker’s world pose directly while anchored.
3. **Hybrid collision sampling** – While anchored, collision queries run against `structure_local_sampler`, which checks edited blocks first, then falls back to the live world sampler when leaving bounds. All new movement code must keep this ordering to avoid temporarily “falling through” structures.
4. **Yaw offsets carry orientation** – Anchor `yaw_offset` stores the player’s facing relative to the structure. Any feature that mutates player yaw while anchored must also refresh this offset or call `StructureAnchor::update_yaw_offset`.
5. **Detachment grace** – `StructureAnchor::grace` gates detach events. Movement or physics changes must respect the countdown semantics (reset to 8 while supported, decrement otherwise) to avoid accidental detach spam.
6. **World fallbacks on detach** – When a structure disappears, the system converts the anchor to a world anchor and passes the last computed world velocity through the walker. New detach paths must provide an equivalent velocity handoff.

If a change needs to violate one of these invariants, update this document and audit the call sites noted above (`App::step`, `App::handle_event`, `structure_local_sampler`).

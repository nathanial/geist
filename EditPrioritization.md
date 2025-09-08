Edit Prioritization — Dual Queues (Option B)

Overview
- Goal: Ensure edits and lighting rebuilds execute promptly under heavy background streaming.
- Approach: Add a high‑priority worker lane that bypasses StreamLoad backlogs.

Design
- Two job lanes in Runtime:
  - High‑priority: edits and lighting rebuilds.
  - Background: streaming loads and hot‑reload rebuilds.
- Reserved workers:
  - Split total workers into `W_hi` and `W_bg`.
  - Default: `W_hi = max(1, N/4)` and `W_bg = N - W_hi` (ensures at least one background worker when `N > 1`).
- Routing:
  - App emits `BuildChunkJobRequested` with `cause: RebuildCause`.
  - App sends jobs to `submit_build_job_hi` for `Edit | LightingBorder`, and to `submit_build_job_bg` for `StreamLoad`.

Implementation (done)
- API and Events:
  - `Event::BuildChunkJobRequested` now includes `cause: RebuildCause`.
  - App’s stale‑completion requeue uses `cause=Edit` to make the catch‑up rebuild high‑priority.
- App routing (src/app.rs):
  - Intents already coalesced and prioritized by cause → distance ring → age.
  - When emitting `BuildChunkJobRequested`, include `cause` derived from `IntentCause`.
  - On handling `BuildChunkJobRequested`, build `BuildJob` and call:
    - `Runtime::submit_build_job_hi` for `Edit | LightingBorder`.
    - `Runtime::submit_build_job_bg` for `StreamLoad`.
- Runtime (src/runtime.rs):
  - Added dual channels: `job_tx_hi/job_rx_hi` and `job_tx_bg/job_rx_bg`.
  - Spawned two worker pools; both send results to the same `res_rx`.
  - Two dispatchers:
    - HI dispatcher round‑robins HI jobs to HI workers.
    - BG dispatcher round‑robins BG jobs to BG workers.
  - Public API:
    - `submit_build_job_hi(job: BuildJob)` and `submit_build_job_bg(job: BuildJob)`.

Notes & Tuning
- Worker split: Adjust `W_hi` if edits or lighting surges saturate HI lane; small values (1–2) are often sufficient.
- App‑side caps remain in place to keep background under control and avoid starvation.
- Correctness preserved via `rev` checks at completion; reordering only affects latency.

Next Steps (optional)
- Opportunistic fallback (HI workers pull BG when HI idle) can be added to increase utilization when edits are quiet.
- HUD metrics for queue depths and edit latency to validate improvements and guide `W_hi` tuning.

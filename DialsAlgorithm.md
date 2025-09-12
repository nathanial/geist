# Dial’s Algorithm Plan for Micro S=2 Lighting BFS

This document describes how to implement an exact, semantics‑preserving priority queue for the lighting wavefront using Dial’s algorithm (aka bucketed Dijkstra) and how to integrate it into the S=2 micro lighting path. It replaces the general `VecDeque` BFS with a constant‑time, cache‑friendly queue while maintaining bit‑for‑bit results.

## Background

- Current propagation uses a BFS with a `VecDeque` and a guard `if arr[ii] + ATT == lvl` to ensure only the expected wavefront expands.
- Each step across a micro edge applies a constant attenuation:
  - Block: `ATT = 16`
  - Skylight: `ATT = 16`
- Seeds come from skylight seeding, seam planes, and emitters. Levels are `u8` and can be any value (e.g., skylight starts at 255, coarse/micro seams seed at various values after attenuation). Propagation applies `saturating_sub(16)` per step.

Because the per‑edge cost is constant and non‑negative, Dial’s algorithm is a perfect fit to order expansions in strict non‑increasing light (equivalently, non‑decreasing distance).

## Dial’s Algorithm (brief)

- Classic Dijkstra’s with a specialized priority queue for integer, bounded edge weights.
- Maintain `C` FIFO buckets (arrays of small queues), where `C` is the maximum edge weight (or the gcd bucket base). For our use, `C = 16`.
- Track a running key `cur_dist`. The “active” bucket index is `cur_dist % C`.
- To extract the next node, advance `cur_dist` until an active bucket is non‑empty, then pop FIFO. Relax edges: each relaxed neighbor with distance `d'` is pushed to bucket `d' % C`.
- This guarantees nodes are processed in non‑decreasing distance order with O(1) amortized push/pop.

## Mapping lighting to distances

We normally maximize light level; Dial’s is min‑distance. Use a monotone mapping:

- Define distance `d = MAX_LIGHT - level` (u8 math widened to u16 when needed).
- One micro step reduces `level` by `ATT = 16` → increases `d` by 16.
- Relaxation and ordering become: non‑decreasing `d` equals non‑increasing `level`.

Notes:
- Initial seeds can have arbitrary `level` and thus arbitrary `d`. Dial’s handles this as long as we start `cur_dist` at the minimum seeded `d`.
- Because our per‑step delta is exactly 16, `d % 16` is invariant along any path.

## Data structures

- `const C: usize = 16` (bucket count)
- `struct Buckets { q: [Vec<(mx,my,mz,u8)>; C], cur_d: u16, nonempty: usize }`
  - FIFO per bucket (`VecDeque` or `Vec` with head index). `VecDeque` is fine and simplest.
  - `cur_d`: current distance key.
  - `nonempty`: number of pending items across all buckets (or track per‑bucket lengths).

Helper functions:
- `bucket_idx_for(level: u8) -> usize { ((MAX_LIGHT as u16 - level as u16) % 16) as usize }`
- `seed(bkts, mx,my,mz, level)`: compute idx, push, update `cur_d = min(cur_d, d)` and `nonempty += 1`.
- `pop(bkts) -> Option<(mx,my,mz, level)>`:
  - If `nonempty == 0`: return `None`.
  - While `bkts.q[cur_d % 16]` empty: `cur_d += 1` (wraps naturally in u16).
  - Pop front; return item.

## Integration plan (micro.rs)

1) Keep all existing logic unchanged except the queue internals:
   - Precomputed occupancy (`occ8/full`) and `micro_solid_at_fast` remain.
   - Immediate seeding remains (no full‑volume scans).
   - Neighbor gating, seam masks, and `before == lvl` guard remain.

2) Introduce a small bucket type scoped inside `compute_light_with_borders_buf_micro` for both block and skylight passes.

3) Seeding
   - When writing a non‑zero value to `micro_blk[i]` or `micro_sky[i]`, compute `d = (MAX_LIGHT as u16) - (level as u16)` and push to `bucket[d % 16]`.
   - Track `cur_d = min(cur_d, d)` while seeding.

4) Processing loop (per light type)
   - While `pop()` returns an item `(mx,my,mz,lvl)`:
     - Skip if `lvl <= 1`.
     - Check `before = arr[midx(...)]`; continue if `before != lvl` (unchanged guard).
     - Relax 6 neighbors using existing `push(...)` helper (bounds, solid check, `saturating_sub(16)`, write if higher).
     - For each neighbor we updated (i.e., when `arr[ii]` increased), push that neighbor into bucket keyed by its new `level`.

5) Maintain semantics
   - No change to attenuation constants or to the guard `if (arr[ii] + ATT) == lvl` used to decide whether a neighbor gets enqueued; this check stays exactly the same; only the outer queue becomes Dial‑ordered.
   - FIFO order within a bucket preserves the BFS traversal feel and avoids biasing axes.

6) Feature gate and rollout
   - Add `#[cfg(feature = "dial_queues")]` around the Dial buckets; keep `VecDeque` BFS as the default path initially.
   - Provide a cargo feature in `geist-lighting/Cargo.toml` so we can A/B test.

7) Validation
   - Unit/property tests must remain bit‑for‑bit equal versus the baseline.
   - Add a test helper that runs both paths on pseudo‑random chunks and asserts equality (behind `#[cfg(test)]` or via a temporary test feature flag).
   - Profile in representative worlds to confirm equal or reduced time in propagation hot loops.

## Pseudocode

```
struct DialQ {
    buckets: [VecDeque<(usize,usize,usize,u8)>; 16],
    cur_d: u16,
    pending: usize,
}

impl DialQ {
    fn new() -> Self { /* init empty */ }
    fn push(&mut self, mx,my,mz, level: u8) {
        let d = 255u16 - level as u16;
        let b = (d & 15) as usize;
        self.buckets[b].push_back((mx,my,mz,level));
        if self.pending == 0 || d < self.cur_d { self.cur_d = d; }
        self.pending += 1;
    }
    fn pop(&mut self) -> Option<(usize,usize,usize,u8)> {
        if self.pending == 0 { return None; }
        loop {
            let bi = (self.cur_d & 15) as usize;
            if let Some(v) = self.buckets[bi].pop_front() {
                self.pending -= 1;
                return Some(v);
            }
            self.cur_d = self.cur_d.wrapping_add(1);
        }
    }
}
```

Use two instances (`q_blk`, `q_sky`) and wire them where the current `VecDeque` queues are used.

## Correctness and pitfalls

- Correctness follows from Dijkstra with non‑negative equal weights and the distance mapping `d = 255 - level`.
- Ensure we only push neighbors when their stored value increases (exactly as today) to avoid duplicates.
- Keep the `before == lvl` guard to suppress stale pops.
- FIFO within a bucket is important for stability and to avoid directional artifacts.
- Start `cur_d` at the minimum seen distance among seeds; update it as you see smaller distances during seeding (typical seeds are near `d=0..32`).

## Performance expectations

- O(1) push/pop per light with small constant (16 buckets), better cache locality than a large `VecDeque` with mixed levels.
- Fewer stale pops compared to naive bucket scans if we maintain `cur_d` and only advance until a non‑empty bucket.
- Same memory traffic in the grids; all improvements are on the queuing side.

## Step‑by‑step implementation checklist

1) Add a `dial_queues` cargo feature in `geist-lighting`.
2) Implement `DialQ` (private to `micro.rs`).
3) Replace `VecDeque` with `DialQ` behind the feature flag; keep immediate seeding.
4) Pass all existing tests (bitwise equality).
5) Add a hidden A/B test runner to compare `dial_queues` vs baseline on randomized worlds.
6) Measure and, if stable, flip the default to `dial_queues` on by default.

## Rollback plan

- Keep the old `VecDeque` BFS compiled when the feature is not enabled; toggling the feature restores prior behavior instantly.

---

Questions or constraints before I proceed:
- Do we want a global feature flag or a runtime toggle (e.g., env var) for easier A/B in the app?
- Any platforms where `VecDeque` performance anomalies suggest using `Vec` + head index per bucket instead?


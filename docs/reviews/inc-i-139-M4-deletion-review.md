# Code Review: INC-I-139 M4 (DC-3) — A1 deep-fork snap redirect deletion

**Workflow:** redesign · **Run:** 455 · **Incident:** INC-I-139 · **Verdict:** APPROVED

## Scope Reviewed
- `crates/network/src/sync/manager/sync_engine/dispatch.rs` (post-deletion state)
- `crates/network/src/sync/manager/tests_inc_i139.rs` (new CLASS 9)
- `crates/network/src/sync/manager/tests.rs` (obsolete F4 deletion; F1/F2/F3 retained)
- `crates/network/src/sync/manager/production_gate.rs:655-766` (the funnel the path now falls into)
- `specs/sync-snap-admission-architecture.md` (CR-2, DC-3, M4)

## Summary
Approved. The deletion is complete, correct, and surgical. Root goal achieved; no
unintended behavior; retained guards and the CR-2 wedge exit intact.

## Verification Results

1. **Root goal achieved (A1 fully removed) — CONFIRMED.** The `consecutive_empty_headers >= 10`
   block retains only the `best_height`/`gap` bindings, the `gap <= 3` gossip-wait guard, the
   minor-fork regime guard (`gap > 3 && gap < MINOR_FORK_GAP_MAX`), and the funnel fallthrough
   `request_genesis_resync(GenesisFallbackEmptyHeaders)`. No `enough_peers` binding, no
   `gap > self.snap.threshold` admission read, no `snap.attempts = 0` anywhere in the file. Both
   counter-reset writers of the redirect are gone.

2. **No unintended behavior — CONFIRMED.** At `empties>=10 && gap>=50` the path reaches
   `request_genesis_resync(GenesisFallbackEmptyHeaders)` — an emergency reason
   (production_gate.rs:666-671) that bypasses Gate 1 (floor) and Gate 4 (`--no-snap-sync`) while
   honoring Gate 2, Gate 3, and Gate 5 (the 3-attempt limiter). Matches spec CR-2 exactly.
   `MINOR_FORK_GAP_MAX = 50`; the gap 4..49 park and gap<=3 gossip-wait regimes are unchanged.
   `dispatch.rs:83-84` (M5/DC-4 scope) is untouched.

3. **`snap.attempts` now preserved (the DC-3 payoff) — CONFIRMED by consumer read.**
   `request_genesis_resync` READS `snap.attempts` at Gate 5 (production_gate.rs:745) but never
   writes it; the accept path only sets `needs_genesis_resync = true`. CLASS 9
   (`class9_a1_does_not_reset_snap_attempts`, not `#[ignore]`) locks this.

4. **Deletion not too far / not too little — CONFIRMED.** `best_height`/`gap` retained (the two
   guards need `gap`); `enough_peers` removed (A1-only). Obsolete F4
   (`test_inc_i017_deep_fork_snap_redirect_allowed_for_synced_nodes`) removed; siblings F1/F2/F3
   remain. F3 (floor=0, attempts=3, gap=499) post-deletion hits the funnel, Gate 5 refuses at
   attempts>=3 without resetting → asserts attempts==3 → still passes.

5. **Spec/docs — CONSISTENT.** The spec documents this deletion as planned M4/DC-3 and the CR-2
   preservation proof. No new drift introduced.

## Minor Findings (resolved)
- **Stale in-code comment (dispatch.rs, above the `>=10` block)** — the comment described the
  now-deleted snap-first redirect. RESOLVED by the runner: replaced with an accurate regime-split
  description referencing INC-I-139 DC-3. Comment-only, zero runtime effect.

## Security Audit Verdict
```
Verdict: AUDIT-SKIP
Signals: none — deletes a node-local sync-recovery redirect and edits tests; no new input
handling, external-data parsing, trust boundary, crypto, or auth. The change only STRENGTHENS
an existing state-integrity limiter (snap attempts) by removing a bypass.
```

## Consensus-shape checklist (INC-I-075)
1. User-submittable tx triggers this path? No — internal sync-recovery orchestration.
2. Producer-action/attestation pattern triggers it? Only via peer empty-header responses; the
   change removes a state-wipe redirect, does not alter block content or acceptance.
3. Bit-identical block content for all reachable inputs? Yes — no block content, coinbase,
   bitfield, tx ordering, or validation-rule change. Rolling-safe, node-local, no activation height.

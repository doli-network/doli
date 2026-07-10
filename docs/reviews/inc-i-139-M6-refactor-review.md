# Code Review: INC-I-139 M6 — snap.threshold sentinel demotion (RC-1/RC-2)

## Scope Reviewed
`sync_engine/decision.rs`, `production_gate.rs`, `block_lifecycle.rs`, `cleanup.rs`, `sync_engine/dispatch.rs`, `tests_inc_i139.rs` — verified against `recovery.rs` (thresholds), `types.rs` (SnapSyncState), and spec `sync-snap-admission-architecture.md` RC-1/RC-2 (lines 154-190) + CR-4 (73). `snap` is `pub(crate)`; reads are confined to the manager module tree, scanned exhaustively (rg unavailable in reviewer session).

## Summary
Approved. Genuine root-cause refactor, not a reshuffle. Every gap-comparator read of `snap.threshold` is re-homed to `thresholds::SNAP_SYNC_GAP_MIN`; the sentinel reads survive; the zero-margin `threshold==MINOR_FORK_GAP_MAX==50` coupling is structurally dissolved; the `10→50` emergency-enable is behaviorally inert. Two stale code-comments are the only drift.

## Verification of the 7 required claims

1. **Root-cause achieved — VERIFIED.** `decision.rs:163` retains sentinel `snap_allowed = self.snap.threshold < u64::MAX`. `decision.rs:177` and `:208` now read `gap > SNAP_SYNC_GAP_MIN`. `dispatch.rs:263` is literal `local_height + 10` (decoupled comment). `production_gate.rs:822` (`is_deep_fork_detected`) and `cleanup.rs:492` read `SNAP_SYNC_GAP_MIN`. Only numeric consumer left is a cosmetic log. Evidence: source read + test `m6_rc1b_no_gap_comparator_read_of_threshold_in_decision`. conf(0.9, observed).

2. **Bit-for-bit 10→50 — VERIFIED.** Complete post-change read-set of `snap.threshold`: sentinel `== u64::MAX` (`production_gate.rs:630,732,740`), sentinel `< u64::MAX` (`decision.rs:163`), writers `disable_snap_sync`=`u64::MAX` / `enable_snap_sync`=`50` (`block_lifecycle.rs:497,508`), one diagnostic log (`block_lifecycle.rs:261`). 10 and 50 indistinguishable under every sentinel comparison; only observable delta is the log string. Emergency re-enable via `enable_snap_sync()` (`production_gate.rs:750`) behaviorally identical to old `threshold = 10`. Test `m6_rc2_emergency_reenable_admits_snap_under_no_snap_sync` asserts `<u64::MAX`, not the literal. conf(0.88, observed).

3. **RC-1c discv5-grace ordering — VERIFIED.** `decision.rs:204` gates grace on `local_height == 0` as first conjunct, after the fresh-node-wait block (`:173`, also h==0-gated). h>0 node never parks (test `m6_h_gt_0_skips_discv5_grace_proceeds_header_first`); h==0 bootstrap snap-wait preserved. conf(0.85, observed).

4. **No missed floor-reads — VERIFIED (tooling caveat).** Manual read of all 11 manager modules found zero remaining `gap > snap.threshold` / arithmetic uses. `snap` is `pub(crate)`, encapsulated; no external reader. Automated crate-wide grep could not run (rg missing); residual risk low given encapsulation + structural test. conf(0.8, observed).

5. **INV-SYNC-011 / no over-reach — VERIFIED.** Demotion moves comparators to a more restrictive constant (500 > 50), opens no new admission path — `should_snap` still requires `local_height==0 || needs_genesis_resync`, both gated. h>0 discv5 gating is a liveness tightening (proceed instead of park), not an admission loosening. No needed guard removed. conf(0.85, inferred).

6. **Deploy safety — VERIFIED.** Sync-coordinator-internal; touches no consensus rule, no block content (no bitfield/coinbase/tx-order/header/state-root), no serialization. Rolling-safe, no activation height, no version bump. conf(0.9, observed).

7. **Spec/docs drift — 2 minor, non-blocking.** See Minor Findings.

## Minor Findings (non-blocking)
- **Stale field doc.** `types.rs:431` — comment describes a gap threshold, no longer a sentinel. Suggested: reword to "enable/disable sentinel: <u64::MAX = enabled". Minor. conf(0.9).
- **Incorrect constructor comment.** `types.rs:461-467` — "Threshold 50 = snap sync activates when >50 blocks behind" now false post-demotion. Suggested: sentinel semantics + point to SNAP_SYNC_GAP_MIN. Minor. conf(0.9).
- **Magic literal (pre-existing, adjacent).** `block_lifecycle.rs:237` `if gap <= 50` mirrors `MINOR_FORK_GAP_MAX` by hardcoded value. Not introduced by M6; optional. Minor. conf(0.85).

## Final Verdict
Approved for merge. Root cause achieved, behavior preserved on the load-bearing path, spec RC-1/RC-2 faithfully implemented, QA green.

## Security Audit Verdict: AUDIT-REQUIRED
Signals: trust boundary (peer-served state-snapshot admission), state integrity (snap/fork recovery path), INC-I-139/120/081 DoS-fork lineage. Justification: although M6 is a behavior-preserving refactor, it edits the attacker-reachable snap-admission surface and its correctness rests entirely on a "bit-for-bit inert" claim that an independent 5-auditor sweep should confirm before commit.

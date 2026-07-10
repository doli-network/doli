# QA Report: INC-I-139 M4 (DC-3) — Delete A1 deep_fork_snap_redirect

## Scope Validated
Surgical deletion (behavior-preservation) of the A1 `deep_fork_snap_redirect` block
in `crates/network/src/sync/manager/sync_engine/dispatch.rs` (the empty-headers
escalation path in `next_request()`'s Headers branch). Node-local sync-recovery
orchestration only. Not a feature addition.

## Summary
**PASS.** The A1 block was deleted exactly as specified (14 lines removed), the
obsolete F4 test was removed, and CLASS 9 locks the post-deletion behavior. The
full `network` lib suite is green (442 passed / 0 failed / 2 ignored), all five
acceptance criteria are met, and no consumer of the deleted `snap.attempts = 0`
reset is stranded.

## System Entrypoint
`cargo test -p network --lib` — pure in-process unit/state-machine tests; no live
node startup required for this node-local sync-orchestration change.

## Change Verification (git diff)
Three milestone-relevant source files changed, matching the spec exactly:
- `dispatch.rs` — 14 deletions (A1 block: `enough_peers` binding + redirect `if`).
- `tests.rs` — 41 deletions (obsolete F4 `test_inc_i017_deep_fork_snap_redirect_allowed_for_synced_nodes`).
- `tests_inc_i139.rs` — 66 insertions (CLASS 9 `class9_a1_does_not_reset_snap_attempts`).

Note: `git diff --stat` also lists pre-existing, unrelated working-tree
modifications present at session start (`.claude/skills/*`, `docs/.workflow/*`,
`CLAUDE.md`, `specs/SPECS.md`, `docs/DOCS.md`). These are NOT part of M4 and were
not touched by this milestone.

## Acceptance Criteria Results

### Must Requirements (REQ-SNAP-001 / REQ-SNAP-002)
#### AC-1: Wedge exit preserved — PASS
The funnel fallthrough `self.request_genesis_resync(RecoveryReason::GenesisFallbackEmptyHeaders)`
remains intact at `dispatch.rs:143-145`, reached when `consecutive_empty_headers >= 10`
and `gap >= MINOR_FORK_GAP_MAX(50)`. `class1_evidence_gated_snap_only_at_gap_50_plus_empties`
and `class5_inc_i138_replay_gap28_no_genesis_resync` both pass.

#### AC-2: Regime guards retained — PASS
Both guards present in `dispatch.rs`: the `gap <= 3 && local_height > 0` gossip-wait
guard (`:104`) and the minor-fork regime guard `gap > 3 && gap < MINOR_FORK_GAP_MAX`
(`:130`). Confirmed by reading the file.

#### AC-3: Attempts-limiter no longer bypassed — PASS
The deep-fork empty-headers path no longer zeroes `snap.attempts`. `class9_a1_does_not_reset_snap_attempts`
passes (attempts preserved at 2 across the escalation).

#### AC-4: dispatch.rs height-based reset untouched (M5 scope) — PASS
`self.fork.use_height_based_headers = false;` (`:83`) and
`self.fork.consecutive_empty_headers = 0;` (`:84`) inside the height-based block are
STILL PRESENT. Not removed — correctly deferred to M5.

#### AC-5: No consensus/block-content change — PASS
Change is confined to the sync manager's request-dispatch state machine. No block
content, coinbase, bitfield, tx ordering, or validation-rule change. Rolling-safe;
no activation height required.

## End-to-End / Test Suite Results
| Suite | Result | Notes |
|---|---|---|
| `cargo test -p network --lib` | 442 passed / 0 failed / 2 ignored | class3 (DC-4/M5 co-test) is the ignored case; matches expected. |
| class9 / class1 / class5 (named) | 3 passed / 0 failed | Locks post-deletion behavior + preserves wedge exit. |

## Exploratory Testing Findings
| # | What Was Tried | Expected | Actual | Severity |
|---|---|---|---|---|
| 1 | Searched all non-test resetters of `snap.attempts = 0` to check for a stranded consumer of A1's reset | Only genuine snap-success / block-apply / gated-retry paths remain | `cleanup.rs:503` (gated 30s retry-after-exhaustion), `block_lifecycle.rs:151/307/371` (sync-complete on real block apply). All correct and independent of A1. | none |

The deleted A1 reset was an *unconditional* wipe on the deep-fork escalation path —
the only such wipe on that path. No downstream logic depends on it; the funnel
fallback (`request_genesis_resync`) reads `snap.attempts` (Gate 5) but never needs
it zeroed. Deletion strands nothing.

## Failure Mode Validation
| Failure Scenario | Triggered | Detected | Recovered | Notes |
|---|---|---|---|---|
| Deep fork, gap>50, empties>=10, floor>0 | Yes (class9 state) | Yes | Yes | Now falls through to gated genesis-resync funnel; attempts limiter intact. |
| Minor fork gap 4..50, empties>=10 | Yes (class5) | Yes | Yes | Awaits G3/coordinator ShallowRollback; no genesis resync (INC-I-138). |

## Security Validation
Not applicable — no external-data ingestion surface. Change is internal state-machine
orchestration.

## Specs/Docs Drift
None found. `specs/sync-snap-admission-architecture.md` (CR-2 / DC-3 / M4) and
`docs/redesigns/sync-snap-admission-redesign-analysis.md` (REQ-SNAP-001/002) match
the delivered behavior.

## Blocking Issues
None.

## Non-Blocking Observations
None.

## Final Verdict
**PASS** — All Must requirements (AC-1..AC-5) met. Suite green (442/0/2). Approved.

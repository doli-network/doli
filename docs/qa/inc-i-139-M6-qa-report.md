# QA Report: INC-I-139 M6 — snap.threshold demotion to enable/disable sentinel

## Scope Validated
Package `network`, sync coordinator (`crates/network/src/sync/manager/`). REFACTOR — acceptance bar is behavior preservation (regression) EXCEPT the one intentional change RC-1c (h>0 no longer waits on discv5 grace).

## Summary
**PASS.** All 6 acceptance criteria met. `snap.threshold` is now a pure enable/disable sentinel: the four gap-comparator reads are re-homed to `thresholds::SNAP_SYNC_GAP_MIN` (=500), the dispatch peer-quality filter is decoupled to a literal `+10`, and the emergency re-enable uses `enable_snap_sync()` (50). Every surviving production read of `snap.threshold` is either a sentinel (`== u64::MAX` / `< u64::MAX`), a value-writer, or a diagnostic log. No version bump, no consensus/block-content change, no activation height touched. Full `cargo test -p network --lib` is green at 446/0/1.

## System Entrypoint
Rust library crate; validated via `cargo test -p network --lib` (workspace root). No running node required — sync-coordinator-internal refactor.

## Traceability Matrix Status
| Requirement | Priority | Has Tests | Tests Pass | Acceptance Met | Notes |
|---|---|---|---|---|---|
| REQ-SNAP-008 (M6 sentinel demotion) | Must | Yes | Yes | Yes | m6_rc1b, m6_h_gt_0, m6_rc2 + 14 tests_inc_i139 backstop |

## Acceptance Criteria Results

### Must Requirements
**AC-1 (RC-1a sentinel preserved) — PASS.** `needs_genesis_resync()` (production_gate.rs:629-647) and `request_genesis_resync()` Gates 1-5 (692-766) structurally unchanged. Sentinel reads intact at 630/732/740 (`== u64::MAX`). Only the emergency-enable mechanism changed (`threshold=10` → `enable_snap_sync()`).

**AC-2 (RC-1b class-kill preserved) — PASS.** Grep confirms zero `gap > self.snap.threshold` comparator reads remain in production code. All four re-homed: decision.rs:177 & 205 (SNAP_SYNC_GAP_MIN), production_gate.rs:822 (is_deep_fork_detected), cleanup.rs:492 (snap retry). `SNAP_SYNC_GAP_MIN=500` (recovery.rs:216) is the single gap floor; `MINOR_FORK_GAP_MAX=50` (recovery.rs:212) unchanged — recovery.rs not in diff. Test m6_rc1b green.

**AC-3 (RC-1c intentional change) — PASS.** decision.rs:202 discv5-grace now gated on `self.local_height == 0` (first condition). h>0 node with gap>500, <3 peers, future grace deadline proceeds to header-first. h==0 grace preserved (still applies for legitimately-fresh bootstrap). Test m6_h_gt_0 green.

**AC-4 (RC-2 bit-for-bit) — PASS.** `enable_snap_sync()` sets 50 (block_lifecycle.rs:508). Value change 10→50 is behaviorally inert: only value writers are `disable_snap_sync()`=u64::MAX and `enable_snap_sync()`=50; every read is `== u64::MAX`/`< u64::MAX` sentinel; the dispatch `+10` is decoupled (literal, was always `.min(10)`=10); block_lifecycle.rs:261 is a diagnostic `warn!` only (no decision). Test m6_rc2 green.

**AC-5 (regression) — PASS.** `cargo test -p network --lib` = **446 passed / 0 failed / 1 ignored** (baseline 443/0/1 + 3 M6). `cargo test -p network --lib tests_inc_i139` = **14 passed / 0 failed** — all INC-I-139 classes green.

**AC-6 (safety) — PASS.** Only 6 network files changed (5 source + tests_inc_i139.rs). No Cargo.toml version, no CURRENT_PROTOCOL_VERSION / EPOCH_STATE_FORMAT_VERSION / MIN_PEER_PROTOCOL_VERSION, no activation_height in real-code diff (matches only in unrelated skill docs). Sync-coordinator-internal; no consensus rule or block-content change.

## Exploratory Testing Findings
| # | What Was Tried | Expected | Actual | Severity |
|---|---|---|---|---|
| 1 | Grep whole workspace (bins/ + crates/, excl. sync manager) for any snap.threshold value read | Only disable/enable writers | `bins/node/src/node/init.rs:696` = `disable_snap_sync()` only; no external value read | none |
| 2 | Grep for any surviving `gap > self.snap.threshold` numeric comparator in production code | None | None (only in test-doc comment strings) | none |
| 3 | Round-trip: `disable_snap_sync()`→u64::MAX then emergency `enable_snap_sync()`→50 (<u64::MAX=enabled) | Enabled after emergency | Confirmed (m6_rc2 + tests.rs:4540 asserts 50) | none |

No remaining path found where the numeric VALUE of `snap.threshold` (vs sentinel state) influences an admission or gap decision. Demotion is complete.

## Failure Mode Validation
Not applicable — internal refactor with no new failure modes; regression suite (446 tests) is the failure-mode backstop.

## Security Validation
Not applicable — no external data surface introduced; sync-coordinator-internal state field only.

## Specs/Docs Drift
Spec `specs/sync-snap-admission-architecture.md` (RC-1/RC-2 ~154-188, CR-4 ~73) aligns with implemented behavior. No drift found.

## Blocking Issues
None.

## Non-Blocking Observations
None.

## Final Verdict
**PASS** — All Must acceptance criteria (AC-1..AC-6) met. Full network lib suite green at 446 passed / 0 failed / 1 ignored; INC-I-139 backstop 14/0. Approved for review.

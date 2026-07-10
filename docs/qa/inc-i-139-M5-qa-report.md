# QA Report: INC-I-139 M5 (DC-4) — Single-Owner Evidence Counter

## Scope Validated
Node-local sync refactor DC-4: removal of the request-dispatch-time
`consecutive_empty_headers = 0` reset in `next_request`'s `use_height_based_headers`
branch (`crates/network/src/sync/manager/sync_engine/dispatch.rs`), plus the two
reconciled pin tests and the de-ignored class-3 co-test. Requirement: REQ-SNAP-007
(single-owner evidence counter, INV-SYNC-011 extended).

## Summary
**PASS.** The DC-4 diff is exactly the described single-line deletion (flag-clear at
:86 kept, gap≤3 gossip-wait reset at :118 kept). The full network lib suite is GREEN
at 443 passed / 0 failed / 1 ignored, matching the expected count precisely. All four
acceptance criteria are met. The post-snap false-positive analysis holds: removing the
dispatch reset cannot trigger spurious escalation because the escalation path is
gap/staleness-gated and diverted by regime guards in the post-snap window.

## System Entrypoint
Validation is at the unit/integration-test layer (node-local refactor, no running
network required). Command: `cargo test -p network --lib`. Diff verified against base
`2930c80b` via `git diff`.

## Acceptance Criteria Results

### AC-1: class3 counter accumulates to ≥10 under sustained empty headers — PASS
`class3_counter_not_starved_by_dispatch84_reset` is de-ignored and passes. The test
drives 15 cycles of (+1 empty header, re-arm `use_height_based_headers`, dispatch) and
asserts `max_counter >= 10`. Pre-DC-4 the dispatch reset zeroed it to 1 each cycle;
post-DC-4 it accumulates. GREEN.

### AC-2: full network lib suite GREEN (443/0/1) — PASS
`cargo test -p network --lib` → `443 passed; 0 failed; 1 ignored`. Exact match.
The single ignored test is `sync::adversarial_tests::test_adaptive_gossip_large_network_floor`
— a gossip/adversarial large-network test, pre-existing and unrelated to DC-4.
Class1/class5 co-tests confirmed passing by name:
- `class1_n4_wedge_parks_no_snap_below_gap_50` (INC-I-012 F1 / INV-SYNC-010) — ok
- `class1_evidence_gated_snap_only_at_gap_50_plus_empties` — ok
- `class5_inc_i138_replay_gap28_no_genesis_resync` (INC-I-138 replay) — ok
- `test_inc_i139_dc4_height_fallback_dispatch_preserves_counter_pin` — ok
- `test_inc_i138_d2_block_applied_resets_counter_pin` (sibling, unchanged) — ok
- `test_inc_i017_height_based_request_fires_before_genesis_fallback` (now asserts 15) — ok
- `test_post_snap_empty_headers_triggers_height_fallback` — ok

### AC-3: no post-snap false-positive escalation — HOLDS (PASS)
Verified by reading `dispatch.rs` + `recovery.rs` classify() + the class1/class5 tests:
- In the post-snap window `use_height_based_headers=true`, so `next_request` takes the
  `:72` height-based branch and returns early — the `:102` escalation (`>=10`) is not
  even reached while the flag is armed.
- The `:102` escalation, when reached, is regime-split: gap≤3 → gossip-wait (which
  DOES reset the counter and parks, INC-I-026); 4≤gap<50 → minor-fork park (no
  genesis-resync).
- Coordinator Rule 2 (`recovery.rs:401-410`): the empty-count `deep_fork_confirmed`
  branch requires gap ≥ `MINOR_FORK_GAP_MAX`(50) AND `last_applied_secs` ≥
  `STALE_TIP_SECS`(300); `large_gap` requires gap ≥ `SNAP_SYNC_GAP_MIN`(500).
- A small post-snap gap with the first apply landing in seconds resets the counter via
  `block_lifecycle.rs` long before the 300s staleness gate — so a preserved counter
  cannot manufacture a deep-fork verdict at small gap. The preservation only bites for
  a genuine gap≥50 sustained-empty deep fork, which is the intended behavior.

### AC-4: no adjacent breakage — PASS
- Height-based request still FIRES before genesis fallback: the `:72` branch precedes
  the `:102` escalation; `test_inc_i017_height_based_request_fires_before_genesis_fallback`
  passes (now asserting the counter is preserved at 15).
- Flag still cleared: `self.fork.use_height_based_headers = false;` retained at :86.
- Block-apply reset intact: `test_inc_i138_d2_block_applied_resets_counter_pin` passes.
- gap≤3 gossip-wait reset intact: retained at dispatch.rs:118; class5 (gap=28) parks.

## Traceability Matrix Status
| Requirement | Priority | Has Tests | Tests Pass | Acceptance Met | Notes |
|---|---|---|---|---|---|
| REQ-SNAP-007 (DC-4 single-owner counter) | Must | Yes | Yes | Yes | class3 + dc4 pin + inc_i017 pin |
| INV-SYNC-011 (extended) | Must | Yes | Yes | Yes | spec §264; only apply + gap≤3 reset |

### Gaps Found
None. Diff is exactly the specified change; no orphan code or unmapped tests.

## Specs/Docs Drift
None. `specs/sync-snap-admission-architecture.md` (DC-4 §134-137, INV-SYNC-011 §264)
matches the shipped code: flag-clear kept, counter reset removed, gap≤3 exception kept.
In-code comments at dispatch.rs:81-85 correctly cite INV-SYNC-011 and INC-I-139 E5.

## Blocking Issues
None.

## Non-Blocking Observations
None.

## Final Verdict
**PASS** — All Must criteria (AC-1..AC-4) met. DC-4 diff is minimal and exactly as
specified; full network lib suite GREEN (443/0/1); the single ignore is pre-existing
and unrelated; post-snap false-positive analysis holds. Approved for review.

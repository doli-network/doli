# QA Report: State-Root Lazy Tier-0 — Milestone M1

- **RUN_ID**: 459
- **Branch**: `feature/state-root-lazy-tier0`
- **Milestone type**: REDESIGN, behavior-ADDITIVE
- **Date**: 2026-07-18

## Scope Validated
The M1 memoize-on-compute seam for the state-root serve path:
- New `Node::serve_state_root()` (`bins/node/src/node/state_root_serve.rs`) and its wiring behind the live `SyncRequest::GetStateRoot` handler (`bins/node/src/node/validation_checks.rs`).
- Additive logging canary `log_state_root_components()` (`crates/storage/src/snapshot.rs`).
- Byte-identity of the served root vs the legacy `storage::compute_state_root` formula.
- Confirmation that all M2-deferred items were NOT touched.

## Summary
**PASS.** M1 is a clean behavior-additive redesign step. The served state-root VALUE is byte-identical to the legacy `compute_state_root` (unchanged three-component BLAKE3 formula), the new best_hash-keyed memo correctly serves O(1) hits, recomputes-and-writes-back on cold/stale memos, and never serves a stale tuple after the tip advances. All 12 targeted tests are green (7 memoize + 5 golden-identity). Every MUST-NOT (M2-deferred) item is verifiably intact.

## System Entrypoint
Rust workspace; validation performed via targeted `cargo test`. Both suites compiled and ran clean:
- `cargo test -p doli-node --test state_root_memoize_m1` → 7 passed, 0 failed.
- `cargo test -p storage --test state_root_golden_identity_test` → 5 passed, 0 failed.

(Note: the storage crate name is `storage`, not a `<storage-crate>` placeholder.)

## Traceability Matrix Status
| Requirement | Priority | Has Tests | Tests Pass | Acceptance Met | Notes |
|---|---|---|---|---|---|
| REQ-SROOT-001/002 (served root == legacy, byte-identical) | Must | Yes | Yes | Yes | Golden `test_compute_state_root_equals_documented_formula` locks `H(H(cs)‖H(utxo)‖H(ps))`; `test_cold_memo_serves_legacy_value` asserts served root == legacy compute. |
| Memo staleness (best_hash-keyed; stale tuple not served) | Must | Yes | Yes | Yes | `test_stale_memo_not_served_recomputes`, `test_stale_memo_overwritten_with_current`. |
| Cold memo computes fresh + writes back | Must | Yes | Yes | Yes | `test_cold_memo_writes_back_to_cache`. |
| Vote-serve path memoizes (repeat quorum votes don't re-scan) | Should | Yes | Yes | Yes | `test_repeat_quorum_vote_hits_populated_memo`, `test_memo_hit_returns_cached_without_recompute`. |
| REQ-SROOT-007 additive canary is behavior-neutral | Should | Yes | Yes | Yes | `test_serve_path_canary_smoke_no_panic`; canary is logging-only. |

### Gaps Found
- Compute-error path (`Err` arm → `SyncResponse::Error`, no memoization) is verified by **code inspection only** — no unit test. This is acceptable: `compute_state_root` is infallible for a valid in-memory `Node`, so the branch is unreachable in test harnesses. Tracked as a non-blocking observation.

## Acceptance Criteria Results

### Must Requirements
#### REQ-SROOT-001/002: Served root byte-identical to legacy
- [x] Formula unchanged: `H(H(cs_canon) ‖ H(utxo_canon) ‖ H(ps_canon))` — PASS (golden test locks it byte-for-byte).
- [x] Cold-memo serve returns exactly the legacy value tagged with current tip — PASS.
- [x] No root-byte / formula change introduced by M1 — PASS.

#### Memo staleness
- [x] Memo keyed on `best_hash`; a tuple with `cached.best_hash != current best_hash` is ignored and recomputed — PASS.
- [x] After a stale serve, cache is overwritten with the current tuple — PASS.
- [x] Cold (`None`) memo computes fresh and writes back — PASS.
- [x] Lock ordering: chain/utxo/producer read guards are dropped **before** the `cached_state_root` write guard is taken (leaf-lock ordering, mirrors `state_update.rs`) — PASS (verified in `serve_state_root.rs` lines 49–63).

### Should Requirements
#### Vote-serve memoization
- [x] Cold GetStateRoot computes AND memoizes; a repeat request at the same height is an O(1) hit with no recompute — PASS.

## End-to-End Flow Results
| Flow | Result | Notes |
|---|---|---|
| GetStateRoot cold → compute → write-back → repeat hit | PASS | Second call served from memo (sentinel-seed proves no recompute). |
| Tip advances (memo left at prior hash) → next GetStateRoot | PASS | Stale tuple rejected, fresh recompute, cache updated to current tip. |

## Exploratory Testing Findings
| # | What Was Tried | Expected | Actual | Severity |
|---|---|---|---|---|
| 1 | Memo written at height N (wrong_hash, height+7) then served at current tip | Rejected + recomputed for current tip, not served stale | Recomputed; served current `best_hash`/`best_height`; sentinel never returned | none (correct) |
| 2 | Compute-error path (code inspection) — does an error poison the memo? | Error returned, memo untouched | `Err` arm returns `SyncResponse::Error`; write only in `Ok` arm → memo not poisoned | none (correct, code-verified) |
| 3 | Additive canary on repeated serves | No behavior change, no panic | Serve returns well-formed StateRoot; canary is logging-only | none (correct) |

## MUST-NOT (M2-Deferred) Verification
| Item | Required State | Observed | Result |
|---|---|---|---|
| Eager compute at `apply_block/state_update.rs` (~135–146) | Still present | Phase-2 compute + Phase-3 write-back to `cached_state_root` intact | PASS |
| `[STATE_FP] sr=` field | Unchanged | `apply_block/mod.rs:438` format string unchanged | PASS |
| `CURRENT_PROTOCOL_VERSION` | Still 8 | `status.rs:49` = `8` | PASS |
| New activation height | None added | No NetworkParams AH change; no new gate | PASS |
| `EpochState` / `EPOCH_SNAPSHOT_HF` | Untouched | Not in changeset | PASS |

## Failure Mode Validation
| Scenario | Triggered | Detected | Recovered | Notes |
|---|---|---|---|---|
| Stale memo after tip advance | Yes (unit) | Yes | Yes | Recompute path serves correct current root. |
| Compute error mid-serve | Untestable (unreachable for valid Node) | n/a | n/a | Code-verified: error not memoized, returned as `SyncResponse::Error`. |

## Security Validation
No new external-data trust boundary introduced. `serve_state_root` reads local state only and returns a hash; `block_hash` request arg is ignored (`block_hash: _`) — serve always reflects the local tip, consistent with the legacy handler. No injection surface added. Out of scope for a dedicated security probe.

## Specs/Docs Drift
| File | Documented Behavior | Actual Behavior | Severity |
|---|---|---|---|
| `specs/state-root-commitment-architecture.md` (Migration steps 1–2) | Tier-0 memoize-on-compute seam, byte-identical root, eager compute retained until M2 | Matches implementation | none |

No drift found.

## Blocking Issues
None.

## Non-Blocking Observations
- **OBS-001**: The `Err` arm of `serve_state_root` (error not memoized) has no unit test because `compute_state_root` is infallible for valid in-memory state. Behavior is correct by inspection; a fault-injection test could be added in a later milestone if the compute path ever becomes fallible.

## Modules Not Validated
None in scope. The two `test_network` node tests that fail under macOS FD ulimit were NOT encountered (only the targeted M1 suites were run); they are environmental (FD exhaustion) and unrelated to M1.

## Final Verdict
**PASS** — All Must and Should acceptance criteria met. Served root is byte-identical to legacy; best_hash-keyed memo, cold/stale recompute-and-write-back, and vote-serve memoization all verified. All M2-deferred MUST-NOT items intact. Approved for review.

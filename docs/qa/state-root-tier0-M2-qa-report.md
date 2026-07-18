# QA Report: State-Root Lazy Tier-0 — Milestone M2 ("Eager-compute removal — the subtraction")

## Scope Validated
Branch `feature/state-root-lazy-tier0`. REDESIGN milestone whose thesis is: remove the eager per-block state-root compute + cache-publish while leaving the state-root **value** byte-identical. Files in scope:
- `bins/node/src/node/apply_block/state_update.rs` (eager Phase-2/3 compute deleted)
- `bins/node/src/node/apply_block/mod.rs` (new `state_fp_sr_field` helper + honest `[STATE_FP]` sr= field)
- `bins/node/src/node/mod.rs` (re-export)
- `crates/storage/src/mmr.rs` (REQ-SROOT-006 tombstone on `IncrementalStateRoot`)
- `crates/updater/src/hardfork.rs` (EPOCH_SNAPSHOT_HF PARKED note; 15→6 call-site count)
- `crates/storage/src/snapshot.rs` (15→6 call-site count)
- `specs/engine-parts.md` (live vs dead sync-request handler drift corrected)

## Summary
**PASS.** All five acceptance criteria met. The eager per-block compute+publish is gone (confirmed in the diff), and every state-root path still produces a byte-identical value: the byte-identity and golden-identity suites pass, the honest `sr=` field correctly refuses to print a stale prior-height root, all M1 memoize tests still pass, and no remaining reader of `cached_state_root` assumes eager population. Constraints hold: `CURRENT_PROTOCOL_VERSION` remains 8, no new activation height, no EpochState/consensus edits — the change alters only *when* the root is computed, never its bytes.

## System Entrypoint
Validation performed via the Rust test harness (`cargo test`), the appropriate entrypoint for a compute-timing refactor with no runtime/wire surface change. No node process startup required; the byte-identity suite exercises all four root-derivation paths in-process.

## Traceability Matrix Status
| Requirement ID | Priority | Has Tests | Tests Pass | Acceptance Met | Notes |
|---|---|---|---|---|---|
| REQ-SROOT-001 | Must | Yes | Yes | Yes | Served root == `compute_state_root` (byte-identity + golden suites) |
| REQ-SROOT-006 | Must | Yes | Yes | Yes | `IncrementalStateRoot` tombstoned; `producer_order_independent` + `equals_documented_formula` prove the canonical root is history-free |
| REQ-SROOT-007 | Must | Yes | Yes | Yes | snap-build / snap-install roots equal served root (byte-identity suite) |
| REQ-SROOT-008 | Must | Yes | Yes | Yes | Honest `sr=` field: None/matching/stale-hash/stale-hash-equal-height all covered |

### Gaps Found
None. Every in-scope requirement has passing tests and corresponding code.

## Acceptance Criteria Results

### AC-BYTE-IDENTITY (REQ-SROOT-001/007) — PASS
- `cargo test -p doli-node --test state_root_byte_identity_m2` → `test_all_state_root_paths_byte_identical ... ok` (1 passed).
- `cargo test -p storage --test state_root_golden_identity_test` → 5 passed, including `test_compute_state_root_equals_documented_formula`, `..._byte_stable_across_calls`, `..._producer_order_independent`.
- Served root, `storage::compute_state_root`, snap-build (`snapshot.rs`) and snap-install (`fork_recovery.rs`) roots are all byte-identical at a given height.

### AC-HONEST-SR (REQ-SROOT-008) — PASS
`cargo test -p doli-node --test state_root_state_fp_honest_m2` → 4 passed:
- `test_sr_field_none_memo_prints_none` (None → "none")
- `test_sr_field_matching_hash_prints_hex_prefix` (matching best_hash → 16-hex prefix)
- `test_sr_field_stale_hash_prints_none` (stale hash → "none")
- `test_sr_field_stale_hash_but_equal_height_still_none` (proves the decision keys on `best_hash`, never on the height field)

### AC-NO-EAGER-READER — PASS
Grep of all `cached_state_root` readers confirms none assume eager per-block population:
- `state_root_serve.rs` (live GetStateRoot) — reads memo, recomputes fresh on miss and writes back. Correct.
- `event_loop.rs:509` (DEAD `handle_sync_request_bg`, per corrected `specs/engine-parts.md`) — reads cache, falls back to fresh `compute_state_root` on `None`. Does not break when eager publish is gone.
- `fork_recovery.rs:342` — *writes* the cache after snap install (fresh compute). Correct.
- `apply_block/mod.rs:456` — honest `[STATE_FP]` reader via `state_fp_sr_field`; handles None/stale gracefully.

### AC-M1-REGRESSION — PASS
`cargo test -p doli-node --test state_root_memoize_m1` → 7 passed (cold-memo write-back, memo-hit no-recompute, stale-memo recompute/overwrite, legacy-value serve, repeat-quorum memo hit, canary smoke).

### AC-CONSTRAINTS — PASS
- `CURRENT_PROTOCOL_VERSION` = 8 (unchanged; `status.rs:49` + self-check `status.rs:344` assert == 8).
- No new activation height (`network_params.rs` not in the changeset).
- EpochState format untouched; `EPOCH_SNAPSHOT_HF` explicitly PARKED/unwired with an INC-I-054 note against any protocol bump.
- `git diff` of the seven files shows only compute-timing removal, the pure `state_fp_sr_field` helper, doc/tombstone/comment edits, and call-site-count corrections (15→6). No consensus-visible byte change.

## End-to-End Flow Results
| Flow | Steps | Result | Notes |
|---|---|---|---|
| Block-apply → lazy root serve | apply block (no eager publish) → GetStateRoot serve → memoize | PASS | Covered by memoize_m1 + byte_identity_m2 |
| Snap build/install root parity | build snapshot root vs install root vs compute | PASS | byte_identity_m2 asserts all four paths equal |

## Exploratory Testing Findings
None. This is a pure compute-timing refactor with no user-facing/wire surface; the honest-sr stale-hash-equal-height case (the one plausible silent-poison trap) is directly covered by a dedicated test.

## Failure Mode Validation
| Failure Scenario | Triggered | Detected | Recovered | Degraded OK | Notes |
|---|---|---|---|---|---|
| Stale/None memo mislabeled as current root in diagnostics | Yes (unit) | Yes | N/A | Yes | `state_fp_sr_field` prints "none" for None and any hash mismatch |
| Memo miss on GetStateRoot after eager publish removed | Yes (unit + code path) | Yes | Yes | Yes | serve path + dead bg handler both recompute fresh on miss |

## Security Validation
| Attack Surface | Test Performed | Result | Notes |
|---|---|---|---|
| Diagnostic-log divergence poison (misreported root) | Honest sr= unit suite | PASS | No stale root can be printed as current |
| State-root value tampering via mechanism swap | mmr tombstone + order-independent test | PASS | `IncrementalStateRoot` disqualified (history-dependent, breaks snap quorum); canonical root proven history-free |

No external-data ingestion surface introduced by this milestone.

## Specs/Docs Drift
| File | Documented Behavior | Actual Behavior | Severity |
|------|-------------------|-----------------|----------|
| `specs/engine-parts.md` | Now marks `handle_sync_request_bg` DEAD and `validation_checks.rs::handle_sync_request` as the LIVE path delegating to `serve_state_root()` | Matches code (`state_root_serve.rs`, `event_loop.rs`) | resolved (drift corrected this milestone) |
| `snapshot.rs` / `hardfork.rs` | "15 call-sites" → "6" | Matches current `compute_state_root` call-site count | resolved |

## Collateral Suites
- `cargo test -p storage --lib` → 245 passed, 0 failed.
- `cargo test -p updater --lib` → 36 passed, 0 failed.
- No parallel-resource artifacts observed; no re-runs needed.

## Blocking Issues
None.

## Non-Blocking Observations
- **OBS-001**: `event_loop.rs` `handle_sync_request_bg` `GetStateRoot` arm remains in-tree though `specs/engine-parts.md` now labels it DEAD. It is harmless (correct fresh-compute fallback) but is a future dead-code-removal candidate; out of scope for M2.

## Modules Not Validated
None within scope.

## Final Verdict
**PASS** — All Must (REQ-SROOT-001/006/007/008) acceptance criteria met. The eager per-block state-root compute is removed while the root value remains byte-identical across serve, compute, snap-build and snap-install paths. No blocking issues. Constraints (protocol version 8, no new activation height, EpochState untouched) confirmed. Approved for review.

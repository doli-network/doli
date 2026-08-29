# QA Report: INC-I-190/191 M3 [F1-dedup] — Finality attester de-duplication

## Scope Validated
`crates/core/src/finality.rs` (`FinalityTracker`, `PendingBlock`) and its 3 production
call sites: `production_gate.rs` wrapper, `network_events.rs::on_new_attestation`,
`startup.rs::create_and_broadcast_attestation`. Node-local finality only — no block
bytes, no state root, no version bump. Worktree: `.claude/worktrees/inc-i-190-191-finality-auth/`.

## Summary
PASS. The scalar attestation accumulator is replaced by `attesters: HashMap<PublicKey,u64>`;
the numerator is the sum of the map, so re-delivery of the same authenticated attester is an
idempotent overwrite that can no longer inflate finality weight. All 6 acceptance criteria
are met, both new tests pass, and no prior finality/attestation test regressed. All three
production call sites feed the map with a locally-derived per-attester weight, and both node
sites gate on positive weight before insertion.

## System Entrypoint
Library-level validation (finality is an in-process gadget, no running node required):
`cargo test -p doli-core --lib finality`, plus adjacent-crate regression sweeps.
Source of truth: reading `finality.rs` and the 3 call sites.

## Acceptance Criteria Results

### Must Requirements
#### AC1: A producer's weight counts AT MOST ONCE (dedup by attester pubkey)
- [x] PASS. `PendingBlock.attesters: HashMap<PublicKey,u64>`; `add_attestation_weight`
  does `pending.attesters.insert(attester, weight)` (overwrite). `numerator()` sums the
  map values. Verified by `test_duplicate_attester_counts_once` (same attester 3x -> 1).

#### AC2: Numerator can no longer exceed total_weight (checkpoint `attestation_weight <= total_weight`)
- [x] PASS. Each distinct producer contributes one map slot; sum of distinct
  per-producer weights cannot exceed the network total. `test_numerator_never_exceeds_total`
  drives 5 producers x weight 1 + one echo, asserts numerator == 5 (not 6) and the emitted
  checkpoint satisfies `attestation_weight <= total_weight`.

#### AC3: Distinct producers still accumulate correctly (K producers x w = K*w)
- [x] PASS. 5 distinct attesters x weight 1 -> numerator 5; the progression test
  (50 + 17 from two distinct producers -> 67) also confirms additive distinct accumulation.

#### AC4: Pre-track buffered attestations (early_attestations) dedupe and fold in on track_block
- [x] PASS. `early_attestations: HashMap<Hash, HashMap<PublicKey,u64>>`; buffered insert
  dedupes by attester; `track_block` does `early_attestations.remove(&hash)` and seeds the
  PendingBlock's `attesters`. `test_early_attestation_applied_on_track` covers the fold.

#### AC5: No liveness regression — genuine 67%+ at depth >=2 still finalizes (F2 intact)
- [x] PASS. `check_finality` uses `numerator()` and keeps the depth-2 gate
  (`applied_tip_height >= height + CONFIRMATION_DEPTH`). `test_normal_finality_at_depth2_no_liveness_regression`
  and `test_no_depth0_self_finality` both green; `FINALITY_THRESHOLD_PCT=67`,
  `CONFIRMATION_DEPTH=2` unchanged.

#### AC6: Two new tests pass; all prior finality/attestation tests stay green
- [x] PASS. `cargo test -p doli-core --lib finality` -> 12 passed / 0 failed
  (incl. `test_duplicate_attester_counts_once`, `test_numerator_never_exceeds_total`).

## End-to-End / Call-Site Flow Results
| Flow | Attester key source | Weight guard before insert | Result |
|---|---|---|---|
| Self-attestation (startup) | own `public_key` | `w == 0 -> return None` | PASS |
| Gossip echo (network_events) | `attestation.attester` | `if weight > 0` | PASS |
| production_gate wrapper | passthrough | (guarded by callers) | PASS |

Both node sites derive weight via `derive_attester_weight(&producers, attester, block_local_height)`
against the LOCAL ProducerSet (INC-I-191 [F1]) — non-members return `None` and are dropped.
Exactly these two production callers exist (grep confirms; the third reference is the wrapper,
the rest are tests).

## Exploratory Testing Findings
| # | What Was Tried (reasoned) | Expected | Actual | Severity |
|---|---|---|---|---|
| E1 | Same attester delivered twice with a DIFFERENT derived weight (e.g. epoch boundary crosses between deliveries) | dedup holds, one value counted | `insert` overwrites -> LATEST delivery's weight wins; still counted once. Correct given [F1] derives weight per-delivery from the freshest local ProducerSet; node-local + self-healing | low |
| E2 | Early-buffer growth / eviction cap | bounded memory | Cap `MAX_EARLY_ATTESTATIONS=100` now bounds DISTINCT BLOCK HASHES; inner map bounded by producer count (P). Worst case ~100*P entries (vs 100 scalars before) — still bounded (~4MB at P=1000). Arbitrary eviction can drop a soon-to-be-tracked block's buffer, but that is pre-existing behavior, re-broadcast repopulates, and it only affects local finality liveness marginally | low |
| E3 | Zero-weight (fully-delegated) attester reaches the map | dropped, no slot consumed | Dropped upstream at BOTH node sites (`weight > 0` / `w == 0 -> None`). The tracker itself would insert a harmless `(attester, 0)` (sum unaffected) but no unguarded production path reaches it | PASS |

## Failure Mode Validation
| Failure Scenario | Triggered | Detected | Degraded OK | Notes |
|---|---|---|---|---|
| Duplicate/echo attestation inflates numerator (root cause: 5/5, 6/5 vs total 5) | Yes (unit) | Yes | Yes | Numerator now equals distinct-producer weight; `attestation_weight <= total_weight` holds |
| Self-attestation + gossip echo double-count | Yes (unit) | Yes | Yes | Same pubkey overwrite is idempotent |
| Non-member forged attestation gains weight | Yes (node test) | Yes | Yes | `test_forged_nonmember_attestation_does_not_gain_finality_weight` green |

## Security Validation
| Attack Surface | Test Performed | Result | Notes |
|---|---|---|---|
| Wire self-declared `attester_weight` spoof | code read of both node sites | PASS | Weight is DERIVED locally, wire value never trusted (INC-I-191 [F1]) |
| Non-producer attestation | `test_forged_nonmember_attestation_does_not_gain_finality_weight` | PASS | `derive_attester_weight` returns `None` -> dropped |
| Re-broadcast flooding to inflate finality | dedup by pubkey | PASS | Overwrite is idempotent; flood cannot raise numerator |

## Specs/Docs Drift
None observed for M3 — finality remains node-local (not in state root), consistent with the
CLAUDE.md/spec description. No consensus rule or block-content change; no version bump required.

## Regression Sweeps
- `cargo test -p doli-core --lib finality` -> 12 passed / 0 failed
- `cargo test -p doli-core --lib` -> 992 passed / 0 failed
- `cargo test -p network --lib` -> 524 passed / 0 failed / 1 ignored (pre-existing)
- `cargo test -p doli-node --lib` -> 74 passed / 0 failed (incl. attestation_authority_tests)

## Blocking Issues (must fix before merge)
None.

## Non-Blocking Observations
- **OBS-1 (E1)**: overwrite uses the latest per-delivery derived weight. Correct and safe
  (node-local, self-healing), but worth a one-line code comment documenting the "latest-wins"
  intent so a future reader does not mistake it for a bug.
- **OBS-2 (E2)**: buffer memory footprint grew from 100 scalars to ~100*P entries; bounded and
  acceptable, but note it in the finality memory-budget accounting if mainnet P grows large.

## Final Verdict
**PASS** — All 6 acceptance criteria met. Dedup by attester pubkey is correct, the numerator
can no longer exceed total_weight, distinct producers still accumulate, the early buffer
dedupes and folds, the F2 depth-2 liveness gate is intact, and both new tests plus all prior
finality/attestation/network/node tests are green. Approved for review. Two low-severity,
non-blocking observations recorded.

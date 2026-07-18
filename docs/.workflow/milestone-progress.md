# Milestone Progress — Tier 0 SSF (lazy state root)

Workflow: `omega-redesign --fix` · RUN_ID=459 · Branch: `feature/state-root-lazy-tier0`
Spec: `specs/state-root-commitment-architecture.md` (Tier 0, VERDICT GO conf 0.97)
Deploy safety: consensus RULES? NO · block CONTENT? NO → rolling-safe, no AH, no version bump.

| ID | Name | Scope (modules) | Requirements | Deps | Status |
|----|------|-----------------|--------------|------|--------|
| M1 | Memo + canary (behavior-additive) | `bins/node/src/node/state_root_serve.rs` (NEW serve_state_root memoize seam), `bins/node/src/node/validation_checks.rs` (live GetStateRoot delegates), `crates/storage/src/snapshot.rs` (log_state_root_components canary seam) | REQ-SROOT-001/002/007; F-D0-2 | — | COMPLETE (2026-07-18) |
| M2 | Eager-compute removal (the subtraction) | `bins/node/src/node/apply_block/state_update.rs` (deleted eager Phase-2/3 compute+publish), `bins/node/src/node/apply_block/mod.rs` (honest best_hash-keyed `[STATE_FP]` sr= via pure `state_fp_sr_field` helper), `crates/storage/src/mmr.rs` (IncrementalStateRoot tombstoned), `crates/updater/src/hardfork.rs` (EPOCH_SNAPSHOT_HF PARKED comment), `crates/storage/src/snapshot.rs` + `specs/engine-parts.md` (comment/drift housekeeping) | REQ-SROOT-001/006/007/008; F-D0-2 | M1 | COMPLETE (2026-07-18) |

## Notes / invariants the runner MUST honor
- **Formula byte-identical** at all heights — this is a `redesign` (regression tests lock current root value BEFORE change; golden identity lazy==legacy).
- **Lock ordering** (M1): in the live handler write-back, DROP chain_state/utxo/producer read guards BEFORE taking the `cached_state_root` write guard (leaf lock; mirror `state_update.rs:135-146`).
- **Only ONE live handler** needs cache-on-compute: `validation_checks.rs:1093-1122`. The `_bg` handler (`event_loop.rs:394-531`) is dead (`#[allow(dead_code)]`) — leave it (or note disposition), do not wire.
- **M2 scope-completeness**: the `[STATE_FP]` `sr=` fix (`apply_block/mod.rs:427-435`) MUST land in the SAME commit as the eager-compute deletion — never after (else stale/none `sr=` at wrong height).
- **NEVER** bump `CURRENT_PROTOCOL_VERSION` (frozen at 8) or touch `EpochState` format (INC-I-054). No `EPOCH_SNAPSHOT_HF` wiring.
- Required tests: golden identity (lazy==legacy per height); memo staleness keyed on `best_hash`; quorum-vote serve on cold memo computes fresh + memoizes; (M2) `[STATE_FP]` stale-`sr=` regression.
- Build gate: `cargo build --release && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`; tests `cargo test -p doli-storage` + `-p` node crate. Local only — do NOT deploy.

## M1 close-out (2026-07-18)
- Implemented `Node::serve_state_root()` memoize-on-compute (new `state_root_serve.rs`); live `GetStateRoot` handler delegates. Leaf-lock write-back (read guards dropped before `cached_state_root` write). Eager compute at `state_update.rs:135-146` left INTACT (additive M1).
- Canary seam `log_state_root_components()` added in `snapshot.rs` (behavior-neutral; retains per-block `[STATE_FP]` scheduler_root + memoized-or-none `sr=`; `sr=` field fix deferred to M2 per scope).
- Tests: `bins/node/tests/state_root_memoize_m1.rs` (7), `crates/storage/tests/state_root_golden_identity_test.rs` (5) — all green. Full affected-crate suites (storage 245 lib + doli-node) green; two extreme in-process scale tests (`test_onchain_liveness_10k_nodes`, `test_cluster_10x100`) pass in isolation but are FD-capacity-bound on this host, verified separately.
- QA: PASS (`docs/qa/state-root-tier0-M1-qa-report.md`). Review: OK, Security Audit Verdict = AUDIT-SKIP (`docs/reviews/state-root-tier0-M1-redesign-review.md`) → 5-auditor sweep skipped.
- Root value byte-identical; no AH; `CURRENT_PROTOCOL_VERSION` stays 8; `EpochState`/`EPOCH_SNAPSHOT_HF` untouched. Committed local (branch `feature/state-root-lazy-tier0`), NOT pushed/deployed.

## M2 close-out (2026-07-18)
- Deleted the eager per-block state-root compute+publish (`apply_block/state_update.rs` Phase-2/3) — the root is now FULLY lazy (memoized on serve since M1; snap build/install compute fresh). Root VALUE byte-identical at every height.
- Fixed the orphaned `[STATE_FP]` `sr=` reader (`apply_block/mod.rs`) in the SAME commit: added pure helper `state_fp_sr_field(memo, current_block_hash) -> String` (re-exported `doli_node::node::state_fp_sr_field`). Honest semantics: `None`/stale-hash → "none"; only prints the 16-hex root when the memo's stored best_hash == the just-applied block hash. Never mislabels a prior-height root as current.
- `IncrementalStateRoot` tombstoned in `crates/storage/src/mmr.rs` (disqualified per REQ-SROOT-006: creation-order + XOR ⇒ snap-synced nodes cannot reconstruct); struct + tests retained (zero non-test callers). `EPOCH_SNAPSHOT_HF` given a PARKED comment (never bump its planned version, INC-I-054) — NOT deleted, version unchanged, unwired. Housekeeping: "15"→"6" call-site comments in `hardfork.rs`/`snapshot.rs`; `specs/engine-parts.md` live/dead sync-request handler drift corrected (`_bg` DEAD, `validation_checks.rs`→`serve_state_root` LIVE).
- Tests: `bins/node/tests/state_root_state_fp_honest_m2.rs` (4, stale-`sr=` regression), `bins/node/tests/state_root_byte_identity_m2.rs` (1, four-path byte-identity lock). Green captured: storage lib 245 + updater lib 36 + golden 5 + memoize-M1 7 + byte-identity 1 + honest-sr 4 = 298 passing (0 failed).
- QA: PASS (`docs/qa/state-root-tier0-M2-qa-report.md`). Review: OK, Security Audit Verdict = AUDIT-SKIP (`docs/reviews/state-root-tier0-M2-redesign-review.md`) → 5-auditor sweep skipped.
- `CURRENT_PROTOCOL_VERSION` stays 8; no AH; `EpochState` untouched. Consensus-shape (INC-I-075): change alters only WHEN the root is computed — bit-identical for all reachable inputs (Q3=YES) ⇒ no activation height. Committed local (branch `feature/state-root-lazy-tier0`), NOT pushed/deployed per payload.

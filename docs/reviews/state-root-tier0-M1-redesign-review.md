# Code Review: State-Root Lazy Tier-0 — M1 (REDESIGN, additive) — RUN 459

**Status:** OK (approved for merge)
**Security Audit Verdict:** AUDIT-SKIP
**Verdict:** M1 is correct and behavior-additive — memo is best_hash-keyed (no stale serve), lock ordering mirrors the eager path, root value is byte-identical, and the only budget notes are pre-existing (not introduced by M1).

## Scope Reviewed
- `bins/node/src/node/state_root_serve.rs` (NEW, 76 lines)
- `bins/node/src/node/validation_checks.rs` (GetStateRoot arm delegation)
- `bins/node/src/node/mod.rs` (`mod state_root_serve;`)
- `crates/storage/src/snapshot.rs` (`log_state_root_components` seam)
- `bins/node/tests/state_root_memoize_m1.rs`, `crates/storage/tests/state_root_golden_identity_test.rs`
- Mechanical clippy `--fix` files: `checkpoint_health.rs` (test mod), `block_store/tests.rs`, `state_db/tests.rs`, `inc_i_136_checkpoint_health_test.rs`

## Summary
Approved for merge. No blocking issues. All five review goals satisfied.

## Goal-by-Goal Findings

### 1. Memoize write-back correctness — PASS
- Location: `state_root_serve.rs:33-74`. Served root comes from `storage::compute_state_root(&chain_state, &utxo_set, &ps)` (line 55) — the identical function/argument triple used by the eager path (`apply_block/state_update.rs:139`). Golden-identity test locks the formula `H(H(cs)||H(utxo)||H(ps))`, determinism, producer-order independence, and per-component sensitivity. Byte-identity holds for all reachable inputs.
- Memo keying: Fast path returns cached only when `hash == self.chain_state.best_hash` (lines 37-45). A prior-height tuple falls through to recompute — proven by `test_stale_memo_not_served_recomputes` and `test_stale_memo_overwritten_with_current`.
- Error path safe: `Err(e) => SyncResponse::Error(...)` (line 72) does NOT write the cache — no poisoned/partial memo.
- Confidence: conf(0.95, observed).

### 2. Lock ordering — PASS
- Location: `state_root_serve.rs:49-64` vs. `apply_block/state_update.rs:135-146`. The three read guards (`chain_state`, `utxo_set`, `producer_set`) are scoped in an inner block (lines 49-59) that ends before the `cached_state_root` write guard is taken (line 63) — exactly mirrors the eager path's Phase-2/Phase-3 split. `cached_state_root` is a leaf lock; no two guards are ever held simultaneously → no lock-order deadlock against apply-write.
- Mutual exclusion: R3 (`handle_sync_request`) runs on the same single event-loop actor as block apply, so apply-write and memoize-write are mutually exclusive by construction.
- Confidence: conf(0.9, observed).

### 3. No unintended behavior change — PASS
- Eager compute at `state_update.rs:135-146` still present and intact.
- `CURRENT_PROTOCOL_VERSION` still `8`.
- No activation height added; `EPOCH_SNAPSHOT_HF`/`compute_state_root_with_epoch_state` untouched.
- `[STATE_ROOT]` output preserved byte-for-byte via `log_state_root_components` (`snapshot.rs:69-86`).
- `[STATE_FP] sr=` reader (`apply_block/mod.rs:428`) untouched — M2 scope.

### 4. Modular size
- `state_root_serve.rs` = 76 lines OK.
- Pre-existing (not introduced by M1): `validation_checks.rs` = 1188 lines and `snapshot.rs` = 598 lines exceed the 500-line budget. M1 *reduced* `validation_checks.rs` and added only ~18 lines to `snapshot.rs`. Flag for a future dedicated split; not blocking.

### 5. Specs/docs drift
- `specs/state-root-commitment-architecture.md` accurate for M1 (scopes Migration steps 1-2, identifies R3 as the LIVE handler, states drop-then-write requirement).
- Pre-existing drift already flagged by the spec: `specs/engine-parts.md:2738,2812` claims production uses `handle_sync_request_bg` (backwards; live path is `handle_sync_request`). Predates M1, out of scope, recommend a one-line docs fix.

## Verification of clippy `--fix` changes
Confirmed test-only and mechanical. `assert_eq!(x, bool)` → `assert!(x)` is behavior-neutral.

## Consensus-Shape Three-Question Checklist
1. User-submittable tx triggers this path? Reached via peer `SyncRequest::GetStateRoot`, but `block_hash` is ignored (`block_hash: _`) — no attacker-controlled data influences the served value.
2. Producer/attestation pattern triggers it? Snap-sync quorum peers request it; spamming only causes more O(1) memo hits — the memo *reduces* DoS surface.
3. Bit-identical to old behavior? YES for the served root VALUE (golden test). Only *when* compute happens changes; the stale-guard makes the served (hash,root) pair strictly more correct.
Since (3) is YES for all reachable inputs, no activation height is required.

## Security Audit Verdict
```
━━━ SECURITY AUDIT VERDICT ━━━
Verdict: AUDIT-SKIP
Signals: none that change served behavior — the request's block_hash is ignored, output is
byte-identical to the legacy compute (golden-locked), no untrusted-byte parsing is added, no new
consensus surface, no crypto/auth change. The path serves snap-sync quorum peers, but the memo is
best_hash-keyed and stale-guarded (verified), so no attacker-reachable input can alter the served
value; the DoS surface is reduced, not expanded.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Final Verdict
Approved for merge. No P0/P1 findings. Two informational items (pre-existing oversized files; pre-existing `engine-parts.md` liveness drift) — both outside M1's making and non-blocking.

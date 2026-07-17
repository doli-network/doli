# QA Report: Disk Guardian M1 — Fail-safe foreground writes

- **Workflow**: new-feature, run_id 458
- **Milestone**: M1 (REQ-DISK-101..106 ACTIVE; REQ-DISK-001..012 SUPERSEDED, out of scope)
- **Verdict**: **PASS**

## Scope Validated
Conversion of seven `StateDb` direct-write methods from panic-on-failure to
`Result<_, StorageError>` so a disk-full (ENOSPC) write surfaces `StorageError`
instead of SIGABRT. Files in scope: `crates/storage/src/state_db/{writes.rs,undo.rs}`,
`crates/storage/src/utxo/set.rs`, `crates/storage/src/lib.rs`,
`crates/storage/src/utxo/types.rs`, `bins/node/src/node/init.rs`,
`bins/node/src/operations/chain.rs`, test `crates/storage/tests/disk_guardian_failsafe_test.rs`.

## Summary
All five verification objectives pass. The success path is provably bit-identical
(diffs are line-local `.expect(...)` → `?`/`Ok(...)`; the state-root bit-identity
test passes; 245 storage lib tests green). The failure path returns
`Err(StorageError::Database(_))` instead of aborting, proven by seven read-only-handle
tests. Callers in `init.rs`, `chain.rs`, and `utxo/set.rs` propagate via `?` and
`doli-node` builds clean. No consensus/version constant was touched and `panic = "abort"`
is retained. `apply_block.rs` and sync hot paths were not modified.

## System Entrypoint
Component-level validation (storage crate + node build). No running node required for
M1 — the read-only RocksDB handle (`from_readonly_for_test`) is a deterministic,
portable ENOSPC stand-in. Commands:
- `cargo test -p storage --test disk_guardian_failsafe_test`
- `cargo test -p storage --lib`
- `cargo build -p doli-node`

## Acceptance Criteria Results (ACTIVE Must/Should/Could)

### REQ-DISK-101 (Must) — write methods return Result, no `.expect` on RocksDB write ops
- [x] `insert_utxo`, `remove_utxo`, `import_utxos`, `add_transaction`, `clear_and_write_genesis`, `put_undo`, `clear_utxos` all return `Result` — PASS
- [x] `.expect(` remaining in non-test `state_db` code are only allowlisted `bincode::serialize(...).expect(...)`; zero on `db.write`/`db.put_cf` — PASS
- [x] Failure-path tests (`*_on_failing_db_returns_err_not_panic`) return `Err` not panic — PASS

### REQ-DISK-102 (Must) — success path bit-identical
- [x] Diffs are line-local (`.expect(...)`/`let _ =` → `?`/`Ok`) — PASS
- [x] `state_root_is_bit_identical_across_equivalent_sequences` passes; 245 storage lib tests pass with no success-path expectation changes — PASS
- [x] No `CURRENT_PROTOCOL_VERSION` / `EPOCH_STATE_FORMAT_VERSION` / `MIN_PEER_PROTOCOL_VERSION` bump, no activation height, no Cargo version bump — PASS

### REQ-DISK-103 (Must) — callers propagate
- [x] `utxo/set.rs` 4 RocksDb arms propagate via `?` — PASS
- [x] `init.rs:233,369` and `chain.rs:133,138` propagate via `?`; `doli-node` builds — PASS
- [x] Rollback path (`truncate_chain`) surfaces `Err`; `rollback_wrapper_insert_and_remove_surface_err` passes — PASS

### REQ-DISK-104 (Must) — clean error, never SIGABRT
- [x] Read-only-handle unit proxy for ENOSPC returns `Err(StorageError::Database(_))` for all seven methods — PASS
- [ ] Gauntlet disk-full scenario is system-level, out of scope for this component pass — deferred to system validation (not an M1 blocker)

### REQ-DISK-105 (Should) — `clear_utxos` no longer swallows
- [x] `let _ = self.db.write(batch)` replaced with `self.db.write(batch)?`; returns `Result`; `clear_utxos_on_failing_db_returns_err_not_panic` passes — PASS

### REQ-DISK-106 (Could) — actionable startup error
- Not implemented in this change set; `init.rs` propagates the `StorageError` upward. Deliberate deferral consistent with Could priority — not a blocker.

## Traceability Matrix Status
| Requirement | Priority | Has Tests | Tests Pass | Acceptance Met |
|---|---|---|---|---|
| REQ-DISK-101 | Must | Yes | Yes | Yes |
| REQ-DISK-102 | Must | Yes | Yes | Yes |
| REQ-DISK-103 | Must | Yes | Yes | Yes |
| REQ-DISK-104 | Must | Yes (unit proxy) | Yes | Yes (system gauntlet deferred) |
| REQ-DISK-105 | Should | Yes | Yes | Yes |
| REQ-DISK-106 | Could | No | n/a | Deferred (Could) |

## Test Execution
- `disk_guardian_failsafe_test`: **19 passed, 0 failed** (incl. bit-identity + 7 failing-db Err tests)
- `cargo test -p storage --lib`: **245 passed, 0 failed**
- `cargo build -p doli-node`: **clean**

## Regression Validation
- `git diff` does NOT touch `bins/node/src/node/apply_block.rs` — confirmed.
- No sync hot-path change: only `crates/network/src/sync/manager/tests_inc_i139.rs` appears, and it is a rustfmt reflow of a test helper signature (benign, no logic change).
- Adjacent test files (`state_db/tests.rs`, `utxo/tests.rs`, `inc_i_064`, `inc_i_071`) are mechanical `.unwrap()`/`?` call-site updates to the new signatures. No behavioral change.
- `spend_transaction` left untouched — confirmed.

## Version / Deploy Safety
- No version-constant bump anywhere in the change set (diff matches are documentation context lines only).
- `panic = "abort"` retained at `Cargo.toml:120`.
- No consensus rule, block content, activation height, or wire-format change.

## Non-Blocking Observations
- **OBS-001**: `cargo clippy -p storage --tests -- -D warnings` fails with ~14 `bool_assert_comparison` errors in `block_store/tests.rs` and `state_db/tests.rs` (lines 1208–1304). Verified **pre-existing debt at HEAD**: the M1 diff only added `.unwrap()` to the `put_undo(...)` lines above the offending `assert_eq!(db.has_undo_data(...), true/false)` asserts; it did not introduce or modify any bool-literal assert. The canonical gate `cargo clippy --workspace -- -D warnings` is green. Not an M1 issue.

## Blocking Issues
None.

## Final Verdict
**PASS** — All Must (REQ-DISK-101..104) and Should (REQ-DISK-105) acceptance criteria met; the critical success-path bit-identity invariant holds; failure path returns `Err` instead of SIGABRT; callers propagate; no version bump; `panic = "abort"` retained. REQ-DISK-104 system-level gauntlet and REQ-DISK-106 (Could) are deferred, non-blocking. Approved for review.

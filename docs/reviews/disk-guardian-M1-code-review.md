# Code Review — Disk Guardian M1: Fail-safe foreground writes

- Workflow: new-feature (run_id 458)
- Milestone: M1 — "Fail-safe foreground writes (disk-full returns an error instead of SIGABRT)"
- Architecture: `specs/disk-guardian-architecture.md` §D1
- Requirements: `specs/disk-guardian-requirements.md` (ACTIVE REQ-DISK-101..106)
- Verdict: **APPROVED**
- Security Audit Verdict: **AUDIT-SKIP**

## Scope reviewed (git diff)

- `crates/storage/src/state_db/writes.rs` — `insert_utxo`, `remove_utxo`, `clear_utxos`,
  `clear_and_write_genesis`, `import_utxos`, `add_transaction` converted from
  `.expect("RocksDB write batch")` / silent `let _ = db.write(...)` to `?`-propagated
  `Result<_, StorageError>`. `spend_transaction` untouched (already Result).
- `crates/storage/src/state_db/undo.rs` — `put_undo` → `Result` (`.expect` → `?`);
  in-memory `bincode::serialize(...).expect(...)` retained per architecture.
- `crates/storage/src/utxo/set.rs` — 4 `RocksDb` wrapper arms propagate `?` (signatures unchanged).
- `crates/storage/src/lib.rs` — added `impl PartialEq for StorageError` (test-only, additive;
  `Io(std::io::Error)` variant blocks a derive).
- `crates/storage/src/utxo/types.rs` — added `PartialEq` derive to `UtxoEntry` (test assertions).
- `bins/node/src/node/init.rs` (L233, L369), `bins/node/src/operations/chain.rs` (L133, L138) —
  callers add `?`; enclosing functions already return `Result`/`anyhow::Result` accepting `StorageError`.
- `crates/storage/src/state_db/open.rs` — `#[doc(hidden)] pub from_readonly_for_test` test constructor.
- `crates/storage/tests/disk_guardian_failsafe_test.rs` — 19 tests.

## Findings (all checks pass)

1. **Root goal achieved.** A failing `db.write`/`put_cf` now returns `Err(StorageError)` instead of
   aborting the process (`panic = "abort"`). Non-consensus robustness fix.
2. **Success path BIT-IDENTICAL.** Each method builds the identical `WriteBatch` in the identical
   order; counter and state-root updates remain post-write and unchanged. The only behavioral delta
   is the failure path (`Err` vs abort). No reordering, no semantic change. Confirmed by the
   `state_root_is_bit_identical_across_equivalent_sequences` test.
3. **No scope creep.** `apply_block`, sync, attestation, consensus, and `NetworkParams` untouched.
   `panic = "abort"` retained (`Cargo.toml:120`). No version bumps
   (CURRENT_PROTOCOL_VERSION / EPOCH_STATE_FORMAT_VERSION / MIN_PEER_PROTOCOL_VERSION / Cargo).
   No new activation height.
4. **Error handling sound.** `?` propagation correct at every caller (init.rs 233/369,
   chain.rs 133/138, set.rs 4 arms); errors are surfaced, not swallowed. `remove_utxo`'s
   `Option` → `Result<Option>` conversion preserves `None` semantics (returns `Ok(None)` on
   absent / deserialize-fail).
5. **PartialEq additions safe.** `StorageError` and `UtxoEntry` PartialEq are additive and used only
   for test assertions; `Serialize`/`Deserialize` untouched, no cascade (`Output` already PartialEq).
6. **Specs/docs aligned.** §D1 change table and the requirements traceability matrix reflect what shipped.

## Path-Coverage assessment

`remove_utxo`'s two `return Ok(None)` lines are re-expressions of pre-existing `None`
short-circuits, structurally required because the function returns the removed entry and its return
type changed from `Option<UtxoEntry>` to `Result<Option<UtxoEntry>, StorageError>`. Same inputs →
same absent-handling result. **Not new early-return Err guards; Path-Coverage not required.**

## Non-blocking note

`from_readonly_for_test` is `#[doc(hidden)] pub` but not `cfg(test)`-gated — consistent with existing
open.rs test helpers, negligible.

## Security Audit Verdict

**AUDIT-SKIP** — the change is internal DB-error plumbing with a bit-identical success path and
processes no external/untrusted input; no new attack surface.

# INC-I-308 M2 — effect tests and handler fixes (run 732, parent 4cf8be85)

Probe (from the worktree root):

```
cargo test -p crypto --lib resolve_multibyte
cargo test -p rpc --test it audit_rpc_effects
```

Parent: crypto 1 failed; rpc 4 failed, 1 passed (`audit_p1_rpc_004_prune_with_force_runs` guards the override and is green on both).
Fix: crypto 1 passed; rpc 5 passed.

## AUDIT-P1-RPC-008 — multi-byte address aborts the node — REAL

- Tests: `crates/crypto/src/address.rs::tests::resolve_multibyte_at_byte_20_is_err_not_panic`;
  `crates/rpc/tests/it/audit_rpc_effects.rs::audit_p1_rpc_008_multibyte_address_returns_invalid_params` (getBalance, getUtxos, getHistory).
- Red on parent: `panicked at crates/crypto/src/address.rs:162:38: end byte index 20 is not a char boundary; it is inside 'é' (bytes 19..21)`.
- Green: both `... ok`; each method returns -32602.
- Fix: the error-message preview counts and takes 20 chars (`chars().take(20)`), not 20 bytes (`address.rs:161-162`).
- Does block/tx validation call this file? No. `grep -rnE "address::resolve|crypto::address|\bresolve\(" crates/core/src/validation bins/node/src/node/apply_block bins/node/src/node/validation_checks crates/mempool/src` -> no match (exit 1). Callers: RPC `context.rs:394`, CLI (`cmd_channel.rs`, `parsers.rs`, `cmd_wallet.rs`, `cmd_nft/transfer.rs`). The fix changes only error text; the set of strings that resolve is unchanged.

## AUDIT-P1-RPC-004 — pruneBlocks deletes history the archive does not cover — REAL

- Tests: `audit_p1_rpc_004_prune_without_covering_archive_keeps_blocks`, `audit_p1_rpc_004_prune_with_force_runs`.
- Red on parent: `h1 pruned without a covering archive: {"result":{"archive_verified":false,"chain_tip":2100,"keep_last_n":2000,"lowest_remaining_height":100,"pruned":5,"status":"ok"}}`.
- Green: both `... ok`.
- Fix (`pruning.rs`): params `[keep_last_n, force]`; the coverage check reads `archive_dir` (`--archive-to`), and `data_dir/archive` only when `archive_dir` is None; no coverage and no `force` -> -32603 error, nothing deleted. Reply shape unchanged for covered/forced.
- Does block/tx validation call this file? No — RPC handler only. No bins/node test calls the pruneBlocks handler; they call `BlockStore::prune_blocks_below` directly (unaffected).

## AUDIT-P1-RPC-007 — repairArchiveFromPeer writes peer-chosen bytes for a peer-chosen range — REAL

- Test: `audit_p1_rpc_007_repair_archive_rejects_garbage_and_clamps_to_local_tip`.
- Red on parent: `above-tip archive file ".../arch/0000000003.block" written: {"result":{"already_present":0,"errors":0,"fetched":5,"peer_tip":5,"status":"ok"}}`.
- Green: `... ok`.
- Fix (`guardian.rs` + new `guardian_repair_check.rs`): loop `1..=peer_tip.min(local_tip)`; every payload must decode (`deserialize_block_compat`); if the local store has a hash at h it must match, else `verify_block_signature_at` + `body_matches_header` must pass; a refused block counts in `errors`. An empty blake3 skips only the checksum compare, never the decode/identity checks.
- Does block/tx validation call this file? No — RPC handler only.

## AUDIT-P1-RPC-005 — backfillFromPeer stores peer-authored blocks — REAL

- Test: `audit_p1_rpc_005_backfill_refuses_block_contradicting_stored_neighbor` (honest h1, h2, h4 stored; peer serves self-signed forged h3 with prev = honest h2).
- Red on parent: `h3 Some(Hash(a01e06ea...)) contradicts stored h4.prev_hash Hash(f6be4313...); ... status {"imported":1,"running":false,"total":1}`.
- Green: `... ok`; existing `req_auth_019_p13_backfill_forged_not_stored` passes unchanged.
- Fix (`backfill.rs` + new `backfill_neighbor.rs`): before `put_block_canonical`, refuse (skip, like a bad signature) when the height index already holds a different hash outside the repair set, when stored h+1 has a different `prev_hash` (unless h+1 is also being repaired), or when the fetched `prev_hash` differs from stored h-1.
- Does block/tx validation call this file? No — RPC handler only.

## Green gate (worktree, with concurrent M1 edits present)

- `cargo test -p crypto`: 132 + 8 + 7 passed, 0 failed.
- `cargo test --no-fail-fast -p rpc`: lib 113, inc_i_173_m3 8, inc_i_236_m2_backfill 1, inc_i_281_ma_backfill 1, it 32 passed (1 ignored), 0 failed.
- `cargo test -p doli-node --test it -- inc_i_300_f inc_i_248_rel_m2`: 29 passed, 0 failed (developer run).
- `cargo clippy -p crypto -p rpc --all-targets -- -D warnings`: clean. `rustfmt --check` on the 7 M2 files: clean.

Consensus shape: (1) no (2) no (3) n/a — RPC handlers and an error-message preview only. No activation height, no version bump.

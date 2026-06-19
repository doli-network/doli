# INC-I-118 Diagnosis — Snap-synced node freezes at first post-snap epoch boundary

**Status:** RESOLVED · fix applied + verified FAIL→PASS on 2026-06-19 · **Feasibility: CODE-FIXABLE**
**Confidence:** conf(0.97, measured) — 5/5 investigator convergence on live testnet (n6/n10); full causal chain independently re-verified against code on 2026-06-18 resume.

> The original `docs/.workflow/*` investigation reports were deleted from disk before this resume. Findings are preserved in `.omega/memory.db` (incident_entries 1160–1176) and re-grounded against the code below. The synthesis report cannot be regenerated in `docs/.workflow/` (source investigation files gone), so the verified diagnosis lives here.

## Symptom
Snap-synced nodes derive an incomplete reward-pool balance. At the first post-snap epoch boundary the canonical full-epoch `EpochReward` exceeds the node's locally-seen pool, so validation rejects the block as an inflation attack (`[ECON_EPOCH_OVERFLOW]`) and the node freezes. Six nodes stuck; n6 cycled through 5 snap→fail loops.

## Root cause (M1): stale InMemory `UtxoSet` after snap install
A snap-synced node leaves `self.utxo_set` as a frozen in-memory copy of the snapshot. Post-snap blocks write UTXO changes only to `state_db`, never to that in-memory copy, so every consensus read that routes through `self.utxo_set` (pool balance, state root) sees snapshot-time data permanently.

### Causal chain (each link read in code this session)
| # | Mechanism | Citation |
|---|-----------|----------|
| 1 | Snapshot deserializes into the **InMemory** backend variant | `crates/storage/src/utxo/set.rs:440` (`deserialize_canonical` → `InMemoryUtxoStore::new()`); enum `set.rs:32-36` |
| 2 | Snap install assigns InMemory set to `self.utxo_set`, never converts to RocksDb | `bins/node/src/node/fork_recovery.rs:386-387, 435` |
| 3 | Snapshot UTXOs persisted to `state_db` via `atomic_replace`, but `self.utxo_set` stays InMemory | `fork_recovery.rs:447-450` |
| 4 | Post-snap `apply_block` writes UTXOs via `BlockBatch → state_db` only — no `self.utxo_set` write | `apply_block/mod.rs:192` ("Phase 3: … no utxo_store writes"); grep for `self.utxo_set` writes in `apply_block/` is empty |
| 5 | Epoch-boundary check reads pool balance from `self.utxo_set` (frozen InMemory) | `validation_checks.rs:~676` (`self.utxo_set.read().await` → `get_by_pubkey_hash(&pool_hash)`) |
| 6 | Frozen pool < canonical full-epoch reward → bail | `validation_checks.rs` `[ECON_EPOCH_OVERFLOW] … inflation attack` |

A continuous node holds the `RocksDb` variant (`init.rs:305` `UtxoSet::from_state_db`) whose reads route through `state_db` (`set.rs:96-113`). A snap-synced node holds the `InMemory` variant. **That backend mismatch is the entire divergence.**

### Measured evidence (live testnet)
- n6: snap h=2954 (pool drained h=2952) → frozen pool 0.3 = exactly 3 × 0.1.
- n10: snap h=2943 (drain h=2916) → frozen pool 2.8 = exactly 28 × 0.1.
- All 6 stuck nodes: `utxo_bytes=40089` frozen from snap install onward.
- M2 (restart) / M3 (truncated snapshot — root-check passed) / M4 (snap height ≠ drain point) eliminated.

## Regression commit
`632045f2` (storage **Phase 3**), which removed the `self.utxo_set` write path from `apply_block`. User-suspected `cd4645a` (Phase 2) is the *start* of the storage-backend migration series; the break lands 2 commits later at Phase 3.

## Secondary blast radius
The same stale read corrupts **state-root computation** on every post-snap block (`fork_recovery.rs:441` and per-block `compute_state_root` read `self.utxo_set`). The epoch-boundary freeze is just the first *hard* rejection, not the first divergence — snap-synced nodes compute wrong state roots continuously until restart.

## Fix (APPLIED 2026-06-19)
In `fork_recovery.rs` `apply_snap_snapshot`, the `atomic_replace` `if let Err` was turned into a `match`; on the `Ok(())` arm the snapshot's in-memory UTXO set is converted to the state_db-backed variant:
```rust
*utxo = storage::UtxoSet::from_state_db(self.state_db.clone());  // mirrors init.rs:305
```
Converts only the backend dispatch on snap-synced nodes (only when persistence succeeded) so their reads match continuous nodes bit-for-bit.
- **Deploy Q1 (consensus rules changed?):** No.
- **Deploy Q2 (block content changed?):** No.
- → No activation height, no synchronized deploy, no version bump. Rolling restart safe.

### Verification
- **Reproduction test:** `bins/node/tests/inc_i_118_snap_utxo_backend.rs::test_post_snap_utxo_write_visible_through_utxo_set` — caller-contract integration: snap-install → apply post-snap block → assert pool balance grows by the post-snap reward. **RED** before fix (`pool_before=100000000, pool_after=100000000`), **GREEN** after (`pool_after == pool_before + block_reward`). conf(0.95, measured).
- Regression: node lib 35/35, diagnostic_d2_emit_test 4/4 (snap), phase4_utxo_store_cleanup 3/3 (backend), recover_replay 4/4 (recovery), test_network `test_cluster_10x100` + `test_onchain_liveness_10k_nodes` pass isolated. clippy clean, fmt clean.
- Note: the two test_network sims transiently failed only under 5-binary parallel execution (resource contention on the 10k-node sim); they pass in isolation and never invoke `apply_snap_snapshot`, so they are unaffected by this fix.

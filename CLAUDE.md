# CLAUDE.md — DOLI

> Code is the single source of truth. Everything else is a projection.
> When a doc drifts from code, the doc is wrong — register hotfix in MEMORY.md.

## Deep Context

For deeper understanding, consult the index of `docs/understanding/PROJECT-UNDERSTANDING.md`. Read only the sections relevant to your task — consume the full document only in critical moments (e.g., consensus changes, cross-cutting refactors, incident recovery).

## Mental Model

DOLI is a PoS blockchain. Understand this flow or you will break things:

```
Producer scheduled for slot → builds block (coinbase → reward pool) → VDF proof → broadcast
All nodes: receive block → validate → apply_block() → update 3 states → cache state root
Epoch boundary: pool drained → rewards distributed bond-weighted to qualified producers
```

**3 states** that must be identical across all nodes (snap sync depends on it):
- `ChainState` — height, best hash, slot, genesis timestamp
- `UtxoSet` — every unspent output (coins, bonds, rewards). UTXO model, not accounts.
- `ProducerSet` — registered producers, bonds, delegations, pending updates

**Data flow**: Block → `apply_block()` → writes to in-memory state AND disk batch atomically → state root cached. On restart, disk → in-memory. Both paths MUST produce identical state.

**Seed nodes are also archivers.** Each seed runs with `--archive-to <dir>/blocks` which writes every block as `{height}.block` + `{height}.blake3` (checksum) plus a `manifest.json`. Archive files are the flat-file source of truth for block history — separate from the block store DB. After snap sync or chain recreation, verify archive completeness: `ls <dir>/blocks/*.block | wc -l` should equal tip height. Use `backfillFromPeer` RPC to fill block store gaps (different from archive).

**Bond lifecycle**: Register (creates Bond UTXOs) → ACTIVATION_DELAY (10 blocks) → scheduled for production → earn epoch rewards → RequestWithdrawal (FIFO, vesting penalty, 7-day delay) → ClaimWithdrawal. Bonds are UTXOs with `output_type=Bond`, `lock_until=MAX`, `extra_data=creation_slot`.

## If You Touch

- `apply_block()` → verify both UTXO paths match, check rollback paths mirror it, test state root convergence. Producer mutations (Register, AddBond, Exit, Slash, Withdrawal, Delegation) are DEFERRED to epoch boundary — never mid-epoch except epoch 0. Maintainer changes are immediate.
- rewards → check `calculate_epoch_rewards()` in `rewards.rs` AND `calculate_expected_epoch_rewards()` in `validation/rewards_legacy.rs` (currently disconnected — see MEMORY.md Open Items).
- storage serialization → every node diverges if canonical encoding changes. Requires chain reset. See `→ snapshot.rs`.
- consensus params → programmatic in `NetworkParams::defaults()`, NOT `include_str!`. Mainnet overrides blocked. Change requires new binary on ALL nodes simultaneously.
- rollback → undo-based rollback is first option (`rollback.rs`). Rebuild-from-genesis is fallback for blocks without undo data.
- Bond `extra_data` → CLI sends `creation_slot=0`, node stamps real slot at apply. Never trust raw tx `extra_data`.

## Law

1. Show plan → show diff → WAIT for approval → execute. Broken after deploy? STOP, report.
2. Never touch N1/N2 while any node syncs. They are chain tip.
3. Code is SOT. For design intent: WHITEPAPER > specs/ > docs/. For reality: read the code.
4. Simplest solution that doesn't compromise safety. Design for 1000s of producers in 10s slots.
5. Commit: `--author "Ivan D. Lozada <ivan@doli.network>"`. Gate: `cargo build && cargo clippy -- -D warnings && cargo fmt --check && cargo test`.
6. Ops: `→ doli-ops skill` — read FIRST before any infra task.
7. Consensus-critical deploys: simultaneous across all nodes. NEVER rolling.
8. Learning protocol: before following any doc/skill, check MEMORY.md hotfixes. Doc drifts from code? Register hotfix, fix the doc. A mistake not fixed at the source repeats forever.
9. Docs sync: after every code modification or new feature, update the relevant `specs/` and `docs/` files for the affected domain. Keep `specs/SPECS.md` and `docs/DOCS.md` indexes current — any new spec or doc file MUST be added to its index. `docs/understanding/PROJECT-UNDERSTANDING.md` MUST also be kept in sync — update affected sections when code changes alter architecture, workflows, parameters, or risk areas.

## Map — Code

| What | Where |
|------|-------|
| **Node struct + getters** | `bins/node/src/node/mod.rs` |
| **Node::new()** | `bins/node/src/node/init.rs` |
| **run(), start_network(), start_rpc()** | `bins/node/src/node/startup.rs` |
| **run_event_loop(), handle_network_event()** | `bins/node/src/node/event_loop.rs` |
| **handle_new_block(), execute_reorg()** | `bins/node/src/node/block_handling.rs` |
| **fork recovery (9 functions)** | `bins/node/src/node/fork_recovery.rs` |
| **apply_block()** | `bins/node/src/node/apply_block/` |
| **try_produce_block()** | `bins/node/src/node/production/` |
| **check_producer_eligibility(), validate_block_*()** | `bins/node/src/node/validation_checks.rs` |
| **calculate_epoch_rewards(), handle_equivocation()** | `bins/node/src/node/rewards.rs` |
| **rollback_one_block(), resolve_shallow_fork()** | `bins/node/src/node/rollback.rs` |
| **run_periodic_tasks()** | `bins/node/src/node/periodic.rs` |
| **genesis producer derivation** | `bins/node/src/node/genesis.rs` |
| Constants | `crates/core/src/consensus/` |
| Config/env | `crates/core/src/network_params/` |
| Scheduler | `crates/core/src/consensus/selection.rs` |
| Validation | `crates/core/src/validation/` |
| Transactions (18 types) | `crates/core/src/transaction/` |
| Block + BlockBuilder | `crates/core/src/block.rs` |
| Chainspec + genesis hash | `crates/core/src/chainspec.rs` |
| Network/gossip | `crates/network/src/service/` |
| Sync state machine | `crates/network/src/sync/manager/` |
| Block storage | `crates/storage/src/block_store.rs` |
| State DB (RocksDB) | `crates/storage/src/state_db/` |
| UTXO set (in-memory) | `crates/storage/src/utxo/` |
| ProducerSet + bonds | `crates/storage/src/producer/` |
| State root + snapshots | `crates/storage/src/snapshot.rs` |
| RPC methods (32) | `crates/rpc/src/methods/` |
| Transaction mempool | `crates/mempool/src/` |
| Auto-update system | `crates/updater/src/` |
| Block archiver | `crates/storage/src/archiver.rs` |
| CLI | `bins/cli/src/` |

## Map — Ops Scripts (run these, don't improvise)

| Task | Script |
|------|--------|
| Balances (all nodes) | `scripts/balances.sh [mainnet\|testnet\|all]` |
| Node status | `scripts/status.sh [mainnet\|testnet\|all]` |
| Bond details | `scripts/bonds.sh [mainnet\|testnet\|all]` |

## Map — Docs & Skills

| What | Where |
|------|-------|
| Architecture | `docs/architecture.md` |
| Rewards system | `docs/rewards.md` |
| RPC reference (32 methods) | `docs/rpc_reference.md` |
| CLI reference | `docs/cli.md` |
| Troubleshooting | `docs/troubleshooting.md` |
| Protocol spec | `specs/protocol.md` |
| Security model | `specs/security_model.md` |
| Ops runbook | `~/.omega/skills/doli-ops/SKILL.md` |
| RPC/debug skill | `.claude/skills/doli-network/SKILL.md` |
| Drift tracker | `MEMORY.md` (auto-memory) |
| Bug reports | `docs/legacy/bugs/` |
| CLI issues | `CLI.md` |

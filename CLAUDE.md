# CLAUDE.md — DOLI

> Code is the single source of truth. Everything else is a projection.
> When a doc drifts from code, the doc is wrong — register hotfix in MEMORY.md.

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
- rewards → check `calculate_epoch_rewards()` in `node.rs:~5993` AND `calculate_expected_epoch_rewards()` in `validation.rs` (currently disconnected — see MEMORY.md Open Items).
- storage serialization → every node diverges if canonical encoding changes. Requires chain reset. See `→ snapshot.rs`.
- consensus params → programmatic in `NetworkParams::defaults()`, NOT `include_str!`. Mainnet overrides blocked. Change requires new binary on ALL nodes simultaneously.
- rollback → undo-based rollback is first option (`node.rs:~6531`). Rebuild-from-genesis is fallback for blocks without undo data.
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

## Map — Code

| What | Where |
|------|-------|
| **apply_block()** | `bins/node/src/node.rs:~3470` |
| **try_produce_block()** | `bins/node/src/node.rs:~4649` |
| **calculate_epoch_rewards()** | `bins/node/src/node.rs:~5993` |
| **rollback_one_block()** | `bins/node/src/node.rs:~6492` |
| **genesis phase end** | `bins/node/src/node.rs:~6129` |
| Constants | `crates/core/src/consensus.rs` |
| Config/env | `crates/core/src/network_params.rs` |
| Scheduler | `crates/core/src/scheduler.rs` |
| Validation (5,698 lines) | `crates/core/src/validation.rs` |
| Transactions (16 types) | `crates/core/src/transaction.rs` |
| Block + BlockBuilder | `crates/core/src/block.rs` |
| Chainspec + genesis hash | `crates/core/src/chainspec.rs` |
| Network/gossip | `crates/network/src/service.rs` |
| Sync state machine | `crates/network/src/sync/manager.rs` |
| Block storage | `crates/storage/src/block_store.rs` |
| State DB (RocksDB) | `crates/storage/src/state_db.rs` |
| UTXO set (in-memory) | `crates/storage/src/utxo.rs` |
| UTXO set (RocksDB) | `crates/storage/src/utxo_rocks.rs` |
| ProducerSet + bonds | `crates/storage/src/producer.rs` |
| State root + snapshots | `crates/storage/src/snapshot.rs` |
| RPC methods (31) | `crates/rpc/src/methods.rs` |
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
| RPC reference (31 methods) | `docs/rpc_reference.md` |
| CLI reference | `docs/cli.md` |
| Troubleshooting | `docs/troubleshooting.md` |
| Protocol spec | `specs/protocol.md` |
| Security model | `specs/security_model.md` |
| Ops runbook | `~/.omega/skills/doli-ops/SKILL.md` |
| RPC/debug skill | `.claude/skills/doli-network/SKILL.md` |
| Drift tracker | `MEMORY.md` (auto-memory) |
| Bug reports | `docs/legacy/bugs/` |
| CLI issues | `CLI.md` |

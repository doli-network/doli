# CLAUDE.md — DOLI

> Code is the single source of truth. Everything else is a projection.
> When a doc drifts from code, the doc is wrong — register hotfix in MEMORY.md.

## Local Development

This is a local-only devnet environment. All nodes run on `127.0.0.1`.
No remote servers (ai1/ai2/ai3), no GitHub access, no explorer API.

- **Devnet data**: `~/.doli/devnet/` (keys, chainspec, data, logs, pids)
- **Testnet data**: `~/testnet/` (keys, seed, n1-n12, logs)
- **Binaries**: Built from source — `cargo build --release` → `target/release/doli-node`, `target/release/doli`
- **RPC ports**: Devnet uses 28500-28550 (seed=28500, producers=28501+)
- **P2P ports**: 50300+ (seed=50300)

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
2. Code is SOT. For design intent: WHITEPAPER > specs/ > docs/. For reality: read the code.
3. Simplest solution that doesn't compromise safety. Design for 1000s of producers in 10s slots.
4. Commit: `--author "Ivan D. Lozada <ivan@doli.network>"`. Gate: `cargo build && cargo clippy -- -D warnings && cargo fmt --check && cargo test`.
5. Learning protocol: before following any doc/skill, check MEMORY.md hotfixes. Doc drifts from code? Register hotfix, fix the doc. A mistake not fixed at the source repeats forever.

## Map — Code

| What | Where |
|------|-------|
| **Node struct + getters** | `bins/node/src/node/mod.rs` |
| **Node::new()** | `bins/node/src/node/init.rs` |
| **run(), start_network(), start_rpc()** | `bins/node/src/node/startup.rs` |
| **run_event_loop(), handle_network_event()** | `bins/node/src/node/event_loop.rs` |
| **handle_new_block(), execute_reorg()** | `bins/node/src/node/block_handling.rs` |
| **fork recovery (9 functions)** | `bins/node/src/node/fork_recovery.rs` |
| **apply_block()** | `bins/node/src/node/apply_block.rs` |
| **try_produce_block()** | `bins/node/src/node/production.rs` |
| **check_producer_eligibility(), validate_block_*()** | `bins/node/src/node/validation_checks.rs` |
| **calculate_epoch_rewards(), handle_equivocation()** | `bins/node/src/node/rewards.rs` |
| **rollback_one_block(), resolve_shallow_fork()** | `bins/node/src/node/rollback.rs` |
| **Fork recovery integration tests (11)** | `bins/node/tests/fork_recovery.rs` |
| **Node::new_for_test()** | `bins/node/src/node/init.rs` |
| **Node lib (test access)** | `bins/node/src/lib.rs` |
| **run_periodic_tasks()** | `bins/node/src/node/periodic.rs` |
| **genesis producer derivation** | `bins/node/src/node/genesis.rs` |
| Constants | `crates/core/src/consensus.rs` |
| Config/env | `crates/core/src/network_params.rs` |
| Scheduler | `crates/core/src/scheduler.rs` |
| Validation (5,698 lines) | `crates/core/src/validation.rs` |
| Transactions (27 types) | `crates/core/src/transaction.rs` |
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
| RPC methods (43) | `crates/rpc/src/methods/` |
| Transaction mempool | `crates/mempool/src/` |
| Auto-update system | `crates/updater/src/` |
| Block archiver | `crates/storage/src/archiver.rs` |
| CLI | `bins/cli/src/` |

## Map — Scripts (local testnet)

| Task | Script |
|------|--------|
| Install launchd services | `scripts/install-local-services.sh` — creates plists for seed + n1-n12 |
| Start/stop/status | `scripts/testnet.sh start\|stop\|restart\|status [seed\|n1\|...\|all]` |
| Tail logs | `scripts/testnet.sh logs [seed\|n1\|...]` |

**Port layout**:
- Seed: P2P=30300, RPC=8500, Metrics=9000
- N{i}: P2P=30300+i, RPC=8500+i, Metrics=9000+i

**Directories**: `~/testnet/` — keys, seed, n1-n12, logs, bin

## Map — Scripts (local devnet)

| Task | Script |
|------|--------|
| Node status | `scripts/status.sh` — scans local RPC ports 28500-28550 |
| Wallet balances | `scripts/balances.sh` — queries all producer wallets |
| Bond details | `scripts/bonds.sh` — shows producer/bond info via RPC |
| Chain reset | `scripts/chain-reset.sh devnet` — kill processes, wipe data |
| Build from source | `scripts/update.sh` — `cargo build --release` |
| Launch 2-node testnet | `scripts/launch_testnet.sh` — creates local devnet |
| Deploy producers | `scripts/deploy_producers.sh` — interactive producer setup |

## Map — Scripts (Seed Guardian)

| Task | Script |
|------|--------|
| Fork detection | `scripts/fork-monitor.sh` — polls all nodes, detects chain tip divergence |
| Emergency halt | `scripts/emergency-halt.sh` — pauses production on all nodes via RPC |
| Emergency resume | `scripts/emergency-resume.sh` — resumes production on all nodes via RPC |
| Seed backup | `scripts/seed-backup.sh` — creates RocksDB checkpoint via RPC |
| Test guardian | `scripts/test_guardian.sh` — smoke test all guardian features |

**Seed auto-checkpoint**: Start seeds with `--auto-checkpoint 100` for automatic snapshots every 100 blocks (keeps last 5, rotates oldest). Essential for seed protection.

## Map — Docs & Skills

| What | Where |
|------|-------|
| Architecture | `docs/architecture.md` |
| Rewards system | `docs/rewards.md` |
| RPC reference (43 methods) | `docs/rpc_reference.md` |
| CLI reference | `docs/cli.md` |
| Troubleshooting | `docs/troubleshooting.md` |
| Protocol spec | `specs/protocol.md` |
| Security model | `specs/security_model.md` |
| RPC/debug skill | `.claude/skills/doli-network/SKILL.md` |
| Drift tracker | `MEMORY.md` (auto-memory) |
| Bug reports | `docs/legacy/bugs/` |
| CLI issues | `CLI.md` |

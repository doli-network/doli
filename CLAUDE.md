# CLAUDE.md — DOLI

## Law

1. Show plan → show diff → WAIT for approval → execute. Broken after deploy? STOP, report.
2. Never touch N1/N2 while any node syncs. They are chain tip.
3. WHITEPAPER > specs/ > docs/ > Code. Conflicts resolve top-down.
4. Simplest solution that doesn't compromise safety. Design for 1000s of producers in 10s slots.
5. Commit: `--author "E. Weil <weil@doli.network>"`. Gate: `cargo build && cargo clippy -- -D warnings && cargo fmt --check && cargo test`.
6. Ops runbook: `~/.omega/skills/doli-ops/SKILL.md` — read FIRST before any infra task.
7. Consensus-critical deploys: simultaneous across all nodes. NEVER rolling.

## What the code won't tell you

**Reward pool.** All coinbase → `BLAKE3("REWARD_POOL" || "doli")` (no private key). Distributed at epoch boundary proportional to bonds. E0 skipped (genesis bonds drain pool). Pool calc includes current block's coinbase to prevent 1-DOLI-per-epoch leak.

**3 servers.** ai1=72.60.228.233, ai2=187.124.95.188, ai3=187.124.148.93 (seeds only, no SSH to ai1/ai2). Mainnet/testnet = separate binaries. ai3 easy to forget on chain resets.

**Embedded chainspecs.** Mainnet/testnet chainspecs compiled into binary (`include_str!`). Disk files and `--chainspec` ignored except devnet.

**Sync recovery.** `rollback_one_block()` first (all networks), snap sync only as fallback. Genesis mismatch peers get 1h silent cooldown. Snap quorum: `max(3, peers/2+1)`.

## Traps

- `--wallet` goes BEFORE subcommand: `doli --wallet f.json send ...`
- `doli send` has NO confirmation prompt
- `register` uses `--bonds N`, `add-bond` uses `--count N`
- RPC balance fields: `confirmed`, `immature`, `total`, `unconfirmed`. NOT `balance` or `spendable`
- `getProducer` returns `addressHash` — use THAT for `getBalance`, not `publicKey`
- Divide raw RPC values by 1e8 for DOLI display
- BLS mandatory for all producers. No BLS key = node refuses to start

## Map

| What | Where |
|------|-------|
| Constants | `crates/core/src/consensus.rs` |
| Config/env | `crates/core/src/network_params.rs` |
| Scheduler | `crates/core/src/scheduler.rs` |
| Validation | `crates/core/src/validation.rs` |
| Node loop + rewards | `bins/node/src/node.rs` |
| Network/gossip | `crates/network/src/service.rs` |
| Block storage | `crates/storage/src/block_store.rs` |
| State DB | `crates/storage/src/state_db.rs` |
| CLI | `bins/cli/src/` |
| Specs (implementation truth) | `specs/` |
| Bug reports | `docs/legacy/bugs/` |
| CLI issues | `CLI.md` |

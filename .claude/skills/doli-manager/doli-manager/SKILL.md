---
name: doli-manager
description: >
  Complete DOLI blockchain management. Covers: install from source, wallet (create/balance/send/history),
  producer (register/bond/withdraw/exit/slash), rewards, systemd/launchd service creation
  (auto-detects OS), node deployment (build/SCP/restart), chain monitoring, governance
  (updates/veto/maintainers), devnet/testnet/mainnet, troubleshooting (forks/sync/RocksDB/clock).
  Triggers: "install doli", "create wallet", "check balance", "send doli", "transfer",
  "register producer", "add bond", "claim rewards", "create service", "systemd", "launchd",
  "deploy", "run node", "start node", "stop node", "chain info", "check producers",
  "devnet", "testnet", "monitor", "troubleshoot", "wipe node", "resync", "upgrade",
  "manage doli", "doli setup". Use for ANY doli-related task.
---

# DOLI Manager

Complete operational skill for managing the DOLI blockchain via Claude Code.

## Quick Reference

### Detect Environment

```bash
OS=$(uname -s)  # Darwin = macOS, Linux = Linux
rustc --version && cargo --version
```

### Build

```bash
cargo build              # dev
cargo build --release    # production

# Pre-commit gate (MANDATORY)
cargo build && cargo clippy -- -D warnings && cargo fmt --check && cargo test
```

### Wallet

```bash
doli new --name mywallet           # create wallet
doli info                          # show doli1... address
doli balance                       # check balance
doli send doli1recipient... 100    # send (accepts doli1... or hex)
doli balance --address doli1...    # specific address balance
doli history                       # tx history
```

### Node

```bash
# Run non-producer (mainnet)
./target/release/doli-node run --yes

# Run producer
./target/release/doli-node --data-dir ~/.doli/mainnet/data run \
  --producer --producer-key ~/.doli/mainnet/keys/producer.json \
  --yes --force-start

# Devnet
./target/release/doli-node devnet init --nodes 3
./target/release/doli-node devnet start
```

### Chain Status

```bash
doli chain
# or RPC (mainnet=8500, testnet=18500, devnet=28500)
curl -s -X POST http://127.0.0.1:8500 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq .
```

## Detailed Guides

Read the appropriate reference when performing specific operations:

| Task | Reference |
|------|-----------|
| Install from source | [references/install.md](references/install.md) |
| Wallet operations | [references/wallet.md](references/wallet.md) |
| Producer operations | [references/producer.md](references/producer.md) |
| Node ops & deployment | [references/node-ops.md](references/node-ops.md) |
| systemd/launchd services | [references/services.md](references/services.md) |
| Rewards | [references/rewards.md](references/rewards.md) |
| Governance | [references/governance.md](references/governance.md) |
| Monitoring & troubleshooting | [references/troubleshooting.md](references/troubleshooting.md) |
| Network parameters | [references/network-params.md](references/network-params.md) |

## Address Format

Bech32m addresses: `doli1...` (mainnet), `tdoli1...` (testnet), `ddoli1...` (devnet).
Old 64-char hex still accepted everywhere.

## CLI Flag Order (CRITICAL)

Global options go BEFORE `doli-node` subcommand:

```bash
# CORRECT
doli-node --network testnet --data-dir /path run --producer
# WRONG
doli-node run --network testnet
```

## Network Ports

| Network | P2P | RPC | Metrics |
|---------|-----|-----|---------|
| Mainnet | 30300 | 8500 | 9000 |
| Testnet | 40300 | 18500 | 19000 |
| Devnet | 50300 | 28500 | 29000 |

## Safety Rules

1. **NEVER stop N1/N2 while N3/N4/N5 are syncing** - they are the chain tip.
2. **NEVER change `genesis.timestamp` or `slot_duration`** - breaks consensus.
3. **Consensus-critical changes**: stop ALL nodes, deploy, restart all.
4. **Non-consensus changes**: rolling restart, one at a time.
5. **Always verify** all nodes at same height/hash after deployment.

## Git Author

```
--author="E. Weil <weil@doli.network>"
```

## Scripts

| Script | Purpose |
|--------|---------|
| `scripts/check-nodes.sh` | Check height/hash of all 5 mainnet nodes |
| `scripts/create-service.sh` | Auto-detect OS, create systemd or launchd service |

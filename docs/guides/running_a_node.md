# Running a DOLI Node

This guide explains how to run a full DOLI node.

## Prerequisites

- Rust 1.75 or later
- 8GB RAM minimum (16GB recommended)
- 50GB disk space
- Stable internet connection

## Building from Source

```bash
# Clone the repository
git clone https://github.com/doli-network/doli.git
cd doli

# Build in release mode
cargo build --release
```

The node binary will be at `target/release/doli-node`.

## Networks

DOLI supports three networks. A single binary can connect to any network using the `--network` flag:

| Network | ID | Purpose | Address Prefix |
|---------|-----|---------|----------------|
| Mainnet | 1   | Production network with real value | `doli` |
| Testnet | 2   | Public test network for development | `tdoli` |
| Devnet  | 99  | Local development and testing | `ddoli` |

### Network Parameters

| Parameter | Mainnet | Testnet | Devnet |
|-----------|---------|---------|--------|
| P2P Port | 30303 | 40303 | 50303 |
| RPC Port | 8545 | 18545 | 28545 |
| Slot Duration | 60s | 10s | 5s |
| Initial Bond | 1,000 DOLI | 10 DOLI | 1 DOLI |
| Block Reward | 5 DOLI | 50 DOLI | 500 DOLI |
| VDF Target Time | ~700ms | ~700ms | ~700ms |
| Veto Period | 7 days | 1 day | 1 minute |
| Genesis Time | 2026-02-01 | 2025-06-01 | Dynamic |

All networks use hash-chain VDF with ~700ms target time for the heartbeat proof of presence.

## Quick Start

```bash
# Initialize the data directory (mainnet by default)
./doli-node init

# Start the node on mainnet
./doli-node run

# Or specify a different network
./doli-node --network testnet init
./doli-node --network testnet run

# Run on devnet for local development
./doli-node --network devnet run
```

## Configuration

Each network has its own data directory and configuration:

| Network | Data Directory | Config File |
|---------|----------------|-------------|
| Mainnet | `~/.doli/mainnet/` | `~/.doli/mainnet/config.toml` |
| Testnet | `~/.doli/testnet/` | `~/.doli/testnet/config.toml` |
| Devnet  | `~/.doli/devnet/`  | `~/.doli/devnet/config.toml` |

Default configuration (mainnet):

```toml
# Network settings
network = "mainnet"
listen_addr = "0.0.0.0:30303"
max_peers = 50

# RPC settings
[rpc]
enabled = true
listen_addr = "127.0.0.1:8545"

# Bootstrap nodes (mainnet)
[[bootstrap_nodes]]
address = "/dns4/seed1.doli.network/tcp/30303/p2p/..."
```

For testnet, ports are different:

```toml
network = "testnet"
listen_addr = "0.0.0.0:40303"

[rpc]
listen_addr = "127.0.0.1:18545"
```

## Command Line Options

```
doli-node [OPTIONS] <COMMAND>

Commands:
  run      Run the node
  init     Initialize a new data directory
  status   Show node status
  import   Import blocks from file
  export   Export blocks to file
  update   Update management commands

Global Options:
  -n, --network <NET>    Network to connect to [default: mainnet]
                         Values: mainnet, testnet, devnet
  -c, --config <FILE>    Configuration file path [default: config.toml]
  -d, --data-dir <DIR>   Data directory (overrides network default)
      --log-level <LVL>  Log level [default: info]
```

### Run Options

```bash
# Run with block production enabled
./doli-node run --producer --producer-key /path/to/key.json

# Run with custom data directory
./doli-node -d /data/doli run

# Override ports (useful for multi-node setups)
./doli-node run --p2p-port 50301 --rpc-port 28541 --metrics-port 9091

# Connect to a bootstrap node
./doli-node run --bootstrap /ip4/127.0.0.1/tcp/50303

# Disable auto-updates
./doli-node run --no-auto-update

# Update notify-only mode (show updates but don't apply)
./doli-node run --update-notify-only
```

### Run Command Options Reference

| Option | Description | Default |
|--------|-------------|---------|
| `--producer` | Enable block production | disabled |
| `--producer-key <PATH>` | Path to producer key file | required with --producer |
| `--p2p-port <PORT>` | P2P listen port | network default |
| `--rpc-port <PORT>` | RPC listen port | network default |
| `--metrics-port <PORT>` | Prometheus metrics port | 9090 |
| `--bootstrap <ADDR>` | Bootstrap node multiaddr | none |
| `--no-dht` | Disable DHT peer discovery | disabled |
| `--no-auto-update` | Disable automatic updates | enabled |
| `--update-notify-only` | Only notify about updates | disabled |

## Data Directory Structure

Each network has its own isolated data directory:

```
~/.doli/
├── mainnet/
│   ├── blocks/          # Block storage (RocksDB)
│   ├── utxo/           # UTXO set
│   ├── chain_state.bin # Chain state snapshot
│   ├── producers.bin   # Producer set
│   ├── config.toml     # Configuration
│   └── node.key        # P2P identity key
├── testnet/
│   └── ...             # Same structure
└── devnet/
    └── ...             # Same structure
```

This isolation allows running multiple networks simultaneously on the same machine (using different ports).

## Development with Devnet

Devnet is designed for local development and testing. The recommended workflow:

```
Devnet (your laptop)
    ↓ works
Testnet (public test network)
    ↓ works
Mainnet (production)
```

### What Devnet Enables

| Capability | How |
|------------|-----|
| Fast blocks | 5-second slots with ~700ms VDF |
| Be a producer for free | Bond of only 1 DOLI |
| Lots of coins | Genesis gives you millions |
| Reset everything | Delete `~/.doli/devnet` and restart |
| Simulate multiple nodes | Different ports on same machine |
| Test attacks | No consequences |
| Debug with logs | `--log-level debug` |
| Change parameters | Modify code and recompile |

### Development Workflow Example

```bash
# 1. Develop feature
vim doli-core/src/new_feature.rs

# 2. Compile
cargo build

# 3. Reset devnet (clean slate)
rm -rf ~/.doli/devnet

# 4. Start devnet node
./target/debug/doli-node --network devnet run

# 5. In another terminal: test
./target/debug/doli --network devnet producer register
./target/debug/doli --network devnet send --to xxx --amount 100

# 6. If it works, test on testnet
./target/release/doli-node --network testnet run

# 7. If it works on testnet for weeks, deploy to mainnet
```

### Simulating Multiple Local Nodes

You can run multiple devnet nodes on one machine using different ports. **Important:** Each node needs unique ports for P2P, RPC, and metrics.

```bash
# Terminal 1: Node 1 (seed producer)
./doli-node --network devnet run \
    --data-dir ~/.doli/devnet-1 \
    --p2p-port 50301 \
    --rpc-port 28541 \
    --metrics-port 9091 \
    --producer \
    --producer-key ~/.doli/devnet-1/wallet.json

# Terminal 2: Node 2 (joins node 1)
./doli-node --network devnet run \
    --data-dir ~/.doli/devnet-2 \
    --p2p-port 50302 \
    --rpc-port 28542 \
    --metrics-port 9092 \
    --producer \
    --producer-key ~/.doli/devnet-2/wallet.json \
    --bootstrap /ip4/127.0.0.1/tcp/50301

# Terminal 3: Node 3 (observer, no production)
./doli-node --network devnet run \
    --data-dir ~/.doli/devnet-3 \
    --p2p-port 50303 \
    --rpc-port 28543 \
    --metrics-port 9093 \
    --bootstrap /ip4/127.0.0.1/tcp/50301
```

**Key points for multi-node setups:**
- Each node needs its own `--data-dir` with a separate wallet
- All ports must be unique: P2P, RPC, and metrics
- Non-seed nodes use `--bootstrap` to connect to the first node
- Start the seed node first, then wait a few seconds before starting others
- Use `--no-dht` to isolate your test network from external peers (recommended)

### Isolated Test Network

When running a local test network, use `--no-dht` to prevent nodes from discovering and connecting to external peers. This ensures your test network operates in complete isolation:

```bash
# Terminal 1: Node 1 (seed producer) - isolated
./doli-node --network testnet run \
    --data-dir ~/.doli/testnet-1 \
    --p2p-port 40301 \
    --rpc-port 18541 \
    --metrics-port 9091 \
    --producer \
    --producer-key ~/.doli/testnet-1/wallet.json \
    --no-dht

# Terminal 2: Node 2 (joins node 1) - isolated
./doli-node --network testnet run \
    --data-dir ~/.doli/testnet-2 \
    --p2p-port 40302 \
    --rpc-port 18542 \
    --metrics-port 9092 \
    --producer \
    --producer-key ~/.doli/testnet-2/wallet.json \
    --bootstrap /ip4/127.0.0.1/tcp/40301 \
    --no-dht
```

**Why use `--no-dht`:**
- Prevents external peers from interfering with your test
- Avoids fork detection issues from peers on different chains
- Ensures reproducible test conditions
- Required for local multi-producer testing

### Simulating Scenarios

| Scenario | How to simulate on devnet |
|----------|---------------------------|
| Chain reorg | Partition network, create forks, reconnect |
| 51% attack | Control majority of producers |
| Node offline | `kill -9` a node, observe behavior |
| Double production | Try to sign two blocks for same slot |
| Slow VDF | Temporarily increase iterations |
| High transaction volume | Script that sends 1000 tx |
| Protocol update | Test the veto system |

### Devnet vs Testnet vs Mainnet

| Aspect | Devnet | Testnet | Mainnet |
|--------|--------|---------|---------|
| Purpose | Local dev | Public testing | Production |
| Coins have value | No | No | Yes |
| VDF time | ~700ms | ~700ms | ~700ms |
| Slot duration | 5s | 10s | 60s |
| Bond | 1 DOLI | 10 DOLI | 1,000 DOLI |
| Reset | Anytime | Never | Never |
| Bootstrap nodes | None (local) | Public seeds | Public seeds |
| Genesis | Dynamic | Fixed | Fixed |

All networks use the same ~700ms hash-chain VDF for heartbeat proof of presence.

### Bootstrap Mode (Devnet Only)

When running on devnet with no registered producers, the network operates in **bootstrap mode**. This allows nodes with `--producer` enabled to produce blocks without formal registration. Understanding bootstrap mode is essential for local development.

#### How Bootstrap Mode Works

1. **Activation**: Bootstrap mode activates automatically when there are no registered producers on devnet
2. **Eligibility**: Any node with `--producer` and `--producer-key` can produce blocks
3. **Leader Election**: Uses deterministic scoring based on slot number and producer public key
4. **Time-Based Scheduling**: Producers are assigned a time offset within each slot based on their score

#### Sync-Before-Produce (Enterprise-Grade)

New nodes in bootstrap mode use **sync-before-produce** logic instead of arbitrary time delays. This scales to thousands of nodes worldwide:

```
Node starts → Discovers peers → Syncs chain → Begins producing
                    ↓
            (No peers? Seed node - produce immediately)
```

**How it works:**
1. **No peers**: Node is the seed - produce immediately
2. **Has peers, not synced**: Wait for sync (chain is behind best peer)
3. **Has peers, synced**: Produce (within 2 slots of best peer)

**Why this is enterprise-grade:**
- No arbitrary time delays that don't scale
- Nodes naturally wait for sync before competing to produce
- Seed nodes can bootstrap the network immediately
- Joining nodes sync first, then participate
- Works for thousands of simultaneous node starts globally

#### Continuous Time-Based Scheduling

In bootstrap mode, each producer gets a deterministic "score" (0-255) for each slot:

```
Score 0   → Produce at 0% of slot (immediately)
Score 127 → Produce at ~40% of slot
Score 255 → Produce at 80% of slot (leaves 20% for propagation)
```

This prevents multiple producers from racing to produce at the exact same time. The node with the lowest score produces first; others only produce if that block doesn't arrive.

#### Common Bootstrap Mode Issues

| Issue | Cause | Solution |
|-------|-------|----------|
| Chain splits | Nodes not synced before producing | Verify `--bootstrap` flag is set |
| Both nodes produce for same slot | Different chain states | Nodes will sync automatically |
| "Block doesn't build on tip" | Race condition during sync | Normal during initial sync |
| Duplicate block warnings | GossipSub deduplication | Expected behavior |
| "Deferring production" log | Node syncing with peers | Normal - wait for sync |

**Note**: Bootstrap mode is for development only. On mainnet and testnet, you must register as a producer through the normal process.

## Monitoring

### Prometheus Metrics

The node exposes Prometheus metrics on port 9090:

```bash
curl http://localhost:9090/metrics
```

Key metrics:
- `doli_chain_height` - Current blockchain height
- `doli_peers_connected` - Number of connected peers
- `doli_mempool_size` - Mempool transaction count
- `doli_sync_progress` - Synchronization progress (0-1)

### JSON-RPC

Query node status via RPC:

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}'
```

## Synchronization

Initial sync may take several hours depending on chain length and network speed.

Monitor sync progress:

```bash
# Via RPC
curl -X POST http://localhost:8545 \
  -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}'

# Via metrics
curl http://localhost:9090/metrics | grep doli_sync
```

## Troubleshooting

### Node won't start

1. Check if ports are available (ports vary by network):
   ```bash
   # Mainnet
   lsof -i :30303
   lsof -i :8545

   # Testnet
   lsof -i :40303
   lsof -i :18545

   # Devnet
   lsof -i :50303
   lsof -i :28545
   ```

2. Check disk space:
   ```bash
   df -h ~/.doli
   ```

3. Check permissions:
   ```bash
   ls -la ~/.doli
   ```

### No peers connecting

1. Check firewall settings (adjust port for your network):
   ```bash
   # Allow P2P port (mainnet)
   sudo ufw allow 30303/tcp

   # Allow P2P port (testnet)
   sudo ufw allow 40303/tcp
   ```

2. Verify bootstrap nodes are reachable

3. Check NAT/router configuration

4. Verify you're on the correct network (peers on different networks won't connect)

### Stuck syncing

1. Check peer count (should be > 0)
2. Restart the node
3. Consider adding more bootstrap nodes

## Upgrading

1. Stop the running node (Ctrl+C or SIGTERM)
2. Build or download new version
3. Start the node

The node automatically handles data migration.

## Security

- The RPC interface is bound to localhost by default
- To expose RPC publicly, configure authentication
- Keep your producer key secure if block producing
- Regular backups of `~/.doli` are recommended

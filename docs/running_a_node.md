# running_a_node.md - Node Setup Guide

This guide covers installing, configuring, and operating a DOLI full node.

---

## 1. Prerequisites

### 1.1. Hardware Requirements

| Component | Minimum | Recommended |
|-----------|---------|-------------|
| CPU | 2 cores | 4+ cores |
| RAM | 4 GB | 8+ GB |
| Storage | 50 GB SSD | 200+ GB NVMe |
| Network | 10 Mbps | 100+ Mbps |

### 1.2. Software Requirements

- Linux (Ubuntu 22.04+, Debian 12+, Fedora 38+) or macOS 13+
- No additional dependencies for pre-built binaries

---

## 2. Installation

Choose one of the following installation methods. Pre-built binaries are recommended for most users.

### 2.1. Pre-built Binary (Recommended)

Download and run in under a minute:

```bash
# Download latest release (Linux x64)
curl -LO https://github.com/e-weil/doli/releases/latest/download/doli-latest-x86_64-unknown-linux-musl.tar.gz

# Or use the install script
curl -L https://raw.githubusercontent.com/e-weil/doli/main/scripts/update.sh | bash

# Verify installation
doli-node --version
```

**Platform-specific downloads:**

| Platform | Download |
|----------|----------|
| Linux x64 (static) | `doli-{version}-x86_64-unknown-linux-musl.tar.gz` |
| Linux x64 | `doli-{version}-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 (static) | `doli-{version}-aarch64-unknown-linux-musl.tar.gz` |
| Linux ARM64 | `doli-{version}-aarch64-unknown-linux-gnu.tar.gz` |
| macOS Intel | `doli-{version}-x86_64-apple-darwin.tar.gz` |
| macOS Apple Silicon | `doli-{version}-aarch64-apple-darwin.tar.gz` |

Download from: https://github.com/e-weil/doli/releases

**Verify checksums:**
```bash
# Download checksum file
curl -LO https://github.com/e-weil/doli/releases/latest/download/SHA256SUMS.txt

# Verify
sha256sum -c SHA256SUMS.txt --ignore-missing
```

### 2.2. Docker (Recommended for Servers)

Run a containerized node with persistent data:

```bash
# Quick start (mainnet)
docker run -d \
  --name doli-node \
  -p 30300:30300 \
  -p 8500:8500 \
  -v doli-data:/data \
  ghcr.io/e-weil/doli-node:latest

# Testnet
docker run -d \
  --name doli-testnet \
  -e DOLI_NETWORK=testnet \
  -p 40300:40300 \
  -p 18500:18500 \
  -v doli-testnet-data:/data \
  ghcr.io/e-weil/doli-node:latest

# View logs
docker logs -f doli-node
```

**Using Docker Compose:**

```bash
# Clone repository (for compose files)
git clone https://github.com/e-weil/doli.git
cd doli

# Start mainnet node
docker compose up -d

# Start testnet node
docker compose -f docker-compose.testnet.yml up -d

# Start with monitoring (Prometheus + Grafana)
docker compose --profile monitoring up -d
```

See [docker.md](./docker.md) for complete Docker documentation.

### 2.3. Build from Source

For developers or when pre-built binaries aren't available:

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Install system dependencies (Ubuntu/Debian)
sudo apt install build-essential pkg-config libssl-dev libgmp-dev librocksdb-dev

# Clone and build (--release is MANDATORY)
git clone https://github.com/e-weil/doli.git
cd doli
cargo build --release

# Install binaries
sudo cp target/release/doli-node target/release/doli /usr/local/bin/
```

> **WARNING: `--release` is mandatory.** Debug builds (`cargo build` without `--release`) are ~10x slower for VDF computation, causing block production timeouts, sync failures, and fork divergence. Debug binaries are also ~2x larger (~17MB vs ~8MB). If your binary is larger than 10MB, you have a debug build — rebuild with `--release`.

**Using Nix (for development):**

```bash
git clone https://github.com/e-weil/doli.git
cd doli
nix develop
cargo build --release
```

---

## 3. Network Selection

| Network | Purpose | Slot Time | Data Directory |
|---------|---------|-----------|----------------|
| Mainnet | Production | 10s | `~/.doli/mainnet/` |
| Testnet | Testing | 10s | `~/.doli/testnet/` |
| Devnet | Development | 10s | `~/.doli/devnet/` |

**Devnet special features:**
- **Dynamic genesis:** Genesis time is set automatically when the first node starts
- **Bootstrapping:** "Sync-Before-Produce" logic prevents split-brain genesis (see [devnet.md](./devnet.md))
- **Fast grace periods:** Reduced wait times for quicker testing
- **Lower bond:** 1 DOLI required (vs 10 DOLI on mainnet/testnet)
- **Faster reward epochs:** 4 blocks per reward epoch (~40 seconds)

### 3.1. Local Multi-Node Devnet (Recommended for Development)

For local development, use the built-in devnet management commands:

```bash
# Initialize a 5-node local devnet
doli-node devnet init --nodes 5

# Start all nodes
doli-node devnet start

# Check status
doli-node devnet status

# Stop all nodes
doli-node devnet stop

# Add producers to a running devnet
doli-node devnet add-producer --count 2

# Clean up (--keep-keys preserves wallet files)
doli-node devnet clean
```

This creates a self-contained devnet at `~/.doli/devnet/` with:
- Auto-generated producer wallets
- Pre-configured chainspec with all producers
- Automatic port allocation (P2P: 50300+, RPC: 28500+, Metrics: 9000+)
- PID tracking for process management

**Adding producers dynamically:** `devnet add-producer` creates a wallet, funds it from producer_0, registers as a producer, and starts a node — all in one command. The new nodes inherit `.env` configuration and are managed by `devnet stop/status`.

---

## 4. Running a Node

### 4.1. Initialize Data Directory

```bash
# Mainnet (default)
./target/release/doli-node init

# Testnet
./target/release/doli-node --network testnet init

# Devnet
./target/release/doli-node --network devnet init
```

### 4.2. Start the Node

```bash
# Mainnet
./target/release/doli-node run

# Testnet
./target/release/doli-node --network testnet run

# Devnet
./target/release/doli-node --network devnet run
```

### 4.3. Common Options

```bash
./target/release/doli-node run \
    --data-dir /path/to/data \    # Custom data directory
    --p2p-port 30300 \            # P2P listen port
    --rpc-port 8500 \             # RPC API port
    --metrics-port 9000 \         # Prometheus metrics port
    --bootstrap /ip4/x.x.x.x/tcp/30300  # Bootstrap node
    --log-level info              # trace|debug|info|warn|error
```

---

## 5. Configuration

DOLI nodes can be configured via:
1. **CLI flags** - Override settings per invocation
2. **Environment variables** - Persistent configuration via `.env` files

### 5.1. CLI Flags

**Common flags:**
```bash
# Network selection
--network <mainnet|testnet|devnet>

# Data directory (default: ~/.doli/<network>/)
--data-dir /path/to/data

# P2P settings
--listen-addr 0.0.0.0:30300
--max-peers 50

# RPC settings
--rpc-addr 127.0.0.1:8500

# Metrics (Prometheus)
--metrics-addr 127.0.0.1:9000

# Logging
--log-level <trace|debug|info|warn|error>
```

**Example with custom settings:**
```bash
./doli-node --network mainnet --data-dir /var/lib/doli --listen-addr 0.0.0.0:30300 --rpc-addr 127.0.0.1:8500 run
```

### 5.2. Environment Variables (.env Files)

Network parameters can be configured via `.env` files in the data directory:

```
~/.doli/mainnet/.env   # Mainnet configuration
~/.doli/testnet/.env   # Testnet configuration
~/.doli/devnet/.env    # Devnet configuration
```

**Quick setup:**
```bash
# For devnet: auto-created on init (reads .env.devnet at runtime)
doli-node devnet init --nodes 3

# For mainnet/testnet: manually copy the template
cp .env.example.mainnet ~/.doli/mainnet/.env

# Edit as needed
nano ~/.doli/devnet/.env
```

**Example devnet .env:**
```bash
# Custom ports
DOLI_P2P_PORT=51303
DOLI_RPC_PORT=29545

# Faster testing
DOLI_BLOCKS_PER_REWARD_EPOCH=2
DOLI_UNBONDING_PERIOD=30
```

### 5.3. Configurable Parameters

| Variable | Default (Mainnet) | Configurable |
|----------|-------------------|--------------|
| `DOLI_P2P_PORT` | 30300 | All networks |
| `DOLI_RPC_PORT` | 8500 | All networks |
| `DOLI_METRICS_PORT` | 9000 | All networks |
| `DOLI_BOOTSTRAP_NODES` | (seeds) | All networks |
| `DOLI_SLOT_DURATION` | 10 | Devnet only |
| `DOLI_GENESIS_TIME` | (fixed) | Devnet only |
| `DOLI_VETO_PERIOD_SECS` | 604800 | All networks |
| `DOLI_UNBONDING_PERIOD` | 60480 | Devnet only |
| `DOLI_BOND_UNIT` | 10B | Devnet only |
| `DOLI_INITIAL_REWARD` | 100M | Devnet only |
| `DOLI_VDF_ITERATIONS` | 800000 | Devnet only |
| `DOLI_BLOCKS_PER_YEAR` | 3153600 | Devnet only |
| `DOLI_BLOCKS_PER_REWARD_EPOCH` | 360 | Devnet only |
| `DOLI_COINBASE_MATURITY` | 6 | Devnet only |

### 5.4. Mainnet Locked Parameters

For security, the following parameters are **locked for mainnet** and cannot be overridden:

- `DOLI_SLOT_DURATION` - Must be 10s
- `DOLI_GENESIS_TIME` - Fixed launch time
- `DOLI_BOND_UNIT` - 10 DOLI per bond
- `DOLI_INITIAL_REWARD` - Emission schedule
- `DOLI_VDF_ITERATIONS` - Consensus security
- `DOLI_BLOCKS_PER_YEAR` - Era calculation
- `DOLI_BLOCKS_PER_REWARD_EPOCH` - Reward distribution

Attempting to override these on mainnet will log a warning and use hardcoded values.

### 5.5. Configuration Precedence

1. **Embedded binary** (mainnet ONLY — chainspec compiled in, `--chainspec` and disk files ignored)
2. **CLI flags** (highest priority for non-chainspec settings, e.g., `--p2p-port`)
3. **Chainspec direct injection** (`--chainspec` or `{data_dir}/chainspec.json`) — testnet/devnet only
4. **Parent process environment variables**
5. **`.env` file variables** (from `{data_dir}/.env` or `~/.doli/{network}/.env` fallback)
6. **Network defaults** (hardcoded in `consensus.rs`)

Example: `--rpc-port 9999` overrides `DOLI_RPC_PORT=8888` in `.env`.

**`.env` file lookup**: When `--data-dir` points to a subdirectory (e.g., `~/.doli/devnet/data/node5`), the node first checks `{data_dir}/.env`, then falls back to `~/.doli/{network}/.env`. This ensures manually-started nodes pick up the shared network configuration.

**Mainnet chainspec security**: For mainnet, the chainspec is always loaded from the binary via `include_str!`. The `--chainspec` flag and any `chainspec.json` on disk are ignored. This prevents genesis-time-hijack attacks where a tampered or stale chainspec could cause slot schedule divergence and chain forks.

**Testnet/devnet chainspec**: For testnet and devnet, chainspec files on disk and `--chainspec` flags work normally, allowing flexible parameter configuration during development.

---

## 6. Systemd Service

### 6.1. Create Service File

```bash
sudo nano /etc/systemd/system/doli-node.service
```

```ini
[Unit]
Description=DOLI Node
After=network.target

[Service]
Type=simple
User=doli
Group=doli
ExecStart=/usr/local/bin/doli-node --network mainnet --data-dir /var/lib/doli run
Restart=always
RestartSec=10
LimitNOFILE=65535

# Security hardening
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/doli

[Install]
WantedBy=multi-user.target
```

### 6.2. Enable and Start

```bash
# Create doli user
sudo useradd -r -s /bin/false doli
sudo mkdir -p /var/lib/doli
sudo chown doli:doli /var/lib/doli

# Copy binary
sudo cp target/release/doli-node /usr/local/bin/

# Enable and start service
sudo systemctl daemon-reload
sudo systemctl enable doli-node
sudo systemctl start doli-node

# Check status
sudo systemctl status doli-node
sudo journalctl -u doli-node -f
```

---

## 7. Monitoring

### 7.1. RPC Health Check

```bash
# Check chain info
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}'

# Check network info
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":[],"id":1}'
```

### 7.2. Prometheus Metrics

Key metrics available at `http://127.0.0.1:9000/metrics`:

| Metric | Description |
|--------|-------------|
| `doli_chain_height` | Current block height |
| `doli_peers_connected` | Number of connected peers |
| `doli_blocks_received_total` | Total blocks received |
| `doli_transactions_received_total` | Total transactions received |
| `doli_mempool_size` | Current mempool size |
| `doli_sync_progress` | Sync progress (0-1) |

### 7.3. Grafana Dashboard

Import the DOLI dashboard from `docs/grafana-dashboard.json` (if available) or create panels for:

- Chain height over time
- Peer count
- Block/transaction rates
- Mempool size
- Sync status

---

## 8. Syncing

### 8.1. Initial Sync

First sync may take several hours depending on chain length:

```
Sync progress:
  1. Connect to peers
  2. Download headers (fast)
  3. Download block bodies (slower)
  4. Validate and apply blocks
```

### 8.2. Check Sync Status

```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":[],"id":1}'

# Response includes:
# "syncing": true/false
# "syncProgress": 0.0-100.0
```

---

## 9. Firewall Configuration

### 9.1. Required Ports

| Port | Protocol | Direction | Purpose |
|------|----------|-----------|---------|
| 30300 | TCP | Inbound | P2P (mainnet) |
| 40300 | TCP | Inbound | P2P (testnet) |
| 50300 | TCP | Inbound | P2P (devnet) |

### 9.2. UFW Example

```bash
# Allow P2P port
sudo ufw allow 30300/tcp

# RPC (only if external access needed - NOT recommended)
# sudo ufw allow 8500/tcp
```

### 9.3. iptables Example

```bash
# Allow P2P port
sudo iptables -A INPUT -p tcp --dport 30300 -j ACCEPT
```

---

## 10. Backup and Recovery

### 10.1. What to Backup

| Path | Content | Priority |
|------|---------|----------|
| `~/.doli/{network}/node.key` | Node identity | High |
| `~/.doli/{network}/db/` | Blockchain data | Low (can resync) |

### 10.2. Backup Procedure

```bash
# Stop node
sudo systemctl stop doli-node

# Backup node key
cp ~/.doli/mainnet/node.key ~/backup/

# Optional: backup database
tar -czf ~/backup/doli-db-$(date +%Y%m%d).tar.gz ~/.doli/mainnet/db/

# Start node
sudo systemctl start doli-node
```

### 10.3. Recovery

```bash
# Restore node key
cp ~/backup/node.key ~/.doli/mainnet/

# Start node (will resync if db not restored)
sudo systemctl start doli-node
```

**If the node is stuck at height 0 but has block data on disk** (e.g., after a dirty shutdown during upgrade), use the `reindex → recover` pipeline instead of wiping:

```bash
# 1. Rebuild height index from existing block headers
doli-node --network mainnet --data-dir /path/to/data reindex

# 2. Rebuild UTXO set, producer registry, and chain state
doli-node --network mainnet --data-dir /path/to/data recover --yes

# 3. Restart the node
sudo systemctl start doli-node
```

This preserves all existing block data and avoids a full resync. See [disaster-recovery.md](disaster-recovery.md) for all recovery methods and a comparison table.

### 10.4. Block Archiver (Disaster Recovery)

Any node can archive blocks by adding `--archive-to`:

```bash
doli-node run --archive-to /path/to/archive/
```

The archiver streams every applied block to flat files with BLAKE3 checksums. On the DOLI network, the seed/archiver nodes serve this role — they are the DNS-registered entry points, public RPC backends (powering `doli.network/explorer.html`), and disaster recovery sources.

**Quick restore:**
```bash
# Full restore from seed RPC (no SSH needed)
doli-node --network mainnet restore --from-rpc http://seed2.doli.network:8500 --yes

# Backfill snap sync gaps (no restart needed)
curl -X POST http://127.0.0.1:8501 -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"backfillFromPeer","params":{"rpc_url":"http://seed2.doli.network:8500"},"id":1}'
```

For full details on archive format, all recovery methods, hot backfill, and seed infrastructure, see **[archiver.md](archiver.md)**.

---

## 11. Upgrading

### 11.1. Auto-Update (Default)

The node automatically downloads and applies updates after the veto period (currently 5 minutes, early network; target 7 days). To receive notifications only:

```bash
./target/release/doli-node run --update-notify-only
```

### 11.2. Manual Update

```bash
# Stop node
sudo systemctl stop doli-node

# Pull latest code
cd doli
git pull

# Rebuild
cargo build --release

# Update binary
sudo cp target/release/doli-node /usr/local/bin/

# Start node
sudo systemctl start doli-node
```

### 11.3. Disable Auto-Update

```bash
./target/release/doli-node run --no-auto-update
```

---

## 12. Command Reference

```bash
# Node commands
doli-node init                    # Initialize data directory
doli-node run                     # Start the node
doli-node status                  # Show node status
doli-node import <file>           # Import blocks from file
doli-node export <file>           # Export blocks to file

# Global flags
--network <mainnet|testnet|devnet>
--config <path>
--data-dir <path>
--log-level <trace|debug|info|warn|error>

# Run flags
--producer                        # Enable block production
--producer-key <path>             # Producer key file
--p2p-port <port>                 # P2P listen port
--rpc-port <port>                 # RPC listen port
--metrics-port <port>             # Metrics port
--bootstrap <multiaddr>           # Bootstrap node
--no-dht                          # Disable DHT discovery
--no-auto-update                  # Disable auto-updates
--update-notify-only              # Notify only, don't apply updates
--archive-to <path>               # Archive blocks to directory for disaster recovery
```

---

## 13. Network Defaults

| Parameter | Mainnet | Testnet | Devnet |
|-----------|---------|---------|--------|
| Network ID | 1 | 2 | 99 |
| P2P Port | 30300 | 40300 | 50300 |
| RPC Port | 8500 | 18500 | 28500 |
| Metrics Port | 9000 | 19000 | 29000 |
| Slot Duration | 10s | 10s | 10s |
| Block Reward | 1 DOLI | 1 DOLI | 20 DOLI |
| Bond Unit | 10 DOLI | 10 DOLI | 1 DOLI |
| VDF Iterations | 800K (~55ms) | 800K (~55ms) | 1 |
| Heartbeat VDF | 800K (~55ms) | 800K (~55ms) | 800K (~55ms) |
| Blocks/Year | 3,153,600 | 3,153,600 | 144 |
| Reward Epoch | 360 blocks | 360 blocks | 4 blocks |
| Address Prefix | `doli` | `tdoli` | `ddoli` |
| Config File | `~/.doli/mainnet/.env` | `~/.doli/testnet/.env` | `~/.doli/devnet/.env` |

---

*Last updated: February 2026*

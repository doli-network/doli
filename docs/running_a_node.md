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
curl -LO https://github.com/e-weil/doli/releases/latest/download/CHECKSUMS.txt

# Verify
sha256sum -c CHECKSUMS.txt --ignore-missing
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
| Mainnet | Production | 10s | Linux: `/var/lib/doli/mainnet/`, macOS: `~/Library/Application Support/doli/mainnet/`, legacy: `~/.doli/mainnet/` |
| Testnet | Testing | 10s | Linux: `/var/lib/doli/testnet/`, macOS: `~/Library/Application Support/doli/testnet/`, legacy: `~/.doli/testnet/` |
| Devnet | Development | 10s | Linux: `/var/lib/doli/devnet/`, macOS: `~/Library/Application Support/doli/devnet/`, legacy: `~/.doli/devnet/` |

**Devnet special features:**
- **Dynamic genesis:** Genesis time is set automatically when the first node starts
- **Bootstrapping:** "Sync-Before-Produce" logic prevents split-brain genesis (see [devnet.md](./devnet.md))
- **Fast grace periods:** Reduced wait times for quicker testing
- **Lower bond:** 1 DOLI required (vs 10 DOLI on mainnet)
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
- Automatic port allocation (P2P: 50300+, RPC: 28500+, Metrics: 29000+)
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
--p2p-port 30300

# RPC settings
--rpc-port 8500
--rpc-bind 0.0.0.0              # Default: 127.0.0.1

# Metrics (Prometheus)
--metrics-port 9000

# Max peers (via environment variable)
# DOLI_MAX_PEERS=50

# Logging
--log-level <trace|debug|info|warn|error>
```

**Example with custom settings:**
```bash
./doli-node --network mainnet --data-dir /var/lib/doli run --p2p-port 30300 --rpc-port 8500
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
| `DOLI_MAX_PEERS` | 50 | All networks |
| `DOLI_BOOTSTRAP_NODES` | (seeds) | All networks |
| `DOLI_SLOT_DURATION` | 10 | Testnet/Devnet |
| `DOLI_GENESIS_TIME` | (fixed) | Testnet/Devnet |
| `DOLI_VETO_PERIOD_SECS` | 300 (5 min) | All networks |
| `DOLI_GRACE_PERIOD_SECS` | 120 (2 min) | All networks |
| `DOLI_UNBONDING_PERIOD` | 60480 | Testnet/Devnet |
| `DOLI_BOND_UNIT` | 1,000,000,000 (10 DOLI) | Testnet/Devnet |
| `DOLI_INITIAL_REWARD` | 100,000,000 (1 DOLI) | Testnet/Devnet |
| `DOLI_VDF_ITERATIONS` | 1000 | Testnet/Devnet |
| `DOLI_HEARTBEAT_VDF_ITERATIONS` | 1000 | Testnet/Devnet |
| `DOLI_VDF_REGISTER_ITERATIONS` | 1000 | Testnet/Devnet |
| `DOLI_BLOCKS_PER_YEAR` | 3153600 | Testnet/Devnet |
| `DOLI_BLOCKS_PER_REWARD_EPOCH` | 360 | Testnet/Devnet |
| `DOLI_COINBASE_MATURITY` | 6 | Testnet/Devnet |
| `DOLI_SLOTS_PER_REWARD_EPOCH` | 360 | Testnet/Devnet |
| `DOLI_VESTING_QUARTER_SLOTS` | 3,153,600 | Testnet/Devnet |
| `DOLI_MIN_VOTING_AGE_SECS` | 2,592,000 (30 days) | All networks |
| `DOLI_UPDATE_CHECK_INTERVAL_SECS` | 600 (10 min) | All networks |
| `DOLI_REGISTRATION_BASE_FEE` | 100,000 (0.001 DOLI) | All networks |
| `DOLI_MAX_REGISTRATION_FEE` | 1,000,000,000 (10 DOLI) | All networks |
| `DOLI_MAX_REGISTRATIONS_PER_BLOCK` | 5 | All networks |
| `DOLI_CRASH_WINDOW_SECS` | 3600 (1 hour) | All networks |
| `DOLI_PRESENCE_WINDOW_MS` | 200 | All networks |
| `DOLI_GENESIS_BLOCKS` | 360 mainnet / 36 testnet / 40 devnet | Testnet/Devnet |
| `DOLI_AUTOMATIC_GENESIS_BOND` | 1,000,000,000 (10 DOLI mainnet) / 100,000,000 (1 DOLI testnet/devnet) | Testnet/Devnet |
| `DOLI_BOOTSTRAP_GRACE_PERIOD_SECS` | 15 mainnet/testnet / 5 devnet | Testnet/Devnet |
| `DOLI_BOOTSTRAP_BLOCKS` | 60480 | Testnet/Devnet |
| `DOLI_INACTIVITY_THRESHOLD` | 50 | Testnet/Devnet |
| `DOLI_FALLBACK_TIMEOUT_MS` | 2000 | Testnet/Devnet |
| `DOLI_MAX_FALLBACK_RANKS` | 2 | Testnet/Devnet |
| `DOLI_NETWORK_MARGIN_MS` | 200 | Testnet/Devnet |
| `DOLI_EVICTION_GRACE_SECS` | 30 | All networks |
| `DOLI_MESH_N` | 12 mainnet / 25 testnet / 12 devnet | Testnet/Devnet |
| `DOLI_MESH_N_LOW` | 8 mainnet / 20 testnet / 8 devnet | Testnet/Devnet |
| `DOLI_MESH_N_HIGH` | 24 mainnet / 50 testnet / 24 devnet | Testnet/Devnet |
| `DOLI_GOSSIP_LAZY` | 12 mainnet / 25 testnet / 12 devnet | Testnet/Devnet |
| `DOLI_YAMUX_WINDOW` | 262144 (256KB) | All networks |
| `DOLI_CONN_LIMIT` | max_peers + 10 | All networks |
| `DOLI_PENDING_LIMIT` | 5 | All networks |
| `DOLI_IDLE_TIMEOUT_SECS` | 86400 mainnet / 300 testnet / 300 devnet | All networks |

### 5.4. Mainnet Locked Parameters

For security, the following parameters are **locked for mainnet** and cannot be overridden:

- `DOLI_SLOT_DURATION` - Must be 10s
- `DOLI_GENESIS_TIME` - Fixed launch time
- `DOLI_INITIAL_REWARD` - Emission schedule (1 DOLI per block)
- `DOLI_BOND_UNIT` - Bond unit (10 DOLI, consensus-critical)
- `DOLI_VDF_ITERATIONS` - Consensus security (1,000 iterations)
- `DOLI_HEARTBEAT_VDF_ITERATIONS` - Heartbeat VDF
- `DOLI_VDF_REGISTER_ITERATIONS` - Registration VDF
- `DOLI_BLOCKS_PER_YEAR` - Era calculation
- `DOLI_BLOCKS_PER_REWARD_EPOCH` - Reward distribution
- `DOLI_SLOTS_PER_REWARD_EPOCH` - Reward epoch timing
- `DOLI_COINBASE_MATURITY` - Coinbase maturity depth
- `DOLI_BOOTSTRAP_BLOCKS` - Bootstrap phase duration
- `DOLI_BOOTSTRAP_GRACE_PERIOD_SECS` - Genesis grace period
- `DOLI_UNBONDING_PERIOD` - Unbonding delay
- `DOLI_INACTIVITY_THRESHOLD` - Inactivity detection
- `DOLI_AUTOMATIC_GENESIS_BOND` - Genesis bond amount
- `DOLI_GENESIS_BLOCKS` - Open registration period
- `DOLI_VESTING_QUARTER_SLOTS` - Vesting schedule
- `DOLI_FALLBACK_TIMEOUT_MS` - Fallback producer timing
- `DOLI_MAX_FALLBACK_RANKS` - Fallback producer count
- `DOLI_NETWORK_MARGIN_MS` - Clock drift tolerance
- `DOLI_MESH_N`, `DOLI_MESH_N_LOW`, `DOLI_MESH_N_HIGH`, `DOLI_GOSSIP_LAZY` - Gossip mesh

Attempting to override these on mainnet will use the hardcoded values silently.

### 5.5. Configuration Precedence

1. **Embedded binary** (mainnet ONLY — chainspec compiled in, `--chainspec` and disk files ignored)
2. **CLI flags** (highest priority for non-chainspec settings, e.g., `--p2p-port`, `--data-dir`)
3. **`DOLI_DATA_DIR` environment variable** (data directory override)
4. **Chainspec direct injection** (`--chainspec` or `{data_dir}/chainspec.json`) — testnet/devnet only
5. **Parent process environment variables**
6. **`.env` file variables** (from `{data_dir}/.env` or `~/.doli/{network}/.env` fallback)
7. **Network defaults** (hardcoded in `consensus/constants.rs`)

**Data directory resolution** (when `--data-dir` and `DOLI_DATA_DIR` are not set):
- Linux: `/var/lib/doli/{network}/`
- macOS: `~/Library/Application Support/doli/{network}/`
- Legacy fallback: `~/.doli/{network}/` (used if it exists and the platform path doesn't)

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
| `~/.doli/node_key` | Node identity (parent of data_dir) | High |
| `~/.doli/{network}/db/` | Blockchain data | Low (can resync) |

### 10.2. Backup Procedure

```bash
# Stop node
sudo systemctl stop doli-node

# Backup node key
cp ~/.doli/node_key ~/backup/

# Optional: backup database
tar -czf ~/backup/doli-db-$(date +%Y%m%d).tar.gz ~/.doli/mainnet/db/

# Start node
sudo systemctl start doli-node
```

### 10.3. Recovery

```bash
# Restore node key
cp ~/backup/node_key ~/.doli/

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
doli-node truncate --blocks <N>   # Remove top N blocks (fork recovery)
doli-node recover                 # Rebuild state from existing block data
doli-node restore                 # Restore chain from archive (disaster recovery)
doli-node reindex                 # Rebuild canonical chain index from headers
doli-node devnet <subcommand>     # Local devnet management (init/start/stop/status/clean/add-producer)
doli-node update <subcommand>     # Update management (check/status/vote/votes/apply/rollback/verify)
doli-node maintainer <subcommand> # Maintainer management (list/remove/add/sign/verify)
doli-node release <subcommand>    # Release signing (sign)
doli-node upgrade                 # Upgrade to latest release from GitHub
doli-node checkpoint-info         # Print checkpoint constants compiled into binary

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
--rpc-bind <address>              # RPC bind address (default: 127.0.0.1)
--metrics-port <port>             # Metrics port (default: 9000)
--external-address <multiaddr>    # External address to advertise to peers
--bootstrap <multiaddr>           # Bootstrap node (can be specified multiple times)
--no-dht                          # Disable DHT discovery
--relay-server                    # Enable relay server mode for NAT traversal
--no-auto-update                  # Disable auto-updates
--no-auto-rollback                # Disable automatic rollback on update failures
--update-notify-only              # Notify only, don't apply updates
--force-start                     # Skip duplicate key detection (DANGEROUS)
--yes                             # Skip interactive confirmations
--chainspec <path>                # Path to chainspec JSON (testnet/devnet only)
--archive-to <path>               # Archive blocks to directory for disaster recovery
--checkpoint-height <height>      # Start syncing from trusted checkpoint
--checkpoint-hash <hash>          # Hash of trusted checkpoint block
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
| Bond Unit | 10 DOLI | 1 DOLI | 1 DOLI |
| VDF Iterations | 1,000 | 1,000 | 1 |
| Heartbeat VDF | 1,000 | 1,000 | 1,000 |
| Blocks/Year | 3,153,600 | 3,153,600 | 144 |
| Reward Epoch | 360 blocks | 36 blocks | 4 blocks |
| Address Prefix | `doli` | `tdoli` | `ddoli` |
| Config File | `~/.doli/mainnet/.env` | `~/.doli/testnet/.env` | `~/.doli/devnet/.env` |

---

*Last updated: March 2026*

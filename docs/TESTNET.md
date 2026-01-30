# DOLI Testnet

Official DOLI testnet for testing and development.

**Website**: [testnet.doli.network](https://testnet.doli.network)

---

## Network Information

| Parameter | Value |
|-----------|-------|
| Network | Testnet |
| Address Prefix | `tdoli` |
| Slot Duration | 10 seconds |
| Block Reward | 50 tDOLI |
| RPC Port | 18545 |
| P2P Port | 40303 |

---

## Quick Start

### 1. Install DOLI

```bash
# Clone repository
git clone https://github.com/dolinetwork/doli.git
cd doli

# Build with Nix (recommended)
nix develop
cargo build --release

# Binaries created:
#   ./target/release/doli-node  (full node)
#   ./target/release/doli       (CLI wallet)
```

### 2. Run a Testnet Node

```bash
# Start testnet node
./target/release/doli-node --network testnet run
```

Your node will automatically connect to the testnet bootstrap nodes and begin syncing.

### 3. Create a Wallet

```bash
# Create new wallet (testnet uses port 18545)
./target/release/doli new --rpc http://127.0.0.1:18545
```

### 4. Check Your Balance

```bash
./target/release/doli balance --rpc http://127.0.0.1:18545
```

---

## Become a Testnet Validator

### Step 1: Start Your Node

```bash
# Initialize and run as a producer
./target/release/doli-node --network testnet run --producer
```

### Step 2: Register as Producer

Once your node is synced:

```bash
# Register with 1 bond (1,000 tDOLI required)
./target/release/doli producer register --bonds 1 --rpc http://127.0.0.1:18545
```

### Step 3: Check Producer Status

```bash
./target/release/doli producer status --rpc http://127.0.0.1:18545
```

---

## CLI Commands (Testnet)

All `doli` commands require `--rpc http://127.0.0.1:18545` for testnet:

```bash
# Wallet
doli new --rpc http://127.0.0.1:18545              # Create wallet
doli balance --rpc http://127.0.0.1:18545          # Check balance
doli send <address> <amount> --rpc http://127.0.0.1:18545  # Send tDOLI

# Producer
doli producer register --bonds 1 --rpc http://127.0.0.1:18545
doli producer status --rpc http://127.0.0.1:18545
doli producer list --rpc http://127.0.0.1:18545

# Chain info
doli chain --rpc http://127.0.0.1:18545
```

**Tip**: Set environment variable to avoid typing RPC every time:
```bash
export DOLI_RPC=http://127.0.0.1:18545
doli balance  # Now works without --rpc flag
```

---

## Running a Persistent Node

### Using Systemd

Create `/etc/systemd/system/doli-testnet.service`:

```ini
[Unit]
Description=DOLI Testnet Node
After=network.target

[Service]
Type=simple
User=YOUR_USER
ExecStart=/path/to/doli-node --network testnet run --producer
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable doli-testnet
sudo systemctl start doli-testnet

# View logs
journalctl -u doli-testnet -f
```

---

## Server Setup (Complete Guide)

For setting up a testnet node on a fresh server:

### 1. System Requirements

- **OS**: Ubuntu 22.04+ or similar Linux
- **CPU**: 2+ cores
- **RAM**: 4 GB minimum
- **Disk**: 50 GB SSD
- **Network**: Port 40303 open (P2P)

### 2. Install Dependencies

```bash
# Update system
sudo apt update && sudo apt upgrade -y

# Install Nix
curl -L https://nixos.org/nix/install | sh -s -- --daemon
exec $SHELL
```

### 3. Build DOLI

```bash
cd ~
git clone https://github.com/dolinetwork/doli.git
cd doli
nix develop --command cargo build --release
```

### 4. Configure Firewall

```bash
sudo ufw allow 40303/tcp comment 'DOLI Testnet P2P'
sudo ufw enable
```

### 5. Start the Node

```bash
./target/release/doli-node --network testnet run
```

### 6. Verify Sync

Check logs for sync progress:
```
Sync progress: height 100, 5 blocks/sec
```

---

## Getting Testnet Coins (tDOLI)

1. **Run a producer node** - Earn 50 tDOLI per block produced
2. **Faucet** - Coming soon
3. **Community** - Ask in Discord/Telegram

---

## Troubleshooting

### Node won't sync

```bash
# Check if P2P port is open
nc -zv testnet.doli.network 40303

# Check firewall
sudo ufw status
```

### CLI can't connect to node

```bash
# Verify node is running
pgrep -a doli-node

# Check RPC is accessible
curl http://127.0.0.1:18545
```

### Producer not producing blocks

1. Ensure `--producer` flag is set
2. Check you're fully synced: `doli chain --rpc http://127.0.0.1:18545`
3. Wait 15 seconds for producer discovery

---

## Seed Server Information

The testnet is operated from `testnet.doli.network` (198.51.100.1).

### Directory Structure

```
~/.doli/testnet/
├── maintainer_keys/     # Auto-update signing keys (5 Ed25519 keypairs)
├── producer_keys/       # Validator keys (5 producers)
├── node1/ - node5/      # Validator node data
└── start_nodes.sh       # Startup script
```

### Maintainer Public Keys (Auto-Update System)

These 5 keys control protocol updates (3-of-5 threshold):

| # | Public Key |
|---|------------|
| 1 | `721d2bc74ced1842eb77754dac75dc78d8cf7a47e10c83a7dc588c82187b70b9` |
| 2 | `d0c62cb4e143d548271eb97c4651e77b6cf52909a016bda6fb500c3bc022298d` |
| 3 | `9fac605a1ebf2acfa54ef8406ab66d604df97d63da1f1ab6a45561c7e51be697` |
| 4 | `97bdb0a9a52d4ed178c2307e3eb17e316b57d098af095b9cefc0c69d73e8817f` |
| 5 | `82ed55afabfe38d826c1e2b870aefcc9ed0de45e5620adb4f858e6f47c8d4096` |

---

## Resources

- [CLI.md](./CLI.md) - Complete CLI reference
- [RUNNING_A_NODE.md](./RUNNING_A_NODE.md) - Detailed node operation guide
- [BECOMING_A_PRODUCER.md](./BECOMING_A_PRODUCER.md) - Producer setup guide
- [WHITEPAPER.md](/WHITEPAPER.md) - Protocol specification

---

## Contact

- GitHub: [github.com/dolinetwork/doli](https://github.com/dolinetwork/doli)

# DOLI Testnet

Official DOLI testnet for testing and development.

**Seed Server**: `testnet.doli.network` (198.51.100.1)

---

## Network Information

| Parameter | Value |
|-----------|-------|
| Network ID | 2 |
| Address Prefix | `tdoli` |
| Genesis Timestamp | 1769736000 (Jan 29, 2026) |
| Slot Duration | 10 seconds |
| Slots per Epoch | 360 (1 hour) |
| Block Reward | 50 DOLI |

---

## Bootstrap Nodes

Connect to the testnet using these bootstrap addresses:

```
/dns4/bootstrap1.testnet.doli.network/tcp/40303/p2p/12D3KooWCKTKPZWVREhd1XBem2eoFcYVFxJRid87WCij95heprha
/dns4/bootstrap2.testnet.doli.network/tcp/40304/p2p/12D3KooWKfg37CcNfgoWvCpm4xNCGB9uTQgRzjS6v6NYgUqQ1Nbr
```

Alternative (IP-based):
```
/ip4/198.51.100.1/tcp/40303/p2p/12D3KooWCKTKPZWVREhd1XBem2eoFcYVFxJRid87WCij95heprha
/ip4/198.51.100.1/tcp/40304/p2p/12D3KooWKfg37CcNfgoWvCpm4xNCGB9uTQgRzjS6v6NYgUqQ1Nbr
```

---

## Ports

| Service | Port Range | Protocol |
|---------|------------|----------|
| P2P | 40303-40307 | TCP |
| RPC | 18545-18549 | TCP |
| Metrics | 9090-9094 | TCP |

---

## Seed Server Directory Structure

The testnet seed server maintains the following structure:

```
~/.doli/testnet/
├── maintainer_keys/           # Auto-update signing keys (5 Ed25519 keypairs)
│   ├── maintainer_1_private.pem
│   ├── maintainer_1_public.pem
│   ├── maintainer_2_private.pem
│   ├── maintainer_2_public.pem
│   ├── maintainer_3_private.pem
│   ├── maintainer_3_public.pem
│   ├── maintainer_4_private.pem
│   ├── maintainer_4_public.pem
│   ├── maintainer_5_private.pem
│   └── maintainer_5_public.pem
├── producer_keys/             # Validator signing keys (5 producer wallets)
│   ├── producer_1.json
│   ├── producer_2.json
│   ├── producer_3.json
│   ├── producer_4.json
│   └── producer_5.json
├── node1/                     # Validator node 1 data
│   ├── data/
│   └── node.log
├── node2/                     # Validator node 2 data
├── node3/                     # Validator node 3 data
├── node4/                     # Validator node 4 data
├── node5/                     # Validator node 5 data
└── start_nodes.sh             # Startup script
```

---

## Maintainer Keys (Auto-Update System)

These 5 Ed25519 public keys control the auto-update system (3-of-5 threshold):

| Maintainer | Public Key (hex) |
|------------|------------------|
| 1 | `721d2bc74ced1842eb77754dac75dc78d8cf7a47e10c83a7dc588c82187b70b9` |
| 2 | `d0c62cb4e143d548271eb97c4651e77b6cf52909a016bda6fb500c3bc022298d` |
| 3 | `9fac605a1ebf2acfa54ef8406ab66d604df97d63da1f1ab6a45561c7e51be697` |
| 4 | `97bdb0a9a52d4ed178c2307e3eb17e316b57d098af095b9cefc0c69d73e8817f` |
| 5 | `82ed55afabfe38d826c1e2b870aefcc9ed0de45e5620adb4f858e6f47c8d4096` |

**Key Storage**: Private keys are stored on the seed server at `~/.doli/testnet/maintainer_keys/`

---

## Joining the Testnet

### Prerequisites

1. **Linux server** (Ubuntu 22.04+ recommended)
2. **Open firewall port** for P2P (default: 40303)
3. **Nix package manager** (for reproducible builds)

### Option A: Run a Non-Producing Node (Sync Only)

Use this to sync the chain and query data without producing blocks.

```bash
# Install Nix (if not already installed)
curl -L https://nixos.org/nix/install | sh -s -- --daemon

# Clone the repository
git clone https://github.com/dolinetwork/doli.git
cd doli

# Build the node
nix develop --command cargo build --release

# Run a sync-only node
./target/release/doli-node --network testnet run \
    --bootstrap /dns4/bootstrap1.testnet.doli.network/tcp/40303/p2p/12D3KooWCKTKPZWVREhd1XBem2eoFcYVFxJRid87WCij95heprha
```

### Option B: Run a Producer Node (Validator)

To participate in block production, you need a producer key.

#### Step 1: Generate a Producer Key

```bash
# Generate a new wallet/producer key
./target/release/doli wallet new --network testnet

# This creates a key file at ~/.doli/testnet/wallet.json
# Note the public key and address for your records
```

#### Step 2: Start the Producer Node

```bash
# Run as a producer
./target/release/doli-node --network testnet run \
    --producer \
    --producer-key ~/.doli/testnet/wallet.json \
    --bootstrap /dns4/bootstrap1.testnet.doli.network/tcp/40303/p2p/12D3KooWCKTKPZWVREhd1XBem2eoFcYVFxJRid87WCij95heprha
```

#### Step 3: Create a Systemd Service (Recommended)

Create `/etc/systemd/system/doli-testnet.service`:

```ini
[Unit]
Description=DOLI Testnet Node
After=network.target
Wants=network-online.target

[Service]
Type=simple
User=YOUR_USERNAME
WorkingDirectory=/home/YOUR_USERNAME
ExecStart=/home/YOUR_USERNAME/doli/target/release/doli-node --network testnet run \
    --producer \
    --producer-key /home/YOUR_USERNAME/.doli/testnet/wallet.json \
    --bootstrap /dns4/bootstrap1.testnet.doli.network/tcp/40303/p2p/12D3KooWCKTKPZWVREhd1XBem2eoFcYVFxJRid87WCij95heprha
Restart=on-failure
RestartSec=10
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable doli-testnet
sudo systemctl start doli-testnet

# View logs
journalctl -u doli-testnet -f
```

---

## Complete Setup Guide for a New Server

This section provides step-by-step instructions for setting up a DOLI testnet node on a fresh server anywhere in the world.

### 1. Server Requirements

- **OS**: Ubuntu 22.04 LTS or later
- **CPU**: 2+ cores
- **RAM**: 4 GB minimum
- **Disk**: 50 GB SSD
- **Network**: Public IP with port 40303 open

### 2. Initial Server Setup

```bash
# Update system
sudo apt update && sudo apt upgrade -y

# Install dependencies
sudo apt install -y build-essential curl git

# Install Nix package manager
curl -L https://nixos.org/nix/install | sh -s -- --daemon

# Restart shell to load Nix
exec $SHELL
```

### 3. Clone and Build DOLI

```bash
# Clone repository
cd ~
git clone https://github.com/dolinetwork/doli.git
cd doli

# Enter Nix development environment and build
nix develop --command cargo build --release

# Verify build
./target/release/doli-node --version
```

### 4. Configure Firewall

```bash
# Allow P2P port
sudo ufw allow 40303/tcp comment 'DOLI Testnet P2P'

# Optional: Allow RPC (only if you need external RPC access)
# sudo ufw allow 18545/tcp comment 'DOLI Testnet RPC'

# Enable firewall if not already enabled
sudo ufw enable
```

### 5. Create Data Directory

```bash
mkdir -p ~/.doli/testnet
```

### 6. Generate Producer Key (Optional - for validators)

```bash
cd ~/doli
./target/release/doli wallet new --network testnet --output ~/.doli/testnet/producer.json

# Save the output - it contains your address and public key
cat ~/.doli/testnet/producer.json
```

### 7. Create Systemd Service

```bash
# Create service file
sudo tee /etc/systemd/system/doli-testnet.service > /dev/null << 'EOF'
[Unit]
Description=DOLI Testnet Node
After=network.target
Wants=network-online.target

[Service]
Type=simple
User=YOUR_USERNAME
WorkingDirectory=/home/YOUR_USERNAME/.doli/testnet
Environment="RUST_LOG=info"

# For sync-only node (remove --producer flags):
# ExecStart=/home/YOUR_USERNAME/doli/target/release/doli-node --network testnet run --bootstrap /dns4/bootstrap1.testnet.doli.network/tcp/40303/p2p/12D3KooWCKTKPZWVREhd1XBem2eoFcYVFxJRid87WCij95heprha

# For producer node:
ExecStart=/home/YOUR_USERNAME/doli/target/release/doli-node --network testnet run \
    --producer \
    --producer-key /home/YOUR_USERNAME/.doli/testnet/producer.json \
    --bootstrap /dns4/bootstrap1.testnet.doli.network/tcp/40303/p2p/12D3KooWCKTKPZWVREhd1XBem2eoFcYVFxJRid87WCij95heprha

Restart=on-failure
RestartSec=10
StandardOutput=journal
StandardError=journal

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=/home/YOUR_USERNAME/.doli

[Install]
WantedBy=multi-user.target
EOF

# Replace YOUR_USERNAME with your actual username
sudo sed -i "s/YOUR_USERNAME/$USER/g" /etc/systemd/system/doli-testnet.service

# Reload and enable
sudo systemctl daemon-reload
sudo systemctl enable doli-testnet
```

### 8. Start the Node

```bash
# Start the service
sudo systemctl start doli-testnet

# Check status
sudo systemctl status doli-testnet

# View logs (follow mode)
journalctl -u doli-testnet -f
```

### 9. Verify Synchronization

After starting, your node will:
1. Connect to bootstrap nodes
2. Discover other peers via DHT
3. Sync the blockchain from genesis

Check sync progress in logs:
```bash
journalctl -u doli-testnet | grep -i "sync\|height"
```

You should see messages like:
```
Sync progress: height 100, 5 blocks/sec
```

### 10. Verify Producer Status (for validators)

Once synced, if you're running as a producer, you'll see:
```
Producer schedule view: ["abc123...", "def456...", ...] (count=N)
Block produced at height X
```

---

## Troubleshooting

### Node won't connect to peers

1. Check firewall: `sudo ufw status`
2. Verify DNS resolution: `dig +short bootstrap1.testnet.doli.network`
3. Test connectivity: `nc -zv bootstrap1.testnet.doli.network 40303`
4. Try IP-based bootstrap address as fallback

### Node crashes on startup

1. Check logs: `journalctl -u doli-testnet -n 100`
2. Ensure data directory is writable
3. Verify producer key file exists and is valid JSON

### Sync is slow

1. Check network connectivity
2. Add both bootstrap nodes for redundancy
3. The initial sync from genesis takes time - be patient

### Producer not producing blocks

1. Verify `--producer` flag is set
2. Check producer key is loaded (look for "Producer key loaded" in logs)
3. Wait for producer discovery (15 seconds stability period)
4. Check you're synced to the chain tip

---

## Node Management

### View Status
```bash
sudo systemctl status doli-testnet
```

### View Logs
```bash
# Real-time logs
journalctl -u doli-testnet -f

# Last 100 lines
journalctl -u doli-testnet -n 100

# Filter for errors
journalctl -u doli-testnet | grep -i error
```

### Restart Node
```bash
sudo systemctl restart doli-testnet
```

### Stop Node
```bash
sudo systemctl stop doli-testnet
```

### Update Node
```bash
# Stop the node
sudo systemctl stop doli-testnet

# Pull latest code
cd ~/doli
git pull

# Rebuild
nix develop --command cargo build --release

# Start the node
sudo systemctl start doli-testnet
```

---

## RPC Endpoints

If RPC is enabled, query the node at `http://localhost:18545`:

```bash
# Get block number
curl -X POST http://localhost:18545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'

# Get sync status
curl -X POST http://localhost:18545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_syncing","params":[],"id":1}'
```

---

## Getting Testnet Coins

Testnet DOLI (tDOLI) can be obtained by:

1. **Running a producer node** - You'll earn block rewards (50 tDOLI per block)
2. **Faucet** - (Coming soon) Request testnet coins from the community faucet
3. **Community** - Ask in the DOLI Discord/Telegram for testnet coins

---

## Resources

- [WHITEPAPER.md](/WHITEPAPER.md) - Protocol specification
- [RUNNING_A_NODE.md](./RUNNING_A_NODE.md) - General node operation guide
- [BECOMING_A_PRODUCER.md](./BECOMING_A_PRODUCER.md) - Producer setup guide
- [RPC_REFERENCE.md](./RPC_REFERENCE.md) - RPC API documentation
- [TROUBLESHOOTING.md](./TROUBLESHOOTING.md) - Common issues and solutions

---

## Contact

- GitHub Issues: https://github.com/dolinetwork/doli/issues
- Discord: (Coming soon)
- Telegram: (Coming soon)

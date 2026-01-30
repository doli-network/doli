---
name: testnet-setup
description: Use this skill when the user wants to create a testnet, join the testnet, run a testnet node, become a testnet producer, set up testnet infrastructure, or asks about testnet configuration.
version: 1.0.0
---

# DOLI Testnet Setup Skill

This skill guides you through setting up and running DOLI testnet nodes and producers.

## Quick Reference

| Action | Command |
|--------|---------|
| Join testnet as producer | `doli-node --network testnet run --producer --producer-key <wallet>` |
| Create wallet | `doli new -w ~/.doli/testnet/producer.json` |
| Check balance | `doli balance -w <wallet> --rpc http://127.0.0.1:18545` |
| Register producer | `doli producer register --bonds 1 -w <wallet> --rpc http://127.0.0.1:18545` |

## Network Parameters

| Parameter | Value |
|-----------|-------|
| Network ID | 2 |
| Address Prefix | `tdoli` |
| Slot Duration | 10 seconds |
| Block Reward | 1 tDOLI |
| Epoch Length | 360 blocks (1 hour) |
| P2P Port | 40303 |
| RPC Port | 18545 |
| Genesis | January 29, 2026 22:00 UTC |

## Scenario 1: Join Existing Testnet as Producer

### Step 1: Build DOLI

```bash
# Enter Nix environment
nix develop

# Build release binaries
cargo build --release
```

### Step 2: Create Producer Wallet

```bash
# Create wallet
./target/release/doli new -w ~/.doli/testnet/producer.json

# View public key (save this!)
./target/release/doli info -w ~/.doli/testnet/producer.json
```

### Step 3: Open Firewall

```bash
sudo ufw allow 40303/tcp comment 'DOLI Testnet P2P'
sudo ufw enable
```

### Step 4: Run Producer Node

```bash
./target/release/doli-node --network testnet run --producer --producer-key ~/.doli/testnet/producer.json
```

Node auto-connects to `testnet.doli.network` and starts syncing. Once synced, you'll see block production messages.

### Step 5: Register as Producer (earn rewards)

```bash
# Set RPC endpoint
export DOLI_RPC=http://127.0.0.1:18545

# Check balance (need 1,000 tDOLI per bond)
doli balance -w ~/.doli/testnet/producer.json

# Register with 1 bond
doli producer register --bonds 1 -w ~/.doli/testnet/producer.json

# Verify registration
doli producer status -w ~/.doli/testnet/producer.json

# List all producers
doli producer list
```

## Scenario 2: Run as Systemd Service (Production)

Create a systemd service for persistent operation:

```bash
sudo tee /etc/systemd/system/doli-testnet.service > /dev/null << 'EOF'
[Unit]
Description=DOLI Testnet Producer
After=network.target

[Service]
Type=simple
User=YOUR_USER
ExecStart=/home/YOUR_USER/doli/target/release/doli-node --network testnet run --producer --producer-key /home/YOUR_USER/.doli/testnet/producer.json
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
EOF

# Replace YOUR_USER with actual username
sudo sed -i "s/YOUR_USER/$USER/g" /etc/systemd/system/doli-testnet.service

# Enable and start
sudo systemctl daemon-reload
sudo systemctl enable doli-testnet
sudo systemctl start doli-testnet

# View logs
journalctl -u doli-testnet -f
```

## Scenario 3: Launch New Testnet (Network Operators Only)

Only for launching a completely new network from scratch.

### Step 1: Create Genesis Producer Wallets

```bash
mkdir -p ~/.doli/genesis

for i in 1 2 3 4 5; do
    ./target/release/doli new -w ~/.doli/genesis/producer_$i.json
done
```

### Step 2: Generate Chainspec

```bash
./scripts/generate_chainspec.sh testnet ~/.doli/genesis testnet.json
```

### Step 3: Start Genesis Node

```bash
./target/release/doli-node --network testnet --chainspec testnet.json run \
    --producer --producer-key ~/.doli/genesis/producer_1.json
```

See [GENESIS.md](/docs/GENESIS.md) for complete network launch procedures.

## CLI Commands Reference

| Command | Description |
|---------|-------------|
| `doli balance` | Check wallet balance |
| `doli send <address> <amount>` | Send tDOLI |
| `doli chain` | Chain info |
| `doli producer status` | Producer status |
| `doli producer list` | List all producers |
| `doli producer register --bonds N` | Register with N bonds |
| `doli producer add-bond --count N` | Add N more bonds |

## Troubleshooting

### Node won't sync

```bash
# Test connectivity
nc -zv testnet.doli.network 40303

# Check firewall
sudo ufw status
```

### Not producing blocks

1. Ensure `--producer` flag is set
2. Wait for sync to complete
3. Wait 15 seconds for producer discovery
4. Check registration status: `doli producer status`

### Check node status

```bash
journalctl -u doli-testnet | grep -i "height\|produced"
```

### Verify connectivity

```bash
# Check if RPC is responding
curl -s http://127.0.0.1:18545 -X POST -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"chain_getInfo","params":[],"id":1}'
```

## Server Requirements

- Ubuntu 22.04+ or similar Linux
- 2+ CPU cores
- 4 GB RAM
- 50 GB SSD
- Port 40303 open (P2P)
- Port 18545 open (RPC, optional for external access)

## Decision Tree

```
User wants to...
│
├─ Join existing testnet?
│  └─ Follow Scenario 1 (Join Existing Testnet)
│
├─ Run persistent production node?
│  └─ Follow Scenario 2 (Systemd Service)
│
├─ Launch brand new network?
│  └─ Follow Scenario 3 (Network Operators)
│     └─ Also read: docs/GENESIS.md
│
└─ Just check testnet status?
   └─ doli chain --rpc http://testnet.doli.network:18545
```

## Related Documentation

- [TESTNET.md](/docs/TESTNET.md) - Full testnet documentation
- [GENESIS.md](/docs/GENESIS.md) - Network launch procedures
- [RUNNING_A_NODE.md](/docs/RUNNING_A_NODE.md) - Node operation guide
- [BECOMING_A_PRODUCER.md](/docs/BECOMING_A_PRODUCER.md) - Producer guide
- [CLI.md](/docs/CLI.md) - Complete CLI reference

---
name: network-setup
description: Use this skill when the user wants to set up a node, create a producer, join a network (devnet/testnet/mainnet), run a node, become a producer, or asks about network configuration.
version: 2.0.0
---

# DOLI Network Setup Skill

This skill guides you through setting up and running DOLI nodes and producers on any network.

## Network Parameters

| Parameter | Devnet | Testnet | Mainnet |
|-----------|--------|---------|---------|
| **Network ID** | 99 | 2 | 1 |
| **Address Prefix** | `ddoli` | `tdoli` | `doli` |
| **Slot Duration** | 5 seconds | 10 seconds | 10 seconds |
| **Epoch Length** | 360 blocks | 360 blocks | 360 blocks |
| **P2P Port** | 50303 | 40303 | 30303 |
| **RPC Port** | 28545 | 18545 | 8545 |
| **Bootstrap** | None (local) | `testnet.doli.network` | `doli.network` |
| **Block Reward** | 1 dDOLI | 1 tDOLI | 1 DOLI |

## Quick Reference

| Action | Command |
|--------|---------|
| Run node | `doli-node --network <NETWORK> run` |
| Run as producer | `doli-node --network <NETWORK> run --producer --producer-key <wallet>` |
| Create wallet | `doli new -w <wallet-path>` |
| Check balance | `doli balance -w <wallet> --rpc <RPC_URL>` |
| Register producer | `doli producer register --bonds 1 -w <wallet> --rpc <RPC_URL>` |

Replace `<NETWORK>` with `devnet`, `testnet`, or `mainnet`.

## Decision Tree

```
User wants to...
│
├─ Local development/testing?
│  └─ Use devnet (fast 5s blocks, no external dependencies)
│
├─ Public testing with other operators?
│  └─ Use testnet (mirrors mainnet timing)
│
├─ Production deployment?
│  └─ Use mainnet
│
└─ Launch a brand new network?
   └─ See Scenario 3 (Network Operators)
```

## Scenario 1: Run a Producer Node

### Step 1: Build DOLI

```bash
# Enter Nix environment
nix develop

# Build release binaries
cargo build --release
```

### Step 2: Create Producer Wallet

```bash
# Create directory and wallet
mkdir -p ~/.doli/<NETWORK>
./target/release/doli new -w ~/.doli/<NETWORK>/producer.json

# View public key (save this!)
./target/release/doli info -w ~/.doli/<NETWORK>/producer.json
```

### Step 3: Open Firewall (testnet/mainnet only)

| Network | Command |
|---------|---------|
| Devnet | Not needed (local) |
| Testnet | `sudo ufw allow 40303/tcp comment 'DOLI Testnet P2P'` |
| Mainnet | `sudo ufw allow 30303/tcp comment 'DOLI Mainnet P2P'` |

### Step 4: Run Producer Node

**Devnet (local development):**
```bash
./target/release/doli-node --network devnet run --producer --producer-key ~/.doli/devnet/producer.json
```

**Testnet:**
```bash
./target/release/doli-node --network testnet run --producer --producer-key ~/.doli/testnet/producer.json
```

**Mainnet:**
```bash
./target/release/doli-node --network mainnet run --producer --producer-key ~/.doli/mainnet/producer.json
```

For testnet/mainnet, the node auto-connects to bootstrap nodes and starts syncing.

### Step 5: Register as Producer (earn rewards)

```bash
# Set RPC endpoint based on network
export DOLI_RPC=http://127.0.0.1:<RPC_PORT>  # 28545 (devnet), 18545 (testnet), 8545 (mainnet)

# Check balance (need 1,000 tokens per bond)
./target/release/doli balance -w ~/.doli/<NETWORK>/producer.json

# Register with 1 bond
./target/release/doli producer register --bonds 1 -w ~/.doli/<NETWORK>/producer.json

# Verify registration
./target/release/doli producer status -w ~/.doli/<NETWORK>/producer.json

# List all producers
./target/release/doli producer list
```

## Scenario 2: Run as Systemd Service (Production)

For persistent testnet/mainnet operation:

```bash
sudo tee /etc/systemd/system/doli-<NETWORK>.service > /dev/null << 'EOF'
[Unit]
Description=DOLI <NETWORK> Producer
After=network.target

[Service]
Type=simple
User=YOUR_USER
ExecStart=/home/YOUR_USER/doli/target/release/doli-node --network <NETWORK> run --producer --producer-key /home/YOUR_USER/.doli/<NETWORK>/producer.json
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
EOF

# Replace placeholders
sudo sed -i "s/YOUR_USER/$USER/g" /etc/systemd/system/doli-<NETWORK>.service
sudo sed -i "s/<NETWORK>/testnet/g" /etc/systemd/system/doli-<NETWORK>.service  # or mainnet

# Enable and start
sudo systemctl daemon-reload
sudo systemctl enable doli-<NETWORK>
sudo systemctl start doli-<NETWORK>

# View logs
journalctl -u doli-<NETWORK> -f
```

## Scenario 3: Launch New Network (Network Operators Only)

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
./scripts/generate_chainspec.sh <NETWORK> ~/.doli/genesis chainspec.json
```

### Step 3: Start Genesis Node

```bash
./target/release/doli-node --network <NETWORK> --chainspec chainspec.json run \
    --producer --producer-key ~/.doli/genesis/producer_1.json
```

See [genesis.md](/docs/genesis.md) for complete network launch procedures.

## CLI Commands Reference

| Command | Description |
|---------|-------------|
| `doli balance` | Check wallet balance |
| `doli send <address> <amount>` | Send tokens |
| `doli chain` | Chain info |
| `doli producer status` | Producer status |
| `doli producer list` | List all producers |
| `doli producer register --bonds N` | Register with N bonds |
| `doli producer add-bond --count N` | Add N more bonds |

## Troubleshooting

### Node won't sync (testnet/mainnet)

```bash
# Test connectivity
nc -zv testnet.doli.network 40303  # or mainnet equivalent

# Check firewall
sudo ufw status
```

### Not producing blocks

1. Ensure `--producer` flag is set
2. Wait for sync to complete (testnet/mainnet)
3. Wait 15 seconds for producer discovery
4. Check registration status: `doli producer status`

### Check node status

```bash
journalctl -u doli-<NETWORK> | grep -i "height\|produced"
```

### Verify RPC connectivity

```bash
# Replace port: 28545 (devnet), 18545 (testnet), 8545 (mainnet)
curl -s http://127.0.0.1:<RPC_PORT> -X POST -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"chain_getInfo","params":[],"id":1}'
```

## Server Requirements

| Requirement | Minimum |
|-------------|---------|
| OS | Ubuntu 22.04+ or similar Linux |
| CPU | 2+ cores |
| RAM | 4 GB |
| Storage | 50 GB SSD |
| Network | P2P port open (see table above) |

## Related Documentation

- [genesis.md](/docs/genesis.md) - Network launch procedures
- [testnet.md](/docs/testnet.md) - Testnet information
- [running_a_node.md](/docs/running_a_node.md) - Node operation guide
- [becoming_a_producer.md](/docs/becoming_a_producer.md) - Producer guide
- [cli.md](/docs/cli.md) - Complete CLI reference

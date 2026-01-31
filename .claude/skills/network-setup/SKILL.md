---
name: network-setup
description: Use this skill when the user wants to set up a node, create a producer, join a network (devnet/testnet/mainnet), run a node, become a producer, or asks about network configuration.
version: 2.3.0
---

# DOLI Network Setup Skill

This skill guides you through setting up and running DOLI nodes and producers on any network.

## Network Parameters

| Parameter | Devnet | Testnet | Mainnet |
|-----------|--------|---------|---------|
| **Network ID** | 99 | 2 | 1 |
| **Address Prefix** | `ddoli` | `tdoli` | `doli` |
| **Slot Duration** | 1 second | 10 seconds | 10 seconds |
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
| Create wallet | `doli -w <wallet-path> new` |
| Check balance | `doli -w <wallet> balance` |
| Register producer | `doli -w <wallet> producer register --bonds 1` |

Replace `<NETWORK>` with `devnet`, `testnet`, or `mainnet`.

## Decision Tree

```
User wants to...
│
├─ Local development/testing?
│  └─ Use devnet (fast 1s blocks, no external dependencies)
│
├─ Public testing with other operators?
│  └─ Use testnet (mirrors mainnet timing)
│
├─ Production deployment?
│  └─ Use mainnet
│
├─ Run as background service?
│  └─ See Scenario 3 (Systemd Service)
│
└─ Launch a brand new network?
   └─ See Scenario 4 (Network Operators)
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
./target/release/doli -w ~/.doli/<NETWORK>/producer.json new

# View public key (save this!)
./target/release/doli -w ~/.doli/<NETWORK>/producer.json info
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
./target/release/doli -w ~/.doli/<NETWORK>/producer.json balance

# Register with 1 bond
./target/release/doli -w ~/.doli/<NETWORK>/producer.json producer register --bonds 1

# Verify registration
./target/release/doli -w ~/.doli/<NETWORK>/producer.json producer status

# List all producers
./target/release/doli producer list
```

## Scenario 2: Local Multi-Node Testnet

For development and testing with multiple nodes on a single machine.

### Option A: Quick 2-Node Launch

```bash
# Use the built-in script
./scripts/launch_testnet.sh
```

This script:
- Creates 2 producer nodes with auto-generated keys
- Sets up proper P2P bootstrapping
- Provides status check and log viewing commands

### Option B: Custom N-Node Testnet

For more control (e.g., 5 nodes with specific configuration):

```bash
# Set up directories
export TESTNET_DIR=~/.doli/testnet
mkdir -p $TESTNET_DIR/keys $TESTNET_DIR/logs
mkdir -p $TESTNET_DIR/{node1,node2,node3,node4,node5}/data

# Generate N producer wallets
for i in 1 2 3 4 5; do
    ./target/release/doli -w $TESTNET_DIR/keys/producer_$i.json new
done

# IMPORTANT: Generate chainspec from wallets (required for local testnet)
./scripts/generate_chainspec.sh testnet $TESTNET_DIR/keys $TESTNET_DIR/chainspec.json
```

**Start Node 1 (Bootstrap/Seed):**
```bash
./target/release/doli-node \
    --data-dir $TESTNET_DIR/node1/data \
    --network testnet \
    run \
    --chainspec $TESTNET_DIR/chainspec.json \
    --producer \
    --producer-key $TESTNET_DIR/keys/producer_1.json \
    --p2p-port 40303 \
    --rpc-port 18545 \
    --metrics-port 9090 \
    --no-auto-update \
    --no-dht
```

**Start Nodes 2-N (Bootstrap from Node 1):**
```bash
# Node 2
./target/release/doli-node \
    --data-dir $TESTNET_DIR/node2/data \
    --network testnet \
    run \
    --chainspec $TESTNET_DIR/chainspec.json \
    --producer \
    --producer-key $TESTNET_DIR/keys/producer_2.json \
    --p2p-port 40304 \
    --rpc-port 18546 \
    --metrics-port 9091 \
    --bootstrap "/ip4/127.0.0.1/tcp/40303" \
    --no-auto-update \
    --no-dht

# Pattern for nodes 3-N: increment ports by 1 for each node
# Node 3: p2p=40305, rpc=18547, metrics=9092
# Node 4: p2p=40306, rpc=18548, metrics=9093
# Node 5: p2p=40307, rpc=18549, metrics=9094
```

**Key flags for local testnet:**
- `--chainspec`: Use custom genesis with your producer wallets
- `--no-dht`: Isolate from external peers (prevents connecting to public testnet)
- `--no-auto-update`: Disable auto-updates during testing

### Port Allocation Pattern

| Node | P2P Port | RPC Port | Metrics Port |
|------|----------|----------|--------------|
| 1 | 40303 | 18545 | 9090 |
| 2 | 40304 | 18546 | 9091 |
| 3 | 40305 | 18547 | 9092 |
| N | 40303+(N-1) | 18545+(N-1) | 9090+(N-1) |

### Check Multi-Node Status

```bash
# Quick status check for 5 nodes
for port in 18545 18546 18547 18548 18549; do
  echo "=== RPC $port ==="
  curl -s http://127.0.0.1:$port -X POST \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | \
    jq -c '.result | {height: .bestHeight, slot: .bestSlot}'
done
```

### Testnet Management Scripts

After setting up a local testnet, these scripts help manage it:

| Script | Location | Description |
|--------|----------|-------------|
| `start_5node_testnet.sh` | `~/.doli/testnet/` | Start all 5 nodes |
| `stop_testnet.sh` | `~/.doli/testnet/` | Stop all nodes gracefully |
| `status.sh` | `~/.doli/testnet/` | Check status of all nodes |
| `rebuild_restart.sh` | `~/.doli/testnet/` | Rebuild binary and restart nodes |

**Rebuild and Restart (after code changes):**
```bash
~/.doli/testnet/rebuild_restart.sh
```

This script:
1. Stops all running testnet nodes
2. Rebuilds the binary with `cargo build --release`
3. Restarts all nodes with the new binary
4. Shows chain status to verify restart

### Available Test Scripts

| Script | Description |
|--------|-------------|
| `scripts/launch_testnet.sh` | Quick 2-node devnet |
| `scripts/test_3node_proportional_rewards.sh` | 3-node reward testing |
| `scripts/test_5node_epoch_rewards_consistency.sh` | 5-node epoch rewards |
| `scripts/test_devnet_3node_rewards.sh` | 3-node devnet rewards |

---

## Scenario 3: Run as Systemd Service (Production)

For persistent production operation (testnet/mainnet):

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

## Scenario 4: Launch New Network (Network Operators Only)

Only for launching a completely new network from scratch.

### Step 1: Create Genesis Producer Wallets

```bash
mkdir -p ~/.doli/genesis

for i in 1 2 3 4 5; do
    ./target/release/doli -w ~/.doli/genesis/producer_$i.json new
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
5. **For local testnet**: Ensure you're using `--chainspec` with a chainspec generated from your producer wallets (see Scenario 2 Option B)

### Check node status

```bash
journalctl -u doli-<NETWORK> | grep -i "height\|produced"
```

### Verify RPC connectivity

```bash
# Replace port: 28545 (devnet), 18545 (testnet), 8545 (mainnet)
curl -s http://127.0.0.1:<RPC_PORT> -X POST -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}'
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

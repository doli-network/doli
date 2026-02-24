---
name: network-setup
description: Use this skill when the user wants to set up a node, create a producer, join a network (devnet/testnet/mainnet), run a node, become a producer, or asks about network configuration, peer discovery, sync issues, forks, or network health.
version: 3.2.0
---

# DOLI Network Setup Skill

## Network Parameters

| Parameter | Devnet | Testnet | Mainnet |
|-----------|--------|---------|---------|
| **Network ID** | 99 | 2 | 1 |
| **Address Prefix** | `ddoli` | `tdoli` | `doli` |
| **Slot Duration** | 10 seconds | 10 seconds | 10 seconds |
| **Epoch Length** | 6 slots (1 min) | 360 slots (1 hr) | 360 slots (1 hr) |
| **P2P Port** | 50303 | 40303 | 30303 |
| **RPC Port** | 28545 | 18545 | 8545 |
| **Bootstrap** | None (local DHT) | `testnet.doli.network` | `doli.network` |
| **Block Reward** | 20 dDOLI | 1 tDOLI | 1 DOLI |
| **Bond Unit** | 1 DOLI | 10 DOLI | 10 DOLI |
| **ACTIVATION_DELAY** | 10 blocks (~100s) | 10 blocks (~100s) | 10 blocks (~100s) |
| **Genesis Bootstrap** | 24 blocks (4 min) | 60,480 blocks (~7 days) | 60,480 blocks (~7 days) |

## Network Architecture

DOLI uses libp2p with three protocols working together:

| Layer | Protocol | Role |
|-------|----------|------|
| **Discovery** | Kademlia DHT | Finds peers via distributed hash table random walks |
| **Propagation** | GossipSub | Publishes blocks/txs to mesh peers (mesh_n=6, max=12 per topic) |
| **Identity** | Identify | Exchanges listen addresses; feeds addresses into Kademlia |

**How peer discovery works:** Node A connects to bootstrap → Kademlia random walks find nodes B, C, D → GossipSub builds mesh overlay on top of connected peers → blocks propagate in O(log N) hops.

**GossipSub mesh pruning:** GossipSub maintains 6-12 peers per topic. When it prunes excess peers, Kademlia provides replacements. Without DHT, pruned nodes become isolated — this is why `--no-dht` is dangerous for networks with >12 nodes.

### Peer Discovery and Network Isolation

DHT (Kademlia) is **always enabled** on all networks. Network isolation happens via **network ID validation** — when a peer connects, the node checks `network_id` in the status handshake and disconnects mismatches. Devnet (ID=99), testnet (ID=2), and mainnet (ID=1) never cross-contaminate, even on the same machine with DHT active. This is how Ethereum, Polkadot, and Filecoin work.

**`--no-dht` exists as a debug/emergency flag but should NEVER be used in normal operation.** It disables Kademlia, which means:
- Nodes can ONLY connect to their `--bootstrap` peer (no discovery)
- GossipSub cannot graft replacements after mesh pruning
- Topology collapses to a star → networks >12 nodes will have isolated nodes
- This was the root cause of fork instability in early devnet testing

### Tiered Architecture

Producers self-classify into tiers based on bond weight at epoch boundaries:

| Tier | Bond Weight | GossipSub mesh_n | Min Peers to Produce |
|------|-------------|-------------------|---------------------|
| Tier 1 | Top 500 by weight | 20 (dense) | 10 |
| Tier 2 | Next tier | 8 (moderate) | 5 |
| Tier 3 | Light producers | 4 (light) | 2 |

Tier classification happens automatically. Initial min_peers before first epoch: 2 (all networks).

## Quick Reference

| Action | Command |
|--------|---------|
| **Local devnet (recommended)** | `doli-node devnet init --nodes 5` |
| Start local devnet | `doli-node devnet start` |
| Check devnet status | `doli-node devnet status` |
| Stop local devnet | `doli-node devnet stop` |
| **Add producer to devnet** | `doli-node devnet add-producer [--count N]` |
| Clean devnet data | `doli-node devnet clean [--keep-keys]` |
| Run single node | `doli-node --network <NETWORK> run` |
| Run as producer | `doli-node --network <NETWORK> run --producer --producer-key <wallet>` |
| Create wallet | `doli -w <wallet-path> new` |
| Check balance | `doli -w <wallet> balance` |
| Register producer | `doli -w <wallet> producer register --bonds 1` |

Replace `<NETWORK>` with `devnet`, `testnet`, or `mainnet`.

## Mandatory Rule: NEVER Reinitialize a Running Devnet

**Before ANY `devnet clean`, `devnet init`, or `rm -rf ~/.doli/devnet`, you MUST check if a devnet is already running.**

**Procedure (ALWAYS run first):**

```bash
# 1. Check if devnet is initialized
cat ~/.doli/devnet/devnet.toml 2>/dev/null

# 2. Check if nodes are running
pgrep -f "doli-node" 2>/dev/null

# 3. If EITHER returns results → devnet EXISTS. Check RPC health:
curl -s --max-time 3 http://127.0.0.1:28545 -X POST \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}'
```

**Decision matrix:**

| devnet.toml exists? | Nodes running? | RPC healthy? | Action |
|---------------------|----------------|--------------|--------|
| No | No | N/A | Safe to `devnet init` + `devnet start` |
| Yes | No | N/A | Run `devnet start` (do NOT reinit) |
| Yes | Yes | Yes | **USE AS-IS** — just run `add-producer` |
| Yes | Yes | No | Investigate logs before touching anything |

**NEVER do this:**
- `rm -rf ~/.doli/devnet` without checking first
- `devnet clean` when the user asked to add producers
- `devnet init` when devnet.toml already exists
- Kill all processes + reinit just because an `add-producer` batch partially failed

**When the user says "add N producers":** That means add to the EXISTING devnet. Only reinitialize if the user explicitly says "start fresh", "clean restart", or the devnet is confirmed broken beyond recovery.

## Mandatory Rule: Kill Zombies Before Deploy

**This rule applies to ALL scenarios below.** Before starting ANY node process, verify the target port range is free of zombie processes. Skipping this causes `Address already in use` panics.

**When to run:**
| Situation | Cleanup Needed? |
|-----------|-----------------|
| Fresh `devnet init --nodes N` | **YES** — previous devnet may have left zombies |
| `devnet start` (restart) | **YES** — previous `devnet stop` may have missed processes |
| `devnet add-producer` | **YES** — previous attempts may have left zombies |
| Manual node startup (any scenario) | **YES** |
| After crash or forced kill | **YES** |

**Procedure (run BEFORE any node launch):**

```bash
# 1. Determine port range for your deployment
#    Formula: P2P=50303+N, RPC=28545+N, Metrics=9090+N (devnet)
#    For N nodes (0 to N-1), check all three port types

# 2. Scan for zombie processes on target ports
echo "=== Checking for zombie doli-node processes ==="
ZOMBIES=$(pgrep -f "doli-node" 2>/dev/null)
if [ -n "$ZOMBIES" ]; then
  echo "Found doli-node processes: $ZOMBIES"
  ps -p $(echo "$ZOMBIES" | tr '\n' ',') -o pid,ppid,state,start,command 2>/dev/null
fi

# 3. Check specific port ranges (adjust for your node count)
#    Example: 10-node devnet uses ports 9090-9099, 28545-28554, 50303-50312
for port_range in "9090-9099" "28545-28554" "50303-50312"; do
  occupied=$(lsof -i :$port_range 2>/dev/null | grep LISTEN)
  if [ -n "$occupied" ]; then
    echo "⚠️  OCCUPIED in range $port_range:"
    echo "$occupied"
  fi
done

# 4. Stop nodes via systemd (NEVER use kill/pkill on production nodes)
# Production (mainnet/testnet):
sudo systemctl stop doli-mainnet-nodeN

# Devnet (local testing only — pkill is acceptable here):
pkill -f "doli-node.*devnet" 2>/dev/null || true

# 5. Wait and verify
sleep 2
remaining=$(pgrep -f "doli-node" 2>/dev/null)
if [ -n "$remaining" ]; then
  echo "❌ Still running: $remaining"
else
  echo "✅ All ports clear — safe to deploy"
fi
```

**Port ranges by network:**
| Network | P2P Range | RPC Range | Metrics Range |
|---------|-----------|-----------|---------------|
| Devnet (N nodes) | 50303–50303+N | 28545–28545+N | 9090–9090+N |
| Testnet (N nodes) | 40303–40303+N | 18545–18545+N | 9090–9090+N |
| Mainnet | 30303 | 8545 | 9090 |
## Decision Tree

```
User wants to...
│
├─ STEP 0 (ALWAYS): Check if devnet already exists and is running
│  └─ Run: cat ~/.doli/devnet/devnet.toml && doli-node devnet status
│  └─ If running and healthy → SKIP init/clean, go straight to add-producer
│  └─ If exists but stopped → devnet start (do NOT reinit)
│  └─ If does not exist → safe to devnet init + devnet start
│
├─ STEP 1: Kill zombies ONLY on target port range (NOT all processes)
│  └─ For add-producer: only clean NEW ports, leave running nodes alone
│  └─ For fresh init: clean ALL ports
│
├─ Add new producers to running devnet?
│  └─ Check existing devnet → `doli-node devnet add-producer --count N`
│  └─ If add-producer partially fails → retry failed ones, do NOT reinit
│
├─ Local development/testing (fresh)?
│  └─ Only if no devnet exists → `doli-node devnet init --nodes 5`
│     doli-node devnet start (DHT enabled by default)
│
├─ Public testing with other operators?
│  └─ Kill zombies → Use testnet (mirrors mainnet timing)
│
├─ Production deployment?
│  └─ Kill zombies → Use mainnet
│
├─ Run as background service?
│  └─ Kill zombies → See Scenario 4 (Systemd Service)
│
├─ Network health issues (forks, stuck nodes, sync failures)?
│  └─ See Network Health Monitoring + Troubleshooting
│
└─ Launch a brand new network?
   └─ Kill zombies → See Scenario 5 (Network Operators)
```

## Scenario 1: Run a Producer Node

### Step 1: Build DOLI

```bash
nix --extra-experimental-features "nix-command flakes" develop --command bash -c "cargo build --release"
```

### Step 2: Create Producer Wallet

```bash
mkdir -p ~/.doli/<NETWORK>
./target/release/doli -w ~/.doli/<NETWORK>/producer.json new
./target/release/doli -w ~/.doli/<NETWORK>/producer.json info
```

### Step 3: Open Firewall (testnet/mainnet only)

| Network | Command |
|---------|---------|
| Devnet | Not needed (local) |
| Testnet | `sudo ufw allow 40303/tcp comment 'DOLI Testnet P2P'` |
| Mainnet | `sudo ufw allow 30303/tcp comment 'DOLI Mainnet P2P'` |

### Step 4: Run Producer Node

**⚠️ FIRST: Run zombie cleanup (see "Mandatory Rule" above) before starting any node.**

**Devnet (recommended):** Use the devnet subcommands — see **Scenario 2 Option A**. They handle keys, chainspec, ports, and PID tracking automatically. Do NOT manually start devnet nodes unless you have a specific reason.

**Testnet/Mainnet:**
```bash
./target/release/doli-node --network <NETWORK> run \
    --producer --producer-key ~/.doli/<NETWORK>/producer.json
```

### Step 5: Register as Producer

```bash
# RPC ports: 28545 (devnet), 18545 (testnet), 8545 (mainnet)
./target/release/doli -r http://127.0.0.1:<RPC_PORT> -w ~/.doli/<NETWORK>/producer.json balance
./target/release/doli -r http://127.0.0.1:<RPC_PORT> -w ~/.doli/<NETWORK>/producer.json producer register --bonds 1
./target/release/doli -r http://127.0.0.1:<RPC_PORT> -w ~/.doli/<NETWORK>/producer.json producer status
```

**Bond cost:** Devnet = 1 DOLI per bond. Testnet/Mainnet = 10 DOLI per bond.

## Scenario 2: Local Multi-Node Devnet

### Option A: Built-in Devnet Commands (Recommended)

The `doli-node devnet` subcommands provide the easiest way to manage a local multi-node network:

**⚠️ FIRST: Run zombie cleanup (see "Mandatory Rule" above) before `init` or `start`.**

```bash
doli-node devnet init --nodes 10
doli-node devnet start
doli-node devnet status
doli-node devnet stop

# Add producers to a running devnet (creates wallet, funds, registers, starts node)
doli-node devnet add-producer --count 2

# Clean up devnet data (--keep-keys preserves wallet files)
doli-node devnet clean
doli-node devnet clean --keep-keys
```

**Directory structure:** `~/.doli/devnet/`
```
├── devnet.toml          # Config (node_count, base ports)
├── chainspec.json       # Genesis with all producers
├── keys/producer_*.json # Wallet files
├── data/node*/          # Node data directories
├── logs/node*.log       # Log files
└── pids/node*.pid       # PID tracking
```

**Port allocation:**
| Node | P2P Port | RPC Port | Metrics Port |
|------|----------|----------|--------------|
| 0 | 50303 | 28545 | 9090 |
| N | 50303+N | 28545+N | 9090+N |

### Option B: Manual Multi-Node Setup (Private Local Testnet)

Use this only when you need a **private** local testnet with custom chainspec:

```bash
export TESTNET_DIR=~/.doli/testnet
mkdir -p $TESTNET_DIR/keys $TESTNET_DIR/logs
mkdir -p $TESTNET_DIR/{node1,node2,node3,node4,node5}/data

for i in 1 2 3 4 5; do
    ./target/release/doli -w $TESTNET_DIR/keys/producer_$i.json new
done

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
    --no-auto-update
```

**Start Nodes 2-N:**
```bash
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
    --no-auto-update
```

**⚠️ WARNING:** `--no-dht` is used here ONLY to isolate from the public testnet (same network ID). This limits the network to ≤12 nodes reliably. For >12 nodes, use unique network ID or devnet commands instead.

**Key flags:**
- `--chainspec`: Custom genesis with your producer wallets
- `--no-auto-update`: Disable auto-updates during testing
- Network isolation is automatic via network ID — no need for `--no-dht`

### Check Multi-Node Status

```bash
for port in 18545 18546 18547 18548 18549; do
  echo "=== RPC $port ==="
  curl -s http://127.0.0.1:$port -X POST \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | \
    jq -c '.result | {height: .bestHeight, slot: .bestSlot}'
done
```

## Scenario 3: Add New Producers to Running Network

> **Routing:** On devnet? Use **Option A** (`devnet add-producer`) — it handles wallet creation, funding, registration, node startup, and PID tracking in one command. Only use **Option B** (manual) for testnet/mainnet or when you need non-standard configuration.

### Option A: Automated (Devnet Only — Always Prefer This)

One command to create a wallet, fund it, register as producer, and start the node:

```bash
# Add 1 producer (default)
doli-node devnet add-producer

# Add 3 producers at once
doli-node devnet add-producer --count 3
```

**What it does per producer:**
1. Generates wallet at `~/.doli/devnet/keys/producer_N.json`
2. Funds from producer_0 (needs sufficient balance)
3. Registers as producer with 1 bond
4. Creates data directory and starts node process
5. Saves PID (managed by `devnet stop/status`)

**Prerequisites:**
- Devnet must be running (`doli-node devnet start`)
- producer_0 must have enough balance to fund new producers (2x bond amount per producer)
- The `doli` CLI binary must be built (`cargo build --release`)

**Port allocation:** Same formula as init — P2P: 50303+N, RPC: 28545+N, Metrics: 9090+N

### Option B: Manual (Testnet/Mainnet or Custom Setup)

For adding producers to testnet/mainnet, or when you need full control over the process.

#### ⚠️ FIRST: Run zombie cleanup

**Run the "Mandatory Rule: Kill Zombies Before Deploy" procedure above**, targeting the port range for the new nodes you're adding. For manually started nodes, use Method B (surgical kill on specific ports) to avoid killing the running devnet nodes.

**Port allocation for dynamic nodes:**
| Node | P2P Port | RPC Port | Metrics Port |
|------|----------|----------|--------------|
| 5 | 50308 | 28550 | 9095 |
| 6 | 50309 | 28551 | 9096 |
| 7 | 50310 | 28552 | 9097 |
| N | 50303+N | 28545+N | 9090+N |

> **Note:** If you want devnet to manage all 10 nodes from the start, use `devnet init --nodes 10` instead of adding producers dynamically.

---

### ⚠️ CRITICAL: Complete 5-Step Workflow Required (Option B Only)

> **Using Option A (`devnet add-producer`)?** Skip this — it automates all 5 steps. This workflow is ONLY for manual producer setup (testnet/mainnet/custom).

**All 5 steps must be completed for a producer to actually produce blocks:**

| Step | Action | Result if Skipped |
|------|--------|-------------------|
| 0. **Clean ports** | Kill zombie processes | **NODE PANIC on startup** |
| 1. Create wallet | `doli wallet new` | No key exists |
| 2. Fund wallet | Send DOLI to address | Cannot register |
| 3. Register | `doli producer register` | Not in producer set |
| 4. **Start node** | `doli-node --producer --producer-key` | **REGISTERED BUT NOT PRODUCING** |

### Step 1: Create Wallets

```bash
for i in {15..29}; do
  ./target/release/doli -w ~/.doli/devnet/keys/producer_$i.json new -n "producer_$i"
done
```

### Step 2: Get Pubkey Hashes

**⚠️ Use "Pubkey Hash (32-byte)", NOT "Public Key" — both are 64 chars but different values!**

```bash
pubkey_hash=$(./target/release/doli -w ~/.doli/devnet/keys/producer_$i.json info 2>/dev/null | grep "Pubkey Hash (32-byte):" | sed 's/.*: //')
```

| Field | Length | Use For |
|-------|--------|---------|
| Address (20-byte) | 40 chars | Display only |
| **Pubkey Hash (32-byte)** | **64 chars** | **Sending coins, RPC queries** |
| Public Key | 64 chars | Verification only |

### Step 3: Fund Producers

```bash
# Use different source wallets to avoid UTXO double-spend errors
for i in {15..29}; do
  src=$((i - 15))
  pubkey=$(./target/release/doli -w ~/.doli/devnet/keys/producer_$i.json info 2>/dev/null | grep "Pubkey Hash (32-byte)" | sed 's/.*: //')
  ./target/release/doli -r http://127.0.0.1:28545 -w ~/.doli/devnet/keys/producer_$src.json send "$pubkey" 2
done
```

### Step 4: Register

```bash
for i in {15..29}; do
  ./target/release/doli -r http://127.0.0.1:28545 -w ~/.doli/devnet/keys/producer_$i.json producer register -b 1
done
```

### Step 5: Start Nodes (DO NOT use --no-dht)

```bash
for i in {15..29}; do
  P2P_PORT=$((50303 + i))
  RPC_PORT=$((28545 + i))
  METRICS_PORT=$((9090 + i))

  ./target/release/doli-node \
    --network devnet \
    --data-dir ~/.doli/devnet/data/node$i \
    run \
    --producer \
    --producer-key ~/.doli/devnet/keys/producer_$i.json \
    --p2p-port $P2P_PORT \
    --rpc-port $RPC_PORT \
    --metrics-port $METRICS_PORT \
    --bootstrap '/ip4/127.0.0.1/tcp/50303' \
    --chainspec ~/.doli/devnet/chainspec.json \
    --yes \
    > ~/.doli/devnet/logs/node$i.log 2>&1 &

  echo "Started node $i (P2P:$P2P_PORT RPC:$RPC_PORT Metrics:$METRICS_PORT)"
done
```

**⚠️ No `--no-dht` flag!** DHT enables peer discovery so nodes form a distributed mesh. Without it, all nodes only connect to the bootstrap node, creating a fragile star topology.

### Producer Activation Timeline

1. **Bootstrap Mode (immediate):** New producers join `known_producers` on block application.
2. **Normal Mode (after ACTIVATION_DELAY = 10 blocks):** Producer appears in `active_producers_at_height()`. ~100 seconds on all networks (10 blocks × 10s slots).

## Scenario 4: Systemd Service (Production)

```bash
# List all active producers
./target/release/doli -r http://127.0.0.1:28545 producer list

# Check specific producer status
./target/release/doli -r http://127.0.0.1:28545 -w ~/.doli/devnet/keys/producer_15.json producer status

# Check if new producers are producing blocks
for i in {15..29}; do
  RPC=$((28545 + i))
  height=$(curl -s http://127.0.0.1:$RPC -X POST -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' | jq -r '.result.bestHeight')
  echo "Node $i: Height $height"
done
```

---

## Scenario 3b: Remove a Producer from Running Network

### ⚠️ CRITICAL: Always Exit Before Killing

**NEVER kill a producer node or delete its wallet key before submitting an exit transaction.** A registered producer with no running node will still be selected by the scheduler but cannot produce blocks. If the producer holds many bonds, this can **halt the entire chain** because the scheduler assigns it a proportional share of slots that go unproduced.

**Correct removal order: Exit → Confirm → Kill → Clean**

```bash
# 1. Submit exit transaction FIRST (while node is still running)
./target/release/doli -r http://127.0.0.1:28545 \
  -w ~/.doli/devnet/keys/producer_N.json producer exit

# 2. Wait for exit confirmation (producer removed from scheduler)
sleep 10
./target/release/doli -r http://127.0.0.1:28545 \
  -w ~/.doli/devnet/keys/producer_N.json producer status
# Should show "exiting" or "exited"

# 3. THEN stop the node (devnet: pkill is acceptable; production: use systemctl)
pkill -f "doli-node.*50309"  # devnet only — use the node's P2P port

# 4. THEN clean up data and keys
rm -rf ~/.doli/devnet/data/nodeN
rm -f ~/.doli/devnet/logs/nodeN.log
rm -f ~/.doli/devnet/keys/producer_N.json
```

**What happens if you skip the exit:**

| Action | Consequence |
|--------|-------------|
| Kill node without exit | Producer stays in scheduler, assigned slots produce no blocks |
| Delete wallet without exit | **Permanent ghost producer** — cannot exit, cannot produce, occupies scheduler slots forever |
| Ghost producer with many bonds | **Chain halt** — if ghost holds >50% of bond weight, majority of slots go empty and liveness checks block all other producers |

**Recovery from ghost producer (devnet only):** Restart the devnet (`devnet stop && devnet clean && devnet init && devnet start`). There is no on-chain mechanism to remove a producer without its private key.

---

## Scenario 4: Run as Systemd Service (Production)

For persistent production operation (testnet/mainnet).

```bash
sudo tee /etc/systemd/system/doli-<NETWORK>.service > /dev/null << 'EOF'
[Unit]
Description=DOLI Producer
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

sudo sed -i "s/YOUR_USER/$USER/g" /etc/systemd/system/doli-producer.service
sudo systemctl daemon-reload
sudo systemctl enable doli-producer
sudo systemctl start doli-producer
journalctl -u doli-producer -f
```

## Scenario 5: Launch New Network

```bash
mkdir -p ~/.doli/genesis
for i in 1 2 3 4 5; do
    ./target/release/doli -w ~/.doli/genesis/producer_$i.json new
done
./scripts/generate_chainspec.sh <NETWORK> ~/.doli/genesis chainspec.json
./target/release/doli-node --network <NETWORK> --chainspec chainspec.json run \
    --producer --producer-key ~/.doli/genesis/producer_1.json
```

## Network Health Monitoring

### Quick Health Check (all nodes)

```bash
# Check height, slot, peers, sync status for all devnet nodes
for i in $(seq 0 $(($(cat ~/.doli/devnet/devnet.toml 2>/dev/null | grep node_count | sed 's/[^0-9]//g') - 1))); do
  RPC=$((28545 + i))
  result=$(curl -s http://127.0.0.1:$RPC -X POST -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}' 2>/dev/null)
  chain=$(curl -s http://127.0.0.1:$RPC -X POST -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' 2>/dev/null)
  peers=$(echo $result | jq -r '.result.peer_count // "DOWN"')
  height=$(echo $chain | jq -r '.result.bestHeight // "DOWN"')
  hash=$(echo $chain | jq -r '.result.bestHash // "?"' | head -c 16)
  echo "Node $i: height=$height peers=$peers hash=$hash..."
done
```

**What to look for:**
- All nodes should have similar heights (within 2-3 blocks)
- All nodes should share the same `bestHash` at the same height (different hash = fork)
- All nodes should have peers > 0 (0 peers = isolated, won't produce)
- Devnet nodes should have >1 peer (just 1 peer = star topology, fragile)

### Detecting Forks

```bash
# Compare best hashes across all nodes at similar heights
heights=()
hashes=()
for i in $(seq 0 4); do
  RPC=$((28545 + i))
  chain=$(curl -s http://127.0.0.1:$RPC -X POST -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' 2>/dev/null)
  h=$(echo $chain | jq -r '.result.bestHeight')
  hash=$(echo $chain | jq -r '.result.bestHash' | head -c 16)
  echo "Node $i: height=$h hash=$hash"
done
# If nodes at similar height have DIFFERENT hashes → fork detected
```

### Log Monitoring Keywords

```bash
# Watch for problems in a specific node's log
grep -E "fork|reorg|stuck|orphan|resync|disconnect|failed" ~/.doli/devnet/logs/node3.log | tail -20

# Watch for block production across all nodes
grep "Produced block\|Producing block" ~/.doli/devnet/logs/node*.log | tail -20

# Watch for peer connection events
grep "Connected to peer\|Disconnected from" ~/.doli/devnet/logs/node0.log | tail -10
```

### Available RPC Methods

| Method | Description |
|--------|-------------|
| `getChainInfo` | Chain tip: height, slot, hash, network |
| `getNetworkInfo` | Peer count, sync status, peer ID |
| `getBlockByHeight` | Block at specific height |
| `getBlockByHash` | Block by hash |
| `getBalance` | Address balance |
| `getUtxos` | UTXOs for address |
| `getProducers` | All registered producers |
| `getProducer` | Specific producer info |
| `getMempoolInfo` | Mempool statistics |
| `getEpochInfo` | Current reward epoch info |
| `sendTransaction` | Submit signed transaction |
| `getNodeInfo` | Node version info |

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

### Node panics: "Address already in use"

**Cause:** Zombie process holding a port. **Fix:**
```bash
lsof -i :<PORT>          # Find PID
kill <PID>               # Kill it
```

**Prevention:** Always run the "Mandatory Rule: Kill Zombies Before Deploy" procedure before starting new producer nodes.

### Node stuck at height 0 or low height

**Possible causes:**
1. **No peers:** Check `getNetworkInfo` → `peer_count`. If 0, node is isolated. Check bootstrap address and firewall.
2. **Sync stuck in Processing state:** The sync manager can get stuck if it downloaded headers but can't apply them (missing parent chain). The node receives gossip blocks which reset the "stale chain" timer, preventing timeout recovery. Check logs for repeated "Processing" state without height advancement.
3. **Legacy `--no-dht` usage:** If the node was started with `--no-dht`, it can't discover peers beyond its bootstrap. Remove the flag and restart. DHT is safe on all networks — isolation is handled by network ID.

### Nodes forked (different hashes at same height)

**Diagnosis:** Run the fork detection check above. If nodes show different hashes at similar heights:

1. **Check peer counts:** Isolated nodes (0-1 peers) fork easily because they don't see competing blocks in time
2. **Check topology:** If all nodes have exactly 1 peer, you have a star topology (all through bootstrap). Enable DHT.
3. **Recovery:** Nodes with fork recovery will attempt to follow the heavier chain automatically. If a node is deeply forked, restart it with clean data and let it resync.

### Not producing blocks

1. Ensure `--producer` flag is set
2. Check sync: node must be synced to tip before producing
3. Check peers: node needs ≥ min_peers (2 initially, tier-dependent after first epoch)
4. Check registration: `doli producer status`
5. Wait for ACTIVATION_DELAY (10 blocks, ~100s on all networks)
6. For manual local testnet: ensure `--chainspec` points to genesis with your producer wallets

### Double spend errors sending multiple transactions

**Cause:** UTXO reuse when sending from same wallet in quick succession.
**Fix:** Use different source wallets, or wait one block (~10s) between sends from the same wallet.

### Chain halted after killing a producer node

**Symptom:** All nodes stuck at the same height, logs show `BlockedBehindPeers` with `height_diff: 0`, slots keep advancing but no blocks produced.

**Cause:** A registered producer was killed without submitting an exit transaction first. The scheduler keeps selecting it for slots proportional to its bond count, but no node produces those blocks. If the dead producer holds many bonds (e.g., 10 out of 16 total), the majority of slots go empty and liveness checks block all remaining producers.

**Prevention:** Always follow the exit-before-kill procedure in Scenario 3b.

**Recovery (devnet):**
```bash
doli-node devnet stop
doli-node devnet clean
doli-node devnet init --nodes N
doli-node devnet start
```

**Recovery (testnet/mainnet):** If the wallet key still exists, restart the node or submit an exit transaction. If the wallet key was deleted, the ghost producer persists until governance intervention.

---

### Sent funds but recipient balance is 0

**Cause:** Used "Public Key" instead of "Pubkey Hash (32-byte)" — both 64 chars but different values.
```bash
# CORRECT
pubkey_hash=$(doli -w wallet.json info | grep "Pubkey Hash (32-byte):" | sed 's/.*: //')
```

### Node won't sync (testnet/mainnet)

```bash
nc -zv testnet.doli.network 40303
sudo ufw status
```

## Server Requirements

| Requirement | Minimum |
|-------------|---------|
| OS | Ubuntu 22.04+ or similar Linux |
| CPU | 2+ cores |
| RAM | 4 GB |
| Storage | 50 GB SSD |
| Network | P2P port open (see Network Parameters table) |

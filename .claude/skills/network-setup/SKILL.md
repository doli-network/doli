---
name: network-setup
description: Use this skill when the user wants to set up a node, create a producer, join a network (devnet/testnet/mainnet), run a node, become a producer, or asks about network configuration.
version: 3.2.0
---

# DOLI Network Setup Skill

This skill guides you through setting up and running DOLI nodes and producers on any network.

## Network Parameters

| Parameter | Devnet | Testnet | Mainnet |
|-----------|--------|---------|---------|
| **Network ID** | 99 | 2 | 1 |
| **Address Prefix** | `ddoli` | `tdoli` | `doli` |
| **Slot Duration** | 10 seconds | 10 seconds | 10 seconds |
| **Epoch Length** | 60 slots | 360 slots | 360 slots |
| **P2P Port** | 50303 | 40303 | 30303 |
| **RPC Port** | 28545 | 18545 | 8545 |
| **Bootstrap** | None (local) | `testnet.doli.network` | `doli.network` |
| **Block Reward** | 1 dDOLI | 1 tDOLI | 1 DOLI |
| **Bond Unit** | 1 DOLI | 100 DOLI | 100 DOLI |
| **ACTIVATION_DELAY** | 10 blocks (~100s) | 10 blocks (~100s) | 10 blocks (~100s) |

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

# 4. Kill zombies (choose one method)
# Method A: Kill ALL doli-node processes (use when doing fresh deploy)
pkill -f "doli-node" 2>/dev/null && echo "Killed all doli-node processes" || echo "No doli-node processes found"

# Method B: Kill specific port (surgical, for adding nodes to running network)
# pid=$(lsof -ti :9095 2>/dev/null); [ -n "$pid" ] && kill $pid

# 5. Wait and verify
sleep 2
remaining=$(pgrep -f "doli-node" 2>/dev/null)
if [ -n "$remaining" ]; then
  echo "❌ Still running: $remaining — use kill -9 if needed"
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
│     doli-node devnet start
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
└─ Launch a brand new network?
   └─ Kill zombies → See Scenario 5 (Network Operators)
```

## Scenario 1: Run a Producer Node

### Step 1: Build DOLI

```bash
# Enter Nix environment
`nix --extra-experimental-features "nix-command flakes" develop --command bash -c "<command>"`

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

**⚠️ FIRST: Run zombie cleanup (see "Mandatory Rule" above) before starting any node.**

**Devnet (recommended):** Use the devnet subcommands — see **Scenario 2 Option A**. They handle keys, chainspec, ports, and PID tracking automatically. Do NOT manually start devnet nodes unless you have a specific reason.

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

## Scenario 2: Local Multi-Node Devnet

For development and testing with multiple nodes on a single machine.

### Option A: Built-in Devnet Commands (Recommended)

The `doli-node devnet` subcommands provide the easiest way to manage a local multi-node network:

**⚠️ FIRST: Run zombie cleanup (see "Mandatory Rule" above) before `init` or `start`.**

```bash
# Initialize a 10-node devnet (generates keys, chainspec, directories)
doli-node devnet init --nodes 10

# Start all nodes (handles bootstrap, port allocation, --force-start)
doli-node devnet start

# Check status (shows running/stopped, height, slot, peers)
doli-node devnet status

# Stop all nodes gracefully
doli-node devnet stop

# Add producers to a running devnet (creates wallet, funds, registers, starts node)
doli-node devnet add-producer --count 2

# Clean up devnet data (--keep-keys preserves wallet files)
doli-node devnet clean
doli-node devnet clean --keep-keys
```

**Directory structure created at `~/.doli/devnet/`:**
```
~/.doli/devnet/
├── devnet.toml          # Config (node_count, base ports)
├── chainspec.json       # Genesis with all producers
├── keys/producer_*.json # Wallet files (compatible with doli-cli)
├── data/node*/          # Node data directories
├── logs/node*.log       # Log files
└── pids/node*.pid       # PID tracking
```

**Port allocation (automatic):**
| Node | P2P Port | RPC Port | Metrics Port |
|------|----------|----------|--------------|
| 0 | 50303 | 28545 | 9090 |
| 1 | 50304 | 28546 | 9091 |
| N | 50303+N | 28545+N | 9090+N |

### Option B: Manual Multi-Node Setup

For more control (e.g., custom ports, specific configuration):

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

> **Note:** For general local development, prefer `doli-node devnet` commands over these scripts.
> These scripts are for specific test scenarios.

| Script | Description |
|--------|-------------|
| `scripts/launch_testnet.sh` | Quick 2-node devnet (legacy) |
| `scripts/test_3node_proportional_rewards.sh` | 3-node reward testing |
| `scripts/test_5node_epoch_rewards_consistency.sh` | 5-node epoch rewards |
| `scripts/test_devnet_3node_rewards.sh` | 3-node devnet rewards |

---

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
| 0. **Clean ports** | Kill zombie processes, verify ports free | **NODE PANIC on startup** |
| 1. Create wallet | `doli wallet new` | No key exists |
| 2. Fund wallet | Send DOLI to address | Cannot register (no bond) |
| 3. Register | `doli producer register` | Not in producer set |
| 4. **Start node** | `doli-node --producer --producer-key <wallet>` | **REGISTERED BUT NOT PRODUCING** |

**Common Mistakes:**
- **Skipping Step 0:** Node panics with "Address already in use" because zombie processes hold ports
- **Skipping Step 4:** Registration puts the public key on the blockchain, but without a running node there's no process to produce blocks

### Step 1: Create Producer Wallets

```bash
# Create wallets for producers 15-29 (example: 15 new producers)
for i in {15..29}; do
  ./target/release/doli -w ~/.doli/devnet/keys/producer_$i.json new -n "producer_$i"
done
```

### Step 2: Get Pubkey Hashes (Required for Sending)

**⚠️ CRITICAL: Use "Pubkey Hash", NOT "Public Key"**

The `doli info` command shows THREE different values. You MUST use the **Pubkey Hash (32-byte)** for sending:

```bash
./target/release/doli -w ~/.doli/devnet/keys/producer_15.json info
# Output:
#   Address (20-byte):     cf98716522ee9e5c...              ❌ DON'T USE (too short)
#   Pubkey Hash (32-byte): cf98716522ee9e5c62f9...686eab84  ✅ USE THIS FOR SENDING
#   Public Key:            cc9a1710b8bffb38...22d7cb51      ❌ DON'T USE (wrong hash)
```

| Field | Length | Use For |
|-------|--------|---------|
| Address (20-byte) | 40 chars | Display only |
| **Pubkey Hash (32-byte)** | **64 chars** | **Sending coins, RPC queries** |
| Public Key | 64 chars | Verification only |

**Common Mistake:** Using "Public Key" instead of "Pubkey Hash" - both are 64 characters but they are DIFFERENT values. The send will succeed but coins go to wrong address!

**Extract Pubkey Hash in scripts:**
```bash
# Correct way to get pubkey hash for sending
pubkey_hash=$(./target/release/doli -w ~/.doli/devnet/keys/producer_$i.json info 2>/dev/null | grep "Pubkey Hash (32-byte):" | sed 's/.*: //')
```

### Step 3: Fund New Producers

**UTXO Reuse Warning:** Sending multiple transactions from the same wallet in quick succession causes "double spend with mempool transaction" errors because the UTXO set isn't refreshed.

**Solution:** Use different source wallets for each send, or wait for confirmation between sends.

```bash
# Fund each new producer from a DIFFERENT source wallet
for i in {15..29}; do
  src=$((i - 15))  # producer_0 sends to 15, producer_1 sends to 16, etc.
  pubkey=$(./target/release/doli -w ~/.doli/devnet/keys/producer_$i.json info 2>/dev/null | grep "Pubkey Hash (32-byte)" | sed 's/.*: //')
  echo "Sending from producer_$src to producer_$i..."
  ./target/release/doli -r http://127.0.0.1:28545 -w ~/.doli/devnet/keys/producer_$src.json send "$pubkey" 2
done
```

**Alternative (slower but uses single wallet):**
```bash
# Wait for confirmation between each send
for i in {15..29}; do
  pubkey=$(./target/release/doli -w ~/.doli/devnet/keys/producer_$i.json info 2>/dev/null | grep "Pubkey Hash (32-byte)" | sed 's/.*: //')
  ./target/release/doli -r http://127.0.0.1:28545 -w ~/.doli/devnet/keys/producer_0.json send "$pubkey" 2
  sleep 12  # Wait for next block
done
```

### Step 4: Register as Producers

**Bond requirements:**
- Devnet: 1 DOLI per bond
- Testnet/Mainnet: 100 DOLI per bond

```bash
# Register all new producers
for i in {15..29}; do
  echo "Registering producer_$i..."
  ./target/release/doli -r http://127.0.0.1:28545 -w ~/.doli/devnet/keys/producer_$i.json producer register -b 1
done
```

### Step 5: Start Producer Nodes (CRITICAL)

**⚠️ This step is REQUIRED. Without a running node, the registered producer cannot produce blocks.**

**⚠️ IMPORTANT: Always specify `--metrics-port` explicitly to avoid port conflicts!**

```bash
# Start each new producer node with UNIQUE ports for P2P, RPC, AND METRICS
for i in {15..29}; do
  P2P_PORT=$((50303 + i))
  RPC_PORT=$((28545 + i))
  METRICS_PORT=$((9090 + i))  # REQUIRED: unique metrics port per node

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
    --no-dht \
    --yes \
    > ~/.doli/devnet/logs/node$i.log 2>&1 &

  echo "Started node $i (P2P: $P2P_PORT, RPC: $RPC_PORT, Metrics: $METRICS_PORT)"
done
```

**Each new producer needs THREE unique ports:**
| Port Type | Formula | Example (node 15) |
|-----------|---------|-------------------|
| P2P | 50303 + N | 50318 |
| RPC | 28545 + N | 28560 |
| **Metrics** | 9090 + N | 9105 |

**Common failure:** Omitting `--metrics-port` causes nodes to use default port 9090, conflicting with node 0.

### Producer Activation Timeline

After registration, producers go through two phases:

1. **Bootstrap Mode (immediate):** New producers are added to `known_producers` on block application. They can be selected via round-robin immediately.

2. **Normal Mode (after ACTIVATION_DELAY):** Requires 10 blocks (~100 seconds) before producer appears in `active_producers_at_height()`. This ensures all nodes have time to sync the registration transaction.

### Verify Producer Status

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

# 3. THEN kill the node process
pkill -f "doli-node.*50309"  # use the node's P2P port

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

## Scenario 5: Launch New Network (Network Operators Only)

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

### Node panics on startup: "Address already in use"

**Symptom:** Node crashes immediately with:
```
Failed to bind metrics server: Os { code: 48, kind: AddrInUse, message: "Address already in use" }
```

**Cause:** Another process (often a zombie from a previous failed attempt) is holding the metrics port.

**Solution:**
```bash
# 1. Find what's using the port (e.g., metrics port 9095)
lsof -i :9095

# 2. Kill the process
kill <PID>

# 3. Or kill by pattern
pkill -f "doli-node.*metrics.*9095"

# 4. Always specify unique metrics port when starting nodes
--metrics-port $((9090 + NODE_NUMBER))
```

**Prevention:** Always run the "Mandatory Rule: Kill Zombies Before Deploy" procedure before starting new producer nodes.

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
6. **After registration**: Wait for ACTIVATION_DELAY (10 blocks, ~100 seconds) for normal mode scheduling
7. **Bootstrap mode**: New producers are added to round-robin immediately on registration (as of commit TBD)

### Producer registered but not producing (balance not increasing)

**Symptom:** Producer shows in `producer list`, registration completed, but balance stuck at initial amount (no rewards).

**Cause:** Node not running with the producer's private key.

| What you did | Result |
|--------------|--------|
| Steps 1-3 only (wallet, fund, register) | Producer registered but **no blocks produced** |
| Steps 1-4 (wallet, fund, register, **start node**) | Producer registered **and producing blocks** |

**Solution:** Start a node with the producer key:
```bash
doli-node --network devnet run \
    --producer \
    --producer-key ~/.doli/devnet/keys/producer_NEW.json \
    --p2p-port <UNIQUE_PORT> \
    --rpc-port <UNIQUE_PORT> \
    --bootstrap /ip4/127.0.0.1/tcp/50303 \
    --chainspec ~/.doli/devnet/chainspec.json \
    --yes
```

**Verification:**
```bash
# Watch for production in logs
grep "Producing block" ~/.doli/devnet/logs/node_NEW.log

# Check balance is increasing (rewards)
doli -w ~/.doli/devnet/keys/producer_NEW.json -r http://127.0.0.1:28545 wallet balance
```

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

**Symptom:** Transaction succeeds but recipient wallet shows 0 balance.

**Cause:** Used "Public Key" instead of "Pubkey Hash (32-byte)" as recipient address. Both are 64 characters but are DIFFERENT values - coins went to wrong address.

**Prevention:**
```bash
# WRONG - using Public Key field
pubkey=$(doli -w wallet.json info | grep "Public Key" | awk '{print $3}')

# CORRECT - using Pubkey Hash field
pubkey_hash=$(doli -w wallet.json info | grep "Pubkey Hash (32-byte):" | sed 's/.*: //')
```

**Recovery:** Funds sent to wrong address are lost unless you control that address.

### Double spend errors when sending multiple transactions

**Symptom:** "RPC error -32603: double spend with mempool transaction"

**Cause:** The wallet reuses the same UTXO when sending multiple transactions in quick succession because the local UTXO cache isn't refreshed.

**Solutions:**
1. **Use different source wallets** for each transaction
2. **Wait for confirmation** (one block, ~10 seconds) between transactions from same wallet
3. **Split funds first** into multiple UTXOs if you need to send many transactions quickly

### Registration succeeds but producer not in list

**Cause:** `ACTIVATION_DELAY` of 10 blocks before producer appears in scheduler.

**Check:**
```bash
# View your registration
./target/release/doli -r http://127.0.0.1:28545 producer list
# Your producer should show "active" but may not produce until 10 blocks pass
```

### Insufficient balance for bond

**Check bond requirements:**
- Devnet: 1 DOLI per bond
- Testnet/Mainnet: 100 DOLI per bond

```bash
# Check balance
./target/release/doli -r http://127.0.0.1:28545 -w <wallet> balance
# Need at least bond_unit + fees
```

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

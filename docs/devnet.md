# DOLI Devnet Guide

Official guide for DOLI Devnet - the high-speed local development environment.

---

## 1. Overview

Devnet is designed for rapid iteration and local testing. Unlike Mainnet/Testnet which are persistent public networks, Devnet is usually ephemeral and runs locally or in a private cluster.

| Feature | Devnet | Mainnet/Testnet | Reason |
|---------|--------|-----------------|--------|
| **Slot Duration** | **10 seconds** (same as mainnet) | 10 seconds | Realistic testing (configurable via env) |
| **Genesis** | **Dynamic** | Fixed | Starts when *you* start |
| **VDF** | **Simulated** | Real Hardware | Instant blocks (no CPU burn) |
| **Bootstrapping** | **Sync-Before-Produce** | Hardcoded Seeds | Prevents split-brain genesis |

---

## 2. Bootstrap Mode (Sync-Before-Produce)

**This is the most critical difference in Devnet behavior.**

On Mainnet, the genesis block is hardcoded. On Devnet, the **first node to start creates the genesis block**. This creates a risk: if two nodes start at the same time without seeing each other, they might both create *different* genesis blocks, creating two incompatible networks ("split-brain").

To prevent this, Devnet nodes use **Sync-Before-Produce** logic:

1.  **Startup**: Node starts and enters "Bootstrap Mode".
2.  **Discovery**: It waits (default 5s) to find any peers.
3.  **Decision**:
    *   **If Peers Found**: It **pauses production** and syncs from them. It will *never* mine genesis.
    *   **If No Peers**: It assumes it is the first node ("Seed") and **mines the genesis block**.

### Implications for Developers

*   **"Stuck" at Height 0**: If you start a node with `--bootstrap` pointing to a peer that is offline, your node will *wait indefinitely* effectively. It won't mine genesis because you told it a peer *should* exist.
*   **Race Conditions**: If you script 5 nodes to start instantly in parallel, they might miss each other during the 1-2s discovery window. **Always start the seed node first**, wait 2 seconds, then start the rest.

---

## 3. Running a Local Devnet

### Option A: Automated (Recommended)

The CLI has a built-in devnet manager that handles ports, wallets, and configuration for you.

```bash
# 1. Initialize 5 nodes
doli-node devnet init --nodes 5

# 2. Start the cluster
doli-node devnet start

# 3. View status (PIDs, ports, heights)
doli-node devnet status

# 4. Stop
doli-node devnet stop

# 5. Clean up (wipes data)
doli-node devnet clean
```

### Option B: Manual (Advanced)

To run nodes manually (e.g., in separate terminals):

**Node 1 (The Seed)**
```bash
./doli-node --network devnet run \
    --data-dir ~/.doli/devnet-1 \
    --p2p-port 50301 --rpc-port 28541 \
    --producer --producer-key producer_1.json
```

**Node 2 (The Peer)**
```bash
# Must wait for Node 1 to be ready!
./doli-node --network devnet run \
    --data-dir ~/.doli/devnet-2 \
    --p2p-port 50302 --rpc-port 28542 \
    --producer --producer-key producer_2.json \
    --bootstrap /ip4/127.0.0.1/tcp/50301  # Connect to Node 1
```

---

## 4. Configuration Overrides

Devnet parameters are highly configurable via the `.env` file in your data directory (e.g., `~/.doli/devnet/.env`).

| Variable | Default (Devnet) | Description |
|----------|------------------|-------------|
| `DOLI_SLOT_DURATION` | `10` | Seconds per slot (same as mainnet) |
| `DOLI_VDF_ITERATIONS` | `1` | Block VDF iterations (1 = instant) |
| `DOLI_HEARTBEAT_VDF_ITERATIONS` | `1000` | Heartbeat VDF iterations |
| `DOLI_UNBONDING_PERIOD` | `60` | ~10 minutes with 10s slots |
| `DOLI_BLOCKS_PER_REWARD_EPOCH` | `4` | ~40 seconds per epoch |

**Example Fast-Test Config:**
```bash
# ~/.doli/devnet/.env
DOLI_SLOT_DURATION=1
DOLI_UNBONDING_PERIOD=10    # 10 second exit
DOLI_COINBASE_MATURITY=5    # Spend rewards in 5 seconds
DOLI_VETO_PERIOD_SECS=60    # 1 minute upgrades
```

---

## 5. Troubleshooting

**"My devnet node isn't producing blocks!"**

1.  **Check Peers**: Run `curl -X POST http://localhost:28500 -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":[],"id":1}'`
    *   If `peerCount` is 0 and height is 0, you might be in the "Choice" phase of Bootstrap Mode.
2.  **Check Logs**:
    *   `"Waiting for peers..."` -> It's looking for sync targets.
    *   `"No peers found, assuming authority..."` -> It decided to mine genesis.
    *   `"Syncing..."` -> It found a peer and is downloading.

**"Why is Height < Slot? (e.g. Height 4 at Slot 8)"**

This "Gap" is the normal cost of safety. It happens because joining nodes **deliberately skip their turns** until they are sure they are on the right chain.

#### The "Gap" Visualization

```mermaid
sequenceDiagram
    autonumber
    participant Time as Time (Slot)
    participant Seed as Node 0 (Seed)
    participant Joiner as Node 1 (Joiner)
    participant Chain as Chain Height

    Note over Seed, Joiner: STARTUP (t=0)
    Seed->>Seed: No peers? Mines Genesis (H=0)
    Joiner->>Joiner: Bootstrapping... (Paused)

    Note over Time, Chain: SLOT 1 (Turn: Node 1)
    Time->>Joiner: It's your turn!
    Joiner->>Joiner: 🛑 BLOCKED (Safe Mode)
    Note right of Joiner: SKIPPED (Gap Created)
    Chain-->>Chain: Height stays 0

    Note over Time, Chain: SLOT 2 (Turn: Node 0)
    Time->>Seed: It's your turn!
    Seed->>Chain: Mines Block (Slot 2, H=1)
    
    Note over Time, Chain: SLOT 3 (Turn: Node 1)
    Time->>Joiner: It's your turn!
    Joiner->>Joiner: 🛑 BLOCKED (Height 1 < 3)
    Note right of Joiner: SKIPPED (Gap Widens)
    Chain-->>Chain: Height stays 1

    Note over Time, Chain: SLOT 4 (Turn: Node 0)
    Time->>Seed: It's your turn!
    Seed->>Chain: Mines Block (Slot 4, H=2)

    Note over Time, Chain: ...syncing...

    Note over Time, Chain: SLOT 8 (Turn: Node 1)
    Seed->>Chain: (Previous blocks mined...)
    Note right of Chain: Chain reaches Height 3
    Joiner->>Joiner: ✅ SAFE! (Height >= 3)
    Joiner->>Chain: Mines Block (Slot 8, H=4)
```

#### What happened here?
1.  **Slot 1**: Assigned to Node 1, but Node 1 was in "Safe Mode". **Result:** Empty slot.
2.  **Slot 3**: Assigned to Node 1, but chain was too short to trust (Height 1). **Result:** Empty slot.
3.  **Slot 8**: Chain is stable (Height 3+). Node 1 finally joins in.

**Result:** We reached **Slot 8**, but only have **4 Blocks** (Height 4). The missing blocks are the "safety tax" we pay to ensure Node 1 didn't accidentally fork the network at the start.

### Late-Joining Node Behavior

When a node joins a running network, it goes through these phases:

1. **Sync phase**: Downloads headers and bodies from peers until caught up
2. **Post-resync grace**: Brief grace period after resync completes
3. **Production phase**: Production is authorized if the node is within 2 slots of its best peer and has minimum peers connected

**Temporary forks are normal.** When new nodes join, 1-3 block forks that self-resolve within 30-60 seconds are expected behavior in any blockchain (Bitcoin, Ethereum, DOLI). The heaviest-chain fork choice rule resolves these automatically. Only persistent non-converging forks indicate a bug.

---

## 6. Adding New Producers to Running Devnet

To add a new producer to an existing devnet, you must complete **all 4 steps**:

### Step 1: Create New Wallet

```bash
doli --wallet ~/.doli/devnet/keys/producer_NEW.json wallet new
```

### Step 2: Fund the Wallet

From an existing producer with mature DOLI.

**⚠️ CRITICAL: Use "Pubkey Hash", NOT "Public Key"**

```bash
doli --wallet ~/.doli/devnet/keys/producer_NEW.json info
# Output shows THREE values - use the MIDDLE one:
#   Address (20-byte):     5977da862f2647b7...              ❌ DON'T USE
#   Pubkey Hash (32-byte): 5977da862f2647b7...97af8f20      ✅ USE THIS
#   Public Key:            cc9a1710b8bffb38...22d7cb51      ❌ DON'T USE (wrong hash!)
```

```bash
# Get new wallet's pubkey hash (CORRECT method)
NEW_ADDR=$(doli --wallet ~/.doli/devnet/keys/producer_NEW.json info | grep "Pubkey Hash (32-byte):" | sed 's/.*: //')

# Send from funded wallet (need 1+ DOLI for devnet bond)
doli --wallet ~/.doli/devnet/keys/producer_0.json \
    --rpc http://127.0.0.1:28500 \
    send $NEW_ADDR 10
```

**Common Mistake:** Using "Public Key" instead of "Pubkey Hash" - both are 64 characters but DIFFERENT values. Coins sent to wrong address are lost!

### Step 3: Register as Producer

```bash
doli --wallet ~/.doli/devnet/keys/producer_NEW.json \
    --rpc http://127.0.0.1:28500 \
    producer register
```

### Step 4: Start Node with New Key (CRITICAL)

```bash
doli-node run --network devnet \
    --producer \
    --producer-key ~/.doli/devnet/keys/producer_NEW.json \
    --p2p-port 50309 \
    --rpc-port 28551 \
    --metrics-port 9096 \
    --bootstrap /ip4/127.0.0.1/tcp/50300 \
    --chainspec ~/.doli/devnet/chainspec.json \
    --yes
```

**Each new producer needs unique ports** (p2p, rpc, metrics).

### Common Mistake

| What you did | Result |
|--------------|--------|
| Steps 1-3 only | Producer registered but **no blocks produced** |
| Steps 1-4 | Producer registered **and producing blocks** |

Registration puts the public key on the blockchain. The node with `--producer-key` uses the private key to sign blocks. Without step 4, there's no process to produce blocks when that producer is selected.

### Verification

```bash
# Check producer is in active set
doli --rpc http://127.0.0.1:28500 producer list

# Watch new node's logs for production
grep "Producing block" ~/.doli/devnet/logs/node_NEW.log

# Verify balance increasing (block rewards)
doli --wallet ~/.doli/devnet/keys/producer_NEW.json \
    --rpc http://127.0.0.1:28500 \
    wallet balance
```

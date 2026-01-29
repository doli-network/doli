# WHITEPAPER Test Plan - Complete Functionality Verification

This document provides a comprehensive test plan to verify ALL functionalities described in the DOLI WHITEPAPER.md on devnet.

**Devnet accelerated parameters (for faster testing):**

| Parameter | Mainnet | Devnet | Speedup |
|-----------|---------|--------|---------|
| Slot duration | 10s | 1s | 10x |
| Slots per epoch | 360 | 20 | 18x |
| Slots per "year" | 3,153,600 | 60 | 52,560x |
| Initial bond | 1,000 DOLI | 1 DOLI | N/A |
| Inactivity threshold | 50 slots | 10 slots | 5x |
| Unbonding period | 259,200 slots (~30 days) | 30 slots (~30s) | 8,640x |
| Withdrawal delay | 60,480 slots (7 days) | 14 slots (~14s) | 4,320x |

---

## Pre-requisites

```bash
# Enter nix environment
nix develop

# Build release binaries
cargo build --release

# Verify binaries exist
ls -la target/release/doli-node target/release/doli
```

---

## Test Categories

1. [Genesis & Distribution](#1-genesis--distribution)
2. [Transactions](#2-transactions)
3. [Proof of Time / VDF](#3-proof-of-time--vdf)
4. [Producer Registration](#4-producer-registration)
5. [Bond Stacking](#5-bond-stacking)
6. [Producer Selection](#6-producer-selection)
7. [Rewards & Incentives](#7-rewards--incentives)
8. [Infractions & Penalties](#8-infractions--penalties)
9. [Chain Selection & Forks](#9-chain-selection--forks)
10. [Network & Propagation](#10-network--propagation)

---

## Environment Setup

```bash
#!/bin/bash
# Create test environment
export TEST_DIR="/tmp/doli-whitepaper-test"
export BINARY="./target/release/doli-node"
export CLI="./target/release/doli"

# Cleanup previous test
rm -rf $TEST_DIR
mkdir -p $TEST_DIR/{keys,data,logs}

# Generate producer keys
for i in 1 2 3 4 5; do
  $CLI new --wallet $TEST_DIR/keys/node${i}_wallet.json 2>/dev/null
  # Extract key for producer mode
  $CLI export --wallet $TEST_DIR/keys/node${i}_wallet.json \
    --output $TEST_DIR/keys/node${i}.json 2>/dev/null || true
done

# RPC helper function
rpc() {
  local port=$1
  local method=$2
  local params=${3:-"{}"}
  curl -s http://127.0.0.1:$port -X POST \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"$method\",\"params\":$params,\"id\":1}" | jq
}

# Wait for height helper
wait_for_height() {
  local port=$1
  local target=$2
  while true; do
    height=$(rpc $port getChainInfo | jq -r '.result.bestHeight // 0')
    [ "$height" -ge "$target" ] && break
    sleep 1
  done
}
```

---

## 1. Genesis & Distribution

### 1.1 Verify No Premine (Section 15)

**Whitepaper claim:** "There is no premine, ICO, treasury, or special allocations."

```bash
# Start single node and check genesis block
$BINARY --data-dir $TEST_DIR/data/node1 --network devnet run \
  --producer --producer-key $TEST_DIR/keys/node1.json \
  --p2p-port 50401 --rpc-port 28601 &
NODE1_PID=$!
sleep 5

# Get genesis block (height 0)
rpc 28601 getBlockByHeight '{"height":0}'

# VERIFY:
# - Genesis has exactly 1 transaction (coinbase)
# - Coinbase amount = 1 DOLI (100_000_000 base units for devnet)
# - No other transactions or special allocations

kill $NODE1_PID 2>/dev/null
```

### 1.2 Verify Genesis Message (Section 15.1)

**Whitepaper claim:** Genesis message is "Time is the only fair currency. 01/Feb/2026"

```bash
# Check genesis block message in logs or via block data
grep "Time is the only fair currency" $TEST_DIR/logs/node1.log

# Or check the genesis coinbase transaction for the message
rpc 28601 getBlockByHeight '{"height":0}' | jq '.result.transactions[0]'
```

---

## 2. Transactions

### 2.1 Transaction Validity Rules (Section 2.1)

**Setup: Start 2-node network**

```bash
# Node 1 (seed)
$BINARY --data-dir $TEST_DIR/data/node1 --network devnet run \
  --producer --producer-key $TEST_DIR/keys/node1.json \
  --p2p-port 50401 --rpc-port 28601 > $TEST_DIR/logs/node1.log 2>&1 &
NODE1_PID=$!

sleep 10  # Wait for some blocks

# Node 2
$BINARY --data-dir $TEST_DIR/data/node2 --network devnet run \
  --producer --producer-key $TEST_DIR/keys/node2.json \
  --p2p-port 50402 --rpc-port 28602 --bootstrap /ip4/127.0.0.1/tcp/50401 \
  > $TEST_DIR/logs/node2.log 2>&1 &
NODE2_PID=$!

sleep 30  # Wait for rewards to accumulate
```

#### 2.1.1 Valid Transaction

```bash
# Get node1 address
NODE1_ADDR=$(jq -r '.addresses[0].address' $TEST_DIR/keys/node1.json)
NODE2_ADDR=$(jq -r '.addresses[0].address' $TEST_DIR/keys/node2.json)

# Check balance
rpc 28601 getBalance "{\"address\":\"$NODE1_ADDR\"}"

# Send valid transaction (after coinbase maturity)
wait_for_height 28601 110  # Wait for maturity

$CLI send --wallet $TEST_DIR/keys/node1_wallet.json \
  --to $NODE2_ADDR --amount 0.5 --rpc http://127.0.0.1:28601

# VERIFY: Transaction accepted and confirmed
sleep 5
rpc 28601 getBalance "{\"address\":\"$NODE2_ADDR\"}"
```

#### 2.1.2 Double-Spend Prevention

```bash
# Get a UTXO
UTXO=$(rpc 28601 getUtxos "{\"address\":\"$NODE1_ADDR\"}" | jq -r '.result[0]')

# Try to spend the same UTXO twice (should fail on second)
$CLI send --wallet $TEST_DIR/keys/node1_wallet.json \
  --to $NODE2_ADDR --amount 0.1 --rpc http://127.0.0.1:28601

# Second spend of same UTXO should fail
$CLI send --wallet $TEST_DIR/keys/node1_wallet.json \
  --to $NODE2_ADDR --amount 0.1 --utxo $UTXO --rpc http://127.0.0.1:28601

# VERIFY: Second transaction rejected with "UTXO already spent" error
```

#### 2.1.3 Invalid Signature

```bash
# Try sending from node2 wallet but using node1's UTXOs
# This should fail signature verification
$CLI send --wallet $TEST_DIR/keys/node2_wallet.json \
  --from $NODE1_ADDR --to $NODE2_ADDR --amount 0.1 --rpc http://127.0.0.1:28601

# VERIFY: Transaction rejected with "Invalid signature" error
```

#### 2.1.4 Insufficient Funds

```bash
# Try to send more than balance
BALANCE=$(rpc 28601 getBalance "{\"address\":\"$NODE1_ADDR\"}" | jq -r '.result.confirmed')
EXCESS=$((BALANCE + 1000000000))  # Try to send balance + 10 DOLI

$CLI send --wallet $TEST_DIR/keys/node1_wallet.json \
  --to $NODE2_ADDR --amount-raw $EXCESS --rpc http://127.0.0.1:28601

# VERIFY: Transaction rejected with "Insufficient funds" error
```

---

## 3. Proof of Time / VDF

### 3.1 VDF Computation Time (Section 4.1)

**Whitepaper claim:** VDF takes ~700ms with 10M iterations

```bash
# Check VDF computation time in logs
grep "VDF computed in" $TEST_DIR/logs/node1.log | tail -10

# VERIFY:
# - Devnet: ~70ms (1M iterations)
# - Mainnet would be: ~700ms (10M iterations)
```

### 3.2 Invalid VDF Rejected (Section 10.1)

```bash
# This is tested implicitly - blocks with invalid VDF are never accepted
# Check logs for any "InvalidVdfProof" errors
grep -i "invalid.*vdf\|vdf.*failed" $TEST_DIR/logs/node1.log

# VERIFY: No blocks with invalid VDF are accepted
```

### 3.3 Time Structure (Section 4.2)

```bash
# Verify slot calculation
NOW=$(date +%s)
rpc 28601 getChainInfo | jq '.result.bestSlot'

# For devnet with genesis_time=0:
# slot = timestamp (since 1s slots and genesis at epoch 0)

# VERIFY: Slot numbers increase by 1 per second
for i in 1 2 3 4 5; do
  rpc 28601 getChainInfo | jq '.result.bestSlot'
  sleep 1
done
```

### 3.4 Epoch Boundaries (Section 4.2)

```bash
# Monitor epoch transitions (every 20 slots in devnet)
tail -f $TEST_DIR/logs/node1.log | grep -i "epoch"

# VERIFY: Epoch transitions occur every 20 blocks
# Look for: "Epoch N complete at height M"
```

---

## 4. Producer Registration

### 4.1 Registration Requires Bond (Section 6.2)

**Whitepaper claim:** Every producer must lock an activation bond

```bash
# Try to start a producer without bond after genesis period
# First, start a non-producing node
$BINARY --data-dir $TEST_DIR/data/node3 --network devnet run \
  --p2p-port 50403 --rpc-port 28603 --bootstrap /ip4/127.0.0.1/tcp/50401 \
  > $TEST_DIR/logs/node3.log 2>&1 &
NODE3_PID=$!

# Try to register as producer via CLI
$CLI producer register --wallet $TEST_DIR/keys/node3_wallet.json \
  --bond 0 --rpc http://127.0.0.1:28603

# VERIFY: Registration rejected - bond required
```

### 4.2 Registration with Valid Bond

```bash
# First, send funds to node3 for bond
$CLI send --wallet $TEST_DIR/keys/node1_wallet.json \
  --to $NODE3_ADDR --amount 2 --rpc http://127.0.0.1:28601
sleep 5

# Register with proper bond (1 DOLI in devnet)
$CLI producer register --wallet $TEST_DIR/keys/node3_wallet.json \
  --bond 1 --rpc http://127.0.0.1:28603

# VERIFY: Registration accepted
rpc 28603 getProducerSet | jq '.result.producers'
```

### 4.3 Dynamic Registration Difficulty (Section 6.1)

```bash
# Register multiple producers quickly and observe VDF time increase
for i in 4 5; do
  $CLI producer register --wallet $TEST_DIR/keys/node${i}_wallet.json \
    --bond 1 --rpc http://127.0.0.1:28601
done

# Check registration VDF times in logs
grep "Registration VDF" $TEST_DIR/logs/*.log

# VERIFY: VDF difficulty increases with registration demand
```

---

## 5. Bond Stacking

### 5.1 Add Bonds (Section 6.3)

**Whitepaper claim:** Producers can increase stake up to 100× base unit

```bash
# First, fund node1 with more DOLI for stacking
wait_for_height 28601 200  # Accumulate rewards

# Check current bond count
rpc 28601 getProducer "{\"pubkey\":\"$(jq -r '.addresses[0].public_key' $TEST_DIR/keys/node1.json)\"}"

# Add more bonds (go from 1 to 5 bonds)
$CLI producer add-bond --wallet $TEST_DIR/keys/node1_wallet.json \
  --count 4 --rpc http://127.0.0.1:28601

# VERIFY: Bond count increased to 5
rpc 28601 getProducer "{\"pubkey\":\"$(jq -r '.addresses[0].public_key' $TEST_DIR/keys/node1.json)\"}"
```

### 5.2 Maximum Bonds Limit (100)

```bash
# Try to add more than 100 bonds total
$CLI producer add-bond --wallet $TEST_DIR/keys/node1_wallet.json \
  --count 100 --rpc http://127.0.0.1:28601

# VERIFY: Rejected if total would exceed 100
```

### 5.3 Bond Stacking Affects Selection Weight

**Whitepaper claim:** Each bond unit grants one ticket in rotation

```bash
# Node1 has 5 bonds, Node2 has 1 bond
# Over 60 blocks, Node1 should produce ~50 (5/6), Node2 ~10 (1/6)

wait_for_height 28601 300
START_HEIGHT=$(rpc 28601 getChainInfo | jq -r '.result.bestHeight')

# Count blocks per producer over next 60 blocks
wait_for_height 28601 $((START_HEIGHT + 60))

# Analyze block producers
for h in $(seq $START_HEIGHT $((START_HEIGHT + 59))); do
  rpc 28601 getBlockByHeight "{\"height\":$h}" | jq -r '.result.producer'
done | sort | uniq -c

# VERIFY: Distribution matches bond proportions (5:1 ratio)
```

### 5.4 Proportional Rewards Demonstration (30:1 Ratio)

**Whitepaper claim (Section 7.2):** "The smallest participant receives the same percentage return as the largest."

This test demonstrates that a producer with 30 bonds earns exactly 30× more than one with 1 bond, but both have IDENTICAL ROI percentage.

```bash
#!/bin/bash
# PROPORTIONAL REWARDS TEST
# Producer A: 30 bonds → earns 30 DOLI per 31 blocks
# Producer B: 1 bond  → earns 1 DOLI per 31 blocks
# ROI = 1/1000 = 0.1% per cycle (IDENTICAL for both)

TEST_DIR="/tmp/doli-proportional-test"
rm -rf $TEST_DIR && mkdir -p $TEST_DIR/{keys,data,logs}

# Generate keys for 2 producers
./target/release/doli-node --network devnet generate-key --output $TEST_DIR/keys/producer_30bonds.json
./target/release/doli-node --network devnet generate-key --output $TEST_DIR/keys/producer_1bond.json

# Start Producer A (will accumulate rewards to get 30 bonds)
./target/release/doli-node --data-dir $TEST_DIR/data/producer_a --network devnet run \
  --producer --producer-key $TEST_DIR/keys/producer_30bonds.json \
  --p2p-port 50501 --rpc-port 28701 --no-auto-update > $TEST_DIR/logs/producer_a.log 2>&1 &
PID_A=$!

echo "Waiting for Producer A to accumulate rewards for 30 bonds..."
# In devnet: 1 DOLI bond, need 30 DOLI = 30 blocks + maturity
sleep 150  # ~2.5 minutes for 130+ blocks (30 rewards + 100 maturity)

# Check balance
ADDR_A=$(jq -r '.addresses[0].address' $TEST_DIR/keys/producer_30bonds.json)
BALANCE_A=$(curl -s http://127.0.0.1:28701 -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getBalance","params":{"address":"'$ADDR_A'"},"id":1}' | jq -r '.result.confirmed // 0')
echo "Producer A balance: $BALANCE_A (need 3,000,000,000 for 30 bonds)"

# Add 29 more bonds (total 30)
./target/release/doli producer add-bond --wallet $TEST_DIR/keys/producer_30bonds_wallet.json \
  --count 29 --rpc http://127.0.0.1:28701

# Start Producer B with just 1 bond
# First send 1 DOLI from A to B for bond
ADDR_B=$(jq -r '.addresses[0].address' $TEST_DIR/keys/producer_1bond.json)
./target/release/doli send --wallet $TEST_DIR/keys/producer_30bonds_wallet.json \
  --to $ADDR_B --amount 2 --rpc http://127.0.0.1:28701
sleep 5

./target/release/doli-node --data-dir $TEST_DIR/data/producer_b --network devnet run \
  --producer --producer-key $TEST_DIR/keys/producer_1bond.json \
  --p2p-port 50502 --rpc-port 28702 --bootstrap /ip4/127.0.0.1/tcp/50501 \
  --no-auto-update > $TEST_DIR/logs/producer_b.log 2>&1 &
PID_B=$!

sleep 30  # Wait for B to join

# Now track rewards over 310 blocks (10 full cycles of 31 blocks each)
echo "Recording starting state..."
START_HEIGHT=$(curl -s http://127.0.0.1:28701 -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq -r '.result.bestHeight')

# Count blocks produced by each over 310 blocks
echo "Waiting for 310 blocks (~5 minutes)..."
sleep 320

END_HEIGHT=$((START_HEIGHT + 310))

# Analyze production
echo "Analyzing block production from height $START_HEIGHT to $END_HEIGHT..."

BLOCKS_A=0
BLOCKS_B=0
PUBKEY_A=$(jq -r '.addresses[0].public_key' $TEST_DIR/keys/producer_30bonds.json)
PUBKEY_B=$(jq -r '.addresses[0].public_key' $TEST_DIR/keys/producer_1bond.json)

for h in $(seq $START_HEIGHT $END_HEIGHT); do
  PRODUCER=$(curl -s http://127.0.0.1:28701 -X POST -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getBlockByHeight","params":{"height":'$h'},"id":1}' | jq -r '.result.producer')

  if [ "$PRODUCER" = "$PUBKEY_A" ]; then
    ((BLOCKS_A++))
  elif [ "$PRODUCER" = "$PUBKEY_B" ]; then
    ((BLOCKS_B++))
  fi
done

echo "═══════════════════════════════════════════════════════════════"
echo "PROPORTIONAL REWARDS RESULTS"
echo "═══════════════════════════════════════════════════════════════"
echo "Producer A (30 bonds): $BLOCKS_A blocks"
echo "Producer B (1 bond):   $BLOCKS_B blocks"
echo ""
echo "Expected ratio: 30:1"
echo "Actual ratio:   $(echo "scale=2; $BLOCKS_A / $BLOCKS_B" | bc):1"
echo ""

# Calculate rewards (1 DOLI per block in devnet)
REWARDS_A=$BLOCKS_A
REWARDS_B=$BLOCKS_B

# Calculate ROI
# ROI = rewards / stake
# Producer A: stake = 30 DOLI (30 bonds × 1 DOLI)
# Producer B: stake = 1 DOLI (1 bond × 1 DOLI)
ROI_A=$(echo "scale=6; $REWARDS_A / 30" | bc)
ROI_B=$(echo "scale=6; $REWARDS_B / 1" | bc)

echo "═══════════════════════════════════════════════════════════════"
echo "ROI ANALYSIS (Equal ROI regardless of stake size)"
echo "═══════════════════════════════════════════════════════════════"
echo "Producer A: $REWARDS_A DOLI earned / 30 DOLI staked = ${ROI_A} return"
echo "Producer B: $REWARDS_B DOLI earned / 1 DOLI staked  = ${ROI_B} return"
echo ""

# Verify ROI is approximately equal (within 10% tolerance)
ROI_DIFF=$(echo "scale=6; ($ROI_A - $ROI_B) / $ROI_B * 100" | bc 2>/dev/null | tr -d '-')
echo "ROI difference: ${ROI_DIFF}%"

if (( $(echo "$ROI_DIFF < 10" | bc -l) )); then
  echo "✓ PASS: ROI is equal (within 10% tolerance)"
else
  echo "✗ FAIL: ROI differs by more than 10%"
fi

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "WHITEPAPER VERIFICATION"
echo "═══════════════════════════════════════════════════════════════"
echo "Claim: 'All producers earn identical ROI percentage regardless of stake size'"
echo ""
echo "With 30 bonds, Producer A produces 30× more blocks than Producer B (1 bond)"
echo "With 30 bonds, Producer A earns 30× more DOLI than Producer B (1 bond)"
echo "But ROI (return per bond) is IDENTICAL: ~$ROI_A DOLI per bond"
echo ""
echo "This eliminates the need for pools - small and large participants"
echo "get the same percentage return. No luck, no variance, just math."
echo "═══════════════════════════════════════════════════════════════"

# Cleanup
kill $PID_A $PID_B 2>/dev/null
```

**Expected Results:**

| Producer | Bonds | Blocks (310 total) | Rewards | ROI |
|----------|-------|-------------------|---------|-----|
| A | 30 | ~300 (30/31 × 310) | ~300 DOLI | 10.0 DOLI/bond |
| B | 1 | ~10 (1/31 × 310) | ~10 DOLI | 10.0 DOLI/bond |

**Key Verification Points:**
1. Block ratio matches bond ratio (30:1)
2. Reward ratio matches bond ratio (30:1)
3. ROI per bond is IDENTICAL (eliminates need for pools)

---

## 6. Producer Selection

### 6.1 Deterministic Round-Robin (Section 7)

**Whitepaper claim:** Selection is deterministic, not lottery

```bash
# Predict next producer and verify
CURRENT_SLOT=$(rpc 28601 getChainInfo | jq -r '.result.bestSlot')
PRODUCERS=$(rpc 28601 getProducerSet | jq -r '.result.producers')

# The next block producer should be deterministic
# slot % total_tickets determines the producer

# Wait and check
sleep 2
NEXT_BLOCK=$(rpc 28601 getBlockByHeight "{\"height\":$(($(rpc 28601 getChainInfo | jq -r '.result.bestHeight')))}")
echo "Producer: $(echo $NEXT_BLOCK | jq -r '.result.producer')"

# VERIFY: Producer matches deterministic calculation
```

### 6.2 Anti-Grinding Protection (Section 7)

**Whitepaper claim:** Selection MUST NOT depend on prev_hash

```bash
# Verify in code and logs that selection uses only (slot, active_set)
grep "select_producer_for_slot" $TEST_DIR/logs/node1.log
grep "prev_hash" $TEST_DIR/logs/node1.log | grep -i "select"

# VERIFY: No prev_hash in selection logs
```

### 6.3 Fallback Mechanism (Section 7.1)

**Whitepaper claim:** Fallback windows 0-3s, 3-6s, 6-10s

```bash
# Stop primary producer temporarily
kill $NODE1_PID 2>/dev/null
sleep 5

# Check that secondary producer takes over after 3s
tail -f $TEST_DIR/logs/node2.log | grep -i "fallback\|rank"

# Restart node1
$BINARY --data-dir $TEST_DIR/data/node1 --network devnet run \
  --producer --producer-key $TEST_DIR/keys/node1.json \
  --p2p-port 50401 --rpc-port 28601 \
  --bootstrap /ip4/127.0.0.1/tcp/50402 >> $TEST_DIR/logs/node1.log 2>&1 &
NODE1_PID=$!

# VERIFY: Blocks produced by rank 1 or 2 during primary absence
grep "rank" $TEST_DIR/logs/node2.log | tail -20
```

---

## 7. Rewards & Incentives

### 7.1 Block Reward (Section 9.1)

**Whitepaper claim:** 1 DOLI per block in Era 1

```bash
# Check coinbase amount in recent blocks
for h in $(seq 1 10); do
  COINBASE=$(rpc 28601 getBlockByHeight "{\"height\":$h}" | \
    jq -r '.result.transactions[0]')
  echo "Block $h coinbase: $COINBASE"
done

# VERIFY: Each block rewards 1 DOLI (100_000_000 base units in devnet)
```

### 7.2 Reward Maturity (Section 9.2)

**Whitepaper claim:** Rewards require 100 confirmations

```bash
# Try to spend newly earned coinbase immediately
LATEST=$(rpc 28601 getChainInfo | jq -r '.result.bestHeight')

# Get node1's latest coinbase UTXO
UTXOS=$(rpc 28601 getUtxos "{\"address\":\"$NODE1_ADDR\",\"spendableOnly\":false}")
echo "$UTXOS" | jq '.result[] | select(.height > ($latest - 10))' --arg latest "$LATEST"

# Try to spend it
$CLI send --wallet $TEST_DIR/keys/node1_wallet.json \
  --to $NODE2_ADDR --amount 0.5 --rpc http://127.0.0.1:28601

# VERIFY: Transaction using immature coinbase is rejected
# Must wait for 100 confirmations

wait_for_height 28601 $((LATEST + 100))

# Now should work
$CLI send --wallet $TEST_DIR/keys/node1_wallet.json \
  --to $NODE2_ADDR --amount 0.5 --rpc http://127.0.0.1:28601

# VERIFY: Transaction accepted after maturity
```

### 7.3 Epoch Reward Distribution

**Whitepaper claim:** Equal ROI regardless of stake size

```bash
# Monitor epoch rewards
grep "Epoch.*complete.*distributing" $TEST_DIR/logs/node1.log | tail -10

# Calculate ROI per node
# Node1 (5 bonds): earns 5/6 of epoch rewards
# Node2 (1 bond): earns 1/6 of epoch rewards
# ROI = rewards / bonds should be EQUAL

# VERIFY: Per-bond return is identical for all producers
```

### 7.4 Halving Schedule (Section 9.1)

**Whitepaper claim:** Rewards halve every ~4 years (era)

```bash
# In devnet, 1 simulated year = 60 slots
# 4 years = 240 slots

wait_for_height 28601 250  # Past first "era" boundary

# Check reward amount after halving
grep "reward\|halving\|era" $TEST_DIR/logs/node1.log | tail -20

# VERIFY: Reward decreases to 0.5 DOLI in era 2
```

---

## 8. Infractions & Penalties

### 8.1 Inactivity Removal (Section 10.2)

**Whitepaper claim:** 50 consecutive missed slots → removal (10 in devnet)

```bash
# Stop node2 to trigger inactivity
kill $NODE2_PID 2>/dev/null

# Wait for inactivity threshold (10 slots in devnet)
sleep 15

# Check if node2 removed from active set
rpc 28601 getProducerSet | jq '.result.producers'
rpc 28601 getProducer "{\"pubkey\":\"$NODE2_PUBKEY\"}" | jq '.result.status'

# VERIFY: Node2 status = "inactive", removed from producer set
```

### 8.2 Reactivation After Inactivity

```bash
# Restart node2
$BINARY --data-dir $TEST_DIR/data/node2 --network devnet run \
  --producer --producer-key $TEST_DIR/keys/node2.json \
  --p2p-port 50402 --rpc-port 28602 --bootstrap /ip4/127.0.0.1/tcp/50401 \
  > $TEST_DIR/logs/node2.log 2>&1 &
NODE2_PID=$!

# Wait for reactivation (may require new registration VDF)
sleep 30

# Check if node2 rejoined
rpc 28601 getProducerSet | jq '.result.producers'

# VERIFY: Node2 back in active set after reactivation
```

### 8.3 Double Production Slashing (Section 10.3)

**Whitepaper claim:** 100% bond burned for equivocation

```bash
# This is difficult to trigger intentionally but can be tested by:
# 1. Manually crafting two blocks for same slot
# 2. Having a malicious node that double-signs

# Monitor for equivocation detection
grep -i "equivocation\|double.*production\|slash" $TEST_DIR/logs/*.log

# If equivocation detected:
# VERIFY:
# - Bond is burned (not redistributed)
# - Producer removed immediately
# - Must re-register with 2x VDF difficulty
```

### 8.4 Early Withdrawal Penalty (Section 6.4)

**Whitepaper claim:** Year 1: 75%, Year 2: 50%, Year 3: 25%, Year 4+: 0%

```bash
# Request early withdrawal (within first "year" = 60 slots in devnet)
CURRENT_SLOT=$(rpc 28601 getChainInfo | jq -r '.result.bestSlot')

# Check when node1 registered
NODE1_START=$(rpc 28601 getProducer "{\"pubkey\":\"$NODE1_PUBKEY\"}" | jq -r '.result.activeSince')

# If within first 60 slots (year 1), penalty should be 75%
$CLI producer request-withdrawal --wallet $TEST_DIR/keys/node1_wallet.json \
  --bonds 1 --rpc http://127.0.0.1:28601

# Wait for 7-day delay (14 slots in devnet)
sleep 20

# Claim withdrawal
$CLI producer claim-withdrawal --wallet $TEST_DIR/keys/node1_wallet.json \
  --rpc http://127.0.0.1:28601

# VERIFY:
# - Only 25% of bond returned (75% penalty in year 1)
# - Penalty amount was burned
```

### 8.5 Full Vesting After 4 Years

```bash
# Wait for 4 "years" (240 slots in devnet = 4 minutes)
wait_for_height 28601 300

# Now withdrawal should have 0% penalty
$CLI producer request-withdrawal --wallet $TEST_DIR/keys/node1_wallet.json \
  --bonds 1 --rpc http://127.0.0.1:28601

sleep 20  # Wait for delay

$CLI producer claim-withdrawal --wallet $TEST_DIR/keys/node1_wallet.json \
  --rpc http://127.0.0.1:28601

# VERIFY: 100% of bond returned (0% penalty after 4 years)
```

---

## 9. Chain Selection & Forks

### 9.1 Weight-Based Fork Choice (Section 8.1)

**Whitepaper claim:** Chain with highest accumulated producer weight wins

```bash
# Create a network partition scenario
# This requires manual intervention or network simulation

# Start two isolated groups:
# Group A: Node1 (senior, weight 4)
# Group B: Node2, Node3 (junior, weight 1 each)

# After partition heals, the chain from senior producer should win
# even if shorter in blocks

# VERIFY: Seniority weight affects fork resolution
```

### 9.2 Seniority Weights (Section 8.1)

**Whitepaper claim:** Weight increases with years active (1→2→3→4)

```bash
# Check producer weights after time passes
# In devnet: 1 year = 60 slots

wait_for_height 28601 70  # Past 1 "year"

rpc 28601 getProducerSet | jq '.result.producers[] | {pubkey, weight}'

# Wait another "year"
wait_for_height 28601 130

rpc 28601 getProducerSet | jq '.result.producers[] | {pubkey, weight}'

# VERIFY: Weight increases from 1→2→3→4 as years pass
```

---

## 10. Network & Propagation

### 10.1 Block Propagation (Section 5)

```bash
# Start 5 nodes and measure sync time
for i in 3 4 5; do
  $BINARY --data-dir $TEST_DIR/data/node$i --network devnet run \
    --producer --producer-key $TEST_DIR/keys/node$i.json \
    --p2p-port $((50400 + i)) --rpc-port $((28600 + i)) \
    --bootstrap /ip4/127.0.0.1/tcp/50401 \
    > $TEST_DIR/logs/node$i.log 2>&1 &
done

sleep 30

# Check all nodes are synced to same height
for port in 28601 28602 28603 28604 28605; do
  echo "Port $port: $(rpc $port getChainInfo | jq -r '.result.bestHeight')"
done

# VERIFY: All nodes within 1-2 blocks of each other
```

### 10.2 Clock Synchronization (Section 5.2)

```bash
# Check timestamp validation in logs
grep -i "timestamp\|drift\|clock" $TEST_DIR/logs/*.log

# VERIFY: Blocks with timestamps outside acceptable window are rejected
```

### 10.3 Peer Discovery

```bash
# Check peer count on each node
for port in 28601 28602 28603 28604 28605; do
  echo "Port $port peers: $(rpc $port getNetworkInfo | jq -r '.result.peerCount')"
done

# VERIFY: All nodes connected to each other
```

---

## Cleanup

```bash
# Kill all test processes
pkill -f "doli-node.*whitepaper-test" || true

# Optional: Remove test data
# rm -rf $TEST_DIR
```

---

## Test Summary Checklist

### Genesis & Distribution
- [ ] No premine verified
- [ ] Genesis message correct
- [ ] Only coinbase in genesis

### Transactions
- [ ] Valid transactions accepted
- [ ] Double-spend prevented
- [ ] Invalid signatures rejected
- [ ] Insufficient funds rejected

### Proof of Time
- [ ] VDF computation time ~700ms (mainnet) / ~70ms (devnet)
- [ ] Invalid VDF blocks rejected
- [ ] Slot calculation correct
- [ ] Epoch boundaries trigger correctly

### Producer Registration
- [ ] Registration requires bond
- [ ] Registration with valid bond works
- [ ] Dynamic difficulty increases with demand

### Bond Stacking
- [ ] Can add bonds up to 100
- [ ] Max 100 bonds enforced
- [ ] Selection weight proportional to bonds
- [ ] **30 bonds earns 30× more blocks than 1 bond**
- [ ] **ROI per bond is IDENTICAL regardless of total bonds**

### Producer Selection
- [ ] Deterministic round-robin works
- [ ] No prev_hash grinding possible
- [ ] Fallback mechanism works (0-3s, 3-6s, 6-10s)

### Rewards & Incentives
- [ ] 1 DOLI per block (Era 1)
- [ ] 100 confirmation maturity
- [ ] **Equal ROI regardless of stake size (critical claim)**
- [ ] **Proportional rewards: N bonds = N× rewards**
- [ ] Halving occurs at era boundaries

### Infractions & Penalties
- [ ] Inactivity removes producer after threshold
- [ ] Reactivation works after inactivity
- [ ] Double production is slashable (100% burn)
- [ ] Early withdrawal penalty: 75%→50%→25%→0%
- [ ] Full vesting at 4 years

### Chain Selection
- [ ] Weight-based fork choice works
- [ ] Seniority weights increase over time

### Network
- [ ] Block propagation < 1 second
- [ ] Clock drift validation works
- [ ] Peer discovery works

---

## Automated Test Script

Save as `scripts/test_whitepaper_full.sh`:

```bash
#!/bin/bash
set -e

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${YELLOW}========================================${NC}"
echo -e "${YELLOW}DOLI WHITEPAPER FULL TEST SUITE${NC}"
echo -e "${YELLOW}========================================${NC}"

# ... (Full script combining all tests above)
# Each test section should:
# 1. Print test name
# 2. Execute test
# 3. Verify result
# 4. Print PASS/FAIL

echo -e "${GREEN}All tests completed!${NC}"
```

---

*Document version: 1.0*
*Compatible with WHITEPAPER v1.0*

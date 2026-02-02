#!/bin/bash
# DOLI Devnet - ClaimEpochReward E2E Test
#
# =============================================================================
# DEPRECATED - This script tests an obsolete feature
# =============================================================================
#
# Per WHITEPAPER.md Section 9.1, rewards work like Bitcoin:
# - Producer produces a block → gets coinbase reward immediately
# - No claiming needed - rewards are automatic
# - The epoch-based claiming system was deprecated and returns 0
#
# The weighted presence reward system was removed in favor of the simpler
# Bitcoin-like model where 100% of block rewards go directly to producers
# via coinbase transactions.
#
# See: crates/core/src/rewards.rs lines 269-276 for deprecation notice
# See: bins/node/src/node.rs lines 2403-2408 for coinbase implementation
#
# This script is kept for historical reference only.
# =============================================================================
#
# Original test scenario (no longer applicable):
# - Start 3 producer nodes
# - Wait for 2 epochs to complete
# - Use CLI to list claimable rewards
# - Claim rewards for epoch 0
# - Verify claim succeeded (balance, history)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
TEST_DIR="/tmp/doli-claim-epoch-test"
NUM_NODES=3

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
NC='\033[0m'

echo -e "${BLUE}============================================${NC}"
echo -e "${BLUE}  DOLI ClaimEpochReward E2E Test          ${NC}"
echo -e "${BLUE}============================================${NC}"
echo
echo -e "${CYAN}Test Parameters:${NC}"
echo -e "  Nodes:           3 producers"
echo -e "  Network:         devnet (1s slots, 60 blocks/epoch)"
echo -e "  Wait:            Wait for 2 epochs (~120s)"
echo -e "  Test:            Claim epoch 0 rewards via CLI"
echo

# Clean up
echo -e "${YELLOW}Cleaning up previous test...${NC}"
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR/data" "$TEST_DIR/keys" "$TEST_DIR/logs"

# Build
echo -e "${YELLOW}Building binaries (release)...${NC}"
cd "$PROJECT_ROOT"
cargo build --release -p doli-node -p doli-cli 2>&1 | grep -iE "compiling|finished|error" | head -5

NODE_BIN="$PROJECT_ROOT/target/release/doli-node"
CLI_BIN="$PROJECT_ROOT/target/release/doli"
if [ ! -f "$NODE_BIN" ] || [ ! -f "$CLI_BIN" ]; then
    echo -e "${RED}Error: binaries not found${NC}"
    exit 1
fi
echo -e "${GREEN}Build complete.${NC}"

# Generate keys
echo -e "${YELLOW}Generating producer keys...${NC}"
NODE_PUBKEYS=()
for i in 1 2 3; do
    $CLI_BIN --wallet "$TEST_DIR/keys/node${i}.json" new -n "node${i}" >/dev/null 2>&1 || true
    if [ -f "$TEST_DIR/keys/node${i}.json" ]; then
        pubkey=$(cat "$TEST_DIR/keys/node${i}.json" | grep -o '"public_key": *"[^"]*' | sed 's/"public_key": *"//' | head -1)
        address=$(cat "$TEST_DIR/keys/node${i}.json" | grep -o '"address": *"[^"]*' | sed 's/"address": *"//' | head -1)
        NODE_PUBKEYS[$i]="$pubkey"
        echo -e "  Node $i: ${pubkey:0:16}... (${address:0:20}...)"
    else
        echo -e "  ${RED}Node $i: failed to generate key${NC}"
        exit 1
    fi
done

# Generate chainspec with genesis producers for fast block production
echo -e "${YELLOW}Generating chainspec with genesis producers...${NC}"
GENESIS_PRODUCERS_JSON=""
for i in 1 2 3; do
    pubkey="${NODE_PUBKEYS[$i]}"
    if [ $i -gt 1 ]; then
        GENESIS_PRODUCERS_JSON="$GENESIS_PRODUCERS_JSON,"
    fi
    GENESIS_PRODUCERS_JSON="$GENESIS_PRODUCERS_JSON
    {
      \"name\": \"producer_$i\",
      \"public_key\": \"$pubkey\",
      \"bond_count\": 1
    }"
done

cat > "$TEST_DIR/chainspec.json" << EOF
{
  "name": "DOLI Devnet",
  "id": "devnet",
  "network": "Devnet",
  "genesis": {
    "timestamp": 0,
    "message": "DOLI ClaimEpochReward Test",
    "initial_reward": 100000000
  },
  "consensus": {
    "slot_duration": 1,
    "slots_per_epoch": 60,
    "bond_amount": 100000000
  },
  "genesis_producers": [$GENESIS_PRODUCERS_JSON
  ]
}
EOF
echo -e "  ${GREEN}Chainspec created${NC}"

# Ports
BASE_P2P=50500
BASE_RPC=28700

# PIDs for cleanup (indexed array - bash 3.x compatible)
NODE_PIDS=()

cleanup() {
    echo
    echo -e "${YELLOW}Stopping all nodes...${NC}"
    for pid in "${NODE_PIDS[@]}"; do
        if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null
        fi
    done
    sleep 1
    echo -e "${GREEN}Nodes stopped.${NC}"
}

trap cleanup EXIT

# Create data directories
for i in 1 2 3; do
    mkdir -p "$TEST_DIR/data/node${i}"
done

# Start node function
start_node() {
    local node_num=$1
    local is_seed=$2
    local p2p_port=$((BASE_P2P + node_num))
    local rpc_port=$((BASE_RPC + node_num))
    local metrics_port=$((9100 + node_num))

    local bootstrap_arg=""
    if [ "$is_seed" != "true" ]; then
        bootstrap_arg="--bootstrap /ip4/127.0.0.1/tcp/$((BASE_P2P + 1))"
    fi

    $NODE_BIN \
        --data-dir "$TEST_DIR/data/node${node_num}" \
        --network devnet \
        run \
        --producer \
        --producer-key "$TEST_DIR/keys/node${node_num}.json" \
        --p2p-port "$p2p_port" \
        --rpc-port "$rpc_port" \
        --metrics-port "$metrics_port" \
        --chainspec "$TEST_DIR/chainspec.json" \
        $bootstrap_arg \
        --no-auto-update \
        > "$TEST_DIR/logs/node${node_num}.log" 2>&1 &

    echo $!
}

# RPC helper functions
check_node_rpc() {
    local node_num=$1
    local rpc_port=$((BASE_RPC + node_num))
    curl -s -X POST "http://127.0.0.1:${rpc_port}" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' 2>/dev/null | grep -q "result"
}

get_height() {
    local node_num=$1
    local rpc_port=$((BASE_RPC + node_num))
    curl -s -X POST "http://127.0.0.1:${rpc_port}" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' 2>/dev/null | \
        grep -o '"height":[0-9]*' | cut -d':' -f2
}

get_epoch_info() {
    local rpc_port=$((BASE_RPC + 1))
    curl -s -X POST "http://127.0.0.1:${rpc_port}" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"getEpochInfo","params":[],"id":1}' 2>/dev/null
}

echo
echo -e "${GREEN}============================================${NC}"
echo -e "${GREEN}  Starting Nodes                           ${NC}"
echo -e "${GREEN}============================================${NC}"

# Start all nodes
for i in 1 2 3; do
    is_seed="false"
    [ "$i" = "1" ] && is_seed="true"

    echo -e "${CYAN}Starting Node $i...${NC}"
    NODE_PIDS[$i]=$(start_node $i $is_seed)
    echo -e "  PID: ${NODE_PIDS[$i]}, P2P: $((BASE_P2P + i)), RPC: $((BASE_RPC + i))"

    echo -n "  Waiting..."
    for j in $(seq 1 30); do
        if check_node_rpc $i; then
            echo -e " ${GREEN}ready${NC}"
            break
        fi
        sleep 1
        echo -n "."
    done

    # Small delay between nodes
    [ "$i" != "3" ] && sleep 5
done

echo
echo -e "${GREEN}============================================${NC}"
echo -e "${GREEN}  Waiting for 2 Epochs (~120 blocks)       ${NC}"
echo -e "${GREEN}============================================${NC}"
echo
echo -e "${CYAN}Devnet: 1s slots, 60 blocks per epoch${NC}"
echo -e "${CYAN}Need height >= 120 for epoch 0 and 1 to be complete${NC}"
echo

# Wait for 2 epochs to complete (need height >= 120)
TARGET_HEIGHT=125
while true; do
    height=$(get_height 1)
    if [ -n "$height" ] && [ "$height" -ge "$TARGET_HEIGHT" ]; then
        echo
        echo -e "${GREEN}Target height reached: $height >= $TARGET_HEIGHT${NC}"
        break
    fi
    printf "\r  Current height: %s / %s  " "${height:-?}" "$TARGET_HEIGHT"
    sleep 5
done

echo
echo -e "${GREEN}============================================${NC}"
echo -e "${GREEN}  Testing Epoch Info RPC                   ${NC}"
echo -e "${GREEN}============================================${NC}"
echo
echo -e "${CYAN}Calling getEpochInfo...${NC}"
epoch_info=$(get_epoch_info)
echo "$epoch_info" | python3 -m json.tool 2>/dev/null || echo "$epoch_info"

echo
echo -e "${GREEN}============================================${NC}"
echo -e "${GREEN}  Testing Rewards CLI Commands             ${NC}"
echo -e "${GREEN}============================================${NC}"

# Test rewards info
echo
echo -e "${CYAN}Testing: doli rewards info${NC}"
$CLI_BIN --wallet "$TEST_DIR/keys/node1.json" --rpc "http://127.0.0.1:$((BASE_RPC + 1))" rewards info 2>&1 || echo "(command may not be fully implemented)"

# Test rewards list
echo
echo -e "${CYAN}Testing: doli rewards list${NC}"
$CLI_BIN --wallet "$TEST_DIR/keys/node1.json" --rpc "http://127.0.0.1:$((BASE_RPC + 1))" rewards list 2>&1 || echo "(command may not be fully implemented)"

# Test rewards history (should be empty initially)
echo
echo -e "${CYAN}Testing: doli rewards history${NC}"
$CLI_BIN --wallet "$TEST_DIR/keys/node1.json" --rpc "http://127.0.0.1:$((BASE_RPC + 1))" rewards history 2>&1 || echo "(command may not be fully implemented)"

# Test claiming epoch 0
echo
echo -e "${GREEN}============================================${NC}"
echo -e "${GREEN}  Testing Claim Epoch 0                    ${NC}"
echo -e "${GREEN}============================================${NC}"
echo
echo -e "${CYAN}Testing: doli rewards claim 0${NC}"
$CLI_BIN --wallet "$TEST_DIR/keys/node1.json" --rpc "http://127.0.0.1:$((BASE_RPC + 1))" rewards claim 0 2>&1 || echo "(claim may require additional implementation)"

# Wait for tx to be included
sleep 5

# Check history after claim
echo
echo -e "${CYAN}Testing: doli rewards history (after claim)${NC}"
$CLI_BIN --wallet "$TEST_DIR/keys/node1.json" --rpc "http://127.0.0.1:$((BASE_RPC + 1))" rewards history 2>&1 || echo "(command may not be fully implemented)"

# Check wallet balance
echo
echo -e "${CYAN}Checking wallet balance...${NC}"
$CLI_BIN --wallet "$TEST_DIR/keys/node1.json" --rpc "http://127.0.0.1:$((BASE_RPC + 1))" wallet balance 2>&1 || echo "(balance check)"

echo
echo -e "${GREEN}============================================${NC}"
echo -e "${GREEN}  Test Summary                             ${NC}"
echo -e "${GREEN}============================================${NC}"
echo
echo -e "${MAGENTA}=== PRESENCE COMMITMENTS IN BLOCKS ===${NC}"
grep -h "presence" "$TEST_DIR/logs/node1.log" 2>/dev/null | head -10 || echo "(none found)"

echo
echo -e "${MAGENTA}=== CLAIM TRANSACTIONS ===${NC}"
grep -h "ClaimEpochReward\|claim" "$TEST_DIR/logs/node1.log" 2>/dev/null | head -10 || echo "(none found)"

echo
echo -e "${MAGENTA}=== BLOCK PRODUCTION SUMMARY ===${NC}"
for node_num in 1 2 3; do
    blocks=$(grep -c "Produced block" "$TEST_DIR/logs/node${node_num}.log" 2>/dev/null || echo "0")
    echo -e "  Node $node_num: produced $blocks blocks"
done

echo
echo -e "${GREEN}Test complete!${NC}"
echo -e "${YELLOW}Logs saved to: $TEST_DIR/logs/${NC}"
echo -e "${YELLOW}To view full logs: tail -100 $TEST_DIR/logs/node1.log${NC}"

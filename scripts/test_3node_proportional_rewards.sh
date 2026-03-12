#!/bin/bash
# DOLI Devnet - 3-Node Proportional Reward Test
#
# Test scenario:
# - Node 1 starts immediately (seed)
# - Node 2 joins after 45 seconds (mid-epoch)
# - Node 3 joins after 90 seconds (near end of epoch 1)
#
# Expected results:
# - Epoch 0: Disproportional rewards (Node1 has more blocks than Node2, Node3 has none)
# - Epoch 1: Equal rewards (all 3 nodes round-robin producing equal blocks)
#
# Devnet parameters:
# - 5 second slots
# - 20 slots per reward epoch = 100 seconds per epoch

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
TEST_DIR="/tmp/doli-3node-test"
NUM_NODES=3
NODE2_DELAY=45  # Node 2 joins after 45s
NODE3_DELAY=90  # Node 3 joins after 90s (45s after Node 2)

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
NC='\033[0m'

echo -e "${BLUE}============================================${NC}"
echo -e "${BLUE}  DOLI 3-Node Proportional Reward Test     ${NC}"
echo -e "${BLUE}============================================${NC}"
echo
echo -e "${CYAN}Test Parameters:${NC}"
echo -e "  Nodes:           3"
echo -e "  Node 1:          Starts immediately (seed)"
echo -e "  Node 2:          Joins at +${NODE2_DELAY}s"
echo -e "  Node 3:          Joins at +${NODE3_DELAY}s"
echo -e "  Network:         devnet (5s slots, 20 slots/epoch = 100s)"
echo
echo -e "${CYAN}Expected Results:${NC}"
echo -e "  Epoch 0: Disproportional (Node1 > Node2 > Node3 in blocks)"
echo -e "  Epoch 1: Equal rewards (all nodes round-robin)"
echo

# Clean up
echo -e "${YELLOW}Cleaning up previous test...${NC}"
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR/data" "$TEST_DIR/keys" "$TEST_DIR/logs"

# Build
echo -e "${YELLOW}Building doli-node (release)...${NC}"
cd "$PROJECT_ROOT"
cargo build --release -p doli-node 2>&1 | grep -iE "compiling|finished|error" | head -5

NODE_BIN="$PROJECT_ROOT/target/release/doli-node"
if [ ! -f "$NODE_BIN" ]; then
    echo -e "${RED}Error: doli-node binary not found${NC}"
    exit 1
fi
echo -e "${GREEN}Build complete.${NC}"

# Generate keys using doli-cli
echo -e "${YELLOW}Generating producer keys...${NC}"
CLI_BIN="$PROJECT_ROOT/target/release/doli"
cargo build --release -p doli-cli 2>&1 | grep -iE "compiling|finished|error" | head -3

for i in 1 2 3; do
    $CLI_BIN --wallet "$TEST_DIR/keys/node${i}.json" new -n "node${i}" >/dev/null 2>&1 || true
    if [ -f "$TEST_DIR/keys/node${i}.json" ]; then
        pubkey=$(cat "$TEST_DIR/keys/node${i}.json" | grep -o '"public_key":"[^"]*' | cut -d'"' -f4 | head -1)
        echo -e "  Node $i: ${pubkey:0:16}..."
    else
        echo -e "  ${RED}Node $i: failed to generate key${NC}"
        exit 1
    fi
done

# Ports
BASE_P2P=50400
BASE_RPC=28640

# PIDs for cleanup
declare -a NODE_PIDS

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
    local metrics_port=$((9000 + node_num))

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
        $bootstrap_arg \
        --no-auto-update \
        > "$TEST_DIR/logs/node${node_num}.log" 2>&1 &

    echo $!
}

# Check RPC
check_node_rpc() {
    local node_num=$1
    local rpc_port=$((BASE_RPC + node_num))
    curl -s -X POST "http://127.0.0.1:${rpc_port}" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' 2>/dev/null | grep -q "result"
}

# Get chain height
get_height() {
    local node_num=$1
    local rpc_port=$((BASE_RPC + node_num))
    curl -s -X POST "http://127.0.0.1:${rpc_port}" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' 2>/dev/null | \
        grep -o '"height":[0-9]*' | cut -d':' -f2
}

echo
echo -e "${GREEN}============================================${NC}"
echo -e "${GREEN}  Starting Nodes                           ${NC}"
echo -e "${GREEN}============================================${NC}"

# Record start time
TEST_START=$(date +%s)

# Start Node 1 (seed)
echo
echo -e "${CYAN}[+0s] Starting Node 1 (seed)...${NC}"
NODE_PIDS[1]=$(start_node 1 true)
echo -e "  PID: ${NODE_PIDS[1]}, P2P: $((BASE_P2P + 1)), RPC: $((BASE_RPC + 1))"

echo -n "  Waiting for Node 1..."
for i in $(seq 1 30); do
    if check_node_rpc 1; then
        echo -e " ${GREEN}ready${NC}"
        break
    fi
    sleep 1
    echo -n "."
done

# Wait for Node 2 join time
echo
echo -e "${CYAN}Waiting ${NODE2_DELAY}s for Node 2 to join...${NC}"
for ((i=NODE2_DELAY; i>0; i--)); do
    height=$(get_height 1)
    printf "\r  Countdown: %3ds | Height: %s  " "$i" "${height:-?}"
    sleep 1
done
echo

# Start Node 2
echo -e "${CYAN}[+${NODE2_DELAY}s] Starting Node 2...${NC}"
NODE_PIDS[2]=$(start_node 2 false)
echo -e "  PID: ${NODE_PIDS[2]}, P2P: $((BASE_P2P + 2)), RPC: $((BASE_RPC + 2))"

echo -n "  Waiting for Node 2..."
for i in $(seq 1 20); do
    if check_node_rpc 2; then
        echo -e " ${GREEN}ready${NC}"
        break
    fi
    sleep 1
    echo -n "."
done

# Wait for Node 3 join time
remaining=$((NODE3_DELAY - NODE2_DELAY))
echo
echo -e "${CYAN}Waiting ${remaining}s for Node 3 to join...${NC}"
for ((i=remaining; i>0; i--)); do
    height=$(get_height 1)
    printf "\r  Countdown: %3ds | Height: %s  " "$i" "${height:-?}"
    sleep 1
done
echo

# Start Node 3
echo -e "${CYAN}[+${NODE3_DELAY}s] Starting Node 3...${NC}"
NODE_PIDS[3]=$(start_node 3 false)
echo -e "  PID: ${NODE_PIDS[3]}, P2P: $((BASE_P2P + 3)), RPC: $((BASE_RPC + 3))"

echo -n "  Waiting for Node 3..."
for i in $(seq 1 20); do
    if check_node_rpc 3; then
        echo -e " ${GREEN}ready${NC}"
        break
    fi
    sleep 1
    echo -n "."
done

echo
echo -e "${GREEN}============================================${NC}"
echo -e "${GREEN}  All 3 Nodes Running - Monitoring Epochs  ${NC}"
echo -e "${GREEN}============================================${NC}"
echo
echo -e "${CYAN}Monitoring for 3 epochs (~300 seconds)...${NC}"
echo -e "${CYAN}Epoch 0: ~100s, Epoch 1: ~200s, Epoch 2: ~300s${NC}"
echo

# Monitor for 3 epochs (300s) to see epoch 0, 1, and 2
MONITOR_DURATION=320
start_monitor=$(date +%s)
last_epoch_check=0

while [ $(($(date +%s) - start_monitor)) -lt $MONITOR_DURATION ]; do
    current_time=$(($(date +%s) - start_monitor))

    # Status update every 20 seconds
    if [ $((current_time % 20)) -eq 0 ] && [ "$current_time" != "$last_epoch_check" ]; then
        last_epoch_check=$current_time
        echo
        echo -e "${CYAN}=== Status at +$((current_time + NODE3_DELAY))s (monitor +${current_time}s) ===${NC}"

        for node_num in 1 2 3; do
            height=$(get_height $node_num 2>/dev/null)
            blocks=$(grep -c "Produced block" "$TEST_DIR/logs/node${node_num}.log" 2>/dev/null || echo "0")
            echo -e "  Node $node_num: height=${height:-?}, blocks_produced=$blocks"
        done

        # Check for epoch events
        echo -e "${YELLOW}  Recent epoch events:${NC}"
        grep -h "Epoch.*complete\|Producer.*blocks.*DOLI" "$TEST_DIR/logs/node1.log" 2>/dev/null | tail -3 | sed 's/^/    /' || echo "    (none yet)"
    fi

    sleep 5
done

echo
echo -e "${GREEN}============================================${NC}"
echo -e "${GREEN}  Test Complete - Results                  ${NC}"
echo -e "${GREEN}============================================${NC}"
echo

# Final analysis
echo -e "${MAGENTA}=== EPOCH REWARD DISTRIBUTION ===${NC}"
echo
grep -h "Epoch.*complete\|Producer.*blocks.*DOLI\|distributing" "$TEST_DIR/logs/node1.log" 2>/dev/null | head -30

echo
echo -e "${MAGENTA}=== BLOCK PRODUCTION SUMMARY ===${NC}"
echo
for node_num in 1 2 3; do
    blocks=$(grep -c "Produced block" "$TEST_DIR/logs/node${node_num}.log" 2>/dev/null || echo "0")
    echo -e "  Node $node_num: produced $blocks blocks total"
done

echo
echo -e "${MAGENTA}=== EXPECTED vs ACTUAL ===${NC}"
echo
echo -e "${CYAN}Epoch 0 (first ~20 blocks):${NC}"
echo -e "  Expected: Node1 has most blocks (started first)"
echo -e "            Node2 has some blocks (joined at +45s)"
echo -e "            Node3 has few/no blocks (joined at +90s)"
echo -e "  -> Rewards should be PROPORTIONAL to blocks produced"
echo
echo -e "${CYAN}Epoch 1 (blocks 21-40):${NC}"
echo -e "  Expected: All 3 nodes produce ~equal blocks (round-robin)"
echo -e "  -> Rewards should be approximately EQUAL"
echo

# Extract epoch distribution from logs
echo -e "${MAGENTA}=== EPOCH 0 DISTRIBUTION ===${NC}"
grep -A 10 "Epoch 0 complete" "$TEST_DIR/logs/node1.log" 2>/dev/null | head -10 || echo "(not found)"

echo
echo -e "${MAGENTA}=== EPOCH 1 DISTRIBUTION ===${NC}"
grep -A 10 "Epoch 1 complete" "$TEST_DIR/logs/node1.log" 2>/dev/null | head -10 || echo "(not found)"

echo
echo -e "${GREEN}Logs saved to: $TEST_DIR/logs/${NC}"
echo -e "${YELLOW}To view full logs: tail -100 $TEST_DIR/logs/node1.log${NC}"

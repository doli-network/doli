#!/bin/bash
# DOLI Devnet - 3-Node Staggered Reward Test
#
# Test scenario:
# - Node 1 starts immediately (seed/genesis)
# - Node 2 joins after 60 seconds
# - Node 3 joins after 120 seconds
# - Monitor for 5 minutes total
# - Generate detailed rewards report with exact decimals
#
# Devnet parameters (updated):
# - 1 second slots
# - 20 slots per reward epoch = 20 seconds per epoch
# - 1 DOLI block reward
# - 1M VDF iterations (~70ms)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
TEST_DIR="/tmp/doli-devnet-3node"
NUM_NODES=3
NODE2_DELAY=60   # Node 2 joins after 60s
NODE3_DELAY=120  # Node 3 joins after 120s
TOTAL_DURATION=300  # 5 minutes

# Devnet timing
SLOT_DURATION=1
SLOTS_PER_EPOCH=20
EPOCH_DURATION=$((SLOT_DURATION * SLOTS_PER_EPOCH))  # 20 seconds

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
NC='\033[0m'

echo -e "${BLUE}============================================${NC}"
echo -e "${BLUE}  DOLI Devnet 3-Node Staggered Reward Test ${NC}"
echo -e "${BLUE}============================================${NC}"
echo
echo -e "${CYAN}Test Parameters:${NC}"
echo -e "  Nodes:              3"
echo -e "  Node 1:             Starts immediately (seed/genesis)"
echo -e "  Node 2:             Joins at +${NODE2_DELAY}s"
echo -e "  Node 3:             Joins at +${NODE3_DELAY}s"
echo -e "  Total duration:     ${TOTAL_DURATION}s (5 minutes)"
echo
echo -e "${CYAN}Devnet Parameters:${NC}"
echo -e "  Slot duration:      ${SLOT_DURATION}s"
echo -e "  Slots per epoch:    ${SLOTS_PER_EPOCH}"
echo -e "  Epoch duration:     ${EPOCH_DURATION}s"
echo -e "  Block reward:       1 DOLI"
echo -e "  VDF iterations:     1M (~70ms)"
echo
echo -e "${CYAN}Epoch Timeline:${NC}"
echo -e "  Epoch 0:   0-20s   (Node 1 only)"
echo -e "  Epoch 1:  20-40s   (Node 1 only)"
echo -e "  Epoch 2:  40-60s   (Node 1 only)"
echo -e "  Epoch 3:  60-80s   (Node 1 + Node 2 joins at 60s)"
echo -e "  Epoch 4:  80-100s  (Node 1 + Node 2)"
echo -e "  Epoch 5: 100-120s  (Node 1 + Node 2)"
echo -e "  Epoch 6: 120-140s  (All 3 nodes, Node 3 joins at 120s)"
echo -e "  ...and so on for 5 minutes (15 epochs total)"
echo

# Clean up
echo -e "${YELLOW}Cleaning up previous test...${NC}"
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR/data" "$TEST_DIR/keys" "$TEST_DIR/logs" "$TEST_DIR/reports"

# Build
echo -e "${YELLOW}Building doli-node (release)...${NC}"
cd "$PROJECT_ROOT"
cargo build --release -p doli-node -p doli-cli 2>&1 | grep -iE "compiling|finished|error" | head -5

NODE_BIN="$PROJECT_ROOT/target/release/doli-node"
CLI_BIN="$PROJECT_ROOT/target/release/doli"

if [ ! -f "$NODE_BIN" ]; then
    echo -e "${RED}Error: doli-node binary not found${NC}"
    exit 1
fi
echo -e "${GREEN}Build complete.${NC}"

# Generate keys
echo -e "${YELLOW}Generating producer keys...${NC}"
declare -A NODE_PUBKEYS

for i in 1 2 3; do
    $CLI_BIN --wallet "$TEST_DIR/keys/node${i}.json" new -n "node${i}" >/dev/null 2>&1 || true
    if [ -f "$TEST_DIR/keys/node${i}.json" ]; then
        pubkey=$(cat "$TEST_DIR/keys/node${i}.json" | grep -o '"public_key":"[^"]*' | cut -d'"' -f4 | head -1)
        NODE_PUBKEYS[$i]=$pubkey
        echo -e "  Node $i: ${pubkey:0:16}..."
    else
        echo -e "  ${RED}Node $i: failed to generate key${NC}"
        exit 1
    fi
done

# Save pubkeys for report
echo "${NODE_PUBKEYS[1]}" > "$TEST_DIR/keys/node1.pubkey"
echo "${NODE_PUBKEYS[2]}" > "$TEST_DIR/keys/node2.pubkey"
echo "${NODE_PUBKEYS[3]}" > "$TEST_DIR/keys/node3.pubkey"

# Ports
BASE_P2P=50500
BASE_RPC=28700

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
    sleep 2
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
        grep -o '"bestHeight":[0-9]*' | cut -d':' -f2
}

# Get blocks produced count from logs
get_blocks_produced() {
    local node_num=$1
    local count
    count=$(grep -c "Produced block" "$TEST_DIR/logs/node${node_num}.log" 2>/dev/null) || count=0
    echo "${count:-0}" | tr -d '[:space:]'
}

echo
echo -e "${GREEN}============================================${NC}"
echo -e "${GREEN}  Starting Nodes                           ${NC}"
echo -e "${GREEN}============================================${NC}"

# Record start time
TEST_START=$(date +%s)
echo "TEST_START=$TEST_START" > "$TEST_DIR/test_timing.txt"

# Start Node 1 (seed/genesis)
echo
echo -e "${CYAN}[T+0s] Starting Node 1 (seed/genesis)...${NC}"
NODE_PIDS[1]=$(start_node 1 true)
echo -e "  PID: ${NODE_PIDS[1]}, P2P: $((BASE_P2P + 1)), RPC: $((BASE_RPC + 1))"
echo "NODE1_START=0" >> "$TEST_DIR/test_timing.txt"

echo -n "  Waiting for Node 1 to be ready..."
for i in $(seq 1 30); do
    if check_node_rpc 1; then
        echo -e " ${GREEN}ready${NC}"
        break
    fi
    sleep 1
    echo -n "."
done

# Wait for Node 2 join time (60s)
echo
echo -e "${CYAN}Waiting until T+${NODE2_DELAY}s for Node 2...${NC}"
while [ $(($(date +%s) - TEST_START)) -lt $NODE2_DELAY ]; do
    elapsed=$(($(date +%s) - TEST_START))
    remaining=$((NODE2_DELAY - elapsed))
    height=$(get_height 1)
    epoch=$((height / SLOTS_PER_EPOCH))
    printf "\r  T+%3ds | Height: %s | Epoch: %s | Node 2 in: %ds  " "$elapsed" "${height:-?}" "$epoch" "$remaining"
    sleep 1
done
echo

# Start Node 2
elapsed=$(($(date +%s) - TEST_START))
echo -e "${CYAN}[T+${elapsed}s] Starting Node 2...${NC}"
NODE_PIDS[2]=$(start_node 2 false)
echo -e "  PID: ${NODE_PIDS[2]}, P2P: $((BASE_P2P + 2)), RPC: $((BASE_RPC + 2))"
echo "NODE2_START=$elapsed" >> "$TEST_DIR/test_timing.txt"

echo -n "  Waiting for Node 2 to be ready..."
for i in $(seq 1 20); do
    if check_node_rpc 2; then
        echo -e " ${GREEN}ready${NC}"
        break
    fi
    sleep 1
    echo -n "."
done

# Wait for Node 3 join time (120s)
echo
echo -e "${CYAN}Waiting until T+${NODE3_DELAY}s for Node 3...${NC}"
while [ $(($(date +%s) - TEST_START)) -lt $NODE3_DELAY ]; do
    elapsed=$(($(date +%s) - TEST_START))
    remaining=$((NODE3_DELAY - elapsed))
    height=$(get_height 1)
    epoch=$((height / SLOTS_PER_EPOCH))
    printf "\r  T+%3ds | Height: %s | Epoch: %s | Node 3 in: %ds  " "$elapsed" "${height:-?}" "$epoch" "$remaining"
    sleep 1
done
echo

# Start Node 3
elapsed=$(($(date +%s) - TEST_START))
echo -e "${CYAN}[T+${elapsed}s] Starting Node 3...${NC}"
NODE_PIDS[3]=$(start_node 3 false)
echo -e "  PID: ${NODE_PIDS[3]}, P2P: $((BASE_P2P + 3)), RPC: $((BASE_RPC + 3))"
echo "NODE3_START=$elapsed" >> "$TEST_DIR/test_timing.txt"

echo -n "  Waiting for Node 3 to be ready..."
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
echo -e "${GREEN}  All 3 Nodes Running - Monitoring         ${NC}"
echo -e "${GREEN}============================================${NC}"
echo
echo -e "${CYAN}Monitoring for 5 minutes (${TOTAL_DURATION}s)...${NC}"
echo

# Monitor until total duration
last_report_epoch=-1

while [ $(($(date +%s) - TEST_START)) -lt $TOTAL_DURATION ]; do
    elapsed=$(($(date +%s) - TEST_START))
    height=$(get_height 1 2>/dev/null || echo "0")
    epoch=$((height / SLOTS_PER_EPOCH))
    remaining=$((TOTAL_DURATION - elapsed))

    # Status every 10 seconds
    if [ $((elapsed % 10)) -eq 0 ]; then
        blocks1=$(get_blocks_produced 1)
        blocks2=$(get_blocks_produced 2)
        blocks3=$(get_blocks_produced 3)
        printf "\r  T+%3ds | Height: %3s | Epoch: %2s | Blocks: N1=%s N2=%s N3=%s | %ds left  " \
            "$elapsed" "${height:-?}" "$epoch" "$blocks1" "$blocks2" "$blocks3" "$remaining"
    fi

    # Report at epoch 3 boundary (around 60s)
    if [ "$epoch" -ge 3 ] && [ "$last_report_epoch" -lt 3 ]; then
        last_report_epoch=3
        echo
        echo -e "${MAGENTA}=== EPOCH 3 BOUNDARY REPORT (T+${elapsed}s) ===${NC}"
        grep -h "Epoch.*complete\|distribut\|reward" "$TEST_DIR/logs/node1.log" 2>/dev/null | tail -20
    fi

    sleep 1
done

echo
echo
echo -e "${GREEN}============================================${NC}"
echo -e "${GREEN}  Test Complete - Generating Report        ${NC}"
echo -e "${GREEN}============================================${NC}"
echo

REPORT_FILE="$TEST_DIR/reports/rewards_report.txt"
{
    echo "================================================================"
    echo "  DOLI DEVNET 3-NODE REWARDS DISTRIBUTION REPORT"
    echo "  Generated: $(date)"
    echo "================================================================"
    echo
    echo "TEST PARAMETERS"
    echo "---------------"
    echo "  Slot duration:       ${SLOT_DURATION}s"
    echo "  Slots per epoch:     ${SLOTS_PER_EPOCH}"
    echo "  Epoch duration:      ${EPOCH_DURATION}s"
    echo "  Block reward:        1.00000000 DOLI"
    echo "  Total test duration: ${TOTAL_DURATION}s"
    echo
    echo "NODE CONFIGURATION"
    echo "------------------"
    echo "  Node 1: Started at T+0s   (seed/genesis)"
    echo "          PubKey: ${NODE_PUBKEYS[1]:0:32}..."
    echo "  Node 2: Started at T+60s"
    echo "          PubKey: ${NODE_PUBKEYS[2]:0:32}..."
    echo "  Node 3: Started at T+120s"
    echo "          PubKey: ${NODE_PUBKEYS[3]:0:32}..."
    echo
    echo "BLOCK PRODUCTION SUMMARY"
    echo "------------------------"

    blocks1=$(get_blocks_produced 1)
    blocks2=$(get_blocks_produced 2)
    blocks3=$(get_blocks_produced 3)
    total_blocks=$((blocks1 + blocks2 + blocks3))

    echo "  Node 1: $blocks1 blocks"
    echo "  Node 2: $blocks2 blocks"
    echo "  Node 3: $blocks3 blocks"
    echo "  Total:  $total_blocks blocks"
    echo
    echo "EPOCH-BY-EPOCH REWARD DISTRIBUTION"
    echo "-----------------------------------"
    echo

    # Extract epoch rewards from logs
    grep -h "Epoch.*complete\|Producer.*DOLI\|distribut" "$TEST_DIR/logs/node1.log" 2>/dev/null | while read -r line; do
        echo "  $line"
    done

    echo
    echo "DETAILED EPOCH ANALYSIS"
    echo "-----------------------"

    # Parse epoch data from logs
    for epoch_num in $(seq 0 14); do
        echo
        echo "Epoch $epoch_num (T+$((epoch_num * EPOCH_DURATION))s - T+$(((epoch_num + 1) * EPOCH_DURATION))s):"

        # Determine which nodes were active in this epoch
        epoch_start=$((epoch_num * EPOCH_DURATION))
        if [ $epoch_start -lt 60 ]; then
            echo "  Active nodes: Node 1 only"
        elif [ $epoch_start -lt 120 ]; then
            echo "  Active nodes: Node 1, Node 2"
        else
            echo "  Active nodes: Node 1, Node 2, Node 3"
        fi

        # Try to extract specific epoch rewards from logs
        grep -h "Epoch $epoch_num" "$TEST_DIR/logs/node1.log" 2>/dev/null | head -5 | while read -r line; do
            echo "    $line"
        done
    done

    echo
    echo "RAW LOG EXCERPTS"
    echo "----------------"
    echo
    echo "=== Node 1 Epoch/Reward Events ==="
    grep -iE "epoch|reward|distribut|DOLI" "$TEST_DIR/logs/node1.log" 2>/dev/null | head -50
    echo
    echo "=== Node 2 Epoch/Reward Events ==="
    grep -iE "epoch|reward|distribut|DOLI" "$TEST_DIR/logs/node2.log" 2>/dev/null | head -30
    echo
    echo "=== Node 3 Epoch/Reward Events ==="
    grep -iE "epoch|reward|distribut|DOLI" "$TEST_DIR/logs/node3.log" 2>/dev/null | head -30

} | tee "$REPORT_FILE"

echo
echo -e "${GREEN}============================================${NC}"
echo -e "${GREEN}  Report saved to: $REPORT_FILE            ${NC}"
echo -e "${GREEN}============================================${NC}"
echo
echo -e "${YELLOW}Full logs available at:${NC}"
echo -e "  Node 1: $TEST_DIR/logs/node1.log"
echo -e "  Node 2: $TEST_DIR/logs/node2.log"
echo -e "  Node 3: $TEST_DIR/logs/node3.log"
echo
echo -e "${CYAN}To view epoch rewards in detail:${NC}"
echo -e "  grep -i 'epoch\\|reward' $TEST_DIR/logs/node1.log"
echo

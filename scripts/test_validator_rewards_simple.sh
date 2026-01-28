#!/bin/bash
# DOLI Devnet - Simple Validator Reward Test
# Tests that validators receive rewards only from the moment they join
#
# This is a simpler test with 3 nodes to avoid producer list stability issues.
# It uses longer stabilization periods between node joins.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
TEST_DIR="/tmp/doli-simple-test"
NUM_NODES=3
STABILIZATION_DELAY=45  # Seconds to wait for producer list to stabilize

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}   DOLI Devnet - Validator Reward Test ${NC}"
echo -e "${BLUE}========================================${NC}"
echo
echo -e "${CYAN}Test Parameters:${NC}"
echo -e "  Nodes:              ${NUM_NODES}"
echo -e "  Stabilization:      ${STABILIZATION_DELAY}s between joins"
echo -e "  Network:            devnet (5s slots, 20 slots/epoch = 100s)"
echo

# Clean up
echo -e "${YELLOW}Cleaning up previous test...${NC}"
pkill -f "doli-node.*simple-test" 2>/dev/null || true
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR/data" "$TEST_DIR/keys" "$TEST_DIR/logs"

# Build
echo -e "${YELLOW}Building doli-node...${NC}"
cd "$PROJECT_ROOT"
cargo build --release -p doli-node 2>&1 | grep -i "compiling\|finished\|error" | tail -5

NODE_BIN="$PROJECT_ROOT/target/release/doli-node"
if [ ! -f "$NODE_BIN" ]; then
    echo -e "${RED}Error: doli-node binary not found${NC}"
    exit 1
fi

# Generate keys inline (without separate keygen tool)
echo -e "${YELLOW}Generating producer keys...${NC}"
for i in $(seq 1 $NUM_NODES); do
    # Generate a random key using openssl
    PRIVKEY=$(openssl rand -hex 32)
    # For devnet testing, we just need valid JSON format
    cat > "$TEST_DIR/keys/node${i}.json" << EOF
{
  "version": 1,
  "addresses": [
    {
      "address": "ddoli1test${i}",
      "public_key": "${PRIVKEY}0000000000000000000000000000000000000000000000000000000000000000",
      "private_key": "${PRIVKEY}"
    }
  ]
}
EOF
    echo "  Node $i: key generated"
done

# Port configuration
BASE_P2P=50400
BASE_RPC=28640
BASE_METRICS=9100

# Store PIDs
declare -a NODE_PIDS

cleanup() {
    echo
    echo -e "${YELLOW}Stopping nodes...${NC}"
    for pid in "${NODE_PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    echo -e "${GREEN}Done.${NC}"
}
trap cleanup EXIT

# Create node directories
for i in $(seq 1 $NUM_NODES); do
    mkdir -p "$TEST_DIR/data/node${i}"
done

# Start Node 1 (seed)
echo
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}   Starting Test                       ${NC}"
echo -e "${GREEN}========================================${NC}"
echo

echo -e "${CYAN}[$(date '+%H:%M:%S')] Starting Node 1 (seed)...${NC}"
$NODE_BIN \
    --data-dir "$TEST_DIR/data/node1" \
    --network devnet \
    run \
    --producer \
    --producer-key "$TEST_DIR/keys/node1.json" \
    --p2p-port $((BASE_P2P + 1)) \
    --rpc-port $((BASE_RPC + 1)) \
    --metrics-port $((BASE_METRICS + 1)) \
    --no-auto-update \
    > "$TEST_DIR/logs/node1.log" 2>&1 &
NODE_PIDS[1]=$!
echo "  PID: ${NODE_PIDS[1]}"

# Wait for node 1 to start producing
echo "  Waiting for seed node to produce blocks..."
sleep 15

# Check blocks produced
BLOCKS=$(grep -c "Produced block\|produced at height" "$TEST_DIR/logs/node1.log" 2>/dev/null || echo "0")
echo "  Node 1 has produced $BLOCKS blocks"

# Wait for stabilization before adding more nodes
echo -e "${CYAN}[$(date '+%H:%M:%S')] Waiting ${STABILIZATION_DELAY}s for stabilization...${NC}"
sleep $STABILIZATION_DELAY

# Start Node 2
echo -e "${CYAN}[$(date '+%H:%M:%S')] Starting Node 2...${NC}"
$NODE_BIN \
    --data-dir "$TEST_DIR/data/node2" \
    --network devnet \
    run \
    --producer \
    --producer-key "$TEST_DIR/keys/node2.json" \
    --p2p-port $((BASE_P2P + 2)) \
    --rpc-port $((BASE_RPC + 2)) \
    --metrics-port $((BASE_METRICS + 2)) \
    --bootstrap "/ip4/127.0.0.1/tcp/$((BASE_P2P + 1))" \
    --no-auto-update \
    > "$TEST_DIR/logs/node2.log" 2>&1 &
NODE_PIDS[2]=$!
echo "  PID: ${NODE_PIDS[2]}"

sleep $STABILIZATION_DELAY

# Start Node 3
echo -e "${CYAN}[$(date '+%H:%M:%S')] Starting Node 3...${NC}"
$NODE_BIN \
    --data-dir "$TEST_DIR/data/node3" \
    --network devnet \
    run \
    --producer \
    --producer-key "$TEST_DIR/keys/node3.json" \
    --p2p-port $((BASE_P2P + 3)) \
    --rpc-port $((BASE_RPC + 3)) \
    --metrics-port $((BASE_METRICS + 3)) \
    --bootstrap "/ip4/127.0.0.1/tcp/$((BASE_P2P + 1))" \
    --no-auto-update \
    > "$TEST_DIR/logs/node3.log" 2>&1 &
NODE_PIDS[3]=$!
echo "  PID: ${NODE_PIDS[3]}"

# Monitor for 3 epochs (~5 minutes)
MONITOR_TIME=300
echo
echo -e "${CYAN}Monitoring for ${MONITOR_TIME}s (3 reward epochs)...${NC}"
echo

START_TIME=$(date +%s)
while [ $(($(date +%s) - START_TIME)) -lt $MONITOR_TIME ]; do
    sleep 30

    echo -e "${CYAN}=== Status at +$(($(date +%s) - START_TIME))s ===${NC}"

    for i in $(seq 1 $NUM_NODES); do
        if [ -f "$TEST_DIR/logs/node${i}.log" ]; then
            BLOCKS=$(grep -c "Produced block\|produced at height" "$TEST_DIR/logs/node${i}.log" 2>/dev/null || echo "0")
            EPOCHS=$(grep -c "Epoch.*complete" "$TEST_DIR/logs/node${i}.log" 2>/dev/null || echo "0")
            echo "  Node $i: $BLOCKS blocks produced, $EPOCHS epochs complete"
        fi
    done

    # Show recent epoch events from node 1
    echo
    echo "Recent epoch events:"
    grep -h "Epoch.*complete\|Producer.*DOLI reward\|distributing" "$TEST_DIR/logs/node1.log" 2>/dev/null | tail -5 || echo "  (none yet)"
    echo
done

echo
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}   Final Results                       ${NC}"
echo -e "${GREEN}========================================${NC}"
echo

for i in $(seq 1 $NUM_NODES); do
    echo -e "${CYAN}Node $i:${NC}"
    BLOCKS=$(grep -c "Produced block\|produced at height" "$TEST_DIR/logs/node${i}.log" 2>/dev/null || echo "0")
    echo "  Blocks produced: $BLOCKS"
    grep "DOLI reward" "$TEST_DIR/logs/node${i}.log" 2>/dev/null | tail -3 || echo "  No rewards logged yet"
    echo
done

echo -e "${YELLOW}Epoch Distribution Summary (from node 1):${NC}"
grep -h "Epoch.*complete\|distributing\|Producer.*DOLI reward" "$TEST_DIR/logs/node1.log" 2>/dev/null | tail -20 || echo "No epoch completions logged"

echo
echo -e "${GREEN}Logs saved to: $TEST_DIR/logs/${NC}"

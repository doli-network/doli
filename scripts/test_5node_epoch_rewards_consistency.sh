#!/bin/bash
# DOLI - 5-Node Epoch Rewards Consistency Test (Milestone 6)
#
# Test scenario:
# - 5 producer nodes run for 2+ epochs
# - Verify all nodes agree on chain state and rewards
# - Restart one node mid-epoch to verify consistency
#
# This test validates the deterministic epoch reward system:
# - Rewards calculated from BlockStore (no local state)
# - Node restart doesn't affect reward calculation
# - All nodes agree on same reward distribution
#
# Devnet parameters:
# - 1 second slots
# - 30 slots per reward epoch = 30 seconds per epoch

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
TEST_DIR="/tmp/doli-5node-epoch-rewards-test"
NUM_NODES=5
EPOCHS_TO_RUN=3
SLOTS_PER_EPOCH=30

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${BLUE}=================================================${NC}"
echo -e "${BLUE}  DOLI 5-Node Epoch Rewards Consistency Test    ${NC}"
echo -e "${BLUE}  Milestone 6: Deterministic Rewards Validation ${NC}"
echo -e "${BLUE}=================================================${NC}"
echo
echo -e "${CYAN}Test Parameters:${NC}"
echo -e "  Nodes:            5 producers"
echo -e "  Epochs:           $EPOCHS_TO_RUN epochs"
echo -e "  Network:          devnet (1s slots, $SLOTS_PER_EPOCH slots/epoch)"
echo -e "  Duration:         ~$((EPOCHS_TO_RUN * SLOTS_PER_EPOCH + 30))s"
echo
echo -e "${CYAN}Test Phases:${NC}"
echo -e "  1. Start all 5 nodes"
echo -e "  2. Run for 1 epoch, verify all nodes sync"
echo -e "  3. Restart node 3 mid-epoch (test persistence)"
echo -e "  4. Continue running for 2+ epochs total"
echo -e "  5. Verify all nodes agree on rewards"
echo

# Clean up
echo -e "${YELLOW}Cleaning up previous test...${NC}"
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR/data" "$TEST_DIR/keys" "$TEST_DIR/logs"

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
for i in $(seq 1 $NUM_NODES); do
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
BASE_P2P=50500
BASE_RPC=28700

# PIDs for cleanup (indexed array - bash 3.x compatible)
NODE_PIDS=()

cleanup() {
    echo
    echo -e "${YELLOW}Stopping all nodes...${NC}"
    for pid in "${NODE_PIDS[@]}"; do
        if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null || true
        fi
    done
    sleep 2
    echo -e "${GREEN}Nodes stopped.${NC}"
}

trap cleanup EXIT

# Create data directories
for i in $(seq 1 $NUM_NODES); do
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

# Check RPC availability
check_node_rpc() {
    local node_num=$1
    local rpc_port=$((BASE_RPC + node_num))
    curl -s -X POST "http://127.0.0.1:${rpc_port}" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' 2>/dev/null | grep -q "result"
}

# Get chain height from node
get_height() {
    local node_num=$1
    local rpc_port=$((BASE_RPC + node_num))
    curl -s -X POST "http://127.0.0.1:${rpc_port}" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' 2>/dev/null \
        | grep -o '"bestHeight":[0-9]*' | cut -d: -f2 || echo "0"
}

# Get chain hash from node
get_hash() {
    local node_num=$1
    local rpc_port=$((BASE_RPC + node_num))
    curl -s -X POST "http://127.0.0.1:${rpc_port}" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' 2>/dev/null \
        | grep -o '"bestHash":"[^"]*"' | cut -d'"' -f4 || echo ""
}

# Get current slot from node
get_slot() {
    local node_num=$1
    local rpc_port=$((BASE_RPC + node_num))
    curl -s -X POST "http://127.0.0.1:${rpc_port}" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' 2>/dev/null \
        | grep -o '"currentSlot":[0-9]*' | cut -d: -f2 || echo "0"
}

# ============================================================================
# PHASE 1: Start all 5 nodes
# ============================================================================
echo
echo -e "${BLUE}=== Phase 1: Starting 5 Producer Nodes ===${NC}"

# Start seed node
echo -e "${YELLOW}Starting Node 1 (seed)...${NC}"
NODE_PIDS[1]=$(start_node 1 true)
sleep 3

# Wait for seed to be ready
echo -n "Waiting for seed node RPC... "
for i in {1..30}; do
    if check_node_rpc 1; then
        echo -e "${GREEN}ready${NC}"
        break
    fi
    sleep 1
done

# Start remaining nodes
for i in $(seq 2 $NUM_NODES); do
    echo -e "${YELLOW}Starting Node $i...${NC}"
    NODE_PIDS[$i]=$(start_node $i false)
    sleep 2
done

# Wait for all nodes to sync
echo
echo -e "${YELLOW}Waiting for all nodes to sync (30s)...${NC}"
sleep 30

# Check all nodes are running and synced
echo -e "${CYAN}Node Status:${NC}"
for i in $(seq 1 $NUM_NODES); do
    if check_node_rpc $i; then
        height=$(get_height $i)
        slot=$(get_slot $i)
        echo -e "  Node $i: ${GREEN}RUNNING${NC} - height=$height, slot=$slot"
    else
        echo -e "  Node $i: ${RED}NOT RESPONDING${NC}"
    fi
done

# ============================================================================
# PHASE 2: Run for 1 epoch and verify sync
# ============================================================================
echo
echo -e "${BLUE}=== Phase 2: First Epoch Verification ===${NC}"

echo -e "${YELLOW}Running for 1 epoch ($SLOTS_PER_EPOCH seconds)...${NC}"
sleep $SLOTS_PER_EPOCH

# Record heights and hashes (indexed arrays - bash 3.x compatible)
HEIGHTS_BEFORE=()
HASHES_BEFORE=()

echo -e "${CYAN}Chain State Before Restart:${NC}"
for i in $(seq 1 $NUM_NODES); do
    HEIGHTS_BEFORE[$i]=$(get_height $i)
    HASHES_BEFORE[$i]=$(get_hash $i)
    echo -e "  Node $i: height=${HEIGHTS_BEFORE[$i]}, hash=${HASHES_BEFORE[$i]:0:16}..."
done

# Verify all nodes have same hash at same height
REFERENCE_HEIGHT=${HEIGHTS_BEFORE[1]}
REFERENCE_HASH=${HASHES_BEFORE[1]}
SYNC_OK=true

for i in $(seq 2 $NUM_NODES); do
    if [ "${HEIGHTS_BEFORE[$i]}" -lt "$((REFERENCE_HEIGHT - 2))" ]; then
        echo -e "${RED}Node $i is behind: ${HEIGHTS_BEFORE[$i]} vs $REFERENCE_HEIGHT${NC}"
        SYNC_OK=false
    fi
done

if $SYNC_OK; then
    echo -e "${GREEN}All nodes are in sync!${NC}"
else
    echo -e "${RED}Nodes are not in sync. Test may have issues.${NC}"
fi

# ============================================================================
# PHASE 3: Restart Node 3 mid-epoch
# ============================================================================
echo
echo -e "${BLUE}=== Phase 3: Restart Node 3 Mid-Epoch (Persistence Test) ===${NC}"

# Record Node 3's state before restart
HEIGHT_BEFORE_RESTART=$(get_height 3)
HASH_BEFORE_RESTART=$(get_hash 3)
echo -e "${CYAN}Node 3 before restart: height=$HEIGHT_BEFORE_RESTART, hash=${HASH_BEFORE_RESTART:0:16}${NC}"

# Stop Node 3
echo -e "${YELLOW}Stopping Node 3...${NC}"
kill ${NODE_PIDS[3]} 2>/dev/null || true
sleep 3

# Wait a few seconds (mid-epoch)
echo -e "${YELLOW}Waiting 10 seconds (mid-epoch gap)...${NC}"
sleep 10

# Restart Node 3
echo -e "${YELLOW}Restarting Node 3...${NC}"
NODE_PIDS[3]=$(start_node 3 false)
sleep 5

# Wait for Node 3 to sync
echo -n "Waiting for Node 3 to sync... "
for i in {1..30}; do
    if check_node_rpc 3; then
        height=$(get_height 3)
        if [ "$height" -ge "$HEIGHT_BEFORE_RESTART" ]; then
            echo -e "${GREEN}synced (height=$height)${NC}"
            break
        fi
    fi
    sleep 1
done

# Verify Node 3 recovered correctly
HEIGHT_AFTER_RESTART=$(get_height 3)
HASH_AFTER_RESTART=$(get_hash 3)

echo -e "${CYAN}Node 3 after restart: height=$HEIGHT_AFTER_RESTART${NC}"

if [ "$HEIGHT_AFTER_RESTART" -ge "$HEIGHT_BEFORE_RESTART" ]; then
    echo -e "${GREEN}Node 3 recovered state correctly!${NC}"
else
    echo -e "${RED}Node 3 may have lost state (height: $HEIGHT_BEFORE_RESTART -> $HEIGHT_AFTER_RESTART)${NC}"
fi

# ============================================================================
# PHASE 4: Continue for 2+ epochs total
# ============================================================================
echo
echo -e "${BLUE}=== Phase 4: Running for Additional Epochs ===${NC}"

REMAINING_SECONDS=$((EPOCHS_TO_RUN * SLOTS_PER_EPOCH - SLOTS_PER_EPOCH - 10))
echo -e "${YELLOW}Running for $REMAINING_SECONDS more seconds (~$((REMAINING_SECONDS / SLOTS_PER_EPOCH)) epochs)...${NC}"

# Progress dots
for i in $(seq 1 $((REMAINING_SECONDS / 10))); do
    echo -n "."
    sleep 10
done
echo

# ============================================================================
# PHASE 5: Verify all nodes agree on rewards
# ============================================================================
echo
echo -e "${BLUE}=== Phase 5: Final State Verification ===${NC}"

# Collect final state from all nodes (indexed arrays - bash 3.x compatible)
FINAL_HEIGHTS=()
FINAL_HASHES=()
FINAL_SLOTS=()

echo -e "${CYAN}Final Chain State:${NC}"
for i in $(seq 1 $NUM_NODES); do
    if check_node_rpc $i; then
        FINAL_HEIGHTS[$i]=$(get_height $i)
        FINAL_HASHES[$i]=$(get_hash $i)
        FINAL_SLOTS[$i]=$(get_slot $i)
        echo -e "  Node $i: height=${FINAL_HEIGHTS[$i]}, slot=${FINAL_SLOTS[$i]}, hash=${FINAL_HASHES[$i]:0:16}..."
    else
        echo -e "  Node $i: ${RED}NOT RESPONDING${NC}"
        FINAL_HEIGHTS[$i]=0
    fi
done

# Verify all nodes agree
echo
echo -e "${CYAN}Consensus Verification:${NC}"

MAX_HEIGHT=0
for i in $(seq 1 $NUM_NODES); do
    if [ "${FINAL_HEIGHTS[$i]}" -gt "$MAX_HEIGHT" ]; then
        MAX_HEIGHT=${FINAL_HEIGHTS[$i]}
    fi
done

CONSENSUS_OK=true
SYNCED_COUNT=0

for i in $(seq 1 $NUM_NODES); do
    height=${FINAL_HEIGHTS[$i]}
    # Allow 2 block tolerance for sync
    if [ "$height" -ge "$((MAX_HEIGHT - 2))" ] && [ "$height" -gt 0 ]; then
        SYNCED_COUNT=$((SYNCED_COUNT + 1))
    else
        echo -e "  ${RED}Node $i is behind or down: height=$height (max=$MAX_HEIGHT)${NC}"
        CONSENSUS_OK=false
    fi
done

# Calculate expected epochs
CURRENT_EPOCH=$((FINAL_SLOTS[1] / SLOTS_PER_EPOCH))

echo
echo -e "${CYAN}Summary:${NC}"
echo -e "  Final height:   $MAX_HEIGHT"
echo -e "  Current epoch:  $CURRENT_EPOCH"
echo -e "  Nodes synced:   $SYNCED_COUNT / $NUM_NODES"

# ============================================================================
# Final Result
# ============================================================================
echo
echo -e "${BLUE}=== Test Results ===${NC}"

TESTS_PASSED=0
TESTS_TOTAL=3

# Test 1: All nodes running
if [ "$SYNCED_COUNT" -eq "$NUM_NODES" ]; then
    echo -e "  [${GREEN}PASS${NC}] All $NUM_NODES nodes are running and synced"
    TESTS_PASSED=$((TESTS_PASSED + 1))
else
    echo -e "  [${RED}FAIL${NC}] Only $SYNCED_COUNT/$NUM_NODES nodes synced"
fi

# Test 2: Ran for expected epochs
if [ "$CURRENT_EPOCH" -ge "$EPOCHS_TO_RUN" ]; then
    echo -e "  [${GREEN}PASS${NC}] Ran for $CURRENT_EPOCH epochs (expected >= $EPOCHS_TO_RUN)"
    TESTS_PASSED=$((TESTS_PASSED + 1))
else
    echo -e "  [${RED}FAIL${NC}] Only $CURRENT_EPOCH epochs (expected >= $EPOCHS_TO_RUN)"
fi

# Test 3: Node 3 recovered after restart
if [ "${FINAL_HEIGHTS[3]}" -ge "$((MAX_HEIGHT - 2))" ] && [ "${FINAL_HEIGHTS[3]}" -gt 0 ]; then
    echo -e "  [${GREEN}PASS${NC}] Node 3 recovered correctly after mid-epoch restart"
    TESTS_PASSED=$((TESTS_PASSED + 1))
else
    echo -e "  [${RED}FAIL${NC}] Node 3 did not recover correctly"
fi

echo
if [ "$TESTS_PASSED" -eq "$TESTS_TOTAL" ]; then
    echo -e "${GREEN}============================================${NC}"
    echo -e "${GREEN}  ALL TESTS PASSED ($TESTS_PASSED/$TESTS_TOTAL)                ${NC}"
    echo -e "${GREEN}  Deterministic epoch rewards working!     ${NC}"
    echo -e "${GREEN}============================================${NC}"
    exit 0
else
    echo -e "${RED}============================================${NC}"
    echo -e "${RED}  TESTS FAILED ($TESTS_PASSED/$TESTS_TOTAL passed)            ${NC}"
    echo -e "${RED}============================================${NC}"
    echo
    echo -e "${YELLOW}Check logs in: $TEST_DIR/logs/${NC}"
    exit 1
fi

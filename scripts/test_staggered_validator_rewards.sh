#!/bin/bash
# DOLI Devnet - Staggered Validator Reward Test
# This script tests that validators only receive rewards from the moment they join
#
# Test scenario:
# - 10 nodes join at different times (staggered by ~30 seconds each)
# - Node 1 (seed) starts immediately
# - Nodes 2-10 join progressively
# - We track when each node starts producing blocks and receiving rewards
#
# Devnet parameters:
# - 5 second slots
# - 20 slots per reward epoch = 100 seconds (~1.6 min) per epoch

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
TEST_DIR="/tmp/doli-staggered-test"
NUM_NODES=10
JOIN_DELAY=30  # Seconds between each node joining

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}   DOLI Devnet - Staggered Join Test   ${NC}"
echo -e "${BLUE}========================================${NC}"
echo
echo -e "${CYAN}Test Parameters:${NC}"
echo -e "  Nodes:           ${NUM_NODES}"
echo -e "  Join interval:   ${JOIN_DELAY}s"
echo -e "  Network:         devnet (5s slots, 20 slots/epoch = 100s per epoch)"
echo -e "  Total test time: ~$((NUM_NODES * JOIN_DELAY + 400))s (~$((NUM_NODES * JOIN_DELAY / 60 + 7)) min)"
echo

# Clean up previous test
echo -e "${YELLOW}Cleaning up previous test data...${NC}"
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR/data" "$TEST_DIR/keys" "$TEST_DIR/logs"

# Build the project in release mode
echo -e "${YELLOW}Building doli-node (release)...${NC}"
cd "$PROJECT_ROOT"
cargo build --release -p doli-node 2>&1 | grep -i "error\|warning\|compiling\|finished" | head -10

NODE_BIN="$PROJECT_ROOT/target/release/doli-node"
if [ ! -f "$NODE_BIN" ]; then
    echo -e "${RED}Error: doli-node binary not found${NC}"
    exit 1
fi

# Generate keys for all nodes
echo -e "${YELLOW}Generating ${NUM_NODES} producer keys...${NC}"

# Create key generation program
cat > "$TEST_DIR/keygen.rs" << 'KEYGEN_EOF'
use crypto::KeyPair;
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: keygen <output_file>");
        std::process::exit(1);
    }

    let keypair = KeyPair::generate();
    let private_key_hex = hex::encode(keypair.private_key().as_bytes());
    let public_key_hex = hex::encode(keypair.public_key().as_bytes());

    let wallet_json = format!(r#"{{
  "version": 1,
  "addresses": [
    {{
      "address": "ddoli1{}",
      "public_key": "{}",
      "private_key": "{}"
    }}
  ]
}}"#, &public_key_hex[..16], public_key_hex, private_key_hex);

    std::fs::write(&args[1], wallet_json).expect("Failed to write wallet file");
    println!("{}", &public_key_hex[..16]);
}
KEYGEN_EOF

cat > "$TEST_DIR/Cargo.toml" << CARGO_EOF
[package]
name = "keygen"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "keygen"
path = "keygen.rs"

[dependencies]
crypto = { path = "$PROJECT_ROOT/crates/crypto" }
hex = "0.4"
CARGO_EOF

cd "$TEST_DIR"
cargo build --release 2>/dev/null

# Generate keys
for i in $(seq 1 $NUM_NODES); do
    printf "  Node %2d: " "$i"
    ./target/release/keygen "$TEST_DIR/keys/node${i}.json"
done

echo

# Calculate ports (starting from devnet defaults)
BASE_P2P=50300
BASE_RPC=28540
BASE_METRICS=9000

# Store PIDs for cleanup
declare -a NODE_PIDS
declare -a NODE_JOIN_TIMES

cleanup() {
    echo
    echo -e "${YELLOW}Cleaning up nodes...${NC}"
    for pid in "${NODE_PIDS[@]}"; do
        if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null
        fi
    done
    echo -e "${GREEN}All nodes stopped.${NC}"
}

trap cleanup EXIT

# Create individual node data directories
for i in $(seq 1 $NUM_NODES); do
    mkdir -p "$TEST_DIR/data/node${i}"
done

# Function to start a node
start_node() {
    local node_num=$1
    local is_seed=$2

    local p2p_port=$((BASE_P2P + node_num))
    local rpc_port=$((BASE_RPC + node_num))
    local metrics_port=$((BASE_METRICS + node_num))

    local bootstrap_arg=""
    if [ "$is_seed" != "true" ]; then
        bootstrap_arg="--bootstrap /ip4/127.0.0.1/tcp/$((BASE_P2P + 1))"
    fi

    local log_file="$TEST_DIR/logs/node${node_num}.log"

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
        > "$log_file" 2>&1 &

    local pid=$!
    echo "$pid"
}

# Function to check node RPC
check_node_rpc() {
    local node_num=$1
    local rpc_port=$((BASE_RPC + node_num))

    curl -s -X POST "http://127.0.0.1:${rpc_port}" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' 2>/dev/null
}

# Function to get node's chain height
get_chain_height() {
    local node_num=$1
    local rpc_port=$((BASE_RPC + node_num))

    local result=$(curl -s -X POST "http://127.0.0.1:${rpc_port}" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' 2>/dev/null)

    echo "$result" | grep -o '"height":[0-9]*' | cut -d':' -f2
}

# Function to check node balance
get_node_balance() {
    local node_num=$1
    local rpc_port=$((BASE_RPC + node_num))
    local pubkey=$(cat "$TEST_DIR/keys/node${node_num}.json" | grep public_key | head -1 | cut -d'"' -f4)

    # Get balance (rewards should accumulate here after epoch boundaries)
    local result=$(curl -s -X POST "http://127.0.0.1:${rpc_port}" \
        -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"getBalance\",\"params\":[\"${pubkey}\"],\"id\":1}" 2>/dev/null)

    echo "$result" | grep -o '"balance":[0-9]*' | cut -d':' -f2
}

echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}   Starting Staggered Join Test        ${NC}"
echo -e "${GREEN}========================================${NC}"
echo

# Record start time
TEST_START=$(date +%s)

# Start node 1 (seed node)
echo -e "${CYAN}[$(date '+%H:%M:%S')] Starting Node 1 (seed node)...${NC}"
NODE_PIDS[1]=$(start_node 1 true)
NODE_JOIN_TIMES[1]=$(date +%s)
echo -e "  PID: ${NODE_PIDS[1]}, RPC: $((BASE_RPC + 1))"

# Wait for seed node to be ready
echo -n "  Waiting for seed node..."
for i in $(seq 1 30); do
    if check_node_rpc 1 >/dev/null; then
        echo -e " ${GREEN}ready${NC}"
        break
    fi
    sleep 1
    echo -n "."
done

# Start remaining nodes with staggered delays
for node_num in $(seq 2 $NUM_NODES); do
    echo
    echo -e "${CYAN}[$(date '+%H:%M:%S')] Waiting ${JOIN_DELAY}s before starting Node ${node_num}...${NC}"

    # Show countdown with current chain status
    for ((remaining=JOIN_DELAY; remaining>0; remaining--)); do
        height=$(get_chain_height 1)
        printf "\r  Countdown: %3ds | Chain height: %s " "$remaining" "${height:-?}"
        sleep 1
    done
    echo

    echo -e "${CYAN}[$(date '+%H:%M:%S')] Starting Node ${node_num}...${NC}"
    NODE_PIDS[$node_num]=$(start_node $node_num false)
    NODE_JOIN_TIMES[$node_num]=$(date +%s)
    echo -e "  PID: ${NODE_PIDS[$node_num]}, RPC: $((BASE_RPC + node_num))"

    # Wait for node to sync
    echo -n "  Waiting for node to sync..."
    for i in $(seq 1 20); do
        if check_node_rpc $node_num >/dev/null; then
            local_height=$(get_chain_height $node_num)
            seed_height=$(get_chain_height 1)
            if [ -n "$local_height" ] && [ -n "$seed_height" ] && [ "$local_height" -ge "$((seed_height - 5))" ]; then
                echo -e " ${GREEN}synced (height: $local_height)${NC}"
                break
            fi
        fi
        sleep 1
        echo -n "."
    done
done

echo
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}   All Nodes Started - Monitoring      ${NC}"
echo -e "${GREEN}========================================${NC}"
echo

# Monitor for multiple epochs (devnet: 20 slots/epoch at 5s = 100s per epoch)
# All nodes join over 300s (10 nodes × 30s), so we need to monitor long enough
# to see several epoch boundaries after the last node joins
MONITOR_DURATION=600  # 10 minutes = ~6 epoch cycles
echo -e "${CYAN}Monitoring for ${MONITOR_DURATION}s (~6 reward epochs)...${NC}"
echo -e "${CYAN}Devnet: 20 slots/epoch × 5s/slot = 100s per epoch (1.6 min)${NC}"

echo
echo -e "${YELLOW}Epoch events will be logged. Checking logs periodically...${NC}"
echo

# Create monitoring summary file
SUMMARY_FILE="$TEST_DIR/test_summary.md"
cat > "$SUMMARY_FILE" << EOF
# Staggered Validator Join Test Summary

Test started: $(date)
Number of nodes: $NUM_NODES
Join interval: ${JOIN_DELAY}s

## Node Join Times (Relative to Test Start)

| Node | Join Time | Join Offset | Expected First Block |
|------|-----------|-------------|---------------------|
EOF

for node_num in $(seq 1 $NUM_NODES); do
    offset=$((NODE_JOIN_TIMES[$node_num] - TEST_START))
    printf "| %d | %s | +%ds | ~slot %d |\n" \
        "$node_num" \
        "$(date -r ${NODE_JOIN_TIMES[$node_num]} '+%H:%M:%S')" \
        "$offset" \
        "$((offset / 5))" >> "$SUMMARY_FILE"
done

echo >> "$SUMMARY_FILE"
echo "## Epoch Reward Events" >> "$SUMMARY_FILE"
echo >> "$SUMMARY_FILE"

# Monitor loop
start_monitor=$(date +%s)
last_check=0

while [ $(($(date +%s) - start_monitor)) -lt $MONITOR_DURATION ]; do
    current_time=$(($(date +%s) - start_monitor))

    # Check every 30 seconds
    if [ $((current_time - last_check)) -ge 30 ]; then
        last_check=$current_time
        echo
        echo -e "${CYAN}=== Status at +${current_time}s ===${NC}"

        # Check each node
        printf "%-6s %-8s %-10s %-15s\n" "Node" "Height" "Balance" "Status"
        printf "%-6s %-8s %-10s %-15s\n" "----" "------" "-------" "------"

        for node_num in $(seq 1 $NUM_NODES); do
            height=$(get_chain_height $node_num 2>/dev/null)
            balance=$(get_node_balance $node_num 2>/dev/null)

            if [ -n "$height" ]; then
                status="${GREEN}running${NC}"
                # Convert balance to DOLI (divide by 10^8)
                if [ -n "$balance" ] && [ "$balance" -gt 0 ]; then
                    balance_doli=$((balance / 100000000))
                else
                    balance_doli=0
                fi
            else
                status="${RED}error${NC}"
                height="?"
                balance_doli="?"
            fi

            printf "%-6d %-8s %-10s " "$node_num" "$height" "${balance_doli} DOLI"
            echo -e "$status"
        done

        # Check for epoch events in logs
        echo
        echo -e "${YELLOW}Recent epoch events:${NC}"
        grep -h "Epoch.*complete\|epoch reward\|DOLI reward" "$TEST_DIR/logs/"*.log 2>/dev/null | tail -5 || echo "  (none yet)"
    fi

    sleep 5
done

echo
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}   Test Complete - Final Analysis      ${NC}"
echo -e "${GREEN}========================================${NC}"
echo

# Final analysis
echo -e "${CYAN}Final Node Status:${NC}"
printf "%-6s %-8s %-12s %-15s %-20s\n" "Node" "Height" "Balance" "Blocks Made" "First Block"
printf "%-6s %-8s %-12s %-15s %-20s\n" "----" "------" "--------" "-----------" "-----------"

for node_num in $(seq 1 $NUM_NODES); do
    height=$(get_chain_height $node_num 2>/dev/null)
    balance=$(get_node_balance $node_num 2>/dev/null)

    # Count blocks produced by this node (from logs)
    blocks_made=$(grep -c "Produced block" "$TEST_DIR/logs/node${node_num}.log" 2>/dev/null || echo "0")

    # Find first block time
    first_block=$(grep "Produced block" "$TEST_DIR/logs/node${node_num}.log" 2>/dev/null | head -1 | cut -d' ' -f1-2 || echo "none")

    if [ -n "$balance" ] && [ "$balance" -gt 0 ]; then
        balance_doli=$((balance / 100000000))
    else
        balance_doli=0
    fi

    printf "%-6d %-8s %-12s %-15s %-20s\n" \
        "$node_num" \
        "${height:-?}" \
        "${balance_doli} DOLI" \
        "$blocks_made" \
        "${first_block:-none}"
done

# Epoch reward analysis
echo
echo -e "${CYAN}Epoch Reward Distribution:${NC}"
grep -h "Producer.*DOLI reward\|Epoch.*complete" "$TEST_DIR/logs/node1.log" 2>/dev/null | tail -20 || echo "(no epoch completions yet)"

# Save final summary
cat >> "$SUMMARY_FILE" << EOF

## Final Results (at $(date))

### Node Balances
$(for node_num in $(seq 1 $NUM_NODES); do
    balance=$(get_node_balance $node_num 2>/dev/null)
    blocks_made=$(grep -c "Produced block" "$TEST_DIR/logs/node${node_num}.log" 2>/dev/null || echo "0")
    if [ -n "$balance" ] && [ "$balance" -gt 0 ]; then
        balance_doli=$((balance / 100000000))
    else
        balance_doli=0
    fi
    echo "- Node $node_num: ${balance_doli} DOLI (produced $blocks_made blocks)"
done)

### Epoch Events (from Node 1 logs)
\`\`\`
$(grep -h "Epoch.*complete\|Producer.*DOLI reward" "$TEST_DIR/logs/node1.log" 2>/dev/null | tail -30 || echo "No epoch completions recorded")
\`\`\`

## Test Conclusion

If the test is working correctly:
1. Validators should ONLY receive rewards for epochs they participated in
2. A node joining mid-epoch should get its fair share in the NEXT complete epoch
3. All participating producers get equal share regardless of how many blocks they produced
EOF

echo
echo -e "${GREEN}Summary saved to: $SUMMARY_FILE${NC}"
echo -e "${GREEN}Logs available in: $TEST_DIR/logs/${NC}"
echo
echo -e "${YELLOW}To view live logs:${NC}"
echo -e "  tail -f $TEST_DIR/logs/node1.log"
echo
echo -e "${YELLOW}To check epoch rewards manually:${NC}"
echo -e "  grep 'Epoch.*complete\\|DOLI reward' $TEST_DIR/logs/*.log"

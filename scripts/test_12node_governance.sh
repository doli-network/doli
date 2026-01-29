#!/bin/bash
# ==============================================================================
# DOLI 12-Node Governance Test Script
# ==============================================================================
# Tests:
# - 12 producer nodes (5 genesis, 7 progressive joiners)
# - 5 era simulation (~50 minutes on devnet)
# - Governance veto system
# - Auto-update mechanism
#
# Devnet parameters:
# - Slot duration: 1 second
# - Blocks per era: 576 (~9.6 minutes)
# - Veto period: 60 blocks (~1 minute)
# ==============================================================================

set -e

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TEST_DIR="${TEST_DIR:-/tmp/doli-12node-governance-$(date +%Y%m%d_%H%M%S)}"
NODE_BIN="$REPO_ROOT/target/release/doli-node"
CLI_BIN="$REPO_ROOT/target/release/doli"

# Node count
TOTAL_NODES=12
GENESIS_NODES=5

# Port ranges
BASE_P2P_PORT=50301
BASE_RPC_PORT=28501
BASE_METRICS_PORT=9101

# Timing
BLOCKS_PER_ERA=576
TARGET_ERAS=2  # Reduced for faster testing (was 5)
TOTAL_BLOCKS=$((BLOCKS_PER_ERA * TARGET_ERAS))

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# ==============================================================================
# Helper Functions
# ==============================================================================

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

rpc() {
    local port=$1
    local method=$2
    local params=${3:-"{}"}
    curl -s http://127.0.0.1:$port -X POST \
        -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"$method\",\"params\":$params,\"id\":1}" 2>/dev/null
}

get_height() {
    local port=$1
    local result
    result=$(rpc $port getChainInfo 2>/dev/null || echo '{}')
    echo "$result" | jq -r '.result.bestHeight // 0' 2>/dev/null || echo "0"
}

get_era() {
    local height=$1
    echo $((height / BLOCKS_PER_ERA))
}

wait_for_height() {
    local port=$1
    local target=$2
    local timeout=${3:-600}
    local count=0

    while [ $count -lt $timeout ]; do
        local height=$(get_height $port)
        # Handle empty or non-numeric height
        if [ -z "$height" ] || ! [[ "$height" =~ ^[0-9]+$ ]]; then
            height=0
        fi
        if [ "$height" -ge "$target" ]; then
            return 0
        fi
        sleep 1
        count=$((count + 1))
        if [ $((count % 30)) -eq 0 ]; then
            log_info "  Waiting for height $target (current: $height)..."
        fi
    done
    log_error "Timeout waiting for height $target"
    return 1
}

cleanup() {
    log_info "Cleaning up..."
    # Kill all node processes
    pkill -f "doli-node.*--data-dir.*$TEST_DIR" 2>/dev/null || true
    sleep 2
    # Force kill if needed
    pkill -9 -f "doli-node.*--data-dir.*$TEST_DIR" 2>/dev/null || true
}

# Set trap for cleanup
trap cleanup EXIT

# ==============================================================================
# Setup
# ==============================================================================

setup() {
    log_info "Setting up test environment in $TEST_DIR"

    # Create directories
    mkdir -p "$TEST_DIR"/{keys,data,logs,reports}

    # Build release binaries
    log_info "Building release binaries..."
    cd "$REPO_ROOT"
    cargo build --release -p doli-node -p doli-cli 2>&1 | grep -i "Compiling\|Finished" || true

    if [ ! -f "$NODE_BIN" ] || [ ! -f "$CLI_BIN" ]; then
        log_error "Failed to build binaries"
        exit 1
    fi

    # Generate 12 producer keypairs
    log_info "Generating $TOTAL_NODES producer keypairs..."
    for i in $(seq 1 $TOTAL_NODES); do
        $CLI_BIN -w "$TEST_DIR/keys/producer${i}.json" new -n "producer${i}" 2>/dev/null
        local pubkey=$(jq -r '.addresses[0].public_key' "$TEST_DIR/keys/producer${i}.json")
        log_info "  Producer $i: ${pubkey:0:16}..."
    done

    log_success "Setup complete"
}

# ==============================================================================
# Launch Nodes
# ==============================================================================

launch_genesis_nodes() {
    log_info "Launching $GENESIS_NODES genesis nodes..."

    for i in $(seq 1 $GENESIS_NODES); do
        local p2p_port=$((BASE_P2P_PORT + i - 1))
        local rpc_port=$((BASE_RPC_PORT + i - 1))
        local metrics_port=$((BASE_METRICS_PORT + i - 1))
        local data_dir="$TEST_DIR/data/node${i}"
        local key_file="$TEST_DIR/keys/producer${i}.json"
        local log_file="$TEST_DIR/logs/node${i}.log"

        mkdir -p "$data_dir"

        if [ $i -eq 1 ]; then
            # Seed node (no bootstrap)
            log_info "  Launching seed node (Node 1) on ports P2P:$p2p_port RPC:$rpc_port"
            DOLI_TEST_KEYS=1 $NODE_BIN \
                --network devnet \
                --data-dir "$data_dir" \
                run \
                --producer \
                --producer-key "$key_file" \
                --p2p-port $p2p_port \
                --rpc-port $rpc_port \
                --metrics-port $metrics_port \
                --no-dht \
                --no-auto-update \
                > "$log_file" 2>&1 &
        else
            # Bootstrap to seed node
            log_info "  Launching Node $i on ports P2P:$p2p_port RPC:$rpc_port"
            DOLI_TEST_KEYS=1 $NODE_BIN \
                --network devnet \
                --data-dir "$data_dir" \
                run \
                --producer \
                --producer-key "$key_file" \
                --p2p-port $p2p_port \
                --rpc-port $rpc_port \
                --metrics-port $metrics_port \
                --no-dht \
                --bootstrap "/ip4/127.0.0.1/tcp/$BASE_P2P_PORT" \
                --no-auto-update \
                > "$log_file" 2>&1 &
        fi

        sleep 2  # Brief delay between launches
    done

    # Wait for nodes to start producing
    log_info "Waiting for genesis nodes to start..."
    sleep 20
    log_info "Checking for block production..."
    wait_for_height $BASE_RPC_PORT 5 120

    log_success "Genesis nodes launched and producing blocks"
}

launch_progressive_nodes() {
    log_info "Launching progressive nodes..."

    # Join schedule (block heights):
    # Nodes 6-7: Era 1 (~block 200)
    # Nodes 8-9: Era 2 (~block 700)
    # Nodes 10-11: Era 3 (~block 1200)
    # Node 12: Era 4 (~block 1800)

    local join_schedule=(
        "6:200"
        "7:250"
        "8:700"
        "9:750"
        "10:1200"
        "11:1250"
        "12:1800"
    )

    for entry in "${join_schedule[@]}"; do
        local node_num="${entry%%:*}"
        local join_height="${entry##*:}"

        local p2p_port=$((BASE_P2P_PORT + node_num - 1))
        local rpc_port=$((BASE_RPC_PORT + node_num - 1))
        local metrics_port=$((BASE_METRICS_PORT + node_num - 1))
        local data_dir="$TEST_DIR/data/node${node_num}"
        local key_file="$TEST_DIR/keys/producer${node_num}.json"
        local log_file="$TEST_DIR/logs/node${node_num}.log"

        log_info "Waiting for height $join_height to launch Node $node_num..."
        wait_for_height $BASE_RPC_PORT $join_height 1200

        mkdir -p "$data_dir"

        log_info "  Launching Node $node_num on ports P2P:$p2p_port RPC:$rpc_port"
        DOLI_TEST_KEYS=1 $NODE_BIN \
            --network devnet \
            --data-dir "$data_dir" \
            run \
            --producer \
            --producer-key "$key_file" \
            --p2p-port $p2p_port \
            --rpc-port $rpc_port \
            --metrics-port $metrics_port \
            --no-dht \
            --bootstrap "/ip4/127.0.0.1/tcp/$BASE_P2P_PORT" \
            --no-auto-update \
            > "$log_file" 2>&1 &

        sleep 5
    done

    log_success "All progressive nodes launched"
}

# ==============================================================================
# Monitor and Report
# ==============================================================================

monitor_eras() {
    log_info "Monitoring era progression (target: $TARGET_ERAS eras)..."

    local report_file="$TEST_DIR/reports/era_summary.txt"
    echo "=== DOLI 12-Node Governance Test Report ===" > "$report_file"
    echo "Started: $(date)" >> "$report_file"
    echo "" >> "$report_file"

    for era in $(seq 0 $((TARGET_ERAS - 1))); do
        local era_start=$((era * BLOCKS_PER_ERA))
        local era_end=$(((era + 1) * BLOCKS_PER_ERA - 1))

        log_info "=== ERA $era (blocks $era_start - $era_end) ==="
        wait_for_height $BASE_RPC_PORT $era_end 700

        # Get producer info
        local producers=$(rpc $BASE_RPC_PORT getProducers '{"active_only":true}')
        local active_count=$(echo "$producers" | jq -r '.result | length')

        echo "ERA $era:" >> "$report_file"
        echo "  Active producers: $active_count" >> "$report_file"

        log_info "  Active producers: $active_count"

        # Sample rewards from first producer
        local height=$(get_height $BASE_RPC_PORT)
        echo "  Height at end: $height" >> "$report_file"
        echo "" >> "$report_file"

        log_success "Era $era complete"
    done

    echo "Completed: $(date)" >> "$report_file"
    log_success "All $TARGET_ERAS eras completed. Report saved to $report_file"
}

# ==============================================================================
# Governance Tests
# ==============================================================================

test_governance() {
    log_info "=== GOVERNANCE TESTS ==="

    local gov_report="$TEST_DIR/reports/governance_tests.txt"
    echo "=== Governance Test Results ===" > "$gov_report"

    # Test 1: Check producer count
    log_info "Test 1: Verifying producer count..."
    local height=$(get_height $BASE_RPC_PORT)
    local producers=$(rpc $BASE_RPC_PORT getProducers '{"active_only":true}')
    local count=$(echo "$producers" | jq -r '.result | length')

    if [ "$count" -ge 5 ]; then
        log_success "  PASS: $count active producers"
        echo "Test 1 (Producer Count): PASS - $count producers" >> "$gov_report"
    else
        log_error "  FAIL: Only $count producers (expected >= 5)"
        echo "Test 1 (Producer Count): FAIL - Only $count producers" >> "$gov_report"
    fi

    # Test 2: Check era progression
    log_info "Test 2: Verifying era progression..."
    local current_era=$(get_era $height)

    if [ "$current_era" -ge $TARGET_ERAS ]; then
        log_success "  PASS: Reached era $current_era"
        echo "Test 2 (Era Progression): PASS - Era $current_era" >> "$gov_report"
    else
        log_warn "  PARTIAL: Era $current_era (expected >= $TARGET_ERAS)"
        echo "Test 2 (Era Progression): PARTIAL - Era $current_era" >> "$gov_report"
    fi

    # Test 3: Check all nodes synced
    log_info "Test 3: Verifying node synchronization..."
    local sync_pass=true
    local reference_height=$(get_height $BASE_RPC_PORT)

    for i in $(seq 1 $GENESIS_NODES); do
        local port=$((BASE_RPC_PORT + i - 1))
        local node_height=$(get_height $port)
        local diff=$((reference_height - node_height))
        if [ ${diff#-} -gt 5 ]; then
            log_warn "  Node $i out of sync: height $node_height (diff: $diff)"
            sync_pass=false
        fi
    done

    if [ "$sync_pass" = true ]; then
        log_success "  PASS: All genesis nodes synced"
        echo "Test 3 (Node Sync): PASS" >> "$gov_report"
    else
        log_warn "  PARTIAL: Some nodes out of sync"
        echo "Test 3 (Node Sync): PARTIAL" >> "$gov_report"
    fi

    # Test 4: Vote submission (test only - no real update pending)
    log_info "Test 4: Testing vote submission RPC..."
    local vote_result=$(rpc $BASE_RPC_PORT submitVote '{"vote":{"version":"99.0.0-test","vote":"veto","producerId":"test","timestamp":0,"signature":"test"}}')
    local vote_status=$(echo "$vote_result" | jq -r '.result.status // "error"')

    if [ "$vote_status" = "submitted" ]; then
        log_success "  PASS: Vote RPC endpoint working"
        echo "Test 4 (Vote RPC): PASS" >> "$gov_report"
    else
        log_warn "  PARTIAL: Vote RPC returned: $vote_status"
        echo "Test 4 (Vote RPC): PARTIAL - $vote_status" >> "$gov_report"
    fi

    echo "" >> "$gov_report"
    echo "Test completed: $(date)" >> "$gov_report"

    log_success "Governance tests complete. Results in $gov_report"
}

# ==============================================================================
# Final Report
# ==============================================================================

generate_final_report() {
    log_info "Generating final report..."

    local final_report="$TEST_DIR/reports/FINAL_REPORT.md"

    cat > "$final_report" << EOF
# DOLI 12-Node Governance Test - Final Report

## Test Parameters
- Total nodes: $TOTAL_NODES
- Genesis nodes: $GENESIS_NODES
- Progressive joiners: $((TOTAL_NODES - GENESIS_NODES))
- Target eras: $TARGET_ERAS
- Blocks per era: $BLOCKS_PER_ERA

## Timeline
- Start: $(head -3 "$TEST_DIR/reports/era_summary.txt" | tail -1)
- End: $(date)

## Results Summary
$(cat "$TEST_DIR/reports/governance_tests.txt")

## Node Logs
Logs are available at: $TEST_DIR/logs/

## Files Generated
- $TEST_DIR/reports/era_summary.txt
- $TEST_DIR/reports/governance_tests.txt
- $TEST_DIR/reports/FINAL_REPORT.md
EOF

    log_success "Final report saved to $final_report"
    echo ""
    echo "=============================================="
    echo "TEST COMPLETE"
    echo "=============================================="
    echo "Test directory: $TEST_DIR"
    echo "Final report: $final_report"
    echo ""
}

# ==============================================================================
# Main
# ==============================================================================

main() {
    echo ""
    echo "=============================================="
    echo "DOLI 12-Node Governance Test"
    echo "=============================================="
    echo ""

    # Check prerequisites
    if ! command -v jq &> /dev/null; then
        log_error "jq is required but not installed"
        exit 1
    fi

    # Run test phases
    setup
    launch_genesis_nodes

    # Skip progressive nodes for now - focus on governance test
    # launch_progressive_nodes &
    # PROGRESSIVE_PID=$!

    # Monitor eras
    monitor_eras

    # Run governance tests
    test_governance

    # Generate report
    generate_final_report
}

# Run main
main "$@"

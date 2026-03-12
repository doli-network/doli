#!/bin/bash
# DOLI - ISSUE-5: Restart Sync Height Double-Counting Test
#
# Tests that restarted nodes do NOT double-count block heights.
#
# Scenario:
#   1. Use existing running devnet (5 nodes)
#   2. Wait for sufficient chain height (>= 20 blocks)
#   3. Record pre-kill heights and hashes
#   4. Kill nodes 2, 3, 4 (keep 0, 1 alive for fallback production)
#   5. Wait for chain to advance on surviving nodes
#   6. Restart killed nodes with FIXED binary
#   7. Verify:
#      a. All nodes converge to same height and hash
#      b. No height double-counting (restarted height <= surviving node height)
#      c. Logs show "Sync manager initialized at height X" (Fix A)
#      d. No "already in store" warnings (Fix A prevents re-download)
#
# Prerequisites:
#   - Running devnet: doli-node devnet init --nodes 5 && doli-node devnet start
#   - Binary rebuilt with ISSUE-5 fix: cargo build --release
#
# Run time: ~3-5 minutes (depends on current chain height)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
NODE_BIN="$PROJECT_ROOT/target/release/doli-node"
DEVNET_DIR="$HOME/.doli/devnet"
LOG_DIR="$DEVNET_DIR/logs"

# Test parameters
MIN_HEIGHT=20          # Minimum height before killing nodes
KILL_NODES=(2 3 4)     # Nodes to kill
ALIVE_NODES=(0 1)      # Nodes that stay alive (fallback producers)
POST_KILL_WAIT=40      # Seconds to wait after kill (4 blocks worth)
SYNC_WAIT=60           # Seconds to wait for restarted nodes to sync
HEIGHT_TOLERANCE=3     # Max height difference for "converged"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'
BOLD='\033[1m'

# Counters
PASS=0
FAIL=0
WARN=0

pass() { PASS=$((PASS + 1)); echo -e "  ${GREEN}PASS${NC}: $1"; }
fail() { FAIL=$((FAIL + 1)); echo -e "  ${RED}FAIL${NC}: $1"; }
warn() { WARN=$((WARN + 1)); echo -e "  ${YELLOW}WARN${NC}: $1"; }
info() { echo -e "  ${CYAN}INFO${NC}: $1"; }

rpc_chain_info() {
    local port=$((28500 + $1))
    curl -s --max-time 5 http://127.0.0.1:$port -X POST \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' 2>/dev/null
}

get_height() {
    rpc_chain_info "$1" | jq -r '.result.bestHeight // "DOWN"'
}

get_hash() {
    rpc_chain_info "$1" | jq -r '.result.bestHash // "DOWN"'
}

get_slot() {
    rpc_chain_info "$1" | jq -r '.result.bestSlot // "DOWN"'
}

echo -e "${BLUE}=========================================================${NC}"
echo -e "${BLUE}  DOLI ISSUE-5: Restart Sync Height Double-Counting Test ${NC}"
echo -e "${BLUE}=========================================================${NC}"
echo
echo -e "${CYAN}Test Plan:${NC}"
echo -e "  1. Verify existing devnet is healthy"
echo -e "  2. Wait for height >= $MIN_HEIGHT"
echo -e "  3. Kill nodes ${KILL_NODES[*]}"
echo -e "  4. Wait ${POST_KILL_WAIT}s for fallback production"
echo -e "  5. Restart killed nodes with fixed binary"
echo -e "  6. Wait ${SYNC_WAIT}s for sync convergence"
echo -e "  7. Verify heights correct (no double-counting)"
echo

# ===================================================================
# PHASE 1: Verify devnet is running
# ===================================================================
echo -e "${BOLD}${BLUE}PHASE 1: Verify Devnet Health${NC}"
echo -e "${BLUE}-------------------------------------------${NC}"

if [ ! -f "$DEVNET_DIR/devnet.toml" ]; then
    echo -e "${RED}No devnet.toml found. Start devnet first: doli-node devnet init --nodes 5 && doli-node devnet start${NC}"
    exit 1
fi

NODE_COUNT=$(grep node_count "$DEVNET_DIR/devnet.toml" | sed 's/[^0-9]//g')
info "Devnet config: $NODE_COUNT nodes"

if [ ! -f "$NODE_BIN" ]; then
    echo -e "${RED}Binary not found at $NODE_BIN. Run: cargo build --release${NC}"
    exit 1
fi

# Verify all 5 nodes are responding
ALL_UP=true
for i in $(seq 0 $((NODE_COUNT - 1))); do
    h=$(get_height $i)
    if [ "$h" = "DOWN" ]; then
        fail "Node $i is DOWN"
        ALL_UP=false
    else
        pass "Node $i alive at height $h"
    fi
done

if [ "$ALL_UP" != "true" ]; then
    echo -e "${RED}Not all nodes are up. Fix devnet before testing.${NC}"
    exit 1
fi

# ===================================================================
# PHASE 2: Wait for minimum height
# ===================================================================
echo
echo -e "${BOLD}${BLUE}PHASE 2: Wait for Height >= $MIN_HEIGHT${NC}"
echo -e "${BLUE}-------------------------------------------${NC}"

CURRENT_HEIGHT=$(get_height 0)
if [ "$CURRENT_HEIGHT" -ge "$MIN_HEIGHT" ]; then
    pass "Already at height $CURRENT_HEIGHT (>= $MIN_HEIGHT)"
else
    BLOCKS_NEEDED=$((MIN_HEIGHT - CURRENT_HEIGHT))
    WAIT_ESTIMATE=$((BLOCKS_NEEDED * 10 + 10))
    info "At height $CURRENT_HEIGHT, need $BLOCKS_NEEDED more blocks (~${WAIT_ESTIMATE}s)"

    while true; do
        sleep 10
        CURRENT_HEIGHT=$(get_height 0)
        if [ "$CURRENT_HEIGHT" = "DOWN" ]; then
            fail "Node 0 went DOWN while waiting"
            exit 1
        fi
        info "Height: $CURRENT_HEIGHT / $MIN_HEIGHT"
        if [ "$CURRENT_HEIGHT" -ge "$MIN_HEIGHT" ]; then
            pass "Reached height $CURRENT_HEIGHT"
            break
        fi
    done
fi

# ===================================================================
# PHASE 3: Record pre-kill state and kill nodes
# ===================================================================
echo
echo -e "${BOLD}${BLUE}PHASE 3: Record State & Kill Nodes${NC}"
echo -e "${BLUE}-------------------------------------------${NC}"

# Record pre-kill state for all nodes (using indexed vars to avoid bash 3 declare -A)
for i in $(seq 0 $((NODE_COUNT - 1))); do
    eval "PRE_KILL_HEIGHT_$i=$(get_height $i)"
    eval "PRE_KILL_HASH_$i=$(get_hash $i)"
    eval "h=\$PRE_KILL_HEIGHT_$i; hash=\$PRE_KILL_HASH_$i"
    info "Node $i pre-kill: height=$h hash=${hash:0:16}..."
done

# Verify consensus (all same hash)
ALL_CONSENSUS=true
for i in $(seq 1 $((NODE_COUNT - 1))); do
    eval "h=\$PRE_KILL_HASH_$i"
    if [ "$h" != "$PRE_KILL_HASH_0" ]; then
        warn "Node $i has different hash pre-kill (possible minor fork)"
        ALL_CONSENSUS=false
    fi
done
if [ "$ALL_CONSENSUS" = "true" ]; then
    pass "All nodes in consensus pre-kill"
fi

# Kill nodes 2, 3, 4
echo
info "Killing nodes: ${KILL_NODES[*]}"
for i in "${KILL_NODES[@]}"; do
    eval "pkh=\$PRE_KILL_HEIGHT_$i"
    P2P_PORT=$((50300 + i))
    # Use port-based kill (most reliable) with PID file as fallback
    PID=$(lsof -ti :$P2P_PORT 2>/dev/null | head -1)
    if [ -z "$PID" ] && [ -f "$DEVNET_DIR/pids/node${i}.pid" ]; then
        PID=$(cat "$DEVNET_DIR/pids/node${i}.pid" 2>/dev/null)
    fi
    if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        kill "$PID"
        info "Killed node $i (PID $PID) at height $pkh"
    else
        warn "Node $i: no process found on port $P2P_PORT"
    fi
done

# Wait for kills and port release (macOS needs extra time for TIME_WAIT)
sleep 5

# Verify killed nodes are down
for i in "${KILL_NODES[@]}"; do
    h=$(get_height $i)
    if [ "$h" = "DOWN" ]; then
        pass "Node $i confirmed DOWN"
    else
        fail "Node $i still responding after kill (height=$h)"
    fi
done

# Verify alive nodes still running
for i in "${ALIVE_NODES[@]}"; do
    h=$(get_height $i)
    if [ "$h" != "DOWN" ]; then
        pass "Node $i still alive at height $h"
    else
        fail "Node $i went DOWN (should be alive)"
        exit 1
    fi
done

# ===================================================================
# PHASE 4: Wait for chain to advance (fallback production)
# ===================================================================
echo
echo -e "${BOLD}${BLUE}PHASE 4: Wait ${POST_KILL_WAIT}s for Fallback Production${NC}"
echo -e "${BLUE}-------------------------------------------${NC}"

info "Waiting ${POST_KILL_WAIT}s for surviving nodes to produce blocks..."
HALF_WAIT=$((POST_KILL_WAIT / 2))
sleep $HALF_WAIT
MID_HEIGHT=$(get_height 0)
info "Mid-wait: Node 0 at height $MID_HEIGHT"
sleep $((POST_KILL_WAIT - HALF_WAIT))

# Record post-wait heights of alive nodes
POST_KILL_HEIGHT_0=$(get_height 0)
POST_KILL_HEIGHT_1=$(get_height 1)
BLOCKS_PRODUCED=$((POST_KILL_HEIGHT_0 - PRE_KILL_HEIGHT_0))

if [ "$BLOCKS_PRODUCED" -gt 0 ]; then
    pass "Chain advanced by $BLOCKS_PRODUCED blocks during kill window (height: $PRE_KILL_HEIGHT_0 -> $POST_KILL_HEIGHT_0)"
else
    warn "No blocks produced during kill window (may need more time for fallback)"
fi

info "Surviving node heights: Node0=$POST_KILL_HEIGHT_0, Node1=$POST_KILL_HEIGHT_1"

# ===================================================================
# PHASE 5: Restart killed nodes with FIXED binary
# ===================================================================
echo
echo -e "${BOLD}${BLUE}PHASE 5: Restart Killed Nodes (Fixed Binary)${NC}"
echo -e "${BLUE}-------------------------------------------${NC}"

# Clear old logs for killed nodes to isolate new log entries
for i in "${KILL_NODES[@]}"; do
    LOG_FILE="$LOG_DIR/node${i}.log"
    if [ -f "$LOG_FILE" ]; then
        # Mark where old log ends so we can find new entries
        echo "===== RESTART MARKER $(date +%s) =====" >> "$LOG_FILE"
    fi
done

for i in "${KILL_NODES[@]}"; do
    P2P_PORT=$((50300 + i))
    RPC_PORT=$((28500 + i))
    METRICS_PORT=$((9000 + i))
    DATA_DIR="$DEVNET_DIR/data/node${i}"
    KEY_FILE="$DEVNET_DIR/keys/producer_${i}.json"
    CHAINSPEC="$DEVNET_DIR/chainspec.json"
    LOG_FILE="$LOG_DIR/node${i}.log"

    # Ensure all 3 ports are free
    for port in $P2P_PORT $RPC_PORT $METRICS_PORT; do
        OCCUPANT=$(lsof -ti :$port 2>/dev/null | head -1)
        if [ -n "$OCCUPANT" ]; then
            warn "Port $port still occupied (PID $OCCUPANT), killing..."
            kill -9 "$OCCUPANT" 2>/dev/null
            sleep 1
        fi
    done

    # Remove stale RocksDB LOCK files (prevents SIGTRAP crash on macOS)
    rm -f "$DATA_DIR/blocks/LOCK" 2>/dev/null
    rm -f "$DATA_DIR/signed_slots.db/LOCK" 2>/dev/null

    BOOTSTRAP_FLAG=""
    if [ "$i" -ne 0 ]; then
        BOOTSTRAP_FLAG="--bootstrap /ip4/127.0.0.1/tcp/50300"
    fi

    "$NODE_BIN" --network devnet \
        --data-dir "$DATA_DIR" \
        run \
        --producer \
        --producer-key "$KEY_FILE" \
        --chainspec "$CHAINSPEC" \
        --p2p-port $P2P_PORT \
        --rpc-port $RPC_PORT \
        --metrics-port $METRICS_PORT \
        --no-auto-update \
        --force-start \
        --yes \
        $BOOTSTRAP_FLAG \
        >> "$LOG_FILE" 2>&1 &

    sleep 2
    # Get PID via port lookup (more reliable than $! in some shells)
    NEW_PID=$(lsof -ti :$P2P_PORT 2>/dev/null | head -1)
    if [ -n "$NEW_PID" ]; then
        echo "$NEW_PID" > "$DEVNET_DIR/pids/node${i}.pid"
        info "Restarted node $i (PID $NEW_PID) with fixed binary"
    else
        warn "Node $i started but PID not found on port $P2P_PORT"
    fi
done

# Give nodes time to initialize and sync
info "Waiting 15s for nodes to initialize..."
sleep 15

# Verify restarted nodes are responding
for i in "${KILL_NODES[@]}"; do
    h=$(get_height $i)
    if [ "$h" != "DOWN" ]; then
        pass "Node $i back online at height $h"
    else
        fail "Node $i not responding after restart"
    fi
done

# ===================================================================
# PHASE 6: Wait for sync convergence
# ===================================================================
echo
echo -e "${BOLD}${BLUE}PHASE 6: Wait for Sync Convergence (${SYNC_WAIT}s max)${NC}"
echo -e "${BLUE}-------------------------------------------${NC}"

CONVERGED=false
for attempt in $(seq 1 $((SYNC_WAIT / 10))); do
    sleep 10

    # Get heights for all nodes
    ALL_HEIGHTS=()
    ALL_HASHES=()
    ALL_OK=true
    for i in $(seq 0 $((NODE_COUNT - 1))); do
        h=$(get_height $i)
        hash=$(get_hash $i)
        ALL_HEIGHTS+=("$h")
        ALL_HASHES+=("$hash")
        if [ "$h" = "DOWN" ]; then
            ALL_OK=false
        fi
    done

    if [ "$ALL_OK" != "true" ]; then
        info "Attempt $attempt: some nodes still DOWN"
        continue
    fi

    # Check convergence: all heights within tolerance and same hash
    MAX_H=0
    MIN_H=999999
    for h in "${ALL_HEIGHTS[@]}"; do
        [ "$h" -gt "$MAX_H" ] && MAX_H=$h
        [ "$h" -lt "$MIN_H" ] && MIN_H=$h
    done
    DIFF=$((MAX_H - MIN_H))

    REF_HASH="${ALL_HASHES[0]}"
    HASH_MATCH=true
    for hash in "${ALL_HASHES[@]}"; do
        if [ "$hash" != "$REF_HASH" ]; then
            HASH_MATCH=false
            break
        fi
    done

    info "Attempt $attempt: heights=[${ALL_HEIGHTS[*]}] diff=$DIFF hash_match=$HASH_MATCH"

    if [ "$DIFF" -le "$HEIGHT_TOLERANCE" ] && [ "$HASH_MATCH" = "true" ]; then
        CONVERGED=true
        break
    fi
done

echo

# ===================================================================
# PHASE 7: Verify Results
# ===================================================================
echo -e "${BOLD}${BLUE}PHASE 7: Verify Results${NC}"
echo -e "${BLUE}-------------------------------------------${NC}"

# 7a. All nodes should be up and at similar heights
echo -e "${CYAN}7a. Node Status:${NC}"
for i in $(seq 0 $((NODE_COUNT - 1))); do
    eval "FINAL_HEIGHT_$i=$(get_height $i)"
    eval "FINAL_HASH_$i=$(get_hash $i)"
    eval "fh=\$FINAL_HEIGHT_$i; fhash=\$FINAL_HASH_$i"
    echo -e "  Node $i: height=$fh hash=${fhash:0:16}..."
done

# 7b. Convergence check
echo
echo -e "${CYAN}7b. Convergence:${NC}"
if [ "$CONVERGED" = "true" ]; then
    pass "All nodes converged within ${SYNC_WAIT}s"
else
    fail "Nodes did NOT converge within ${SYNC_WAIT}s"
fi

# 7c. Height double-counting check (THE CRITICAL TEST)
echo
echo -e "${CYAN}7c. Height Double-Counting Check (CRITICAL):${NC}"

for i in "${KILL_NODES[@]}"; do
    eval "RESTARTED_H=\$FINAL_HEIGHT_$i"
    REFERENCE_H=$FINAL_HEIGHT_0

    if [ "$RESTARTED_H" = "DOWN" ]; then
        fail "Node $i is DOWN, cannot verify"
        continue
    fi

    # The restarted node's height should be <= reference node's height + tolerance
    # If double-counted, height would be much larger (e.g., pre_kill + current = 2x)
    MAX_ACCEPTABLE=$((REFERENCE_H + HEIGHT_TOLERANCE))

    if [ "$RESTARTED_H" -le "$MAX_ACCEPTABLE" ]; then
        pass "Node $i height=$RESTARTED_H (reference=$REFERENCE_H, max_acceptable=$MAX_ACCEPTABLE)"
    else
        fail "Node $i height=$RESTARTED_H EXCEEDS reference=$REFERENCE_H by more than $HEIGHT_TOLERANCE (DOUBLE-COUNTED!)"
    fi

    # Extra check: restarted height should be >= pre-kill height (didn't go backwards)
    eval "pkh=\$PRE_KILL_HEIGHT_$i"
    if [ "$RESTARTED_H" -ge "$pkh" ]; then
        pass "Node $i height progressed: $pkh -> $RESTARTED_H"
    else
        fail "Node $i height went BACKWARDS: $pkh -> $RESTARTED_H"
    fi

    # Detect the BUG pattern: if height > 2x pre-kill, it's likely double-counting
    DOUBLE_COUNT_THRESHOLD=$(( pkh + REFERENCE_H ))
    if [ "$RESTARTED_H" -gt "$DOUBLE_COUNT_THRESHOLD" ]; then
        fail "Node $i height=$RESTARTED_H > $pkh + $REFERENCE_H = $DOUBLE_COUNT_THRESHOLD (CONFIRMED DOUBLE-COUNT BUG)"
    fi
done

# 7d. Hash consensus check
echo
echo -e "${CYAN}7d. Hash Consensus:${NC}"
ALL_MATCH=true
for i in $(seq 0 $((NODE_COUNT - 1))); do
    eval "fhash=\$FINAL_HASH_$i; fh=\$FINAL_HEIGHT_$i"
    if [ "$fhash" != "$FINAL_HASH_0" ] && [ "$fhash" != "DOWN" ]; then
        # Check if heights differ - different hash at different height is ok
        if [ "$fh" = "$FINAL_HEIGHT_0" ]; then
            fail "Node $i has different hash at SAME height (fork!)"
            ALL_MATCH=false
        else
            info "Node $i hash differs but at different height (height=$fh vs $FINAL_HEIGHT_0)"
        fi
    fi
done
if [ "$ALL_MATCH" = "true" ]; then
    pass "All nodes at same height share the same hash"
fi

# 7e. Log analysis - check for Fix A (SyncManager init) and Fix B (duplicate skip)
echo
echo -e "${CYAN}7e. Log Analysis:${NC}"

for i in "${KILL_NODES[@]}"; do
    LOG_FILE="$LOG_DIR/node${i}.log"
    if [ ! -f "$LOG_FILE" ]; then
        warn "No log file for node $i"
        continue
    fi

    # Only check post-restart log entries (after the marker)
    RESTART_LOG=$(sed -n '/RESTART MARKER/,$p' "$LOG_FILE" 2>/dev/null)
    if [ -z "$RESTART_LOG" ]; then
        RESTART_LOG=$(cat "$LOG_FILE" 2>/dev/null)
    fi

    # Check for Fix A: "Sync manager initialized at height X"
    SYNC_INIT=$(echo "$RESTART_LOG" | grep -c "Sync manager initialized at height" 2>/dev/null)
    SYNC_INIT=${SYNC_INIT:-0}
    if [ "$SYNC_INIT" -gt 0 ]; then
        INIT_LINE=$(echo "$RESTART_LOG" | grep "Sync manager initialized at height" | tail -1)
        pass "Node $i: Fix A active - $INIT_LINE"
    else
        warn "Node $i: No 'Sync manager initialized' log found (may be at height 0 or log level filtered)"
    fi

    # Check for Fix B: "already in store" warnings (should be 0 if Fix A works)
    DUP_COUNT=$(echo "$RESTART_LOG" | grep -c "already in store" 2>/dev/null)
    DUP_COUNT=${DUP_COUNT:-0}
    if [ "$DUP_COUNT" -eq 0 ]; then
        pass "Node $i: No duplicate block warnings (Fix A prevented re-download)"
    else
        warn "Node $i: $DUP_COUNT 'already in store' warnings (Fix B caught duplicates that slipped past Fix A)"
    fi
done

# ===================================================================
# SUMMARY
# ===================================================================
echo
echo -e "${BLUE}=========================================================${NC}"
echo -e "${BOLD}${BLUE}  RESULTS SUMMARY${NC}"
echo -e "${BLUE}=========================================================${NC}"
echo
echo -e "  ${GREEN}PASSED${NC}: $PASS"
echo -e "  ${RED}FAILED${NC}: $FAIL"
echo -e "  ${YELLOW}WARNINGS${NC}: $WARN"
echo
echo -e "  Pre-kill heights:  Node0=$PRE_KILL_HEIGHT_0 Node1=$PRE_KILL_HEIGHT_1 Node2=$PRE_KILL_HEIGHT_2 Node3=$PRE_KILL_HEIGHT_3 Node4=$PRE_KILL_HEIGHT_4"
echo -e "  Final heights:     Node0=$FINAL_HEIGHT_0 Node1=$FINAL_HEIGHT_1 Node2=$FINAL_HEIGHT_2 Node3=$FINAL_HEIGHT_3 Node4=$FINAL_HEIGHT_4"
echo

if [ "$FAIL" -eq 0 ]; then
    echo -e "  ${GREEN}${BOLD}ISSUE-5 FIX VERIFIED: No height double-counting on restart sync${NC}"
    echo
    exit 0
else
    echo -e "  ${RED}${BOLD}ISSUE-5 TEST FAILED: $FAIL failures detected${NC}"
    echo
    exit 1
fi

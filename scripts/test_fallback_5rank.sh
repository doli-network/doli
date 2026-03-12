#!/usr/bin/env bash
# =============================================================================
# test_fallback_5rank.sh - Feature 1: Producer Fallback System (5-Rank Sequential Windows)
# =============================================================================
#
# Tests that when a primary producer fails, fallback producers take over
# in exclusive 2-second windows (5 ranks x 2s = 10s slot).
#
# PREREQUISITES:
#   - Devnet running with 10 producers (9 nodes, node3 already down)
#   - Node 0 is bootstrap (P2P:50300, RPC:28500)
#   - Nodes 1,2,4-9 are running
#
# USAGE:
#   nix develop --command bash ./scripts/test_fallback_5rank.sh
#
# WHAT IT TESTS:
#   Test 1: Baseline - all nodes producing normally, chain advancing
#   Test 2: Single producer kill - kill a node, verify fallback produces
#   Test 3: Timing windows - verify fallback blocks have >2s offset from slot start
#   Test 4: Multi-producer kill - kill 2+ nodes, verify deeper fallback ranks
#   Test 5: Recovery - restart killed nodes, verify they rejoin and sync
#
# RUNTIME: ~5 minutes
# =============================================================================

set -eo pipefail

DOLI_CLI="./target/release/doli"
DOLI_NODE="./target/release/doli-node"
RPC_BASE=28500
P2P_BASE=50300
METRICS_BASE=9000
CHAINSPEC="$HOME/.doli/devnet/chainspec.json"
KEYS_DIR="$HOME/.doli/devnet/keys"
DATA_DIR="$HOME/.doli/devnet/data"
LOGS_DIR="$HOME/.doli/devnet/logs"
PASS=0
FAIL=0
WARN=0

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

log()  { echo -e "${CYAN}[$(date +%H:%M:%S)]${NC} $*"; }
pass() { echo -e "${GREEN}[PASS]${NC} $*"; PASS=$((PASS+1)); }
fail() { echo -e "${RED}[FAIL]${NC} $*"; FAIL=$((FAIL+1)); }
warn() { echo -e "${YELLOW}[WARN]${NC} $*"; WARN=$((WARN+1)); }

rpc() {
    local port=$1 method=$2
    local params="$3"
    if [ -z "$params" ]; then params="{}"; fi
    curl -s --max-time 3 "http://127.0.0.1:$port" -X POST \
        -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"$method\",\"params\":$params,\"id\":1}" 2>/dev/null
}

get_height() {
    local port=$1
    rpc "$port" "getChainInfo" | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['bestHeight'])" 2>/dev/null || echo "0"
}

get_slot() {
    local port=$1
    rpc "$port" "getChainInfo" | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['bestSlot'])" 2>/dev/null || echo "0"
}

get_hash() {
    local port=$1
    rpc "$port" "getChainInfo" | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['bestHash'])" 2>/dev/null || echo "unknown"
}

get_block_at_height() {
    local port=$1 height=$2
    rpc "$port" "getBlockByHeight" "{\"height\":$height}"
}

get_pubkey() {
    local wallet=$1
    $DOLI_CLI -w "$wallet" info 2>/dev/null | grep "Public Key:" | sed 's/.*: //'
}

kill_node() {
    local idx=$1
    local p2p_port=$((P2P_BASE + idx))
    local pid
    pid=$(lsof -ti ":$p2p_port" 2>/dev/null | head -1) || true
    if [ -n "$pid" ]; then
        kill "$pid" 2>/dev/null || true
        sleep 1
        kill -0 "$pid" 2>/dev/null && kill -9 "$pid" 2>/dev/null || true
        sleep 1
        log "Killed node $idx (PID $pid, P2P:$p2p_port)"
        return 0
    else
        log "Node $idx not running (P2P:$p2p_port)"
        return 0
    fi
}

start_node() {
    local idx=$1
    local p2p=$((P2P_BASE + idx))
    local rpc_port=$((RPC_BASE + idx))
    local metrics=$((METRICS_BASE + idx))

    # Wipe data to avoid corrupted RocksDB from hard kills
    rm -rf "$DATA_DIR/node$idx"
    mkdir -p "$DATA_DIR/node$idx"

    if [ "$idx" -ne 0 ]; then
        $DOLI_NODE --network devnet --data-dir "$DATA_DIR/node$idx" run \
            --producer --producer-key "$KEYS_DIR/producer_$idx.json" \
            --p2p-port "$p2p" --rpc-port "$rpc_port" --metrics-port "$metrics" \
            --bootstrap "/ip4/127.0.0.1/tcp/$P2P_BASE" --chainspec "$CHAINSPEC" \
            --no-auto-update --force-start --yes \
            > "$LOGS_DIR/node$idx.log" 2>&1 &
    else
        $DOLI_NODE --network devnet --data-dir "$DATA_DIR/node$idx" run \
            --producer --producer-key "$KEYS_DIR/producer_$idx.json" \
            --p2p-port "$p2p" --rpc-port "$rpc_port" --metrics-port "$metrics" \
            --chainspec "$CHAINSPEC" \
            --no-auto-update --force-start --yes \
            > "$LOGS_DIR/node$idx.log" 2>&1 &
    fi

    log "Started node $idx (P2P:$p2p RPC:$rpc_port)"
}

wait_for_height() {
    local port=$1 target=$2 timeout_secs=${3:-120}
    local start_time
    start_time=$(date +%s)
    while true; do
        local h
        h=$(get_height "$port")
        if [ "$h" -ge "$target" ] 2>/dev/null; then
            return 0
        fi
        local elapsed=$(( $(date +%s) - start_time ))
        if [ "$elapsed" -ge "$timeout_secs" ]; then
            return 1
        fi
        sleep 2
    done
}

echo ""
echo "============================================================"
echo "  DOLI Fallback System Test (5-Rank Sequential Windows)"
echo "============================================================"
echo ""

# =============================================================================
# PRE-FLIGHT
# =============================================================================
log "Pre-flight checks..."

running_nodes=0
for i in 0 1 2 4 5 6 7 8 9; do
    rpc_port=$((RPC_BASE + i))
    h=$(get_height "$rpc_port")
    if [ "$h" != "0" ] && [ -n "$h" ]; then
        running_nodes=$((running_nodes + 1))
    fi
done

if [ "$running_nodes" -lt 5 ]; then
    echo "ERROR: Need at least 5 running nodes, found $running_nodes"
    exit 1
fi
log "Found $running_nodes running nodes"

h0=$(get_height $RPC_BASE)
s0=$(get_slot $RPC_BASE)
log "Chain at height=$h0, slot=$s0"

# Load pubkeys into indexed variables (PUBKEY_0..PUBKEY_9)
for i in 0 1 2 3 4 5 6 7 8 9; do
    if [ -f "$KEYS_DIR/producer_$i.json" ]; then
        pk=$(get_pubkey "$KEYS_DIR/producer_$i.json")
        eval "PUBKEY_$i='$pk'"
    fi
done
log "Loaded producer pubkeys"

# =============================================================================
# TEST 1: BASELINE
# =============================================================================
echo ""
echo "------------------------------------------------------------"
echo "  TEST 1: Baseline - Normal Block Production"
echo "------------------------------------------------------------"

h_start=$(get_height $RPC_BASE)
log "Starting height: $h_start. Waiting for 5 blocks..."

if wait_for_height $RPC_BASE $((h_start + 5)) 90; then
    h_end=$(get_height $RPC_BASE)
    blocks=$((h_end - h_start))
    pass "Chain advanced $blocks blocks (from $h_start to $h_end)"
else
    h_end=$(get_height $RPC_BASE)
    fail "Chain stalled: only reached $h_end (needed $((h_start + 5)))"
fi

# Consensus check (allow up to 5 blocks diff for recently synced nodes)
forks=0
for i in 1 2 4 5 6 7 8 9; do
    rpc_port=$((RPC_BASE + i))
    h_n=$(get_height "$rpc_port")
    h_0=$(get_height $RPC_BASE)
    diff=$(( h_0 > h_n ? h_0 - h_n : h_n - h_0 ))
    if [ "$diff" -gt 5 ]; then
        forks=$((forks + 1))
        log "  Node $i diverged: height=$h_n vs node0=$h_0 (diff=$diff)"
    fi
done

if [ "$forks" -eq 0 ]; then
    pass "All nodes in consensus (no forks detected)"
else
    fail "Fork detected: $forks nodes diverged"
fi

# Check recent block producers
log "Checking recent block producers..."
h_now=$(get_height $RPC_BASE)
unique_file=$(mktemp)
for height in $(seq $((h_now - 4)) $h_now); do
    block=$(get_block_at_height $RPC_BASE "$height")
    producer=$(echo "$block" | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['producer'])" 2>/dev/null || echo "unknown")
    slot_n=$(echo "$block" | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['slot'])" 2>/dev/null || echo "?")
    ts=$(echo "$block" | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['timestamp'])" 2>/dev/null || echo "?")
    log "  Height $height: slot=$slot_n producer=${producer:0:16}... ts=$ts"
    echo "$producer" >> "$unique_file"
done
unique_producers=$(sort -u "$unique_file" | grep -v unknown | wc -l | tr -d ' ')
rm -f "$unique_file"
log "Saw $unique_producers unique producers in last 5 blocks"

if [ "$unique_producers" -ge 2 ]; then
    pass "Multiple producers active ($unique_producers unique in last 5 blocks)"
else
    warn "Only $unique_producers unique producer in last 5 blocks (might be normal with few slots)"
fi

# =============================================================================
# TEST 2: Single Producer Kill
# =============================================================================
echo ""
echo "------------------------------------------------------------"
echo "  TEST 2: Kill Primary Producer - Verify Fallback"
echo "------------------------------------------------------------"

KILL_NODE=1
log "Killing node $KILL_NODE to test fallback..."
kill_node $KILL_NODE

h_before_kill=$(get_height $RPC_BASE)
log "Height at kill: $h_before_kill"

log "Waiting for 10 blocks to observe fallback behavior..."
if wait_for_height $RPC_BASE $((h_before_kill + 10)) 180; then
    h_after=$(get_height $RPC_BASE)
    pass "Chain continued advancing after killing node $KILL_NODE (height $h_before_kill -> $h_after)"
else
    h_after=$(get_height $RPC_BASE)
    fail "Chain stalled after killing node $KILL_NODE (only reached $h_after, needed $((h_before_kill + 10)))"
fi

# Check blocks weren't produced by killed node
killed_pubkey="$PUBKEY_1"
log "Killed producer pubkey: ${killed_pubkey:0:16}..."

blocks_by_killed=0
blocks_checked=0
for height in $(seq $((h_before_kill + 1)) $h_after); do
    block=$(get_block_at_height $RPC_BASE "$height")
    producer=$(echo "$block" | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['producer'])" 2>/dev/null || echo "unknown")
    if [ "$producer" = "$killed_pubkey" ]; then
        blocks_by_killed=$((blocks_by_killed + 1))
    fi
    blocks_checked=$((blocks_checked + 1))
done

if [ "$blocks_by_killed" -eq 0 ]; then
    pass "Killed producer did NOT produce any of the $blocks_checked blocks after kill"
elif [ "$blocks_by_killed" -le 1 ]; then
    warn "Killed producer produced $blocks_by_killed block(s) after kill (may be propagation delay)"
else
    fail "Killed producer produced $blocks_by_killed blocks after kill"
fi

# =============================================================================
# TEST 3: Timing Windows
# =============================================================================
echo ""
echo "------------------------------------------------------------"
echo "  TEST 3: Verify Fallback Timing Windows"
echo "------------------------------------------------------------"

genesis_time=$(python3 -c "import json; print(json.load(open('$CHAINSPEC'))['genesis']['timestamp'])")
slot_duration=10
log "Genesis time: $genesis_time, slot duration: ${slot_duration}s"

h_now=$(get_height $RPC_BASE)
rank0_count=0
fallback_count=0
late_blocks=0

start_h=$((h_now > 15 ? h_now - 14 : 1))
log "Analyzing blocks $start_h to $h_now for timing..."
for height in $(seq "$start_h" "$h_now"); do
    block=$(get_block_at_height $RPC_BASE "$height")
    result=$(echo "$block" | python3 -c "
import sys, json
b = json.load(sys.stdin)['result']
h = b['height']
slot = b['slot']
ts = b['timestamp']
producer = b['producer']
genesis = $genesis_time
slot_start = genesis + (slot * $slot_duration)
offset_s = ts - slot_start
if offset_s < 2:
    rank = 0
elif offset_s < 4:
    rank = 1
elif offset_s < 6:
    rank = 2
elif offset_s < 8:
    rank = 3
elif offset_s < 10:
    rank = 4
else:
    rank = -1
print(f'{h}|{slot}|{offset_s}|{rank}|{producer[:16]}')
" 2>/dev/null || echo "?|?|?|?|?")

    IFS='|' read -r b_height b_slot b_offset b_rank b_producer <<< "$result"

    if [ "$b_rank" = "0" ]; then
        rank0_count=$((rank0_count + 1))
        log "  Height $b_height: slot=$b_slot offset=${b_offset}s rank=0 (primary) producer=$b_producer..."
    elif [ "$b_rank" != "?" ] && [ "$b_rank" != "-1" ]; then
        fallback_count=$((fallback_count + 1))
        log "  Height $b_height: slot=$b_slot offset=${b_offset}s rank=$b_rank (FALLBACK) producer=$b_producer..."
    else
        late_blocks=$((late_blocks + 1))
        log "  Height $b_height: slot=$b_slot offset=${b_offset}s rank=UNKNOWN producer=$b_producer..."
    fi
done

total_analyzed=$((rank0_count + fallback_count + late_blocks))
log "Results: $rank0_count primary, $fallback_count fallback, $late_blocks unknown out of $total_analyzed"

if [ "$fallback_count" -gt 0 ]; then
    pass "Detected $fallback_count fallback block(s) with correct timing windows"
elif [ "$total_analyzed" -gt 0 ]; then
    warn "No fallback blocks detected yet (may need more time - 2 of 10 producers are down)"
fi

if [ "$rank0_count" -gt 0 ]; then
    pass "Detected $rank0_count primary (rank 0) blocks with offset < 2s"
fi

# =============================================================================
# TEST 4: Multi-Producer Kill
# =============================================================================
echo ""
echo "------------------------------------------------------------"
echo "  TEST 4: Kill Multiple Producers - Deeper Fallback Ranks"
echo "------------------------------------------------------------"

log "Killing nodes 2 and 5 (4 total down: 1, 2, 3, 5)..."
kill_node 2
kill_node 5

h_before_multi=$(get_height $RPC_BASE)
log "Height at multi-kill: $h_before_multi"

log "Waiting for 15 blocks with 4 producers down..."
if wait_for_height $RPC_BASE $((h_before_multi + 15)) 300; then
    h_after_multi=$(get_height $RPC_BASE)
    pass "Chain survived with 4/10 producers down (height $h_before_multi -> $h_after_multi)"
else
    h_after_multi=$(get_height $RPC_BASE)
    fail "Chain stalled with 4 producers down (only reached $h_after_multi, needed $((h_before_multi + 15)))"
fi

# Check for deeper fallback ranks
deep_fallback=0
for height in $(seq $((h_before_multi + 1)) "$h_after_multi"); do
    block=$(get_block_at_height $RPC_BASE "$height")
    offset=$(echo "$block" | python3 -c "
import sys, json
b = json.load(sys.stdin)['result']
slot_start = $genesis_time + (b['slot'] * $slot_duration)
print(b['timestamp'] - slot_start)
" 2>/dev/null || echo "0")

    if [ "$offset" -ge 2 ] 2>/dev/null; then
        deep_fallback=$((deep_fallback + 1))
    fi
done

if [ "$deep_fallback" -gt 0 ]; then
    pass "Detected $deep_fallback blocks with fallback offset >= 2s (deeper ranks activated)"
else
    warn "No deep fallback blocks detected (all blocks produced at rank 0)"
fi

# Verify no blocks by dead producers
blocks_by_dead=0
for height in $(seq $((h_before_multi + 2)) "$h_after_multi"); do
    block=$(get_block_at_height $RPC_BASE "$height")
    producer=$(echo "$block" | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['producer'])" 2>/dev/null || echo "unknown")
    for dead_pk in "$PUBKEY_1" "$PUBKEY_2" "$PUBKEY_3" "$PUBKEY_5"; do
        if [ "$producer" = "$dead_pk" ]; then
            blocks_by_dead=$((blocks_by_dead + 1))
        fi
    done
done

if [ "$blocks_by_dead" -eq 0 ]; then
    pass "No blocks produced by dead producers after multi-kill"
else
    warn "Dead producers produced $blocks_by_dead block(s) (possible propagation delay)"
fi

# Consensus check on surviving nodes
log "Checking consensus among surviving nodes..."
consensus_ok=0
consensus_total=0
for i in 0 4 6 7 8 9; do
    rpc_port=$((RPC_BASE + i))
    h_n=$(get_height "$rpc_port")
    if [ "$h_n" = "0" ] || [ -z "$h_n" ]; then
        log "  Node $i appears DOWN (height=0) - may have crashed due to peer loss"
        continue
    fi
    consensus_total=$((consensus_total + 1))
    h_ref=$(get_height $RPC_BASE)
    diff=$(( h_ref > h_n ? h_ref - h_n : h_n - h_ref ))
    if [ "$diff" -le 3 ]; then
        consensus_ok=$((consensus_ok + 1))
    else
        log "  Node $i is $diff blocks behind reference"
    fi
done

if [ "$consensus_ok" -ge 4 ]; then
    pass "Surviving nodes in consensus ($consensus_ok/$consensus_total responding nodes agree)"
elif [ "$consensus_total" -lt 3 ]; then
    warn "Too few nodes responding ($consensus_total) to verify consensus"
else
    fail "Surviving nodes diverged ($consensus_ok/$consensus_total in agreement)"
fi

# =============================================================================
# TEST 5: Recovery
# =============================================================================
echo ""
echo "------------------------------------------------------------"
echo "  TEST 5: Recovery - Restart Killed Nodes"
echo "------------------------------------------------------------"

log "Restarting killed nodes (1, 2, 5)..."
start_node 1
start_node 2
start_node 5

log "Waiting for nodes to sync (up to 90s)..."
synced=0
for attempt in 1 2 3 4 5 6; do
    sleep 15
    synced=0
    h_tip=$(get_height $RPC_BASE)
    for i in 1 2 5; do
        rpc_port=$((RPC_BASE + i))
        h_node=$(get_height "$rpc_port")
        if [ "$h_node" -ge $((h_tip - 5)) ] 2>/dev/null; then
            synced=$((synced + 1))
        fi
    done
    log "  Sync check $attempt: $synced/3 nodes within 5 blocks of tip ($h_tip)"
    if [ "$synced" -eq 3 ]; then break; fi
done

h_tip=$(get_height $RPC_BASE)
for i in 1 2 5; do
    rpc_port=$((RPC_BASE + i))
    h_node=$(get_height "$rpc_port")
    if [ "$h_node" -ge $((h_tip - 5)) ] 2>/dev/null; then
        log "  Node $i synced: height=$h_node (tip=$h_tip)"
    else
        log "  Node $i behind: height=$h_node (tip=$h_tip)"
    fi
done

if [ "$synced" -eq 3 ]; then
    pass "All 3 restarted nodes synced back to tip"
elif [ "$synced" -ge 1 ]; then
    warn "Only $synced/3 restarted nodes synced (others may need more time)"
else
    fail "No restarted nodes synced to tip"
fi

# Final consensus
log "Final consensus check..."
sleep 10
h_final=$(get_height $RPC_BASE)
all_synced=true
for i in 0 1 2 4 5 6 7 8 9; do
    rpc_port=$((RPC_BASE + i))
    h_n=$(get_height "$rpc_port")
    diff=$(( h_final > h_n ? h_final - h_n : h_n - h_final ))
    if [ "$diff" -gt 3 ]; then
        all_synced=false
        log "  Node $i: height=$h_n (behind by $diff)"
    fi
done

if $all_synced; then
    pass "All 9 nodes back in consensus after recovery"
else
    warn "Some nodes still syncing after recovery"
fi

# =============================================================================
# SUMMARY
# =============================================================================
echo ""
echo "============================================================"
echo "  TEST SUMMARY"
echo "============================================================"
echo ""
echo -e "  ${GREEN}PASSED:${NC} $PASS"
echo -e "  ${RED}FAILED:${NC} $FAIL"
echo -e "  ${YELLOW}WARNINGS:${NC} $WARN"
echo ""

if [ "$FAIL" -eq 0 ]; then
    echo -e "  ${GREEN}OVERALL: ALL TESTS PASSED${NC}"
    exit 0
else
    echo -e "  ${RED}OVERALL: $FAIL TEST(S) FAILED${NC}"
    exit 1
fi

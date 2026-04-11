#!/usr/bin/env bash
#
# DOLI Binary Certification Test
# Usage: ./scripts/cert-test.sh [--binary /path/to/doli-node] [--no-swap]
#
# Runs a ~10 minute rolling-upgrade scenario against the live mainnet
# (ai1/ai2/ai3) and validates that the chain survives each restart sequence.
#
# Phases:
#   1. Baseline       - chain healthy, capture tip
#   2. Stage binary   - scp to /tmp on all 3 servers (skipped if --no-swap)
#   3. Upgrade ai1    - restart 4 services (seed + n1/n2/n3)
#   4. Upgrade ai2    - restart 3 services (seed + n4/n5)
#   5. Upgrade ai3    - restart 1 service  (seed)
#   6. Stability      - 60s observation window
#   7. Log audit      - grep for critical errors
#   8. Final          - unified hash + version check
#
# PASS/FAIL per phase. Final exit 0 = certified, 1 = rejected.

set -u

# ============================================================================
# CONFIG
# ============================================================================
SERVERS=(ai1 ai2 ai3)

services_for() {
    case "$1" in
        ai1) echo "doli-mainnet-seed doli-mainnet-n1 doli-mainnet-n2 doli-mainnet-n3" ;;
        ai2) echo "doli-mainnet-seed doli-mainnet-n4 doli-mainnet-n5" ;;
        ai3) echo "doli-mainnet-seed" ;;
        *) echo ""; return 1 ;;
    esac
}

# (server:port:label) — order matters for snapshot display
NODES=(
    "ai1:8500:s1"
    "ai1:8501:n1"
    "ai1:8502:n2"
    "ai1:8503:n3"
    "ai2:8500:s2"
    "ai2:8504:n4"
    "ai2:8505:n5"
    "ai3:8500:s3"
)

BINARY_PATH=""
NO_SWAP=false
RESULTS=()
FAIL_COUNT=0

# ============================================================================
# ARG PARSING
# ============================================================================
while [[ $# -gt 0 ]]; do
    case $1 in
        --binary) BINARY_PATH="$2"; shift 2 ;;
        --no-swap) NO_SWAP=true; shift ;;
        -h|--help) grep '^#' "$0" | sed 's/^# \?//' ; exit 0 ;;
        *) echo "Unknown arg: $1"; exit 2 ;;
    esac
done

if ! $NO_SWAP && [[ -z "$BINARY_PATH" ]]; then
    echo "ERROR: must provide --binary PATH or --no-swap"
    exit 2
fi

if ! $NO_SWAP && [[ ! -f "$BINARY_PATH" ]]; then
    echo "ERROR: binary not found: $BINARY_PATH"
    exit 2
fi

# ============================================================================
# HELPERS
# ============================================================================
rpc_chain_info() {
    local host=$1 port=$2
    ssh "$host" "curl -s -m3 -X POST http://127.0.0.1:$port/ \
        -H 'Content-Type: application/json' \
        -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":[],\"id\":1}'" 2>/dev/null
}

get_height() {
    rpc_chain_info "$1" "$2" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d["result"]["bestHeight"])' 2>/dev/null
}

get_hash() {
    rpc_chain_info "$1" "$2" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d["result"]["bestHash"][:16])' 2>/dev/null
}

get_version() {
    rpc_chain_info "$1" "$2" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d["result"]["version"])' 2>/dev/null
}

snapshot() {
    local label=$1
    echo "--- snapshot: $label ---"
    for entry in "${NODES[@]}"; do
        IFS=: read -r host port name <<< "$entry"
        local info
        info=$(rpc_chain_info "$host" "$port" | python3 -c 'import sys,json;r=json.load(sys.stdin).get("result");print("h="+str(r["bestHeight"]),"s="+str(r["bestSlot"]),r["bestHash"][:16],"v"+r["version"]) if r else print("?")' 2>/dev/null || echo "?")
        printf "  %-14s %s\n" "$host:$name" "$info"
    done
}

# verify that a set of nodes are on unified chain
# args: tolerance_height (max height diff allowed) -- hash must always match for heights that overlap
verify_unified() {
    local max_gap=${1:-2}
    local heights=()
    local top_hashes=()
    local unified=true
    local reason=""

    for entry in "${NODES[@]}"; do
        IFS=: read -r host port name <<< "$entry"
        local h hash
        h=$(get_height "$host" "$port" 2>/dev/null)
        hash=$(get_hash "$host" "$port" 2>/dev/null)
        if [[ -z "$h" || -z "$hash" ]]; then
            unified=false
            reason="$host:$name unreachable"
            break
        fi
        heights+=("$h")
        top_hashes+=("$host:$name:$h:$hash")
    done

    if ! $unified; then
        echo "UNIFIED_CHECK FAIL: $reason"
        return 1
    fi

    # check max/min height gap
    local max_h min_h
    max_h=$(printf '%s\n' "${heights[@]}" | sort -n | tail -1)
    min_h=$(printf '%s\n' "${heights[@]}" | sort -n | head -1)
    local gap=$((max_h - min_h))
    if [[ $gap -gt $max_gap ]]; then
        echo "UNIFIED_CHECK FAIL: height gap $gap > $max_gap (min=$min_h max=$max_h)"
        return 1
    fi

    # verify all nodes at min_h have the same hash (compare at common height)
    local ref_hash=""
    for entry in "${NODES[@]}"; do
        IFS=: read -r host port name <<< "$entry"
        local h_at_min
        h_at_min=$(ssh "$host" "curl -s -m3 -X POST http://127.0.0.1:$port/ -H 'Content-Type: application/json' -d '{\"jsonrpc\":\"2.0\",\"method\":\"getBlockByHeight\",\"params\":[$min_h],\"id\":1}'" 2>/dev/null | python3 -c 'import sys,json;d=json.load(sys.stdin);print(d["result"]["hash"][:16] if "result" in d else "ERR")' 2>/dev/null)
        if [[ -z "$ref_hash" ]]; then
            ref_hash=$h_at_min
        elif [[ "$h_at_min" != "$ref_hash" ]]; then
            echo "UNIFIED_CHECK FAIL: $host:$name hash at h=$min_h = $h_at_min != $ref_hash"
            return 1
        fi
    done

    echo "  unified: all 8 nodes, min_h=$min_h max_h=$max_h gap=$gap hash@$min_h=$ref_hash ✓"
    return 0
}

wait_for_advance() {
    local expected_advance=$1 max_wait=$2
    local start_h
    start_h=$(get_height "ai1" "8501")
    local deadline=$((SECONDS + max_wait))
    while [[ $SECONDS -lt $deadline ]]; do
        sleep 5
        local now_h
        now_h=$(get_height "ai1" "8501" 2>/dev/null || echo 0)
        if [[ $((now_h - start_h)) -ge $expected_advance ]]; then
            echo "  advanced from h=$start_h to h=$now_h ✓"
            return 0
        fi
    done
    echo "  FAIL: chain did not advance $expected_advance blocks in ${max_wait}s (still h=$(get_height ai1 8501))"
    return 1
}

record() {
    local phase=$1 status=$2 detail=$3
    RESULTS+=("$phase|$status|$detail")
    if [[ "$status" != "PASS" ]]; then
        FAIL_COUNT=$((FAIL_COUNT + 1))
    fi
    printf "  [%s] %s — %s\n" "$status" "$phase" "$detail"
}

# ============================================================================
# PHASE 1: BASELINE
# ============================================================================
echo "==================================================================="
echo "DOLI Binary Certification Test"
echo "Started: $(date -u '+%Y-%m-%dT%H:%M:%SZ')"
echo "Binary:  ${BINARY_PATH:-<no-swap>}"
echo "==================================================================="
echo
echo "### PHASE 1: Baseline ###"

snapshot "baseline"
if verify_unified 2; then
    baseline_h=$(get_height "ai1" "8501")
    record "baseline" "PASS" "chain healthy at h=$baseline_h"
else
    record "baseline" "FAIL" "chain not unified at start — abort"
    echo
    echo "Cannot run cert test on unhealthy chain. Exit."
    exit 1
fi

# ============================================================================
# PHASE 2: STAGE BINARY
# ============================================================================
echo
echo "### PHASE 2: Stage binary ###"

if $NO_SWAP; then
    record "stage" "SKIP" "--no-swap, testing restart only"
else
    EXPECTED_MD5=$(md5 -q "$BINARY_PATH" 2>/dev/null || md5sum "$BINARY_PATH" | awk '{print $1}')
    echo "  expected md5: $EXPECTED_MD5"
    for s in "${SERVERS[@]}"; do
        scp -q "$BINARY_PATH" "$s:/tmp/doli-node-cert" &
    done
    wait
    all_match=true
    for s in "${SERVERS[@]}"; do
        actual=$(ssh "$s" "md5sum /tmp/doli-node-cert 2>/dev/null | awk '{print \$1}'")
        if [[ "$actual" != "$EXPECTED_MD5" ]]; then
            echo "  FAIL: $s md5=$actual != $EXPECTED_MD5"
            all_match=false
        fi
    done
    if $all_match; then
        record "stage" "PASS" "md5 matches on all 3 servers"
    else
        record "stage" "FAIL" "md5 mismatch"
        exit 1
    fi
fi

# ============================================================================
# PHASE 3-5: ROLLING UPGRADE (ai1 → ai2 → ai3)
# ============================================================================
upgrade_server() {
    local server=$1 phase_num=$2
    local services
    services=$(services_for "$server")
    echo
    echo "### PHASE $phase_num: Upgrade $server ($services) ###"

    local pre_h
    pre_h=$(get_height "ai1" "8501")
    echo "  pre-upgrade tip h=$pre_h"

    if $NO_SWAP; then
        echo "  restart only (no binary swap)"
        ssh "$server" "sudo systemctl restart $services" 2>&1 | tail -3
    else
        ssh "$server" "sudo systemctl stop $services && sudo cp /tmp/doli-node-cert /mainnet/bin/doli-node && sudo chmod +x /mainnet/bin/doli-node && sudo systemctl start $services" 2>&1 | tail -3
    fi

    echo "  waiting 10s for services to boot..."
    sleep 10
    echo "  waiting for chain advancement..."
    if wait_for_advance 3 60; then
        sleep 5
        if verify_unified 3; then
            record "upgrade-$server" "PASS" "services restarted, chain advanced, hash unified"
        else
            record "upgrade-$server" "FAIL" "chain not unified after restart"
            snapshot "$server-failure"
            return 1
        fi
    else
        record "upgrade-$server" "FAIL" "chain failed to advance after $server restart"
        snapshot "$server-failure"
        return 1
    fi
}

upgrade_server "ai1" "3" || { FAIL_COUNT=$((FAIL_COUNT + 1)); }
upgrade_server "ai2" "4" || { FAIL_COUNT=$((FAIL_COUNT + 1)); }
upgrade_server "ai3" "5" || { FAIL_COUNT=$((FAIL_COUNT + 1)); }

# ============================================================================
# PHASE 6: STABILITY WINDOW
# ============================================================================
echo
echo "### PHASE 6: Stability window (60s) ###"
stable_start_h=$(get_height "ai1" "8501")
echo "  h_start=$stable_start_h, waiting 60s..."
sleep 60
stable_end_h=$(get_height "ai1" "8501")
stable_advance=$((stable_end_h - stable_start_h))
echo "  h_end=$stable_end_h, advanced $stable_advance blocks"

if [[ $stable_advance -ge 4 ]]; then
    if verify_unified 3; then
        record "stability" "PASS" "advanced $stable_advance blocks in 60s, unified"
    else
        record "stability" "FAIL" "chain not unified during stability window"
    fi
else
    record "stability" "FAIL" "chain advanced only $stable_advance blocks in 60s (expected ≥4)"
fi

# ============================================================================
# PHASE 7: LOG AUDIT
# ============================================================================
echo
echo "### PHASE 7: Log audit ###"
PATTERNS='ERRTX070|BLOCK_POISON|BlockedConflictsFinality|DEEP_FORK|PROCESSING_STALL'
total_errors=0
for s in "${SERVERS[@]}"; do
    c=$(ssh "$s" "sudo grep -cE '$PATTERNS' /var/log/doli/mainnet/*.log 2>/dev/null | awk -F: '{s+=\$2} END {print s+0}'")
    c=${c:-0}
    total_errors=$((total_errors + c))
    echo "  $s: $c critical error lines"
done

if [[ $total_errors -eq 0 ]]; then
    record "log-audit" "PASS" "0 critical errors across all servers"
else
    record "log-audit" "FAIL" "$total_errors critical error lines detected"
fi

# ============================================================================
# PHASE 8: FINAL STATE
# ============================================================================
echo
echo "### PHASE 8: Final state ###"
snapshot "final"

# check version unified across all nodes
versions=()
for entry in "${NODES[@]}"; do
    IFS=: read -r host port name <<< "$entry"
    v=$(get_version "$host" "$port" 2>/dev/null || echo "?")
    versions+=("$v")
done
unique_versions=$(printf '%s\n' "${versions[@]}" | sort -u | tr '\n' ' ')
if [[ $(printf '%s\n' "${versions[@]}" | sort -u | wc -l) -eq 1 ]]; then
    record "version-unified" "PASS" "all nodes on $unique_versions"
else
    record "version-unified" "FAIL" "versions diverge: $unique_versions"
fi

# ============================================================================
# FINAL REPORT
# ============================================================================
echo
echo "==================================================================="
echo "CERTIFICATION REPORT"
echo "==================================================================="
for r in "${RESULTS[@]}"; do
    IFS='|' read -r phase status detail <<< "$r"
    printf "  %-20s %-6s %s\n" "$phase" "$status" "$detail"
done
echo
if [[ $FAIL_COUNT -eq 0 ]]; then
    echo "CERTIFICATION: PASS ✓"
    echo "Safe to deploy."
    exit 0
else
    echo "CERTIFICATION: FAIL ✗"
    echo "$FAIL_COUNT phase(s) failed. Do NOT deploy."
    exit 1
fi

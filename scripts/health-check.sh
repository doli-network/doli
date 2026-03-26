#!/usr/bin/env bash
# health-check.sh — Verify network health after reset or incident
#
# Usage: scripts/health-check.sh [mainnet|testnet|all]
#
# Architecture v9 (2026-03-23):
#   ai1 = mainnet seed+N1-N3 + testnet seed+NT1-NT5
#   ai2 = mainnet seed+N4-N5 + testnet seed + build + explorer
#   ai3 = seeds (both networks) + named producers (SANTIAGO, IVAN), ai4 = mainnet N6-N8, ai5 = mainnet N9-N12 + testnet NT6-NT12
#
# Checks (all must pass):
#   1. All nodes responding to RPC
#   2. All nodes have >= 1 peer
#   3. All nodes share the same genesis hash
#   4. Block heights within acceptable range
#   5. No "Unexpected peer ID" errors in recent logs
#   6. Service files have --bootstrap flag
#   7. Service files have LimitNOFILE=65535
set -euo pipefail

AI1="${DOLI_AI1:?Set DOLI_AI1=user@host}"    # Mainnet seed+N1-N3 + Testnet seed+NT1-NT5
AI2="${DOLI_AI2:?Set DOLI_AI2=user@host}"   # Mainnet seed+N4-N5 + Testnet seed + build + explorer
AI3="${DOLI_AI3:?Set DOLI_AI3=user@host}"   # Seeds (both networks) + named producers (SANTIAGO, IVAN)
AI4="${DOLI_AI4:?Set DOLI_AI4=user@host}"  # Mainnet N6-N8
AI5="${DOLI_AI5:?Set DOLI_AI5=user@host}"    # Mainnet N9-N12 + Testnet NT6-NT12

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

ERRORS=0
WARNINGS=0

pass()  { echo -e "  ${GREEN}PASS${NC} $1"; }
fail()  { echo -e "  ${RED}FAIL${NC} $1"; ERRORS=$((ERRORS+1)); }
warn()  { echo -e "  ${YELLOW}WARN${NC} $1"; WARNINGS=$((WARNINGS+1)); }

# SSH wrapper
do_ssh() {
  local server="$1"; shift
  ssh -p "${DOLI_SSH_PORT:-22}" -o ConnectTimeout=5 "$server" "$@"
}

rpc_call() {
  local server="$1" port="$2" method="$3"
  do_ssh "$server" "curl -sf --max-time 5 -X POST http://127.0.0.1:${port} \
    -H 'Content-Type: application/json' \
    -d '{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":{},\"id\":1}'" 2>/dev/null
}

check_network() {
  local net_label="$1" net_name="$2"
  shift 2
  local nodes=("$@")  # "name:server:rpc_port:log_path:service_name" entries

  echo "=== ${net_label} Health Check ==="
  echo ""

  local ref_genesis="" max_height=0
  local all_heights=()

  for entry in "${nodes[@]}"; do
    IFS=':' read -r name server port log_path svc_name <<< "$entry"
    echo "[$name]"

    # Check 1: RPC responding
    local info
    info=$(rpc_call "$server" "$port" "getChainInfo" | python3 -c "
import sys,json
try:
    r=json.load(sys.stdin).get('result',{})
    print(f\"{r.get('bestHeight','?')}|{r.get('genesisHash','?')}|{r.get('bestHash','?')}\")
except: print('OFFLINE')
" 2>/dev/null) || info="OFFLINE"

    if [[ "$info" == "OFFLINE" ]]; then
      fail "RPC not responding"
      continue
    fi
    pass "RPC responding"

    IFS='|' read -r height genesis best_hash <<< "$info"

    # Check 2: Peers
    local peer_count
    peer_count=$(rpc_call "$server" "$port" "getPeerInfo" | python3 -c "
import sys,json
try:
    r=json.load(sys.stdin).get('result',[])
    print(len(r) if isinstance(r,list) else 0)
except: print(0)
" 2>/dev/null) || peer_count=0

    if [[ "$peer_count" -ge 1 ]]; then
      pass "Peers: ${peer_count}"
    else
      fail "0 peers — node is isolated"
    fi

    # Check 3: Genesis hash
    if [[ -z "$ref_genesis" ]]; then
      ref_genesis="$genesis"
    fi
    if [[ "$genesis" == "$ref_genesis" ]]; then
      pass "Genesis: ${genesis:0:16}..."
    else
      fail "Genesis MISMATCH: ${genesis:0:16}... (expected ${ref_genesis:0:16}...)"
    fi

    # Track heights
    all_heights+=("$height")
    if (( height > max_height )); then max_height=$height; fi

    # Check 4: Height
    if (( max_height > 0 && height == 0 )); then
      fail "Height: 0 (stuck at genesis)"
    elif (( max_height > 0 )); then
      local behind=$((max_height - height))
      if (( behind <= 5 )); then
        pass "Height: ${height} (synced)"
      elif (( behind <= 100 )); then
        warn "Height: ${height} (behind ${behind} blocks, syncing)"
      else
        warn "Height: ${height} (behind ${behind} blocks)"
      fi
    else
      pass "Height: ${height}"
    fi

    # Check 5: Peer ID errors
    local pid_errors
    pid_errors=$(do_ssh "$server" "tail -200 ${log_path} 2>/dev/null | grep -c 'Unexpected peer ID' || echo 0" 2>/dev/null) || pid_errors=0

    if (( pid_errors == 0 )); then
      pass "No peer ID errors"
    elif (( pid_errors <= 5 )); then
      warn "Peer ID errors: ${pid_errors} (transient, may self-heal)"
    else
      fail "Peer ID errors: ${pid_errors} (persistent connectivity issue)"
    fi

    # Check 6: Service file has --bootstrap
    local has_bootstrap
    has_bootstrap=$(do_ssh "$server" "grep -c 'bootstrap' /etc/systemd/system/${svc_name}.service 2>/dev/null || echo 0" 2>/dev/null) || has_bootstrap=0

    local needs_bootstrap="true"
    if [[ "$name" == "Seed2" || "$name" == "SeedT1" ]]; then needs_bootstrap="false"; fi

    if [[ "$needs_bootstrap" == "true" && "$has_bootstrap" -lt 1 ]]; then
      fail "Service file MISSING --bootstrap flag"
    else
      pass "Service file has --bootstrap"
    fi

    # Check 7: Service file has LimitNOFILE=65535
    local has_nofile
    has_nofile=$(do_ssh "$server" "grep -c 'LimitNOFILE=65535' /etc/systemd/system/${svc_name}.service 2>/dev/null || echo 0" 2>/dev/null) || has_nofile=0

    if [[ "$has_nofile" -lt 1 ]]; then
      fail "Service file MISSING LimitNOFILE=65535"
    else
      pass "Service file has LimitNOFILE=65535"
    fi

    echo ""
  done

  echo "--- ${net_label} Summary ---"
  echo "  Max height: ${max_height}"
  echo "  Genesis:    ${ref_genesis:0:16}..."
  echo ""
}

do_mainnet() {
  local nodes=(
    "Seed1:${AI1}:8500:/var/log/doli/mainnet/seed.log:doli-mainnet-seed"
    "Seed2:${AI2}:8500:/var/log/doli/mainnet/seed.log:doli-mainnet-seed"
    "Seed3:${AI3}:8500:/var/log/doli/mainnet/seed.log:doli-mainnet-seed"
    "N1:${AI1}:8501:/var/log/doli/mainnet/n1.log:doli-mainnet-n1"
    "N2:${AI1}:8502:/var/log/doli/mainnet/n2.log:doli-mainnet-n2"
    "N3:${AI1}:8503:/var/log/doli/mainnet/n3.log:doli-mainnet-n3"
    "N4:${AI2}:8504:/var/log/doli/mainnet/n4.log:doli-mainnet-n4"
    "N5:${AI2}:8505:/var/log/doli/mainnet/n5.log:doli-mainnet-n5"
    "N6:${AI4}:8506:/var/log/doli/mainnet/n6.log:doli-mainnet-n6"
    "N7:${AI4}:8507:/var/log/doli/mainnet/n7.log:doli-mainnet-n7"
    "N8:${AI4}:8508:/var/log/doli/mainnet/n8.log:doli-mainnet-n8"
    "N9:${AI5}:8509:/var/log/doli/mainnet/n9.log:doli-mainnet-n9"
    "N10:${AI5}:8510:/var/log/doli/mainnet/n10.log:doli-mainnet-n10"
    "N11:${AI5}:8511:/var/log/doli/mainnet/n11.log:doli-mainnet-n11"
    "N12:${AI5}:8512:/var/log/doli/mainnet/n12.log:doli-mainnet-n12"
    "SANTIAGO:${AI3}:8513:/var/log/doli/mainnet/santiago.log:doli-mainnet-santiago"
    "IVAN:${AI3}:8514:/var/log/doli/mainnet/ivan.log:doli-mainnet-ivan"
  )
  check_network "Mainnet" "mainnet" "${nodes[@]}"
}

do_testnet() {
  local nodes=(
    "SeedT1:${AI1}:18500:/var/log/doli/testnet/seed.log:doli-testnet-seed"
    "SeedT2:${AI2}:18500:/var/log/doli/testnet/seed.log:doli-testnet-seed"
    "SeedT3:${AI3}:18500:/var/log/doli/testnet/seed.log:doli-testnet-seed"
    "NT1:${AI1}:18501:/var/log/doli/testnet/nt1.log:doli-testnet-nt1"
    "NT2:${AI1}:18502:/var/log/doli/testnet/nt2.log:doli-testnet-nt2"
    "NT3:${AI1}:18503:/var/log/doli/testnet/nt3.log:doli-testnet-nt3"
    "NT4:${AI1}:18504:/var/log/doli/testnet/nt4.log:doli-testnet-nt4"
    "NT5:${AI1}:18505:/var/log/doli/testnet/nt5.log:doli-testnet-nt5"
    "NT6:${AI5}:18506:/var/log/doli/testnet/nt6.log:doli-testnet-nt6"
    "NT7:${AI5}:18507:/var/log/doli/testnet/nt7.log:doli-testnet-nt7"
    "NT8:${AI5}:18508:/var/log/doli/testnet/nt8.log:doli-testnet-nt8"
    "NT9:${AI5}:18509:/var/log/doli/testnet/nt9.log:doli-testnet-nt9"
    "NT10:${AI5}:18510:/var/log/doli/testnet/nt10.log:doli-testnet-nt10"
    "NT11:${AI5}:18511:/var/log/doli/testnet/nt11.log:doli-testnet-nt11"
    "NT12:${AI5}:18512:/var/log/doli/testnet/nt12.log:doli-testnet-nt12"
  )
  check_network "Testnet" "testnet" "${nodes[@]}"
}

case "${1:-all}" in
  mainnet) do_mainnet ;;
  testnet) do_testnet ;;
  all)     do_mainnet; do_testnet ;;
  *)       echo "Usage: $0 [mainnet|testnet|all]"; exit 1 ;;
esac

echo "============================="
if (( ERRORS > 0 )); then
  echo -e "${RED}FAILED: ${ERRORS} errors, ${WARNINGS} warnings${NC}"
  echo "DO NOT proceed until all errors are resolved."
  exit 1
elif (( WARNINGS > 0 )); then
  echo -e "${YELLOW}PASSED with ${WARNINGS} warnings${NC}"
  echo "Warnings may self-resolve (syncing nodes). Re-check in 5 minutes."
  exit 0
else
  echo -e "${GREEN}ALL CHECKS PASSED${NC}"
  exit 0
fi

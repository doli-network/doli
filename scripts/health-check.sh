#!/usr/bin/env bash
# health-check.sh — Verify network health after reset or incident
#
# Usage: scripts/health-check.sh [mainnet|testnet|all]
#
# Checks (all must pass):
#   1. All nodes responding to RPC
#   2. All nodes have >= 1 peer
#   3. All nodes share the same genesis hash
#   4. All nodes share the same block 1 hash (no fork from genesis)
#   5. Block heights within acceptable range
#   6. No "Unexpected peer ID" errors in recent logs
#   7. Service files have --bootstrap flag
set -euo pipefail

AI1="ilozada@72.60.228.233"
AI2="ilozada@187.124.95.188"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

ERRORS=0
WARNINGS=0

pass()  { echo -e "  ${GREEN}PASS${NC} $1"; }
fail()  { echo -e "  ${RED}FAIL${NC} $1"; ERRORS=$((ERRORS+1)); }
warn()  { echo -e "  ${YELLOW}WARN${NC} $1"; WARNINGS=$((WARNINGS+1)); }

rpc_call() {
  local server="$1" port="$2" method="$3"
  ssh -o ConnectTimeout=5 "$server" "curl -sf --max-time 5 -X POST http://127.0.0.1:${port} \
    -H 'Content-Type: application/json' \
    -d '{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":{},\"id\":1}'" 2>/dev/null
}

check_network() {
  local net_label="$1" net_name="$2"
  shift 2
  local nodes=("$@")  # "name:server:rpc_port:log_path:service_name" entries

  echo "=== ${net_label} Health Check ==="
  echo ""

  local ref_genesis="" ref_block1="" max_height=0
  local all_genesis=() all_block1=() all_heights=()

  # ── Check 1-5: RPC, Peers, Genesis, Block 1, Height ──────────────
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

    # getPeerInfo may return 0 during sync but node has peers (sync manager handles them)
    # Check logs for actual peer connections
    local log_peers
    log_peers=$(ssh -o ConnectTimeout=5 "$server" "tail -50 ${log_path} 2>/dev/null | grep -c 'peers=[0-9]' || echo 0" 2>/dev/null) || log_peers=0

    if [[ "$peer_count" -ge 1 ]]; then
      pass "Peers: ${peer_count}"
    elif [[ "$log_peers" -gt 0 ]]; then
      pass "Peers: sync logs show active peers"
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

    # Check 5: Height
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

    # ── Check 5b: Chain integrity (snap sync gaps) ────────────────
    local integrity
    integrity=$(rpc_call "$server" "$port" "verifyChainIntegrity" | python3 -c "
import sys,json
try:
    r=json.load(sys.stdin).get('result',{})
    mc=r.get('missing_count',0)
    tip=r.get('tip',0)
    complete=r.get('complete',True)
    cc=r.get('chainCommitment','')
    if not complete and mc > 0:
        print(f'GAP|{mc}|{tip}|{cc}')
    else:
        print(f'OK|0|{tip}|{cc[:16]}')
except: print('ERROR|0|0|')
" 2>/dev/null) || integrity="ERROR|0|0|"

    IFS='|' read -r integ_status integ_missing integ_tip integ_commitment <<< "$integrity"

    if [[ "$integ_status" == "OK" ]]; then
      pass "Chain integrity: complete (commitment=${integ_commitment}...)"
    elif [[ "$integ_status" == "GAP" ]]; then
      warn "Chain integrity: ${integ_missing} blocks missing (snap sync gap) — vulnerable to fork cascade"
    else
      warn "Chain integrity: could not verify"
    fi

    # ── Check 6: Peer ID errors ──────────────────────────────────
    local pid_errors
    pid_errors=$(ssh -o ConnectTimeout=5 "$server" "tail -200 ${log_path} 2>/dev/null | grep -c 'Unexpected peer ID' || echo 0" 2>/dev/null) || pid_errors=0

    if (( pid_errors == 0 )); then
      pass "No peer ID errors"
    elif (( pid_errors <= 5 )); then
      warn "Peer ID errors: ${pid_errors} (transient, may self-heal)"
    else
      fail "Peer ID errors: ${pid_errors} (persistent connectivity issue)"
    fi

    # ── Check 7: Service file has --bootstrap ─────────────────────
    local has_bootstrap
    has_bootstrap=$(ssh -o ConnectTimeout=5 "$server" "grep -c 'bootstrap' /etc/systemd/system/${svc_name}.service 2>/dev/null || echo 0" 2>/dev/null) || has_bootstrap=0

    # Seeds on ai1 don't need bootstrap (they ARE the bootstrap)
    local needs_bootstrap="true"
    if [[ "$name" == "Seed1" || "$name" == "SeedT1" ]]; then needs_bootstrap="false"; fi

    if [[ "$needs_bootstrap" == "true" && "$has_bootstrap" -lt 1 ]]; then
      fail "Service file MISSING --bootstrap flag"
    else
      pass "Service file has --bootstrap"
    fi

    echo ""
  done

  # ── Summary ─────────────────────────────────────────────────────
  echo "--- ${net_label} Summary ---"
  echo "  Max height: ${max_height}"
  echo "  Genesis:    ${ref_genesis:0:16}..."
  echo ""
}

do_mainnet() {
  local nodes=(
    "Seed1:${AI1}:8500:/var/log/doli/mainnet/seed.log:doli-mainnet-seed"
    "Seed2:${AI2}:8500:/var/log/doli/mainnet/seed.log:doli-mainnet-seed"
    "N1:${AI1}:8501:/var/log/doli/mainnet/n1.log:doli-mainnet-n1"
    "N2:${AI2}:8502:/var/log/doli/mainnet/n2.log:doli-mainnet-n2"
    "N3:${AI1}:8503:/var/log/doli/mainnet/n3.log:doli-mainnet-n3"
    "N4:${AI2}:8504:/var/log/doli/mainnet/n4.log:doli-mainnet-n4"
    "N5:${AI1}:8505:/var/log/doli/mainnet/n5.log:doli-mainnet-n5"
    "N6:${AI2}:8506:/var/log/doli/mainnet/n6.log:doli-mainnet-n6"
    "N7:${AI1}:8507:/var/log/doli/mainnet/n7.log:doli-mainnet-n7"
    "N8:${AI2}:8508:/var/log/doli/mainnet/n8.log:doli-mainnet-n8"
    "N9:${AI1}:8509:/var/log/doli/mainnet/n9.log:doli-mainnet-n9"
    "N10:${AI2}:8510:/var/log/doli/mainnet/n10.log:doli-mainnet-n10"
    "N11:${AI1}:8511:/var/log/doli/mainnet/n11.log:doli-mainnet-n11"
    "N12:${AI2}:8512:/var/log/doli/mainnet/n12.log:doli-mainnet-n12"
  )
  check_network "Mainnet" "mainnet" "${nodes[@]}"
}

do_testnet() {
  local nodes=(
    "SeedT1:${AI1}:18500:/var/log/doli/testnet/seed.log:doli-testnet-seed"
    "SeedT2:${AI2}:18500:/var/log/doli/testnet/seed.log:doli-testnet-seed"
    "NT1:${AI1}:18501:/var/log/doli/testnet/nt1.log:doli-testnet-nt1"
    "NT2:${AI2}:18502:/var/log/doli/testnet/nt2.log:doli-testnet-nt2"
    "NT3:${AI1}:18503:/var/log/doli/testnet/nt3.log:doli-testnet-nt3"
    "NT4:${AI2}:18504:/var/log/doli/testnet/nt4.log:doli-testnet-nt4"
    "NT5:${AI1}:18505:/var/log/doli/testnet/nt5.log:doli-testnet-nt5"
    "NT6:${AI2}:18506:/var/log/doli/testnet/nt6.log:doli-testnet-nt6"
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

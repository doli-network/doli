#!/usr/bin/env bash
# Usage: scripts/status.sh [mainnet|testnet|all]
# Shows node status (height, slot, version, fork detection) via explorer API.
# Falls back to SSH if explorer is down.
set -euo pipefail

EXPLORER_MN="https://explorer.doli.network/api/mainnet"
EXPLORER_TN="https://explorer.doli.network/api/testnet"

rpc_via_explorer() {
  local base="$1" node="$2" method="$3"
  curl -sf --max-time 5 -X POST "${base}/${node}" \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":{},\"id\":1}" 2>/dev/null | python3 -c "import sys,json; print(json.dumps(json.load(sys.stdin).get('result',{})))" 2>/dev/null
}

check_nodes() {
  local net_label="$1" base="$2"; shift 2; local nodes=("$@")
  local ref_hash="" ref_height=0

  printf "%-5s %8s %8s %10s %s\n" "Node" "Height" "Slot" "Version" "Status"
  printf "%-5s %8s %8s %10s %s\n" "-----" "--------" "--------" "----------" "------"

  for n in "${nodes[@]}"; do
    local info
    info=$(rpc_via_explorer "$base" "$n" "getChainInfo")
    if [[ -z "$info" || "$info" == "{}" || "$info" == "null" ]]; then
      printf "%-5s %8s %8s %10s %s\n" "$(echo "$n" | tr '[:lower:]' '[:upper:]')" "-" "-" "-" "❌ OFFLINE"
      continue
    fi

    local h s v bh
    h=$(echo "$info" | python3 -c "import sys,json; print(json.load(sys.stdin).get('bestHeight','?'))")
    s=$(echo "$info" | python3 -c "import sys,json; print(json.load(sys.stdin).get('bestSlot','?'))")
    v=$(echo "$info" | python3 -c "import sys,json; print(json.load(sys.stdin).get('version','?'))")
    bh=$(echo "$info" | python3 -c "import sys,json; print(json.load(sys.stdin).get('bestHash','?'))")

    # Track reference for fork detection
    if [[ -z "$ref_hash" ]]; then
      ref_hash="$bh"
      ref_height="$h"
    fi

    local status="✅"
    if [[ "$bh" != "$ref_hash" ]]; then
      # Different hash — check if just behind or actual fork
      local diff=$((ref_height - h))
      if (( diff > 5 && diff <= 50 )); then
        status="⚠️ BEHIND"
      elif (( diff > 50 )); then
        status="❌ STUCK"
      else
        status="⚠️ FORK"
      fi
    fi

    printf "%-5s %8s %8s %10s %s\n" "$(echo "$n" | tr '[:lower:]' '[:upper:]')" "$h" "$s" "$v" "$status"
  done
  echo ""
}

do_mainnet() {
  echo "⛏ Mainnet"
  check_nodes "MN" "$EXPLORER_MN" n1 n2 n3 n4 n5 n6 n7 n8 n9 n10 n11 n12
  # Chain stats from N1
  local stats
  stats=$(rpc_via_explorer "$EXPLORER_MN" "n1" "getChainStats")
  if [[ -n "$stats" && "$stats" != "{}" ]]; then
    local supply staked producers
    supply=$(echo "$stats" | python3 -c "import sys,json; print(f'{json.load(sys.stdin).get(\"totalSupply\",0)/1e8:.1f}')")
    staked=$(echo "$stats" | python3 -c "import sys,json; print(f'{json.load(sys.stdin).get(\"totalStaked\",0)/1e8:.1f}')")
    producers=$(echo "$stats" | python3 -c "import sys,json; print(json.load(sys.stdin).get('activeProducers','?'))")
    echo "📊 Supply=${supply} DOLI  Staked=${staked} DOLI  Producers=${producers}"
    echo ""
  fi
}

do_testnet() {
  echo "🧪 Testnet"
  check_nodes "TN" "$EXPLORER_TN" nt1 nt2 nt3 nt4 nt5 nt6 nt7 nt8 nt9 nt10 nt11 nt12
}

case "${1:-all}" in
  mainnet) do_mainnet ;;
  testnet) do_testnet ;;
  all)     do_mainnet; do_testnet ;;
  *)       echo "Usage: $0 [mainnet|testnet|all]"; exit 1 ;;
esac

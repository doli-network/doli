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

  # Pass 1: collect all node data
  local -a names=() heights=() slots=() versions=() hashes=() online=()
  for n in "${nodes[@]}"; do
    local info
    info=$(rpc_via_explorer "$base" "$n" "getChainInfo")
    names+=("$(echo "$n" | tr '[:lower:]' '[:upper:]')")
    if [[ -z "$info" || "$info" == "{}" || "$info" == "null" ]]; then
      heights+=("-"); slots+=("-"); versions+=("-"); hashes+=("-"); online+=(0)
      continue
    fi
    heights+=("$(echo "$info" | python3 -c "import sys,json; print(json.load(sys.stdin).get('bestHeight','?'))")")
    slots+=("$(echo "$info" | python3 -c "import sys,json; print(json.load(sys.stdin).get('bestSlot','?'))")")
    versions+=("$(echo "$info" | python3 -c "import sys,json; print(json.load(sys.stdin).get('version','?'))")")
    hashes+=("$(echo "$info" | python3 -c "import sys,json; print(json.load(sys.stdin).get('bestHash','?'))")")
    online+=(1)
  done

  # Pass 2: find majority hash (most common hash = canonical tip)
  local max_height=0
  for i in "${!names[@]}"; do
    if (( online[i] )); then
      local h="${heights[$i]}"
      (( h > max_height )) && max_height="$h"
    fi
  done

  # Count occurrences of each hash, pick the most common
  local majority_hash="" majority_count=0
  local unique_hashes
  unique_hashes=$(printf '%s\n' "${hashes[@]}" | grep -v '^-$' | sort -u)
  while IFS= read -r candidate; do
    local count=0
    for i in "${!names[@]}"; do
      (( online[i] )) && [[ "${hashes[$i]}" == "$candidate" ]] && (( count++ ))
    done
    if (( count > majority_count )); then
      majority_hash="$candidate"
      majority_count="$count"
    fi
  done <<< "$unique_hashes"

  # Pass 3: display with correct status
  printf "%-5s %8s %8s %10s %s\n" "Node" "Height" "Slot" "Version" "Status"
  printf "%-5s %8s %8s %10s %s\n" "-----" "--------" "--------" "----------" "------"

  for i in "${!names[@]}"; do
    if (( ! online[i] )); then
      printf "%-5s %8s %8s %10s %s\n" "${names[$i]}" "-" "-" "-" "❌ OFFLINE"
      continue
    fi

    local status="✅"
    local bh="${hashes[$i]}" h="${heights[$i]}"
    if [[ "$bh" != "$majority_hash" ]]; then
      local diff=$(( max_height - h ))
      if (( diff <= 3 )); then
        # Within 3 blocks of tip with different hash = propagation delay
        status="✅ (syncing)"
      elif (( diff <= 50 )); then
        status="⚠️ BEHIND"
      elif (( diff > 50 )); then
        status="❌ STUCK"
      fi
    fi
    # Same height, different hash from majority = actual fork
    if [[ "$bh" != "$majority_hash" && "$h" == "$max_height" ]]; then
      status="⚠️ FORK"
    fi

    printf "%-5s %8s %8s %10s %s\n" "${names[$i]}" "$h" "${slots[$i]}" "${versions[$i]}" "$status"
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

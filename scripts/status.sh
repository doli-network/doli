#!/usr/bin/env bash
# Usage: scripts/status.sh [devnet]
# Shows status of local DOLI nodes by scanning RPC ports.
# Auto-detects running nodes in the devnet port range (28500-28550).
set -euo pipefail

BASE_RPC_PORT=28500
MAX_SCAN=50  # scan 28500-28550

rpc_call() {
  local port="$1" method="$2"
  curl -sf --max-time 2 -X POST "http://127.0.0.1:${port}" \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":{},\"id\":1}" 2>/dev/null \
    | python3 -c "import sys,json; print(json.dumps(json.load(sys.stdin).get('result',{})))" 2>/dev/null
}

echo "Scanning local RPC ports ${BASE_RPC_PORT}-$((BASE_RPC_PORT + MAX_SCAN))..."
echo ""

# Pass 1: collect all node data
declare -a names=() heights=() slots=() versions=() hashes=() ports=()
found=0

for ((offset=0; offset<=MAX_SCAN; offset++)); do
  port=$((BASE_RPC_PORT + offset))
  info=$(rpc_call "$port" "getChainInfo") || continue
  [[ -z "$info" || "$info" == "{}" || "$info" == "null" ]] && continue

  found=$((found + 1))
  if [[ "$offset" -eq 0 ]]; then
    names+=("SEED")
  else
    names+=("N${offset}")
  fi
  ports+=("$port")
  heights+=("$(echo "$info" | python3 -c "import sys,json; print(json.load(sys.stdin).get('bestHeight','?'))")")
  slots+=("$(echo "$info" | python3 -c "import sys,json; print(json.load(sys.stdin).get('bestSlot','?'))")")
  versions+=("$(echo "$info" | python3 -c "import sys,json; print(json.load(sys.stdin).get('version','?'))")")
  hashes+=("$(echo "$info" | python3 -c "import sys,json; print(json.load(sys.stdin).get('bestHash','?'))")")
done

if [[ "$found" -eq 0 ]]; then
  echo "No running nodes found on ports ${BASE_RPC_PORT}-$((BASE_RPC_PORT + MAX_SCAN))."
  echo ""
  echo "Start a devnet with:"
  echo "  doli-node devnet start"
  echo "  # or: scripts/launch_testnet.sh"
  exit 0
fi

# Pass 2: find majority hash
max_height=0
for i in "${!names[@]}"; do
  h="${heights[$i]}"
  (( h > max_height )) && max_height="$h"
done

majority_hash="" majority_count=0
unique_hashes=$(printf '%s\n' "${hashes[@]}" | sort -u)
while IFS= read -r candidate; do
  count=0
  for i in "${!names[@]}"; do
    [[ "${hashes[$i]}" == "$candidate" ]] && (( count++ ))
  done
  if (( count > majority_count )); then
    majority_hash="$candidate"
    majority_count="$count"
  fi
done <<< "$unique_hashes"

# Pass 3: display
printf "%-6s %5s %8s %8s %10s %s\n" "Node" "Port" "Height" "Slot" "Version" "Status"
printf "%-6s %5s %8s %8s %10s %s\n" "------" "-----" "--------" "--------" "----------" "------"

for i in "${!names[@]}"; do
  status="✅"
  bh="${hashes[$i]}" h="${heights[$i]}"
  if [[ "$bh" != "$majority_hash" ]]; then
    diff=$(( max_height - h ))
    if (( diff <= 3 )); then
      status="✅ (syncing)"
    elif (( diff <= 50 )); then
      status="⚠️ BEHIND"
    else
      status="❌ STUCK"
    fi
  fi
  if [[ "$bh" != "$majority_hash" && "$h" == "$max_height" ]]; then
    status="⚠️ FORK"
  fi
  printf "%-6s %5s %8s %8s %10s %s\n" "${names[$i]}" "${ports[$i]}" "$h" "${slots[$i]}" "${versions[$i]}" "$status"
done

echo ""
echo "Found $found node(s)."

# Chain stats from first node
stats=$(rpc_call "${ports[0]}" "getChainStats") || true
if [[ -n "$stats" && "$stats" != "{}" ]]; then
  supply=$(echo "$stats" | python3 -c "import sys,json; print(f'{json.load(sys.stdin).get(\"totalSupply\",0)/1e8:.1f}')")
  staked=$(echo "$stats" | python3 -c "import sys,json; print(f'{json.load(sys.stdin).get(\"totalStaked\",0)/1e8:.1f}')")
  producers=$(echo "$stats" | python3 -c "import sys,json; print(json.load(sys.stdin).get('activeProducers','?'))")
  echo "Supply=${supply} DOLI  Staked=${staked} DOLI  Producers=${producers}"
  echo ""
fi

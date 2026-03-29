#!/usr/bin/env bash
# fork-monitor.sh — Detect chain forks across DOLI nodes
#
# Usage:
#   scripts/fork-monitor.sh                    # scan local devnet ports (28500-28550)
#   scripts/fork-monitor.sh --testnet          # scan testnet ports (8500-8512)
#   scripts/fork-monitor.sh --loop [SECS]      # continuous monitoring (default: 30s)
#   scripts/fork-monitor.sh --endpoints FILE   # read host:port from file
#
# Exit codes: 0 = all nodes agree, 1 = FORK DETECTED, 2 = error
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
NC='\033[0m'

# Defaults
MODE="devnet"
LOOP=false
LOOP_INTERVAL=30
ENDPOINTS_FILE=""

# Parse args
while [[ $# -gt 0 ]]; do
  case "$1" in
    --testnet)  MODE="testnet"; shift ;;
    --devnet)   MODE="devnet"; shift ;;
    --loop)
      LOOP=true
      if [[ "${2:-}" =~ ^[0-9]+$ ]]; then
        LOOP_INTERVAL="$2"; shift
      fi
      shift ;;
    --endpoints) ENDPOINTS_FILE="$2"; shift 2 ;;
    *) echo "Unknown arg: $1"; exit 2 ;;
  esac
done

rpc_call() {
  local port="$1" method="$2"
  curl -sf --max-time 3 -X POST "http://127.0.0.1:${port}" \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":{},\"id\":1}" 2>/dev/null
}

check_forks() {
  local timestamp
  timestamp=$(date '+%Y-%m-%d %H:%M:%S')

  # Determine port range
  local start_port end_port
  if [[ "$MODE" == "testnet" ]]; then
    start_port=8500; end_port=8512
  else
    start_port=28500; end_port=28550
  fi

  # Collect chain info from all reachable nodes
  local alive=0
  local node_data=""  # "name|height|hash\n" lines

  for ((port=start_port; port<=end_port; port++)); do
    local result
    result=$(rpc_call "$port" "getChainInfo" 2>/dev/null) || continue
    [[ -z "$result" || "$result" == "null" ]] && continue

    local height hash
    height=$(echo "$result" | python3 -c "import sys,json; print(json.load(sys.stdin).get('result',{}).get('bestHeight',''))" 2>/dev/null) || continue
    hash=$(echo "$result" | python3 -c "import sys,json; print(json.load(sys.stdin).get('result',{}).get('bestHash',''))" 2>/dev/null) || continue

    [[ -z "$height" || -z "$hash" ]] && continue

    local name
    if [[ $port -eq $start_port ]]; then name="Seed"; else name="N$((port - start_port))"; fi

    node_data+="${name}|${height}|${hash}"$'\n'
    alive=$((alive + 1))
  done

  if [[ $alive -eq 0 ]]; then
    echo -e "${RED}[$timestamp] No nodes reachable!${NC}"
    return 2
  fi

  # Use python3 to group by hash and report (avoids bash associative arrays)
  local result_code
  result_code=$(echo "$node_data" | python3 -c "
import sys

lines = [l.strip() for l in sys.stdin if l.strip()]
groups = {}
for line in lines:
    parts = line.split('|')
    if len(parts) != 3:
        continue
    name, height, h = parts
    short = h[:12] + '...' + h[-6:] if len(h) > 18 else h
    key = (short, height)
    groups.setdefault(short, {'height': height, 'nodes': []})
    groups[short]['nodes'].append(name)

num_groups = len(groups)
alive = sum(len(g['nodes']) for g in groups.values())

if num_groups == 1:
    h = list(groups.keys())[0]
    height = list(groups.values())[0]['height']
    print(f'OK|{alive}|{height}|{h}')
else:
    print(f'FORK|{alive}|{num_groups}')
    for h, g in groups.items():
        print(f'GROUP|{h}|{g[\"height\"]}|{', '.join(g[\"nodes\"])}')
" 2>/dev/null)

  # Parse python output
  local first_line
  first_line=$(echo "$result_code" | head -1)
  local status="${first_line%%|*}"

  if [[ "$status" == "OK" ]]; then
    local ok_alive ok_height ok_hash
    IFS='|' read -r _ ok_alive ok_height ok_hash <<< "$first_line"
    echo -e "${GREEN}[$timestamp] OK${NC} — $ok_alive nodes, height=$ok_height, hash=$ok_hash"
    return 0
  elif [[ "$status" == "FORK" ]]; then
    local f_alive f_groups
    IFS='|' read -r _ f_alive f_groups <<< "$first_line"
    echo ""
    echo -e "${RED}${BOLD}[$timestamp] FORK DETECTED — $f_groups chain tips across $f_alive nodes!${NC}"
    echo ""
    local group_num=1
    while IFS='|' read -r tag hash height nodes; do
      [[ "$tag" != "GROUP" ]] && continue
      echo -e "  ${YELLOW}Group $group_num${NC}: hash=$hash  height=$height"
      echo -e "    Nodes: $nodes"
      group_num=$((group_num + 1))
    done <<< "$result_code"
    echo ""
    echo -e "  ${RED}ACTION: Run 'scripts/emergency-halt.sh' to stop all producers${NC}"
    echo ""
    return 1
  else
    echo -e "${RED}[$timestamp] Error parsing node data${NC}"
    return 2
  fi
}

# Main
if [[ "$LOOP" == true ]]; then
  echo -e "${BOLD}Fork monitor running (every ${LOOP_INTERVAL}s). Ctrl+C to stop.${NC}"
  echo ""
  while true; do
    check_forks || true
    sleep "$LOOP_INTERVAL"
  done
else
  check_forks
fi

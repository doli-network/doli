#!/usr/bin/env bash
# emergency-halt.sh — Pause block production on all DOLI producer nodes
#
# Seeds are unaffected (they never produce). The chain freezes at its current
# height but all data is preserved. Resume with emergency-resume.sh.
#
# Usage:
#   scripts/emergency-halt.sh                  # local devnet (28500-28550)
#   scripts/emergency-halt.sh --testnet        # local testnet (8500-8512)
#   scripts/emergency-halt.sh --endpoints FILE # custom endpoint list
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
NC='\033[0m'

MODE="devnet"
ENDPOINTS_FILE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --testnet)  MODE="testnet"; shift ;;
    --devnet)   MODE="devnet"; shift ;;
    --endpoints) ENDPOINTS_FILE="$2"; shift 2 ;;
    *) echo "Unknown arg: $1"; exit 1 ;;
  esac
done

rpc_call() {
  local endpoint="$1" method="$2"
  curl -sf --max-time 5 -X POST "http://${endpoint}" \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":[],\"id\":1}" 2>/dev/null
}

get_endpoints() {
  if [[ -n "$ENDPOINTS_FILE" ]]; then
    grep -v '^#' "$ENDPOINTS_FILE" | grep -v '^$'
  elif [[ "$MODE" == "testnet" ]]; then
    for ((i=0; i<=12; i++)); do echo "127.0.0.1:$((8500 + i))"; done
  else
    for ((i=0; i<=50; i++)); do echo "127.0.0.1:$((28500 + i))"; done
  fi
}

echo -e "${BOLD}${RED}=== EMERGENCY PRODUCTION HALT ===${NC}"
echo ""
echo "This will pause block production on ALL reachable nodes."
echo "Seeds continue running. Chain data is preserved."
echo ""
read -rp "Proceed? (yes/no): " confirm
if [[ "$confirm" != "yes" ]]; then
  echo "Aborted."
  exit 0
fi

echo ""
halted=0
failed=0
unreachable=0

while IFS= read -r endpoint; do
  # First check if node is reachable
  result=$(rpc_call "$endpoint" "getGuardianStatus" 2>/dev/null) || {
    unreachable=$((unreachable + 1))
    continue
  }
  [[ -z "$result" || "$result" == "null" ]] && { unreachable=$((unreachable + 1)); continue; }

  # Pause production
  pause_result=$(rpc_call "$endpoint" "pauseProduction" 2>/dev/null) || {
    echo -e "  ${RED}FAIL${NC} $endpoint — RPC call failed"
    failed=$((failed + 1))
    continue
  }

  status=$(echo "$pause_result" | python3 -c "import sys,json; print(json.load(sys.stdin).get('result',{}).get('status','error'))" 2>/dev/null)
  if [[ "$status" == "paused" ]]; then
    echo -e "  ${GREEN}HALT${NC} $endpoint"
    halted=$((halted + 1))
  else
    echo -e "  ${YELLOW}WARN${NC} $endpoint — unexpected response: $pause_result"
    failed=$((failed + 1))
  fi
done < <(get_endpoints)

echo ""
echo -e "${BOLD}Results:${NC} $halted halted, $failed failed, $unreachable unreachable"
echo ""
if [[ $halted -gt 0 ]]; then
  echo -e "${GREEN}Production paused on $halted nodes.${NC}"
  echo "  - Seeds continue running (chain data safe)"
  echo "  - Run 'scripts/emergency-resume.sh' when ready to resume"
  echo "  - Run 'scripts/seed-backup.sh' to create a checkpoint first"
fi

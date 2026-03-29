#!/usr/bin/env bash
# emergency-resume.sh — Resume block production on all DOLI nodes
#
# Counterpart to emergency-halt.sh. Clears the production pause flag
# on all reachable nodes.
#
# Usage:
#   scripts/emergency-resume.sh                  # local devnet (28500-28550)
#   scripts/emergency-resume.sh --testnet        # local testnet (8500-8512)
#   scripts/emergency-resume.sh --endpoints FILE # custom endpoint list
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

echo -e "${BOLD}${GREEN}=== RESUME PRODUCTION ===${NC}"
echo ""

resumed=0
failed=0
unreachable=0

while IFS= read -r endpoint; do
  result=$(rpc_call "$endpoint" "resumeProduction" 2>/dev/null) || {
    unreachable=$((unreachable + 1))
    continue
  }
  [[ -z "$result" || "$result" == "null" ]] && { unreachable=$((unreachable + 1)); continue; }

  status=$(echo "$result" | python3 -c "import sys,json; print(json.load(sys.stdin).get('result',{}).get('status','error'))" 2>/dev/null)
  if [[ "$status" == "resumed" ]]; then
    echo -e "  ${GREEN}RESUME${NC} $endpoint"
    resumed=$((resumed + 1))
  else
    echo -e "  ${YELLOW}WARN${NC} $endpoint — $result"
    failed=$((failed + 1))
  fi
done < <(get_endpoints)

echo ""
echo -e "${BOLD}Results:${NC} $resumed resumed, $failed failed, $unreachable unreachable"

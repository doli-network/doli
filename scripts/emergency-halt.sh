#!/usr/bin/env bash
# emergency-halt.sh — Pause block production on all DOLI producer nodes
#
# Seeds are unaffected (they never produce). The chain freezes at its current
# height but all data is preserved. Resume with emergency-resume.sh.
#
# Usage:
#   scripts/emergency-halt.sh                  # local devnet (28500-28550)
#   scripts/emergency-halt.sh --testnet        # local testnet (8500-8512)
#   scripts/emergency-halt.sh --mainnet        # mainnet (RPC + SSH fallback)
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
    --mainnet)  MODE="mainnet"; shift ;;
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

# INC-I-055: SSH fallback — stop the systemd service when RPC is unreachable.
# Uses SSH aliases from ~/.ssh/config (ai1-ai5). Returns 0 on success.
ssh_stop_service() {
  local ssh_host="$1" service="$2"
  echo -e "  ${YELLOW}SSH${NC} Trying SSH fallback: ssh ${ssh_host} sudo systemctl stop ${service}"
  if ssh -o ConnectTimeout=5 -o BatchMode=yes "${ssh_host}" "sudo systemctl stop ${service}" 2>/dev/null; then
    return 0
  else
    return 1
  fi
}

get_endpoints() {
  if [[ -n "$ENDPOINTS_FILE" ]]; then
    grep -v '^#' "$ENDPOINTS_FILE" | grep -v '^$'
  elif [[ "$MODE" == "testnet" ]]; then
    for ((i=0; i<=12; i++)); do echo "127.0.0.1:$((8500 + i))"; done
  elif [[ "$MODE" == "mainnet" ]]; then
    # Mainnet producers: ssh_alias:rpc_endpoint:service_name
    # Format: ssh_host:ip:port:service
    # Using SSH aliases from ~/.ssh/config
    echo "ai1:8501:doli-mainnet-n1"
    echo "ai1:8502:doli-mainnet-n2"
    echo "ai1:8503:doli-mainnet-n3"
    echo "ai2:8504:doli-mainnet-n4"
    echo "ai2:8505:doli-mainnet-n5"
    echo "ai4:8506:doli-mainnet-n6"
    echo "ai4:8507:doli-mainnet-n7"
    echo "ai4:8508:doli-mainnet-n8"
    echo "ai5:8509:doli-mainnet-n9"
    echo "ai5:8510:doli-mainnet-n10"
    echo "ai5:8511:doli-mainnet-n11"
    echo "ai5:8512:doli-mainnet-n12"
    echo "ai3:8513:doli-mainnet-ivan"
    echo "ai3:8514:doli-mainnet-santiago"
  else
    for ((i=0; i<=50; i++)); do echo "127.0.0.1:$((28500 + i))"; done
  fi
}

echo -e "${BOLD}${RED}=== EMERGENCY PRODUCTION HALT ===${NC}"
echo ""
echo "Mode: ${MODE}"
echo "This will pause block production on ALL reachable nodes."
echo "Seeds continue running. Chain data is preserved."
if [[ "$MODE" == "mainnet" ]]; then
  echo "SSH fallback will be used when RPC is unreachable."
fi
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
ssh_halted=0

if [[ "$MODE" == "mainnet" ]]; then
  # Mainnet mode: try RPC first, fall back to SSH + systemctl stop
  while IFS=: read -r ssh_host rpc_port service; do
    # Resolve RPC endpoint: SSH to get the bind address, or use the SSH host
    # For mainnet, RPC listens on 0.0.0.0 so we connect via SSH tunnel concept
    # Actually, we need the real IP. Use ssh to call localhost on the remote.
    rpc_endpoint="localhost:${rpc_port}"

    # Try RPC via SSH port forward (direct RPC only works if ports are exposed)
    # Simpler: try to call RPC through SSH
    rpc_result=$(ssh -o ConnectTimeout=5 -o BatchMode=yes "${ssh_host}" \
      "curl -sf --max-time 5 -X POST 'http://127.0.0.1:${rpc_port}' \
       -H 'Content-Type: application/json' \
       -d '{\"jsonrpc\":\"2.0\",\"method\":\"pauseProduction\",\"params\":[],\"id\":1}'" 2>/dev/null) || rpc_result=""

    if [[ -n "$rpc_result" ]]; then
      status=$(echo "$rpc_result" | python3 -c "import sys,json; print(json.load(sys.stdin).get('result',{}).get('status','error'))" 2>/dev/null || echo "error")
      if [[ "$status" == "paused" ]]; then
        echo -e "  ${GREEN}HALT${NC} ${ssh_host}:${service} (via RPC)"
        halted=$((halted + 1))
        continue
      fi
    fi

    # RPC failed — fall back to SSH systemctl stop
    if ssh_stop_service "${ssh_host}" "${service}"; then
      echo -e "  ${GREEN}HALT${NC} ${ssh_host}:${service} (via SSH systemctl stop)"
      ssh_halted=$((ssh_halted + 1))
    else
      echo -e "  ${RED}FAIL${NC} ${ssh_host}:${service} — both RPC and SSH failed"
      failed=$((failed + 1))
    fi
  done < <(get_endpoints)
else
  # Local mode: RPC only (devnet/testnet)
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
fi

echo ""
if [[ "$MODE" == "mainnet" ]]; then
  echo -e "${BOLD}Results:${NC} $halted halted (RPC), $ssh_halted halted (SSH), $failed failed"
else
  echo -e "${BOLD}Results:${NC} $halted halted, $failed failed, $unreachable unreachable"
fi
echo ""
total_halted=$((halted + ssh_halted))
if [[ $total_halted -gt 0 ]]; then
  echo -e "${GREEN}Production paused on $total_halted nodes.${NC}"
  echo "  - Seeds continue running (chain data safe)"
  echo "  - Run 'scripts/emergency-resume.sh' when ready to resume"
  echo "  - Run 'scripts/seed-backup.sh' to create a checkpoint first"
  if [[ $ssh_halted -gt 0 ]]; then
    echo ""
    echo -e "  ${YELLOW}NOTE:${NC} $ssh_halted nodes were stopped via systemctl (not paused)."
    echo "  These nodes need 'sudo systemctl start <service>' to restart, not resumeProduction."
  fi
fi

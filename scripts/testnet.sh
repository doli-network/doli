#!/usr/bin/env bash
# testnet.sh — Manage local testnet launchd services
#
# Usage:
#   scripts/testnet.sh start [seed|n1|n2|...|all]   Start services
#   scripts/testnet.sh stop [seed|n1|n2|...|all]     Stop services
#   scripts/testnet.sh restart [seed|n1|n2|...|all]  Restart services
#   scripts/testnet.sh status                        Show all service status
#   scripts/testnet.sh logs [seed|n1|n2|...]         Tail logs
#
# Examples:
#   scripts/testnet.sh start all          # Start seed + all producers
#   scripts/testnet.sh start seed         # Start seed only
#   scripts/testnet.sh start n1 n5 n12    # Start specific producers
#   scripts/testnet.sh stop all           # Stop everything
#   scripts/testnet.sh status             # Show status of all services
#   scripts/testnet.sh logs n1            # Tail n1 log
set -euo pipefail

TESTNET_DIR="$HOME/testnet"
LOG_DIR="$TESTNET_DIR/logs"
LABEL_PREFIX="network.doli.testnet"

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# Resolve node names to launchd labels
resolve_targets() {
  local targets=()
  for arg in "$@"; do
    case "$arg" in
      all)
        targets+=("${LABEL_PREFIX}-seed")
        for i in $(seq 1 12); do targets+=("${LABEL_PREFIX}-n${i}"); done
        targets+=("network.doli.swap")
        targets+=("network.doli.explorer")
        ;;
      seed)
        targets+=("${LABEL_PREFIX}-seed")
        ;;
      swap)
        targets+=("network.doli.swap")
        ;;
      explorer)
        targets+=("network.doli.explorer")
        ;;
      n[0-9]|n[0-9][0-9])
        targets+=("${LABEL_PREFIX}-${arg}")
        ;;
      [0-9]|[0-9][0-9])
        targets+=("${LABEL_PREFIX}-n${arg}")
        ;;
      *)
        echo "Unknown target: $arg"
        exit 1
        ;;
    esac
  done
  echo "${targets[@]}"
}

do_start() {
  if [[ $# -eq 0 ]]; then
    echo "Usage: $0 start [seed|n1|n2|...|all]"
    exit 1
  fi

  local targets
  targets=($(resolve_targets "$@"))

  # Start seed first if included
  for label in "${targets[@]}"; do
    if [[ "$label" == *"-seed" ]]; then
      local plist="$HOME/Library/LaunchAgents/${label}.plist"
      if [[ ! -f "$plist" ]]; then
        echo -e "  ${RED}Not installed:${NC} $label"
        continue
      fi
      echo -n "  Starting $label... "
      launchctl load "$plist" 2>/dev/null || true
      launchctl start "$label" 2>/dev/null || true
      echo -e "${GREEN}OK${NC}"
      # Give seed time to initialize before producers
      if [[ ${#targets[@]} -gt 1 ]]; then
        echo "  Waiting 5s for seed to initialize..."
        sleep 5
      fi
    fi
  done

  # Then start producers
  for label in "${targets[@]}"; do
    [[ "$label" == *"-seed" ]] && continue
    local plist="$HOME/Library/LaunchAgents/${label}.plist"
    if [[ ! -f "$plist" ]]; then
      echo -e "  ${RED}Not installed:${NC} $label"
      continue
    fi
    echo -n "  Starting $label... "
    launchctl load "$plist" 2>/dev/null || true
    launchctl start "$label" 2>/dev/null || true
    echo -e "${GREEN}OK${NC}"
    sleep 1
  done
}

do_stop() {
  if [[ $# -eq 0 ]]; then
    echo "Usage: $0 stop [seed|n1|n2|...|all]"
    exit 1
  fi

  local targets
  targets=($(resolve_targets "$@"))

  # Stop producers first, then seed
  for label in "${targets[@]}"; do
    [[ "$label" == *"-seed" ]] && continue
    echo -n "  Stopping $label... "
    launchctl stop "$label" 2>/dev/null || true
    launchctl unload "$HOME/Library/LaunchAgents/${label}.plist" 2>/dev/null || true
    echo -e "${GREEN}OK${NC}"
  done

  for label in "${targets[@]}"; do
    [[ "$label" != *"-seed" ]] && continue
    echo -n "  Stopping $label... "
    launchctl stop "$label" 2>/dev/null || true
    launchctl unload "$HOME/Library/LaunchAgents/${label}.plist" 2>/dev/null || true
    echo -e "${GREEN}OK${NC}"
  done
}

do_restart() {
  do_stop "$@"
  sleep 2
  do_start "$@"
}

do_status() {
  printf "%-8s %6s %8s %8s %10s %s\n" "Node" "PID" "Height" "Slot" "Version" "Status"
  printf "%-8s %6s %8s %8s %10s %s\n" "--------" "------" "--------" "--------" "----------" "------"

  for label in "${LABEL_PREFIX}-seed" $(for i in $(seq 1 12); do echo "${LABEL_PREFIX}-n${i}"; done); do
    local name="${label##*-}"
    local plist="$HOME/Library/LaunchAgents/${label}.plist"

    if [[ ! -f "$plist" ]]; then
      continue
    fi

    # Check if loaded/running
    local pid=""
    pid=$(launchctl list 2>/dev/null | grep "$label" | awk '{print $1}' || true)
    [[ "$pid" == "-" || "$pid" == "0" ]] && pid=""

    # Determine RPC port from name
    local rpc_port
    if [[ "$name" == "seed" ]]; then
      rpc_port=8500
    else
      local n="${name#n}"
      rpc_port=$((8500 + n))
    fi

    # Try RPC
    local height="-" slot="-" version="-" status=""
    local info
    info=$(curl -sf --max-time 2 -X POST "http://127.0.0.1:${rpc_port}" \
      -H "Content-Type: application/json" \
      -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' 2>/dev/null || echo "")

    if [[ -n "$info" ]] && echo "$info" | grep -q "result"; then
      height=$(echo "$info" | python3 -c "import sys,json; print(json.load(sys.stdin).get('result',{}).get('bestHeight','?'))" 2>/dev/null || echo "?")
      slot=$(echo "$info" | python3 -c "import sys,json; print(json.load(sys.stdin).get('result',{}).get('bestSlot','?'))" 2>/dev/null || echo "?")
      version=$(echo "$info" | python3 -c "import sys,json; print(json.load(sys.stdin).get('result',{}).get('version','?'))" 2>/dev/null || echo "?")
      status="${GREEN}Running${NC}"
    elif [[ -n "$pid" && "$pid" != "0" && "$pid" != "-" ]]; then
      status="${YELLOW}Starting${NC}"
      pid="$pid"
    else
      status="${RED}Stopped${NC}"
      pid="-"
    fi

    printf "%-8s %6s %8s %8s %10s " "$name" "${pid:-"-"}" "$height" "$slot" "$version"
    echo -e "$status"
  done

  # Swap bot status
  local swap_plist="$HOME/Library/LaunchAgents/network.doli.swap.plist"
  if [[ -f "$swap_plist" ]]; then
    local swap_pid=""
    swap_pid=$(launchctl list 2>/dev/null | grep "network.doli.swap" | awk '{print $1}' || true)
    [[ "$swap_pid" == "-" || "$swap_pid" == "0" ]] && swap_pid=""

    local swap_status=""
    if curl -sf --max-time 2 "http://127.0.0.1:3000" >/dev/null 2>&1; then
      swap_status="${GREEN}Running${NC}"
    elif [[ -n "$swap_pid" ]]; then
      swap_status="${YELLOW}Starting${NC}"
    else
      swap_status="${RED}Stopped${NC}"
      swap_pid="-"
    fi
    printf "%-8s %6s %8s %8s %10s " "swap" "${swap_pid:-"-"}" "-" "-" ":3000"
    echo -e "$swap_status"
  fi

  # Explorer status
  local exp_plist="$HOME/Library/LaunchAgents/network.doli.explorer.plist"
  if [[ -f "$exp_plist" ]]; then
    local exp_pid=""
    exp_pid=$(launchctl list 2>/dev/null | grep "network.doli.explorer" | awk '{print $1}' || true)
    [[ "$exp_pid" == "-" || "$exp_pid" == "0" ]] && exp_pid=""

    local exp_status=""
    if curl -sf --max-time 2 "http://127.0.0.1:8080" >/dev/null 2>&1; then
      exp_status="${GREEN}Running${NC}"
    elif [[ -n "$exp_pid" ]]; then
      exp_status="${YELLOW}Starting${NC}"
    else
      exp_status="${RED}Stopped${NC}"
      exp_pid="-"
    fi
    printf "%-8s %6s %8s %8s %10s " "explorer" "${exp_pid:-"-"}" "-" "-" ":8080"
    echo -e "$exp_status"
  fi
  echo ""
}

do_logs() {
  local target="${1:-seed}"
  local logfile

  case "$target" in
    seed) logfile="$LOG_DIR/seed.log" ;;
    swap) logfile="$TESTNET_DIR/doli-swap-bot/stdout.log" ;;
    explorer) logfile="$LOG_DIR/explorer.log" ;;
    n[0-9]|n[0-9][0-9]) logfile="$LOG_DIR/${target}.log" ;;
    [0-9]|[0-9][0-9]) logfile="$LOG_DIR/n${target}.log" ;;
    *) echo "Usage: $0 logs [seed|n1|n2|...|swap]"; exit 1 ;;
  esac

  if [[ ! -f "$logfile" ]]; then
    echo "Log file not found: $logfile"
    exit 1
  fi
  tail -f "$logfile"
}

# Main
ACTION="${1:-status}"
shift || true

case "$ACTION" in
  start)   do_start "$@" ;;
  stop)    do_stop "$@" ;;
  restart) do_restart "$@" ;;
  status)  do_status ;;
  logs)    do_logs "$@" ;;
  *)
    echo "Usage: $0 {start|stop|restart|status|logs} [targets...]"
    echo ""
    echo "Targets: seed, n1-n12, all"
    exit 1
    ;;
esac

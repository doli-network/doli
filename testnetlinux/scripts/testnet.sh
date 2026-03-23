#!/usr/bin/env bash
# testnet.sh — Manage local testnet systemd user services (Linux)
#
# Linux equivalent of the macOS testnet.sh (launchctl).
# Uses systemctl --user — no sudo required.
#
# Usage:
#   testnetlinux/scripts/testnet.sh start [seed|n1|n2|...|all]   Start services
#   testnetlinux/scripts/testnet.sh stop [seed|n1|n2|...|all]     Stop services
#   testnetlinux/scripts/testnet.sh restart [seed|n1|n2|...|all]  Restart services
#   testnetlinux/scripts/testnet.sh status                        Show all service status
#   testnetlinux/scripts/testnet.sh logs [seed|n1|n2|...]         Tail logs
#   testnetlinux/scripts/testnet.sh enable [seed|n1|...|all]      Auto-start on boot
#   testnetlinux/scripts/testnet.sh disable [seed|n1|...|all]     Disable auto-start
#
# Examples:
#   testnetlinux/scripts/testnet.sh start all          # Start seed + all producers + explorer
#   testnetlinux/scripts/testnet.sh start seed         # Start seed only
#   testnetlinux/scripts/testnet.sh start n1 n5 n12    # Start specific producers
#   testnetlinux/scripts/testnet.sh stop all           # Stop everything
#   testnetlinux/scripts/testnet.sh status             # Show status of all services
#   testnetlinux/scripts/testnet.sh logs n1            # Tail n1 log
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TESTNET_DIR="$(dirname "$SCRIPT_DIR")"
LOG_DIR="$TESTNET_DIR/logs"
SERVICE_PREFIX="doli-testnet"

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# Resolve node names to systemd service names
resolve_targets() {
  local targets=()
  for arg in "$@"; do
    case "$arg" in
      all)
        targets+=("${SERVICE_PREFIX}-seed")
        for i in $(seq 1 12); do targets+=("${SERVICE_PREFIX}-n${i}"); done
        targets+=("doli-explorer")
        ;;
      seed)
        targets+=("${SERVICE_PREFIX}-seed")
        ;;
      swap)
        targets+=("doli-swap")
        ;;
      explorer)
        targets+=("doli-explorer")
        ;;
      n[0-9]|n[0-9][0-9])
        targets+=("${SERVICE_PREFIX}-${arg}")
        ;;
      [0-9]|[0-9][0-9])
        targets+=("${SERVICE_PREFIX}-n${arg}")
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
  for service in "${targets[@]}"; do
    if [[ "$service" == *"-seed" ]]; then
      local unit="${service}.service"
      if ! systemctl --user cat "$unit" &>/dev/null; then
        echo -e "  ${RED}Not installed:${NC} $service"
        continue
      fi
      echo -n "  Starting $service... "
      systemctl --user start "$unit"
      echo -e "${GREEN}OK${NC}"
      # Give seed time to initialize before producers
      if [[ ${#targets[@]} -gt 1 ]]; then
        echo "  Waiting 5s for seed to initialize..."
        sleep 5
      fi
    fi
  done

  # Then start producers + explorer
  for service in "${targets[@]}"; do
    [[ "$service" == *"-seed" ]] && continue
    local unit="${service}.service"
    if ! systemctl --user cat "$unit" &>/dev/null; then
      echo -e "  ${RED}Not installed:${NC} $service"
      continue
    fi
    echo -n "  Starting $service... "
    systemctl --user start "$unit"
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

  # Stop producers + explorer first, then seed
  for service in "${targets[@]}"; do
    [[ "$service" == *"-seed" ]] && continue
    echo -n "  Stopping $service... "
    systemctl --user stop "${service}.service" 2>/dev/null || true
    echo -e "${GREEN}OK${NC}"
  done

  for service in "${targets[@]}"; do
    [[ "$service" != *"-seed" ]] && continue
    echo -n "  Stopping $service... "
    systemctl --user stop "${service}.service" 2>/dev/null || true
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

  for service in "${SERVICE_PREFIX}-seed" $(for i in $(seq 1 12); do echo "${SERVICE_PREFIX}-n${i}"; done); do
    local name="${service##*-}"
    local unit="${service}.service"

    # Check if unit file exists
    if ! systemctl --user cat "$unit" &>/dev/null; then
      continue
    fi

    # Check if running and get PID
    local pid=""
    pid=$(systemctl --user show -p MainPID --value "$unit" 2>/dev/null || echo "0")
    [[ "$pid" == "0" ]] && pid=""

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
    elif [[ -n "$pid" ]]; then
      status="${YELLOW}Starting${NC}"
    else
      status="${RED}Stopped${NC}"
      pid="-"
    fi

    printf "%-8s %6s %8s %8s %10s " "$name" "${pid:-"-"}" "$height" "$slot" "$version"
    echo -e "$status"
  done

  # Explorer status
  local exp_unit="doli-explorer.service"
  if systemctl --user cat "$exp_unit" &>/dev/null; then
    local exp_pid=""
    exp_pid=$(systemctl --user show -p MainPID --value "$exp_unit" 2>/dev/null || echo "0")
    [[ "$exp_pid" == "0" ]] && exp_pid=""

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
    *) echo "Usage: $0 logs [seed|n1|n2|...|explorer|swap]"; exit 1 ;;
  esac

  if [[ ! -f "$logfile" ]]; then
    echo "Log file not found: $logfile"
    exit 1
  fi
  tail -f "$logfile"
}

do_enable() {
  if [[ $# -eq 0 ]]; then
    echo "Usage: $0 enable [seed|n1|n2|...|all]"
    exit 1
  fi
  local targets
  targets=($(resolve_targets "$@"))
  for service in "${targets[@]}"; do
    echo -n "  Enabling $service... "
    systemctl --user enable "${service}.service" 2>/dev/null || true
    echo -e "${GREEN}OK${NC}"
  done
}

do_disable() {
  if [[ $# -eq 0 ]]; then
    echo "Usage: $0 disable [seed|n1|n2|...|all]"
    exit 1
  fi
  local targets
  targets=($(resolve_targets "$@"))
  for service in "${targets[@]}"; do
    echo -n "  Disabling $service... "
    systemctl --user disable "${service}.service" 2>/dev/null || true
    echo -e "${GREEN}OK${NC}"
  done
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
  enable)  do_enable "$@" ;;
  disable) do_disable "$@" ;;
  *)
    echo "Usage: $0 {start|stop|restart|status|logs|enable|disable} [targets...]"
    echo ""
    echo "Targets: seed, n1-n12, explorer, swap, all"
    exit 1
    ;;
esac

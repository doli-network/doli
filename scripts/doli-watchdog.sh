#!/usr/bin/env bash
# doli-watchdog.sh — Health check for doli-node services
#
# Checks RPC liveness for all doli-node services on the local machine.
# If a node's RPC doesn't respond within 5s, restarts its systemd service.
# A unit that became active less than DOLI_WATCHDOG_COLD_START_GRACE seconds
# ago (default 600) is skipped, not restarted.
# Designed to run via systemd timer every 2 minutes.
#
# Install: scripts/install-services.sh installs this + timer on all servers.
set -euo pipefail

TIMEOUT=5
DEFAULT_GRACE=600
GRACE="$DEFAULT_GRACE"

# Integer seconds since boot; prints nothing if /proc/uptime is unreadable.
monotonic_now_s() {
  local up=""
  [[ -r /proc/uptime ]] || return 0
  read -r up _ < /proc/uptime 2>/dev/null || return 0
  printf '%s\n' "${up%%.*}"
}

# Values are regex-checked before any arithmetic: bash arithmetic runs
# command substitutions found in array subscripts.
read_grace() {
  GRACE="$DEFAULT_GRACE"
  [[ -n "${DOLI_WATCHDOG_COLD_START_GRACE+x}" ]] || return 0
  local v="$DOLI_WATCHDOG_COLD_START_GRACE"
  if [[ "$v" =~ ^[0-9]{1,9}$ ]]; then
    GRACE=$((10#$v))
  else
    logger -t doli-watchdog "invalid DOLI_WATCHDOG_COLD_START_GRACE; using default ${DEFAULT_GRACE}s"
  fi
}

check_and_restart() {
  local service="$1" port="$2"

  # Only check services that are supposed to be running
  if ! systemctl is-active --quiet "$service" 2>/dev/null; then
    return
  fi

  if curl -sf --max-time "$TIMEOUT" "http://127.0.0.1:${port}" \
    -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' > /dev/null 2>&1; then
    return
  fi

  local start now age=-1
  start="$(systemctl show -p ActiveEnterTimestampMonotonic --value "$service" 2>/dev/null || true)"
  now="$(monotonic_now_s || true)"
  if [[ "$start" =~ ^[1-9][0-9]{0,17}$ && "$now" =~ ^[0-9]{1,18}$ ]]; then
    age=$((10#$now - 10#$start / 1000000))
  fi

  if ((age < 0)); then
    logger -t doli-watchdog "start time unreadable for ${service}; restarting"
  elif ((age < GRACE)); then
    logger -t doli-watchdog "SKIP ${service} — started ${age}s ago < grace ${GRACE}s"
    return
  fi

  logger -t doli-watchdog "RESTART ${service} — RPC port ${port} unresponsive"
  systemctl restart "$service"
}

main() {
  read_grace

# Discover which doli services are enabled on this machine
# Mainnet producers N1-N12
for N in $(seq 1 12); do
  if systemctl is-enabled --quiet "doli-mainnet-n${N}" 2>/dev/null; then
    check_and_restart "doli-mainnet-n${N}" "$((8500 + N))"
  fi
done

# Testnet producers NT1-NT12
for N in $(seq 1 12); do
  if systemctl is-enabled --quiet "doli-testnet-nt${N}" 2>/dev/null; then
    check_and_restart "doli-testnet-nt${N}" "$((18500 + N))"
  fi
done

# Seeds
if systemctl is-enabled --quiet "doli-mainnet-seed" 2>/dev/null; then
  check_and_restart "doli-mainnet-seed" 8500
fi
if systemctl is-enabled --quiet "doli-testnet-seed" 2>/dev/null; then
  check_and_restart "doli-testnet-seed" 18500
fi

# Santiago (ai3)
if systemctl is-enabled --quiet "doli-mainnet-santiago" 2>/dev/null; then
  check_and_restart "doli-mainnet-santiago" 8513
fi

# Ivan (ai3)
if systemctl is-enabled --quiet "doli-mainnet-ivan" 2>/dev/null; then
  check_and_restart "doli-mainnet-ivan" 8514
fi
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then main "$@"; fi

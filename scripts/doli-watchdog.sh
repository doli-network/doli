#!/usr/bin/env bash
# doli-watchdog.sh — Health check for doli-node services
#
# Checks RPC liveness for all doli-node services on the local machine.
# If a node's RPC doesn't respond within 5s, restarts its systemd service.
# Designed to run via systemd timer every 2 minutes.
#
# Install: scripts/install-services.sh installs this + timer on all servers.
set -euo pipefail

TIMEOUT=5

check_and_restart() {
  local service="$1" port="$2"

  # Only check services that are supposed to be running
  if ! systemctl is-active --quiet "$service" 2>/dev/null; then
    return
  fi

  if ! curl -sf --max-time "$TIMEOUT" "http://127.0.0.1:${port}" \
    -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' > /dev/null 2>&1; then
    logger -t doli-watchdog "RESTART ${service} — RPC port ${port} unresponsive"
    systemctl restart "$service"
  fi
}

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

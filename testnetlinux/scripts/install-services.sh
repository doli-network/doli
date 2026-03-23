#!/usr/bin/env bash
# install-services.sh — Create systemd user services for local testnet nodes (Linux)
#
# Linux equivalent of the macOS install-local-services.sh (launchd plists).
# Uses systemd user services in ~/.config/systemd/user/ — no sudo required.
#
# Creates services for:
#   - doli-testnet-seed (relay + archive)
#   - doli-testnet-n1 through doli-testnet-n12 (producers)
#   - doli-explorer (block explorer on :8080)
#
# Usage:
#   testnetlinux/scripts/install-services.sh          # Install all (seed + n1-n12 + explorer)
#   testnetlinux/scripts/install-services.sh seed     # Install seed only
#   testnetlinux/scripts/install-services.sh explorer  # Install explorer only
#   testnetlinux/scripts/install-services.sh 1 5      # Install n1 through n5
#
# Management:
#   testnetlinux/scripts/testnet.sh start|stop|status [seed|n1|n2|...|all]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TESTNET_DIR="$(dirname "$SCRIPT_DIR")"
DOLI_REPO="$HOME/repos/doli"
NODE_BIN="$DOLI_REPO/target/release/doli-node"
SYSTEMD_USER_DIR="$HOME/.config/systemd/user"
LOG_DIR="$TESTNET_DIR/logs"

# Port scheme (matches macOS local testnet)
SEED_P2P=30300  SEED_RPC=8500  SEED_METRICS=9000

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

mkdir -p "$SYSTEMD_USER_DIR" "$LOG_DIR"

# Check binary exists
if [[ ! -f "$NODE_BIN" ]]; then
  echo -e "${RED}doli-node binary not found at ${NODE_BIN}${NC}"
  echo "Run: cargo build --release -p doli-node"
  exit 1
fi

# Enable lingering so user services survive logout
if command -v loginctl &>/dev/null; then
  if ! loginctl show-user "$USER" 2>/dev/null | grep -q "Linger=yes"; then
    echo -e "${YELLOW}Enabling linger for $USER (services persist after logout)...${NC}"
    loginctl enable-linger "$USER" 2>/dev/null || echo -e "${YELLOW}Warning: could not enable linger (may need sudo loginctl enable-linger $USER)${NC}"
  fi
fi

install_seed() {
  local service="$SYSTEMD_USER_DIR/doli-testnet-seed.service"
  mkdir -p "$TESTNET_DIR/seed/data" "$TESTNET_DIR/seed/blocks"

  cat > "$service" << EOF
[Unit]
Description=Doli Testnet Seed (Relay + Archive)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=${NODE_BIN} \\
  --network testnet \\
  --data-dir ${TESTNET_DIR}/seed/data \\
  run \\
  --relay-server \\
  --p2p-port ${SEED_P2P} \\
  --rpc-port ${SEED_RPC} \\
  --metrics-port ${SEED_METRICS} \\
  --archive-to ${TESTNET_DIR}/seed/blocks \\
  --yes
Restart=on-failure
RestartSec=10
StandardOutput=append:${LOG_DIR}/seed.log
StandardError=append:${LOG_DIR}/seed.log
LimitNOFILE=65535

[Install]
WantedBy=default.target
EOF
  echo -e "  ${GREEN}Installed${NC} seed → $service"
}

install_producer() {
  local n="$1"
  local p2p=$((SEED_P2P + n))
  local rpc=$((SEED_RPC + n))
  local metrics=$((SEED_METRICS + n))
  local service="$SYSTEMD_USER_DIR/doli-testnet-n${n}.service"

  mkdir -p "$TESTNET_DIR/n${n}/data"

  cat > "$service" << EOF
[Unit]
Description=Doli Testnet Producer N${n}
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=${NODE_BIN} \\
  --network testnet \\
  --data-dir ${TESTNET_DIR}/n${n}/data \\
  run \\
  --producer \\
  --producer-key ${TESTNET_DIR}/keys/producer_${n}.json \\
  --p2p-port ${p2p} \\
  --rpc-port ${rpc} \\
  --rpc-bind 127.0.0.1 \\
  --metrics-port ${metrics} \\
  --bootstrap /ip4/127.0.0.1/tcp/${SEED_P2P} \\
  --yes \\
  --force-start
Restart=on-failure
RestartSec=10
StandardOutput=append:${LOG_DIR}/n${n}.log
StandardError=append:${LOG_DIR}/n${n}.log
LimitNOFILE=65535

[Install]
WantedBy=default.target
EOF
  echo -e "  ${GREEN}Installed${NC} n${n} → $service (P2P:${p2p} RPC:${rpc})"
}

install_explorer() {
  local service="$SYSTEMD_USER_DIR/doli-explorer.service"
  local explorer_dir="$TESTNET_DIR/explorer"

  if ! command -v node &>/dev/null; then
    echo -e "  ${YELLOW}Warning: node not found — explorer service requires Node.js${NC}"
  fi

  cat > "$service" << EOF
[Unit]
Description=Doli Block Explorer (HTTP :8080)
After=network-online.target

[Service]
Type=simple
WorkingDirectory=${explorer_dir}
ExecStart=$(command -v node 2>/dev/null || echo /usr/bin/node) ${explorer_dir}/server.js
Restart=on-failure
RestartSec=5
StandardOutput=append:${LOG_DIR}/explorer.log
StandardError=append:${LOG_DIR}/explorer.log

[Install]
WantedBy=default.target
EOF
  echo -e "  ${GREEN}Installed${NC} explorer → $service (:8080)"
}

# Parse args
if [[ "${1:-}" == "seed" ]]; then
  echo "Installing seed service..."
  install_seed
elif [[ "${1:-}" == "explorer" ]]; then
  echo "Installing explorer service..."
  install_explorer
elif [[ -n "${1:-}" && -n "${2:-}" ]]; then
  echo "Installing producer services n${1} through n${2}..."
  for ((i=$1; i<=$2; i++)); do
    install_producer "$i"
  done
else
  echo "Installing all services (seed + n1-n12 + explorer)..."
  install_seed
  for i in $(seq 1 12); do
    install_producer "$i"
  done
  install_explorer
fi

# Reload systemd user daemon
systemctl --user daemon-reload
echo ""
echo -e "${GREEN}Done.${NC} Manage with:"
echo "  testnetlinux/scripts/testnet.sh start|stop|status [seed|n1|n2|...|all]"
echo ""
echo "Port layout:"
echo "  Seed:     P2P=${SEED_P2P}  RPC=${SEED_RPC}  Metrics=${SEED_METRICS}"
echo "  N{i}:     P2P=$((SEED_P2P))+i  RPC=$((SEED_RPC))+i  Metrics=${SEED_METRICS}+i"
echo "  Explorer: HTTP=8080"
echo ""
echo "Logs: ${LOG_DIR}/"

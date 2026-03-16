#!/usr/bin/env bash
# install-local-services.sh — Create launchd plists for local testnet nodes
#
# Creates LaunchAgents for:
#   - doli-testnet-seed (relay + archive)
#   - doli-testnet-n1 through doli-testnet-n12 (producers)
#
# Usage:
#   scripts/install-local-services.sh          # Install all (seed + n1-n12)
#   scripts/install-local-services.sh seed     # Install seed only
#   scripts/install-local-services.sh 1 5      # Install n1 through n5
#
# Management:
#   scripts/testnet.sh start|stop|status [seed|n1|n2|...|all]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
TESTNET_DIR="$HOME/testnet"
NODE_BIN="$PROJECT_ROOT/target/release/doli-node"
LAUNCH_AGENTS_DIR="$HOME/Library/LaunchAgents"
LOG_DIR="$TESTNET_DIR/logs"

# Port scheme (matches remote servers)
SEED_P2P=30300  SEED_RPC=8500  SEED_METRICS=9000

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

mkdir -p "$LAUNCH_AGENTS_DIR" "$LOG_DIR"

# Check binary exists
if [[ ! -f "$NODE_BIN" ]]; then
  echo -e "${RED}doli-node binary not found at ${NODE_BIN}${NC}"
  echo "Run: cargo build --release -p doli-node"
  exit 1
fi

install_seed() {
  local plist="$LAUNCH_AGENTS_DIR/network.doli.testnet-seed.plist"
  mkdir -p "$TESTNET_DIR/seed/data" "$TESTNET_DIR/seed/blocks"

  cat > "$plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>network.doli.testnet-seed</string>
    <key>ProgramArguments</key>
    <array>
        <string>${NODE_BIN}</string>
        <string>--network</string>
        <string>testnet</string>
        <string>--data-dir</string>
        <string>${TESTNET_DIR}/seed/data</string>
        <string>run</string>
        <string>--relay-server</string>
        <string>--p2p-port</string>
        <string>${SEED_P2P}</string>
        <string>--rpc-port</string>
        <string>${SEED_RPC}</string>
        <string>--metrics-port</string>
        <string>${SEED_METRICS}</string>
        <string>--archive-to</string>
        <string>${TESTNET_DIR}/seed/blocks</string>
        <string>--yes</string>
        <string>--no-snap-sync</string>
    </array>
    <key>RunAtLoad</key>
    <false/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardOutPath</key>
    <string>${LOG_DIR}/seed.log</string>
    <key>StandardErrorPath</key>
    <string>${LOG_DIR}/seed.log</string>
    <key>SoftResourceLimits</key>
    <dict>
        <key>NumberOfFiles</key>
        <integer>65535</integer>
    </dict>
</dict>
</plist>
EOF
  echo -e "  ${GREEN}Installed${NC} seed → $plist"
}

install_producer() {
  local n="$1"
  local p2p=$((SEED_P2P + n))
  local rpc=$((SEED_RPC + n))
  local metrics=$((SEED_METRICS + n))
  local plist="$LAUNCH_AGENTS_DIR/network.doli.testnet-n${n}.plist"

  mkdir -p "$TESTNET_DIR/n${n}/data"

  cat > "$plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>network.doli.testnet-n${n}</string>
    <key>ProgramArguments</key>
    <array>
        <string>${NODE_BIN}</string>
        <string>--network</string>
        <string>testnet</string>
        <string>--data-dir</string>
        <string>${TESTNET_DIR}/n${n}/data</string>
        <string>run</string>
        <string>--producer</string>
        <string>--producer-key</string>
        <string>${TESTNET_DIR}/keys/producer_${n}.json</string>
        <string>--p2p-port</string>
        <string>${p2p}</string>
        <string>--rpc-port</string>
        <string>${rpc}</string>
        <string>--rpc-bind</string>
        <string>127.0.0.1</string>
        <string>--metrics-port</string>
        <string>${metrics}</string>
        <string>--bootstrap</string>
        <string>/ip4/127.0.0.1/tcp/${SEED_P2P}</string>
        <string>--yes</string>
        <string>--force-start</string>
    </array>
    <key>RunAtLoad</key>
    <false/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardOutPath</key>
    <string>${LOG_DIR}/n${n}.log</string>
    <key>StandardErrorPath</key>
    <string>${LOG_DIR}/n${n}.log</string>
    <key>SoftResourceLimits</key>
    <dict>
        <key>NumberOfFiles</key>
        <integer>65535</integer>
    </dict>
</dict>
</plist>
EOF
  echo -e "  ${GREEN}Installed${NC} n${n} → $plist (P2P:${p2p} RPC:${rpc})"
}

# Parse args
if [[ "${1:-}" == "seed" ]]; then
  echo "Installing seed service..."
  install_seed
elif [[ -n "${1:-}" && -n "${2:-}" ]]; then
  echo "Installing producer services n${1} through n${2}..."
  for ((i=$1; i<=$2; i++)); do
    install_producer "$i"
  done
else
  echo "Installing all services (seed + n1-n12)..."
  install_seed
  for i in $(seq 1 12); do
    install_producer "$i"
  done
fi

echo ""
echo -e "${GREEN}Done.${NC} Manage with:"
echo "  scripts/testnet.sh start|stop|status [seed|n1|n2|...|all]"
echo ""
echo "Port layout:"
echo "  Seed: P2P=${SEED_P2P}  RPC=${SEED_RPC}  Metrics=${SEED_METRICS}"
echo "  N{i}: P2P=$((SEED_P2P))+i  RPC=$((SEED_RPC))+i  Metrics=${SEED_METRICS}+i"
echo ""
echo "Logs: ${LOG_DIR}/"

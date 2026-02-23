#!/usr/bin/env bash
# Create a DOLI node system service (auto-detects macOS vs Linux).
#
# Usage: ./create-service.sh [OPTIONS]
#   --network    mainnet|testnet|devnet  (default: mainnet)
#   --producer   Enable block production
#   --key        Path to producer key file
#   --data-dir   Data directory (default: ~/.doli/<network>/data)
#   --binary     Path to doli-node binary (default: auto-detect)
#   --name       Service name suffix (default: network name)
#   --port-offset N  Add N to default ports (for multi-node on same host)
#
# Examples:
#   ./create-service.sh --network mainnet --producer --key ~/.doli/mainnet/keys/producer.json
#   ./create-service.sh --network testnet
#   ./create-service.sh --network mainnet --producer --key keys/p2.json --port-offset 1 --name node2

set -euo pipefail

# Defaults
NETWORK="mainnet"
PRODUCER=false
KEY_PATH=""
DATA_DIR=""
BINARY=""
SERVICE_NAME=""
PORT_OFFSET=0

# Parse args
while [[ $# -gt 0 ]]; do
    case $1 in
        --network)    NETWORK="$2"; shift 2 ;;
        --producer)   PRODUCER=true; shift ;;
        --key)        KEY_PATH="$2"; shift 2 ;;
        --data-dir)   DATA_DIR="$2"; shift 2 ;;
        --binary)     BINARY="$2"; shift 2 ;;
        --name)       SERVICE_NAME="$2"; shift 2 ;;
        --port-offset) PORT_OFFSET="$2"; shift 2 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Detect OS
OS=$(uname -s)
USER_NAME=$(whoami)
HOME_DIR=$(eval echo "~${USER_NAME}")

# Set defaults based on network
case "$NETWORK" in
    mainnet) P2P=30303; RPC=8545; METRICS=9090 ;;
    testnet) P2P=40303; RPC=18545; METRICS=19090 ;;
    devnet)  P2P=50303; RPC=28545; METRICS=29090 ;;
    *) echo "Unknown network: $NETWORK"; exit 1 ;;
esac

# Apply port offset
P2P=$((P2P + PORT_OFFSET))
RPC=$((RPC + PORT_OFFSET))
METRICS=$((METRICS + PORT_OFFSET))

# Defaults
DATA_DIR="${DATA_DIR:-${HOME_DIR}/.doli/${NETWORK}/data}"
SERVICE_NAME="${SERVICE_NAME:-${NETWORK}}"

# Find binary
if [[ -z "$BINARY" ]]; then
    if [[ -f "${HOME_DIR}/repos/doli/target/release/doli-node" ]]; then
        BINARY="${HOME_DIR}/repos/doli/target/release/doli-node"
    elif [[ -f "/opt/doli/target/release/doli-node" ]]; then
        BINARY="/opt/doli/target/release/doli-node"
    elif command -v doli-node &>/dev/null; then
        BINARY=$(command -v doli-node)
    else
        echo "Error: Cannot find doli-node binary. Use --binary to specify."
        exit 1
    fi
fi

# Build command args
CMD_ARGS="--data-dir ${DATA_DIR} run"
if [[ "$PRODUCER" == true ]]; then
    if [[ -z "$KEY_PATH" ]]; then
        echo "Error: --key required when --producer is set"
        exit 1
    fi
    CMD_ARGS="${CMD_ARGS} --producer --producer-key ${KEY_PATH} --force-start"
fi
CMD_ARGS="${CMD_ARGS} --p2p-port ${P2P} --rpc-port ${RPC} --metrics-port ${METRICS}"
CMD_ARGS="${CMD_ARGS} --no-auto-update --yes"

if [[ "$OS" == "Linux" ]]; then
    # ==================== systemd ====================
    SERVICE_FILE="/etc/systemd/system/doli-${SERVICE_NAME}.service"

    echo "Creating systemd service: doli-${SERVICE_NAME}"
    echo "  Binary:   ${BINARY}"
    echo "  Data dir: ${DATA_DIR}"
    echo "  Ports:    P2P=${P2P} RPC=${RPC} Metrics=${METRICS}"
    echo "  Producer: ${PRODUCER}"
    echo ""

    cat > /tmp/doli-${SERVICE_NAME}.service << UNIT
[Unit]
Description=DOLI ${NETWORK} Node (${SERVICE_NAME})
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=${USER_NAME}
ExecStart=${BINARY} ${CMD_ARGS}
Restart=on-failure
RestartSec=10
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
UNIT

    echo "Service file written to /tmp/doli-${SERVICE_NAME}.service"
    echo ""
    echo "To install:"
    echo "  sudo cp /tmp/doli-${SERVICE_NAME}.service ${SERVICE_FILE}"
    echo "  sudo systemctl daemon-reload"
    echo "  sudo systemctl enable doli-${SERVICE_NAME}"
    echo "  sudo systemctl start doli-${SERVICE_NAME}"

elif [[ "$OS" == "Darwin" ]]; then
    # ==================== launchd ====================
    PLIST_NAME="network.doli.${SERVICE_NAME}"
    PLIST_FILE="${HOME_DIR}/Library/LaunchAgents/${PLIST_NAME}.plist"
    LOG_DIR="${HOME_DIR}/.doli/${NETWORK}"

    echo "Creating launchd service: ${PLIST_NAME}"
    echo "  Binary:   ${BINARY}"
    echo "  Data dir: ${DATA_DIR}"
    echo "  Ports:    P2P=${P2P} RPC=${RPC} Metrics=${METRICS}"
    echo "  Producer: ${PRODUCER}"
    echo ""

    mkdir -p "${HOME_DIR}/Library/LaunchAgents"
    mkdir -p "${LOG_DIR}"

    # Build ProgramArguments array
    ARGS_XML="        <string>${BINARY}</string>"
    ARGS_XML="${ARGS_XML}\n        <string>--data-dir</string>\n        <string>${DATA_DIR}</string>"
    ARGS_XML="${ARGS_XML}\n        <string>run</string>"

    if [[ "$PRODUCER" == true ]]; then
        ARGS_XML="${ARGS_XML}\n        <string>--producer</string>"
        ARGS_XML="${ARGS_XML}\n        <string>--producer-key</string>\n        <string>${KEY_PATH}</string>"
        ARGS_XML="${ARGS_XML}\n        <string>--force-start</string>"
    fi

    ARGS_XML="${ARGS_XML}\n        <string>--p2p-port</string>\n        <string>${P2P}</string>"
    ARGS_XML="${ARGS_XML}\n        <string>--rpc-port</string>\n        <string>${RPC}</string>"
    ARGS_XML="${ARGS_XML}\n        <string>--metrics-port</string>\n        <string>${METRICS}</string>"
    ARGS_XML="${ARGS_XML}\n        <string>--no-auto-update</string>"
    ARGS_XML="${ARGS_XML}\n        <string>--yes</string>"

    cat > "${PLIST_FILE}" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>${PLIST_NAME}</string>
    <key>ProgramArguments</key>
    <array>
$(echo -e "${ARGS_XML}")
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardOutPath</key>
    <string>${LOG_DIR}/node.log</string>
    <key>StandardErrorPath</key>
    <string>${LOG_DIR}/node.err</string>
    <key>ThrottleInterval</key>
    <integer>10</integer>
</dict>
</plist>
PLIST

    echo "Plist written to ${PLIST_FILE}"
    echo ""
    echo "To start:"
    echo "  launchctl load ${PLIST_FILE}"
    echo "  launchctl start ${PLIST_NAME}"
    echo ""
    echo "To stop:"
    echo "  launchctl stop ${PLIST_NAME}"
    echo "  launchctl unload ${PLIST_FILE}"

else
    echo "Unsupported OS: ${OS}"
    exit 1
fi

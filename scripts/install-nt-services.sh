#!/usr/bin/env bash
# install-nt-services.sh — Generate and install systemd services for NT test nodes
#
# DEPRECATED: Use scripts/install-services.sh instead (handles all networks + ai3 seeds).
# This script is kept for ad-hoc NT node creation on a single server.
#
# Usage (run ON the target server):
#   sudo ./install-nt-services.sh <binary_path> <offset> <count> <user>
#
# Example:
#   sudo ./install-nt-services.sh /testnet/bin/doli-node 0 6 ilozada
#
# This creates services: doli-testnet-nt14 through doli-testnet-nt18
# Managed via: sudo systemctl restart doli-testnet-nt18

set -euo pipefail

BINARY="${1:?Usage: $0 <binary_path> <offset> <count> <user>}"
OFFSET="${2:?}"
COUNT="${3:?}"
USER="${4:?}"
HOME_DIR=$(eval echo "~$USER")

# Bootstrap nodes are NOT needed — mainnet defaults (seed1/seed2.doli.network)
# are embedded in the binary via NetworkParams::for_network(Mainnet).
BASE="$HOME_DIR/doli-test"

P2P_BASE=31000
RPC_BASE=9000
METRICS_BASE=9100

echo "=== Installing $COUNT NT systemd services (NT$((OFFSET+1))–NT$((OFFSET+COUNT))) ==="
echo "Binary: $BINARY"
echo "User: $USER"
echo "Base dir: $BASE"
echo ""

for i in $(seq 1 "$COUNT"); do
    ID=$((OFFSET + i))
    SERVICE="doli-testnet-nt${ID}"
    UNIT_FILE="/etc/systemd/system/${SERVICE}.service"
    DATA_DIR="$BASE/nt${ID}/data"
    KEY_FILE="$BASE/keys/nt${ID}.json"
    P2P_PORT=$((P2P_BASE + i))
    RPC_PORT=$((RPC_BASE + i))
    METRICS_PORT=$((METRICS_BASE + i))
    LOG_FILE="$BASE/nt${ID}/node.log"

    echo "--- NT${ID}: ${SERVICE} (p2p=${P2P_PORT}, rpc=${RPC_PORT}) ---"

    # Verify key file exists
    if [ ! -f "$KEY_FILE" ]; then
        echo "  ERROR: Key file not found: $KEY_FILE — skipping"
        continue
    fi

    # Ensure data dir exists
    mkdir -p "$DATA_DIR"
    chown "$USER:$USER" "$DATA_DIR"

    cat > "$UNIT_FILE" <<UNIT
[Unit]
Description=DOLI Testnet Node NT${ID}
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=${USER}
ExecStart=${BINARY} \\
    --data-dir ${DATA_DIR} run \\
    --producer --producer-key ${KEY_FILE} \\
    --yes --force-start \\
    --p2p-port ${P2P_PORT} \\
    --rpc-port ${RPC_PORT} \\
    --metrics-port ${METRICS_PORT}
Restart=on-failure
RestartSec=5
StandardOutput=append:${LOG_FILE}
StandardError=append:${LOG_FILE}

[Install]
WantedBy=multi-user.target
UNIT

    systemctl daemon-reload
    systemctl enable "$SERVICE" 2>/dev/null
    echo "  Installed: $UNIT_FILE"
done

echo ""
echo "=== Done. Services installed but NOT started. ==="
echo ""
echo "To migrate from manage.sh:"
echo "  1. Stop manage.sh nodes:  source ~/doli-test/.env && ~/doli-test/manage.sh stop"
echo "  2. Start systemd services: sudo systemctl start doli-testnet-nt{$((OFFSET+1))..$((OFFSET+COUNT))}"
echo ""
echo "Per-node control:"
echo "  sudo systemctl restart doli-testnet-nt<ID>"
echo "  sudo systemctl stop doli-testnet-nt<ID>"
echo "  journalctl -u doli-testnet-nt<ID> -f"

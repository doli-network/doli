#!/bin/bash
set -e

# DOLI Node Docker Entrypoint
# Maps environment variables to CLI arguments

# Build command arguments
ARGS=""

# Network selection
case "${DOLI_NETWORK:-mainnet}" in
    mainnet)
        ARGS="$ARGS"
        DOLI_RPC_PORT="${DOLI_RPC_PORT:-8545}"
        DOLI_P2P_PORT="${DOLI_P2P_PORT:-30303}"
        ;;
    testnet)
        ARGS="$ARGS --network testnet"
        DOLI_RPC_PORT="${DOLI_RPC_PORT:-18545}"
        DOLI_P2P_PORT="${DOLI_P2P_PORT:-40303}"
        ;;
    devnet)
        ARGS="$ARGS --network devnet"
        DOLI_RPC_PORT="${DOLI_RPC_PORT:-28545}"
        DOLI_P2P_PORT="${DOLI_P2P_PORT:-50303}"
        ;;
    *)
        echo "Error: Invalid DOLI_NETWORK value: ${DOLI_NETWORK}"
        echo "Valid options: mainnet, testnet, devnet"
        exit 1
        ;;
esac

# Export for healthcheck
export DOLI_RPC_PORT

# Data directory
if [ -n "$DOLI_DATA_DIR" ]; then
    ARGS="$ARGS --data-dir $DOLI_DATA_DIR"
fi

# Log level
if [ -n "$DOLI_LOG_LEVEL" ]; then
    export RUST_LOG="${DOLI_LOG_LEVEL}"
fi

# P2P port override
if [ -n "$DOLI_P2P_PORT" ]; then
    ARGS="$ARGS --p2p-port $DOLI_P2P_PORT"
fi

# RPC port override
if [ -n "$DOLI_RPC_PORT" ]; then
    ARGS="$ARGS --rpc-port $DOLI_RPC_PORT"
fi

# External IP for NAT traversal
if [ -n "$DOLI_EXTERNAL_IP" ]; then
    ARGS="$ARGS --external-ip $DOLI_EXTERNAL_IP"
fi

# Bootstrap nodes
if [ -n "$DOLI_BOOTSTRAP" ]; then
    ARGS="$ARGS --bootstrap $DOLI_BOOTSTRAP"
fi

# Producer mode via key file (preferred)
# Note: --producer-key accepts a file path to the key file
if [ -n "$DOLI_PRODUCER_KEY_FILE" ]; then
    ARGS="$ARGS --producer --producer-key $DOLI_PRODUCER_KEY_FILE"
fi

# Producer mode via inline key (less secure, for testing)
# If key file not set, use inline key
if [ -z "$DOLI_PRODUCER_KEY_FILE" ] && [ -n "$DOLI_PRODUCER_KEY" ]; then
    # Create temp key file from inline key
    echo "$DOLI_PRODUCER_KEY" > /tmp/producer.key
    ARGS="$ARGS --producer --producer-key /tmp/producer.key"
fi

# Disable auto-update
if [ "$DOLI_NO_AUTO_UPDATE" = "true" ]; then
    ARGS="$ARGS --no-auto-update"
fi

# Disable DHT discovery
if [ "$DOLI_NO_DHT" = "true" ]; then
    ARGS="$ARGS --no-dht"
fi

# Custom chainspec
if [ -n "$DOLI_CHAINSPEC" ]; then
    ARGS="$ARGS --chainspec $DOLI_CHAINSPEC"
fi

# Metrics port
if [ -n "$DOLI_METRICS_PORT" ]; then
    ARGS="$ARGS --metrics-port $DOLI_METRICS_PORT"
fi

echo "Starting DOLI node..."
echo "Network: ${DOLI_NETWORK:-mainnet}"
echo "Data directory: ${DOLI_DATA_DIR:-/data}"
echo "Command: doli-node $ARGS $@"

exec doli-node $ARGS "$@"

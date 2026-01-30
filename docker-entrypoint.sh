#!/bin/bash
set -e

# DOLI Node Docker Entrypoint
# Translates environment variables to CLI arguments

# Global arguments (before subcommand)
GLOBAL_ARGS=()

# Subcommand arguments (after 'run')
RUN_ARGS=()

# Network selection (default: mainnet)
if [ -n "$DOLI_NETWORK" ]; then
    GLOBAL_ARGS+=("--network" "$DOLI_NETWORK")
fi

# Data directory (default: /data)
if [ -n "$DOLI_DATA_DIR" ]; then
    GLOBAL_ARGS+=("--data-dir" "$DOLI_DATA_DIR")
fi

# Log level
if [ -n "$DOLI_LOG_LEVEL" ]; then
    GLOBAL_ARGS+=("--log-level" "$DOLI_LOG_LEVEL")
fi

# Custom P2P port
if [ -n "$DOLI_P2P_PORT" ]; then
    RUN_ARGS+=("--p2p-port" "$DOLI_P2P_PORT")
fi

# Custom RPC port
if [ -n "$DOLI_RPC_PORT" ]; then
    RUN_ARGS+=("--rpc-port" "$DOLI_RPC_PORT")
fi

# Metrics port (default: 9090)
if [ -n "$DOLI_METRICS_PORT" ]; then
    RUN_ARGS+=("--metrics-port" "$DOLI_METRICS_PORT")
fi

# Bootstrap node (single multiaddr)
if [ -n "$DOLI_BOOTSTRAP" ]; then
    RUN_ARGS+=("--bootstrap" "$DOLI_BOOTSTRAP")
fi

# Producer mode configuration
if [ -n "$DOLI_PRODUCER_KEY_FILE" ]; then
    RUN_ARGS+=("--producer" "--producer-key" "$DOLI_PRODUCER_KEY_FILE")
elif [ -n "$DOLI_PRODUCER" ] && [ "$DOLI_PRODUCER" = "true" ]; then
    RUN_ARGS+=("--producer")
fi

# Disable auto-updates
if [ -n "$DOLI_NO_AUTO_UPDATE" ] && [ "$DOLI_NO_AUTO_UPDATE" = "true" ]; then
    RUN_ARGS+=("--no-auto-update")
fi

# Disable DHT discovery
if [ -n "$DOLI_NO_DHT" ] && [ "$DOLI_NO_DHT" = "true" ]; then
    RUN_ARGS+=("--no-dht")
fi

# Chainspec file
if [ -n "$DOLI_CHAINSPEC" ]; then
    RUN_ARGS+=("--chainspec" "$DOLI_CHAINSPEC")
fi

# If first argument is a subcommand, pass everything through
if [ "$1" = "run" ] || [ "$1" = "init" ] || [ "$1" = "status" ] || [ "$1" = "import" ] || [ "$1" = "export" ] || [ "$1" = "update" ]; then
    exec doli-node "${GLOBAL_ARGS[@]}" "$@"
fi

# If first argument is a flag, assume it's for the run subcommand
if [ "${1:0:1}" = '-' ]; then
    exec doli-node "${GLOBAL_ARGS[@]}" run "${RUN_ARGS[@]}" "$@"
fi

# If first argument is doli-node, execute it directly with remaining args
if [ "$1" = "doli-node" ]; then
    shift
    exec doli-node "${GLOBAL_ARGS[@]}" "$@"
fi

# Default: run the node
if [ $# -eq 0 ]; then
    exec doli-node "${GLOBAL_ARGS[@]}" run "${RUN_ARGS[@]}"
fi

# Otherwise, execute the command as-is (for debugging, etc.)
exec "$@"

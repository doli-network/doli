#!/usr/bin/env bash
# DOLI Release Smoke Test Script
# Verifies that a release binary works correctly
#
# Usage:
#   ./scripts/smoke_test_release.sh                          # Test local build
#   ./scripts/smoke_test_release.sh /path/to/doli-node       # Test specific binary
#   ./scripts/smoke_test_release.sh --docker                 # Test Docker image

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Configuration
TIMEOUT=60
RPC_PORT=28500
P2P_PORT=50300
DATA_DIR=""
PID=""
DOCKER_MODE=false
CONTAINER_NAME="doli-smoke-test-$$"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

cleanup() {
    log_info "Cleaning up..."

    if [ "$DOCKER_MODE" = true ]; then
        docker stop "$CONTAINER_NAME" 2>/dev/null || true
        docker rm "$CONTAINER_NAME" 2>/dev/null || true
    else
        if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
            kill "$PID" 2>/dev/null || true
            wait "$PID" 2>/dev/null || true
        fi
    fi

    if [ -n "$DATA_DIR" ] && [ -d "$DATA_DIR" ]; then
        rm -rf "$DATA_DIR"
    fi
}

trap cleanup EXIT

wait_for_rpc() {
    local max_attempts=$((TIMEOUT / 2))
    local attempt=0

    log_info "Waiting for RPC to be ready (max ${TIMEOUT}s)..."

    while [ $attempt -lt $max_attempts ]; do
        if curl -sf "http://localhost:${RPC_PORT}/health" > /dev/null 2>&1; then
            log_info "RPC is ready!"
            return 0
        fi

        # Also try JSON-RPC format
        local response
        response=$(curl -sf "http://localhost:${RPC_PORT}" \
            -H "Content-Type: application/json" \
            -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' 2>/dev/null || echo "")

        if echo "$response" | grep -q "result"; then
            log_info "RPC is ready!"
            return 0
        fi

        attempt=$((attempt + 1))
        sleep 2
    done

    log_error "RPC did not become ready within ${TIMEOUT}s"
    return 1
}

check_p2p_port() {
    log_info "Checking P2P port ${P2P_PORT}..."

    if command -v nc &> /dev/null; then
        if nc -z localhost "$P2P_PORT" 2>/dev/null; then
            log_info "P2P port is listening"
            return 0
        fi
    elif command -v ss &> /dev/null; then
        if ss -tlnp | grep -q ":${P2P_PORT}"; then
            log_info "P2P port is listening"
            return 0
        fi
    fi

    log_warn "Could not verify P2P port (may still be starting)"
    return 0
}

test_rpc_methods() {
    log_info "Testing RPC methods..."

    # Test getChainInfo
    local chain_info
    chain_info=$(curl -sf "http://localhost:${RPC_PORT}" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' 2>/dev/null || echo "")

    if echo "$chain_info" | grep -q "result"; then
        log_info "getChainInfo: OK"
    else
        log_error "getChainInfo failed"
        return 1
    fi

    # Test getPeerCount
    local peer_count
    peer_count=$(curl -sf "http://localhost:${RPC_PORT}" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"getPeerCount","params":[],"id":1}' 2>/dev/null || echo "")

    if echo "$peer_count" | grep -q "result"; then
        log_info "getPeerCount: OK"
    else
        log_warn "getPeerCount: No response (may not be implemented)"
    fi

    return 0
}

run_binary_test() {
    local binary="$1"

    log_info "Testing binary: $binary"

    # Verify binary exists and is executable
    if [ ! -x "$binary" ]; then
        log_error "Binary not found or not executable: $binary"
        return 1
    fi

    # Check if it's a valid binary
    if ! file "$binary" | grep -qi "executable\|ELF"; then
        log_error "Not a valid executable: $binary"
        return 1
    fi

    # Test --help
    log_info "Testing --help..."
    if ! "$binary" --help > /dev/null 2>&1; then
        log_error "--help failed"
        return 1
    fi
    log_info "--help: OK"

    # Test --version
    log_info "Testing --version..."
    if "$binary" --version 2>&1 | grep -qi "doli\|version"; then
        log_info "--version: OK"
    else
        log_warn "--version: No version output"
    fi

    # Create temporary data directory
    DATA_DIR=$(mktemp -d)
    log_info "Using data directory: $DATA_DIR"

    # Start node in devnet mode (fastest startup)
    log_info "Starting node in devnet mode..."
    "$binary" --network devnet --data-dir "$DATA_DIR" run &
    PID=$!

    # Wait for RPC
    if ! wait_for_rpc; then
        log_error "Node failed to start"
        return 1
    fi

    # Check P2P port
    check_p2p_port

    # Test RPC methods
    if ! test_rpc_methods; then
        return 1
    fi

    # Test clean shutdown
    log_info "Testing clean shutdown..."
    kill -TERM "$PID" 2>/dev/null || true

    local shutdown_timeout=10
    local count=0
    while kill -0 "$PID" 2>/dev/null && [ $count -lt $shutdown_timeout ]; do
        sleep 1
        count=$((count + 1))
    done

    if kill -0 "$PID" 2>/dev/null; then
        log_warn "Node did not shut down cleanly, force killing..."
        kill -KILL "$PID" 2>/dev/null || true
    else
        log_info "Clean shutdown: OK"
    fi

    PID=""
    return 0
}

run_docker_test() {
    local image="${1:-doli-node:latest}"

    log_info "Testing Docker image: $image"

    # Check if image exists
    if ! docker image inspect "$image" > /dev/null 2>&1; then
        log_error "Docker image not found: $image"
        return 1
    fi

    # Start container
    log_info "Starting Docker container..."
    docker run -d \
        --name "$CONTAINER_NAME" \
        -e DOLI_NETWORK=devnet \
        -p "${RPC_PORT}:${RPC_PORT}" \
        -p "${P2P_PORT}:${P2P_PORT}" \
        "$image"

    DOCKER_MODE=true

    # Wait for RPC
    if ! wait_for_rpc; then
        log_error "Container failed to start"
        docker logs "$CONTAINER_NAME"
        return 1
    fi

    # Check P2P port
    check_p2p_port

    # Test RPC methods
    if ! test_rpc_methods; then
        return 1
    fi

    # Check container health
    log_info "Checking container health..."
    local health
    health=$(docker inspect --format='{{.State.Health.Status}}' "$CONTAINER_NAME" 2>/dev/null || echo "unknown")
    log_info "Container health: $health"

    # Test clean shutdown
    log_info "Testing container stop..."
    docker stop "$CONTAINER_NAME"
    log_info "Container stopped: OK"

    return 0
}

# Main
main() {
    log_info "=== DOLI Release Smoke Test ==="

    local binary=""

    case "${1:-}" in
        --docker)
            run_docker_test "${2:-doli-node:latest}"
            ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS] [BINARY_PATH]"
            echo ""
            echo "Options:"
            echo "  --docker [IMAGE]  Test Docker image (default: doli-node:latest)"
            echo "  --help            Show this help"
            echo ""
            echo "If no path is provided, tests target/release/doli-node"
            exit 0
            ;;
        "")
            # Default: test local release build
            binary="$PROJECT_ROOT/target/release/doli-node"
            run_binary_test "$binary"
            ;;
        *)
            # Test specified binary
            binary="$1"
            run_binary_test "$binary"
            ;;
    esac

    log_info "=== Smoke Test PASSED ==="
    exit 0
}

main "$@"

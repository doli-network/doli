#!/bin/bash
# Smoke Test Script for DOLI Release Verification
#
# This script verifies a release artifact works correctly by:
# 1. Downloading the release artifact
# 2. Verifying the checksum
# 3. Starting a node in devnet mode
# 4. Verifying RPC responds
# 5. Verifying P2P port is listening
# 6. Clean shutdown
#
# Usage:
#   ./scripts/smoke_test_release.sh [OPTIONS]
#
# Options:
#   --binary PATH        Path to pre-existing binary (skip download)
#   --version VERSION    Version tag to download (e.g., v1.0.0)
#   --url URL            Direct URL to download tarball
#   --target TARGET      Target triple (default: auto-detect)
#   --timeout SECONDS    Test timeout (default: 60)
#   --keep               Keep test directory on success
#   --help               Show this help message
#
# Exit codes:
#   0  - All tests passed
#   1  - Test failed
#   2  - Invalid arguments
#   3  - Download/checksum failed

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
BINARY_PATH=""
VERSION=""
DOWNLOAD_URL=""
TARGET=""
TIMEOUT=60
KEEP_DIR=false
TEST_DIR=""
NODE_PID=""

# Default ports for devnet
P2P_PORT=50303
RPC_PORT=28545

# Print colored message
log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[PASS]${NC} $1"; }
log_warning() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[FAIL]${NC} $1"; }

# Show usage
show_help() {
    head -30 "$0" | tail -27 | sed 's/^# //' | sed 's/^#//'
    exit 0
}

# Auto-detect platform target
detect_target() {
    local os arch
    os=$(uname -s | tr '[:upper:]' '[:lower:]')
    arch=$(uname -m)

    case "$os" in
        linux)
            case "$arch" in
                x86_64) echo "x86_64-unknown-linux-musl" ;;
                aarch64|arm64) echo "aarch64-unknown-linux-musl" ;;
                *) echo "x86_64-unknown-linux-gnu" ;;
            esac
            ;;
        darwin)
            case "$arch" in
                x86_64) echo "x86_64-apple-darwin" ;;
                aarch64|arm64) echo "aarch64-apple-darwin" ;;
                *) echo "x86_64-apple-darwin" ;;
            esac
            ;;
        *)
            log_error "Unsupported OS: $os"
            exit 2
            ;;
    esac
}

# Cleanup function
cleanup() {
    local exit_code=$?

    # Kill node if running
    if [ -n "$NODE_PID" ] && kill -0 "$NODE_PID" 2>/dev/null; then
        log_info "Stopping node (PID: $NODE_PID)..."
        kill -TERM "$NODE_PID" 2>/dev/null || true

        # Wait for graceful shutdown
        for i in {1..10}; do
            if ! kill -0 "$NODE_PID" 2>/dev/null; then
                break
            fi
            sleep 0.5
        done

        # Force kill if still running
        if kill -0 "$NODE_PID" 2>/dev/null; then
            log_warning "Force killing node..."
            kill -9 "$NODE_PID" 2>/dev/null || true
        fi
    fi

    # Clean up test directory
    if [ -n "$TEST_DIR" ] && [ -d "$TEST_DIR" ]; then
        if [ "$KEEP_DIR" = true ] || [ $exit_code -ne 0 ]; then
            log_info "Test directory preserved: $TEST_DIR"
        else
            log_info "Cleaning up test directory..."
            rm -rf "$TEST_DIR"
        fi
    fi

    exit $exit_code
}

trap cleanup EXIT INT TERM

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --binary)
            BINARY_PATH="$2"
            shift 2
            ;;
        --version)
            VERSION="$2"
            shift 2
            ;;
        --url)
            DOWNLOAD_URL="$2"
            shift 2
            ;;
        --target)
            TARGET="$2"
            shift 2
            ;;
        --timeout)
            TIMEOUT="$2"
            shift 2
            ;;
        --keep)
            KEEP_DIR=true
            shift
            ;;
        --help|-h)
            show_help
            ;;
        *)
            log_error "Unknown option: $1"
            exit 2
            ;;
    esac
done

# Auto-detect target if not specified
if [ -z "$TARGET" ]; then
    TARGET=$(detect_target)
fi

# Create test directory
TEST_DIR=$(mktemp -d /tmp/doli-smoke-test-XXXXXX)
log_info "Test directory: $TEST_DIR"

# =============================================================================
# Step 1: Get the binary
# =============================================================================
log_info "=== Step 1: Acquiring binary ==="

if [ -n "$BINARY_PATH" ]; then
    # Use provided binary
    if [ ! -f "$BINARY_PATH" ]; then
        log_error "Binary not found: $BINARY_PATH"
        exit 3
    fi
    cp "$BINARY_PATH" "$TEST_DIR/doli-node"
    chmod +x "$TEST_DIR/doli-node"
    log_success "Using provided binary: $BINARY_PATH"

elif [ -n "$DOWNLOAD_URL" ]; then
    # Download from direct URL
    log_info "Downloading from: $DOWNLOAD_URL"

    TARBALL="$TEST_DIR/doli-release.tar.gz"
    if ! curl -fsSL "$DOWNLOAD_URL" -o "$TARBALL"; then
        log_error "Failed to download: $DOWNLOAD_URL"
        exit 3
    fi

    # Extract
    cd "$TEST_DIR"
    tar -xzf "$TARBALL"

    if [ ! -f "$TEST_DIR/doli-node" ]; then
        log_error "Binary not found in tarball"
        exit 3
    fi
    chmod +x "$TEST_DIR/doli-node"
    log_success "Downloaded and extracted binary"

elif [ -n "$VERSION" ]; then
    # Download from GitHub Releases
    ARTIFACT_NAME="doli-${VERSION}-${TARGET}"
    DOWNLOAD_URL="https://github.com/e-weil/doli/releases/download/${VERSION}/${ARTIFACT_NAME}.tar.gz"
    CHECKSUM_URL="${DOWNLOAD_URL}.sha256"

    log_info "Downloading: $ARTIFACT_NAME"
    log_info "URL: $DOWNLOAD_URL"

    TARBALL="$TEST_DIR/${ARTIFACT_NAME}.tar.gz"
    CHECKSUM_FILE="$TEST_DIR/${ARTIFACT_NAME}.tar.gz.sha256"

    # Download tarball
    if ! curl -fsSL "$DOWNLOAD_URL" -o "$TARBALL"; then
        log_error "Failed to download tarball"
        exit 3
    fi

    # Download checksum
    if ! curl -fsSL "$CHECKSUM_URL" -o "$CHECKSUM_FILE"; then
        log_warning "Checksum file not found, skipping verification"
    else
        # Verify checksum
        log_info "Verifying checksum..."
        cd "$TEST_DIR"

        # Extract just the hash from the checksum file
        EXPECTED_HASH=$(cat "$CHECKSUM_FILE" | awk '{print $1}')
        ACTUAL_HASH=$(sha256sum "$TARBALL" | awk '{print $1}')

        if [ "$EXPECTED_HASH" != "$ACTUAL_HASH" ]; then
            log_error "Checksum mismatch!"
            log_error "Expected: $EXPECTED_HASH"
            log_error "Actual:   $ACTUAL_HASH"
            exit 3
        fi
        log_success "Checksum verified"
    fi

    # Extract
    cd "$TEST_DIR"
    tar -xzf "$TARBALL"

    if [ ! -f "$TEST_DIR/doli-node" ]; then
        log_error "Binary not found in tarball"
        exit 3
    fi
    chmod +x "$TEST_DIR/doli-node"
    log_success "Downloaded and extracted: $VERSION"

else
    # Try to find locally built binary
    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

    if [ -f "$PROJECT_ROOT/target/release/doli-node" ]; then
        BINARY_PATH="$PROJECT_ROOT/target/release/doli-node"
        cp "$BINARY_PATH" "$TEST_DIR/doli-node"
        chmod +x "$TEST_DIR/doli-node"
        log_success "Using local build: $BINARY_PATH"
    else
        log_error "No binary source specified. Use --binary, --version, or --url"
        exit 2
    fi
fi

BINARY="$TEST_DIR/doli-node"

# Verify binary is executable
if ! "$BINARY" --version >/dev/null 2>&1; then
    log_error "Binary is not executable or crashed on --version"
    exit 1
fi

BINARY_VERSION=$("$BINARY" --version 2>&1 | head -1)
log_success "Binary version: $BINARY_VERSION"

# =============================================================================
# Step 2: Start node in devnet mode
# =============================================================================
log_info "=== Step 2: Starting node in devnet mode ==="

DATA_DIR="$TEST_DIR/data"
LOG_FILE="$TEST_DIR/node.log"
mkdir -p "$DATA_DIR"

# Find available ports
find_free_port() {
    local port=$1
    while nc -z localhost $port 2>/dev/null; do
        port=$((port + 1))
    done
    echo $port
}

# Check if nc is available, otherwise use default ports
if command -v nc &>/dev/null; then
    P2P_PORT=$(find_free_port $P2P_PORT)
    RPC_PORT=$(find_free_port $RPC_PORT)
fi

log_info "P2P port: $P2P_PORT"
log_info "RPC port: $RPC_PORT"

# Start node
"$BINARY" \
    --network devnet \
    --data-dir "$DATA_DIR" \
    --log-level warn \
    run \
    --p2p-port "$P2P_PORT" \
    --rpc-port "$RPC_PORT" \
    --no-auto-update \
    > "$LOG_FILE" 2>&1 &

NODE_PID=$!
log_info "Node started with PID: $NODE_PID"

# Give node time to initialize
log_info "Waiting for node to initialize..."
sleep 5

# Check if node is still running
if ! kill -0 "$NODE_PID" 2>/dev/null; then
    log_error "Node crashed during startup"
    log_error "Last 20 lines of log:"
    tail -20 "$LOG_FILE" || true
    exit 1
fi

log_success "Node is running"

# =============================================================================
# Step 3: Verify RPC responds
# =============================================================================
log_info "=== Step 3: Verifying RPC endpoint ==="

RPC_URL="http://127.0.0.1:$RPC_PORT"
RPC_OK=false

# Wait for RPC to become available
for i in $(seq 1 $TIMEOUT); do
    if curl -sf "$RPC_URL/health" >/dev/null 2>&1; then
        RPC_OK=true
        break
    fi

    # Also try JSON-RPC getNodeInfo
    RESPONSE=$(curl -sf -X POST "$RPC_URL" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"getNodeInfo","params":[],"id":1}' 2>/dev/null || true)

    if echo "$RESPONSE" | grep -q '"result"'; then
        RPC_OK=true
        break
    fi

    # Check if node is still running
    if ! kill -0 "$NODE_PID" 2>/dev/null; then
        log_error "Node crashed while waiting for RPC"
        tail -20 "$LOG_FILE" || true
        exit 1
    fi

    sleep 1
done

if [ "$RPC_OK" = false ]; then
    log_error "RPC did not respond within $TIMEOUT seconds"
    tail -20 "$LOG_FILE" || true
    exit 1
fi

# Get node info
NODE_INFO=$(curl -sf -X POST "$RPC_URL" \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getNodeInfo","params":[],"id":1}' 2>/dev/null || echo '{}')

if echo "$NODE_INFO" | grep -q '"result"'; then
    log_success "RPC responding: $RPC_URL"

    # Extract some info
    NETWORK=$(echo "$NODE_INFO" | grep -o '"network":"[^"]*"' | cut -d'"' -f4 || echo "unknown")
    log_info "Network: $NETWORK"
else
    log_warning "RPC responded but getNodeInfo failed"
    log_info "Response: $NODE_INFO"
fi

# =============================================================================
# Step 4: Verify P2P port is listening
# =============================================================================
log_info "=== Step 4: Verifying P2P port ==="

P2P_OK=false

# Check if P2P port is listening
if command -v ss &>/dev/null; then
    if ss -tlnp 2>/dev/null | grep -q ":$P2P_PORT"; then
        P2P_OK=true
    fi
elif command -v netstat &>/dev/null; then
    if netstat -tlnp 2>/dev/null | grep -q ":$P2P_PORT"; then
        P2P_OK=true
    fi
elif command -v lsof &>/dev/null; then
    if lsof -i ":$P2P_PORT" -P -n 2>/dev/null | grep -q LISTEN; then
        P2P_OK=true
    fi
else
    # Fallback: try to connect
    if command -v nc &>/dev/null; then
        if nc -z localhost $P2P_PORT 2>/dev/null; then
            P2P_OK=true
        fi
    else
        log_warning "Cannot verify P2P port (no ss/netstat/lsof/nc available)"
        P2P_OK=true  # Skip this check
    fi
fi

if [ "$P2P_OK" = true ]; then
    log_success "P2P port listening: $P2P_PORT"
else
    log_error "P2P port not listening: $P2P_PORT"
    exit 1
fi

# =============================================================================
# Step 5: Additional health checks
# =============================================================================
log_info "=== Step 5: Additional health checks ==="

# Check block height (should be 0 or 1 for fresh devnet)
BLOCK_HEIGHT=$(curl -sf -X POST "$RPC_URL" \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getBlockHeight","params":[],"id":1}' 2>/dev/null || echo '{}')

if echo "$BLOCK_HEIGHT" | grep -q '"result"'; then
    HEIGHT=$(echo "$BLOCK_HEIGHT" | grep -oE '"result":[0-9]+' | grep -oE '[0-9]+' || echo "0")
    log_success "Block height: $HEIGHT"
else
    log_warning "Could not get block height"
fi

# Check that node has been running for at least 5 seconds without crashing
sleep 5
if kill -0 "$NODE_PID" 2>/dev/null; then
    log_success "Node stable after initialization"
else
    log_error "Node crashed during stability check"
    tail -20 "$LOG_FILE" || true
    exit 1
fi

# =============================================================================
# Step 6: Clean shutdown
# =============================================================================
log_info "=== Step 6: Testing clean shutdown ==="

# Send SIGTERM for graceful shutdown
kill -TERM "$NODE_PID"
SHUTDOWN_OK=false

for i in {1..15}; do
    if ! kill -0 "$NODE_PID" 2>/dev/null; then
        SHUTDOWN_OK=true
        break
    fi
    sleep 1
done

if [ "$SHUTDOWN_OK" = true ]; then
    log_success "Node shut down gracefully"
else
    log_warning "Node did not shut down gracefully, force killing"
    kill -9 "$NODE_PID" 2>/dev/null || true
fi

# Clear NODE_PID since we've handled shutdown
NODE_PID=""

# =============================================================================
# Summary
# =============================================================================
echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}     SMOKE TEST PASSED${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo "Binary:      $BINARY"
echo "Version:     $BINARY_VERSION"
echo "Target:      $TARGET"
echo "Test dir:    $TEST_DIR"
echo ""

exit 0

#!/bin/bash
#
# DOLI Local Build Script
#
# Builds doli-node and doli CLI from local source.
# No network access required.
#
# Usage:
#   ./scripts/update.sh          # Build release binaries
#   ./scripts/update.sh debug    # Build debug binaries (faster compile)
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

info() { echo -e "${BLUE}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[OK]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }

BUILD_MODE="${1:-release}"

echo ""
echo "=============================================="
echo "         DOLI Local Build"
echo "=============================================="
echo ""

cd "$PROJECT_ROOT"

# Check current version before build
CURRENT_VERSION=""
if [ -f "$PROJECT_ROOT/target/release/doli-node" ]; then
    CURRENT_VERSION=$("$PROJECT_ROOT/target/release/doli-node" --version 2>/dev/null | head -1 || echo "unknown")
    info "Current version: $CURRENT_VERSION"
fi

# Build
if [ "$BUILD_MODE" = "debug" ]; then
    info "Building debug binaries..."
    cargo build -p doli-node -p doli-cli 2>&1
    NODE_BIN="$PROJECT_ROOT/target/debug/doli-node"
    CLI_BIN="$PROJECT_ROOT/target/debug/doli"
else
    info "Building release binaries..."
    cargo build --release -p doli-node -p doli-cli 2>&1
    NODE_BIN="$PROJECT_ROOT/target/release/doli-node"
    CLI_BIN="$PROJECT_ROOT/target/release/doli"
fi

# Verify
if [ ! -f "$NODE_BIN" ]; then
    error "doli-node binary not found after build"
fi
if [ ! -f "$CLI_BIN" ]; then
    error "doli CLI binary not found after build"
fi

NEW_VERSION=$("$NODE_BIN" --version 2>/dev/null | head -1 || echo "unknown")

echo ""
echo "=============================================="
echo -e "${GREEN}         BUILD SUCCESSFUL${NC}"
echo "=============================================="
echo ""
echo "  Mode:     $BUILD_MODE"
echo "  Node:     $NODE_BIN"
echo "  CLI:      $CLI_BIN"
echo "  Version:  $NEW_VERSION"
echo "  Previous: ${CURRENT_VERSION:-none}"
echo ""
echo "=============================================="

#!/bin/bash
#
# DOLI Manual Update Script
#
# Downloads and verifies DOLI node binary from GitHub Releases.
# This script is for manual updates - nodes normally auto-update.
#
# Usage:
#   ./scripts/update.sh           # Update to latest version
#   ./scripts/update.sh v1.0.1    # Update to specific version
#
# Or via curl:
#   curl -L https://raw.githubusercontent.com/doli-network/doli/main/scripts/update.sh | bash
#   curl -L https://raw.githubusercontent.com/doli-network/doli/main/scripts/update.sh | bash -s v1.0.1
#

set -e

# Configuration
# Using author's personal repo (like torvalds/linux, antirez/redis)
REPO="e-weil/doli"
VERSION="${1:-latest}"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

# Detect platform
detect_platform() {
    local os=$(uname -s | tr '[:upper:]' '[:lower:]')
    local arch=$(uname -m)

    case "$os-$arch" in
        linux-x86_64)  echo "linux-x64" ;;
        linux-aarch64) echo "linux-arm64" ;;
        darwin-x86_64) echo "macos-x64" ;;
        darwin-arm64)  echo "macos-arm64" ;;
        *)
            echo "Unsupported platform: $os-$arch" >&2
            exit 1
            ;;
    esac
}

PLATFORM=$(detect_platform)

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

info() { echo -e "${BLUE}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[OK]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }

echo ""
echo "=============================================="
echo "         DOLI Update Script"
echo "=============================================="
echo ""

# Get latest version if not specified
if [ "$VERSION" = "latest" ]; then
    info "Fetching latest release from GitHub..."
    VERSION=$(curl -s "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name"' | cut -d'"' -f4)

    if [ -z "$VERSION" ]; then
        error "Could not determine latest version. Check your internet connection."
    fi

    success "Latest version: $VERSION"
fi

# Ensure version starts with 'v'
if [[ ! "$VERSION" =~ ^v ]]; then
    VERSION="v$VERSION"
fi

# URLs
BINARY_URL="https://github.com/$REPO/releases/download/$VERSION/doli-node-$PLATFORM"
HASH_URL="https://github.com/$REPO/releases/download/$VERSION/SHA256SUMS"

info "Platform: $PLATFORM"
info "Version:  $VERSION"
echo ""

# Create temp directory
TEMP_DIR=$(mktemp -d)
trap "rm -rf $TEMP_DIR" EXIT

# Download binary
info "Downloading doli-node-$PLATFORM..."
if ! curl -fSL -o "$TEMP_DIR/doli-node" "$BINARY_URL" 2>/dev/null; then
    error "Download failed. Check if version $VERSION exists."
fi
success "Binary downloaded"

# Download checksums
info "Downloading checksums..."
if ! curl -fSL -o "$TEMP_DIR/SHA256SUMS" "$HASH_URL" 2>/dev/null; then
    error "Could not download checksums. Release may be incomplete."
fi
success "Checksums downloaded"

# Verify hash
info "Verifying SHA-256 hash..."
EXPECTED_HASH=$(grep "doli-node-$PLATFORM" "$TEMP_DIR/SHA256SUMS" | cut -d' ' -f1)

if [ -z "$EXPECTED_HASH" ]; then
    error "Could not find hash for $PLATFORM in SHA256SUMS"
fi

ACTUAL_HASH=$(sha256sum "$TEMP_DIR/doli-node" | cut -d' ' -f1)

if [ "$EXPECTED_HASH" != "$ACTUAL_HASH" ]; then
    echo ""
    error "HASH MISMATCH - Download may be corrupted or tampered!
   Expected: $EXPECTED_HASH
   Actual:   $ACTUAL_HASH"
fi

success "Hash verified: ${ACTUAL_HASH:0:16}..."

# Make executable
chmod +x "$TEMP_DIR/doli-node"

# Check current version
CURRENT_VERSION=""
if [ -f "$INSTALL_DIR/doli-node" ]; then
    CURRENT_VERSION=$($INSTALL_DIR/doli-node --version 2>/dev/null | head -1 || echo "unknown")
fi

# Install
echo ""
info "Installing to $INSTALL_DIR..."

# Backup current if exists
if [ -f "$INSTALL_DIR/doli-node" ]; then
    sudo cp "$INSTALL_DIR/doli-node" "$INSTALL_DIR/doli-node.backup"
    success "Backup created: doli-node.backup"
fi

# Install new binary
sudo mv "$TEMP_DIR/doli-node" "$INSTALL_DIR/doli-node"
success "Binary installed"

# Verify installation
NEW_VERSION=$($INSTALL_DIR/doli-node --version 2>/dev/null | head -1 || echo "$VERSION")

echo ""
echo "=============================================="
echo -e "${GREEN}         UPDATE SUCCESSFUL${NC}"
echo "=============================================="
echo ""
echo "  Version:  $VERSION"
echo "  Binary:   $INSTALL_DIR/doli-node"
echo "  Previous: ${CURRENT_VERSION:-none}"
echo ""
echo "  To apply, restart your node:"
echo "    sudo systemctl restart doli-node"
echo ""
echo "=============================================="

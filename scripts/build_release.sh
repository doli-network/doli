#!/usr/bin/env bash
# DOLI Release Build Script
# Builds release binaries for all supported platforms
#
# Usage:
#   ./scripts/build_release.sh          # Build all targets
#   ./scripts/build_release.sh linux    # Build Linux targets only
#   ./scripts/build_release.sh macos    # Build macOS targets only
#   ./scripts/build_release.sh musl     # Build musl (static) target only

set -euo pipefail

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BUILD_DIR="$PROJECT_ROOT/target/release-builds"
NODE_BINARY="doli-node"
CLI_BINARY="doli"

# Get version from Cargo.toml
VERSION=$(grep -m1 '^version' "$PROJECT_ROOT/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')
if [ -z "$VERSION" ]; then
    VERSION="dev"
fi

# Git tag override
if git describe --tags --exact-match 2>/dev/null; then
    VERSION=$(git describe --tags --exact-match)
fi

echo "=== DOLI Release Build ==="
echo "Version: $VERSION"
echo "Output: $BUILD_DIR"
echo ""

# Supported targets
LINUX_TARGETS=(
    "x86_64-unknown-linux-gnu"
    "x86_64-unknown-linux-musl"
)

MACOS_TARGETS=(
    "x86_64-apple-darwin"
    "aarch64-apple-darwin"
)

# Determine which targets to build
TARGETS=()
case "${1:-all}" in
    linux)
        TARGETS=("${LINUX_TARGETS[@]}")
        ;;
    macos)
        TARGETS=("${MACOS_TARGETS[@]}")
        ;;
    musl)
        TARGETS=("x86_64-unknown-linux-musl")
        ;;
    gnu)
        TARGETS=("x86_64-unknown-linux-gnu")
        ;;
    all)
        TARGETS=("${LINUX_TARGETS[@]}" "${MACOS_TARGETS[@]}")
        ;;
    *)
        echo "Unknown target group: $1"
        echo "Usage: $0 [linux|macos|musl|gnu|all]"
        exit 1
        ;;
esac

# Create build directory
mkdir -p "$BUILD_DIR"

# Check for required tools
check_tools() {
    if ! command -v cargo &> /dev/null; then
        echo "Error: cargo is not installed"
        exit 1
    fi
}

# Build for a single target
build_target() {
    local target="$1"
    local artifact_name="doli-${VERSION}-${target}"
    local artifact_dir="$BUILD_DIR/$artifact_name"

    echo ""
    echo "=== Building for $target ==="

    # Clean previous build
    rm -rf "$artifact_dir"
    mkdir -p "$artifact_dir"

    # Determine build method
    case "$target" in
        *-linux-musl)
            # Use cross for musl builds
            if command -v cross &> /dev/null; then
                echo "Using cross for musl build..."
                cross build --release --target "$target" --package doli-node --package doli-cli
            else
                echo "Warning: cross not found, attempting native cargo build..."
                cargo build --release --target "$target" --package doli-node --package doli-cli
            fi
            ;;
        *-apple-darwin)
            # Native macOS build (requires macOS host or cross-compilation setup)
            if [[ "$(uname -s)" == "Darwin" ]]; then
                rustup target add "$target" 2>/dev/null || true
                cargo build --release --target "$target" --package doli-node --package doli-cli
            else
                echo "Skipping $target (not on macOS)"
                return 0
            fi
            ;;
        *)
            # Standard cargo build
            cargo build --release --target "$target" --package doli-node --package doli-cli
            ;;
    esac

    # Check if builds succeeded
    local node_path="$PROJECT_ROOT/target/$target/release/$NODE_BINARY"
    local cli_path="$PROJECT_ROOT/target/$target/release/$CLI_BINARY"
    if [ ! -f "$node_path" ]; then
        echo "Error: Binary not found at $node_path"
        return 1
    fi
    if [ ! -f "$cli_path" ]; then
        echo "Error: Binary not found at $cli_path"
        return 1
    fi

    # Copy binaries
    cp "$node_path" "$artifact_dir/$NODE_BINARY"
    cp "$cli_path" "$artifact_dir/$CLI_BINARY"

    # Create README for the release
    cat > "$artifact_dir/README.txt" << EOF
DOLI - Version $VERSION
Target: $target

Included binaries:
  doli-node  — Full node
  doli       — Wallet CLI

Quick Start:
  chmod +x $NODE_BINARY $CLI_BINARY
  ./$NODE_BINARY run                    # Start mainnet node
  ./$NODE_BINARY --network testnet run  # Start testnet node
  ./$NODE_BINARY --help                 # Show all options
  ./$CLI_BINARY --help                  # Wallet commands

Documentation: https://github.com/e-weil/doli/docs/running_a_node.md
Support: https://github.com/e-weil/doli/issues
EOF

    # Create tarball
    echo "Creating tarball..."
    local tarball="$BUILD_DIR/${artifact_name}.tar.gz"
    tar -czf "$tarball" -C "$BUILD_DIR" "$artifact_name"

    # Generate checksum
    echo "Generating checksum..."
    if command -v sha256sum &> /dev/null; then
        sha256sum "$tarball" | cut -d' ' -f1 > "$tarball.sha256"
    elif command -v shasum &> /dev/null; then
        shasum -a 256 "$tarball" | cut -d' ' -f1 > "$tarball.sha256"
    else
        echo "Warning: No sha256sum tool found, skipping checksum"
    fi

    # Show results
    echo "Built: $tarball"
    if [ -f "$tarball.sha256" ]; then
        echo "SHA256: $(cat "$tarball.sha256")"
    fi

    # Clean up extracted directory
    rm -rf "$artifact_dir"

    return 0
}

# Main
check_tools

echo "Targets to build:"
for target in "${TARGETS[@]}"; do
    echo "  - $target"
done

# Build each target
FAILED=()
for target in "${TARGETS[@]}"; do
    if ! build_target "$target"; then
        FAILED+=("$target")
    fi
done

# Summary
echo ""
echo "=== Build Summary ==="
echo "Output directory: $BUILD_DIR"
echo ""

ls -la "$BUILD_DIR"/*.tar.gz 2>/dev/null || echo "No artifacts built"

if [ ${#FAILED[@]} -gt 0 ]; then
    echo ""
    echo "Failed targets:"
    for target in "${FAILED[@]}"; do
        echo "  - $target"
    done
    exit 1
fi

echo ""
echo "All builds completed successfully!"

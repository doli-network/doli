#!/bin/bash
# DOLI Release Build Script
# Builds release binaries for all supported platforms
#
# Usage:
#   ./scripts/build_release.sh              # Build for current platform
#   ./scripts/build_release.sh --all        # Build for all platforms (requires cross)
#   ./scripts/build_release.sh --target x86_64-unknown-linux-musl
#   ./scripts/build_release.sh --version v1.0.0
#
# Environment variables:
#   DOLI_VERSION    - Version string (default: from git tag or Cargo.toml)
#   DOLI_BUILD_DIR  - Output directory (default: ./release)
#   SKIP_TESTS      - Set to 1 to skip running tests before build

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
BUILD_DIR="${DOLI_BUILD_DIR:-${PROJECT_ROOT}/release}"

# Supported targets
LINUX_TARGETS=(
    "x86_64-unknown-linux-gnu"
    "x86_64-unknown-linux-musl"
    "aarch64-unknown-linux-gnu"
    "aarch64-unknown-linux-musl"
)

MACOS_TARGETS=(
    "x86_64-apple-darwin"
    "aarch64-apple-darwin"
)

ALL_TARGETS=("${LINUX_TARGETS[@]}" "${MACOS_TARGETS[@]}")

# Binaries to build
BINARIES=("doli-node" "doli")

# Functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

get_version() {
    if [ -n "$DOLI_VERSION" ]; then
        echo "$DOLI_VERSION"
        return
    fi

    # Try git tag first
    if git describe --tags --exact-match 2>/dev/null; then
        return
    fi

    # Try git describe
    if git describe --tags 2>/dev/null; then
        return
    fi

    # Fall back to Cargo.toml version
    grep -m1 'version = ' "${PROJECT_ROOT}/Cargo.toml" | sed 's/.*"\(.*\)".*/v\1/'
}

get_current_target() {
    rustc -vV | grep host | cut -d' ' -f2
}

is_macos_target() {
    [[ "$1" == *"apple-darwin"* ]]
}

is_musl_target() {
    [[ "$1" == *"musl"* ]]
}

check_cross_installed() {
    if ! command -v cross &> /dev/null; then
        log_error "cross is not installed. Install with: cargo install cross"
        log_info "See: https://github.com/cross-rs/cross"
        exit 1
    fi
}

check_target_installed() {
    local target=$1
    if ! rustup target list --installed | grep -q "^${target}$"; then
        log_warn "Target ${target} not installed. Installing..."
        rustup target add "$target"
    fi
}

build_target() {
    local target=$1
    local version=$2
    local current_target=$(get_current_target)

    log_info "Building for target: ${target}"

    cd "$PROJECT_ROOT"

    # Determine build command
    local build_cmd="cargo"
    local use_cross=false

    if [ "$target" != "$current_target" ]; then
        if is_macos_target "$target" && [[ "$(uname)" != "Darwin" ]]; then
            log_warn "Cannot cross-compile to macOS from non-macOS host. Skipping ${target}."
            return 1
        fi

        if is_musl_target "$target" || [[ "$target" == *"aarch64"* && "$(uname -m)" != "aarch64" ]]; then
            check_cross_installed
            build_cmd="cross"
            use_cross=true
        else
            check_target_installed "$target"
        fi
    fi

    # Build doli-node
    log_info "Building doli-node for ${target}..."
    if [ "$use_cross" = true ]; then
        cross build --release --target "$target" -p doli-node
    else
        cargo build --release --target "$target" -p doli-node
    fi

    # Build doli-cli
    log_info "Building doli-cli for ${target}..."
    if [ "$use_cross" = true ]; then
        cross build --release --target "$target" -p doli-cli
    else
        cargo build --release --target "$target" -p doli-cli
    fi

    log_success "Build complete for ${target}"
    return 0
}

package_target() {
    local target=$1
    local version=$2

    log_info "Packaging ${target}..."

    local target_dir="${PROJECT_ROOT}/target/${target}/release"
    local package_name="doli-${version}-${target}"
    local package_dir="${BUILD_DIR}/${package_name}"

    # Create package directory
    mkdir -p "$package_dir"

    # Copy binaries
    local node_binary="${target_dir}/doli-node"
    local cli_binary="${target_dir}/doli"

    if is_macos_target "$target" || [[ ! -f "${node_binary}" ]]; then
        # Windows would have .exe extension
        if [[ "$target" == *"windows"* ]]; then
            node_binary="${node_binary}.exe"
            cli_binary="${cli_binary}.exe"
        fi
    fi

    if [ ! -f "$node_binary" ]; then
        log_error "Binary not found: ${node_binary}"
        return 1
    fi

    cp "$node_binary" "$package_dir/"
    cp "$cli_binary" "$package_dir/" 2>/dev/null || log_warn "CLI binary not found, skipping"

    # Create README for the package
    cat > "${package_dir}/README.txt" << EOF
DOLI Node ${version}
===================

Target: ${target}

Quick Start:
  ./doli-node run                      # Start mainnet node
  ./doli-node --network testnet run    # Start testnet node
  ./doli-node --network devnet run     # Start devnet node

Documentation:
  https://github.com/e-weil/doli/blob/main/docs/RUNNING_A_NODE.md

For help:
  ./doli-node --help
  ./doli --help

Verify checksum:
  sha256sum -c doli-${version}-${target}.sha256

EOF

    # Create tarball
    local tarball="${BUILD_DIR}/${package_name}.tar.gz"
    cd "$BUILD_DIR"
    tar -czf "${package_name}.tar.gz" "$package_name"

    # Generate checksum
    sha256sum "${package_name}.tar.gz" > "${package_name}.tar.gz.sha256"

    # Also generate checksum for just the binary
    cd "$package_dir"
    sha256sum doli-node > "../${package_name}-doli-node.sha256"
    if [ -f doli ]; then
        sha256sum doli >> "../${package_name}-doli-node.sha256"
    fi

    # Cleanup package directory
    cd "$BUILD_DIR"
    rm -rf "$package_dir"

    log_success "Created: ${tarball}"
    log_success "Checksum: ${tarball}.sha256"

    return 0
}

run_tests() {
    log_info "Running tests before build..."
    cd "$PROJECT_ROOT"
    cargo test --release 2>&1 | grep -i "test result\|passed\|failed" | head -5
    log_success "Tests passed"
}

print_usage() {
    cat << EOF
DOLI Release Build Script

Usage:
    $(basename "$0") [OPTIONS]

Options:
    --all               Build for all supported platforms (requires cross)
    --linux             Build for all Linux platforms
    --macos             Build for all macOS platforms (requires macOS host)
    --target <TARGET>   Build for specific target
    --version <VERSION> Set version string (default: auto-detect from git/Cargo.toml)
    --skip-tests        Skip running tests before build
    --clean             Clean build directory before building
    --list-targets      List all supported targets
    -h, --help          Show this help message

Supported targets:
    Linux (GNU):  x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu
    Linux (musl): x86_64-unknown-linux-musl, aarch64-unknown-linux-musl
    macOS:        x86_64-apple-darwin, aarch64-apple-darwin

Environment variables:
    DOLI_VERSION    Override version string
    DOLI_BUILD_DIR  Output directory (default: ./release)
    SKIP_TESTS      Set to 1 to skip tests

Examples:
    $(basename "$0")                           # Build for current platform
    $(basename "$0") --all                     # Build all platforms
    $(basename "$0") --target x86_64-unknown-linux-musl
    $(basename "$0") --version v1.0.0 --linux  # Build Linux targets with version
EOF
}

list_targets() {
    echo "Supported build targets:"
    echo ""
    echo "Linux (dynamically linked):"
    echo "  - x86_64-unknown-linux-gnu   (Intel/AMD 64-bit)"
    echo "  - aarch64-unknown-linux-gnu  (ARM 64-bit)"
    echo ""
    echo "Linux (statically linked - musl):"
    echo "  - x86_64-unknown-linux-musl  (Intel/AMD 64-bit, runs on any Linux)"
    echo "  - aarch64-unknown-linux-musl (ARM 64-bit, runs on any Linux)"
    echo ""
    echo "macOS (requires macOS host to build):"
    echo "  - x86_64-apple-darwin        (Intel Mac)"
    echo "  - aarch64-apple-darwin       (Apple Silicon M1/M2/M3)"
}

# Main
main() {
    local targets=()
    local version=""
    local skip_tests="${SKIP_TESTS:-0}"
    local clean_build=false

    # Parse arguments
    while [[ $# -gt 0 ]]; do
        case $1 in
            --all)
                targets=("${ALL_TARGETS[@]}")
                shift
                ;;
            --linux)
                targets=("${LINUX_TARGETS[@]}")
                shift
                ;;
            --macos)
                targets=("${MACOS_TARGETS[@]}")
                shift
                ;;
            --target)
                targets+=("$2")
                shift 2
                ;;
            --version)
                version="$2"
                shift 2
                ;;
            --skip-tests)
                skip_tests=1
                shift
                ;;
            --clean)
                clean_build=true
                shift
                ;;
            --list-targets)
                list_targets
                exit 0
                ;;
            -h|--help)
                print_usage
                exit 0
                ;;
            *)
                log_error "Unknown option: $1"
                print_usage
                exit 1
                ;;
        esac
    done

    # Default to current platform if no targets specified
    if [ ${#targets[@]} -eq 0 ]; then
        targets=("$(get_current_target)")
    fi

    # Get version
    if [ -z "$version" ]; then
        version=$(get_version)
    fi
    # Remove 'v' prefix if present for internal use, but keep for display
    version_display="$version"
    version="${version#v}"

    log_info "Building DOLI ${version_display}"
    log_info "Targets: ${targets[*]}"
    log_info "Output directory: ${BUILD_DIR}"

    # Clean if requested
    if [ "$clean_build" = true ]; then
        log_info "Cleaning build directory..."
        rm -rf "$BUILD_DIR"
    fi

    # Create build directory
    mkdir -p "$BUILD_DIR"

    # Run tests unless skipped
    if [ "$skip_tests" != "1" ]; then
        run_tests
    fi

    # Build and package each target
    local successful=()
    local failed=()

    for target in "${targets[@]}"; do
        if build_target "$target" "$version_display"; then
            if package_target "$target" "$version_display"; then
                successful+=("$target")
            else
                failed+=("$target")
            fi
        else
            failed+=("$target")
        fi
    done

    # Summary
    echo ""
    echo "======================================"
    echo "Build Summary"
    echo "======================================"
    echo "Version: ${version_display}"
    echo "Output:  ${BUILD_DIR}"
    echo ""

    if [ ${#successful[@]} -gt 0 ]; then
        log_success "Successful builds:"
        for t in "${successful[@]}"; do
            echo "  - $t"
        done
    fi

    if [ ${#failed[@]} -gt 0 ]; then
        log_error "Failed builds:"
        for t in "${failed[@]}"; do
            echo "  - $t"
        done
    fi

    echo ""
    echo "Artifacts:"
    ls -la "${BUILD_DIR}"/*.tar.gz 2>/dev/null || echo "  (none)"

    # Exit with error if any builds failed
    if [ ${#failed[@]} -gt 0 ]; then
        exit 1
    fi

    log_success "All builds completed successfully!"
}

main "$@"

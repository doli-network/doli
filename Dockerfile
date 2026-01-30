# DOLI Node - Multi-stage Docker Build
# Stage 1: Builder - Compiles the Rust binary with all dependencies
# Stage 2: Runtime - Minimal image with only runtime dependencies

# =============================================================================
# BUILDER STAGE
# =============================================================================
FROM rust:1.85-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    clang \
    llvm \
    libclang-dev \
    m4 \
    libgmp-dev \
    librocksdb-dev \
    libssl-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Set environment variables for linking
ENV LIBCLANG_PATH=/usr/lib/llvm-14/lib
ENV ROCKSDB_LIB_DIR=/usr/lib

WORKDIR /build

# Copy manifests first for dependency caching
COPY Cargo.toml Cargo.lock ./
COPY crates/crypto/Cargo.toml crates/crypto/
COPY crates/vdf/Cargo.toml crates/vdf/
COPY crates/core/Cargo.toml crates/core/
COPY crates/mempool/Cargo.toml crates/mempool/
COPY crates/storage/Cargo.toml crates/storage/
COPY crates/network/Cargo.toml crates/network/
COPY crates/rpc/Cargo.toml crates/rpc/
COPY crates/updater/Cargo.toml crates/updater/
COPY bins/node/Cargo.toml bins/node/
COPY bins/cli/Cargo.toml bins/cli/
COPY testing/benchmarks/Cargo.toml testing/benchmarks/
COPY testing/integration/Cargo.toml testing/integration/

# Create stub source files for dependency compilation
RUN mkdir -p crates/crypto/src && echo "pub fn stub() {}" > crates/crypto/src/lib.rs && \
    mkdir -p crates/vdf/src && echo "pub fn stub() {}" > crates/vdf/src/lib.rs && \
    mkdir -p crates/core/src && echo "pub fn stub() {}" > crates/core/src/lib.rs && \
    mkdir -p crates/mempool/src && echo "pub fn stub() {}" > crates/mempool/src/lib.rs && \
    mkdir -p crates/storage/src && echo "pub fn stub() {}" > crates/storage/src/lib.rs && \
    mkdir -p crates/network/src && echo "pub fn stub() {}" > crates/network/src/lib.rs && \
    mkdir -p crates/rpc/src && echo "pub fn stub() {}" > crates/rpc/src/lib.rs && \
    mkdir -p crates/updater/src && echo "pub fn stub() {}" > crates/updater/src/lib.rs && \
    mkdir -p bins/node/src && echo "fn main() {}" > bins/node/src/main.rs && \
    mkdir -p bins/cli/src && echo "fn main() {}" > bins/cli/src/main.rs && \
    mkdir -p testing/benchmarks/src && echo "fn main() {}" > testing/benchmarks/src/main.rs && \
    mkdir -p testing/integration/src && echo "fn main() {}" > testing/integration/src/main.rs

# Build dependencies only (this layer will be cached)
RUN cargo build --release -p doli-node 2>&1 || true

# Remove stub source files
RUN rm -rf crates/*/src bins/*/src testing/*/src

# Copy actual source code
COPY crates crates
COPY bins bins
COPY testing/benchmarks testing/benchmarks
COPY testing/integration testing/integration

# Touch source files to invalidate compilation cache for our code only
RUN find crates bins -name "*.rs" -exec touch {} \;

# Build the actual binary
RUN cargo build --release -p doli-node

# Verify binary was built
RUN test -f /build/target/release/doli-node

# =============================================================================
# RUNTIME STAGE
# =============================================================================
FROM debian:bookworm-slim AS runtime

# Install only runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libgmp10 \
    librocksdb7.8 \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create non-root user for security
RUN useradd -r -u 1000 -m -s /bin/bash doli

# Create data directory
RUN mkdir -p /data && chown doli:doli /data

# Copy binary from builder
COPY --from=builder /build/target/release/doli-node /usr/local/bin/doli-node

# Copy entrypoint script
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh

# Ensure binary and entrypoint are executable
RUN chmod +x /usr/local/bin/doli-node /usr/local/bin/docker-entrypoint.sh

# Install curl for healthcheck
RUN apt-get update && apt-get install -y --no-install-recommends curl \
    && rm -rf /var/lib/apt/lists/*

# Switch to non-root user
USER doli

# Set working directory
WORKDIR /data

# Environment variables for configuration
# Network: mainnet, testnet, or devnet
ENV DOLI_NETWORK=mainnet
# Data directory
ENV DOLI_DATA_DIR=/data
# Log level: error, warn, info, debug, trace
ENV DOLI_LOG_LEVEL=info

# Expose ports
# Mainnet: P2P=30303, RPC=8545
# Testnet: P2P=40303, RPC=18545
# Devnet:  P2P=50303, RPC=28545
# Metrics: 9090
EXPOSE 30303 40303 50303 8545 18545 28545 9090

# Health check - verifies RPC is responding
HEALTHCHECK --interval=30s --timeout=10s --start-period=60s --retries=3 \
    CMD curl -sf http://localhost:8545/health || curl -sf http://localhost:18545/health || curl -sf http://localhost:28545/health || exit 1

# Volume for persistent blockchain data
VOLUME ["/data"]

# Entrypoint handles environment variable to CLI argument translation
ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["run"]

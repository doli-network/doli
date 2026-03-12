# DOLI Node - Multi-stage Dockerfile
# Build: docker build -t doli-node -f docker/Dockerfile .
# Run: docker run -d -p 30300:30300 -p 8500:8500 -v doli-data:/data doli-node

# =============================================================================
# Stage 1: Builder
# =============================================================================
FROM rust:1.87-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    cmake \
    clang \
    libclang-dev \
    llvm-dev \
    libgmp-dev \
    libmpfr-dev \
    librocksdb-dev \
    libssl-dev \
    pkg-config \
    protobuf-compiler \
    m4 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy workspace configuration first (for better caching)
COPY Cargo.toml Cargo.lock ./

# Copy all crates, bins, and chainspec (embedded in binary via include_str!)
COPY crates/ crates/
COPY bins/ bins/
COPY testing/ testing/
COPY chainspec.mainnet.json chainspec.testnet.json ./

# Build release binary
RUN cargo build --release --package doli-node

# =============================================================================
# Stage 2: Runtime
# =============================================================================
FROM debian:bookworm-slim AS runtime

# Install runtime dependencies only
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libgmp10 \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd -r doli && useradd -r -g doli -u 1000 doli

# Create data directory
RUN mkdir -p /data && chown doli:doli /data

# Copy binary from builder
COPY --from=builder /build/target/release/doli-node /usr/local/bin/doli-node

# Set permissions
RUN chmod +x /usr/local/bin/doli-node

# Environment variables with defaults
ENV DOLI_NETWORK=mainnet \
    DOLI_DATA_DIR=/data \
    DOLI_LOG_LEVEL=info \
    DOLI_METRICS_PORT=9000 \
    DOLI_PRODUCER=false \
    RUST_BACKTRACE=1

# Expose ports
# P2P: 30300 (mainnet), 40300 (testnet), 50300 (devnet)
# RPC: 8500 (mainnet), 18500 (testnet), 28500 (devnet)
# Metrics: 9000 (mainnet), 19000 (testnet), 29000 (devnet)
EXPOSE 30300 40300 50300 8500 18500 28500 9000 19000 29000

# Volume for blockchain data
VOLUME ["/data"]

# Switch to non-root user
USER doli

# Health check - verify RPC is responding via JSON-RPC getChainInfo
HEALTHCHECK --interval=30s --timeout=10s --start-period=60s --retries=3 \
    CMD curl -sf -X POST http://localhost:${DOLI_RPC_PORT:-8500} \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' \
        | grep -q '"result"' || exit 1

# Entrypoint script handles environment variable mapping
COPY --chmod=755 docker/docker-entrypoint.sh /docker-entrypoint.sh

ENTRYPOINT ["/docker-entrypoint.sh"]
CMD ["run"]

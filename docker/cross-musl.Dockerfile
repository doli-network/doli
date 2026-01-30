# Cross-compilation Docker image for building static DOLI binaries
# This image includes all dependencies statically linked for musl targets
#
# Build: docker build -f docker/cross-musl.Dockerfile -t ghcr.io/e-weil/doli-cross:x86_64-musl .
# Usage: Used by cross via Cross.toml configuration

ARG RUST_VERSION=1.85
ARG TARGET=x86_64-unknown-linux-musl

FROM rust:${RUST_VERSION}-alpine AS builder

# Build arguments
ARG TARGET
ARG GMP_VERSION=6.3.0
ARG OPENSSL_VERSION=3.2.1
ARG ROCKSDB_VERSION=9.0.0

# Install build dependencies
RUN apk add --no-cache \
    musl-dev \
    gcc \
    g++ \
    make \
    cmake \
    perl \
    linux-headers \
    git \
    pkgconfig \
    clang \
    llvm \
    m4 \
    autoconf \
    automake \
    libtool \
    curl

# Set up environment
ENV CC=musl-gcc
ENV CXX=g++
ENV MUSL_PREFIX=/musl
ENV PKG_CONFIG_PATH=${MUSL_PREFIX}/lib/pkgconfig
ENV PKG_CONFIG_ALLOW_CROSS=1
ENV PKG_CONFIG_ALL_STATIC=1

RUN mkdir -p ${MUSL_PREFIX}/lib ${MUSL_PREFIX}/include

# Build GMP (statically)
WORKDIR /build/gmp
RUN curl -LO https://gmplib.org/download/gmp/gmp-${GMP_VERSION}.tar.xz && \
    tar xf gmp-${GMP_VERSION}.tar.xz && \
    cd gmp-${GMP_VERSION} && \
    ./configure \
        --prefix=${MUSL_PREFIX} \
        --enable-static \
        --disable-shared \
        --host=${TARGET} \
        CC=musl-gcc && \
    make -j$(nproc) && \
    make install

# Build OpenSSL (statically)
WORKDIR /build/openssl
RUN curl -LO https://www.openssl.org/source/openssl-${OPENSSL_VERSION}.tar.gz && \
    tar xf openssl-${OPENSSL_VERSION}.tar.gz && \
    cd openssl-${OPENSSL_VERSION} && \
    ./Configure \
        linux-x86_64 \
        --prefix=${MUSL_PREFIX} \
        --openssldir=${MUSL_PREFIX}/ssl \
        no-shared \
        no-zlib \
        -static \
        CC=musl-gcc && \
    make -j$(nproc) && \
    make install_sw

# Build RocksDB (statically)
WORKDIR /build/rocksdb
RUN git clone --depth 1 --branch v${ROCKSDB_VERSION} https://github.com/facebook/rocksdb.git && \
    cd rocksdb && \
    mkdir build && cd build && \
    cmake .. \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX=${MUSL_PREFIX} \
        -DCMAKE_C_COMPILER=musl-gcc \
        -DCMAKE_CXX_COMPILER=g++ \
        -DROCKSDB_BUILD_SHARED=OFF \
        -DWITH_GFLAGS=OFF \
        -DWITH_TESTS=OFF \
        -DWITH_BENCHMARK_TOOLS=OFF \
        -DWITH_TOOLS=OFF \
        -DWITH_CORE_TOOLS=OFF \
        -DWITH_ALL_TESTS=OFF \
        -DUSE_RTTI=ON \
        -DFAIL_ON_WARNINGS=OFF && \
    make -j$(nproc) rocksdb && \
    make install

# Cleanup
RUN rm -rf /build

# Set up Rust target
RUN rustup target add ${TARGET}

# Environment for building DOLI
ENV OPENSSL_STATIC=1
ENV OPENSSL_LIB_DIR=${MUSL_PREFIX}/lib
ENV OPENSSL_INCLUDE_DIR=${MUSL_PREFIX}/include
ENV ROCKSDB_STATIC=1
ENV ROCKSDB_LIB_DIR=${MUSL_PREFIX}/lib
ENV GMP_STATIC=1
ENV GMP_LIB_DIR=${MUSL_PREFIX}/lib
ENV GMP_INCLUDE_DIR=${MUSL_PREFIX}/include

# Verify libraries exist
RUN ls -la ${MUSL_PREFIX}/lib/*.a

WORKDIR /project

# Default command
CMD ["cargo", "build", "--release"]

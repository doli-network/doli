# Installation Guide

## Table of Contents
- Prerequisites
- Install from Source
- Verify Installation
- First Run

## Prerequisites

### Rust Toolchain

```bash
# Install Rust (if not present)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Verify (minimum 1.85)
rustc --version
cargo --version
```

### System Dependencies

**macOS:**
```bash
xcode-select --install
brew install gmp cmake protobuf
```

**Ubuntu/Debian:**
```bash
sudo apt update
sudo apt install -y build-essential cmake pkg-config libssl-dev \
  libgmp-dev protobuf-compiler clang libclang-dev
```

**Fedora/RHEL:**
```bash
sudo dnf install -y gcc gcc-c++ cmake openssl-devel gmp-devel \
  protobuf-compiler clang clang-devel
```

### Nix (Optional but Recommended)

```bash
curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh

# Enter dev shell (all deps provided)
nix --extra-experimental-features "nix-command flakes" develop
```

## Install from Source

```bash
# Clone
git clone https://github.com/e-weil/doli.git
cd doli

# Build (dev)
cargo build

# Build (release - for production nodes)
cargo build --release

# Verify
./target/release/doli-node --version
./target/release/doli --version
```

### Build Outputs

| Binary | Path | Purpose |
|--------|------|---------|
| `doli-node` | `target/release/doli-node` | Full node (sync, produce, validate) |
| `doli` | `target/release/doli` | Wallet CLI (send, balance, producer cmds) |

## Verify Installation

```bash
# Run tests
cargo test

# Full pre-commit check
cargo build && cargo clippy -- -D warnings && cargo fmt --check && cargo test
```

## First Run

```bash
# 1. Create wallet
./target/release/doli new

# 2. Start node (mainnet, non-producer)
./target/release/doli-node run --yes

# 3. Check sync progress
./target/release/doli chain

# 4. Check balance once synced
./target/release/doli balance
```

### Data Directories

| Network | Default Dir |
|---------|-------------|
| Mainnet | `~/.doli/mainnet/` |
| Testnet | `~/.doli/testnet/` |
| Devnet | `~/.doli/devnet/` |

Files created on first run:
- `chainspec.json` - embedded network config
- `data/blocks/` - RocksDB block storage
- `chain_state.bin` - chain tip state
- `producers.bin` - producer registry
- `utxo.bin` - UTXO set

# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.


### Prerequisites: 
1. Check @specs/SPECS.md and @docs/DOCS.md for instructions on how to build and test the project.
2. Enter Nix Environment

**All commands must be run inside the Nix environment.** This ensures all dependencies and tools are available.

Always use nix to run all the stack tech using filter best practices to avoid context pollution.
```bash
nix --extra-experimental-features "nix-command flakes" develop
```

## Build & Test Commands

## Command Output Filtering Best Practices
- **Reduce noise and pollution when running commands**: Use output redirection when running nix commands like > /tmp/output.log, and then use awk, sort, head to filter just what you need specially with nix commands when building and testing.
- **Case-insensitive filtering**: Use grep -i for keywords like "pass|fail|error|success|warning"
- **Deduplication**: Use sort -u or awk '!seen[$0]++' for order-preserving deduplication
- **Limit output**: Use head -n 20 or tail -n 10 to control output volume
- **Test filtering**: test_command | grep -i "pass\|fail\|error" | sort -u | head -20
- **Build command filtering**: build_command | grep -i "error\|warning\|failed\|success" | head -10
- **Common filter pattern**: command 2>&1 | grep -i "keyword1\|keyword2" | awk '!seen[$0]++' | head -15
- **Test summary extraction**: test_command | grep -E "^\+[0-9]+|-[0-9]+|passed|failed|Some tests" | tail -5

## Project Overview

DOLI is a Rust-based cryptocurrency where Verifiable Delay Functions (VDF) are the **primary consensus mechanism**—the first blockchain with this design. Time is the scarce resource, not energy or stake.

**Consensus**: Proof of Time (PoT) using hash-chain VDF with deterministic round-robin producer selection based on bond count.

## Build Commands

```bash
# Build all crates
cargo build
cargo build --release

# Run tests (all workspace crates)
cargo test

# Run tests for a specific crate
cargo test -p core
cargo test -p crypto
cargo test -p vdf

# Run a single test
cargo test -p core test_name

# Linting and formatting
cargo clippy
cargo fmt --check
cargo fmt

# Generate API documentation
cargo doc --workspace --no-deps --open
```

## Running Binaries

```bash
# Run node (package names differ from binary names)
cargo run -p doli-node -- run                      # mainnet
cargo run -p doli-node -- --network testnet run   # testnet
cargo run -p doli-node -- --network devnet run    # local dev

# Run CLI wallet
cargo run -p doli-cli -- wallet new
cargo run -p doli-cli -- wallet balance <address>
```

## Fuzz Testing

Fuzz tests are in a separate workspace at `testing/fuzz/` (not part of main workspace):

```bash
cd testing/fuzz
cargo +nightly fuzz run fuzz_block_deserialize
cargo +nightly fuzz run fuzz_tx_deserialize
cargo +nightly fuzz run fuzz_vdf_verify
```

## Nix Development Environment

```bash
nix develop  # Enters shell with Rust 1.75, GMP, RocksDB, OpenSSL
```

## Architecture

### Crate Dependency Flow

```
bins/node (doli-node)          bins/cli (doli-cli)
    │                              │
    ├─→ network ─┐                 │
    ├─→ rpc ─────┤                 │
    ├─→ mempool ─┤                 │
    ├─→ storage ─┤                 │
    ├─→ updater ─┤                 │
    │            ▼                 │
    └─────────→ core ←─────────────┘
                 │
                 ▼
         ┌───────┴───────┐
         ▼               ▼
      crypto            vdf
```

### Key Crate Responsibilities

- **`crypto`**: BLAKE3 hashing, Ed25519 signatures, key derivation. Foundation layer with no internal dependencies.
- **`vdf`**: Hash-chain VDF with dynamic calibration (~10M iterations for ~700ms). Uses `rug` (GMP bindings) for performance.
- **`core`**: Types, validation rules, consensus parameters. The `tpop/` module handles Temporal Proof of Presence (producer scheduling, heartbeat VDF, calibration).
- **`storage`**: RocksDB-backed persistence for blocks, UTXO set, chain state, producer registry.
- **`network`**: libp2p-based P2P layer with gossipsub, Kademlia DHT, header/body sync, equivocation detection.
- **`mempool`**: Transaction pool with fee policies.
- **`rpc`**: JSON-RPC server (Axum) for wallet/explorer interaction.

### Consensus: How Proof of Time Works

1. **Slot-based time**: 60-second slots, wall-clock time determines when slots occur
2. **Deterministic selection**: `slot % total_bonds` selects producer (no lottery variance)
3. **Heartbeat VDF**: ~700ms proof of sequential presence per block (anti-grinding)
4. **Epoch Lookahead**: Leaders determined at epoch start, so grinding current block cannot influence future selection
5. **Weight-based fork choice**: Chain with highest accumulated producer weight wins (weight = 1 + sqrt(months_active/12), capped at 4)

### Time Structure

- Slot = 10 seconds (mainnet), 10s (testnet), 5s (devnet)
- Epoch = 60 slots (1 hour mainnet)
- Era = 12,614,400 slots (~4 years) - triggers reward halving

### Networks

| Network | ID | Slot | P2P Port | RPC Port | Address Prefix |
|---------|-----|------|----------|----------|----------------|
| Mainnet | 1   | 10s  | 30303    | 8545     | `doli`         |
| Testnet | 2   | 10s  | 40303    | 18545    | `tdoli`        |
| Devnet  | 99  | 5s   | 50303    | 28545    | `ddoli`        |

## Code Conventions

- Branch naming: `feature/`, `fix/`, `docs/`, `refactor/`
- Commits: Conventional Commits format (`feat(scope): description`)
- Max line length: 100 characters
- Unit tests in same file, integration tests in `testing/`
- Property-based testing (proptest) for cryptographic code

## Workflow Rules

### After Passing Tests (Bug Fixes or New Features)

Once code changes pass all unit tests:
1. Update `specs/` and `docs/` if the changes affect documented behavior or specifications
2. Commit all changes together

### Handling Bug Reports (`REPORT_*.md` Files)

When fixing a bug documented in a `REPORT_*.md` file:
1. Implement the fix and verify tests pass
2. Update the report file with:
   - Description of the fix applied
   - Resolution date
3. Move the report to `docs/legacy/bugs/`
4. Update `specs/` and `docs/` if affected
5. Commit all changes together

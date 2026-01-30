# DOLI

[![CI](https://github.com/e-weil/doli/actions/workflows/ci.yml/badge.svg)](https://github.com/e-weil/doli/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/e-weil/doli)](https://github.com/e-weil/doli/releases)
[![Docker](https://img.shields.io/badge/docker-ghcr.io%2Fe--weil%2Fdoli--node-blue)](https://github.com/e-weil/doli/pkgs/container/doli-node)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

**A Peer-to-Peer Electronic Cash System Based on Time**

DOLI is a cryptocurrency that uses Verifiable Delay Functions (VDF) instead of traditional Proof of Work. The scarce resource is not energy or permanent capital, but **time**.

## Overview

DOLI solves the double-spending problem through a peer network that anchors consensus to verifiable sequential work. Each block requires a VDF computation that takes fixed time and cannot be accelerated through parallelization.

### Key Features

- **Time-based consensus**: Blocks are produced every 10 seconds using VDF proofs
- **Sybil resistance**: Producer registration requires sequential computation + economic bond
- **Fair distribution**: No pre-mine, no ICO - coins are distributed through block production
- **Energy efficient**: Sequential computation cannot benefit from parallel hardware
- **Predictable issuance**: ~25,228,800 total supply with halving every ~4 years

### Unique Contribution

DOLI is the first blockchain where Verifiable Delay Functions (VDF) are the **primary consensus mechanism**, not a complement.

| System    | Scarce Resource   | VDF Role                          |
|-----------|-------------------|-----------------------------------|
| Bitcoin   | Energy            | Not used                          |
| Ethereum  | Capital (stake)   | Not used                          |
| Chia      | Disk space        | Secondary (synchronizes timing)   |
| Solana    | Capital (stake)   | Auxiliary (orders events)         |
| **DOLI**  | **Time**          | **Primary consensus mechanism**   |

In Chia, Proof of Space determines who wins the block; VDF only synchronizes timing. In Solana, stake determines validators; VDF (Proof of History) only orders events within a slot. In both cases, VDF is auxiliary.

In DOLI, sequential time is the only resource that determines consensus. The economic bond is a Sybil filter, not a selection mechanism. This represents a paradigm shift: from "whoever has more resources wins" to "time passes equally for everyone".

## Quick Start

### Option 1: Pre-built Binary (Fastest)

```bash
# Download and install
curl -L https://raw.githubusercontent.com/e-weil/doli/main/scripts/update.sh | bash

# Run a node
doli-node run

# Generate a wallet
doli wallet new
```

### Option 2: Docker

```bash
# Run a mainnet node
docker run -d --name doli-node \
  -p 30303:30303 -p 8545:8545 \
  -v doli-data:/data \
  ghcr.io/e-weil/doli-node:latest

# View logs
docker logs -f doli-node
```

### Option 3: Build from Source

```bash
# Clone and build
git clone https://github.com/e-weil/doli.git
cd doli
cargo build --release

# Run
./target/release/doli-node run
```

### Network Selection

```bash
# Mainnet (default)
doli-node run

# Testnet
doli-node --network testnet run

# Devnet (local development)
doli-node --network devnet run
```

## Crate Structure

DOLI is implemented as a Rust workspace with the following crates:

| Crate | Description |
|-------|-------------|
| `doli-crypto` | Cryptographic primitives (BLAKE3, Ed25519, key management) |
| `doli-vdf` | VDF implementation (hash-chain with dynamic calibration) |
| `doli-core` | Core types, validation rules, and consensus parameters |
| `doli-storage` | Persistent storage layer (RocksDB) |
| `doli-network` | P2P networking (libp2p-based) with equivocation detection |
| `doli-node` | Full node binary |
| `doli-cli` | Command-line wallet and utilities |

## Documentation

- [Whitepaper (Spanish)](WHITEPAPER.md) - Complete technical specification
- [Architecture](docs/architecture.md) - System design and components
- [Protocol Specification](docs/protocol.md) - Detailed protocol rules
- [Roadmap](docs/roadmap.md) - Development timeline and milestones
- [Contributing](CONTRIBUTING.md) - How to contribute to the project

### API Documentation

Generate the Rust API documentation:

```bash
cargo doc --workspace --no-deps --open
```

## Technical Summary

### Consensus Mechanism

DOLI uses **Proof of Time** with two complementary mechanisms:

1. **Epoch Lookahead** (anti-grinding): Leaders are determined deterministically at epoch start
2. **Heartbeat VDF** (~700ms): Proves continuous sequential presence

| Network | Slot  | VDF Time | Purpose                        |
|---------|-------|----------|--------------------------------|
| All     | 60/10/5s | ~700ms | Heartbeat proof of presence    |

```
vdf_input = HASH("DOLI_HEARTBEAT_V1" || producer_key || slot || prev_hash)
vdf_output = hash_chain(vdf_input, iterations)  // ~10M iterations
```

**Why Epoch Lookahead prevents grinding:**
- Leaders determined by `slot % total_bonds` (deterministic)
- Selection uses bond distribution, NOT prev_hash
- Attackers cannot influence future leader selection
- Grinding current block is USELESS for next slot

**Two Types of Proof of Time:**
1. **Immediate Time (VDF)**: ~700ms heartbeat proves you're present NOW
2. **Historical Time (Longevity)**: Rights earned over weeks/months/years of presence

**Weight-Based Fork Choice**: The chain with the highest accumulated producer weight wins. Producer weight ranges from 1 (new) to 4 (veteran, based on seniority).

### Time Structure

| Unit    | Slots      | Duration    |
|---------|------------|-------------|
| Slot    | 1          | 10 seconds  |
| Epoch   | 360        | 1 hour      |
| Day     | 8,640      | 24 hours    |
| Era     | 12,614,400 | ~4 years    |

### Emission Schedule

| Era | Years  | Reward  | Cumulative   | % of Total |
|-----|--------|---------|--------------|------------|
| 1   | 0-4    | 1.0     | 12,614,400   | 50.00%     |
| 2   | 4-8    | 0.5     | 18,921,600   | 75.00%     |
| 3   | 8-12   | 0.25    | 22,075,200   | 87.50%     |
| 4   | 12-16  | 0.125   | 23,652,000   | 93.75%     |
| 5   | 16-20  | 0.0625  | 24,440,400   | 96.88%     |

### Producer Registration

To become a block producer:

1. Complete a VDF proof (base: 10 minutes, adjusts with demand)
2. Lock an activation bond (Era 1: 1,000 coins, decreases 30% per era)
3. Bond is locked for ~4 years and slashed for misbehavior

### Bond Stacking

Producers can stake 1-100 bonds to increase their block production share:

| Bonds | Investment | Blocks/Cycle | ROI |
|-------|------------|--------------|-----|
| 1 | 1,000 DOLI | 1 of 10 | 0.5% |
| 5 | 5,000 DOLI | 5 of 10 | 0.5% |
| 10 | 10,000 DOLI | 10 of 20 | 0.5% |

**Key:** Uses deterministic round-robin rotation (NOT lottery). More bonds = more blocks, same ROI %.

### Cryptographic Primitives

- **Hash function**: BLAKE3-256
- **Signatures**: Ed25519
- **VDF**: Hash-chain (iterated SHA-256, ~10M iterations)

## Networks

DOLI supports multiple networks with a single binary:

| Network | ID | Purpose | P2P Port | RPC Port |
|---------|-----|---------|----------|----------|
| Mainnet | 1   | Production network | 30303 | 8545 |
| Testnet | 2   | Public test network | 40303 | 18545 |
| Devnet  | 99  | Local development | 50303 | 28545 |

Each network has its own genesis, address prefix (`doli`, `tdoli`, `ddoli`), and isolated data directory.

## Protocol Parameters (Mainnet)

| Parameter           | Value                  |
|---------------------|------------------------|
| Genesis time        | 2026-02-01T00:00:00Z   |
| Slot duration       | 10 seconds             |
| Epoch duration      | 360 slots (1 hour)     |
| Block size limit    | 1,000,000 bytes        |
| Reward maturity     | 100 blocks             |
| Initial bond        | 1,000 coins            |
| Bond decay          | 30% per era            |
| Slashing (double)   | 100% of bond           |
| Exclusion period    | 60,480 slots (7 days)  |

## Network

- **Website**: [www.doli.network](https://www.doli.network)
- **Email**: doli@protonmail.com
- **GitHub**: [github.com/doli-network](https://github.com/doli-network)

## Security

DOLI is secure as long as honest participants collectively control more sequential computation capacity than any group of attackers. Unlike PoW systems, attackers cannot accelerate attacks by adding parallel hardware.

### Attack Cost

1. **Time cost**: Registration VDF scales with demand (up to 24 hours per identity)
2. **Economic cost**: Each identity requires a locked bond
3. **Risk**: Double production results in 100% bond slashing

## License

MIT License - see [LICENSE](LICENSE) for details.

---

*"Time is the most democratic resource. DOLI turns it into money."*

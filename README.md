# DOLI

**A Peer-to-Peer Electronic Cash System Based on Verifiable Time**

> Consensus weight emerges from bonded capital anchored by sequential time — not trust, not scale, and not purchasable acceleration.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

**Website:** [doli.network](https://doli.network) | **Explorer:** [explorer.doli.network](https://explorer.doli.network) | **Marketplace:** [doli.network/marketplace.html](https://doli.network/marketplace.html) | **Whitepaper:** [WHITEPAPER.md](WHITEPAPER.md)

## What is DOLI?

DOLI is a cryptocurrency where the scarce resource is **time** — the one resource that cannot be accumulated, parallelized, or purchased. One second passes at the same rate for an individual as for a nation-state.

- **No premine, no ICO, no treasury.** Every coin comes from block rewards.
- **No mining pools needed.** The protocol is the pool — epoch rewards are distributed on-chain to all qualified producers.
- **No special hardware.** Any CPU can participate. A $5/month VPS is sufficient.
- **Equal ROI for all.** A producer with 1 bond earns the same percentage return as one with 3,000 bonds.

## How It Works

**Block production** follows pure round-robin: every active producer receives equal block assignments regardless of stake. Bonds affect only reward distribution, not production frequency.

**Rewards** accumulate into an epoch pool (360 blocks, ~1 hour) and are distributed proportionally by bond weight to all producers who proved continuous presence via on-chain liveness attestations.

**Programmable outputs** without a virtual machine: NFTs, fungible tokens, AMM pools, lending, and cross-chain bridges are native UTXO output types with declarative spending conditions — no gas, no EVM, no shared mutable state.

| System | Scarce Resource | Selection | Min Hardware | Pools Needed |
|--------|----------------|-----------|-------------|-------------|
| Bitcoin | Energy | Lottery (hashpower) | ASIC ($5,000+) | Yes |
| Ethereum | Capital (32 ETH) | Lottery (stake) | Server ($100K+) | Yes |
| Solana | Capital (stake) | Stake-weighted | $10K+ server | Yes |
| **DOLI** | **Time** | **Deterministic round-robin** | **Any CPU ($5/mo)** | **Built-in** |

## Quick Start

```bash
# Install
curl -sSf https://doli.network/install.sh | bash

# Run a node
doli-node run

# Create a wallet
doli wallet new

# Check balance
doli balance
```

### Build from Source

```bash
git clone https://github.com/doli-network/doli.git
cd doli
cargo build --release
./target/release/doli-node run
```

## Architecture

DOLI is implemented as a Rust workspace:

| Crate | Description |
|-------|-------------|
| `crypto` | BLAKE3 hashing, Ed25519 + BLS12-381 signatures, bech32m addresses |
| `doli-core` | Transactions (27 types), validation, consensus parameters, conditions |
| `storage` | RocksDB state database, block store, UTXO set, producer set |
| `network` | libp2p P2P networking, gossipsub, Kademlia DHT, sync engine |
| `mempool` | Transaction pool with UTXO verification and chained tx support |
| `rpc` | JSON-RPC server (45+ methods) |
| `bridge` | Cross-chain atomic swap library (Bitcoin, Ethereum) |
| `wallet` | Transaction builder, UTXO selection, key management |
| `updater` | Auto-update system, hard fork scheduling |
| `doli-node` | Full node binary |
| `doli-cli` | Command-line wallet and utilities |

## Protocol Parameters

| Parameter | Value |
|-----------|-------|
| Block time | 10 seconds |
| Epoch | 360 slots (1 hour) |
| Era | 12,614,400 blocks (~4 years) |
| Total supply | 25,228,800 DOLI |
| Block reward (Era 1) | 1 DOLI |
| Halving | Every era (~4 years) |
| Bond unit | 10 DOLI |
| Max bonds/producer | 3,000 (30,000 DOLI) |
| Liveness threshold | 90% attestation per epoch |
| Unbonding period | 7 days |
| Slashing (double production) | 100% of bond |
| Hash function | BLAKE3-256 |
| Signatures | Ed25519 (transactions) + BLS12-381 (attestation aggregation) |
| Addresses | Bech32m (doli1...) |

## Native DeFi

All financial primitives are compiled UTXO output types — no VM, no gas, no reentrancy:

| Primitive | Output Type | Description |
|-----------|------------|-------------|
| Transfers | Normal | Standard P2PKH payments |
| Staking | Bond | Producer bonds with FIFO vesting |
| NFTs | UniqueAsset | On-chain art, royalties, batch minting |
| Tokens | FungibleAsset | User-issued fixed-supply tokens |
| AMM Pools | Pool + LPShare | Constant-product AMM (x*y=k), 0.3% fee |
| Lending | Collateral + LendingDeposit | Collateralized loans with TWAP oracle |
| Bridges | BridgeHTLC | Atomic swaps: BTC, ETH, BSC, LTC, XMR, ADA |
| Multisig | Multisig | N-of-M authorization |
| Timelocks | HTLC, Vesting | Hash + time locked outputs |

MEV and sandwich attacks are structurally impossible: the pool is a UTXO consumed atomically — two swaps referencing the same pool are mutually exclusive.

## Documentation

- [Whitepaper](WHITEPAPER.md) — Complete technical specification
- [Whitepaper (Spanish)](WHITEPAPER-es.md) — Especificacion tecnica completa
- [Architecture](docs/architecture.md) — System design and components
- [RPC Reference](docs/rpc_reference.md) — All 45+ JSON-RPC methods
- [CLI Reference](docs/cli.md) — Command-line wallet and node usage

## Networks

| Network | Address Prefix | Purpose |
|---------|---------------|---------|
| Mainnet | `doli1...` | Production network |
| Testnet | `tdoli1...` | Public test network |
| Devnet | `ddoli1...` | Local development |

```bash
doli-node run                      # Mainnet (default)
doli-node --network testnet run    # Testnet
doli-node --network devnet run     # Devnet
```

## License

MIT

---

*"Time is the only fair currency."*

I. Lozada &middot; [ivan@doli.network](mailto:ivan@doli.network) | A. Lozada &middot; [antonio@doli.network](mailto:antonio@doli.network)

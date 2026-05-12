# DOLI

**A Peer-to-Peer Electronic Cash System Built by Agents, for an Agentic Era**

> Consensus weight emerges from bonded capital anchored by sequential time — not trust, not scale, and not purchasable acceleration.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

**Website:** [doli.network](https://doli.network) | **Explorer:** [explorer.doli.network](https://explorer.doli.network) | **Marketplace:** [doli.network/marketplace.html](https://doli.network/marketplace.html) | **Whitepaper:** [WHITEPAPER.md](WHITEPAPER.md)

## What is DOLI?

DOLI is a cryptocurrency where the scarce resource is **time** — the one resource that cannot be accumulated, parallelized, or purchased. One second passes at the same rate for an individual as for a nation-state.

DOLI is built entirely by [OMEGA](https://github.com/omegacortex), a multi-agent AI workflow system where specialized agents — architect, developer, reviewer, auditor, QA — collaborate through shared institutional memory. Every line of code passes through multiple validation layers before it reaches the chain. This isn't a blockchain with an AI wrapper bolted on; it's a blockchain engineered from genesis by agents, with every design decision made for machine-readability first.

- **No premine, no ICO, no treasury.** Every coin comes from block rewards.
- **No mining pools needed.** The protocol is the pool — epoch rewards are distributed on-chain to all qualified producers.
- **No special hardware.** Any CPU can participate. A $5/month VPS is sufficient.
- **Equal ROI for all.** A producer with 1 bond earns the same percentage return as one with 3,000 bonds.

## Agent-Native by Design

Most blockchains were designed for human developers reading docs. Their errors are cryptic strings, their state is implicit, and their scheduling is opaque. Retrofitting good agent design onto a mature chain is extremely painful — Ethereum has been trying for years.

DOLI was designed during the agentic transition. Every interface speaks the language agents understand: structured data, stable codes, explicit state, and deterministic timing.

### Structured Errors for Self-Correction

47 machine-readable error codes with structured JSON responses. When a transaction fails, agents don't parse strings — they read `error_code`, understand the failure, and self-correct:

```json
{
  "code": -32002,
  "message": "validation failed: insufficient funds",
  "data": {
    "error_code": "INSUFFICIENT_FUNDS",
    "stage": "mempool_validation",
    "inputs": 500,
    "outputs": 1000
  }
}
```

Every error carries a **stage** field (`deserialization`, `mempool`, `mempool_validation`) so agents know exactly where in the pipeline the failure occurred.

### Explicit State via UTXO Model

Unlike account-based chains where agents must simulate the entire state transition to predict outcomes, DOLI's UTXO model makes every coin an individually queryable object. An agent can call `getUtxos`, select specific coins by outpoint, know the exact state before broadcasting, and get told which specific UTXO failed and why. No gas estimation guessing. No state simulation.

### Deterministic Scheduling

`getSlotSchedule` tells agents exactly which producer handles which slot. `getProducerSchedule` shows when a specific producer is next up. Timing is predictable, not probabilistic.

### Agent-Readable Skills

DOLI ships with 20 [skill files](.claude/skills/) committed to the repository — structured knowledge maps that give any AI agent deep understanding of the codebase. Each skill covers a domain (consensus, networking, storage, RPC, CLI, wallet, etc.) with indexed entry points, data flows, constraints, and patterns. An agent can grep the [SKILLS-INDEX](.claude/skills/SKILLS-INDEX.md) for any keyword and jump directly to the relevant code section.

These aren't documentation written for humans to skim. They're machine-optimized navigation data: `@INDEX` headers for offset-based loading, line-range references for surgical reads, and cross-domain dependency maps. Any agent — not just OMEGA — can use them to understand, operate, debug, and extend the blockchain.

## How It Works

**Block production** follows deterministic bond-weighted scheduling: each producer receives block assignments proportional to their bond count. Both production frequency and epoch rewards scale linearly with bonds — ensuring identical ROI percentage for all participants.

**Rewards** accumulate into an epoch pool (360 blocks, ~1 hour) and are distributed proportionally by bond weight to all producers who proved continuous presence via on-chain liveness attestations.

**Programmable outputs** without a virtual machine: NFTs, fungible tokens, AMM pools, lending, and cross-chain bridges are native UTXO output types with declarative spending conditions — no gas, no EVM, no shared mutable state.

| System | Scarce Resource | Selection | Min Hardware | Pools Needed |
|--------|----------------|-----------|-------------|-------------|
| Bitcoin | Energy | Lottery (hashpower) | ASIC ($5,000+) | Yes |
| Ethereum | Capital (32 ETH) | Lottery (stake) | Server ($100K+) | Yes |
| Solana | Capital (stake) | Stake-weighted | $10K+ server | Yes |
| **DOLI** | **Time** | **Deterministic bond-weighted** | **Any CPU ($5/mo)** | **Built-in** |

## Quick Start

A basic VPS with 2 cores, 2 GB RAM, and 20 GB SSD (~$4/month) is all you need.

```bash
# 0. Remove previous installation (if any)
sudo doli service stop 2>/dev/null; sudo rm -f /usr/bin/doli /usr/bin/doli-node

# 1. Install
curl -sSfL https://doli.network/install.sh | sudo sh

# 2. Create wallet + BLS key
doli init

# 3. Start the node
sudo doli service install

# 4. Verify your node is synced
doli chain

# 5. Request 10.01 DOLI from the faucet
#    Open a faucet request — paste your address (doli info) and doli chain output
#    Wait ~7 minutes for the bot to process

# 6. Verify you received the DOLI
doli balance

# 7. Register as producer
doli producer register --bonds 1
```

**Permission model:** Wallet commands (`doli send`, `doli balance`) never need sudo. System commands (`doli service install`, `doli upgrade`, `doli snap`) do.

Full guide: [doli.network/guide.html](https://doli.network/guide.html)

### Build from Source

```bash
git clone https://github.com/doli-network/doli.git
cd doli
cargo build --release
sudo cp target/release/doli-node target/release/doli /usr/bin/
```

## Architecture

DOLI is implemented as a Rust workspace:

| Crate | Description |
|-------|-------------|
| `crypto` | BLAKE3 hashing, Ed25519 + BLS12-381 signatures, bech32m addresses |
| `doli-core` | Transactions (30 types), validation, consensus parameters, conditions, EpochState |
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
- [Agentic Readiness Report](docs/agentic-era-readiness-report.md) — How DOLI is designed for AI agents
- [Skills Index](.claude/skills/SKILLS-INDEX.md) — Agent-optimized codebase navigation

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

D. Guanipa &middot; [daniel@doli.network](mailto:daniel@doli.network) | I. Lozada &middot; [ivan@doli.network](mailto:ivan@doli.network) | A. Lozada &middot; [antonio@doli.network](mailto:antonio@doli.network)

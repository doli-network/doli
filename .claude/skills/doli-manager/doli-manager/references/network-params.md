# Network Parameters & Constants

## Networks

| Field | Mainnet | Testnet | Devnet |
|-------|---------|---------|--------|
| ID | 1 | 2 | 99 |
| Address Prefix | `doli` | `tdoli` | `ddoli` |
| P2P Port | 30300 | 40300 | 50300 |
| RPC Port | 8500 | 18500 | 28500 |
| Metrics Port | 9000 | 19000 | 29000 |
| Magic Bytes | `D0 11 00 01` | `D0 11 00 02` | `D0 11 00 63` |
| Genesis | 2026-02-01 | 2026-01-29 | Dynamic |

## Consensus Constants

| Parameter | Mainnet | Devnet | Note |
|-----------|---------|--------|------|
| Slot Duration | 10s | 10s | `SLOT_DURATION` |
| Slots per Epoch | 360 (1h) | 360 | `SLOTS_PER_EPOCH` |
| Blocks per Reward Epoch | 360 | 60 | |
| Era | 12.6M slots (~4y) | 576 | Halving trigger |
| VDF Block | 800K iter (~55ms) | 800K | `T_BLOCK` |
| VDF Register | 600M iter (~10m) | 5M | `T_REGISTER_BASE` |
| Bond Unit | 10 DOLI | 1 DOLI | `BOND_UNIT` |
| Unbond Delay | 7 days | 10 min | `WITHDRAWAL_DELAY_SLOTS` |
| Fallback Ranks | 5 | 5 | `MAX_FALLBACK_RANKS` |
| Fallback Timeout | 2000ms | 2000ms | `FALLBACK_TIMEOUT_MS` |
| Clock Drift Max | 1s / 200ms | same | `MAX_DRIFT` / `MAX_DRIFT_MS` |
| Coinbase Maturity | 100 | 10 | Blocks before spendable |
| Max Block Size | 1MB + header | same | |

## Economics

- Supply cap: ~25.2M DOLI
- Rewards: 100% to producer
- Halving: every era (~4 years)
- Seniority weights: Year 0 (1x) to Year 3+ (4x)
- Fork choice: heaviest chain (seniority-weighted)
- Burnt: slashing (100%), early withdrawal (75%->0%), registration fees

## Transaction Types

| Value | Type | Description |
|-------|------|-------------|
| 0 | Transfer | Standard value transfer |
| 1 | Register | Producer registration |
| 2 | Exit | Producer exit |
| 4 | ClaimBond | Claim unbonded stake |
| 5 | Slash | Slash equivocating producer |
| 6 | Coinbase | Block reward |
| 7 | AddBond | Increase stake |
| 8 | WithdrawalRequest | Start withdrawal delay |
| 9 | WithdrawalClaim | Claim after delay |
| 10 | EpochReward | Epoch presence reward |
| 11 | MaintainerAdd | Add maintainer |
| 12 | MaintainerRemove | Remove maintainer |

## DNS Seeds

| Record | Resolves To |
|--------|-------------|
| `seed1.doli.network` | 72.60.228.233 (N1) |
| `seed2.doli.network` | 72.60.228.233 (N1) |

Hardcoded in `crates/core/src/network_params.rs`.

## Environment Configuration

Network-specific overrides via `~/.doli/{network}/.env`:

**Always configurable:** `DOLI_P2P_PORT`, `DOLI_RPC_PORT`, `DOLI_METRICS_PORT`, `DOLI_BOOTSTRAP_NODES`

**Devnet only (locked for mainnet):** `DOLI_SLOT_DURATION`, `DOLI_GENESIS_TIME`, `DOLI_BOND_UNIT`, `DOLI_INITIAL_REWARD`, `DOLI_BLOCKS_PER_YEAR`, `DOLI_VDF_ITERATIONS`, `DOLI_FALLBACK_TIMEOUT_MS`

## Crate Map

| Crate | Purpose |
|-------|---------|
| `core` | Consensus, types, scheduler, discovery |
| `crypto` | BLAKE3, Ed25519, Merkle, bech32m addresses |
| `vdf` | Wesolowski (registration) & hash-chain (block) |
| `network` | libp2p gossipsub, sync, equivocation |
| `storage` | RocksDB (headers, bodies, UTXO, producers) |
| `mempool` | Tx pool, double-spend checks |
| `updater` | 3/5 multisig auto-update |
| `rpc` | JSON-RPC API server (Axum) |

## RPC Methods

| Method | Description |
|--------|-------------|
| `getChainInfo` | Best height, slot, hash, network |
| `getBalance` | Balance for address (doli1... or hex) |
| `getUtxos` | UTXOs for address |
| `getHistory` | Transaction history |
| `sendTransaction` | Submit signed tx |
| `getBlockByHash` | Block by hash |
| `getBlockByHeight` | Block by height |
| `getProducer` | Producer info by pubkey |
| `getProducers` | All producers |
| `getNetworkInfo` | Peer count, sync status |
| `getMempoolInfo` | Mempool stats |
| `getEpochInfo` | Current epoch details |
| `getNetworkParams` | Bond unit, slot duration, etc. |
| `getUpdateStatus` | Pending updates, veto status |
| `getMaintainerSet` | Current maintainers |
| `submitVote` | Submit governance vote |
| `getNodeInfo` | Version, platform |

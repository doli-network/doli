# CLAUDE.md - Project Brain

## 🚨 CRITICAL RULES
1. **Environment**: All commands **MUST** run via Nix:
   `nix --extra-experimental-features "nix-command flakes" develop --command bash -c "<command>"`
2. **Truth Hierarchy**: `WHITEPAPER.md` (Law) > `specs/` (Tech) > `docs/` (User) > Code.
3. **Pre-Commit Gate**:
   - Sync docs (`specs/` vs code).
   - Update `CLAUDE.md` if arch/constants change.
   - Verify: `build`, `clippy`, `test`.
   - Use `/fix-bug` for bugs. **No masking symptoms.**
4. **Filtering**: `command 2>&1 | grep -iE "error|warn|fail|pass" | head -20`

## 🛠 Commands (Wrapped)
| Action | Command (Implicitly wrapped in Nix) |
|--------|-------------------------------------|
| **Build** | `cargo build`, `cargo build --release`, `cargo clippy`, `cargo fmt` |
| **Test** | `cargo test`, `cargo test -p core` |
| **Fuzz** | `cd testing/fuzz && cargo +nightly fuzz run <target>` |
| **Run Node** | `cargo run -p doli-node -- (--network testnet) run` |
| **Run Wallet** | `cargo run -p doli-cli -- <command>` |

## 🧠 System Architecture
**Type**: Proof of Time (PoT). **Resource**: Time (VDF). **Selection**: Deterministic bond-weighted round-robin.
**Consensus**: Heaviest chain (Seniority-weighted). **Engine**: RocksDB + libp2p + Axum.

### Crates & Responsibilities
| Crate | Purpose | Key Files |
|-------|---------|-----------|
| `core` | Consensus, Types, Scheduler | `consensus.rs`, `scheduler.rs`, `validation.rs`, `discovery/` |
| `crypto` | BLAKE3, Ed25519, Merkle | `hash.rs`, `keys.rs`, `merkle.rs` (Domain separated) |
| `vdf` | Wesolowski (Reg) & Hash-Chain (Block) | `vdf.rs`, `proof.rs` (GMP/Rug) |
| `network` | Gossipsub, Sync, Equivocation | `service.rs`, `sync/`, `gossip.rs` |
| `storage` | RocksDB (Headers, Bodies, UTXO) | `block_store.rs`, `utxo.rs`, `producer.rs` |
| `mempool` | Tx Pool, Double-spend checks | `pool.rs`, `policy.rs` |
| `updater` | 3/5 Multisig Auto-Update | `lib.rs`, `vote.rs` |

### ⚙️ Consensus Constants
| Param | Mainnet | Devnet | Note |
|-------|---------|--------|------|
| **Slot** | 10s | 10s | `SLOT_DURATION` |
| **Epoch** | 360 slots (1h) | 360 | `SLOTS_PER_EPOCH` |
| **Era** | 12.6M slots (~4y) | 576 | Halving trigger |
| **VDF Block** | 10M iter (~700ms) | 10M | `T_BLOCK` |
| **VDF Reg** | 600M iter (~10m) | 5M | `T_REGISTER_BASE` (Anti-Sybil) |
| **Bond** | 100 DOLI | 1 DOLI | `BOND_UNIT` |
| **Unbond** | 7 days | 10m | `WITHDRAWAL_DELAY_SLOTS` |
| **Selection**| `slot % bonds` | - | Primary window 0-3s |

### 🌐 Network & Ports
| Net | ID | Port (P2P/RPC) | Magic | Prefix | Genesis |
|-----|----|----------------|-------|--------|---------|
| Main| 1 | 30303 / 8545 | `D0 11 00 01` | `doli` | 2026-02-01 |
| Test| 2 | 40303 / 18545 | `D0 11 00 02` | `tdoli`| 2026-01-29 |
| Dev | 99 | 50303 / 28545 | `D0 11 00 63` | `ddoli`| Dynamic |

## 💰 Economics (Deflationary)
- **Supply**: ~25.2M DOLI. **Rewards**: 100% to producer. **Halving**: Every Era (~4y).
- **Weights**: Year 0-1 (1x) → Year 3+ (4x). **Fork Choice**: Heaviest weight.
- **Burnt**: Slashing (100%), Early Withdrawal (75%→0% over 4y), Reg Fees.

### Bond Vesting (Withdrawal Penalty)
| Age | Penalty |
|-----|---------|
| <1y | 75% Burn|
| 1-2y| 50% Burn|
| 2-3y| 25% Burn|
| 3y+ | 0% |

## 🛡 Validation & Security
- **Block**: Ver=1, Time advances, Max size (1MB+), Merkle match, VDF valid.
- **Tx**: Sig valid, Inputs exist, No double-spend. Malleability: Sig excluded from hash.
- **Slashing**: Double-production = 100% BURN. Detection: Network + SignedSlotsDB.
- **Governance**: 3/5 Maintainers. Veto: 40% stake (7d period).

## 📂 File Map
### Core
- `consensus.rs`: Constants, Bond logic.
- `scheduler.rs`: Round-robin logic (`select_producer`).
- `validation.rs`: 37 error types (`InvalidTimestamp`, `DoubleSpend`).
- `discovery/`: Signed announcements, CRDT (`gset.rs`), Bloom filters.

### Network (`sync/`)
- `manager.rs`: Orch. `reorg.rs`: Fork choice (Depth=100).
- `equivocation.rs`: Detect double-prop (`EquivocationProof`).
- `headers.rs` / `bodies.rs`: Header-first download.

### Storage
- CFs: `headers`, `bodies`, `height_index`, `slot_index`, `presence`.
- `utxo.rs`: HashMap driven. `producer.rs`: Registry.

### Transaction Types (`TxType`)
0:Transfer, 1:Register, 2:Exit, 4:ClaimBond, 5:Slash, 6:Coinbase, 7:AddBond, 8/9:Withdrawal, 10:EpochReward, 11/12:Maintainer.

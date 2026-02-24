# DOLI Mainnet Genesis Launch Guide

Complete technical documentation for launching DOLI mainnet.

## Key Points

| Component | Location | Configurable? |
|-----------|----------|---------------|
| Genesis producers | Self-register during genesis phase | No chainspec keys |
| Maintainer keys (5) | On-chain (first 5 producers) | Yes - via governance |

- **Genesis producers**: FREE, no bond, self-register during 7-day genesis phase
- **Regular producers**: 10 DOLI bond, `doli producer register`
- **Maintainer keys**: Derived from first 5 registered producers on-chain; bootstrap keys in binary used as fallback before chain sync

---

## Table of Contents

1. [Pre-Launch Checklist](#1-pre-launch-checklist)
2. [Critical Lessons from Testnet](#2-critical-lessons-from-testnet)
3. [Maintainer Keys](#3-maintainer-keys-auto-update-system)
4. [Genesis Producer Setup](#4-genesis-producer-setup)
5. [Network Parameters](#5-network-parameters)
6. [DNS Configuration](#6-dns-configuration)
7. [Launch Procedure](#7-launch-procedure)
8. [Verification Steps](#8-verification-steps)
9. [Troubleshooting](#9-troubleshooting)
10. [Epoch Rewards Architecture](#10-epoch-rewards-architecture)

---

## 1. Pre-Launch Checklist

**CRITICAL**: Complete ALL items before genesis. Any mistake requires a full chain reset.

### Code Verification

- [ ] `BOOTSTRAP_MAINTAINER_KEYS` in `crates/updater/src/lib.rs` contains 5 real Ed25519 public keys
- [ ] `is_using_placeholder_keys()` returns `false`
- [ ] All unit tests pass: `cargo test`
- [ ] Release build compiles: `cargo build --release`

### Chainspec Verification

- [ ] Chainspec at `resources/chainspec/mainnet.json` has `genesis_producers: []`
- [ ] Genesis timestamp and consensus parameters are correct
- [ ] Producers self-register during the 7-day genesis phase (no pre-generated keys)

### Parameter Verification

- [ ] Block reward: 1 DOLI (100,000,000 atomic units)
- [ ] Slot duration: 10 seconds
- [ ] Epoch length: 360 blocks (1 hour)
- [ ] Genesis timestamp: 2026-02-01T00:00:00Z (Unix: 1769904000)
- [ ] BlockStore query methods for epoch rewards exist

### Infrastructure

- [ ] DNS records `seed1.doli.network` and `seed2.doli.network` point to seed server IP
- [ ] Firewall allows port 30303 (P2P) and 8545 (RPC)
- [ ] Producer servers ready with wallet files
- [ ] Binary deployed to all servers

---

## 2. Critical Lessons from Testnet

### Bug #1: Genesis Producer Pubkey Mismatch

**What happened**: Genesis producers were registered with incorrect public keys that didn't match the wallet files. Nodes couldn't produce blocks because their actual pubkey wasn't in the registered producer list.

**Root cause**: Manual copying of pubkeys from wallet files to `genesis.rs` introduced human error.

**Solution implemented**: **Industry-standard chainspec system**

Instead of manually copying pubkeys, we now use:
1. `scripts/generate_chainspec.sh` - Automatically extracts pubkeys from wallet files
2. `--chainspec` flag - Loads genesis config from JSON file at runtime
3. Validation - Chainspec rejects placeholder keys and validates pubkey format

```bash
# OLD (error-prone): Manually copy pubkeys to genesis.rs
# NEW (safe): Generate chainspec from wallets automatically
./scripts/generate_chainspec.sh mainnet ~/.doli/mainnet/producer_keys mainnet.json
./doli-node --network mainnet --chainspec mainnet.json run --producer
```

This follows the same pattern as Ethereum, Cosmos, and Polkadot.

### Bug #2: Epoch State Not Persisting (Superseded)

**What happened**: After node restart, epoch tracking reset to zero. Rewards were never distributed because epoch counters reset on every restart.

**Original fix**: Added persistent epoch state fields to ChainState.

**Final resolution**: Replaced with deterministic BlockStore-based rewards (see `REWARDS.md`).

**Status**: SUPERSEDED - Deterministic rewards system implemented. See [Epoch Rewards Architecture](#epoch-rewards-architecture) below.

### Bug #3: RPC Showed Wrong Network

**What happened**: RPC `getChainInfo` always returned "mainnet" even on testnet.

**Resolution**: Added `.with_network()` call in node.rs when creating RPC context.

**Status**: FIXED.

---

## 3. Maintainer Keys (Auto-Update System)

5 maintainers control protocol updates with a 3-of-5 signature threshold.

### Key Derivation

Maintainer keys are derived from the **first 5 registered producers** on-chain.
Once the node has synced, these on-chain keys are used for release signature verification.

| Source | When Used |
|--------|-----------|
| On-chain (first 5 producers by registration height) | Running node with synced chain |
| Bootstrap keys in `crates/updater/src/lib.rs` | Before chain sync, CLI commands |

| Parameter | Value |
|-----------|-------|
| Keys | 5 Ed25519 public keys (derived from chain) |
| Threshold | 3-of-5 signatures required |

### Bootstrap Keys

These keys are hardcoded as fallback before chain state is available:

```rust
pub const BOOTSTRAP_MAINTAINER_KEYS: [&str; 5] = [
    "721d2bc74ced1842eb77754dac75dc78d8cf7a47e10c83a7dc588c82187b70b9",
    "d0c62cb4e143d548271eb97c4651e77b6cf52909a016bda6fb500c3bc022298d",
    "9fac605a1ebf2acfa54ef8406ab66d604df97d63da1f1ab6a45561c7e51be697",
    "97bdb0a9a52d4ed178c2307e3eb17e316b57d098af095b9cefc0c69d73e8817f",
    "82ed55afabfe38d826c1e2b870aefcc9ed0de45e5620adb4f858e6f47c8d4096",
];
```

### Changing Maintainers

Maintainer changes happen via on-chain governance transactions (3/5 multisig):
- `doli-node maintainer add --target <pubkey> --key <maintainer.json>`
- `doli-node maintainer remove --target <pubkey> --key <maintainer.json>`

---

## 4. Genesis Producer Setup

**Genesis producers are FREE** - no bond required. Pre-registered at block 0.

| Type | Bond | How |
|------|------|-----|
| Genesis producer | None | Self-register during 7-day genesis phase |
| Regular producer | 10 DOLI | `doli producer register` |

### Regular Producer (Joining After Genesis)

```bash
doli new -w producer.json
doli-node --network mainnet run --producer --producer-key producer.json
doli producer register --bonds 1 -w producer.json
```

### Genesis Producer (Network Launch)

```bash
# 1. Create wallets
for i in 1 2 3 4 5; do
    doli new -w ~/.doli/mainnet/producer_$i.json
done

# 2. Generate chainspec (auto-extracts pubkeys)
./scripts/generate_chainspec.sh mainnet ~/.doli/mainnet mainnet.json

# 3. Run node
doli-node --network mainnet --chainspec mainnet.json run \
    --producer --producer-key ~/.doli/mainnet/producer_1.json
```

---

## 5. Network Parameters

### Consensus Parameters

| Parameter | Value | Code Location |
|-----------|-------|---------------|
| Block Reward | 1 DOLI (100,000,000 atomic) | `network.rs:initial_reward()` |
| Slot Duration | 10 seconds | `consensus.rs:SLOT_DURATION` |
| Epoch Length | 360 blocks (1 hour) | `consensus.rs:SLOTS_PER_EPOCH` |
| Reward Epoch | 360 blocks (1 hour) | `consensus.rs:SLOTS_PER_REWARD_EPOCH` |
| Bond Amount | 10 DOLI | `consensus.rs:BOND_UNIT` |
| Total Supply | 100,000,000 DOLI | `consensus.rs:TOTAL_SUPPLY` |

### Genesis Configuration

| Parameter | Value |
|-----------|-------|
| Timestamp | 2026-02-01T00:00:00Z (Unix: 1769904000) |
| Message | "Time is the only fair currency. 01/Feb/2026" |
| Genesis Producers | None in chainspec — producers self-register during 7-day genesis phase |
| Genesis Blocks | 60,480 (7 days of open registration) |

### Network Ports

| Port | Purpose |
|------|---------|
| 30303 | P2P (libp2p) |
| 8545 | JSON-RPC |
| 9090 | Metrics (Prometheus) |

---

## 6. DNS Configuration

### Required DNS Records

```
seed1.doli.network    A       72.60.228.233
seed2.doli.network    A       72.60.228.233
```

### Seed Server Requirements

- Static IP address
- Ports 30303 and 8545 open
- 99.9% uptime SLA recommended
- Located in geographically central location for low latency

### Bootstrap Configuration

Nodes connect automatically using the hardcoded defaults in `network_params.rs`.
No `--bootstrap` flag is needed:

```bash
./doli-node --network mainnet run --chainspec resources/chainspec/mainnet.json --producer --producer-key producer.json
```

The `<PEER_ID>` is printed in logs when seed node starts:
```
Local peer id: 12D3KooW...
```

---

## 7. Launch Procedure

### Phase 1: Final Preparation (T-24h)

```bash
# 1. Create wallets
for i in 1 2 3 4 5; do doli new -w producer_$i.json; done

# 2. Generate chainspec
./scripts/generate_chainspec.sh mainnet . mainnet.json

# 3. Build and test
cargo test && cargo build --release

# 4. Deploy to servers
# Copy binary, chainspec, and wallet files to each server
```

### Phase 2: Pre-Genesis (T-1h)

1. Clear any existing data:
```bash
rm -rf ~/.doli/mainnet/node*/data/
```

2. Verify DNS resolves correctly:
```bash
dig seed1.doli.network
dig seed2.doli.network
```

3. Verify firewall rules:
```bash
sudo ufw status
# Should show 30303/tcp ALLOW
```

4. Verify chainspec is present and valid:
```bash
cat resources/chainspec/mainnet.json | jq '.genesis_producers | length'
# Should output: 5
```

### Phase 3: Genesis (T-0)

```bash
# 1. Start seed node
doli-node --network mainnet --chainspec mainnet.json run \
    --producer --producer-key producer_1.json

# 2. Get peer ID from logs
grep "Local peer id" node.log

# 3. Start other nodes (on other servers)
#    Bootstrap is automatic via seed1/seed2.doli.network defaults
doli-node --network mainnet --chainspec mainnet.json run \
    --producer --producer-key producer_2.json
```

---

## 8. Verification Steps

### Immediate Checks (Within 1 minute)

```bash
# 1. Check chain is producing blocks
curl -s -H "Content-Type: application/json" http://127.0.0.1:8545 \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}'
# Expected: height > 0, network = "mainnet"

# 2. Verify producers registered
curl -s -H "Content-Type: application/json" http://127.0.0.1:8545 \
    -d '{"jsonrpc":"2.0","method":"getProducers","params":{"active_only":true},"id":1}'
# Expected: producers self-registered during genesis phase

# 3. Check all nodes producing
for i in 1 2 3 4 5; do
    echo "=== Node $i ==="
    grep "produced at height" node$i/node.log | tail -2
done
```

### 1-Hour Check (After First Epoch)

```bash
# Check height (should be ~360 after 1 hour)
curl -s -H "Content-Type: application/json" http://127.0.0.1:8545 \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' \
    | jq '.result.bestHeight'

# Verify epoch reward distribution by checking block 361 for EpochReward transactions
curl -s -H "Content-Type: application/json" http://127.0.0.1:8545 \
    -d '{"jsonrpc":"2.0","method":"getBlock","params":{"height":361},"id":1}' \
    | jq '.result.transactions[] | select(.type == "EpochReward")'
# Expected: EpochReward transactions for each producer who produced blocks
```

### Node Restart Test (Critical!)

```bash
# Stop a node (production: systemd; devnet: pkill acceptable)
sudo systemctl stop doli-mainnet-node3  # production
# pkill -f "doli-node.*node3"           # devnet only

# Wait 30 seconds
sleep 30

# Restart
./doli-node --network mainnet -d node3/data run \
    --producer \
    --producer-key producer_keys/producer_3.json \
    --p2p-port 30305 \
    --rpc-port 8547 \
    --bootstrap "$BOOTSTRAP" \
    > node3/node.log 2>&1 &

# Verify chain state loaded correctly (node resumes from BlockStore)
grep -i "Loaded chain state\|Resuming" node3/node.log | head -5
```

---

## 9. Troubleshooting

### Blocks Not Being Produced

**Symptom**: Chain stuck at height 0

**Check 1**: Are producers registered?
```bash
curl -s -H "Content-Type: application/json" http://127.0.0.1:8545 \
    -d '{"jsonrpc":"2.0","method":"getProducers","params":{},"id":1}'
```
If empty: Genesis producers not initialized. Check `main.rs` initialization code.

**Check 2**: Do pubkeys match?
```bash
# From RPC
curl -s -H "Content-Type: application/json" http://127.0.0.1:8545 \
    -d '{"jsonrpc":"2.0","method":"getProducers","params":{},"id":1}' | jq '.result[].publicKey'

# From wallet
cat producer_keys/producer_1.json | jq '.addresses[0].public_key'
```
If different: **CRITICAL BUG** - Pubkeys in genesis.rs don't match wallet files!

**Check 3**: Is it my turn?
```bash
grep "Production check\|it's my turn\|Producing block" node1/node.log | tail -10
```

### Wrong Network Shown

**Symptom**: RPC shows "mainnet" on testnet (or vice versa)

**Fix**: Ensure `.with_network()` is called in node.rs:
```rust
let context = RpcContext::new(...)
    .with_network(self.config.network.name().to_string())
```

### Epoch Rewards Not Distributing

**Symptom**: After 360 blocks, no rewards distributed

**Check**: Epoch rewards are calculated deterministically from the BlockStore (see [Epoch Rewards Architecture](#epoch-rewards-architecture)). At each epoch boundary, the producer queries the BlockStore to determine what rewards are due and includes `EpochReward` transactions. To verify:

```bash
# Check for EpochReward transactions in recent blocks
curl -s -H "Content-Type: application/json" http://127.0.0.1:8545 \
    -d '{"jsonrpc":"2.0","method":"getBlock","params":{"height":361},"id":1}' | jq '.result.transactions'
```

If missing, check:
1. The producer is running in `EpochPool` reward mode (default)
2. BlockStore query methods are returning correct data
3. The producer's slot calculation matches the epoch boundary

**Key behavior**: If the epoch boundary slot is empty, the next block that gets produced will distribute the rewards. Empty slots reduce the reward pool (no phantom rewards).

---

## 10. Epoch Rewards Architecture

DOLI uses a deterministic, BlockStore-based reward system. This design ensures all nodes calculate identical rewards without maintaining local state.

### Core Principle

> **Everything is derived from the BlockStore. Zero local state.**

Any node can independently verify rewards by reading the same blocks. No synchronization of reward state is required between nodes.

### Key Properties

| Property | Description |
|----------|-------------|
| **No local state** | Epoch tracking is derived, never stored in memory or ChainState |
| **Restart-safe** | Node can restart at any time; rewards recalculate from BlockStore |
| **Sync-safe** | New nodes verify all historical rewards during sync |
| **Fork-safe** | Chain reorganizations automatically recalculate from the new fork |

### How It Works

1. **Epoch detection**: `current_epoch = current_slot / 360`
2. **Boundary check**: Query `BlockStore::get_last_rewarded_epoch()` to find the most recent epoch with distributed rewards
3. **Trigger**: If `current_epoch > last_rewarded_epoch`, rewards are due
4. **Calculation**: Query `BlockStore::get_blocks_in_slot_range()` for the epoch's blocks
5. **Distribution**: Create `EpochReward` transactions proportional to blocks produced

### Empty Slot Handling

| Scenario | Behavior |
|----------|----------|
| Empty slots within epoch | Reduce the reward pool (no phantom rewards for unproduced blocks) |
| Empty boundary slot | Next block distributes rewards for the previous epoch |
| Multiple empty epochs | Catch-up one epoch per block until current |

### Multi-Epoch Catch-up

If multiple epochs pass without blocks (e.g., network outage), the first block back distributes rewards for one epoch. Each subsequent block distributes the next epoch until caught up:

```
Slot 1080 (epoch 3), last_rewarded = 0:

Block N+0: epoch 3 > last_rewarded 0 → distribute epoch 1
Block N+1: epoch 3 > last_rewarded 1 → distribute epoch 2
Block N+2: epoch 3 > last_rewarded 2 → distribute epoch 3
Block N+3: epoch 3 == last_rewarded 3 → normal block, no rewards
```

### Verification

Validators independently recalculate expected rewards from the BlockStore and reject blocks with incorrect `EpochReward` transactions. The validation is exact-match, not approximate.

For implementation details, see `REWARDS.md` in the repository root.

---

## Appendix A: File Locations

### Chainspec (genesis producers - configurable)

| File | Purpose |
|------|---------|
| `resources/chainspec/mainnet.json` | Mainnet genesis producers |
| `resources/chainspec/testnet.json` | Testnet genesis producers |
| `scripts/generate_chainspec.sh` | Generate chainspec from wallet files |
| `crates/core/src/chainspec.rs` | Chainspec parsing and validation |

### Binary (bootstrap maintainer keys)

| File | Purpose |
|------|---------|
| `crates/updater/src/lib.rs` | **5 bootstrap maintainer keys** (fallback before chain sync) |

### Other

| File | Purpose |
|------|---------|
| `crates/core/src/genesis.rs` | Genesis configuration (fallback) |
| `crates/core/src/network.rs` | Network parameters |
| `crates/storage/src/chain_state.rs` | Chain state (tip, supply tracking) |
| `crates/storage/src/block_store.rs` | Block storage and epoch reward queries |
| `bins/node/src/main.rs` | `--chainspec` flag |
| `bins/node/src/node.rs` | Production logic |

## Appendix B: Key Constants

```rust
// Block timing
pub const SLOT_DURATION: u64 = 10;          // 10 seconds per block
pub const SLOTS_PER_EPOCH: u32 = 360;       // 1 hour

// Economics
pub const INITIAL_REWARD: u64 = 100_000_000;  // 1 DOLI per block
pub const BOND_UNIT: u64 = 1_000_000_000;     // 10 DOLI per bond
pub const TOTAL_SUPPLY: u64 = 100_000_000 * DOLI; // 100M DOLI

// Governance
pub const VETO_PERIOD: Duration = Duration::from_secs(7 * 24 * 3600);  // 7 days
pub const GRACE_PERIOD: Duration = Duration::from_secs(48 * 3600);     // 48 hours
pub const VETO_THRESHOLD_PERCENT: u8 = 40;
pub const REQUIRED_SIGNATURES: usize = 3;  // 3-of-5 maintainers
```

---

**Document Version**: 2.3
**Last Updated**: 2026-01-31
**Author**: DOLI Core Team

**Changelog:**
- v2.3: Added comprehensive Epoch Rewards Architecture section (BlockStore-based design)
- v2.2: Updated for deterministic BlockStore-based rewards (removed epoch state persistence references)
- v2.4: Unified maintainer system - on-chain keys with bootstrap fallback
- v2.1: Clarified maintainer keys are hardcoded in binary (not chainspec)
- v2.0: Added industry-standard chainspec system for genesis configuration

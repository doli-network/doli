# DOLI Mainnet Genesis Launch Guide

Complete technical documentation for launching DOLI mainnet. This document covers every critical detail to avoid the mistakes that occurred during testnet launch.

---

## Table of Contents

1. [Pre-Launch Checklist](#1-pre-launch-checklist)
2. [Critical Lessons from Testnet](#2-critical-lessons-from-testnet)
3. [Maintainer Key Generation](#3-maintainer-key-generation)
4. [Genesis Producer Setup](#4-genesis-producer-setup)
5. [Network Parameters](#5-network-parameters)
6. [DNS Configuration](#6-dns-configuration)
7. [Launch Procedure](#7-launch-procedure)
8. [Verification Steps](#8-verification-steps)
9. [Troubleshooting](#9-troubleshooting)

---

## 1. Pre-Launch Checklist

**CRITICAL**: Complete ALL items before genesis. Any mistake requires a full chain reset.

### Code Verification

- [ ] `MAINTAINER_KEYS` in `crates/updater/src/lib.rs` contains 5 real Ed25519 public keys
- [ ] `is_using_placeholder_keys()` returns `false`
- [ ] Genesis producer pubkeys match actual wallet files (LESSON FROM TESTNET!)
- [ ] All unit tests pass: `cargo test`
- [ ] Release build compiles: `cargo build --release`

### Parameter Verification

- [ ] Block reward: 1 DOLI (100,000,000 atomic units)
- [ ] Slot duration: 10 seconds
- [ ] Epoch length: 360 blocks (1 hour)
- [ ] Genesis timestamp: 2026-02-01T00:00:00Z (Unix: 1769904000)
- [ ] Epoch state persistence fields exist in ChainState

### Infrastructure

- [ ] DNS record `mainnet.doli.network` points to seed server IP
- [ ] Firewall allows port 30303 (P2P) and 8545 (RPC)
- [ ] 5 producer servers ready with wallet files
- [ ] Binary deployed to all 5 servers

---

## 2. Critical Lessons from Testnet

### Bug #1: Genesis Producer Pubkey Mismatch

**What happened**: Genesis producers were registered with incorrect public keys that didn't match the wallet files. Nodes couldn't produce blocks because their actual pubkey wasn't in the registered producer list.

**Root cause**: The pubkeys in `genesis.rs` were NOT the same as the pubkeys in the wallet JSON files.

**How to avoid**:
1. Generate wallet files FIRST
2. Extract pubkeys from wallet files
3. Copy those EXACT pubkeys to genesis.rs
4. VERIFY they match before launch

```bash
# Extract pubkey from wallet file
cat producer_keys/producer_1.json | grep public_key
# Output: "public_key": "8f5b66af162a74d3d0992e73adbb3c6baf774ee3b75e01dd393eaba8907621a2"

# This EXACT string must appear in MAINNET_GENESIS_PRODUCERS in genesis.rs
```

### Bug #2: Epoch State Not Persisting

**What happened**: After node restart, epoch tracking reset to zero. Rewards were never distributed because epoch counters reset on every restart.

**Resolution**: Added persistent epoch state fields to ChainState:
- `epoch_producer_blocks`: HashMap<String, u64>
- `epoch_start_height`: u64
- `epoch_reward_pool`: u64
- `current_reward_epoch`: u64

**Status**: FIXED in storage/src/chain_state.rs with `#[serde(default)]` for backward compatibility.

### Bug #3: RPC Showed Wrong Network

**What happened**: RPC `getChainInfo` always returned "mainnet" even on testnet.

**Resolution**: Added `.with_network()` call in node.rs when creating RPC context.

**Status**: FIXED.

---

## 3. Maintainer Key Generation

### Overview

5 maintainers control protocol updates with a 3-of-5 signature threshold.

| Role | Responsibility |
|------|----------------|
| Maintainer 1-5 | Sign protocol updates, can veto malicious changes |
| 3-of-5 required | Any 3 maintainers must sign for update to be valid |

### Key Generation Process

Each maintainer must:

1. **Generate offline** on air-gapped machine:
```bash
# Using OpenSSL
openssl genpkey -algorithm ed25519 -out maintainer_private.pem
openssl pkey -in maintainer_private.pem -pubout -outform DER | tail -c 32 | xxd -p -c 32

# Or using doli-cli
./target/release/doli wallet new --output maintainer_wallet.json
cat maintainer_wallet.json | jq -r '.addresses[0].public_key'
```

2. **Store private key** in secure offline storage (never online!)

3. **Submit public key** (hex-encoded) for inclusion in code

### Current Maintainer Keys (Testnet)

Location: `crates/updater/src/lib.rs`

```rust
pub const MAINTAINER_KEYS: [&str; 5] = [
    "721d2bc74ced1842eb77754dac75dc78d8cf7a47e10c83a7dc588c82187b70b9", // Maintainer 1
    "d0c62cb4e143d548271eb97c4651e77b6cf52909a016bda6fb500c3bc022298d", // Maintainer 2
    "9fac605a1ebf2acfa54ef8406ab66d604df97d63da1f1ab6a45561c7e51be697", // Maintainer 3
    "97bdb0a9a52d4ed178c2307e3eb17e316b57d098af095b9cefc0c69d73e8817f", // Maintainer 4
    "82ed55afabfe38d826c1e2b870aefcc9ed0de45e5620adb4f858e6f47c8d4096", // Maintainer 5
];
```

**FOR MAINNET**: Replace with production keys generated offline!

---

## 4. Genesis Producer Setup

### Step 1: Generate Producer Wallets

On the mainnet seed server, create 5 producer wallet files:

```bash
mkdir -p ~/.doli/mainnet/producer_keys

for i in 1 2 3 4 5; do
    ./target/release/doli wallet new --output ~/.doli/mainnet/producer_keys/producer_$i.json
done
```

### Step 2: Extract Public Keys

```bash
for i in 1 2 3 4 5; do
    echo "Producer $i:"
    cat ~/.doli/mainnet/producer_keys/producer_$i.json | jq -r '.addresses[0].public_key'
done
```

Save these pubkeys - they will be needed for genesis.rs!

### Step 3: Update genesis.rs

**CRITICAL**: Use the EXACT pubkeys from Step 2!

```rust
// In crates/core/src/genesis.rs
pub const MAINNET_GENESIS_PRODUCERS: &[(&str, u32)] = &[
    ("<pubkey_from_producer_1.json>", 1),
    ("<pubkey_from_producer_2.json>", 1),
    ("<pubkey_from_producer_3.json>", 1),
    ("<pubkey_from_producer_4.json>", 1),
    ("<pubkey_from_producer_5.json>", 1),
];

pub fn mainnet_genesis_producers() -> Vec<(PublicKey, u32)> {
    MAINNET_GENESIS_PRODUCERS
        .iter()
        .filter_map(|(hex, bonds)| {
            let bytes = hex::decode(hex).ok()?;
            if bytes.len() != 32 {
                return None;
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Some((PublicKey::from_bytes(arr), *bonds))
        })
        .collect()
}
```

### Step 4: Update main.rs for Mainnet Genesis

Ensure `bins/node/src/main.rs` initializes genesis producers for mainnet:

```rust
// In producer_set initialization
if network == Network::Mainnet {
    use doli_core::genesis::mainnet_genesis_producers;
    let genesis_producers = mainnet_genesis_producers();
    if !genesis_producers.is_empty() {
        info!("Initializing mainnet with {} genesis producers", genesis_producers.len());
        ProducerSet::with_genesis_producers(genesis_producers)
    } else {
        ProducerSet::new()
    }
}
```

### Step 5: Verify Pubkey Match

**BEFORE BUILDING**, double-check:

```bash
# From genesis.rs
grep -A5 "MAINNET_GENESIS_PRODUCERS" crates/core/src/genesis.rs

# From wallet files
for i in 1 2 3 4 5; do
    cat ~/.doli/mainnet/producer_keys/producer_$i.json | jq -r '.addresses[0].public_key'
done

# THESE MUST MATCH EXACTLY!
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
| Bond Amount | 1000 DOLI | `network.rs:initial_bond()` |
| Total Supply | 100,000,000 DOLI | `consensus.rs:TOTAL_SUPPLY` |

### Genesis Configuration

| Parameter | Value |
|-----------|-------|
| Timestamp | 2026-02-01T00:00:00Z (Unix: 1769904000) |
| Message | "Time is the only fair currency. 01/Feb/2026" |
| Genesis Producers | 5 (registered at height 0) |

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
mainnet.doli.network    A       <seed_server_ip>
```

### Seed Server Requirements

- Static IP address
- Ports 30303 and 8545 open
- 99.9% uptime SLA recommended
- Located in geographically central location for low latency

### Bootstrap Configuration

Other nodes worldwide connect using:

```bash
./doli-node --network mainnet run --bootstrap /dns4/mainnet.doli.network/tcp/30303/p2p/<PEER_ID>
```

The `<PEER_ID>` is printed in logs when seed node starts:
```
Local peer id: 12D3KooW...
```

---

## 7. Launch Procedure

### Phase 1: Final Preparation (T-24h)

1. Finalize all code changes
2. Run full test suite: `cargo test`
3. Build release binary: `cargo build --release`
4. Deploy binary to all 5 servers
5. Copy producer key files to each server

### Phase 2: Pre-Genesis (T-1h)

1. Clear any existing data:
```bash
rm -rf ~/.doli/mainnet/node*/data/
```

2. Verify DNS resolves correctly:
```bash
dig mainnet.doli.network
```

3. Verify firewall rules:
```bash
sudo ufw status
# Should show 30303/tcp ALLOW
```

### Phase 3: Genesis (T-0)

1. Start seed node (Node 1) FIRST:
```bash
./doli-node --network mainnet -d node1/data run \
    --producer \
    --producer-key producer_keys/producer_1.json \
    --p2p-port 30303 \
    --rpc-port 8545 \
    > node1/node.log 2>&1 &
```

2. Get seed node peer ID:
```bash
grep "Local peer id" node1/node.log
# Example: 12D3KooWEVkrAJ8Xikd3xVSeMzX9LL5ZQidBG9x8qA2HvPb7hYp4
```

3. Start remaining nodes with bootstrap:
```bash
BOOTSTRAP="/dns4/mainnet.doli.network/tcp/30303/p2p/<PEER_ID>"

for i in 2 3 4 5; do
    ./doli-node --network mainnet -d node$i/data run \
        --producer \
        --producer-key producer_keys/producer_$i.json \
        --p2p-port $((30303 + i - 1)) \
        --rpc-port $((8545 + i - 1)) \
        --bootstrap "$BOOTSTRAP" \
        > node$i/node.log 2>&1 &
done
```

---

## 8. Verification Steps

### Immediate Checks (Within 1 minute)

```bash
# 1. Check chain is producing blocks
curl -s -H "Content-Type: application/json" http://127.0.0.1:8545 \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}'
# Expected: height > 0, network = "mainnet"

# 2. Verify 5 producers registered
curl -s -H "Content-Type: application/json" http://127.0.0.1:8545 \
    -d '{"jsonrpc":"2.0","method":"getProducers","params":{"active_only":true},"id":1}'
# Expected: 5 producers with registrationHeight = 0

# 3. Check all nodes producing
for i in 1 2 3 4 5; do
    echo "=== Node $i ==="
    grep "produced at height" node$i/node.log | tail -2
done
```

### 1-Hour Check (After First Epoch)

```bash
# Verify epoch reward distribution
curl -s -H "Content-Type: application/json" http://127.0.0.1:8545 \
    -d '{"jsonrpc":"2.0","method":"getProducers","params":{"active_only":true},"id":1}' \
    | jq '.result[] | {publicKey: .publicKey[0:16], blocksProduced}'
# Expected: blocksProduced > 0 for all producers

# Check height (should be ~360 after 1 hour)
curl -s -H "Content-Type: application/json" http://127.0.0.1:8545 \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' \
    | jq '.result.bestHeight'
```

### Node Restart Test (Critical!)

```bash
# Stop a node
pkill -f "doli-node.*node3"

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

# Verify epoch state persisted (check logs for "Loaded epoch state")
grep -i "epoch" node3/node.log | head -5
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

**Check**: Verify epoch state is being saved on shutdown and loaded on startup.
Look for log messages about epoch tracking.

---

## Appendix A: File Locations

| File | Purpose |
|------|---------|
| `crates/core/src/genesis.rs` | Genesis configuration, producer pubkeys |
| `crates/core/src/network.rs` | Network parameters (rewards, timing) |
| `crates/updater/src/lib.rs` | Maintainer keys for auto-update |
| `crates/storage/src/chain_state.rs` | Epoch state persistence |
| `bins/node/src/main.rs` | Genesis producer initialization |
| `bins/node/src/node.rs` | Production logic, RPC context |

## Appendix B: Key Constants

```rust
// Block timing
pub const SLOT_DURATION: u64 = 10;          // 10 seconds per block
pub const SLOTS_PER_EPOCH: u32 = 360;       // 1 hour

// Economics
pub const INITIAL_REWARD: u64 = 100_000_000;  // 1 DOLI per block
pub const BOND_UNIT: u64 = 100_000_000_000;   // 1000 DOLI per bond
pub const TOTAL_SUPPLY: u64 = 100_000_000 * DOLI; // 100M DOLI

// Governance
pub const VETO_PERIOD: Duration = Duration::from_secs(7 * 24 * 3600);  // 7 days
pub const GRACE_PERIOD: Duration = Duration::from_secs(48 * 3600);     // 48 hours
pub const VETO_THRESHOLD_PERCENT: u8 = 40;
pub const REQUIRED_SIGNATURES: usize = 3;  // 3-of-5 maintainers
```

---

**Document Version**: 1.0
**Last Updated**: 2026-01-30
**Author**: DOLI Core Team

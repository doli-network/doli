# DOLI Scaling Roadmap

> From 34 producers to 100,000 participants — without committees, without sharding, without complexity.

## Current State (2026-04)

- 34 active producers across 10 servers
- All 34 verify every block, attest every minute, receive epoch rewards
- Round-robin production: each producer gets 1 block every ~340 seconds
- Verification cost: negligible (no VM, fixed-cost per output)
- Network: stable at v5.5.0, 10s blocks, 100% attestation rates

## Already Built for Scale

The infrastructure required to scale to 100,000 participants is **already implemented**. These are not future work — they are live in production:

### BLS Attestation (aggregate-first)

Every attestation is a BLS12-381 signature. The block producer aggregates all signatures into ONE aggregate signature (48 bytes) regardless of signer count. Validators verify with a single pairing check (~2ms).

```
100,000 attestors → bls_aggregate() → 1 signature (48 bytes) → 1 verification (~2ms)
```

This is the same math Ethereum uses for committees of 128. DOLI uses it for ALL participants because the architecture has no bottlenecks that force committee selection. The aggregation is already implemented in `production/assembly.rs:485` using the `blst` library.

### Compact Bitfield

Each block header carries a `presence_root` — one bit per producer indicating who attested. At 100,000 participants: 12.5 KB. Trivial.

Already implemented in `production/assembly.rs:284` with `encode_attestation_bitfield()`.

### Dynamic Gossip Mesh

Gossipsub mesh parameters scale automatically with network size:

```rust
// gossip/config.rs:31
mesh_n = sqrt(total_peers) × 1.5, capped at 50
```

At 100 peers: mesh_n = 15. At 10,000 peers: mesh_n = 50 (cap). Each node maintains connections to ~50 peers regardless of network size — O(sqrt(N)) scaling, not O(N).

### Peer Churn Protection

Production-hardened from real incidents (INC-I-011, INC-I-014):

| Protection | What it does |
|-----------|-------------|
| Eviction cooldown (120s) | Evicted peers can't reconnect immediately — breaks evict→reconnect→evict loop |
| Eviction rate limit (5/60s) | Max 5 evictions per minute — prevents RAM explosion from churn |
| DHT bootstrap skip | No DHT discovery when peer table full + churn active |
| Dial backoff (exponential) | Failed peers get increasing backoff, removed from Kademlia after 5 failures |
| Genesis mismatch cooldown (24h) | Peers on different chains rejected for 24 hours |
| Stale peer blocklist (24h) | Peers with changed identity blocked to prevent DHT pollution |

### Gossipsub Peer Scoring

Producers are prioritized in the gossip mesh by `first_message_deliveries` scoring — they naturally score higher because they create blocks (first-seen). Non-producers relay only and score lower. The mesh self-organizes to keep producers connected.

```
Blocks topic:       first_message_weight = 10.0
Transactions topic: invalid_message_weight = -10.0
Producers topic:    invalid_message_weight = -20.0
IP colocation:      configurable threshold (default 500 dev, 5 production)
```

### Per-Peer Rate Limiting

Token bucket rate limiter per peer. Prevents any single peer from flooding the node with requests. Already live in `rate_limit.rs`.

### Stale Tip Recovery (INC-I-020)

The sync engine ignores 1-2 block gaps (assumes gossip delivers). When gossip misses, the health check (every 30s) detects `best_peer_h > local_h` and requests the missing block directly. Prevents permanent 1-block desync after restarts.

## The Non-Problem

DOLI does not have Ethereum's scaling problem. Ethereum needs committees because smart contract execution has unbounded cost — you can't ask 900,000 validators to execute arbitrary Solidity. DOLI has no VM. Verification is O(n) over outputs with fixed cost per output. No loops, no recursion, no gas metering. A 2MB block verifies in milliseconds on any CPU.

**Total verification by all participants scales to thousands of nodes without committees.** The bottleneck is not verification — it's production scheduling.

## The Real Bottleneck

Round-robin assigns slots sequentially. More producers in the rotation = longer wait between turns:

| Producers in rotation | Block interval per producer |
|-----------------------|---------------------------|
| 34 (current)          | ~5.7 min                  |
| 100                   | ~17 min                   |
| 500                   | ~83 min                   |
| 15,000                | ~42 hours (unacceptable)  |

The solution: **fixed production list + unlimited attestation participation.**

## Architecture: Active List + Hot Standby

### The Active List (100 producers)

A fixed list of 100 producers handles all block production via round-robin. With 10s slots, each producer gets 1 block every ~17 minutes. This interval is optimal for global propagation and stays constant regardless of total network size.

### Attestors (unlimited)

Every participant with bonds who attests 90%+ receives epoch rewards — whether they are in the active list or not. There is no economic penalty for being outside the list. Rewards are bond-weighted across ALL qualified attestors:

```
reward(i) = epoch_pool × bonds(i) / total_qualifying_bonds
```

A participant with 10 bonds outside the list earns the same percentage as a participant with 10 bonds inside the list. Production is a duty, not an economic privilege.

### Replacement: Random Promotion at Epoch Boundary

When a producer in the active list fails (drops below attestation threshold, deregisters, gets slashed), the slot is filled at the next epoch boundary:

1. Identify vacant slots in the active list
2. Collect all attestors with 90%+ qualification in the completed epoch
3. Select replacement randomly: `BLAKE3(epoch_number || state_root)` as deterministic seed
4. Promote selected attestor to the active list

Properties:
- **Deterministic** — every node computes the same replacement from chain state
- **Non-manipulable** — the state_root depends on all transactions in the epoch, not controllable by any single producer
- **Fair** — every qualified attestor has equal probability regardless of bond size
- **Self-healing** — the active list repairs itself every epoch without human intervention

### If a Producer Returns

A producer removed from the active list for inactivity continues as an attestor. They keep receiving rewards (bond-weighted, same as everyone). They can be randomly selected back into the active list in a future epoch. There is no fast-track re-entry.

No deregistration. No bond loss. No reward loss. Only the production responsibility changes.

## Scaling Properties

| Participants | Active List | Attestors | Block interval | Throughput |
|-------------|-------------|-----------|---------------|------------|
| 100         | 100         | 100       | ~17 min       | 6 blk/min  |
| 1,000       | 100         | 1,000     | ~17 min       | 6 blk/min  |
| 10,000      | 100         | 10,000    | ~17 min       | 6 blk/min  |
| 100,000     | 100         | 100,000   | ~17 min       | 6 blk/min  |

**Production is constant.** The network produces 6 blocks per minute regardless of whether there are 100 or 100,000 participants. What scales is economic participation — more attestors, more bonds staked, more security, more decentralization of rewards.

### Network Overhead at Scale

| Component | 100 participants | 100,000 participants | Scaling |
|-----------|-----------------|---------------------|---------|
| Block propagation | Same | Same | O(1) — only 100 producers |
| Attestation signatures | 100 BLS sigs/min | 100,000 BLS sigs/min | Aggregated into 1 sig |
| Attestation bandwidth | ~10 KB/min | ~10 MB/min | Linear, manageable |
| Bitfield in header | 13 bytes | 12.5 KB | Linear, trivial |
| Aggregate verification | ~2ms (1 pairing) | ~2ms (1 pairing) | O(1) — single check |
| Aggregation by producer | <1ms | ~10ms | Linear, fast (blst) |
| Gossip mesh per node | ~15 peers | ~50 peers (cap) | O(sqrt(N)) |
| Peer table per node | 100 entries | max_peers (configurable) | Bounded |

### Gossip Noise Reduction

With the active list model, attestors don't need full gossip mesh participation. They need:
1. Receive blocks (subscribe to topic)
2. Send 1 attestation per minute (100 bytes)

Attestors can operate as **light participants** — 2-3 connections to seeds, minimal peer table. Only the 100 active producers need dense mesh for fast block propagation.

Network gossip drops from O(N²) peer connections to O(100² + N×3) — dense mesh among 100 producers, sparse connections for N attestors.

### Comparison with Ethereum

| Property | Ethereum | DOLI |
|----------|----------|------|
| Who verifies blocks | Committee of ~128 (random subset) | ALL participants |
| Who attests | Committee of ~128 | ALL participants |
| Attestation aggregation | BLS aggregate (128 sigs) | BLS aggregate (100,000 sigs) |
| Aggregate verification | 1 pairing check | 1 pairing check |
| Trust model | Trust the random committee | Trust no one — all verified |
| Why committees exist | VM execution is unbounded | Not needed — no VM |

Ethereum has the same BLS math but uses it for 128 validators because its architecture forces committees. DOLI uses the same math for ALL participants because it has no bottleneck that forces subsampling.

## Implementation

### What already exists (95% of the work):
- BLS attestation with aggregate-first verification
- Compact bitfield in block headers
- Bond-weighted epoch reward distribution
- Dynamic gossip mesh (sqrt scaling)
- Peer churn protection (eviction cooldown, rate limiting)
- Gossipsub peer scoring (producers prioritized)
- Liveness filter (exclusion/re-inclusion)
- Round-robin scheduler with seniority weighting
- DelegateBond / RevokeDelegation transaction types
- Delegation reward split (10% operator / 90% staker)
- Stale tip recovery (INC-I-020)

### What needs to be built (~65 lines):
- `is_active_producer` flag in producer state (~10 lines)
- Scheduler filter: only active list enters round-robin (~5 lines)
- Random promotion at epoch boundary when slots are vacant (~50 lines)

### What does NOT change:
- Block format
- Transaction format
- Attestation protocol
- Reward calculation
- Gossip protocol
- Sync engine
- UTXO model
- BLS aggregation

## Phases

### Phase 1: Current (34 producers) — Live

All producers in active list. No distinction. Network grows organically.

### Phase 2: Active List (at ~100 producers) — Future

Activate the 100-producer active list when the network approaches that size. Initial list: top 100 by `bonds × seniority_weight`. Random replacement at epoch boundary for dropouts.

### Phase 3: Delegation (at ~500 participants) — Future

Enable Tier 3 delegation for participants who don't want to run infrastructure. 90% rewards to delegator, 10% to operator. Operators compete for delegation by offering reliability and reputation.

### Phase 4: Dynamic List Size (at ~1,000+ producers) — Future

The active list size could increase with eras:

| Era | Active List | Slot Duration | Block interval |
|-----|-------------|---------------|---------------|
| 1   | 100         | 10s           | ~17 min       |
| 2   | 150         | 10s           | ~25 min       |
| 3   | 200         | 8s            | ~27 min       |
| 4   | 250         | 6s            | ~25 min       |

Slot duration decreases as network infrastructure matures, keeping block intervals manageable even with a larger active list.

## Why This Works

The insight: **production and participation are independent concerns.**

Every prior blockchain conflates them — if you validate, you produce. If you produce, you earn more. This creates competition for production slots that drives centralization (mining pools, staking pools, MEV extraction).

DOLI separates them:
- **Production** = civic duty, assigned randomly, rotated at epoch boundaries
- **Participation** = attestation + bonds, open to everyone, rewarded equally
- **Rewards** = bond-weighted across all qualified attestors, not production-weighted

A participant who never produces a single block earns the same ROI as one who produces 100 blocks per day. The economic incentive is to attest and hold bonds, not to compete for production slots.

The architecture was designed in 2026 with 17 years of blockchain hindsight. Every decision — no VM, UTXO model, BLS attestation, dynamic mesh, epoch rewards — was made knowing how it would scale. The infrastructure is already built. The only missing piece is a 65-line scheduler filter that activates when the network grows large enough to need it.

---

*"The best scaling solution is the one you don't need to build because the architecture doesn't create the problem."*

I. Lozada · ivan@doli.network

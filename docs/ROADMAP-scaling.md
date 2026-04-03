# DOLI Scaling Roadmap

> From 34 producers to 100,000 participants — without committees, without sharding, without complexity.

## Current State (2026-04)

- 34 active producers across 10 servers
- All 34 verify every block, attest every minute, receive epoch rewards
- Round-robin production: each producer gets 1 block every ~340 seconds
- Verification cost: negligible (no VM, fixed-cost per output)
- Network: stable at v5.5.0, 10s blocks, 100% attestation rates

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

## Architecture: Active Set + Hot Standby

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
2. Collect all Tier 2 attestors with 90%+ qualification in the completed epoch
3. Select replacement randomly: `BLAKE3(epoch_number || state_root)` as deterministic seed
4. Promote selected attestor to the active list

Properties:
- **Deterministic** — every node computes the same replacement from chain state
- **Non-manipulable** — the state_root depends on all transactions in the epoch, not controllable by any single producer
- **Fair** — every qualified attestor has equal probability regardless of bond size
- **Self-healing** — the active list repairs itself every epoch without human intervention

### If a Producer Returns

A producer removed from the active list for inactivity continues as an attestor. They keep receiving rewards (bond-weighted, same as everyone). They can be randomly selected back into the active list in a future epoch. There is no fast-track re-entry — losing your slot means waiting for the random draw like everyone else.

No deregistration. No bond loss. No reward loss. Only the production responsibility changes.

## Scaling Properties

| Participants | Active List | Attestors | Block interval | Throughput |
|-------------|-------------|-----------|---------------|------------|
| 100         | 100         | 100       | ~17 min       | 6 blk/min  |
| 1,000       | 100         | 1,000     | ~17 min       | 6 blk/min  |
| 10,000      | 100         | 10,000    | ~17 min       | 6 blk/min  |
| 100,000     | 100         | 100,000   | ~17 min       | 6 blk/min  |

**Production is constant.** The network produces 6 blocks per minute regardless of whether there are 100 or 100,000 participants. What scales is economic participation — more attestors, more bonds staked, more security, more decentralization of rewards.

### BLS Aggregation at Scale

The attestation protocol uses BLS12-381 signature aggregation. At 100,000 participants:
- Bitfield size: 100,000 bits = 12.5 KB per block header (trivial)
- Aggregate signature: 1 BLS signature regardless of participant count (48 bytes)
- Verification: 1 pairing check verifies all 100,000 attestations

BLS was designed for exactly this — the protocol already uses it. No changes needed.

### Comparison

| System | Scaling approach | Adds complexity | Production set |
|--------|-----------------|----------------|----------------|
| Ethereum | Random committees (128 of 900K) | Significant | Stake-weighted lottery |
| Solana | Stake-weighted leader schedule | Moderate | All validators |
| Cosmos | Separate chains (IBC) | Significant | Per-chain set |
| Polkadot | Parachains + relay chain | Significant | Per-parachain |
| **DOLI** | **Fixed list + unlimited attestors** | **Minimal** | **100 fixed, rotated randomly** |

## Implementation

### What already exists:
- Attestation system with bitfield + BLS aggregation
- Bond-weighted epoch reward distribution
- Liveness filter (exclusion/re-inclusion)
- Round-robin scheduler with seniority weighting
- DelegateBond / RevokeDelegation transaction types
- Delegation reward split (10% operator / 90% staker)

### What needs to be built (~200 lines):
- `is_active_producer` flag in producer state
- Active list initialization: top 100 by `bonds × seniority_weight`
- Scheduler filter: only active list enters round-robin
- Random promotion at epoch boundary when slots are vacant
- CLI: `doli producer status` shows active list membership
- RPC: `isActiveProducer` field in getProducer response

### What does NOT change:
- Block format
- Transaction format
- Attestation protocol
- Reward calculation
- Gossip protocol
- Sync engine
- UTXO model

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

Every prior blockchain conflates them — if you validate, you produce. If you produce, you earn more. This creates a competition for production slots that drives centralization (mining pools, staking pools, MEV extraction).

DOLI separates them:
- **Production** = civic duty, assigned randomly, rotated at epoch boundaries
- **Participation** = attestation + bonds, open to everyone, rewarded equally
- **Rewards** = bond-weighted across all qualified attestors, not production-weighted

A participant who never produces a single block earns the same ROI as one who produces 100 blocks per day. The economic incentive is to attest and hold bonds, not to compete for production slots.

The result: scaling the network adds participants (security) without adding production overhead (complexity). The simplest possible separation of concerns.

---

*"The best scaling solution is the one you don't need to build because the architecture doesn't create the problem."*

I. Lozada · ivan@doli.network

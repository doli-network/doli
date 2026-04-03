# DOLI Scaling Roadmap

> From 34 producers to 15,500 participants — without committees, without sharding, without complexity.

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

With 500 producers in round-robin at 10s slots: 1 block per producer every ~83 minutes. Manageable.

With 15,000 producers in round-robin at 10s slots: 1 block per producer every ~42 hours. Unacceptable — one missed slot costs two days of production opportunity.

The solution: **decouple production from participation.**

## Phase 1: Current (34 producers)

```
All producers:
  ✓ Verify every block
  ✓ Attest every minute
  ✓ Produce blocks (round-robin)
  ✓ Receive epoch rewards (bond-weighted)
```

No changes needed. 34 producers with total verification is trivial.

## Phase 2: Tier System (500 + 15,000)

### Tier 1 — Block Producers (up to 500)

- Verify every block
- Attest every minute
- Produce blocks (round-robin among Tier 1 only)
- Receive epoch rewards (bond-weighted, same as all tiers)
- Can accept Tier 3 delegation (earn 10% operator fee)
- Subject to double-production slashing (100% bond)

### Tier 2 — Attestors (up to 15,000)

- Verify every block (same as Tier 1)
- Attest every minute (same as Tier 1)
- Do NOT produce blocks
- Receive epoch rewards (bond-weighted, identical ROI % as Tier 1)
- Can accept Tier 3 delegation (earn 10% operator fee)
- No slashing risk (can't double-produce what they don't produce)

### Tier 3 — Delegators (unlimited)

- Do NOT run a node
- Delegate stake to a Tier 1 or Tier 2 operator
- Receive 90% of rewards proportional to delegated bonds
- Operator receives 10% for running infrastructure
- Can redelegate after unbonding period

### Key Properties

**Equal ROI across tiers.** A bond in Tier 1 earns the same percentage as a bond in Tier 2 or a delegated bond in Tier 3. The difference is operational: Tier 1 has production responsibility + slashing risk, Tier 2 has attestation responsibility only, Tier 3 has zero operational requirement.

**No committees.** All 15,500 participants (Tier 1 + Tier 2) verify every block. There is no random subset, no rotation, no trust delegation. The network is as secure as its total participant count, not as secure as a randomly sampled committee.

**Production stays bounded.** Round-robin over 500 Tier 1 producers: 1 block per producer every ~83 minutes. Network throughput is independent of total participant count.

### Tier Selection

How a participant becomes Tier 1 vs Tier 2:

**Option A — Bond threshold:** Tier 1 requires minimum N bonds. Simple, but favors wealthy participants.

**Option B — Self-selection:** Participants choose their tier at registration. Tier 1 has higher slashing risk. Natural market: those with reliable infrastructure choose Tier 1 for delegation fees.

**Option C — Seniority + bonds:** Top 500 by `bonds × seniority_weight` are Tier 1. Rewards operational commitment over time. Newcomers start as Tier 2 and graduate naturally.

**Recommended: Option C.** It aligns with DOLI's core thesis — time as the scarce resource. A new participant with $1M can't buy Tier 1 status immediately. They start as Tier 2, build seniority, and earn their way into the production set over years. Same principle as the 4-year seniority weight, extended to tier selection.

## Phase 3: Dynamic Scaling

As the network grows beyond 500 Tier 1 producers:

**Era-based Tier 1 cap increase:**
| Era | Tier 1 Cap | Tier 2 Cap | Block interval per producer |
|-----|-----------|-----------|---------------------------|
| 1   | 500       | 15,000    | ~83 min                   |
| 2   | 750       | 22,500    | ~125 min                  |
| 3   | 1,000     | 30,000    | ~167 min                  |

The Tier 1 cap grows with the era, allowing more producers into the rotation as the network matures and slot duration can potentially decrease.

**Slot duration reduction:** With more producers and better global network infrastructure, slot duration could decrease from 10s to 5s, doubling throughput while keeping production intervals reasonable.

## Implementation Estimate

### What already exists (90% of the work):
- Attestation system with bitfield + BLS aggregation
- Bond-weighted epoch reward distribution
- Liveness filter (exclusion/re-inclusion)
- Round-robin scheduler with seniority weighting
- DelegateBond / RevokeDelegation transaction types
- Delegation reward split (10% operator / 90% staker) in protocol

### What needs to be built (~200 lines):
- `ProducerTier` enum (Tier1, Tier2) in producer registration
- Scheduler filter: `if tier == Tier1` for round-robin assignment
- Tier promotion/demotion logic at epoch boundaries
- CLI: `doli producer register --tier 2`
- RPC: tier field in getProducer / getProducers responses

### What does NOT need to change:
- Block format
- Transaction format
- Attestation protocol
- Reward calculation (already bond-weighted across all qualified producers)
- Gossip protocol
- Sync engine
- UTXO model

## Why This Works

Every blockchain scaling solution adds complexity:
- Ethereum: committees, random selection, slashing for attestation failures
- Solana: leader schedule based on stake weighting, vote transactions consuming bandwidth
- Cosmos: separate chains with IBC bridges
- Polkadot: parachains with relay chain coordination

DOLI's approach adds almost nothing:
- One boolean per producer (Tier 1 or Tier 2)
- One filter in the scheduler
- Everything else stays the same

The reason it's this simple: DOLI never had the problems that make other chains complex. No VM means no execution bottleneck. No gas means no fee market. No shared mutable state means no MEV. No committees means no committee selection. The architecture was designed in 2026 with 17 years of blockchain hindsight — the complexity was avoided at the design level, not solved at the engineering level.

---

*"The best scaling solution is the one you don't need to build because the architecture doesn't create the problem."*

I. Lozada · ivan@doli.network

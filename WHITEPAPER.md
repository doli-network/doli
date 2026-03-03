# DOLI<sub>τ</sub>

## Deterministic Ordering through Linear Iterations

### Only Time Matters...
### A Peer-to-Peer Electronic Cash System Based on Verifiable Time

**E. Weil** · weil@doli.network

---

## Abstract

Bitcoin consumes the energy of a mid-sized country. Proof of Stake requires significant capital to participate in consensus. Both use probabilistic reward systems that force small participants into pools, reintroducing centralization.

We propose a system where the only resource required is time—the one resource distributed equally to all participants. Block production follows deterministic rotation, not lottery. A participant with one bond knows exactly when their next block will be produced. Pools become unnecessary.

Transactions are ordered through verifiable time. Blocks are produced at fixed intervals using a sequential delay function that cannot be parallelized. Participation in block production requires sustained time commitment, limiting the ability to create influence instantly.

The result is a system where consensus weight emerges from time rather than trust, capital, or scale.

---

## 1. Introduction

Every consensus mechanism ever designed shares one assumption: security requires a scarce resource that can be accumulated. Bitcoin chose energy. Ethereum chose capital. Both created systems where the largest participant has a structural advantage over the smallest.

This assumption is wrong. There exists a resource that cannot be accumulated, cannot be parallelized, and is distributed equally to every participant on Earth: **time**.

One second passes at the same rate for an individual running a single node as it does for a nation-state with unlimited budget. No amount of money can buy more time. No amount of hardware can make time pass faster.

We propose an electronic cash system where consensus derives from verifiable sequential computation — proof that real time has elapsed. Double-spending is prevented not by energy expenditure or capital lockup, but by the irreversible passage of time. The system is secure as long as honest participants collectively maintain more sequential computation presence than any cooperating group of attackers.

The result is a network where a participant with a single bond (10 DOLI) earns the same percentage return as the largest producer. Pools are unnecessary. Rewards are deterministic. The smallest participant knows exactly when their next block will be produced.

### 1.1. Why Now

The blockchain trilemma assumes three competing properties: decentralization, security, and scalability. Proposed solutions trade one for another. Proof of Work sacrifices energy efficiency for security. Proof of Stake sacrifices decentralization for scalability. Sharding sacrifices simplicity for throughput.

These tradeoffs arise from a common assumption: consensus requires a scarce resource that can be accumulated. Hashpower can be bought. Stake can be concentrated. Storage can be hoarded.

**Time cannot be accumulated.** One entity cannot acquire "more time" than another. A nation-state with unlimited budget still experiences one second per second.

Verifiable Delay Functions, first formalized by Boneh, Bonneau, Bünz, and Fisch in 2018, made it possible to prove that sequential time has elapsed without trusting the prover. Before this cryptographic primitive existed, Proof of Time was not implementable.

The trilemma persists because it never considered time as the scarce resource. By anchoring consensus to sequential computation that cannot be parallelized, we achieve:

- **Decentralization:** No special hardware required. Any CPU can participate.
- **Security:** Attacking requires real time, not purchasable resources.
- **Scalability:** Block production is bounded by time, not by resource competition.

The tradeoff disappears when the resource cannot be monopolized.

---

## 2. Transactions

A coin is a chain of digital signatures. To transfer ownership, the current holder signs the transaction hash together with the recipient's public key. The recipient verifies the signature chain to confirm provenance.

```
┌─────────────────────────────────┐
│         Transaction             │
├─────────────────────────────────┤
│  Hash of previous TX            │
│  Recipient public key           │
│  Owner signature                │
└─────────────────────────────────┘
                │
                ▼
┌─────────────────────────────────┐
│         Transaction             │
├─────────────────────────────────┤
│  Hash of previous TX            │
│  Recipient public key           │
│  Owner signature                │
└─────────────────────────────────┘
```

The fundamental challenge is double-spending: without a central authority, how does the recipient know the sender hasn't already spent the same coin elsewhere? Centralized solutions work but create single points of failure and require universal trust.

DOLI solves this through public announcement of all transactions and deterministic time-ordering. Every transaction is broadcast to the network and included in a block at a specific slot. The sequential nature of slots — each anchored by a VDF proof — establishes an unambiguous ordering. The earliest transaction in the time sequence is the valid one. Later attempts to spend the same output are rejected by every honest node.

### 2.1. Transaction Validity

A transaction is valid if:

1. Each input references an existing, unspent output.
2. The signature corresponds to the public key of the referenced output.
3. The sum of inputs is greater than or equal to the sum of outputs.
4. All amounts are positive.

The difference between inputs and outputs constitutes the fee for the block producer.

---

## 3. Timestamp Server

The solution begins with a distributed timestamp server. The network acts as a timestamp server by taking a hash of a block of items to be timestamped and widely publishing the hash. The timestamp proves that the data must have existed at the time in order to enter the hash.

```
                     ┌──────────────────┐
                     │      Block       │
                     ├──────────────────┤
                     │  Previous hash   │
                     │  Timestamp       │
 Transactions ───▶   │  Transactions    │
                     │  VDF Proof       │
                     └──────────────────┘
                              │
                              ▼
                     ┌──────────────────┐
                     │      Block       │
                     ├──────────────────┤
                     │  Previous hash   │
                     │  Timestamp       │
                     │  Transactions    │
                     │  VDF Proof       │
                     └──────────────────┘
```

Each timestamp includes the previous timestamp in its hash, forming a chain. Each additional timestamp reinforces the ones before it.

---

## 4. Proof of Time

To implement a distributed timestamp server on a peer-to-peer basis, we need a mechanism that makes producing blocks costly and prevents that cost from being evaded through parallelization or resource accumulation.

The solution is to use **Verifiable Delay Functions**. A VDF is a function that:

1. Requires a fixed number of sequential operations to compute.
2. Produces a proof that can be verified quickly.
3. Cannot be significantly accelerated through parallelization.

> **Technical note:** A VDF does not directly prove that "time has passed," but rather that *N* sequential operations were executed. However, since there is no known way to accelerate this computation through parallelization, time is the effective lower bound. The term "Proof of Time" reflects this property.

> **Scope clarification:** The VDF is used to prove sequential work per slot and to anchor time ordering. It is NOT used as a randomness source and does not affect leader selection. Producer selection is purely deterministic based on slot number and the epoch-frozen active set.

> **Hardware acceleration:** Faster VDF hardware provides no advantage. The VDF is a "heartbeat"—proof of presence, not proof of work. Producer selection is determined by Epoch Lookahead: the schedule for slot *s* is fixed at epoch start, independent of VDF computation speed.

For each block, the producer must calculate:

```
input  = HASH(prefix || previous_hash || tx_root || slot || producer_key)
output = HASH^n(input)
proof  = π
```

Where *n* is the difficulty parameter that determines how long the computation takes.

### 4.1. VDF Construction

```
Input: prev_hash || slot || producer_key
         │
         ▼
    ┌─────────┐
    │ BLAKE3  │ ◄──┐
    └────┬────┘    │
         │         │
         └─────────┘  × 800,000 iterations
         │
         ▼
      Output (hash chain result)
```

**Verification:** Nodes verify by recomputing the hash chain. Since the computation is inherently sequential, verification requires the same work as computation. This is acceptable because the block VDF (~55ms) is short.

The block VDF uses iterated BLAKE3 hashing (hash chain). This construction is simple, well-studied, and sufficient for the "heartbeat" proof of presence required per block.

### 4.2. Time Structure

The network defines time as follows:

```
GENESIS_TIME = 2026-02-01T00:00:00Z (UTC)
```

A slot is 10 seconds. A slot number derives deterministically from the timestamp:

```
slot = floor((timestamp - GENESIS_TIME) / 10)
```

An epoch is 360 slots (1 hour). At epoch boundaries, the active producer set updates.

| Unit  | Slots      | Duration  |
|-------|------------|-----------|
| Slot  | 1          | 10 sec    |
| Epoch | 360        | 1 hour    |
| Day   | 8,640      | 24 hours  |
| Era   | 12,614,400 | ~4 years  |

### 4.3. Dynamic Calibration

To maintain consistent timing (~55ms) regardless of hardware, iterations adjust dynamically:

```
TARGET_TIME    = 55ms
TOLERANCE      = 10%
MAX_ADJUSTMENT = 20% per cycle
```

If measured VDF time deviates more than 10%, iterations adjust by up to 20% per cycle. This allows heterogeneous hardware to participate while maintaining consistent block times.

With 10-second slots, the VDF takes ~55ms, leaving the remainder for block construction and propagation.

Every consensus system enforces a scarce resource. In DOLI, this resource is sequential time—analogous to energy in Proof of Work or capital in Proof of Stake. Nodes that cannot reliably supply sequential time and availability cannot earn rewards, just as offline miners or inactive validators cannot in existing systems.

---

## 5. Network

The steps to run the network are as follows:

1. New transactions are broadcast to all nodes.
2. Each eligible producer collects new transactions into a block.
3. The producer assigned to the slot computes the VDF proof.
4. The producer broadcasts the block to the network.
5. Nodes accept the block only if all transactions in it are valid and the VDF proof is correct.
6. Nodes express their acceptance of the block by working on creating the next block, using the hash of the accepted block as the previous hash.

Nodes always consider the chain covering the most time to be the correct one and will keep working on extending it. If two nodes broadcast different versions of the next block simultaneously, some nodes may receive one or the other first. In that case, they work on the first one they received but save the other branch in case it becomes longer. The tie will be broken when the next block is produced and one branch covers more slots; the nodes that were working on the other branch will then switch to the longer one.

### 5.1. Block Validity

A block *B* is valid if:

1. `B.timestamp > prev_block.timestamp`
2. `B.timestamp <= network_time + DRIFT`
3. `B.slot` derives correctly from `B.timestamp`
4. `B.slot > prev_block.slot`
5. `B.producer` has valid rank for `B.slot` (after bootstrap period)
6. `VDF_verify(preimage, B.vdf_output, B.vdf_proof, T) == true`
7. All transactions in the block are valid

### 5.2. Clock Synchronization

Consensus depends on nodes having reasonably synchronized clocks. Nodes synchronize through:

- NTP servers
- Median offset from connected peers

```
network_time = local_clock + median(peer_offsets)
```

Blocks with timestamps outside the acceptable window are rejected.

---

## 6. Producer Registration

In an open network, anyone can create identities at no cost. Allowing unlimited and free identity creation would expose the network to Sybil attacks where an attacker floods the system with fake nodes.

To prevent this, registration requires completing a VDF whose difficulty adjusts automatically with network congestion.

```
input  = HASH(prefix || public_key || epoch)
output = VDF(input, T_registration)
```

A registration is valid if:

1. The VDF proof verifies correctly with `T_registration`.
2. The epoch is current or previous.
3. The public key is not already registered.
4. The activation bond is included.

### 6.1. Dynamic Registration Difficulty

The time required to register adjusts per epoch:

```
T_registration(E+1) = T_base × max(1, D_E / R_target)
```

Where `D_E` is the smoothed demand based on recent registrations. When demand is high, registration takes longer. When demand is low, registration is faster.

### 6.2. Activation Bond

Every producer registration must lock an activation bond of 10 DOLI (1 bond unit). The bond unit is fixed and does not change across eras.

```
BOND_UNIT = 10 DOLI (fixed across all eras)
```

Block rewards halve each era (~4 years), making early participation more rewarding:

| Era | Years | Bond    | Reward  | Blocks to recover bond |
|-----|-------|---------|---------|------------------------|
| 1   | 0-4   | 10 DOLI | 1.0     | 10                     |
| 2   | 4-8   | 10 DOLI | 0.5     | 20                     |
| 3   | 8-12  | 10 DOLI | 0.25    | 40                     |
| 4   | 12-16 | 10 DOLI | 0.125   | 80                     |
| 5   | 16-20 | 10 DOLI | 0.0625  | 160                    |

### 6.3. Bond Stacking

Producers can increase their stake up to 10,000 times the base bond unit.

```
BOND_UNIT = 10 DOLI
MIN_STAKE = 1 × BOND_UNIT (10 DOLI)
MAX_STAKE = 10,000 × BOND_UNIT (100,000 DOLI)
```

Selection uses deterministic round-robin, not probabilistic lottery. Each bond unit grants one ticket in the rotation:

**Example (3 producers, 10 total bond units):**

```
Alice: 1 bond unit  (10 DOLI)   → 1 block every 10 slots
Bob:   5 bond units (50 DOLI)   → 5 blocks every 10 slots
Carol: 4 bond units (40 DOLI)   → 4 blocks every 10 slots
```

At 1 DOLI reward per block:

- Alice earns 1 DOLI per 10 slots (10% ROI per cycle)
- Bob earns 5 DOLI per 10 slots (10% ROI per cycle)
- Carol earns 4 DOLI per 10 slots (10% ROI per cycle)

**All producers earn identical ROI percentage regardless of stake size.**

| Parameter             | Value                    |
|-----------------------|--------------------------|
| Bond unit             | 10 DOLI                  |
| Min stake             | 10 DOLI (1 bond)         |
| Max stake             | 100,000 DOLI (10,000 bonds)|
| Block reward (Era 1)  | 1 DOLI                   |

#### Accessibility at Scale

At network maturity (60,000 total bonds across all producers):

| Your Stake | Bonds | Blocks/Week | Income/Week | Hardware |
|-----------|-------|-------------|-------------|----------|
| 10 DOLI   | 1     | ~1          | ~1 DOLI     | Any CPU  |
| 100 DOLI  | 10    | ~10         | ~10 DOLI    | Any CPU  |
| 1,000 DOLI| 100   | ~100        | ~100 DOLI   | Any CPU  |

No mining rigs. No staking pools. No minimum hardware requirements. A \/month VPS is sufficient to produce blocks and earn deterministic rewards.

### 6.4. Bond Lifecycle

The bond has a 4-year commitment period with per-bond FIFO tracking:

```
T_commitment = 12,614,400 blocks (~4 years)
T_withdrawal = next epoch boundary (max 1 hour)
```

Each bond tracks its own creation time. Withdrawal uses FIFO order (oldest bonds first), with penalty calculated individually per bond based on its age.

**Withdrawal is instant** — funds are returned in the same epoch boundary. No 7-day delay. No separate claim step.

Early withdrawal incurs a tiered penalty based on individual bond age:

| Bond Age  | Penalty | Returned |
|-----------|---------|----------|
| < 1 year  | 75%     | 25%      |
| 1-2 years | 50%     | 50%      |
| 2-3 years | 25%     | 75%      |
| 3+ years  | 0%      | 100%     |

A producer with bonds of mixed ages can withdraw selectively. The oldest bonds (lowest penalty) are withdrawn first. This rewards long-term commitment while allowing flexible exit.

All penalties are burned permanently, removing coins from circulation.

---

## 7. Producer Selection

For each slot, a deterministic round-robin process determines which producer creates the block:

```python
def select_producer(slot, active_producers):
    sorted_producers = sorted(active_producers, key=lambda p: p.public_key)
    total_tickets = sum(p.bond_count for p in sorted_producers)
    ticket_index = slot % total_tickets
    
    accumulated = 0
    for producer in sorted_producers:
        accumulated += producer.bond_count
        if ticket_index < accumulated:
            return producer.public_key
```

> **Anti-grinding invariant:** The producer for slot *s* is a pure function of `(s, ActiveSet(epoch(s)))` and bond counts. It MUST NOT depend on `prev_hash`, transaction ordering, timestamps within the drift window, or any other value the current producer can influence. This eliminates grinding attacks entirely: no block content affects future leader selection.

**Grinding is impossible in DOLI because the future is not a function of present choices. It is a function of time and a schedule fixed in advance.**

### 7.1. Fallback Mechanism

To avoid empty slots when the primary producer is offline, 5 fallback ranks activate in sequential 2-second windows:

| Time in slot | Eligible producer |
|--------------|-------------------|
| 0s - 2s      | rank 0 only       |
| 2s - 4s      | rank 1 only       |
| 4s - 6s      | rank 2 only       |
| 6s - 8s      | rank 3 only       |
| 8s - 10s     | rank 4 only       |

Each rank has an exclusive 2-second window. A block from rank *N* is valid only if `timestamp >= slot_start + N × 2s`. If multiple valid blocks arrive for the same slot, the one with lower rank wins.

### 7.2. Deterministic Rewards

In Proof of Work, a small miner may mine for years without finding a block. The reward is a lottery: probability proportional to hashpower, but with extreme variance. This forces miners to join pools, delegating control to centralized operators.

In Proof of Stake, validator selection is probabilistically weighted by stake. A small validator may wait weeks between blocks. Staking pools emerge for the same reason.

**Pools exist because rewards are probabilistic.** When the reward is a lottery, variance is the enemy of the small participant. Pools reduce variance in exchange for centralization and fees.

DOLI eliminates this problem entirely. Selection is deterministic round-robin:

```
Alice: 1 bond  → produces exactly 1 block every 10 slots
Bob:   5 bonds → produces exactly 5 blocks every 10 slots
Carol: 4 bonds → produces exactly 4 blocks every 10 slots
```

**There is no lottery. There is no variance. There is no luck.**

| System       | Selection                 | Variance | Pools | Energy        | Min Hardware    |
|--------------|---------------------------|----------|-------|---------------|-----------------|
| Bitcoin      | Lottery (hashpower)       | High     | Yes   | ~150 TWh/yr   | ASIC (,000+)  |
| Ethereum PoS | Lottery (stake)           | Medium   | Yes   | ~2.6 GWh/yr   | 32 ETH (0K+)  |
| Solana PoH   | Leader schedule (stake)   | Low      | Yes   | ~4 GWh/yr     | ,000+ server  |
| DOLI PoT     | Deterministic round-robin | **Zero** | **No**| **Negligible**| **Any CPU (/mo)**|

The distinction between DOLI and Solana deserves emphasis. Solana uses Proof of History as a clock, but leader selection is still stake-weighted with probabilistic elements. Validators require high-performance hardware. DOLI uses the VDF purely as a heartbeat — leader selection is a deterministic pure function of slot number and bond count. No hardware advantage exists.

A producer with 1 bond knows exactly when their next block will be produced. There is no need to join a pool to "smooth" rewards because there is nothing to smooth.

**The smallest participant receives the same percentage return as the largest.** The only difference is absolute magnitude, not probability or timing.

---

## 8. Chain Selection

When multiple valid chains exist, nodes must agree which to follow.

### 8.1. Weight-based Fork Choice

The canonical chain is the one with the highest accumulated producer weight:

```
accumulated_weight(block) = accumulated_weight(parent) + producer_weight
```

Producer weight derives from seniority:

| Years active | Weight |
|--------------|--------|
| 0-1          | 1      |
| 1-2          | 2      |
| 2-3          | 3      |
| 3+           | 4      |

This prevents attacks where an attacker creates many new-producer blocks to outpace a chain built by established producers.

---

## 9. Incentive

By convention, the first transaction in a block is a special transaction that creates new coins owned by the producer of the block. This adds an incentive for nodes to support the network and provides a way to initially distribute coins into circulation.

### 9.1. Emission

| Parameter        | Value                     |
|------------------|---------------------------|
| Initial reward   | 1 DOLI/block              |
| Block time       | 10 seconds                |
| Halving interval | 12,614,400 blocks (~4 years) |
| Total supply     | 25,228,800 DOLI           |

| Era | Years | Reward  | Cumulative  | % of total |
|-----|-------|---------|-------------|------------|
| 1   | 0-4   | 1.0     | 12,614,400  | 50.00%     |
| 2   | 4-8   | 0.5     | 18,921,600  | 75.00%     |
| 3   | 8-12  | 0.25    | 22,075,200  | 87.50%     |
| 4   | 12-16 | 0.125   | 23,652,000  | 93.75%     |
| 5   | 16-20 | 0.0625  | 24,440,400  | 96.88%     |
| 6   | 20-24 | 0.03125 | 24,834,600  | 98.44%     |

### 9.2. Reward Maturity

Outputs from reward transactions require 100 confirmations before they can be spent.

### 9.3. Fees

```
fee = sum(inputs) - sum(outputs)
```

The fee goes to the block producer. A minimum fee rate prevents spam.

---

## 10. Infractions

### 10.1. Invalid Blocks

The network rejects blocks that:

- Have invalid VDF proofs
- Are produced by wrong producer for that slot
- Have timestamps outside the valid window
- Contain invalid transactions

The producer loses the slot opportunity. The bond remains intact.

### 10.2. Inactivity

If a producer fails to produce for 60,480 consecutive blocks (~7 days):

- Classified as inactive
- Excluded from block scheduling
- Bond remains locked (no penalty for inactivity)
- Reactivates automatically upon resuming production

Inactivity is not punished — it is tolerated. A producer who goes offline loses income (missed block rewards) but not capital (bond remains intact). This is a deliberate design choice: penalizing downtime would discourage small operators with less reliable infrastructure.

### 10.3. Double Production

If a producer creates two different blocks for the same slot, anyone can construct a proof of this infraction.

**Penalty:**

- 100% of bond burned permanently
- Immediate exclusion from producer set
- To reactivate: new registration with `T_registration × 2`

This is the only infraction that results in slashing because it is the only one that is unambiguously intentional.

---

## 11. Security

If the attacker controls less sequential computation capacity than the honest network, the probability of catching up to the honest chain decreases rapidly with the number of slots difference.

In this system, an attacker cannot "accelerate" the production of an alternative chain by adding parallel hardware, because each block requires a sequential computation of fixed duration.

### 11.1. Attack Cost

To dominate the network, an attacker would need to:

1. Register more producers than honest ones (time cost per identity)
2. Lock more bonds than honest participants (economic cost)
3. Maintain those producers active (operational cost)
4. Risk total bond loss if detected double-producing

The protocol automatically regulates the rate at which new identities can join.

### 11.2. Attack Probability

Given that blocks require fixed sequential time *T*, an attacker who begins producing an alternative chain at block *N* cannot catch up to the honest chain regardless of parallel resources.

Let:

- *h* = honest chain length (blocks)
- *a* = attacker chain length (blocks)
- *T* = time per block (fixed, ~10 seconds)

If the attacker starts at time *t₀* with deficit *d = h - a*:

```
time_to_catch_up = d × T (minimum)
```

Since the honest network continues producing blocks during this time:

```
new_honest_blocks = d × T / T = d
```

**The attacker's deficit remains constant or grows.** Unlike Proof of Work, where an attacker with 51% hashpower eventually wins, in Proof of Time an attacker with any amount of parallel hardware cannot reduce the time required.

The only attack vector is controlling >50% of registered producers, which requires:

1. **Time:** `T_registration` per identity (scales with demand, cannot be parallelized)
2. **Capital:** Bond per identity (10+ DOLI each)
3. **Risk:** 100% bond loss if detected double-producing

For a network with 1,000 honest producers and 100,000 DOLI bonded, an attacker would need:

- 1,001 identities minimum
- 10,010 DOLI in bonds
- 1,001 × `T_registration` in sequential time

**The sequential time requirement cannot be bypassed.** An attacker with 1,000 machines still needs 1,001 × `T_registration` wall-clock time to register 1,001 identities.

---

## 12. Reclaiming Disk Space

Once the latest transaction in a coin is buried under enough blocks, the spent transactions before it can be discarded to save disk space. Transactions are hashed in a Merkle tree, with only the root included in the block hash.

A block header with no transactions is approximately 340 bytes. With blocks every 10 seconds, that is ~1 GB per year for headers alone.

---

## 13. Simplified Payment Verification

It is possible to verify payments without running a full node. A user only needs to keep a copy of the block headers of the longest chain and obtain the Merkle branch linking the transaction to the block it is timestamped in.

---

## 14. Privacy

The public can see that someone is sending an amount to someone else, but without information linking the transaction to anyone. This is similar to the level of information released by stock exchanges.

As an additional firewall, a new key pair should be used for each transaction to keep them from being linked to a common owner.

---

## 15. Distribution

There is no premine, ICO, treasury, or special allocations. Every coin in circulation comes from block rewards.

### 15.1. Genesis Block

The genesis block contains a single coinbase transaction with the message:

> *"Time is the only fair currency. 01/Feb/2026"*

This serves as proof that no blocks were mined before this date and as a statement of the system's philosophy.

The genesis block contains exactly:

- One coinbase transaction with 1 DOLI (standard reward)
- Zero additional transactions

**No hidden allocations exist.**

---

## 16. Immutability

Transactions are final. There are no mechanisms to reverse transactions, recover funds, or modify history.

| Situation            | Protocol response   |
|----------------------|---------------------|
| Lost private keys    | Funds lost permanently |
| Erroneous transaction| Not reversible      |
| Exchange hack        | Not reversible      |
| Court order          | Not enforceable     |

**The code is law. Transactions are final.**

---

## 17. Protocol Updates

Software requires maintenance. Bugs must be fixed. The question is: who decides?

In centralized systems, the operator decides. In Bitcoin, informal consensus among developers, miners, and users determines which changes are adopted. This works but is slow and contentious.

DOLI formalizes the process. Updates are signed by maintainers and reviewed by producers.

### 17.1. Release Signing

Every release requires signatures from 3 of 5 maintainers. A single compromised key cannot push malicious code.

### 17.2. Veto Period

When a new version is published, producers have 7 days to review. Any producer can vote to reject. If 40% or more vote against, the update is rejected.

| Veto votes | Result   |
|------------|----------|
| < 40%      | Approved |
| ≥ 40%      | Rejected |

The threshold is weighted by seniority. An attacker cannot create many new nodes to force an update through.

### 17.3. Adoption

After approval, producers have 48 hours to update. Nodes running outdated versions cannot produce blocks. This is not punishment—it is protection. A vulnerability in old code affects the entire network.

The choice is simple: participate in consensus with current software, or do not participate.

---

## 18. Live Network

DOLI is not a proposal. The network described in this paper is operational.

As of March 2026, the mainnet runs with multiple independent producers across geographically distributed nodes. The source code is open and the chain state is publicly verifiable.

| Metric | Value |
|--------|-------|
| Block time | 10 seconds |
| VDF computation | ~55ms per block |
| Block propagation | < 500ms (5-node network) |
| Forks since genesis | 0 |
| Missed slot rate | < 10% (fallback mechanism) |
| Node hardware | Standard VPS, any CPU |
| Minimum bond | 10 DOLI |

```
Genesis:    March 2026
Consensus:  Proof of Time (VDF heartbeat + deterministic round-robin)
Status:     Live
Source:     https://github.com/e-weil/doli
Explorer:   https://doli.network
```

---

## 19. Conclusion

We have proposed a system for electronic transactions that requires no trust in institutions, no massive energy expenditure, and no capital accumulation to participate in consensus.

We started with the usual framework of coins made from digital signatures, which provides strong control of ownership. This is incomplete without a way to prevent double-spending. To solve this, we proposed a peer-to-peer network using verifiable delay functions to anchor consensus to time.

**Nodes vote with their time.** The network cannot be accelerated by wealth or parallelized by hardware. One hour of sequential computation is one hour, whether performed by an individual or a nation-state.

**Rewards are deterministic, not probabilistic.** A participant knows exactly when their next block will be produced. Pools are unnecessary. The smallest participant receives the same percentage return as the largest.

The network is robust in its simplicity. Nodes work with little coordination. They do not need to be identified, since messages are not routed to any particular place and only need to be delivered on a best effort basis. Nodes can leave and rejoin the network at will, accepting the heaviest chain as proof of what happened while they were gone.

**The rules are fixed at genesis. The emission is predictable.**

Any needed rules and incentives can be enforced with this consensus mechanism.

---

**DOLI v1.0.25**

*"Time is the only fair currency."*

**E. Weil** · contact: weil@doli.network

---

## References

1. Nakamoto, S. (2008). *Bitcoin: A Peer-to-Peer Electronic Cash System.*

2. Boneh, D., Bonneau, J., Bünz, B., & Fisch, B. (2018). *Verifiable Delay Functions.* In Advances in Cryptology – CRYPTO 2018.

3. Wesolowski, B. (2019). *Efficient Verifiable Delay Functions.* In Advances in Cryptology – EUROCRYPT 2019.

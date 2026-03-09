# DOLI<sub>τ</sub>

## Deterministic Ordering through Linear Iterations

### A Peer-to-Peer Electronic Cash System Based on Verifiable Time

**E. Weil** · weil@doli.network

---

## Abstract

We propose a peer-to-peer electronic cash system where the only resource required for consensus is time — the one resource distributed equally to all participants.

Block production follows deterministic rotation: a participant with one bond knows exactly when their next block will be produced. Pools are unnecessary. Rewards compound into productive stake, creating predictable exponential growth for every participant regardless of size.

A new producer receiving 10 DOLI can reinvest block rewards to double their stake at regular intervals. The doubling rate is identical for all participants — one bond or three thousand. No lottery. No variance. No pools. Just time.

Transactions are ordered through verifiable delay functions that cannot be parallelized. No special hardware is required. Any CPU can participate in consensus. The result is a system where consensus weight emerges from time rather than trust, capital, or scale.

---

## 1. Introduction

Every consensus mechanism ever designed shares one assumption: security requires a scarce resource that can be accumulated. Bitcoin chose energy. Ethereum chose capital. Both created systems where the largest participant has a structural advantage over the smallest.

This assumption is wrong. There exists a resource that cannot be accumulated, cannot be parallelized, and is distributed equally to every participant on Earth: **time**.

One second passes at the same rate for an individual running a single node as it does for a nation-state with unlimited budget. No amount of money can buy more time. No amount of hardware can make time pass faster.

We propose an electronic cash system where consensus derives from verifiable sequential computation — proof that real time has elapsed. The system is secure as long as honest participants collectively maintain more sequential computation presence than any cooperating group of attackers.

### 1.1. Why Now

The blockchain trilemma assumes three competing properties: decentralization, security, and scalability. Proposed solutions trade one for another — energy for security (PoW), decentralization for scalability (PoS), simplicity for throughput (sharding). These tradeoffs arise because every prior system anchors consensus to a resource that can be accumulated: hashpower, stake, or storage.

Verifiable Delay Functions, first formalized by Boneh, Bonneau, Bünz, and Fisch [2] in 2018, made it possible to prove that sequential time has elapsed without trusting the prover. Before this cryptographic primitive existed, Proof of Time was not implementable.

By anchoring consensus to sequential computation:

- **Decentralization:** No special hardware required. Any CPU can participate.
- **Security:** Attacking requires real time, not purchasable resources.
- **Scalability:** Block production is bounded by time, not by resource competition.

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

### 2.2. Output Structure

Each unspent output contains five fields:

```
┌──────────────────────────────────────────────────────────┐
│                    Output (UTXO)                         │
├──────────────┬───────────────────────────────────────────┤
│ type         │ What kind of value (transfer, bond, ...)  │
│ amount       │ How much                                  │
│ owner        │ Who can spend (public key hash)           │
│ lock_until   │ When it becomes spendable                 │
│ extra_data   │ Extensible spending conditions            │
└──────────────┴───────────────────────────────────────────┘
```

The first four fields cover all cash operations. The fifth field — `extra_data` — reserves space for arbitrary spending conditions without requiring changes to the output format.

For basic transfers and bonds, `extra_data` is empty. New output types define how `extra_data` is interpreted, adding validation rules to the protocol while the structure remains fixed from genesis.

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

> **Note:** The VDF proves that *N* sequential operations were executed — time is the effective lower bound since no known technique accelerates sequential computation through parallelization. The VDF serves as a heartbeat (proof of presence), not as a randomness source. Producer selection is a pure function of `(slot, ActiveSet(epoch))`, fixed at epoch start, independent of VDF speed. Faster hardware provides no scheduling advantage.

For each block, the producer must calculate:

```
input  = HASH(prefix || previous_hash || tx_root || slot || producer_key)
output = HASH^n(input)
proof  = π
```

Where *n* is the difficulty parameter that determines how long the computation takes.

### 4.1. VDF Construction

DOLI uses an **iterated hash chain** (BLAKE3), not an algebraic VDF over groups of unknown order (Wesolowski [3], Pietrzak). The distinction matters:

| Property | Algebraic VDF (Wesolowski) | Iterated Hash Chain (DOLI) |
|----------|---------------------------|---------------------------|
| Verification | *O(1)* — constant time | *O(T)* — must recompute |
| Trusted setup | Required (RSA group) | None |
| Quantum resistance | Uncertain | Hash-based (conservative) |
| Implementation | Complex (GMP/big integers) | Simple (~10 lines) |

Algebraic VDFs offer *O(1)* verification, which is critical when the delay parameter *T* is large (minutes to hours). DOLI's block VDF requires only *T* = 800,000 iterations (~55ms), making *O(T)* verification acceptable — every node recomputes the chain in the same ~55ms.

The tradeoff is deliberate: DOLI gains simplicity, auditability, and no trusted setup at the cost of linear verification. For a heartbeat proof where *T* is small, this is the correct engineering choice.

```
Input: prev_hash ∥ slot ∥ producer_key
         │
         ▼
    ┌─────────┐
    │ BLAKE3  │ ◄──┐
    └────┬────┘    │
         │         │
         └─────────┘  × T iterations (T = 800,000)
         │
         ▼
      Output: h_T = H^T(input)
```

**Verification:** A verifier recomputes *h_T = H^T(input)* and checks *h_T == claimed_output*. No shortcuts exist — the sequential dependency *h_{i+1} = H(h_i)* prevents parallelization.

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

Every consensus system enforces a scarce resource. In DOLI, that resource is sequential time.

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

### 5.3. Throughput

With 10-second block times and a maximum block size of 1 MB:

| Metric              | Value          |
|---------------------|----------------|
| Block time          | 10 seconds     |
| Max block size      | 1 MB           |
| Avg transaction     | ~250 bytes     |
| Theoretical max TPS | ~400           |
| Practical TPS       | 100-200        |

DOLI does not compete on raw throughput. It competes on accessibility:

| System       | TPS       | Min hardware to participate |
|--------------|-----------|----------------------------|
| Bitcoin      | ~7        | ASIC ($5,000+)             |
| Ethereum PoS | ~30       | 32 ETH + server ($100K+)   |
| Solana       | ~4,000    | 256GB RAM server ($10K+)   |
| DOLI         | ~200      | Any CPU ($5/mo VPS)        |

200 TPS on a $5/month VPS is a different proposition than 4,000 TPS on hardware most people cannot afford. The throughput is sufficient for a cash system; the accessibility is sufficient for global participation.

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

Producers can increase their stake up to 3,000 times the base bond unit.

```
BOND_UNIT = 10 DOLI
MIN_STAKE = 1 × BOND_UNIT (10 DOLI)
MAX_STAKE = 3,000 × BOND_UNIT (30,000 DOLI)
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
| Max stake             | 30,000 DOLI (3,000 bonds) |
| Block reward (Era 1)  | 1 DOLI                   |

#### Accessibility at Scale

At network maturity (18,000 total bonds across all producers):

| Your Stake | Bonds | Blocks/Week | Income/Week | Hardware |
|-----------|-------|-------------|-------------|----------|
| 10 DOLI   | 1     | ~3          | ~3 DOLI     | Any CPU  |
| 100 DOLI  | 10    | ~34         | ~34 DOLI    | Any CPU  |
| 1,000 DOLI| 100   | ~336        | ~336 DOLI   | Any CPU  |

No mining rigs. No staking pools. No minimum hardware requirements. A $5/month VPS is sufficient.

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

For each slot, a deterministic function selects the block producer. Let *P* = {*p₁*, ..., *p_n*} be the active set sorted by public key, and *b(p_i)* the bond count of producer *p_i*. Define *B* = Σ *b(p_i)*.

```
producer(s) = p_j  where j = min{j : Σ_{i=1}^{j} b(p_i) > s mod B}
```

The function is pure: `producer(s) = f(s, ActiveSet(epoch(s)))`. It depends on no value the current producer can influence — not `prev_hash`, not transaction ordering, not timestamps within the drift window. **Grinding is impossible because the schedule is a function of time alone, fixed at epoch start.**

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

### 7.2. Comparison with Existing Systems

Pools exist in PoW and PoS because rewards are probabilistic — variance forces small participants to delegate control to centralized operators. DOLI's deterministic round-robin eliminates variance entirely (see Section 6.3).

| System       | Selection                 | Variance | Pools | Energy        | Min Hardware    |
|--------------|---------------------------|----------|-------|---------------|-----------------|
| Bitcoin      | Lottery (hashpower)       | High     | Yes   | ~150 TWh/yr   | ASIC ($5,000+)  |
| Ethereum PoS | Lottery (stake)           | Medium   | Yes   | ~2.6 GWh/yr   | 32 ETH ($100K+)  |
| Solana PoH   | Leader schedule (stake)   | Low      | Yes   | ~4 GWh/yr     | $10,000+ server  |
| DOLI PoT     | Deterministic round-robin | **Zero** | **No**| **Negligible**| **Any CPU ($5/mo)**|

Solana uses Proof of History as a clock, but leader selection remains stake-weighted with probabilistic elements and requires high-performance hardware. DOLI uses the VDF purely as a heartbeat — leader selection is a pure function of `(slot, ActiveSet(epoch))`. No hardware advantage exists.

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

### 9.4. Compound Growth

DOLI rewards compound into productive capital. Every DOLI earned from block production can be reinvested as additional bond units, increasing future block assignments proportionally.

**Definition.** Let *b* = a producer's bond count, *B* = total network bonds, *R* = block reward, *S* = slots per week (60,480). The producer's weekly earnings and doubling time are:

```
E(b) = S · R · b / B          (weekly earnings)
D    = BOND_UNIT · B / (S · R) (doubling time in weeks)
```

*D* is independent of *b*. A producer with 1 bond and a producer with 1,000 bonds both double their stake in *D* weeks. The growth rate is uniform; only the absolute magnitude differs.

**Example (Era 1, *B* = 18,000, *R* = 1 DOLI):**

*D* = 10 × 18,000 / (60,480 × 1) ≈ 3 weeks.

```
Week 0:   1 bond     →   3.3 DOLI/week
Week 3:   2 bonds    →   6.6 DOLI/week
Week 6:   4 bonds    →    13 DOLI/week
Week 12:  16 bonds   →    53 DOLI/week
Week 24:  256 bonds  →   853 DOLI/week
```

Starting with 10 DOLI, a producer who reinvests all rewards reaches the 3,000-bond cap within months. This trajectory is calculable before the first block is produced.

**Self-regulation:** As *B* grows, *D* increases proportionally. Rapid early growth naturally converges toward stable distribution without governance intervention. Late entrants face longer doubling times but benefit from a more secure and valuable network.

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

**Theorem (Sequential Deficit).** Let *T* be the fixed sequential time per block. An attacker who begins an alternative chain with deficit *d* ≥ 1 blocks cannot reduce *d* regardless of parallel computational resources.

**Proof.** Let *t₀* be the time the attacker begins forking. Define:

- *H(t)* = honest chain length at time *t*
- *A(t)* = attacker chain length at time *t*
- *d(t) = H(t) − A(t)* = deficit at time *t*

At *t₀*: *d(t₀) = d* ≥ 1.

Each block requires exactly *T* sequential computation. The attacker produces at most one block per *T* seconds per chain (sequential dependency: block *i+1* requires hash of block *i*). The honest network also produces at most one block per *T* seconds.

After elapsed time *Δt*:

```
H(t₀ + Δt) ≤ H(t₀) + ⌊Δt / T⌋
A(t₀ + Δt) ≤ A(t₀) + ⌊Δt / T⌋
```

Therefore:

```
d(t₀ + Δt) = H(t₀ + Δt) − A(t₀ + Δt) ≥ d(t₀) = d
```

The deficit is monotonically non-decreasing. Adding parallel hardware allows computing multiple *independent* chains, but each chain is sequential — the attacker cannot merge parallel chains into a single longer one. ∎

**Contrast with Proof of Work:** In PoW, an attacker with >50% hashpower reduces the deficit probabilistically because hash attempts are parallelizable. In Proof of Time, the sequential dependency *h_{i+1} = H(h_i)* makes each chain inherently serial. The attacker's deficit is bounded below by its initial value, regardless of budget.

The only attack vector is controlling >50% of bond-weighted slots, which requires:

1. *T_registration* sequential time per identity (cannot be parallelized per identity)
2. *BOND_UNIT* capital per identity
3. 100% bond loss risk if detected double-producing

### 11.3. The CPU Accumulation Objection

A natural objection: "Time cannot be accumulated, but the capacity to run VDFs can — more CPUs enable more parallel identities."

This is correct and by design. An attacker with *M* machines can register *M* identities in parallel, each completing *T_registration* independently. However, each identity still requires:

1. **Sequential time:** *T_registration* wall-clock seconds (cannot be reduced by adding cores)
2. **Capital:** *BOND_UNIT* locked per identity (linear cost in *M*)
3. **Ongoing presence:** One VDF heartbeat per slot per identity (linear operational cost in *M*)

The attack cost is therefore *O(M)* in capital and operational expense, identical to Proof of Stake's security model. The critical difference is the **time floor**: even with unlimited capital, registering *M* identities takes at least *T_registration* wall-clock time. The registration difficulty adjusts per epoch (Section 6.1), so a burst of registrations increases *T_registration* for subsequent attempts.

Compare with PoW: an attacker with *M* ASICs gains *M×* hashpower immediately, with no per-identity time delay. In DOLI, the same *M* machines yield *M* identities, but the registration pipeline enforces a sequential bottleneck per identity and the capital requirement scales linearly.

The system does not claim immunity from wealthy adversaries — no system can. It claims that time imposes an irreducible cost floor that capital alone cannot bypass.

### 11.4. Safety Theorem

**Theorem.** Let *n* = total bond-weighted slots per epoch. An attacker controlling *f* < *n/2* bond-weighted slots cannot produce a heavier chain than the honest network over any interval of *k* ≥ 1 epochs.

**Proof.** Define per epoch *e*:

- *S_h(e)* = set of slots assigned to honest producers
- *S_a(e)* = set of slots assigned to attacker producers
- *w(p)* = seniority weight of producer *p* ∈ {1, 2, 3, 4}

The schedule is a pure function of `(slot, ActiveSet(epoch))` — no block content influences it.

The accumulated chain weight over *k* epochs:

```
W_h(k) = Σ_{e=1}^{k} Σ_{s ∈ S_h(e)} w(producer(s))
W_a(k) = Σ_{e=1}^{k} Σ_{s ∈ S_a(e)} w(producer(s))
```

Since *f < n/2*, we have *|S_a(e)| < |S_h(e)|* for all *e*. Additionally, seniority weighting (Section 8.1) penalizes new identities: *w(new) = 1* while *w(established) ≤ 4*. Therefore *W_h(k) > W_a(k)* for all *k* ≥ 1.

By the Sequential Deficit theorem (11.2), the attacker cannot compensate by computing faster — the VDF's sequential dependency prevents parallel acceleration. ∎

**Corollary.** An attacker starting from zero needs ~3 years of sustained presence before seniority weight equals an established honest producer, even with equal bond count. The attack window is therefore bounded not just by capital but by calendar time.

---

## 12. Reclaiming Disk Space

Once the latest transaction in a coin is buried under enough blocks, the spent transactions before it can be discarded to save disk space. Transactions are hashed in a Merkle tree, with only the root included in the block hash.

A block header with no transactions is approximately 340 bytes. With blocks every 10 seconds, that is ~1 GB per year for headers alone.

---

## 13. Simplified Payment Verification

It is possible to verify payments without running a full node. A user only needs to keep a copy of the block headers of the longest chain and obtain the Merkle branch linking the transaction to the block it is timestamped in.

---

## 14. Privacy

DOLI adopts the pseudonymous privacy model described by Nakamoto [1]: transactions are public, but identities behind keys are not. Users generate new key pairs per transaction to prevent linkage. This provides privacy equivalent to public stock exchange disclosures — amounts and flows are visible, participants are not.

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

## 19. Programmable Outputs

The `extra_data` field in every output (Section 2.2) makes DOLI outputs programmable without a virtual machine, without gas, and without a Turing-complete scripting language.

### 19.1. Design Principles

Smart contract platforms chose generality: a universal computer on every node, executing arbitrary code at every transaction. The cost is complexity, attack surface, and unpredictable execution.

DOLI chooses the opposite: **declarative conditions**. An output does not contain code to execute — it contains conditions to verify. The distinction matters:

| Property | Ethereum (EVM) | Bitcoin Script | DOLI Conditions |
|----------|---------------|----------------|-----------------|
| Model | Account + VM | UTXO + Stack machine | UTXO + Native rules |
| Execution | Turing-complete | Intentionally limited | Declarative verification |
| Gas/Fees | Unpredictable (gas) | Fixed | Fixed (no metering) |
| State | Mutable shared state | Stateless | Stateless |
| Attack surface | Unbounded | Small | Minimal |
| Runtime | Interpreted (slow) | Interpreted | Compiled (native) |

Conditions are not interpreted at runtime — they are compiled into the node binary as native Rust validation rules. Each output type defines which conditions are valid and how `extra_data` is decoded. Adding a new condition type is a protocol upgrade, not a deployment.

### 19.2. Condition Language

Conditions are composable predicates. Each condition returns true or false. An output is spendable when all its conditions are satisfied.

```
Condition := Signature(pubkey_hash)
           | Multisig(threshold, [pubkey_hash, ...])
           | Hashlock(hash)
           | Timelock(min_height)
           | TimelockExpiry(max_height)
           | And(Condition, Condition)
           | Or(Condition, Condition)
           | Threshold(n, [Condition, ...])
```

**Encoding:** Conditions are serialized into `extra_data` as a compact binary format. For basic transfers, `extra_data` is empty — the default condition is `Signature(owner)`.

**Verification cost:** Every condition resolves to a fixed number of cryptographic operations (signature checks, hash comparisons, height comparisons). No loops. No recursion. No unbounded computation. Verification cost is known before execution.

### 19.3. Native Output Types

Each output type is a named pattern over the condition language:

| Type | Conditions | Use Case |
|------|-----------|----------|
| Transfer | `Signature(owner)` | Standard payment |
| Bond | `Signature(owner) AND Protocol(withdrawal)` | Producer stake |
| Multisig | `Multisig(n, keys)` | Shared custody |
| Hashlock | `Signature(owner) AND Hashlock(h)` | Atomic swaps |
| HTLC | `(Hashlock(h) AND Timelock(t)) OR TimelockExpiry(t+d)` | Payment channels |
| Escrow | `Threshold(2, [buyer, seller, arbiter])` | Trustless commerce |
| Vesting | `Signature(owner) AND Timelock(unlock_height)` | Time-locked grants |

These are not separate implementations — they are compositions of the same primitive conditions. A developer does not write a smart contract. A developer selects conditions.

### 19.4. What This Enables

**Without a virtual machine:**

- **Decentralized exchanges:** Atomic swaps between DOLI and any chain that supports hash locks. No intermediary, no custody, no counterparty risk.
- **Payment channels:** Off-chain transactions with on-chain settlement. HTLCs enable a Lightning-equivalent network natively.
- **Multi-party custody:** Corporate treasuries, DAOs, inheritance — any scenario requiring N-of-M authorization.
- **Trustless escrow:** Buyer, seller, and arbiter each hold a key. Any two can release funds.
- **Vesting schedules:** Time-locked outputs for team allocations, grants, or contractual obligations.

**Without shared mutable state:**

Every output is independent. Spending one output cannot affect another. There is no reentrancy, no front-running, no MEV. Transactions are fully parallelizable — validation scales linearly with cores.

### 19.5. What This Does Not Enable

DOLI outputs cannot maintain persistent state across transactions. There is no on-chain storage, no loops, no arbitrary computation. This is deliberate.

Applications requiring shared state — automated market makers, lending protocols, on-chain governance with complex voting — belong on Layer 2 or application-specific chains that settle to DOLI.

The base layer provides: **value transfer, time-anchored ordering, and programmable spending conditions.** Everything else builds on top.

---

## 20. Scope

DOLI optimizes for moving value with deterministic finality, predictable timing, and extensible spending conditions. The base layer is intentionally minimal — but extensible by design.

This constraint is a feature. A system that does one thing well is more secure, more auditable, and more resistant to governance capture than a system that attempts to be a universal computer. Bitcoin demonstrated that a focused protocol can sustain a trillion-dollar network. Complexity is not a prerequisite for value.

The difference: Bitcoin's output format was fixed in 2009. DOLI's output format was designed in 2026 with seventeen years of hindsight. The `extra_data` field exists from genesis — no SegWit, no Taproot, no backward-compatibility hacks required.

---

## 21. Conclusion

We have proposed a system for electronic transactions that requires no trust in institutions, no massive energy expenditure, and no capital accumulation to participate in consensus.

We started with the usual framework of coins made from digital signatures, which provides strong control of ownership. This is incomplete without a way to prevent double-spending. To solve this, we proposed a peer-to-peer network using verifiable delay functions to anchor consensus to time.

**Nodes vote with their time.** The network cannot be accelerated by wealth or parallelized by hardware. One hour of sequential computation is one hour, whether performed by an individual or a nation-state.

**Rewards are deterministic, not probabilistic.** A participant knows exactly when their next block will be produced. Pools are unnecessary. The smallest participant receives the same percentage return as the largest.

The network is robust in its simplicity. Nodes work with little coordination. They do not need to be identified, since messages are not routed to any particular place and only need to be delivered on a best effort basis. Nodes can leave and rejoin the network at will, accepting the heaviest chain as proof of what happened while they were gone.

**The rules are fixed at genesis. The emission is predictable.**

Any needed rules and incentives can be enforced with this consensus mechanism.

---

**DOLI v2.0.26**

*"Time is the only fair currency."*

**E. Weil** · contact: weil@doli.network

---

## References

1. Nakamoto, S. (2008). *Bitcoin: A Peer-to-Peer Electronic Cash System.*

2. Boneh, D., Bonneau, J., Bünz, B., & Fisch, B. (2018). *Verifiable Delay Functions.* In Advances in Cryptology – CRYPTO 2018.

3. Wesolowski, B. (2019). *Efficient Verifiable Delay Functions.* In Advances in Cryptology – EUROCRYPT 2019.

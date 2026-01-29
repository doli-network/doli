# DOLI<sub>τ</sub>

## Deterministic Ordering through Linear Iterations

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

Commerce on the Internet has come to rely almost exclusively on trusted financial institutions to process electronic payments. While the model works well enough for most transactions, it suffers from the inherent weaknesses of the trust-based model.

Transactions can be reversed, which increases costs of mediation and limits the minimum practical transaction size. The possibility of reversal requires merchants to request more information from customers than otherwise necessary. A certain percentage of fraud is accepted as unavoidable.

These costs and payment uncertainties can be avoided using physical currency, but no mechanism exists to make payments over a communication channel without a trusted party.

What is needed is an electronic payment system based on cryptographic proof instead of trust, allowing any two willing parties to transact directly without the need for a trusted third party.

In this paper, we propose a solution to the double-spending problem using a peer-to-peer network that anchors consensus to the passage of time through verifiable delay functions. The system is secure as long as honest participants collectively control more sequential computation capacity than any cooperating group of attackers.

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

We define an electronic coin as a chain of digital signatures. Each owner transfers the coin to the next by digitally signing a hash of the previous transaction and the public key of the next owner and adding these to the end of the coin. A payee can verify the signatures to verify the chain of ownership.

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

The problem is that the payee cannot verify that one of the owners did not double-spend the coin. A common solution is to introduce a trusted central authority that checks every transaction. After each transaction, the coin must be returned to the authority to issue a new one, and only coins issued directly from the authority are trusted not to be double-spent.

The problem with this solution is that the fate of the entire monetary system depends on the entity running the authority, with every transaction having to pass through it.

We need a way for the payee to know that the previous owners did not sign any earlier transactions. For our purposes, the earliest transaction is the one that counts, so we do not care about later attempts to double-spend. The only way to confirm the absence of a transaction is to be aware of all transactions. To accomplish this without a trusted party, transactions must be publicly announced, and we need a system for participants to agree on a single history of the order in which they were received.

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
    │ SHA-256 │ ◄──┐
    └────┬────┘    │
         │         │
         └─────────┘  × 10,000,000 iterations
         │
         ▼
      Output + Proof π
```

**Verification:** Any node can verify by recomputing the hash chain or by checking π using the Wesolowski proof scheme.

The implementation uses iterated SHA-256 hashing. This construction is well-studied and has known security properties.

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

To maintain consistent timing (~700ms) regardless of hardware, iterations adjust dynamically:

```
TARGET_TIME    = 700ms
TOLERANCE      = 10%
MAX_ADJUSTMENT = 20% per cycle
```

If measured VDF time deviates more than 10%, iterations adjust by up to 20% per cycle. This allows heterogeneous hardware to participate while maintaining consistent block times.

With 10-second slots, the VDF takes ~700ms, leaving ~9 seconds for block construction and propagation.

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

Every producer registration must lock an activation bond: coins immobilized for a fixed period.

```
B(H) = B_initial × 0.7^era(H)
```

| Era | Years | Bond  | Reward  | Blocks equivalent |
|-----|-------|-------|---------|-------------------|
| 1   | 0-4   | 1,000 | 1.0     | 1,000             |
| 2   | 4-8   | 700   | 0.5     | 1,400             |
| 3   | 8-12  | 490   | 0.25    | 1,960             |
| 4   | 12-16 | 343   | 0.125   | 2,744             |
| 5   | 16-20 | 240   | 0.0625  | 3,840             |

The bond decays 30% per era while reward decays 50%. This makes joining as a producer progressively more expensive in relative terms.

### 6.3. Bond Stacking

Producers can increase their stake up to 100 times the base bond unit.

```
BOND_UNIT = 1,000 DOLI
MIN_STAKE = 1 × BOND_UNIT (1,000 DOLI)
MAX_STAKE = 100 × BOND_UNIT (100,000 DOLI)
```

Selection uses deterministic round-robin, not probabilistic lottery. Each bond unit grants one ticket in the rotation:

**Example (3 producers, 10 total bond units):**

```
Alice: 1 bond unit  (1,000 DOLI)  → 1 block every 10 slots
Bob:   5 bond units (5,000 DOLI)  → 5 blocks every 10 slots
Carol: 4 bond units (4,000 DOLI)  → 4 blocks every 10 slots
```

At 1 DOLI reward per block:

- Alice earns 1 DOLI per 10 slots (0.1% ROI per cycle)
- Bob earns 5 DOLI per 10 slots (0.1% ROI per cycle)
- Carol earns 4 DOLI per 10 slots (0.1% ROI per cycle)

**All producers earn identical ROI percentage regardless of stake size.**

| Parameter             | Value                  |
|-----------------------|------------------------|
| Bond unit             | 1,000 DOLI             |
| Min stake             | 1,000 DOLI (1 bond)    |
| Max stake             | 100,000 DOLI (100 bonds)|
| Block reward (Era 1)  | 1 DOLI                 |

### 6.4. Bond Lifecycle

The bond has a 4-year commitment period:

```
T_commitment = 12,614,400 blocks (~4 years)
T_unbonding  = 259,200 blocks (~30 days)
```

After 4 years, the producer can exit without penalty or renew with a reduced bond at the current era rate.

Early exit incurs a proportional penalty:

```
penalty_pct = (time_remaining × 100) / T_commitment
return = bond × (100 - penalty_pct) / 100
```

Early exit penalties recycle to the reward pool. Slashing penalties are burned.

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

To avoid empty slots when the primary producer is offline:

| Time in slot | Eligible producer |
|--------------|-------------------|
| 0s - 3s      | rank 0 only       |
| 3s - 6s      | rank 0 or 1       |
| 6s - 10s     | rank 0, 1, or 2   |

A block from rank *N* is valid only if `timestamp >= slot_start + (N × 3)`. If multiple valid blocks arrive for the same slot, the one with lower rank wins.

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

| System       | Selection                 | Variance | Pools required |
|--------------|---------------------------|----------|----------------|
| Bitcoin      | Lottery (hashpower)       | High     | Yes            |
| Ethereum PoS | Lottery (stake)           | Medium   | Yes            |
| DOLI         | Deterministic round-robin | Zero     | No             |

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

If a producer fails to produce when selected for 50 consecutive slots:

- Removed from active set
- Bond remains locked
- Can reactivate with new registration VDF

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
2. **Capital:** Bond per identity (1,000+ DOLI each)
3. **Risk:** 100% bond loss if detected double-producing

For a network with 1,000 honest producers and 1,000,000 DOLI bonded, an attacker would need:

- 1,001 identities minimum
- 1,001,000 DOLI in bonds
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

## 17. Conclusion

We have proposed a system for electronic transactions that requires no trust in institutions, no massive energy expenditure, and no capital accumulation to participate in consensus.

We started with the usual framework of coins made from digital signatures, which provides strong control of ownership. This is incomplete without a way to prevent double-spending. To solve this, we proposed a peer-to-peer network using verifiable delay functions to anchor consensus to time.

**Nodes vote with their time.** The network cannot be accelerated by wealth or parallelized by hardware. One hour of sequential computation is one hour, whether performed by an individual or a nation-state.

**Rewards are deterministic, not probabilistic.** A participant knows exactly when their next block will be produced. Pools are unnecessary. The smallest participant receives the same percentage return as the largest.

The network is robust in its simplicity. Nodes work with little coordination. They do not need to be identified, since messages are not routed to any particular place and only need to be delivered on a best effort basis. Nodes can leave and rejoin the network at will, accepting the heaviest chain as proof of what happened while they were gone.

**The rules are fixed at genesis. The emission is predictable.**

Any needed rules and incentives can be enforced with this consensus mechanism.

---

**DOLI v1.0**

*"Time is the only fair currency."*

**E. Weil** · contact: weil@doli.network

---

## References

1. Nakamoto, S. (2008). *Bitcoin: A Peer-to-Peer Electronic Cash System.*

2. Boneh, D., Bonneau, J., Bünz, B., & Fisch, B. (2018). *Verifiable Delay Functions.* In Advances in Cryptology – CRYPTO 2018.

3. Wesolowski, B. (2019). *Efficient Verifiable Delay Functions.* In Advances in Cryptology – EUROCRYPT 2019.

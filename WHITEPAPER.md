# DOLI<sub>τ</sub>

## Deterministic Ordering through Linear Iterations

### A Peer-to-Peer Electronic Cash System Based on Verifiable Time

**E. Weil** · weil@doli.network

---

## Abstract

We propose a peer-to-peer electronic cash system where the only resource required for consensus is time — the one resource distributed equally to all participants.

Block production follows pure round-robin: every active producer receives equal block assignments regardless of stake. The protocol distributes rewards every epoch through a built-in pool — proportional to bonds, no external mining pools, no operators, no fees. Rewards compound into productive stake, creating predictable exponential growth for every participant regardless of size.

A new producer receiving 10 DOLI can reinvest block rewards to double their stake at regular intervals. The doubling rate is identical for all participants — one bond or three thousand. Continuous presence is proven through on-chain liveness attestations — producers who are online and following the chain qualify for their share. A dynamic liveness filter excludes offline producers from the rotation and re-includes them upon attestation. No lottery. No variance. No pools. Just time.

Transactions are ordered through sequential delay proofs — iterated hash computations that cannot be parallelized. No special hardware is required. Any CPU can participate in consensus. The result is a system where consensus weight emerges from time rather than trust, capital, or scale.

We demonstrate that NFTs, fungible tokens, and trustless cross-chain bridges can be implemented as native UTXO output types with declarative spending conditions, without a virtual machine, without gas metering, and without trusted committees — achieving equivalent expressiveness to VM-based approaches for these use cases while maintaining bounded, predictable verification cost.

---
## 1. Introduction

Every consensus mechanism ever designed shares one assumption: security requires a scarce resource that can be accumulated. Bitcoin chose energy. Ethereum chose capital. Both created systems where the largest participant has a structural advantage over the smallest.

This assumption is wrong. There exists a resource that cannot be accumulated, cannot be parallelized, and is distributed equally to every participant on Earth: **time**.

One second passes at the same rate for an individual running a single node as it does for a nation-state with unlimited budget. No amount of money can buy more time. No amount of hardware can make time pass faster.

We propose an electronic cash system where consensus derives from verifiable sequential computation — proof that real time has elapsed. The system is secure as long as honest participants collectively maintain more sequential computation presence than any cooperating group of attackers.

### 1.1. Why Now

The blockchain trilemma assumes three competing properties: decentralization, security, and scalability. Proposed solutions trade one for another — energy for security (PoW), decentralization for scalability (PoS), simplicity for throughput (sharding). These tradeoffs arise because every prior system anchors consensus to a resource that can be accumulated: hashpower, stake, or storage.

The formalization of Verifiable Delay Functions by Boneh, Bonneau, Bünz, and Fisch [2] in 2018 demonstrated that sequential computation could serve as a consensus primitive — proving that real time has elapsed without trusting the prover. This insight motivated our approach, though DOLI uses a simpler construction (Section 5.1).

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

DOLI solves this through public announcement of all transactions and deterministic time-ordering. Every transaction is broadcast to the network and included in a block at a specific slot. The sequential nature of slots — each anchored by a sequential delay proof — establishes an unambiguous ordering. The earliest transaction in the time sequence is the valid one. Later attempts to spend the same output are rejected by every honest node.

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

For basic transfers, `extra_data` is empty. For bonds, `extra_data` encodes the bond creation slot (4 bytes, little-endian), enabling per-bond FIFO vesting calculations directly from the UTXO set. New output types define how `extra_data` is interpreted, adding validation rules to the protocol while the structure remains fixed from genesis.

---

## 3. Programmable Outputs

The `extra_data` field in every output (Section 2.2) makes DOLI outputs programmable without a virtual machine, without gas, and without a Turing-complete scripting language.

### 3.1. Design Principles

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

### 3.2. Condition Language

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

### 3.3. Native Output Types

Each output type is a named pattern over the condition language:

| Type | Conditions | Use Case |
|------|-----------|----------|
| Transfer | `Signature(owner)` | Standard payment |
| Bond | `Signature(owner) AND Protocol(withdrawal)` | Producer stake |
| Multisig | `Multisig(n, keys)` | Shared custody |
| Hashlock | `Signature(owner) AND Hashlock(h)` | Atomic swaps |
| HTLC | `(Hashlock(h) AND Timelock(t)) OR TimelockExpiry(t+d)` | Payment channels |
| Escrow* | `Threshold(2, [buyer, seller, arbiter])` | Trustless commerce |
| Vesting | `Signature(owner) AND Timelock(unlock_height)` | Time-locked grants |
| UniqueAsset | `Condition + [token_id, content_hash]` | Non-fungible tokens |
| FungibleAsset | `Condition + [asset_id, supply, ticker]` | User-issued tokens |
| BridgeHTLC | `HTLC + [target_chain, target_address]` | Cross-chain bridges |

These are not separate implementations — they are compositions of the same primitive conditions. A developer does not write a smart contract. A developer selects conditions.

*Escrow is a composition pattern using Multisig or Threshold conditions, not a separate output type.

### 3.4. Non-Fungible Tokens (UniqueAsset)

A UniqueAsset output carries a globally unique token representing ownership of a singular digital object. The output's `extra_data` stores the spending condition followed by metadata:

```
extra_data = [condition_bytes][version][token_id][content_hash_len][content_hash]
```

**Token identity.** The `token_id` is deterministic: `BLAKE3("DOLI_NFT" || creator_pubkey_hash || nonce)`. Two mints with different nonces always produce different tokens. The content hash may be an IPFS CID, an HTTP URI, or a raw BLAKE3 digest — the protocol stores bytes without interpreting them.

**Spending conditions.** The condition field is the same composable language as any other output. The simplest case is `Signature(owner)` — only the current holder can transfer the NFT. But nothing prevents a Multisig custody, a Hashlock-gated reveal, or a Timelock-based auction where the NFT becomes spendable by anyone after a deadline.

**Transfer.** Transferring an NFT spends the old UTXO and creates a new UniqueAsset output with the same `token_id` and `content_hash` but a new owner and potentially new conditions. The token_id is the permanent identity; the UTXO is the current ownership record.

**No registry, no contract, no global state.** The NFT exists entirely within the UTXO that carries it. Indexing is a reader concern — the protocol validates structure and conditions, nothing more.

### 3.5. User-Issued Tokens (FungibleAsset)

A FungibleAsset output represents a fixed-supply user-issued token. The `extra_data` stores the spending condition followed by asset metadata:

```
extra_data = [condition_bytes][version][asset_id][total_supply][ticker_len][ticker]
```

**Asset identity.** The `asset_id` is derived from the genesis transaction: `BLAKE3("DOLI_ASSET" || genesis_tx_hash || output_index)`. This makes it unique by construction — no two issuances can produce the same asset_id because no two transactions share a hash.

**Fixed supply.** Total supply is set at issuance and encoded in every UTXO carrying the token. The protocol does not enforce supply invariants across UTXOs — that is an indexer responsibility. What the protocol enforces: the `extra_data` structure is valid, the condition is satisfiable, and the output follows standard UTXO rules.

**Ticker.** Up to 16 ASCII characters. `DOGEOLI`, `STBL`, `GOLD` — the ticker is metadata for human readability, stored on-chain and queryable through the RPC.

**What this enables:** meme coins, stablecoins, loyalty points, tokenized securities, game currencies — any scenario where a fixed-supply fungible token is needed. The token lives on the same chain as DOLI, validated by the same producers, at the same speed. No sidechain, no bridge, no wrapper.

### 3.6. Cross-Chain Bridges (BridgeHTLC)

A BridgeHTLC output is a standard HTLC with routing metadata for cross-chain atomic swaps. The `extra_data` stores the HTLC condition followed by bridge metadata:

```
extra_data = [condition_bytes][version][target_chain][addr_len][target_address]
```

The condition is always an HTLC: `(Hashlock(h) AND Timelock(t)) OR TimelockExpiry(t+d)`. The metadata tells counterparties which chain to lock on and where.

**Supported chains:**

| Chain | ID | Address Format | Hashlock Support |
|-------|-----|---------------|-----------------|
| Bitcoin | 1 | Base58/Bech32 | Native (OP_SHA256, OP_HASH160) |
| Ethereum | 2 | 0x-prefixed hex | 30-line Solidity contract |
| Monero | 3 | Standard/Integrated | Native (Ed25519 adaptor sigs) |
| Litecoin | 4 | Base58/Bech32 | Native (same as Bitcoin) |
| Cardano | 5 | Bech32 | Plutus script |

**Atomic swap protocol:**

```
1. Alice (DOLI) generates secret S, computes H = BLAKE3(S)
2. Alice locks X DOLI in BridgeHTLC(H, lock=L, expiry=E, chain=Bitcoin, to=Bob_BTC)
3. Bob sees the lock on DOLI chain, verifies H
4. Bob locks Y BTC in Bitcoin HTLC with same hash H, shorter expiry
5. Alice claims Bob's BTC by revealing S on Bitcoin
6. Bob reads S from Bitcoin, claims Alice's DOLI by revealing S on DOLI
7. If Bob never locks → Alice refunds after E
8. If Alice never claims → Bob refunds after his Bitcoin expiry
```

Both sides are protected. Neither can lose funds. The preimage revelation on one chain enables the claim on the other. This is the same mechanism that secures the Lightning Network — applied across chains.

**What this is not.** This is not a bridge with validators, multisigs, or custodians. There is no bridge committee. There is no wrapped token. There is no TVL to exploit. Each swap is an independent UTXO with a hash lock. The only trust assumption is that both chains will include transactions before their respective expiries — the same assumption underlying every blockchain.

**What this eliminates.** Every major bridge hack — Ronin ($624M), Wormhole ($326M), Nomad ($190M), Harmony ($100M) — exploited the same pattern: a small committee guarding a large pool. DOLI has no pool. Each swap is point-to-point, funded by the participants, secured by mathematics. There is nothing to hack because there is nothing to custody.

### 3.7. Witness Separation (SegWit-Style)

Spending a conditioned output requires a witness — the data that satisfies the conditions. A Hashlock requires the preimage. A Signature condition requires a signature from the matching key. A Multisig requires N signatures.

Witnesses are stored in the transaction's `extra_data` field, separate from the signing hash. The signing message covers inputs and outputs but excludes witness data — the same separation Bitcoin SegWit introduced to solve transaction malleability.

```
signing_hash = BLAKE3(version || tx_type || inputs || outputs)
    ↑ excludes extra_data (witnesses)

tx_hash = BLAKE3(version || tx_type || inputs || outputs || extra_data)
    ↑ includes extra_data (immutable commitment)
```

This prevents a chicken-and-egg: a Signature witness must sign a hash that does not include the witness itself. The witness is committed in the full `tx_hash` for immutability but excluded from `signing_hash` for constructability.

### 3.8. What This Enables

**Without a virtual machine:**

- **Decentralized exchanges:** Atomic swaps between DOLI and any chain that supports hash locks. No intermediary, no custody, no counterparty risk.
- **Payment channels:** Off-chain transactions with on-chain settlement. HTLCs enable a Lightning-equivalent network natively.
- **Multi-party custody:** Corporate treasuries, DAOs, inheritance — any scenario requiring N-of-M authorization.
- **Trustless escrow:** Buyer, seller, and arbiter each hold a key. Any two can release funds.
- **Vesting schedules:** Time-locked outputs for team allocations, grants, or contractual obligations.
- **Native NFTs:** Digital art, identity tokens, certificates — unique assets with composable spending conditions, no contract deployment.
- **User-issued tokens:** Meme coins, stablecoins, loyalty points — fixed-supply tokens on the base layer, no sidechain required.
- **Cross-chain bridges:** Trustless atomic swaps with Bitcoin, Ethereum, Monero, Litecoin, and Cardano. No bridge committee, no wrapped tokens, no custodial risk.

**Without shared mutable state:**

Every output is independent. Spending one output cannot affect another. There is no reentrancy, no front-running, no MEV. Transactions are fully parallelizable — validation scales linearly with cores.

### 3.9. What This Does Not Enable

DOLI outputs cannot maintain persistent state across transactions. There is no on-chain storage, no loops, no arbitrary computation. This is deliberate.

Applications requiring shared state — automated market makers, lending protocols, on-chain governance with complex voting — belong on Layer 2 or application-specific chains that settle to DOLI.

The base layer provides: **value transfer, time-anchored ordering, programmable spending conditions, native assets, and trustless cross-chain settlement.** Everything else builds on top.

---

## 4. Timestamp Server

The solution begins with a distributed timestamp server. The network acts as a timestamp server by taking a hash of a block of items to be timestamped and widely publishing the hash. The timestamp proves that the data must have existed at the time in order to enter the hash.

```
                     ┌──────────────────┐
                     │      Block       │
                     ├──────────────────┤
                     │  Previous hash   │
                     │  Timestamp       │
 Transactions ───▶   │  Transactions    │
                     │  Delay Proof     │
                     └──────────────────┘
                              │
                              ▼
                     ┌──────────────────┐
                     │      Block       │
                     ├──────────────────┤
                     │  Previous hash   │
                     │  Timestamp       │
                     │  Transactions    │
                     │  Delay Proof     │
                     └──────────────────┘
```

Each timestamp includes the previous timestamp in its hash, forming a chain. Each additional timestamp reinforces the ones before it.

---

## 5. Proof of Time

To implement a distributed timestamp server on a peer-to-peer basis, we need a mechanism that makes producing blocks costly and prevents that cost from being evaded through parallelization or resource accumulation.

The solution is to use **sequential delay proofs** — functions that enforce a minimum wall-clock time per block through inherently serial computation. The construction is inspired by Verifiable Delay Functions [2, 3] but uses a simpler primitive (Section 5.1). The essential properties are:

1. Requires a fixed number of sequential operations to compute.
2. Cannot be significantly accelerated through parallelization.
3. Can be verified by any node (by recomputation).

> **Note:** The delay proof demonstrates that *N* sequential operations were executed — time is the effective lower bound since no known technique accelerates sequential hash computation through parallelization. The proof serves as a heartbeat (proof of presence), not as a randomness source. Producer selection is a pure function of `(slot, ActiveSet(epoch))`, fixed at epoch start, independent of proof speed. Faster hardware provides no scheduling advantage.

For each block, the producer must calculate:

```
input  = HASH(prefix || previous_hash || tx_root || slot || producer_key)
output = HASH^n(input)
```

Where *n* is the difficulty parameter that determines how long the computation takes.

### 5.1. Delay Proof Construction

DOLI uses an **iterated hash chain** (BLAKE3), not an algebraic VDF over groups of unknown order (Wesolowski [3], Pietrzak). The distinction matters:

| Property | Algebraic VDF (Wesolowski) | Iterated Hash Chain (DOLI) |
|----------|---------------------------|---------------------------|
| Verification | *O(log T)* — near-constant | *O(T)* — must recompute |
| Trusted setup | Required (RSA group) | None |
| Quantum resistance | Uncertain | Hash-based (conservative) |
| Implementation | Complex (GMP/big integers) | Simple (~10 lines) |

Algebraic VDFs offer *O(log T)* verification, which is critical when the delay parameter *T* is large (minutes to hours). DOLI's block delay proof requires only *T* = 1,000 iterations (negligible computation time), making *O(T)* verification trivially fast — every node recomputes the chain in microseconds.

The tradeoff is deliberate: DOLI gains simplicity, auditability, and no trusted setup at the cost of linear verification. For a lightweight anti-grinding barrier where *T* is small, this is the correct engineering choice. The bond requirement (10 DOLI per producer) is the primary Sybil defense; the delay proof serves as a protocol-level heartbeat and anti-flash barrier, not a time-intensive proof of work.

```
Input: prev_hash ∥ slot ∥ producer_key
         │
         ▼
    ┌─────────┐
    │ BLAKE3  │ ◄──┐
    └────┬────┘    │
         │         │
         └─────────┘  × T iterations (T = 1,000)
         │
         ▼
      Output: h_T = H^T(input)
```

**Verification:** A verifier recomputes *h_T = H^T(input)* and checks *h_T == claimed_output*. The sequential dependency *h_{i+1} = H(h_i)* prevents parallelization. No shortcut for computing *H^T* faster than *T* sequential evaluations is known for BLAKE3 or any cryptographic hash function — this is a standard assumption in hash-based cryptography, not a proven lower bound. The security of the delay proof rests on this assumption, which we share with all iterated hash constructions including Solana's Proof of History [4].

**Parallel verification of multiple proofs:** When a block contains multiple transactions requiring VDF verification (e.g., several producer registrations), the node verifies each proof in a separate thread using `thread::scope`. Each individual proof remains sequential, but independent proofs are verified concurrently. This ensures that blocks with many registrations do not create a verification bottleneck.

### 5.2. Time Structure

The network defines time as follows:

```
GENESIS_TIME = 2026-03-19T22:31:20Z (UTC)
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

### 5.3. Iteration Parameters

Each network defines a fixed iteration count:

```
T_BLOCK = 1,000 iterations (negligible computation time)
```

With 10-second slots, the delay proof completes in microseconds, leaving the remainder for block construction and propagation. The fixed iteration count ensures all nodes compute identical proofs — no per-node calibration or dynamic adjustment is needed.

The delay proof is deliberately lightweight. The bond requirement (Section 7.2) is the primary cost of participation; the VDF serves as an anti-grinding measure that prevents a producer from trivially precomputing blocks without knowledge of the previous block hash. Every consensus system enforces a scarce resource. In DOLI, that resource is bonded capital anchored by sequential time.

---

## 6. Network

The steps to run the network are as follows:

1. New transactions are broadcast to all nodes.
2. Each eligible producer collects new transactions into a block.
3. The producer assigned to the slot computes the delay proof.
4. The producer broadcasts the block to the network.
5. Nodes accept the block only if all transactions in it are valid and the delay proof is correct.
6. Nodes express their acceptance of the block by working on creating the next block, using the hash of the accepted block as the previous hash.

Nodes always consider the chain covering the most time to be the correct one and will keep working on extending it. If two nodes broadcast different versions of the next block simultaneously, some nodes may receive one or the other first. In that case, they work on the first one they received but save the other branch in case it becomes longer. The tie will be broken when the next block is produced and one branch covers more slots; the nodes that were working on the other branch will then switch to the longer one.

### 6.1. Block Validity

A block *B* is valid if:

1. `B.timestamp > prev_block.timestamp`
2. `B.timestamp <= network_time + DRIFT`
3. `B.slot` derives correctly from `B.timestamp`
4. `B.slot > prev_block.slot`
5. `B.producer` is the correct round-robin selection for `B.slot` given the active set and liveness filter
6. `verify_hash_chain(preimage, B.delay_output, T) == true`
7. All transactions in the block are valid

### 6.2. Clock Synchronization

Consensus depends on nodes having reasonably synchronized clocks. Nodes synchronize through:

- NTP servers
- Median offset from connected peers

```
network_time = local_clock + median(peer_offsets)
```

Blocks with timestamps outside the acceptable window are rejected.

### 6.3. Throughput

With 10-second block times and a base block size of 2 MB (doubling each era, capped at 32 MB):

| Metric              | Era 1          | Era 2          | Era 4 (cap)    |
|---------------------|----------------|----------------|----------------|
| Block time          | 10 seconds     | 10 seconds     | 10 seconds     |
| Max block size      | 2 MB           | 4 MB           | 32 MB          |
| Avg transaction     | ~250 bytes     | ~250 bytes     | ~250 bytes     |
| Theoretical max TPS | ~800           | ~1,600         | ~12,800        |
| Practical TPS       | 200-400        | 400-800        | 3,000-6,000    |

DOLI does not compete on raw throughput. It competes on accessibility:

| System       | TPS       | Min hardware to participate |
|--------------|-----------|----------------------------|
| Bitcoin      | ~7        | ASIC ($5,000+)             |
| Ethereum PoS | ~30       | 32 ETH + server ($100K+)   |
| Solana       | ~4,000    | 256GB RAM server ($10K+)   |
| DOLI         | ~400      | Any CPU ($5/mo VPS)        |

400 TPS on a $5/month VPS is a different proposition than 4,000 TPS on hardware most people cannot afford. The throughput is sufficient for a cash system; the accessibility is sufficient for global participation. The era-doubling schedule ensures capacity grows with network maturity.

---

## 7. Producer Registration

In an open network, anyone can create identities at no cost. Allowing unlimited and free identity creation would expose the network to Sybil attacks where an attacker floods the system with fake nodes.

To prevent this, registration requires both a sequential delay proof and an activation bond (Section 7.2). The bond is the primary Sybil deterrent; the delay proof adds a lightweight anti-grinding barrier.

```
input  = HASH(prefix || public_key || epoch)
output = HASH^T(input)    where T = T_REGISTER_BASE = 1,000 iterations
```

A registration is valid if:

1. The delay proof verifies correctly with `T_REGISTER_BASE` iterations.
2. The epoch is current or previous.
3. The public key is not already registered.
4. The activation bond is included.

### 7.1. Registration Difficulty

The registration difficulty is fixed:

```
T_registration = T_REGISTER_BASE = 1,000 iterations (negligible computation time)
```

This is deliberately lightweight. The capital cost of the activation bond (10 DOLI) is the primary Sybil deterrent. The delay proof serves as an anti-grinding measure — it binds the registration to a specific epoch and public key, preventing precomputation of registration proofs. An attacker with *M* machines can register *M* identities, but each requires *BOND_UNIT* capital, making the cost of a Sybil attack *O(M)* in bonded capital.

### 7.2. Activation Bond

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

### 7.3. Bond Stacking

Producers can increase their stake up to 3,000 times the base bond unit.

```
BOND_UNIT = 10 DOLI
MIN_STAKE = 1 × BOND_UNIT (10 DOLI)
MAX_STAKE = 3,000 × BOND_UNIT (30,000 DOLI)
```

Block production uses pure round-robin — every active producer receives equal block assignments regardless of bond count. A producer with 1 bond produces the same number of blocks as a producer with 250 bonds. Bonds affect only epoch reward distribution (Section 10.2), not production frequency.

**Example (3 producers):**

```
Alice: 1 bond unit  (10 DOLI)   → 1 block every 3 slots
Bob:   5 bond units (50 DOLI)   → 1 block every 3 slots
Carol: 4 bond units (40 DOLI)   → 1 block every 3 slots
```

All three produce at equal frequency. At epoch boundary, the reward pool distributes proportionally to bonds:

- Alice earns 10% of epoch rewards (1/10 bonds)
- Bob earns 50% of epoch rewards (5/10 bonds)
- Carol earns 40% of epoch rewards (4/10 bonds)

**All producers earn identical ROI percentage regardless of stake size** — because reward share scales linearly with bonds, and each DOLI bonded generates the same return.

| Parameter             | Value                    |
|-----------------------|--------------------------|
| Bond unit             | 10 DOLI                  |
| Min stake             | 10 DOLI (1 bond)         |
| Max stake             | 30,000 DOLI (3,000 bonds) |
| Block reward (Era 1)  | 1 DOLI                   |
| Production frequency  | Equal per producer (round-robin) |
| Reward distribution   | Proportional to bonds (epoch pool) |

#### Accessibility at Scale

At network maturity (500 producers, 18,000 total bonds):

| Your Stake | Bonds | Blocks/Week | Income/Week | Hardware |
|-----------|-------|-------------|-------------|----------|
| 10 DOLI   | 1     | ~121        | ~3 DOLI     | Any CPU  |
| 100 DOLI  | 10    | ~121        | ~34 DOLI    | Any CPU  |
| 1,000 DOLI| 100   | ~121        | ~336 DOLI   | Any CPU  |

Every producer receives the same number of block assignments. Income differs only through bond-weighted epoch rewards. No mining rigs. No staking pools. No minimum hardware requirements. A $5/month VPS is sufficient.

### 7.4. Bond Lifecycle

The bond has a 4-year commitment period with per-bond FIFO tracking:

```
T_commitment = 12,614,400 blocks (~4 years)
```

Each bond tracks its own creation time. Withdrawal uses FIFO order (oldest bonds first), with penalty calculated individually per bond based on its age.

**Withdrawal uses a two-step process with a 7-day unbonding period** (60,480 blocks). A producer submits a `RequestWithdrawal` transaction, which begins the unbonding countdown. After 60,480 blocks (~7 days), the producer submits a `ClaimWithdrawal` transaction to receive the funds. Bond removal from the active set takes effect at the next epoch boundary after the request.

The unbonding period prevents short-range attacks where a producer withdraws immediately after misbehaving, and ensures the network retains slashing capability during the dispute window.

Early withdrawal incurs a tiered FIFO vesting penalty based on individual bond age:

| Bond Age  | Penalty | Returned |
|-----------|---------|----------|
| < 1 year  | 75%     | 25%      |
| 1-2 years | 50%     | 50%      |
| 2-3 years | 25%     | 75%      |
| 3+ years  | 0%      | 100%     |

A producer with bonds of mixed ages can withdraw selectively. The oldest bonds (lowest penalty) are withdrawn first. This rewards long-term commitment while allowing flexible exit.

All penalties are burned permanently, removing coins from circulation.

---

## 8. Producer Selection

For each slot, a deterministic function selects the block producer. Let *P* = {*p₁*, ..., *p_n*} be the active set sorted by public key, and *L* ⊂ *P* be the set of producers currently excluded by the liveness filter (Section 8.1). Define the effective set *E* = *P* \ *L*, sorted lexicographically by public key bytes.

```
producer(s) = E[s mod |E|]
```

The function is pure round-robin: `producer(s) = f(s, ActiveSet(epoch(s)), LivenessFilter(s))`. It depends on no value the current producer can influence — not `prev_hash`, not transaction ordering, not timestamps within the drift window, not bond count. **Grinding is impossible because the schedule is a function of time alone.**

Bond count does not influence production frequency. A producer with 1 bond and a producer with 250 bonds produce at equal frequency. Bonds affect only epoch reward distribution (Section 10.2). This separation ensures that block production capacity cannot be purchased — only presence and time determine who produces.

### 8.1. Liveness Filter

The protocol maintains a dynamic liveness filter that temporarily excludes producers who are demonstrably offline, preventing wasted slots in the round-robin rotation.

**Exclusion.** When a block is committed, the protocol checks whether the slot gap between consecutive blocks exceeds 1. If a producer was scheduled for a skipped slot and did not produce, they are added to the excluded set.

**Re-inclusion.** The presence bitfield committed in each block header (`presence_root`, Section 10.3) records which producers attested during that block's attestation window. A producer in the excluded set who appears in a block's presence bitfield is immediately re-included in the effective set.

**Reconstruction.** The excluded set is not persisted — it is reconstructed at node startup by scanning the last epoch of blocks (360 blocks) from the block store. This ensures that nodes joining via any sync method compute an identical excluded set from the same chain data.

| Event | Effect |
|-------|--------|
| Producer misses assigned slot | Added to excluded set |
| Producer appears in presence bitfield | Removed from excluded set |
| Node startup | Excluded set rebuilt from last 360 blocks |

The liveness filter is deterministic: every node reading the same chain computes the same excluded set and therefore the same producer for each slot. No randomness, no voting, no subjectivity.

### 8.2. Comparison with Existing Systems

Pools exist in PoW and PoS because rewards are probabilistic — variance forces small participants to delegate control to centralized operators. DOLI's pure round-robin eliminates production variance entirely — every producer gets equal block assignments. The built-in epoch reward pool (Section 10.5) distributes rewards bond-weighted to all qualified producers. External pools cannot offer a better deal.

| System       | Selection                 | Variance | Pools | Energy        | Min Hardware    |
|--------------|---------------------------|----------|-------|---------------|-----------------|
| Bitcoin      | Lottery (hashpower)       | High     | Yes   | ~150 TWh/yr   | ASIC ($5,000+)  |
| Ethereum PoS | Lottery (stake)           | Medium   | Yes   | ~2.6 GWh/yr   | 32 ETH ($100K+)  |
| Solana PoH   | Leader schedule (stake)   | Low      | Yes   | ~4 GWh/yr     | $10,000+ server  |
| DOLI PoT     | Deterministic round-robin | **Zero** | **Built-in**| **Negligible**| **Any CPU ($5/mo)**|

Solana uses Proof of History as a clock, but leader selection remains stake-weighted with probabilistic elements and requires high-performance hardware. DOLI uses the delay proof purely as a heartbeat — leader selection is a pure round-robin function of `(slot, ActiveSet(epoch), LivenessFilter)`. No hardware advantage exists. No stake advantage exists for production — only for rewards.

**Tiered scaling path.** The protocol defines a two-tier architecture for future growth: up to 500 Tier 1 validators (block producers with full consensus participation) and up to 15,000 Tier 2 attestors (liveness attestation without block production). Delegation enables Tier 3 participants to stake without running infrastructure, with rewards split 10% to the delegate (Tier 1/2 node operator) and 90% to the staker. This tiered model preserves the accessibility of the base protocol while scaling consensus participation beyond the active producer set.

---

## 9. Chain Selection

When multiple valid chains exist, nodes must agree which to follow.

### 9.1. Weight-based Fork Choice

The canonical chain is the one with the highest accumulated producer weight:

```
accumulated_weight(block) = accumulated_weight(parent) + producer_weight
```

Producer weight derives from seniority:

| Years active | Weight |
|--------------|--------|
| 0            | 1.00   |
| 1            | 1.75   |
| 2            | 2.50   |
| 3            | 3.25   |
| 4+           | 4.00   |

Weight follows a continuous formula: `weight = 1.0 + min(years, 4) × 0.75`.

This prevents attacks where an attacker creates many new-producer blocks to outpace a chain built by established producers.

---

## 10. Incentive

Rewards are not distributed per block. Instead, the protocol accumulates block rewards into an **epoch pool** and distributes them once per epoch (every 360 blocks, ~1 hour) to all producers who proved continuous presence during the epoch. The protocol acts as a **built-in pool** — no external mining pools, no operators, no fees, no trust.

### 10.1. Emission

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

### 10.2. Epoch Reward Distribution

At the first block of each new epoch, the block producer emits a single reward transaction distributing the accumulated pool to all qualified producers, proportional to their bond count:

```
epoch_pool = Σ block_reward(h) for h in [epoch_start, epoch_end)
reward(i)  = epoch_pool × bonds(i) / Σ qualifying_bonds
```

Only producers who attested in ≥90% of the epoch's 1-minute attestation windows qualify. Non-qualifiers receive nothing; their share is redistributed to qualified producers.

This produces one UTXO per producer per epoch instead of one per block — eliminating reward dust while maintaining identical total emission.

### 10.3. Liveness Attestation

Block production proves a producer was online during their assigned slots. But with deterministic scheduling, a producer knows exactly which slots are theirs and can be offline the rest of the time.

To prove **continuous** presence, each producer signs a liveness attestation every minute using both Ed25519 and BLS12-381 keys:

```
attestation = Sign(block_hash || slot)
```

The block hash proves the producer is not just alive but actively following and validating the chain. Attestations are gossiped to the network. Each block producer commits:

1. A **bitfield** in the block header (`presence_root`) — one bit per producer (attested or not), supporting up to 256 producers (32 bytes)
2. An **aggregate BLS signature** in the block body — cryptographic proof that the bitfield is honest

The aggregate BLS signature compresses all individual attestation signatures into a single verification. A fake bit — claiming a producer attested when they didn't — causes aggregate signature verification to fail. The block is rejected.

At epoch boundary, every node scans the bitfields committed in the epoch's blocks and counts per-producer attestation minutes. Each epoch spans 60 attestation minutes (one per 6 slots). Two distinct thresholds apply:

1. **Epoch reward qualification (90%):** A producer must attest in at least 54 of 60 minutes to qualify for that epoch's reward distribution. Non-qualifiers receive nothing; their share is redistributed to qualified producers.
2. **Active set retention (50%):** A producer must maintain a presence rate of at least 50% to remain in the active producer set. Producers falling below this threshold are removed from the set and must re-register to rejoin.

Both thresholds are deterministic: every node reads the same chain, computes the same counts, agrees on the same qualification and retention decisions.

### 10.4. Dual-Key Design

Producers hold two key pairs:

| Key | Curve | Purpose |
|-----|-------|---------|
| Ed25519 | Curve25519 | Transactions, block signing |
| BLS | BLS12-381 | Liveness attestation aggregation |

Ed25519 is faster for single-signature operations (block signing, transaction signing). BLS is used solely for attestation because it is the only scheme that supports signature aggregation — compressing N signatures into one for on-chain efficiency.

Both keys are committed on-chain at producer registration. The BLS public key is stored in the registration transaction.

### 10.5. Built-in Pool

Traditional mining pools exist because rewards in PoW and most PoS systems are probabilistic — small participants experience high variance and must delegate to centralized operators for income smoothing.

DOLI eliminates the need for external pools entirely. The protocol itself is the pool:

| Property | External mining pool | DOLI epoch rewards |
|----------|---------------------|--------------------|
| Operator | Third party (takes fee) | Protocol (no fee) |
| Trust | Trust the operator | Trustless (on-chain math) |
| Distribution | Pool decides splits | Deterministic: bonds × qualification |
| Centralization | Concentrates power | Each producer runs own node |
| Reward variance | Smoothed by pool | Zero — deterministic by design |

Every producer participates automatically. Rewards are distributed proportionally by bond weight to all qualified producers. No intermediary can offer a better deal than the protocol itself.

### 10.6. Fees

```
fee = sum(inputs) - sum(outputs)
```

The fee goes to the block producer. A minimum fee rate prevents spam.

### 10.7. Reward Maturity

Outputs from epoch reward transactions require 6 confirmations (~1 minute) before they can be spent.

### 10.8. Compound Growth

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

## 11. Infractions

### 11.1. Invalid Blocks

The network rejects blocks that:

- Have invalid delay proofs
- Are produced by wrong producer for that slot
- Have timestamps outside the valid window
- Contain invalid transactions

The producer loses the slot opportunity. The bond remains intact.

### 11.2. Inactivity

If a producer fails to produce when selected for 50 consecutive slots:

- Removed from the active set
- Bond remains locked (no penalty for inactivity)
- Can reactivate with a new registration delay proof

Inactivity is not punished — it is tolerated. A producer who goes offline loses income (missed block rewards) but not capital (bond remains intact). This is a deliberate design choice: penalizing downtime would discourage small operators with less reliable infrastructure.

### 11.3. Double Production

If a producer creates two different blocks for the same slot, anyone can construct a proof of this infraction.

**Penalty:**

- 100% of bond burned permanently
- Immediate exclusion from producer set
- To reactivate: new registration with `T_registration × 2`

This is the only infraction that results in slashing because it is the only one that is unambiguously intentional.

---

## 12. Security

If the attacker controls less sequential computation capacity than the honest network, the probability of catching up to the honest chain decreases rapidly with the number of slots difference.

In this system, an attacker cannot "accelerate" the production of an alternative chain by adding parallel hardware, because each block requires a sequential computation of fixed duration.

### 12.1. Attack Cost

To dominate the network, an attacker would need to:

1. Register more producers than honest ones (time cost per identity)
2. Lock more bonds than honest participants (economic cost)
3. Maintain those producers active (operational cost)
4. Risk total bond loss if detected double-producing

The protocol automatically regulates the rate at which new identities can join.

### 12.2. Attack Probability

**Assumption (Sequential Hardness).** For a cryptographic hash function *H*, computing *H^T(x)* requires at least *T* sequential evaluations of *H*. No algorithm can produce *H^T(x)* in fewer than *T* steps, regardless of parallel resources. This is a standard assumption in hash-based cryptography — no counterexample exists for any hash function considered secure, but no formal proof of this lower bound exists either.

**Theorem (Sequential Deficit).** Under the Sequential Hardness assumption, let *T* be the fixed sequential time per block. An attacker who begins an alternative chain with deficit *d* ≥ 1 blocks cannot reduce *d* regardless of parallel computational resources.

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

The only attack vector is controlling >50% of producers in the active set, which requires:

1. *BOND_UNIT* capital locked per identity (linear cost, subject to slashing)
2. *T_registration* delay proof per identity (anti-grinding, epoch-bound)
3. 100% bond loss risk if detected double-producing

### 12.3. The Capital Accumulation Objection

A natural objection: "If registration requires only 1,000 hash iterations (negligible time), what prevents an attacker from flooding the network with identities?"

The answer is **bonded capital**. Each identity requires *BOND_UNIT* (10 DOLI) locked as an activation bond. An attacker registering *M* identities must lock *M × BOND_UNIT* capital — the cost is *O(M)*, identical to Proof of Stake. DOLI does not escape the fundamental economics of Sybil resistance: preventing identity flooding requires a scarce resource that scales linearly with the number of identities.

What DOLI adds beyond pure PoS:

1. **Capital at risk:** *BOND_UNIT* locked per identity, with 100% slashing for double production and vesting penalties for early withdrawal.
2. **Anti-grinding:** The registration VDF (1,000 iterations) binds the proof to a specific epoch and public key, preventing precomputation of registration proofs across future epochs.
3. **Ongoing operational cost:** Each identity requires continuous liveness attestation (90% uptime per epoch). Maintaining *M* identities at this threshold has compounding operational cost — one machine per identity, indefinitely.
4. **Seniority disadvantage:** New identities receive weight 1.0; established producers accumulate up to 4.0. An attacker starting from zero needs ~3 years before seniority weight matches honest incumbents.

Compare with PoW: an attacker with *M* ASICs gains *M×* hashpower immediately, with no per-identity capital lockup. In DOLI, each identity requires locked capital that can be destroyed upon misbehavior.

The system does not claim immunity from wealthy adversaries — no system can. It claims that (1) identity creation has linear capital cost with slashing risk, (2) the anti-grinding VDF prevents precomputation attacks, and (3) once registered, an attacker's per-identity operational cost is permanent, not a one-time expense.

### 12.4. Safety Theorem

**Theorem.** Let *n* = total producers in the active set. An attacker controlling *f* < *n/2* producers cannot produce a heavier chain than the honest network over any interval of *k* ≥ 1 epochs.

**Proof.** Define per epoch *e*:

- *S_h(e)* = set of slots assigned to honest producers (round-robin)
- *S_a(e)* = set of slots assigned to attacker producers (round-robin)
- *w(p)* = seniority weight of producer *p* ∈ [1.0, 4.0]

The schedule is a pure function of `(slot, ActiveSet(epoch), LivenessFilter)` — no block content influences it. Under round-robin, each producer receives equal slot assignments: *|S_h(e)| + |S_a(e)| = total slots*, distributed uniformly.

The accumulated chain weight over *k* epochs:

```
W_h(k) = Σ_{e=1}^{k} Σ_{s ∈ S_h(e)} w(producer(s))
W_a(k) = Σ_{e=1}^{k} Σ_{s ∈ S_a(e)} w(producer(s))
```

Since *f < n/2*, we have *|S_a(e)| < |S_h(e)|* for all *e* (honest producers outnumber attackers, so they receive more round-robin assignments). Additionally, seniority weighting (Section 9.1) penalizes new identities: *w(new) = 1* while *w(established) ≤ 4*. Therefore *W_h(k) > W_a(k)* for all *k* ≥ 1.

By the Sequential Deficit theorem (12.2), the attacker cannot compensate by computing faster — the hash chain's sequential dependency prevents parallel acceleration. ∎

**Corollary.** An attacker starting from zero needs ~3 years of sustained presence before seniority weight equals an established honest producer, even with equal bond count. The attack window is therefore bounded not just by capital but by calendar time.

**Limitation.** Seniority protects against *late* attackers — those who attempt to join and dominate an established network. It does not protect against *patient early* attackers who register during the network's infancy and accumulate seniority legitimately alongside honest producers. This is mitigated by: (1) the capital cost still scales linearly with bond count, (2) 100% bond loss risk for double production, and (3) the attestation requirement — maintaining *M* identities at 90% uptime for years has compounding operational cost. No consensus system can distinguish a patient adversary from a legitimate participant; the defense is making sustained dishonesty expensive, not impossible.

---

## 13. Reclaiming Disk Space

Once the latest transaction in a coin is buried under enough blocks, the spent transactions before it can be discarded to save disk space. Transactions are hashed in a Merkle tree, with only the root included in the block hash.

A block header with no transactions is approximately 340 bytes. With blocks every 10 seconds, that is ~1 GB per year for headers alone.

---

## 14. Simplified Payment Verification

It is possible to verify payments without running a full node. A user only needs to keep a copy of the block headers of the longest chain and obtain the Merkle branch linking the transaction to the block it is timestamped in.

---

## 15. Privacy

DOLI adopts the pseudonymous privacy model described by Nakamoto [1]: transactions are public, but identities behind keys are not. Users can generate multiple addresses to reduce linkage. This provides privacy equivalent to public stock exchange disclosures — amounts and flows are visible, participants are not.

---

## 16. Distribution

There is no premine, ICO, treasury, or special allocations. Every coin in circulation comes from block rewards.

### 16.1. Genesis Block

The genesis block contains a single coinbase transaction with the message:

> *"Time is the only fair currency."*

This serves as a statement of the system's philosophy. The genesis timestamp is embedded in the block, proving no blocks were mined before that moment.

The genesis block contains exactly:

- One coinbase transaction with 1 DOLI (standard reward)
- Zero additional transactions

**No hidden allocations exist.**

### 16.2. Network Bootstrap

A Proof-of-Time chain faces a circular dependency at launch: producers need bonds to produce blocks, but bonds require DOLI, and DOLI only exists through block production.

The protocol resolves this in three phases within a single epoch (360 blocks, ~1 hour):

**Phase 1 — Work without reward.** Five genesis producers receive a temporary scheduling placeholder (a zero-hash bond entry) that allows the scheduler to assign slots. This placeholder carries no value — it exists only so the round-robin algorithm has an input. During this first epoch, every block reward goes directly to the reward pool. The genesis producers receive nothing.

**Phase 2 — Automatic conversion.** At block 361 (first block after epoch 0), the protocol executes `consume_genesis_bond_utxos`: it collects all accumulated pool UTXOs, creates one real bond (10 DOLI) per genesis producer funded entirely from the pool, and returns the remainder to the pool for epoch 1 distribution. The temporary placeholders are replaced by real bond UTXOs backed by work already performed.

**Phase 3 — Equal rules.** From block 361 onward, genesis producers operate under the same rules as any future participant. Their bonds vest on the same schedule, earn the same rewards, and face the same withdrawal penalties.

The result: the founding producers paid for their own bonds with real block production. No coins were created outside the standard emission schedule. No advantage was granted that any future producer does not also receive through the same mechanism.

**The founders received no privilege — they paid the bootstrap cost with work.**

**Phase 4 — Open participation.** At block 26,979 (~3 days after genesis), the founding producers funded a public faucet from their own earned rewards — 250 DOLI each, 1,500 DOLI total. The faucet distributes 10.01 DOLI (1 bond unit + transaction fees) to any new participant who requests it. The barrier to entry is zero capital — only a $5/month VPS and the willingness to run a node. All faucet transactions are on-chain and verifiable.

---

## 17. Immutability

Transactions are final. There are no mechanisms to reverse transactions, recover funds, or modify history.

| Situation            | Protocol response   |
|----------------------|---------------------|
| Lost private keys    | Funds lost permanently |
| Erroneous transaction| Not reversible      |
| Exchange hack        | Not reversible      |
| Court order          | Not enforceable     |

**The code is law. Transactions are final.**

---

## 18. Protocol Updates

Software requires maintenance. Bugs must be fixed. The question is: who decides?

In centralized systems, the operator decides. In Bitcoin, informal consensus among developers, miners, and users determines which changes are adopted. This works but is slow and contentious.

DOLI formalizes the process. Updates are signed by maintainers and reviewed by producers.

### 18.1. Release Signing

Every release requires signatures from 3 of 5 maintainers. A single compromised key cannot push malicious code.

### 18.2. Veto Period

When a new version is published, producers have 7 days to review. Any producer can vote to reject. If 40% or more vote against, the update is rejected.

| Veto votes | Result   |
|------------|----------|
| < 40%      | Approved |
| ≥ 40%      | Rejected |

The threshold is weighted by stake and seniority (bonds × seniority multiplier). An attacker cannot create many new nodes to force an update through.

### 18.3. Adoption

After approval, producers have 1 hour to update. Nodes running outdated versions cannot produce blocks. This is not punishment — it is protection. A vulnerability in old code affects the entire network.

The choice is simple: participate in consensus with current software, or do not participate.

---

## 19. Live Network

DOLI is not a proposal. The network described in this paper is operational.

As of March 2026, the mainnet is in its **bootstrap phase** — operational and producing blocks, but with a small producer set operated primarily by the founding team across geographically distributed servers. The source code is open, the chain state is publicly verifiable, and external producers have begun joining. The chain has undergone multiple genesis resets during bootstrap; metrics below reflect the current chain (genesis: 2026-03-19).

| Metric | Value |
|--------|-------|
| Block time | 10 seconds |
| Delay proof computation | Negligible (~1,000 iterations) |
| Block propagation | < 500ms |
| Node hardware | Standard VPS, any CPU |
| Minimum bond | 10 DOLI |
| Liveness filter | Dynamic exclusion/re-inclusion |
| Unbonding period | 7 days (60,480 blocks) |

The current producer count reflects the bootstrap phase described in Section 16.2. The protocol's security properties strengthen as independent producers join — each additional operator increases the cost of a >50% producer-count attack and reduces reliance on the founding set. The target is a producer set large enough that no single entity controls a meaningful fraction of the active producer set.

```
Genesis:    2026-03-19 (current chain)
Consensus:  Proof of Time (delay proof heartbeat + deterministic round-robin)
Status:     Live
Source:     https://github.com/doli-network/doli
Explorer:   https://doli.network
```

---

## 20. Scope

DOLI optimizes for moving value with deterministic finality, predictable timing, and extensible spending conditions. The base layer is intentionally minimal — but extensible by design.

This constraint is a feature. A system that does one thing well is more secure, more auditable, and more resistant to governance capture than a system that attempts to be a universal computer. Bitcoin demonstrated that a focused protocol can sustain a trillion-dollar network. Complexity is not a prerequisite for value.

The difference: Bitcoin's output format was fixed in 2009. DOLI's output format was designed in 2026 with seventeen years of hindsight. The `extra_data` field exists from genesis — no SegWit, no Taproot, no backward-compatibility hacks required.

---

## 21. Conclusion

We have proposed a system for electronic transactions that requires no trust in institutions, no massive energy expenditure, and no capital accumulation to participate in consensus.

We started with the usual framework of coins made from digital signatures, which provides strong control of ownership. This is incomplete without a way to prevent double-spending. To solve this, we proposed a peer-to-peer network using sequential delay proofs to anchor consensus to time.

**Nodes vote with their time.** The network cannot be accelerated by wealth or parallelized by hardware. One hour of sequential computation is one hour, whether performed by an individual or a nation-state.

**Rewards are deterministic, not probabilistic.** Every producer receives equal block assignments through pure round-robin. The protocol acts as a built-in pool, distributing epoch rewards bond-weighted on-chain to all producers who prove continuous presence through on-chain liveness attestations. External pools are unnecessary. The smallest participant receives the same percentage return as the largest.

The network is robust in its simplicity. Nodes work with little coordination. They do not need to be identified, since messages are not routed to any particular place and only need to be delivered on a best effort basis. Nodes can leave and rejoin the network at will, accepting the heaviest chain as proof of what happened while they were gone.

**The rules are fixed at genesis. The emission is predictable.**

Any needed rules and incentives can be enforced with this consensus mechanism.

---

**DOLI v4.4.9**

*"Time is the only fair currency."*

**E. Weil** · contact: weil@doli.network

---
## References

1. Nakamoto, S. (2008). *Bitcoin: A Peer-to-Peer Electronic Cash System.*

2. Boneh, D., Bonneau, J., Bünz, B., & Fisch, B. (2018). *Verifiable Delay Functions.* In Advances in Cryptology – CRYPTO 2018.

3. Wesolowski, B. (2019). *Efficient Verifiable Delay Functions.* In Advances in Cryptology – EUROCRYPT 2019. (Cited for contrast — DOLI uses iterated hash chains, not algebraic VDFs. See Section 5.1.)

4. Yakovenko, A. (2018). *Solana: A new architecture for a high performance blockchain.* Uses SHA-256 iterated hashing for Proof of History under the same sequential hardness assumption.

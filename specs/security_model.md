# DOLI Security

This document describes the security model, threat analysis, cryptographic foundations, and implementation security measures of the DOLI protocol.

## Table of Contents

1. [Security Model](#1-security-model)
2. [Threat Model](#2-threat-model)
3. [Cryptographic Security](#3-cryptographic-security)
4. [Economic Security](#4-economic-security)
5. [Consensus Security](#5-consensus-security)
6. [Implementation Security](#6-implementation-security)
   - 6.1 [Network Layer Security](#61-network-layer-security) (Equivocation Detection, Peer Scoring, Rate Limiting, TX Malleability)
   - 6.2 [Consensus Security Implementations](#62-consensus-security-implementations) (Anti-Sybil, Anti-Grinding)
   - 6.3-6.7 [Code Security](#63-constant-time-operations) (Constant-Time, Overflow, Validation, Memory, Serialization)
7. [Known Limitations](#7-known-limitations)
8. [Audit Trail](#8-audit-trail)
9. [Responsible Disclosure](#9-responsible-disclosure)

---

## 1. Security Model

### 1.1 Core Security Assumption

DOLI's security relies on the assumption that **honest participants collectively control more sequential computation capacity than any adversarial coalition**. This differs from Proof-of-Work systems where security depends on hash rate, and from Proof-of-Stake systems where security depends on bonded capital.

### 1.2 Security Properties

| Property | Guarantee | Mechanism |
|----------|-----------|-----------|
| **Double-spend prevention** | Computational | UTXO model with signature verification |
| **Censorship resistance** | Economic | Producer rotation, bond requirements |
| **Finality** | Probabilistic | VDF chain extension |
| **Sybil resistance** | Time + Economic | VDF registration + bond |
| **Liveness** | Conditional | Honest majority of producers |

### 1.3 Trust Assumptions

- **No trusted setup**: The protocol does not require trusted parameter generation
- **No trusted parties**: All validation is deterministic and verifiable
- **No trusted time source**: Time is anchored via VDF proofs, not external oracles

---

## 2. Threat Model

### 2.1 Adversary Capabilities

We assume an adversary that can:

1. **Control network**: Delay, reorder, or drop messages (but not indefinitely)
2. **Create identities**: Register as many producers as time/bond allows
3. **Coordinate**: Multiple malicious actors can coordinate attacks
4. **Compute in parallel**: Use arbitrarily many parallel processors

The adversary **cannot**:

1. **Break cryptographic assumptions**: Ed25519, BLAKE3 preimage/collision resistance
2. **Accelerate sequential computation**: Hash-chain VDFs require inherently sequential work
3. **Forge signatures**: Without possessing private keys
4. **Violate timing bounds**: VDF output cannot be computed faster than T iterations

### 2.2 Attack Categories

#### 2.2.1 Double-Spend Attacks

| Attack | Mitigation |
|--------|------------|
| Race attack | Wait for confirmations; VDF prevents fast reorgs |
| Finney attack | Producer rotation limits pre-mining advantage |
| Long-range attack | Checkpointing; bond lock duration (~4 years) |

#### 2.2.2 Sybil Attacks

| Attack | Mitigation |
|--------|------------|
| Identity flooding | VDF registration requires sequential time |
| Cheap identity creation | Bond requirement (10 DOLI/bond mainnet, 1 DOLI/bond testnet/devnet, 1-3,000 bonds) |
| Identity accumulation | Registration difficulty scales with demand |

#### 2.2.3 Consensus Attacks

| Attack | Mitigation |
|--------|------------|
| Nothing-at-stake | Bond slashing for equivocation |
| Grinding | Epoch Lookahead: selection uses `slot % total_tickets`, independent of `prev_hash` |
| Time manipulation | Slot anchored to VDF-proven timestamp |
| Genesis-time hijack | `genesis_hash` in every block header (see 2.2.5) |

#### 2.2.4 Network Attacks

| Attack | Mitigation |
|--------|------------|
| Eclipse attack | Kademlia DHT discovery; gossipsub peer scoring with eviction; IP diversity tracking (max 3 per /24); transport headroom (1.5× max_peers) ensures scoring runs before rejection |
| DoS on producers | Producer rotation; multiple active producers |
| Transaction censorship | Fee market; producer competition |

#### 2.2.5 Genesis-Time Hijack Attack

**Discovered**: 2026-02-28 during mainnet incident

**Attack**: A node with a different `genesis_timestamp` produces blocks with wildly different
slot numbers (e.g., slot 9000 vs slot 400). If validation is bypassed or insufficient, other
nodes accept these blocks. If the attacker has majority stake, fork choice selects their chain
and honest nodes can no longer produce (their `current_slot` is far behind the new tip).

**Root cause**: Integer division in `timestamp_to_slot()` means a 1-second genesis difference
produces the same slot 90% of the time, making slot derivation alone insufficient.

**Mitigation (v2 protocol)**:

1. **`genesis_hash` in BlockHeader**: Every block carries
   `BLAKE3(genesis_time || network_id || slot_duration || message)`. Any parameter change
   produces a completely different hash. Checked as the FIRST validation step in both
   Full and Light modes.

2. **Embedded chainspec (mainnet)**: Mainnet always uses the chainspec compiled into the
   binary. Disk files and `--chainspec` CLI flag are ignored. Prevents accidental or
   malicious genesis parameter override.

3. **Slot derivation in Light mode**: Sync path now validates
   `slot == timestamp_to_slot(timestamp)` even for historical blocks.

**Defense layers**:

| Layer | Check | Mode |
|-------|-------|------|
| 1 | `genesis_hash` match | Full + Light |
| 2 | Slot derivation from genesis | Full + Light |
| 3 | Slot advancing from parent | Full + Light |
| 4 | Timestamp bounds (wall-clock) | Full only |

---

## 3. Cryptographic Security

### 3.1 Hash Function: BLAKE3-256

**Security Level**: 128-bit collision resistance, 256-bit preimage resistance

**Properties**:
- Merkle-Damgard based with tree structure
- No known practical attacks
- Faster than SHA-256 while maintaining security

**Usage**:
- Transaction hashing
- Block header hashing
- Merkle tree construction
- VDF input derivation
- Address derivation

**Implementation**:
```rust
// Domain separation prevents cross-protocol attacks
HASH("DOLI_VDF_BLOCK_V1" || prev_hash || tx_root || slot || producer_key)  // Block VDF input
HASH("DOLI_VDF_REGISTER_V1" || data)                                       // Registration VDF input
HASH("SEED" || data)                                                        // Producer selection seed
```

### 3.2 Digital Signatures: Ed25519

**Security Level**: ~128-bit security (equivalent to 3072-bit RSA)

**Properties**:
- Deterministic signatures (no random nonce needed)
- Fast verification (~15,000 verifications/second)
- Small signatures (64 bytes) and keys (32 bytes)
- Resistant to timing attacks by design

**Usage**:
- Transaction authorization
- Message authentication

**Implementation Security**:
```rust
// Constant-time signature verification
pub fn verify(message: &[u8], signature: &Signature, public_key: &PublicKey) -> bool {
    // Uses ed25519-dalek with constant-time operations
    public_key.verify(message, signature).is_ok()
}
```

### 3.3 Verifiable Delay Functions

DOLI uses two VDF types for different purposes:

#### 3.3.1 Block/Heartbeat VDF: Hash-Chain

**Construction**: Iterated hash chain using BLAKE3

**Security Assumptions**:
- Sequentiality of hash chain computation (no parallel speedup)
- Preimage resistance of BLAKE3

**Parameters**:
| Parameter | Consensus Constant | NetworkParams Default | Rationale |
|-----------|-------------------|----------------------|-----------|
| T_BLOCK | 800,000 | 1,000 (all networks) | Bond is primary Sybil defense |
| Target Time | ~55ms (at 800K) | <1ms (at 1,000) | Minimal computation overhead |

**Properties**:
- **Sequentiality**: Cannot be parallelized
- **Verification**: Recompute the entire chain (O(T))
- **Unique output**: Given input, only one valid output exists
- **NetworkParams override**: All networks default to 1,000 iterations via `NetworkParams::defaults()`. The consensus constant `T_BLOCK=800,000` exists for potential future tightening via protocol upgrade

**Note**: A dynamic calibration module exists in the codebase but is not currently
used for block production. Block iterations are fixed per network configuration.

**Security Considerations**:
1. Input includes domain tag (`"DOLI_VDF_BLOCK_V1"`) for block VDF separation (`"DOLI_HEARTBEAT_V1"` is used separately for the tpop heartbeat system)
2. Calibration bounds prevent extreme values (min: 100K, max: 100M iterations)
3. Combined with Epoch Lookahead for grinding prevention

#### 3.3.2 Registration VDF: Hash-Chain

**Construction**: Iterated BLAKE3 hash chain (same as block VDF, higher iteration count)

**Security Assumptions**:
- Sequentiality of hash chain computation (no parallel speedup)
- Preimage resistance of BLAKE3

**Parameters**:
| Parameter | Consensus Constant | NetworkParams Default | Rationale |
|-----------|-------------------|----------------------|-----------|
| T_REGISTER_BASE | 1,000 | 1,000 (all networks) | Minimal barrier; bond is primary Sybil defense |
| T_REGISTER_CAP | 5,000,000 | N/A (not currently applied) | Reserved for future tightening |

**Properties**:
- **Sequentiality**: Cannot be parallelized
- **Verification**: Recompute the entire chain (O(T))
- **Unique output**: Given input, only one valid output exists
- **Devnet exemption**: VDF validation skipped on devnet for fast testing

**Note**: A Wesolowski VDF crate (`doli-vdf`) exists in the codebase using class groups
with GMP, but is NOT used in production. Both block and registration VDFs use the
iterated BLAKE3 hash-chain implementation in `doli-core/src/tpop/heartbeat.rs`.

### 3.4 Cryptographic Constants

```rust
// crypto crate domain tags (crates/crypto/src/lib.rs)
pub const SIGN_DOMAIN: &[u8] = b"DOLI_SIGN_V1";
pub const ADDRESS_DOMAIN: &[u8] = b"DOLI_ADDR_V1";
pub const TX_DOMAIN: &[u8] = b"DOLI_TX_V1";
pub const BLOCK_DOMAIN: &[u8] = b"DOLI_BLOCK_V1";
pub const VDF_DOMAIN: &[u8] = b"DOLI_VDF_V1";
pub const ATTESTATION_DOMAIN: &[u8] = b"DOLI_ATTEST_V1";

// Inline domain tags used in core/vdf/tpop modules
b"DOLI_VDF_BLOCK_V1"          // Block VDF input (consensus/vdf.rs)
b"DOLI_VDF_REGISTER_V1"       // Registration VDF input (vdf crate)
b"DOLI_HEARTBEAT_V1"          // tpop heartbeat VDF input
b"DOLI_HEARTBEAT_SIGN_V1"     // Heartbeat signing
b"DOLI_HEARTBEAT_WITNESS_V1"  // Heartbeat witness
b"DOLI_PRESENCE_V1"           // Presence heartbeats
b"DOLI_PRESENCE_CHECKPOINT_V1" // Presence checkpoints
b"DOLI_PRODUCER_ANN_V1"       // Producer announcements
b"DOLI_HASHLOCK"              // Hashlock conditions
b"DOLI_ADAPTOR_NONCE_V1"      // Adaptor signature nonces
b"SEED"                        // Producer selection seed

// Hash output size
pub const HASH_SIZE: usize = 32;

// Signature sizes
pub const SIGNATURE_SIZE: usize = 64;
pub const PUBLIC_KEY_SIZE: usize = 32;
pub const PRIVATE_KEY_SIZE: usize = 32;
```

---

## 4. Economic Security

### 4.1 Bond Mechanism

The bond serves multiple security functions:

| Function | Mechanism |
|----------|-----------|
| Sybil resistance | Capital requirement limits identity creation |
| Accountability | Bond at risk for misbehavior |
| Long-term alignment | Lock duration creates stake in network success |

**Bond Unit**: Mainnet: 10 DOLI per bond (1,000,000,000 base units). Testnet/Devnet: 1 DOLI per bond (100,000,000 base units). Fixed across all eras (never decreases). Producers can stake 1-3,000 bonds. Bonds are stored as Bond UTXOs (`output_type=1`, `lock_until=u64::MAX`) with `creation_slot` in `extra_data`.

### 4.2 Slashing Conditions

| Violation | Penalty | Proof Required |
|-----------|---------|----------------|
| Double production | 100% bond burned | Two valid BlockHeaders for same slot |
| Invalid block | None (slot lost) | N/A - Block rejected by network |
| Inactivity | None | N/A - Natural consequence (missed rewards) |

**Important**: Slashing is reserved ONLY for double production (equivocation). This is the only offense that cannot happen by accident - it requires intentionally signing two different blocks for the same slot.

**Slashing Evidence Requirements**:
```rust
SlashingEvidence::DoubleProduction {
    block_header_1: BlockHeader,  // Full header with VDF proof
    block_header_2: BlockHeader,  // Full header with VDF proof
}
```

Validators verify:
1. Both headers have the same producer public key
2. Both headers have the same slot number
3. Both headers have different hashes
4. Both headers have valid VDF outputs (proving actual computation)

**Why 100% Burn?**
- Invalid blocks are NOT slashed - the natural penalty (losing the slot and its reward) is sufficient
- Following Bitcoin's philosophy: the network rejects bad blocks, you wasted your time, end of story
- Double production is unambiguously intentional fraud

**Why Burned, Not Redistributed?**
- Prevents incentivizing false accusations
- Eliminates collusion between accusers and validators
- Removes gaming opportunities from slashing mechanism
- Deflationary: reduces total supply

### 4.3 Economic Attack Costs

To control the network, an attacker would need to:

1. **Register majority of producers**: Requires (N/2 + 1) * VDF_TIME sequential computation
2. **Maintain bonds**: Requires (N/2 + 1) * BOND_AMOUNT capital at risk
3. **Risk detection**: Equivocation leads to permanent bond loss

**Cost Analysis** (Era 0, assuming 100 active producers):
- Minimum 51 registrations: VDF barrier is minimal (1,000 iterations per registration, <1ms each). Bond staking is the real cost.
- Bond requirement: 51 * 10 DOLI = 510 DOLI minimum at risk (1 bond each)
- Potential loss if detected: All bonded capital slashed
- **Note**: The primary Sybil defense is the bond requirement, not VDF computation time. Registration VDF at T_REGISTER_BASE=1,000 is near-instant. Anti-parallel protection comes from the chained registration hash requirement.

### 4.4 Time-Based Economics

Unlike capital-based systems, time cannot be:
- Borrowed or leveraged
- Accumulated faster through spending
- Transferred between parties

This creates a fundamental limit on how fast identities can be created, regardless of financial resources.

---

## 5. Consensus Security

### 5.1 Chain Selection (Weight-Based Fork Choice)

DOLI uses a **weight-based fork choice rule**. The chain with the highest accumulated
producer weight wins. Each block's weight equals the producer's `effective_weight`
(seniority-based: 1 for Year 1, up to 4 for Year 4+).

```python
def should_reorg(current_chain, new_chain):
    current_weight = accumulated_weight(current_chain.tip)
    new_weight = accumulated_weight(new_chain.tip)
    return new_weight > current_weight
```

**Security Properties**:
- Prevents Sybil attacks with many low-weight producers
- Senior producers' chains are preferred (seniority = trust)
- Favors honest, long-running chains
- Deterministic: all nodes converge to the heaviest chain

### 5.2 Producer Selection (Deterministic Round-Robin)

```
sorted_producers = sort by pubkey (deterministic)
total_tickets = sum of all bond counts (each bond = 1 ticket)
ticket_index = slot % total_tickets
selected = find producer whose cumulative ticket range contains ticket_index
```

**Security Properties**:
- **Independent of prev_hash**: Selection uses `slot % total_tickets`, NOT hash-based lottery. This prevents grinding attacks entirely (Epoch Lookahead).
- **Deterministic**: All honest nodes compute same result for any slot
- **Proportional**: Each producer gets exactly their bond proportion of slots
- **Unbiasable**: Attacker cannot influence future selection by manipulating block content

### 5.3 Timing Constraints

```
slot_start = GENESIS_TIME + (slot * SLOT_DURATION)
valid_window = [slot_start + SLOT_DURATION - NETWORK_MARGIN,
                slot_start + SLOT_DURATION + DRIFT]
```

**Security Properties**:
- Prevents "rushing" blocks with accelerated hardware
- Provides tolerance for network latency
- Anchors consensus to real-world time progression

### 5.4 Bootstrap Phase Security

During bootstrap (first 60,480 blocks, ~1 week):

| Risk | Mitigation |
|------|------------|
| Single party dominance | Weight-based fork choice (accumulated producer weight) |
| No economic stake | Bootstrap phase is time-limited |
| Racing attacks | VDF still required for each block |

---

## 6. Implementation Security

### 6.1 Network Layer Security

#### 6.1.1 Equivocation Detection

The equivocation detector tracks block production to detect double-signing (the only slashable offense).

**Implementation Details** (`crates/network/src/sync/equivocation.rs`):

```
┌─────────────────────────────────────────────────────────────────┐
│  EQUIVOCATION DETECTOR                                          │
├─────────────────────────────────────────────────────────────────┤
│  Storage: HashMap<(PublicKey, Slot), BlockHeader>               │
│  LRU Cache: MAX_TRACKED_SLOTS = 1000                            │
│  Data Stored: Full BlockHeader (not just hash)                  │
│  Purpose: Enable VDF verification in slashing evidence          │
└─────────────────────────────────────────────────────────────────┘
```

**Why Full Headers?**
- Block headers contain VDF output and proof
- Validators must verify the producer actually computed valid VDFs for both blocks
- Prevents fabricated evidence attacks (can't invent fake block hashes)
- Evidence is cryptographically verifiable by any node

**Detection Flow**:
1. Block received → check if `(producer, slot)` exists in tracker
2. If exists with different hash → **EQUIVOCATION DETECTED**
3. Create `EquivocationProof` with both full `BlockHeader`s
4. Convert to `SlashProducer` transaction with reporter signature
5. Broadcast for inclusion in blockchain

**Slashing Evidence Structure**:
```rust
SlashingEvidence::DoubleProduction {
    block_header_1: BlockHeader,  // Full header with VDF
    block_header_2: BlockHeader,  // Full header with VDF
}
```

#### 6.1.2 Peer Scoring System

The peer scorer tracks reputation to identify and penalize misbehaving nodes.

**Implementation Details** (`crates/network/src/scoring.rs`):

| Infraction | Penalty | Notes |
|------------|---------|-------|
| Invalid Block | -100 | Serious protocol violation |
| Invalid Transaction | -20 | May be honest relay of bad tx |
| Timeout | -5 × count | Escalating penalty (max -50) |
| Spam | -50 | DoS attempt |
| Duplicate Message | -5 | Mild, may be network issue |
| Malformed Message | -30 | Cannot parse data |
| Incompatible Version | -200 | Instant disconnect — peer protocol version below minimum |

**Thresholds**:

| Threshold | Value | Action |
|-----------|-------|--------|
| Disconnect | -200 | Close connection immediately |
| Ban | -500 | Block peer for 1 hour |
| Score Range | -1000 to +1000 | Clamped to prevent overflow |

**Positive Scoring**:
- Valid block received: +10 points
- Valid transaction received: +1 point

**Score Decay**:
- Scores decay towards zero over time (1 point/minute by default)
- Infractions older than 1 hour are pruned from history
- Bans expire after 1 hour

#### 6.1.3 Rate Limiting

Token bucket rate limiting protects against DoS attacks at both per-peer and global levels.

**Implementation Details** (`crates/network/src/rate_limit.rs`):

| Resource | Per-Peer Limit | Global Limit |
|----------|---------------|--------------|
| Blocks | 10/minute | 100/minute |
| Transactions | 50/second | 200/second |
| Requests | 20/second | - |
| Bandwidth | 1 MB/second | 10 MB/second |

**Token Bucket Algorithm**:
- Capacity: Burst allowance (e.g., 10 blocks for per-peer block limit)
- Refill Rate: Tokens per second (e.g., 10/60 = 0.167 blocks/sec)
- Check before accept: `can_consume(amount)` returns false if bucket empty
- Record after accept: `try_consume(amount)` deducts tokens

**Features**:
- Per-peer tracking via `HashMap<PeerId, PeerLimits>`
- Global limits prevent aggregate flooding
- Stale peer cleanup (max 1000 tracked peers)
- Can be disabled for testing via config

#### 6.1.4 Transaction Malleability Prevention

Transaction hashes exclude signatures to prevent third-party modification.

**Implementation** (`crates/core/src/transaction/core.rs`):

```rust
pub fn hash(&self) -> Hash {
    let mut hasher = Hasher::new();
    hasher.update(&self.version.to_le_bytes());
    hasher.update(&(self.tx_type as u32).to_le_bytes());

    // Hash inputs (WITHOUT signatures)
    hasher.update(&(self.inputs.len() as u32).to_le_bytes());
    for input in &self.inputs {
        hasher.update(input.prev_tx_hash.as_bytes());
        hasher.update(&input.output_index.to_le_bytes());
        // Signature is NOT included in tx hash
    }

    // Hash outputs and extra_data
    // ...
}
```

**What This Prevents**:
- Third parties cannot modify a signed transaction's hash
- Protects against transaction replacement attacks
- Ensures txid stability for dependent transactions
- Similar to Bitcoin's SegWit approach

**What Is Hashed**:
- Version, transaction type
- Input outpoints (prev_tx_hash, output_index) - **NOT signatures**
- All outputs (type, amount, pubkey_hash, lock_until)
- Extra data (registration info, slash evidence, etc.)

### 6.2 Consensus Security Implementations

#### 6.2.1 Anti-Sybil Protection (Registration)

Producer registration uses a hash-chain VDF (iterated BLAKE3) for Sybil resistance.

**Implementation Details**:

| Parameter | Consensus Constant | Network Default (all networks) |
|-----------|-------------------|-------------------------------|
| T_REGISTER_BASE | 1,000 | 1,000 (via `vdf_register_iterations`) |
| T_REGISTER_CAP | 5,000,000 | Not applied (reserved for future tightening) |

**Chained Hash System** (`RegistrationData` in `transaction/data.rs`):
```rust
pub struct RegistrationData {
    pub public_key: PublicKey,
    pub epoch: u32,
    pub vdf_output: Vec<u8>,
    pub vdf_proof: Vec<u8>,
    pub prev_registration_hash: Hash,  // Chain to previous registration
    pub sequence_number: u64,          // Monotonic counter
    pub bond_count: u32,               // On-chain bond count (consensus-critical)
    pub bls_pubkey: Vec<u8>,           // BLS12-381 public key (48 bytes, optional)
    pub bls_pop: Vec<u8>,              // BLS proof-of-possession (96 bytes, optional)
}
```

**Anti-Parallel Attack**:
- Each registration references previous registration's hash
- Attacker cannot register multiple nodes simultaneously
- Must wait for each registration to be confirmed before starting next
- Sequence numbers prevent replay attacks

#### 6.2.2 Anti-Grinding Protection (Block Production)

Block VDF input construction prevents pre-computation attacks.

**VDF Input Construction** (`crates/core/src/consensus/vdf.rs` and `crates/vdf/src/lib.rs`):
```rust
// In consensus/vdf.rs: construct_vdf_input()
// In vdf crate: block_input()
// Both produce identical output:
pub fn construct_vdf_input(
    prev_hash: &Hash,        // Unknown until previous block
    tx_root: &Hash,          // Block content commitment (merkle root)
    slot: Slot,              // Time ordering
    producer_key: &PublicKey, // Identity binding
) -> Hash {
    hash_concat(&[
        b"DOLI_VDF_BLOCK_V1",
        prev_hash.as_bytes(),
        tx_root.as_bytes(),
        &slot.to_le_bytes(),
        producer_key.as_bytes(),
    ])
}
```

**Parameters**:

| Parameter | Consensus Constant | NetworkParams Default | Effect |
|-----------|-------------------|----------------------|--------|
| T_BLOCK | 800,000 iterations | 1,000 | <1ms computation at network default |
| Slot Duration | 10 seconds | 10 seconds | Full slot for production |
| Fallback Window | 2,000ms per rank | 2,000ms | 2 ranks (primary + single fallback) |

**Why Grinding Is Impractical**:
1. **Epoch Lookahead**: Selection uses `slot % total_tickets`, independent of `prev_hash`
2. **prev_hash in VDF input**: VDF output changes with different block content, but cannot influence selection
3. **Sequential VDF**: Cannot be parallelized regardless of iteration count
4. **No Compounding**: Winning slot N provides no advantage for N+1
5. **Deterministic rotation**: Producer schedule is fixed for the entire epoch at epoch boundary

### 6.3 Constant-Time Operations

Critical operations use constant-time implementations to prevent timing attacks:

```rust
// Constant-time comparison for sensitive data
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
```

**Protected Operations**:
- Signature verification
- Hash comparison in validation
- Private key operations

### 6.4 Integer Overflow Protection

All arithmetic operations use checked or saturating arithmetic:

```rust
// Amount calculations use checked arithmetic
pub fn total_output(&self) -> Option<Amount> {
    self.outputs.iter()
        .map(|o| o.amount)
        .try_fold(0u64, |acc, amt| acc.checked_add(amt))
}

// Bond calculation uses u128 to prevent overflow
pub fn bond_amount(&self, height: BlockHeight) -> Amount {
    let mut numerator: u128 = self.initial_bond as u128;
    let mut denominator: u128 = 1;
    for _ in 0..era {
        numerator *= 7;
        denominator *= 10;
    }
    (numerator / denominator) as Amount
}
```

### 6.5 Input Validation

All external inputs are validated before processing:

```rust
// Transaction validation checks
pub fn validate_transaction(tx: &Transaction, ctx: &ValidationContext)
    -> Result<(), ValidationError>
{
    // Version check
    if tx.version != CURRENT_VERSION {
        return Err(ValidationError::InvalidVersion { ... });
    }

    // Structural checks
    if tx.inputs.is_empty() && !tx.is_coinbase() {
        return Err(ValidationError::NoInputs);
    }
    if tx.outputs.is_empty() {
        return Err(ValidationError::NoOutputs);
    }

    // Amount validation
    for output in &tx.outputs {
        if output.amount == 0 {
            return Err(ValidationError::ZeroAmount { ... });
        }
        if output.pubkey_hash.is_zero() {
            return Err(ValidationError::ZeroPubkeyHash { ... });
        }
    }

    // Total supply check
    if tx.total_output() > TOTAL_SUPPLY {
        return Err(ValidationError::ExceedsTotalSupply { ... });
    }

    Ok(())
}
```

### 6.6 Memory Safety

- Written in Rust, providing memory safety guarantees
- No unsafe code in core cryptographic operations
- Bounds checking on all array accesses
- Automatic cleanup of sensitive data via Drop trait

### 6.7 Serialization Security

```rust
// Deserialization validates all fields
impl Transaction {
    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        // bincode with strict limits
        let config = bincode::config::standard()
            .with_limit::<MAX_TX_SIZE>();
        bincode::decode_from_slice(bytes, config).ok()
    }
}
```

---

## 7. Known Limitations

### 7.1 Long-Range Attacks

**Risk**: Attacker with old keys could create alternative history

**Mitigations**:
- Bond lock duration (~4 years) makes this expensive
- Social consensus on checkpoints
- New nodes should sync from trusted sources

### 7.2 VDF Hardware Acceleration

**Risk**: Custom hardware (ASICs) could compute VDFs faster

**Mitigations**:
- BLAKE3 hash-chain VDF computation is inherently sequential
- T parameter can be increased via protocol upgrade
- Security degrades gracefully (faster blocks, not broken security)

### 7.3 Network Partition

**Risk**: Network splits could create competing chains

**Mitigations**:
- Weight-based fork choice rule provides clear resolution (heaviest chain wins)
- VDF ensures time passes even during partition
- Reconciliation upon reconnection is deterministic

### 7.4 Producer Centralization

**Risk**: Small number of producers could dominate

**Mitigations**:
- No economies of scale in VDF computation
- Bond is fixed at BOND_UNIT (10 DOLI mainnet) with MAX_BONDS cap (3,000) limiting maximum stake per producer
- Producers who miss slots miss rewards (natural economic penalty). No slashing for inactivity

### 7.5 Producer Selection Grinding (Epoch Lookahead Defense)

**Risk**: A malicious producer of block N-1 could attempt to manipulate `prev_hash` to influence producer selection for future slots.

**Defense: Epoch Lookahead**

DOLI's producer selection uses `slot % total_tickets` (deterministic round-robin based on bond count), which is completely independent of `prev_hash`. This eliminates grinding attacks entirely:

```
selected_producer(slot) = find_ticket_owner(slot % total_tickets)
```

The selection depends ONLY on:
1. The slot number (deterministic from wall-clock time)
2. The producer set and bond counts (frozen at epoch boundary)

Since `prev_hash` is not used in selection, manipulating block content cannot influence who produces future blocks. This is a fundamental design choice -- Epoch Lookahead trades theoretical unpredictability for grinding immunity.

**prev_hash in VDF input**: The VDF input includes `prev_hash`, but this only affects VDF output validity (anti-pre-computation), NOT producer selection. An attacker cannot pre-compute VDF for a future block because they do not know `prev_hash` until the previous block is finalized.

**Residual risk**: The only remaining vector is manipulating the producer set itself (registering/deregistering at epoch boundaries to shift ticket assignments). This is mitigated by:
1. Registration requires VDF computation + bond
2. Bond withdrawals incur vesting penalties
3. Producer set changes are deferred to epoch boundaries
4. MAX_BONDS cap (3,000) limits any single entity's influence

**Status**: Grinding is a non-issue due to Epoch Lookahead. No monitoring needed for this specific attack vector.

*Updated: 2026-03-29 -- rewrote to reflect actual deterministic selection (was describing obsolete hash-based selection)*

---

## 8. Audit Trail

### 8.1 Cryptographic Libraries

| Component | Library | Version | Audit Status |
|-----------|---------|---------|--------------|
| Hashing | blake3 | 1.x | Widely reviewed |
| Signatures | ed25519-dalek | 2.x | Audited |
| Random | rand | 0.8.x | Audited |
| Serialization | bincode | 2.x | Widely used |

### 8.2 Security Reviews

| Date | Scope | Reviewer | Status |
|------|-------|----------|--------|
| 2026-01-25 | VDF slashing evidence | Internal audit | Fixed (5863805) |
| 2026-02-02 | Security documentation | Internal | Documented |
| 2026-02-28 | Genesis-time hijack, merkle root, chainspec hardening | Internal | Fixed (5 fixes) |
| TBD | Full protocol | TBD | Pending |

**2026-02-28 - Genesis Security Hardening (5 fixes)**:
- **Fix 1**: Removed MERKLE_FIX_HEIGHT gate — all blocks validated unconditionally
- **Fix 2**: Mainnet chainspec embedded in binary — disk/CLI overrides disabled
- **Fix 3**: Added slot derivation + genesis_hash checks to Light validation mode
- **Fix 4**: Unified block production — merkle root computed from final transaction list
- **Fix 5**: Added `genesis_hash` field to BlockHeader (v2) — chain identity fingerprint
- **Incident**: N6 joined with old binary (25h behind genesis), causing slot divergence and chain takeover

**2026-02-02 - Security Implementation Documentation**:
- **Scope**: Documented network layer security implementations
- **Items**: Equivocation detection (LRU cache, full headers), peer scoring system,
  rate limiting (token bucket), transaction malleability prevention, anti-Sybil
  (chained VDF), anti-grinding (prev_hash in VDF input)
- **Status**: All implementations verified against codebase

**2026-01-25 - VDF Slashing Evidence Fix**:
- **Issue**: Slashing evidence only contained block hashes, not full headers
- **Impact**: Verifiers couldn't validate VDF proofs in slashing claims
- **Fix**: Changed SlashingEvidence to include full BlockHeaders for VDF verification
- **Commit**: `5863805`

**Note**: VDF iterations are configured via NetworkParams. Block production defaults to 1,000 iterations (<1ms) across all networks. Registration uses T_REGISTER_BASE=1,000 iterations (consensus constant T_REGISTER_CAP=5,000,000 exists but is not currently applied by network defaults). Bond staking is the primary Sybil defense.

### 8.3 Test Coverage

| Module | Unit Tests | Property Tests | Coverage |
|--------|------------|----------------|----------|
| doli-crypto/hash | 12 | 5 | High |
| doli-crypto/keys | 10 | 2 | High |
| doli-crypto/signature | 9 | 3 | High |
| doli-crypto/merkle | 11 | 2 | High |
| doli-core/types | 5 | 6 | High |
| doli-core/transaction | 10 | 12 | High |
| doli-core/consensus | 12 | 15 | High |
| doli-core/validation | 18 | 12 | High |

### 8.4 Formal Verification

- [ ] VDF correctness proof
- [ ] Consensus safety proof
- [ ] Economic security analysis

---

## 9. Responsible Disclosure

### 9.1 Reporting Security Issues

If you discover a security vulnerability, please report it responsibly:

**Email**: doli@doli.network

**PGP Key**: [To be published]

### 9.2 Disclosure Policy

1. **Report privately**: Contact us before public disclosure
2. **Allow 90 days**: For fix development and deployment
3. **Coordinate disclosure**: Work with us on timing
4. **Credit**: Reporters will be credited (if desired)

### 9.3 Bug Bounty

A bug bounty program will be established after mainnet launch. Categories:

| Severity | Example | Reward Range |
|----------|---------|--------------|
| Critical | Consensus break, fund theft | $10,000+ |
| High | DoS attack, privacy leak | $1,000-$10,000 |
| Medium | Minor validation bypass | $100-$1,000 |
| Low | Best practice violation | $50-$100 |

---

## References

1. Boneh, D., et al. "Verifiable Delay Functions." CRYPTO 2018.
2. Wesolowski, B. "Efficient Verifiable Delay Functions." EUROCRYPT 2019.
3. Bernstein, D., et al. "High-speed high-security signatures." Journal of Cryptographic Engineering, 2012.
4. Buchmann, J., Vollmer, U. "Binary Quadratic Forms." Springer, 2007.

---

*Last updated: 2026-03-29 (synced against code)*
*DOLI Security Team*
# DOLI Security Checklist

This checklist helps node operators and producers secure their DOLI infrastructure.

## Node Security

### Network Configuration

- [ ] **Firewall enabled** - Only allow necessary ports
  ```bash
  # Allow P2P
  sudo ufw allow 30300/tcp

  # Allow RPC only from trusted IPs (if needed externally)
  sudo ufw allow from 192.168.1.0/24 to any port 8500

  # Allow metrics only from monitoring server
  sudo ufw allow from 10.0.0.5 to any port 9000

  # Enable firewall
  sudo ufw enable
  ```

- [ ] **RPC bound to localhost** - Default is `127.0.0.1:8500`
  ```toml
  # config.toml - GOOD
  [rpc]
  listen_addr = "127.0.0.1:8500"

  # AVOID exposing to all interfaces
  # listen_addr = "0.0.0.0:8500"  # DANGEROUS
  ```

- [ ] **RPC authentication** - Enable if exposing RPC externally
  ```toml
  [rpc]
  enabled = true
  auth_required = true
  api_key = "your-secure-api-key-here"
  ```

- [ ] **Rate limiting enabled** - Protect against DoS
  ```toml
  [rpc]
  rate_limit_per_second = 100
  rate_limit_burst = 200
  ```

- [ ] **Peer diversity protection** - Prevent eclipse attacks
  ```toml
  [network]
  enable_diversity = true
  max_per_prefix = 3
  max_per_asn = 5
  ```

### File Permissions

- [ ] **Data directory permissions**
  ```bash
  chmod 700 ~/.doli
  chmod 600 ~/.doli/node.key
  chmod 600 ~/.doli/config.toml
  ```

- [ ] **Producer key permissions**
  ```bash
  chmod 600 /path/to/producer.key
  ```

- [ ] **Verify no world-readable secrets**
  ```bash
  find ~/.doli -perm /o+r -type f
  # Should return nothing
  ```

### System Hardening

- [ ] **Run as non-root user**
  ```bash
  # Create dedicated user
  sudo useradd -r -s /bin/false doli

  # Set ownership
  sudo chown -R doli:doli ~/.doli

  # Run node as doli user
  sudo -u doli ./doli-node run
  ```

- [ ] **Use systemd with security options**
  ```ini
  # /etc/systemd/system/doli-node.service
  [Service]
  User=doli
  Group=doli
  NoNewPrivileges=yes
  PrivateTmp=yes
  ProtectSystem=strict
  ProtectHome=read-only
  ReadWritePaths=/var/lib/doli
  ```

- [ ] **Enable automatic updates** - Keep OS patched

- [ ] **Disable unnecessary services**

---

## Producer Security

### Key Management

- [ ] **Generate keys offline** (recommended)
  ```bash
  # On air-gapped machine
  ./doli wallet new --name producer
  ./doli wallet export --producer-key producer.key

  # Transfer only public key to online server
  # Keep private key backup secure
  ```

- [ ] **Encrypt producer key at rest**
  ```bash
  # Encrypt with password
  ./doli wallet export --producer-key producer.key --encrypt

  # Decrypt at runtime (requires password input)
  ./doli-node run --producer --producer-key producer.key.enc
  ```

- [ ] **Backup producer key securely**
  - Store encrypted backup offline
  - Consider hardware security module (HSM) for high-value operations
  - Document recovery procedure

- [ ] **Never share or commit keys**
  ```bash
  # Add to .gitignore
  echo "*.key" >> .gitignore
  echo "*.key.enc" >> .gitignore
  ```

### Operational Security

- [ ] **Use dedicated production server** - Separate from development

- [ ] **Enable monitoring and alerting**
  ```bash
  # Example alert rules
  # - Node offline > 5 minutes
  # - Peer count < 3
  # - Missed block slots
  # - Sync falling behind
  ```

- [ ] **Set up hot standby** (optional but recommended)
  - Keep synced node ready
  - Automated failover with proper key handling
  - Never run two nodes with same key simultaneously (causes slashing!)

- [ ] **Document recovery procedures**
  - Node crash recovery
  - Key compromise response
  - Hardware failure response

### Avoiding Slashing

- [ ] **Never run multiple nodes with same producer key**
  - Double production = 100% bond slashed
  - Wait for cooldown when migrating

- [ ] **Monitor VDF completion times**
  ```bash
  # Check metrics
  curl http://localhost:9000/metrics | grep vdf_compute

  # Should complete in <1ms (1,000 iterations at network default)
  ```

- [ ] **Ensure clock synchronization**
  ```bash
  # Install and configure NTP
  sudo apt install chrony
  sudo systemctl enable chrony

  # Verify sync
  chronyc tracking
  ```

---

## Wallet Security

### Key Storage

- [ ] **Encrypt wallet files**
  ```bash
  ./doli wallet new --name main --encrypt
  ```

- [ ] **Use strong passwords** - Minimum 16 characters, random

- [ ] **Backup wallet securely**
  ```bash
  # Export mnemonic (write down, store securely)
  ./doli wallet export --mnemonic

  # Never store mnemonic digitally
  ```

- [ ] **Consider hardware wallet** (when supported)

### Transaction Safety

- [ ] **Verify recipient addresses** - Double-check before sending

- [ ] **Use appropriate fees** - Too low may cause stuck transactions

- [ ] **Test with small amounts** - Before large transfers

- [ ] **Wait for confirmations** - 6+ for important transactions

---

## Network Security

### Connection Security

- [ ] **Use trusted bootstrap nodes**
  ```toml
  [[bootstrap_nodes]]
  address = "/dns4/seed1.doli.network/tcp/30300/p2p/..."

  [[bootstrap_nodes]]
  address = "/dns4/seed2.doli.network/tcp/30300/p2p/..."
  ```

- [ ] **Monitor peer diversity**
  ```bash
  curl http://localhost:8500 \
    -d '{"jsonrpc":"2.0","method":"getPeers","params":{},"id":1}'

  # Check for variety in IP ranges
  ```

- [ ] **Enable NAT traversal** (if behind NAT)
  ```toml
  [network]
  enable_relay = true
  enable_autonat = true
  ```

### Monitoring Suspicious Activity

- [ ] **Watch for eclipse attack signs**
  - All peers from same IP range
  - Sudden peer disconnections
  - Chain state diverging from explorers

- [ ] **Monitor rate limiting triggers**
  ```bash
  grep "rate limit" ~/.doli/logs/*.log
  ```

- [ ] **Check for invalid block attempts**
  ```bash
  grep "invalid block" ~/.doli/logs/*.log
  ```

---

## Incident Response

### Suspected Compromise

1. **Immediately stop producing blocks**
   ```bash
   kill -TERM $(pgrep doli-node)
   ```

2. **Rotate producer key** if compromised
   - Generate new key on secure machine
   - Withdraw bonds (instant — FIFO penalty based on bond age)
   - Re-register with new key

3. **Investigate**
   - Check logs for unauthorized access
   - Review system logs
   - Check for malware

4. **Report** to network operators if network-wide issue

### Key Loss

1. **If backup exists** - Restore from encrypted backup

2. **If no backup** - Bond is lost after withdrawal cooldown expires

3. **Prevention** - Always maintain secure backups

---

## Security Audit Checklist

Run this periodically (monthly recommended):

```bash
#!/bin/bash
echo "=== DOLI Security Audit ==="

echo -n "Firewall status: "
sudo ufw status | head -1

echo -n "RPC binding: "
grep listen_addr ~/.doli/config.toml | grep rpc -A1 | tail -1

echo -n "Data dir permissions: "
stat -c %a ~/.doli

echo -n "Node key permissions: "
stat -c %a ~/.doli/node.key 2>/dev/null || echo "N/A"

echo -n "Running as root: "
[ $(id -u) -eq 0 ] && echo "YES (BAD!)" || echo "No (good)"

echo -n "Peer count: "
curl -s http://localhost:8500 \
  -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}' \
  | grep -o '"peer_count":[0-9]*' | cut -d: -f2

echo -n "Sync status: "
curl -s http://localhost:8500 \
  -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}' \
  | grep -o '"syncing":[a-z]*' | cut -d: -f2

echo "=== Audit Complete ==="
```

---

## Resources

- [DOLI Security Advisories](https://github.com/doli-network/doli/security/advisories)
- [Troubleshooting Guide](./troubleshooting.md)
- [Node Operation Guide](./running_a_node.md)
- [Producer Guide](./becoming_a_producer.md)

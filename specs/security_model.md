<!-- OUTPUT CONTRACT: N/A — security spec document, not a test -->
<!-- INPUT PARTITIONS: N/A — security spec document -->

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
   - 6.8 [Staged Upgrade Trust Boundary](#68-staged-upgrade-trust-boundary) (unprivileged writer, verifying root consumer)
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
| Grinding | Epoch Lookahead: selection uses `active_list[slot % len]`, independent of `prev_hash` |
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
| Double production | 100% of the accused's Bond UTXOs burned (at and above the slashing gate), plus its Unbonding outputs still locked (at and above the unbonding gate) | Two validly signed BlockHeaders for the same slot |
| Invalid block | None (slot lost) | N/A - Block rejected by network |
| Inactivity | None | N/A - Natural consequence (missed rewards) |

**Important**: Slashing is reserved ONLY for double production (equivocation). This is the only offense that cannot happen by accident - it requires intentionally signing two different blocks for the same slot.

**Activation gate**: `inc_i_255_slashing_activation_height`. Mainnet: `806859` (pinned with the INC-I-236, INC-I-257 and INC-I-182 gates). Testnet: `330849` (the same height as the INC-I-236, INC-I-257 and INC-I-182 gates). Devnet: 0, with the devnet-only override `DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT`. The rules apply at `NetworkParams::effective_slashing_activation_height()`, the maximum of this gate, the INC-I-236 block-signature gate and the INC-I-181 pending-root gate. Thus slashing can never start before blocks are signed. Below the effective height no SlashProducer can be mined, because the fee floor refuses its fee of 0. Full rules: `specs/protocol.md` §3.10.

**Slashing Evidence Requirements**:
```rust
SlashingEvidence::DoubleProduction {
    block_header_1: BlockHeader,  // Full header with the producer signature
    block_header_2: BlockHeader,  // Full header with the producer signature
}
```

**Authorship proof**: the block signature, under INV-AUTH-003. A header is authentic only if it carries a valid strict Ed25519 signature by `header.producer` over the domain-separated header hash (`signing_message()`). The VDF does not prove authorship. It only proves sequential work over public inputs, so anyone can compute it for any producer.

At and above the effective height, validators verify:
1. Both headers have the same producer public key, and it is the accused key
2. Both headers have the same slot number
3. Both headers have different hashes
4. Each header has a valid producer signature (INV-AUTH-003)
5. Each header carries the genesis hash of this chain, so evidence from another chain fails
6. The encoded evidence is at most 64 KiB, is canonical (no trailing bytes) and lists the headers in ascending hash order
7. The accused is a registered producer, not Slashed, has no queued Slash, and is Active or Unbonding or still holds locked collateral (an Unbonding output owned by its bond owner with judged height < `lock_until`)
8. The evidence is not expired: its age is less than `unbonding_period`
9. The evidence does not predate the accused's current registration: the tenure is at least 1, the age is at most the blocks since the effective height, and, with no locked collateral, the offence slot is at or after the slot of the accused's oldest Bond UTXO

Rules 7, 8 and 9 run in this order, after rules 1 to 6.

**Evidence window (AUDIT-P2-539, AUDIT-P2-547)**: the age is counted in slots, from the offence slot (the slot of the two headers) to the judged slot, which is the parent slot + 1 (saturating; an offence slot at or after the judged slot has age 0). Mempool admission and revalidation use the tip slot that the caller passes, the builder uses the parent slot (not the slot of the block it builds), and block validation uses the parent slot, so all four paths give one verdict. `unbonding_period` is a block count. A block takes at least one slot, so the slot window expires no later than the block window. The tenure is `judged height − max(registered_at, effective height)` (saturating); a tenure of 0 refuses. The gate bound refuses an age above `judged height − effective height`: block slots strictly increase with height, so the offence is then at or after the slot of the activation block. The registration floor is `oldest_bond_slot`, the smallest stamped slot of the Bond UTXOs owned by the accused's bond owner; apply stamps every Bond UTXO with the slot of its block on both UTXO backends, so for the registration bonds it is the slot of the registration block. With no locked collateral, an offence slot below it, or no Bond UTXO at all, refuses. `slash_targets` reads the floor in the same bounded owner lookup as `locked_collateral`, so no new field is stored and the state root does not change. Punishment is forward-only, and an offence in the registration block's own slot is slashable. Refusal texts: `slash evidence expired: offence older than the unbonding period` and `slash evidence predates the accused's current registration` (`specs/protocol.md` §3.10.4).

**Gate-bound caveat**: the gate bound is sound, not complete. When the slots missed since the activation block are more than the offence's distance after it, an offence after the activation block is refused. Example: activation block at slot 1000, dense blocks to slot 1010, offence at slot 1010, then 20 blocks over 40 slots: age 41 > 31 blocks since activation, so the slash is refused. The effect is a missed punishment, never a wrongful burn, and the bound stops binding once the age limit (`unbonding_period`) is below the blocks since activation. The registration floor has no such caveat: it compares slots with slots. No slash is valid in the activation block itself, because the tenure there is 0: the floor block is then the judged block, and its slot can be above the parent slot + 1. The floor block's header is not read: a snap-synced node may not hold it, and the verdict must not depend on the node. The Bond UTXO slots are consensus state on every node.

**Pin order (INC-I-262)**: `effective_slashing_activation_height()` must be at or above `inc_i_257_unbonding_activation_height` on every network. Below the unbonding gate a RequestWithdrawal pays at once, so no locked collateral exists and the waiver of the registration floor protects nothing. A unit test over the mainnet, testnet and devnet defaults locks this order (`inc_i_262_m2_pin_order`).

**Terminal Slashed (AUDIT-P2-546)**: a slash moves the accused to `Slashed` from any status (Active, Unbonding or Exited). A `Slashed` key can never register again: from the effective height, a Registration or AddBond naming a Slashed or slash-queued key is refused with `[ERRTX071]` on mempool admission, revalidation, the builder, apply and replay (`specs/protocol.md` §3.10.4). A second slash for one offence is refused (`slash target is already slashed`, or `slash target already has a pending slash` while the first one is queued). Evidence against an old registration cannot slash a new one once the old Bond UTXOs are gone and the old collateral has unlocked: the floor is then the slot of the new registration's Bond UTXOs. While the old collateral is still locked the floor is waived, so a proof from the old registration stays slashable until it expires.

**Reporter field**: `SlashData.reporter_signature` must be the all-zero signature at and above the gate. It is outside consensus: no node verifies or records a reporter, and no reporter gets a reward. A third party therefore cannot make a second transaction id for the same evidence.

**Detector admission**: at and above the gate the detector stores and pairs only headers that pass the per-header evidence checks (rules 4 and 5 and the VDF, `evidence_header_admissible`), and it makes a proof only from a pair whose canonical SlashData fits 64 KiB. It keeps the smaller-encoded header per (producer, slot). Thus a decoy header from the accused (foreign genesis, bad VDF, or oversize) cannot block the valid pair. The detector has no "reported" latch; the handler's re-offer ledger is the only one. Cost: before a proof, at most one VDF verification per distinct header; after it, at most 4 more per (producer, slot). There is no count cap before a proof, because the accused could spend it with decoys. Full rules: `specs/protocol.md` §3.10.6.

**Detector window**: the detector reads time from the node's wall-clock slot, never from the incoming block. It keeps headers for slots from wall slot − 360 to wall slot + `MAX_FUTURE_SLOTS` (1). A header past that end skips the detector only; the block is not dropped. At the first block judged at or above the effective height, the detector drops every stored header once, so headers stored under the admit-all rule below the gate are learned again under the gate rules. Residual: a node more than 360 slots behind wall time does not detect double production in those old blocks. The clear runs once; a deep reorg back below the gate does not run it again.

**Flood bounds**: at most `MAX_SLASH_TXS_PER_BLOCK` (8) slashes per block and one per accused. The mempool keeps one resident slash per accused. A slash that fails these rules is refused where a node first sees it, and that node does not relay it.

**Slash delivery**: at and above the effective height a slash admitted fee-exempt is never evicted for room and never expires by mempool age. Revalidation still evicts it when `validate_slash_target` refuses it, for example when its evidence window closes. When the producer-set lock is busy, admission refuses the slash with the transient `Busy`; the same transaction is admissible once the lock is free. The node that created a slash keeps it in a bounded ledger (1,024 accused, not persisted) and re-offers the same transaction from a periodic tick until admission refuses it for good. While the accused is Slashed or slash-queued the tick skips the entry but keeps it, so a rollback or reorg that removes the slash before the boundary gets the same transaction re-offered. Residual: a node does not re-offer a slash that a peer created. Full rules: `specs/protocol.md` §3.10.4.

**Why 100% Burn?**
- Invalid blocks are NOT slashed - the natural penalty (losing the slot and its reward) is sufficient
- Following Bitcoin's philosophy: the network rejects bad blocks, you wasted your time, end of story
- Double production is unambiguously intentional fraud

**Why Burned, Not Redistributed?**
- Prevents incentivizing false accusations
- Eliminates collusion between accusers and validators
- Removes gaming opportunities from slashing mechanism
- Deflationary: at and above the effective height, a mined slash reduces total supply by exactly the burned amount. Below it nothing is burned.

**Exclusion point**: apply burns the bonds in the slash block. The producer-set change (status Slashed, delegations removed) applies at the next epoch boundary, or at the next block in epoch 0. From the slash block on, a RequestWithdrawal by the accused is refused. A block that holds the slash cannot also hold a RequestWithdrawal that names the accused, a Registration of the accused's key or an AddBond that names the accused (`[ERRTX071]`). It also cannot hold any transaction, of any type, that spends a Bond UTXO owned by the accused, whether that UTXO is in the pre-block set or was created earlier in the same block. These rules hold in either order and in Full, Light and Replay. The bond-spend rule keys on the spent UTXO, not on the transaction type, because below the unbonding gate any type can spend a Bond UTXO once its `lock_until` has passed, and a Bond output with a finite `lock_until` is still accepted at creation (AUDIT-P1-003). The builder keeps the slash and drops the withdrawal, Registration, AddBond or spend. The mempool evicts or refuses it. A spend mined in an earlier block than the slash is not reversed: the burn takes only the bonds that are still live. After the flush, the producer has no attester weight. A rollback or reorg of the slash block restores the bonds and the producer set.

**Failed-apply atomicity**: a block that fails to apply leaves the ProducerSet, its pending updates and the producer part of the state root exactly as on a node that never saw the block. When `apply_block` returns an error after its transaction loop starts, the node restores the in-memory set from the snapshot it took before the loop (`bins/node/src/node/apply_block/producer_undo.rs`). No apply error comes after the block batch commits, so disk is not changed either. A failed block that ran AddMaintainer or RemoveMaintainer also gets its pre-block maintainer set put back, in memory and in `maintainer_state.bin`. This holds on every path that calls `apply_block`. It is node-local, not a consensus rule.

**Cross-block withdrawal escape (AUDIT-P2-540, INC-I-257)**: below `inc_i_257_unbonding_activation_height` a RequestWithdrawal pays a Normal output that is spendable in the next block. A producer that has signed two blocks for one slot can withdraw its bonds before the slash is mined. The withdrawn value leaves the slashable set at once, the later slash finds no live Bond to burn, and the producer keeps the funds. An accused that the withdrawal has already moved to `Exited` stays `Exited`. At and above the gate the payout is one Unbonding output owned by the producer's own address and locked for at least `unbonding_period` blocks (mainnet 60,480, testnet 72, devnet 60). No transaction type can spend it before `lock_until`, and at and above the gate only RequestWithdrawal can spend a Bond (`specs/protocol.md` §3.13.1). The value therefore stays at a known owner for the whole period. A slash mined at height H burns every UTXO of the accused's bond owner that is slash-burnable at H (`is_slash_burnable`): each Bond, and each Unbonding output with H < `lock_until`. Burnable is the exact opposite of spendable for Unbonding, so the burn never races a legal spend. Supply drops by the burned amount, and rollback restores the burned entries with their type and `lock_until`. An accused that the withdrawal has moved to `Exited` stays slashable while it holds locked collateral; it stays `Exited` after the burn. Once every Unbonding output has unlocked (H >= `lock_until`), a slash of an exited accused is refused at admission, never selected by the builder, and makes a block invalid. Mempool (judged at tip + 1), builder (at the height built) and block validation (at the block height, against the parent UTXO set) read the same population function, `mempool::slash_admission::slash_targets`. It does at most one owner lookup per accused and at most `MAX_SLASH_TXS_PER_BLOCK` per call; an accused past that budget counts as holding no collateral. The mempool skips the owner lookup for an accused already Slashed or slash-queued; the verdict is the same. The gate is `806859` on mainnet, `330849` on testnet and `0` on devnet.

**Bond-output rule (AUDIT-P1-003, INC-I-257)**: at and above `inc_i_257_unbonding_activation_height`, a Bond output may appear only in a Registration or an AddBond. Each one must be owned by `bond_owner(producer)` (the key named in the registration or add-bond data) and must be worth exactly one network bond unit (`Network::bond_unit()`, the unit the producer set counts). Any other transaction type with a Bond output is refused, and so is a foreign-owner or non-unit Bond. Thus every Bond that adds selection weight is one the slash of that producer can find. `lock_until` is not constrained by this rule. Genesis bonds are written straight to the UTXO set and do not pass through transaction validation. Below the gate the rule does not apply.

#### 4.2.1 Delegator Slashing — Tezos LPoS Equivalence (INC-I-078)

**DOLI delegator principal is never slashed.** Equivocation slashing
burns only the offending producer's self-bond. This is a structural
consequence of DOLI's UTXO ownership invariant (CS1: only the
delegator's Ed25519 key can spend their bond UTXOs) — a
consensus-forced UTXO destruction for delegated bonds would be the
first and only exception to this invariant.

When `slash_producer` runs, `cleanup_all_delegations` (in
`crates/storage/src/producer/set_lifecycle.rs`) returns every
delegator's principal to them — they lose only the *weight*
contribution they had assigned to the slashed producer. The burn
(`bins/node/src/node/apply_block/slash_burn.rs`) selects the accused's
slash-burnable UTXOs (Bonds, and Unbonding outputs still locked) by
owner. It burns the bonds the accused owns, also those it delegated to
another producer. It never burns a delegator's own bonds.

This is functionally equivalent to **Tezos LPoS slashing** ("the
delegate alone is slashed for the validator's misbehavior" — Tezos
protocol manual, Adaptive Issuance & Liquid PoS). Polkadot-NPoS /
Cosmos-SDK style proportional burning of delegator principal does NOT
port to DOLI and is removed from the delegation-mitigation menu
(`specs/delegation-architecture.md` §7.2.1).

This is an INTENTIONAL design, not a gap. Producers and delegators
should be aware that delegating amplifies the *governance* and
*reward* attack surface (your weight contribution can be misused) but
not the *capital* attack surface (your principal cannot be burned by a
producer's misbehavior).

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

### 4.5 Withdrawal Payout Bound — Vesting Penalty (INC-I-171)

**Risk**: the bond vesting penalty (Y1 75% / Y2 50% / Y3 25% / Y4+ 0%) is
subtracted by the CLI when it builds a `RequestWithdrawal`, and nothing above
the transaction's structural checks ever re-derived it. A patched client can
emit a `RequestWithdrawal` whose single `Normal` output pays **100% of the Bond
UTXO inputs** at any bond age; the block is accepted and the penalty is never
burned. The whole four-year commitment is unenforced against an adversary who
does not run the official binary.

**Defense — the payout bound** (`crates/core/src/validation/vesting.rs`):
`check_withdrawal_payout_bound` rejects a block whose `RequestWithdrawal` pays
more than `Σ penalized_bond_net(spent Bond inputs) + Σ non-Bond input value`,
error code `ECON_WITHDRAWAL_PAYOUT_EXCEEDS_NET`. Bond age is
`block.header.slot − creation_slot` from the **node-stamped** `extra_data`, so
the tier is a function of the block under validation and not of the submitter's
claim, the chain tip, or a wall clock. Paying less is allowed — the rule is a
ceiling, so it can never reject an honest withdrawal.

**Fail-closed**: a Bond input whose `extra_data` does not decode to a
`creation_slot` is refused with `ECON_WITHDRAWAL_BOND_EXTRA_DATA_MALFORMED`
rather than defaulted. An unknown age must never read as fully vested, which is
the direction that would pay 100%. A `vesting_quarter_slots` outside
`1..=u32::MAX` is refused with `ECON_VESTING_QUARTER_INVALID`, and the
activation gate is evaluated **before** that config check so a dormant rule
cannot turn one node's misconfiguration into block rejections at every height.

**Three-site parity**: mempool admission (`crates/mempool/src/vesting_bound.rs`),
the block builder (`bins/node/src/node/production/withdrawal_holdings.rs`) and
block validation (`bins/node/src/node/validation_checks/withdrawal_economics.rs`)
all reach their verdict through the single core predicate above. A rule enforced
only at validation would turn an ordinary user mistake into free block poison —
the transaction never confirms, so no fee is paid and no input is spent, yet
every producer that selects it burns a block build. Admission additionally
evaluates at a **non-regressing slot watermark** and never evicts on a vesting
error, so a reorg cannot censor an honest in-flight withdrawal.

**Residual risk**: `inc_i_171_vesting_penalty_activation_height` is `u64::MAX`
on mainnet, testnet and devnet
(`crates/core/src/network_params/defaults.rs:276,526,785`). **Nothing above is
enforced today.** Until a height is pinned, the penalty remains a client-side
convention and the attack described under *Risk* is live. Full-mode nodes
already count the verdict they *would* reach in
`doli_vesting_would_reject_total{code}`, which is evidence for choosing that
height but is not itself a defense. Pinning it changes consensus rules and is a
separate decision session.

**Signer binding (INC-I-182)**: from `inc_i_182_producer_auth_activation_height`, a
RequestWithdrawal also needs the named producer's own signature, so no third party
can force a producer's full exit (`specs/protocol.md` §3.13.2).

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
# Epoch boundary (crates/core/src/epoch_state/mod.rs, derive_at_boundary):
producer_list = active producers that attested in any of the last 3 epochs
                (liveness filter + deadlock floor), sorted by pubkey
if len(producer_list) > ACTIVE_PRODUCERS_CAP (50):
    candidates  = producers with >= MIN_ATTESTATION_MINUTES (30) attested minutes
                  in the just-completed epoch (before inc_i_193_attestor_refill_activation_height:
                  also >= 80% of expected blocks produced — the INC-I-193 shrink-only defect;
                  after: attestation minutes only)
    active_list = first 50 of candidates sorted by registered_at asc (pubkey tiebreak)
    if len(active_list) < len(producer_list) / 3 or active_list is empty:
        active_list = producer_list          # deadlock safety
else:
    active_list = producer_list

# Per slot (production/scheduling.rs and validation/producer.rs, same formula):
selected = active_list[slot % len(active_list)]
```

**Security Properties**:
- **Independent of prev_hash**: Selection uses `active_list[slot % len]`, NOT hash-based lottery. This prevents grinding attacks entirely (Epoch Lookahead).
- **Deterministic**: All honest nodes compute same result for any slot
- **Equal**: Every producer in `active_list` holds exactly one position per rotation, regardless of bond count. Bond count never enters slot assignment.
- **Unbiasable**: Attacker cannot influence future selection by manipulating block content
- **Sybil resistance for production membership**: a position in `active_list` is earned by registration seniority + liveness, not by bond count. Bonds back registration cost, epoch reward share, and slashing.
- **Empty slots**: an offline producer leaves its slot empty; the next slot goes to the next producer (INC-I-026 trade-off — no fallback rank).

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
│  Cap: MAX_TRACKED_ENTRIES = 10,000, window 360 slots            │
│  Pending proofs: at most 64; one proof per (producer, slot)     │
│  Data Stored: Full BlockHeader (not just hash)                  │
│  Purpose: Carry the producer signatures into slashing evidence  │
└─────────────────────────────────────────────────────────────────┘
```

**Why Full Headers?**
- Each header carries the producer block signature (INC-I-236)
- Validators verify both signatures against the accused key, so nobody can forge evidence
- Evidence is cryptographically verifiable by any node

**Detection Flow**:
1. Block received → at and above the gate, drop the header from the detector unless it passes the per-header evidence checks (§4.2 "Detector admission")
2. If `(producer, slot)` holds a different admitted header and the pair fits 64 KiB → **EQUIVOCATION DETECTED**; the smaller header stays stored
3. Create `EquivocationProof` with both full `BlockHeader`s
4. Below the effective slashing height (§4.2): log the proof only. No `SlashProducer`
   transaction is created, pooled or broadcast.
5. At and above it: the handler (`bins/node/src/node/slash_submit.rs`) builds the one
   canonical slash and submits it through mempool admission. It relays the slash only
   after admission, and only when the accused has no resident slash and was not
   reported before (reported set bounded at 1,024 keys).

**Slashing Evidence Structure**:
```rust
SlashingEvidence::DoubleProduction {
    block_header_1: BlockHeader,  // Full header, signed by the producer
    block_header_2: BlockHeader,  // Full header, signed by the producer
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
| Transactions (one token per tx; a 0xBA batch of n txs costs n) | 500/second, burst 2,000 | 1,000/second, burst 4,000 |
| Requests (votes, producer gossip) | 20/second, burst 200 | - |
| Attestations (own bucket, INC-I-237 M5 decision C) | 20/second, burst 200 | - |
| Bandwidth | 1 MB/second | 10 MB/second |

**Token Bucket Algorithm**:
- Capacity: Burst allowance (e.g., 10 blocks for per-peer block limit)
- Refill Rate: Tokens per second (e.g., 10/60 = 0.167 blocks/sec)
- Check before accept: `can_consume(amount)` returns false if bucket empty
- Record after accept: `try_consume(amount)` deducts tokens
- Own-origin RPC txs draw on `Node.origin_tx_bucket` (500/second, burst 2,000) before a command slot is reserved; an empty bucket answers Busy (INC-I-237 AUDIT-P2-511)

**Features**:
- Per-peer tracking via `HashMap<PeerId, PeerLimits>`
- Global limits prevent aggregate flooding
- Stale peer cleanup (max 1000 tracked peers)
- Can be disabled for testing via config


#### 6.1.3a Gossip Event Queue Load-Shedding (INC-I-114)

Gossip block floods can back-pressure the event channel, suspending the swarm
task and causing libp2p's internal message buffer to grow unboundedly (7.44 GB
in the INC-I-114 incident). Defense in depth:

1. **Staleness filter** (cc1b9c77): `validate_messages=true` + `classify_block_gossip()`
   suppresses re-forwarding of stale blocks at the gossipsub validation layer.
2. **Load-shedding enqueue** (M1): accepted blocks use `try_send` (non-blocking)
   instead of `.send().await`. When the bounded event channel is full, the block
   is dropped and counted by `GossipShedMetrics`. The swarm task never suspends,
   so libp2p drains its internal queue normally.
3. All gossip-topic event sends are load-shed into two bounded lanes (consensus
   lane: blocks, attestations, control; bulk lane: txs, votes, producers), each
   bounded by count and bytes. Control events (peer, status, sync) that do not
   fit use a FIFO backlog (8,192 events; status/sync also 64 MiB), then are
   dropped and counted (F-12, open). The swarm task never awaits event delivery,
   so it cannot deadlock against the node loop's command send (INC-I-237).
4. Dropped blocks are recoverable via sync protocol (GetBlocks request-response).
5. **Memory watchdog** (M2): a periodic sampler (5s interval) reads process RSS
   via `/proc/self/statm` (Linux) and trips a shared `AtomicBool` when RSS
   crosses a configurable soft threshold (`DOLI_MEMORY_WATCHDOG_BYTES`). When
   tripped, the gossip hot path sheds ALL accepted blocks (after
   `report_message_validation_result`, before enqueue) until memory recovers.
   Fail-open: on non-Linux or if the sampler is unavailable, the flag stays
   false (never sheds). Default: disabled (threshold=0).


**Implementation**: `crates/network/src/service/backpressure.rs` (metrics + enqueue_or_shed), `crates/network/src/service/event_sink.rs` (non-blocking event delivery), `crates/network/src/watchdog.rs` (memory watchdog).

**Gossip relay authority (INC-I-237).** The swarm forwards only one topic before the node judges it: a decodable, non-stale block (`Accept`; undecodable block bytes are the only `Reject`). For every other subscribed topic the NODE is the relay authority: the swarm defers the message, the vendor mcache holds it un-forwarded, and the node's verdict (`Accept | Ignore`, never `Reject`) is reported once on the original message id within 3.5 s. A late verdict is counted (`doli_gossip_verdict_late_total{topic}`) and never reported; a verdict the command channel cannot take is counted (`doli_gossip_verdict_dropped_total{topic}`); the message then expires un-forwarded. A tx is forwarded only by a hop that newly admitted every tx in the message; the node attempts each distinct tx in a message once, keyed on the full serialized tx bytes (INV-NETWORK-004), and a repeat makes the verdict `Ignore`. Zero-flow admission authentication (`crates/mempool/src/zero_flow_auth.rs`, run by `Mempool::check_system_transaction` inside `add_system_transaction` on all 3 admission paths: gossip `handle_new_transaction`, RPC `sendTransaction`, RPC `submitMaintainerChange`) refuses a DelegateBond or RevokeDelegation without a valid delegator signature and an AddMaintainer or RemoveMaintainer without the current maintainer-set multisig, as apply does; a refused tx is not admitted and not forwarded, and a genuine one relays multi-hop. Any other zero-flow tx (e.g. a genesis-window Registration) is never forwarded. A keyless rewrite of a captured maintainer change (`reason`, signature-entry order or padding, trailing bytes; AUDIT-P1-509) buys no relay: newness is keyed on the signed content `(action, target)`, the change must still have an effect on the current set, the payload must be canonical with no repeated signature entry, and at most `MAX_MAINTAINERS` changes per action are resident. A delegation tx signed by a throwaway or unqualified key buys no relay (AUDIT-P1-515): admission refuses a DelegateBond or RevokeDelegation that the next epoch boundary would turn into a no-op (the `delegate_bonds_capped` / `revoke_delegation` preconditions on the applied ProducerSet plus the delegator's queued updates; no producer-set view refuses; `MPTX016`), and keeps at most one resident delegation tx per delegator. The INC-I-246 admission stop-gap refuses a RevokeDelegation that names a delegate other than the effective one, a DelegateBond apply would refuse (received cap), and a DelegateBond already in a canonical block (`MPTX014`, DelegateBond only). Admission order on all 3 paths (`zero_flow_mined::admit_zero_flow`, AUDIT-P1-516): a DelegateBond takes every cheaper refusal first under the mempool read lock (residency, size, `validate_transaction`, signature, standing, stop-gap, newness); then the mined check with no mempool lock held, so its block body read never blocks block production; then insertion under the write lock, which re-runs every check. Registration, RevokeDelegation, AddMaintainer and RemoveMaintainer take no read-lock phase and no mined check: they go straight to insertion under the write lock, which runs the same cheaper refusals (canonical payload and effect, signature or multisig, standing, stop-gap, newness) first. `MPTX014` is checked after every other refusal; repeated copies hit a bounded cache (10,000 txids, each pinned to its carrying block and re-checked with one canonical height-index point get, no body read; memory only). A forged or no-effect copy therefore never costs a block read, and a mined copy costs one read per distinct txid per cache lifetime. One registered key can mint an unlimited set of distinct mined DelegateBonds (REL-574-R1-01), so the body reads draw on one per-node budget (M12.3): 2 reads/s with burst 2, memory only, full at start. A cache hit and a tx-index miss (every honest new DelegateBond) spend nothing; a tx-index hit with the budget spent is refused `MPTX017` (retry) with no read and no cache entry, `Ignore` on gossip. The mined check therefore reads at most about 2 block bodies per second, whatever the corpus. These checks are admission-only, outside `crates/core/src/validation/`, so they are not a consensus rule; a producer can still mine a replayed delegation tx until the INC-I-246 consensus fix (signed nonce / expiry / chain binding, `RevokeDelegation.delegate` checked at apply) ships at an activation height. An attestation is forwarded only after its Ed25519 signature and the attester's ProducerSet membership verify, and only for the first copy of its `(attester, slot)` or the first copy with a valid BLS half for it (`AttestSeen`; at most 2 forwards per `(attester, slot)`). When the attested block is unknown, a not-first copy gets a relay-only BLS pairing check that does not touch pooling or finality. `AttestSeen` is pruned against the wall clock and holds at most attesting members × 15 entries (12 stale + 2 future slots + 1). A vote is forwarded only for a verified active producer; a producer batch only when the GSet merge added a verified entry. The swarm Ignores an attestation more than 2 slots ahead of the wall-clock slot. Per-peer buckets charge a tx message one token per tx (500 tx/s, burst 2,000, plus the global tx bucket), so a message of more than 2,000 txs never passes; attestations have their own bucket (20/s, burst 200), separate from the request bucket that votes and producer batches use. An attestation is shed before it can take the consensus-lane headroom kept for blocks. The node never re-publishes received content. Rules: `specs/protocol.md` §7.2; design: `specs/gossip-relay-verdict-architecture.md`.

- **INV-NETWORK-004** is carried by the vendor message id: BLAKE3 over the FULL raw message bytes, kept 300 s by the gossipsub duplicate cache (`GOSSIP_DUPLICATE_CACHE_TIME`). Two vendor time caches have a count cap with oldest-first eviction (TTL unchanged): the duplicate cache at 218,000 ids (`GOSSIP_DUPLICATE_CACHE_MAX_ENTRIES`) and the peer-score deliveries cache at 50,000 records (`PEER_SCORE_DELIVERIES_MAX_ENTRIES`), so a distinct-id flood cannot grow them without bound; an id evicted early is judged again like a first copy. The cap makes the duplicate-cache retention min(TTL, cap ÷ distinct-id ingress), not a fixed 180 s floor: 180 s holds only up to 1,211 ids/s, 12 slots (120 s) up to 1,816 ids/s, the 60 s block window up to 3,633 ids/s (120.8 s at the measured 1,804 ids/s soak ingress). The ingress is attacker-paced: a distinct-id flood can evict a genuine tip-block id, and the block re-forward that follows is not bucket-bounded (open item "block re-forward cliff", REL-573-R-01, owed before mainnet). Attestation newness (`AttestSeen`, M6) and tx newness (D5 txid residency, M5/M8/M11) do not depend on this cache. The only application-level relay-newness state is `AttestSeen`, keyed on `(attester, slot)` only after the Ed25519 signature and membership verify (authenticated content), so a forged copy cannot pre-seed it. Mempool residency keyed on the signature-excluding `tx.hash()` never authorizes a forward.
- **INC-I-238 (OPEN, pre-existing):** an attestation's Ed25519 signature covers only `(block_hash, slot)`. A copy of a genuine member-signed attestation with changed unsigned fields (`height`, `attester_weight`, `bls_signature`) is a new message id. Since M6 the relay half is closed: `AttestSeen` forwards at most 2 copies per `(attester, slot)`, and the per-peer attestation bucket bounds the rest; the wire half (the signature scope) stays open. Votes have the same shape through their JSON form and hex case; vote variants stay open (AUDIT-P1-507, INC-I-239), bounded by the per-peer request bucket.

**INV-NETWORK-002** (construction-time invariant, Level 3): Any aggressive gossip-propagation setting (`flood_publish=true` and/or `duplicate_cache_time <= 30s`) REQUIRES `validate_messages=true` AND bounded (load-shedding) event lanes. The invariant is enforced at network-service construction via `assert_production_hardening()` → `assert_gossip_hardening_invariant()`, whose queue half reads the real lane budgets (`LaneBudgets::is_bounded()`) — the node fails to start if either mitigation half is missing. Incident lineage: INC-I-009, INC-I-014, INC-I-118, INC-I-120, INC-I-114 (5 occurrences of the same unbounded-queue + aggressive-propagation shape). Implementation: `crates/network/src/gossip/config.rs` (`assert_gossip_hardening_invariant`, `AGGRESSIVE_DEDUP_THRESHOLD`). 7 gate tests cover all branch arms.

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
    pub bls_pubkey: Vec<u8>,           // BLS12-381 public key (48 bytes, MANDATORY)
    pub bls_pop: Vec<u8>,              // BLS proof-of-possession (96 bytes, MANDATORY)
}
```

**BLS keys are mandatory, not optional.** Both registration paths reject an empty `bls_pubkey`
and an empty `bls_pop`, then run `validate_bls_pop` on the pair
(`crates/core/src/validation/registration.rs:46-57` and `:143-154`). A producer cannot register
without a proof-of-possession-backed BLS key.

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
1. **Epoch Lookahead**: Selection uses `active_list[slot % len]`, independent of `prev_hash`
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

### 6.8 Staged Upgrade Trust Boundary

`{data_dir}/updates/` is a **trust boundary**. The unprivileged node process writes it; a
root process reads it and installs from it. The two are different principals, so nothing
in that directory is trusted on sight.

| Side | Principal | Role |
|------|-----------|------|
| `bins/node/src/updater/staged_apply.rs` | node service account, sandboxed unit | **Untrusted writer.** Stages a release it verified but cannot install. |
| `bins/cli/src/cmd_upgrade_staged.rs` (`doli upgrade --from-staged`) | root, `Type=oneshot` unit | **Verifying consumer.** Re-derives every conclusion from the bytes on disk. |

**Verification runs inside the root process.** The node's verification is not inherited.
`doli upgrade --from-staged` resolves the trust root from this host's
`{data_dir}/maintainer_state.bin` — a decode failure ABORTS and never falls back to the
compiled bootstrap keys — and re-runs the full **L1–L4** gate over the staged bytes:

```text
  L1  SIGNATURES.json version   == the version the `ready` marker names
  L2  sf.checksums_sha256       == sha256(the staged CHECKSUMS.txt)
  L3  threshold DISTINCT maintainer signatures over "{version}:{checksums_sha256}"
  L4  sha256(release.tar.gz)    == the per-platform hash parsed from THAT CHECKSUMS.txt
```

The threshold is read from `TrustRoot::threshold()`, never hard-coded: a 5-member on-chain
maintainer set requires **3-of-5** distinct signers, a 4-member set 3, and an empty or
sub-threshold root is not usable at all (fail-closed). A root that holds one of the 5 retired
mainnet genesis keys (`RETIRED_COMPROMISED_MAINTAINER_KEYS`) is not usable either (INC-I-248).
L3 counts DISTINCT signers, so one key signing repeatedly cannot reach the threshold.

**The staged operand is the signed tarball, never an extracted binary.** The signature
chain terminates at `release.tar.gz`; an extracted ELF is covered by nothing and would
leave the root side with no verifiable operand. Root extracts inside its own process, after
L4 passes.

**The `ExecStart` is fixed and never interpolates staged content.**
`{service}-upgrade.service` is rendered once at `doli service install` time from the
resolved service name, network, data directory and CLI path. Its `ExecStart` is a literal
`<cli> --network <net> upgrade --from-staged <dir> --data-dir <dir> --service <unit> --yes`.
Nothing an attacker can write into `{data_dir}/updates/` reaches the command line, so a
hostile staging directory cannot change what root executes — only what root refuses. The
`.path` unit matches only `PathExists=` on the `ready` marker.

**Additional bounds on the root side:**

- `ready` is created only by rename, after every other staged file is fsynced and renamed
  into place. A watcher that fires on it never observes a partial handoff.
- A `ready` whose version disagrees with `SIGNATURES.json` is refused before any signature
  work.
- A staged version that is not newer than the installed one is refused, so a stale staging
  directory cannot reinstall an older build.
- A refusal installs nothing: the existing binaries stay byte-identical, and the marker is
  renamed to **`ready.rejected`** — evidence is preserved, the trigger is disarmed. A
  sub-threshold, tampered or replayed staging directory therefore parks as
  `ready.rejected` and never becomes an install.

**Residual exposure.** Anyone who can write `{data_dir}/updates/` can force root to spend
one verification pass per write. They cannot cause an install: the write is not a
signature, and the threshold is enforced on the reading side.

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

DOLI's producer selection uses `active_list[slot % len(active_list)]` (deterministic round-robin over the epoch-frozen active producer list, one position per producer), which is completely independent of `prev_hash`. This eliminates grinding attacks entirely:

```
selected_producer(slot) = active_list[slot % len(active_list)]
```

The selection depends ONLY on:
1. The slot number (deterministic from wall-clock time)
2. The active producer list (frozen at epoch boundary, see 5.2 — bond counts are not an input)

Since `prev_hash` is not used in selection, manipulating block content cannot influence who produces future blocks. This is a fundamental design choice -- Epoch Lookahead trades theoretical unpredictability for grinding immunity.

**prev_hash in VDF input**: The VDF input includes `prev_hash`, but this only affects VDF output validity (anti-pre-computation), NOT producer selection. An attacker cannot pre-compute VDF for a future block because they do not know `prev_hash` until the previous block is finalized.

**Residual risk**: The only remaining vector is manipulating the producer set itself (registering/deregistering at epoch boundaries to shift slot assignments). This is mitigated by:
1. Registration requires VDF computation + bond
2. Bond withdrawals incur vesting penalties
3. Producer set changes are deferred to epoch boundaries
4. One position per producer regardless of bond count; entering the capped list requires registration seniority + liveness (5.2)

**Status**: Grinding is a non-issue due to Epoch Lookahead. No monitoring needed for this specific attack vector.

*Updated: 2026-03-29 -- rewrote to reflect actual deterministic selection (was describing obsolete hash-based selection)*

### 7.6 Catastrophic Consensus Failure & Recovery (Canonical Anchors)

**Risk**: A critical consensus bug (e.g. the INC-I-026 cascade of 2026-04-09) produces a hostile chain that a seed-guardian recovery procedure cannot reliably eject, because seeds passively accept the heaviest chain and any surviving hostile producer can re-poison a freshly-restored seed within seconds.

**Defense: Canonical anchors** (`crates/updater/src/anchor.rs`)

A **canonical anchor** pins a `(height, block_hash, state_root)` tuple at compile time via `updater::AnchorSchedule::for_network(network)`. The schedule mirrors `HardForkSchedule` structurally: an append-only, compile-time, network-keyed list of commitments baked into the binary. Each anchor also carries a `min_version` that forces old binaries off the network at the anchor height, identical to `HardForkInfo::min_version` semantics.

**Enforcement points** (all no-op when the schedule is empty; current state for all networks):

1. **Block validation (Point A)** — `validate_block_for_apply` in `bins/node/src/node/validation_checks/mod.rs` rejects any block whose `(height, hash)` pair contradicts an anchor, emitting `ValidationError::CanonicalAnchorViolation`. Runs BEFORE VDF/producer checks so hostile blocks cost zero CPU.
2. **Reorg gate (Point B)** — `ReorgHandler::check_reorg_weighted` and `ReorgHandler::plan_reorg` in `crates/network/src/sync/reorg/mod.rs` reject any reorg whose common ancestor sits strictly below the highest anchor, regardless of accumulated chain weight.
3. **Snap sync verification (Point C)** — `handle_snap_snapshot` in `crates/network/src/sync/manager/snap_sync.rs` rejects snapshots at or below the anchor height that don't match the anchor's `block_hash` and `state_root`, and rejects any snapshot whose height is strictly below the anchor (stale by construction).
4. **Rollback refuse (Point D)** — `rollback_one_block` in `bins/node/src/node/rollback.rs` returns an error if a rollback would cross the anchor height, preventing corruption of the anchored state.

**Rules (enforced by review, not code)**:
- Anchors are append-only. Once shipped in a release, an anchor is never removed or modified — only superseded by a newer anchor at a higher height.
- Anchors must be derived from a genuinely healthy guardian checkpoint, confirmed by at least 2 independent seed operators emitting identical anchor proposals.
- A testnet cycle MUST precede any real mainnet anchor release.
- Each anchor PR requires two reviewers.

**What canonical anchors do NOT replace**:
- The guardian fork detection + halt loop (see `.claude/skills/guardian/SKILL.md`)
- The `--auto-checkpoint` snapshot cadence on seeds
- The hard fork schedule in `crates/updater/src/hardfork.rs`

### 7.7 DeFi Safety Gate — Phase 0 (INC-I-088)

**Risk**: 11 DeFi transaction types — `CreatePool`, `AddLiquidity`,
`RemoveLiquidity`, `Swap`, `CreateLoan`, `RepayLoan`, `LiquidateLoan`,
`LendingDeposit`, `LendingWithdraw`, `FractionalizeNft`, `RedeemNft` — were
accepted by `validate_transaction` from genesis with no activation gate. Any
user could submit them via raw RPC at any height. Several have known
semantic gaps that the structural validators do not catch:

- `validate_liquidate_loan` performs two checks (≥ 1 input, ≥ 1 output) and
  delegates everything else to apply_block. The lending subsystem still has
  no oracle consumer in core (Phase 2.3 design will wire the OraclePrice UTXO
  in); any actor with the liquidation TX format could drain undercollateralized
  loans. Phase 2.1 ships the oracle PROVIDER side (TxType 16 PriceAttestation +
  OutputType 15 OraclePrice + bond-weighted median aggregator + 55% structural-
  share sunset) — gated by `oracle_activation_height = u64::MAX` until a
  future binary activates it. See `specs/oracle-structural-anchored-economics.md`.
- `validate_create_loan` does not pin `outputs[0].pubkey_hash` to the
  derived loan address — a borrower could put their own pubkey_hash there
  and later spend the `Collateral` UTXO with a single signature.

Compounding risk: `OutputType::Collateral` was not in `is_conditioned()`
(`crates/core/src/transaction/types.rs`), so the spend path used the
single-signature branch of `verify_input_conditions`.

**Defense — DeFi activation gate** (`crates/core/src/validation/transaction.rs`):
A `defi_activation_height: u64` field on `NetworkParams` and
`ValidationContext` (default `u64::MAX` on all networks) gates the 11 tx
types behind one consolidated typed error:

```
ValidationError::DefiNotActivated {
    tx_type: u32,            // TxType discriminant
    activation_height: u64,
    current_height: u64,
}
// error_code() == "DEFI_NOT_ACTIVATED"
```

Comparison is strict `<` — at `current_height == defi_activation_height`
the gate opens. Pre-activation, every validator (block-apply, mempool
admission, block-assembly) rejects DeFi txs identically. Cross-version
deploy safety: mempool symmetry guarantees an upgraded producer never
includes a DeFi tx in its block during a rolling deploy. An old producer
could still produce a block containing a DeFi tx that an upgraded validator
will reject — relied-upon assumption is that no DeFi traffic is in flight
on mainnet today (verified via pre-deploy UTXO audit).

**Defense — Collateral hard-freeze** (`crates/core/src/validation/utxo.rs::verify_input_conditions`):
Before evaluating any condition tree, `verify_input_conditions` rejects
every input whose UTXO has `OutputType::Collateral` with the stable error
code `[ERRTX-DEFI001]`. This is deterministic — it does not depend on the
`is_conditioned()` route, which is only probabilistically a freeze because
`CollateralMetadata` occasionally parses as a satisfiable condition
(`Signature(Hash::ZERO)` on the zero-pool_id construction, for example).
`OutputType::Collateral` is also added to `is_conditioned()` as secondary
documentation of intent. The two controls together fully freeze the
lending subsystem until it is properly audited and un-gated.

**Three-question gate verdict (INC-I-075)**:
- Q1 (user-submittable): YES — DeFi txs are user-originated.
- Q2 (producer-action): NO — validators reject; producers never include.
- Q3 (bit-identical to old behavior): NO — accept → reject for the same
  input.
Result: activation height REQUIRED. Compliant.

**Constraint compliance**:
- C0 (no genesis reset): satisfied — new field defaults to `u64::MAX`,
  forward-only activation.
- C1 (INC-I-054 immutable crossed heights): N/A — NEW field, never
  crossed.
- C2 (CURRENT_PROTOCOL_VERSION): NOT bumped — `EpochState` unchanged.
- C3 (HardForkSchedule entry): not added — constant gate, not consensus
  hardfork.
- C5 (INC-I-062 block content): rolling-deploy safe in the zero-DeFi-traffic
  state mainnet is in today.

**Residual risk**:
- If a real Collateral UTXO exists in mainnet today, the hard-freeze
  renders it unspendable until the DeFi subsystem is properly fixed.
  Pre-deploy UTXO audit (output types 7, 9, 10, 11, 12) is the operator's
  responsibility before flipping the gate.
- The freeze does NOT remove the underlying validator gaps; un-gating
  requires implementing the missing checks (oracle for `LiquidateLoan`,
  `pubkey_hash = loan_addr` for `CreateLoan`) as separate consensus
  changes, each with its own activation height.

---

### 7.8 Operator Recovery & Slashing Protection Preservation

**Risk**: The common operator response to a poisoned or forked producer is to
stop the node, wipe its `data/` directory, and restart (or rsync from a healthy
peer). This procedure, applied naively, destroys `signed_slots.db` — the local
record of every slot the producer has previously signed. A producer restarted
without its `signed_slots.db` has no memory of prior signatures, so if it is
scheduled for a slot it previously signed on the now-discarded chain it will
sign a *second* block for that slot. This is equivocation — the only slashable
offense in DOLI (see §6.1.1) — and re-exposes the producer's bond to slashing
even though the operator's intent was recovery.

A second, subtler risk exists when `utxo_store/` is copied from a different
source than `state_db/` (e.g. restored from a checkpoint while the old UTXO
cache remains on disk, or rsync'd from a node whose caches are at a different
height). Prior to v6.7.9 the startup code trusted `utxo_store/` blindly
whenever it was non-empty, operating on mismatched UTXOs while reporting the
correct height on RPC. This caused the silent-corruption rollback that killed
ai1/ai2 during the INC-I-026 recovery on 2026-04-09 (tracked as INC-I-027).

**Defense** (`bins/node/src/node/init.rs::init_utxo_set`, v6.7.9+):

`init_utxo_set` compares `utxo_store.len()` against `state_db.utxo_len()` at
startup. On mismatch it WARNs, clears `utxo_store/`, and rebuilds from
`state_db.iter_utxos()` — the authoritative loop used for empty stores. This
makes recovery procedures self-healing regardless of `utxo_store/` state.

**Defense** (operator procedure — `scripts/node-heal.sh`):

The `node-heal.sh` guardian script automates producer recovery with the
following invariants enforced in tooling rather than operator discipline:

1. **`signed_slots.db` is preserved across the wipe.** The script deletes
   every entry in `data/` *except* `signed_slots.db` and explicitly excludes
   it from the rsync so the source's (wrong-key) slashing history does not
   overwrite the target's protection. Operators must pass `--wipe-signed-slots`
   to override, and the script will refuse without a confirmation prompt.
2. **`utxo_store/` is excluded from the rsync.** The target rebuilds it from
   the fresh `state_db/` on startup via the v6.7.9 self-healing path,
   guaranteeing `utxo_store/` and `state_db/` share the same origin height.
3. **Stale runtime state is cleaned.** `producer.lock` and
   `pending_update.json` are deleted post-rsync — the former to prevent lock
   conflicts, the latter to avoid stale auto-update enforcement silently
   pausing production.
4. **Seeds are refused.** The script errors out on `--target seed` or any
   non-`nN`-pattern target name. Seeds require a different recovery procedure
   (checkpoint restore + full-fleet halt; see §7.6 and
   `.claude/skills/guardian/SKILL.md` §L2).

**Residual risk**:
- If an operator uses `--wipe-signed-slots`, the producer loses slashing
  protection and any scheduled re-sign of a historical slot will cause
  equivocation. This override exists only for fresh producers that have never
  signed on any chain.
- If the healthy source is itself on a hostile chain, `node-heal.sh` will
  faithfully replicate the hostile state onto the target. The script performs
  an RPC pre-check against the source's reported chain tip but cannot
  independently verify canonicity; this is the operator's judgement call.
  Canonical anchors (§7.6) close this gap at and below the anchor height.
- Manual recovery procedures documented in `docs/troubleshooting.md` §4.2 and
  §4.4 still exist as fallbacks for environments where `node-heal.sh` is
  unavailable. These fallbacks are annotated to preserve `signed_slots.db`
  and wipe `utxo_store/` alongside `state_db/` and `blocks/`, matching the
  script's invariants.

### 7.9 Attestation Aggregate — Conditional Guarantee (Post-Activation-Height Only)

**Risk**: The BLS aggregate is read as a general "the bitfield is honest" proof. It is neither
that, nor active on any network today: `inc_i_178_attestation_bls_activation_height` is `u64::MAX`
on mainnet, testnet and devnet (`crates/core/src/network_params/defaults.rs:268,510,763`). Below
that height no aggregate is built or verified, so nothing in this subsection is yet in force.

**What the aggregate proves once a height is pinned**: every bit **set** in the block body's
`attestation_bitfield` is backed by a real BLS signature over the attested parent block hash.

**What it does not prove**:

- **Omission honesty is out of scope** (REQ-BLS-019, explicit). A set bit is proven; a bit left at
  **zero** proves nothing. A producer can still clear bits to deny rewards, or to push a rival
  below the 54/60 reward threshold. This is the largest open attestation risk.
- **Light and snap-sync nodes skip the pairing** until the next epoch rebuild. They still verify
  the `presence_root` commitment, but not the aggregate itself.

**Mitigations**:

- **Aggregate poisoning is neutralised at ingress.** An unverifiable BLS half never enters the
  parent-signature pool, so it can never reach an aggregate
  (`bins/node/src/node/attestation/ingress.rs`).

**Residual risk**:

- **Key mismatch is permanent exclusion until a rotation TxType exists.** No `TxType` among the 24
  rotates a BLS key; `bls_pubkey` is written only at Registration apply, genesis completion and
  rebuild. Losing the BLS key means `Exit` plus a fresh `Registration`, forfeiting `registered_at`
  seniority under INC-I-193 tiering. Key rotation is an open decision (O5) and a pinning
  precondition (P4) in `specs/attestation-bls-architecture.md`.

### 7.10 Output-Creation Authority — Unbacked Non-Native Value (INC-I-234)

**Risk**: Pool, FungibleAsset and LPShare outputs carry value that is not native DOLI. Below
`inc_i_234_activation_height`, consensus checks that these outputs are well-formed, but not that
the creating transaction has the right to create them. Vectors (analysis
`docs/bugfixes/inc-i-234-output-creation-authority-analysis.md` §2):

- **W1** a `Transfer` writes a Pool with unbacked reserves; a later `Swap` pays out native DOLI
  from it (native mint).
- **W2** a `Transfer` writes FA units it never owned (counterfeit `asset_b`), then drains a real
  pool through `Swap`.
- **W3** a counterfeit LPShare for a real `pool_id` drains that pool through `RemoveLiquidity`.
- **W4** `BurnAsset` with FA outputs greater than FA inputs inflates the asset.
- **W5** `CreatePool` change outputs of a foreign asset create counterfeit FA.
- **W6** an extra Pool at output[1..] of `Swap`/`AddLiquidity`/`RemoveLiquidity` claims an
  asset it is not backed by.
- **W7** a producer shapes a coinbase output as Pool/FA/LP (no inputs, no UTXO checks).
- **W8** a pre-existing phantom Pool with the attacker's key is consumed as a second input of a
  `Swap` and its reserves are counted.

**Mitigation** (`specs/protocol.md` §3.22.1, `crates/core/src/validation/output_authority.rs`):
at and above the activation height, Pool outputs appear only at output[0] of the four AMM types
(exactly one), `Swap`/`AddLiquidity`/`RemoveLiquidity` consume exactly one Pool at input[0], the
coinbase is Normal-only, and FA (per `asset_id`) and LP (per `pool_id`) are conserved
(`Σout ≤ Σin`), except the keys the AMM rules already bind and one input-anchored issuance per
`Transfer`. Codes `[ERRTX-AUTH001]`..`[ERRTX-AUTH005]`. Mempool, block assembly and block apply
share the same predicates. The gate is an own `NetworkParams` field; the mainnet value cannot be
overridden by env.

**Residual risk**:

- **Forward-only.** The rule does not re-validate or neutralize Pool/FA/LP UTXOs created
  **before** the activation height. A pre-AH phantom stays in the UTXO set. A read-only census of
  all existing Pool/FA/LP UTXOs (Pool lineage from `CreatePool`, per-asset Σ ≤ issued supply,
  per-pool LP Σ within `total_lp`) is required before the mainnet height is pinned.
- `ZKRollup` has the same free-creation shape; it is not exploitable while `ZKSettle` is frozen.

### 7.11 Maintainer Trust Root of Snap-Synced Nodes (INC-I-248)

**Risk**: The maintainer set is the release trust root. It is node-local (`maintainer_state.bin`)
and is not in the state root. A snap-synced node cannot replay the governance history below its
snapshot. Before INC-I-248 such a node seeded its set from the current producer set. On mainnet
that gave the 5 retired genesis keys, whose private halves are public. The node then refused every
real release (`Insufficient signatures: 0/3`), and a holder of the leaked keys could sign a
release for it.

**Mitigation** (`specs/protocol.md` §7.3.2 and §10.2):

- A root that holds a retired genesis key is never usable for releases, on every network.
- A snap-synced node never seeds from producers. It stores an EMPTY set at the snap height, which
  authorises no release (fail closed).
- The peer quorum changes only an EMPTY root stamped at a snap height S. It never changes a
  non-empty root or a root from the node's own replay. A snapshot install always resets the root
  to EMPTY at S.
- The node adopts a set only at the end of the 15 s window, when the largest digest group reaches
  `snap_quorum()` AND is a strict majority of the counted answers. An answer counts when the peer
  tip >= S. Each digest is recomputed with the local genesis hash. A minority, a tie or an empty
  set never wins.
- Retirement height (`retired_genesis_maintainer_keys_height`): mainnet 331_457, testnet and devnet
  `u64::MAX`. Above it, an answer whose set holds a retired genesis key is dropped. At start and
  after a producer seed, a local set that holds one becomes EMPTY at the tip (the sanitizer). An
  EMPTY set verifies no governance signature, so the leaked keys cannot rotate it. The probe then
  repairs the root.
- Provenance check at start: a non-empty set whose `last_derived_height` D >= 1 is below the tip,
  with block D in the local block store and block D+1 absent, was not derived from replayed
  history. The node resets it to EMPTY at the tip and the peer quorum repairs it. Pruning removes
  a contiguous prefix from height 1, so a pruned or full replay node never matches.
- Post-gate seed check at start: a non-empty set with `last_updated` == D == `last_derived_height`
  >= G > 0 (G = `maintainer_derivation_activation_height`), the tip above D, and the node's own
  block D present with no AddMaintainer or RemoveMaintainer transaction came from a producer seed
  above the gate. The node resets it to EMPTY at the tip and the peer quorum repairs it. A missing
  block D keeps the set. A replay node holds a pre-gate seed or a governance-block D and keeps its
  set. Devnet (G = 0) never matches.
- A reply to the probe never reaches the sync manager, and a node never answers the probe busy.
- `doli snap` installs the snapshot's maintainer set only when the seeds agree on its digest by
  the state-root rule, with the configured seed count (silent seeds count against it), and the
  recomputed digest matches. A set that holds a key retired at the snapshot height is written
  EMPTY at that height.

**Residual risk**:

- **Same trust as the snapshot.** A dishonest majority of peers can give a snap-synced node a
  wrong maintainer set, as it can give it a wrong state. A node that replayed the chain is never
  changed by peers. The producer veto window still applies to every release.
- **No peers, no repair.** With fewer agreeing answers than the quorum (for example a devnet below
  3 peers, or old peers that do not answer) the node keeps its root. An EMPTY root then refuses
  every release until enough upgraded peers answer.

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

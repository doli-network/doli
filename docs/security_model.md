# security_model.md - Security Analysis

This document describes DOLI's security model, threat analysis, and cryptographic guarantees.

---

## 1. Security Principles

DOLI is built on three fundamental security principles:

1. **Time as the scarce resource** - Cannot be accumulated or parallelized
2. **Deterministic consensus** - No lottery variance, predictable selection
3. **Economic finality** - Misbehavior results in bond loss

---

## 2. Cryptographic Primitives

### 2.1. Hash Function: BLAKE3-256

| Property | Value |
|----------|-------|
| Output size | 256 bits |
| Security level | 128-bit collision resistance |
| Performance | ~3x faster than SHA-256 |

**Usage:**
- Block hashing
- Transaction hashing
- Merkle tree construction
- Address derivation
- VDF input construction

### 2.2. Digital Signatures: Ed25519

| Property | Value |
|----------|-------|
| Key size | 256-bit private, 256-bit public |
| Signature size | 512 bits |
| Security level | ~128-bit |

**Security measures:**
- Constant-time operations (timing attack resistant)
- Zeroization on drop (memory protection)
- Domain separation tags prevent cross-protocol attacks

**Domain tags:**
- `SIGN_V1` - Generic signing
- `TX_V1` - Transaction signing
- `BLOCK_V1` - Block signing
- `VDF_V1` - VDF preimage
- `ADDR_V1` - Address derivation

### 2.3. VDF Implementations

**Block/Heartbeat VDF (Hash-Chain):**
| Property | Value |
|----------|-------|
| Construction | Iterated SHA-256 hash chain |
| Iterations | ~10,000,000 (~700ms) |
| Verification | Recompute (linear time) |
| Purpose | Block production heartbeat |

**Registration VDF (Wesolowski Class Groups):**
| Property | Value |
|----------|-------|
| Group | Imaginary quadratic class groups |
| Discriminant | 2048 bits |
| Base iterations | 600,000,000 (~10 min) |
| Verification | O(log t) using Wesolowski proof |
| Purpose | Anti-Sybil protection |

**Security guarantees:**
- Sequential computation required (no parallelization)
- Grinding prevention via Epoch Lookahead (deterministic selection)
- Registration time investment prevents Sybil attacks

---

## 3. Consensus Security

### 3.1. Proof of Time Properties

| Property | Guarantee |
|----------|-----------|
| **Safety** | No conflicting blocks finalized |
| **Liveness** | Blocks produced at slot rate |
| **Fairness** | Proportional to bond count |

### 3.2. Attack Resistance

**51% Attack:**
- Requires controlling >50% of total bond tickets
- Each bond requires 10 DOLI locked for 4 years
- Cannot accelerate through hardware (VDF is sequential)

**Grinding Attack:**
- Selection is deterministic: `slot % total_tickets`
- Producer cannot influence future selection
- Active set frozen at epoch boundaries

**Long-Range Attack:**
- Weight-based fork choice favors senior producers
- New producers have weight 1, seniors have up to 4
- Attacker cannot quickly accumulate weight

### 3.3. Finality Model

```
Probabilistic finality increases with confirmations:

Confirmations    Reorganization probability
     1                  ~1% (1 slot delay)
     6                  <0.1%
    60                  <0.0001% (1 epoch)
   360                  Effectively final
```

---

## 4. Economic Security

### 4.1. Bond Requirements

| Parameter | Value |
|-----------|-------|
| Minimum bond | 10 DOLI |
| Maximum bonds per producer | 10,000 |
| Lock period | 4 years |
| Unbonding period | 7 days |
| Withdrawal delay | 7 days |

### 4.2. Slashing Conditions

Only one slashable offense exists:

| Offense | Penalty | Detection |
|---------|---------|-----------|
| Double production | 100% bond burned | Any node can prove |

**Non-slashable offenses:**
- Inactivity (50 missed slots) → Removal from active set, bond retained
- Invalid blocks → Block rejected, no penalty

### 4.3. Economic Attack Costs

**To control 51% of a network with 1M DOLI bonded:**

```
Required capital: 1,001,000 DOLI
Required time: 1,001 × T_registration (sequential)
Risk: 100% loss if caught double-producing
```

The sequential time requirement cannot be bypassed with additional hardware.

---

## 5. Network Security

### 5.1. Transport Security

| Layer | Protection |
|-------|------------|
| Transport | TCP with Noise Protocol encryption |
| Authentication | Ed25519 node keys |
| Multiplexing | Yamux (prevents head-of-line blocking) |

### 5.2. Message Security

| Mechanism | Purpose |
|-----------|---------|
| Signed gossip | Prevent message spoofing |
| Network ID check | Prevent cross-network attacks |
| Genesis hash check | Prevent chain confusion |
| Rate limiting | DoS protection |
| Peer scoring | Misbehavior detection |

### 5.3. Rate Limits

| Resource | Limit |
|----------|-------|
| Blocks per peer/minute | 100 |
| Transactions per peer/minute | 1000 |
| Sync requests per peer/minute | 60 |

### 5.4. Peer Scoring

| Behavior | Score Change |
|----------|--------------|
| Valid block | +10 |
| Valid transaction | +1 |
| Invalid block | -50 |
| Invalid transaction | -10 |
| Timeout | -5 |
| Protocol violation | -100 |

Peers below threshold are disconnected and banned.

---

## 6. Anti-Sybil Defenses

### 6.1. Defense Layers

1. **Chained VDF Registration**
   - Each registration must reference previous registration hash
   - Prevents parallel identity creation
   - One identity per registration window

2. **Weight by Seniority** (only factor affecting weight)
   - Year 1: weight 1
   - Year 2: weight 2
   - Year 3: weight 3
   - Year 4+: weight 4

3. **No Activity Penalty**
   - Producers who miss slots simply miss rewards
   - No slashing or weight reduction for inactivity
   - Only slashable offense: double production (equivocation)

4. **Bond Count vs Weight**
   - Bond count affects slot allocation (more slots per cycle)
   - Bond count does NOT affect weight (seniority only)

5. **Bond Stacking Cap**
   - Maximum 100 bonds per producer
   - Prevents single-identity dominance

### 6.2. Governance Protection

| Mechanism | Threshold |
|-----------|-----------|
| Update veto | 40% of weighted votes |
| Veto period | 7 days |
| Weight calculation | Seniority-based |

---

## 7. Validation Rules

### 7.1. Block Validation

```
1. Syntactic validation
   - Version > 0
   - Previous hash exists (or genesis)
   - Merkle root matches transactions
   - Timestamp reasonable

2. Consensus validation
   - Slot > previous slot
   - Producer eligible for slot
   - VDF proof valid
   - Producer not already produced for slot

3. Transaction validation
   - All transactions individually valid
   - No double-spends within block
   - Coinbase correct amount
```

### 7.2. Transaction Validation

```
1. Structural validation
   - At least one input (except coinbase)
   - At least one output
   - All amounts positive

2. Input validation
   - Each input references existing UTXO
   - Signature valid for referenced output's pubkey
   - Not already spent

3. Amount validation
   - sum(inputs) >= sum(outputs)
   - Difference is fee (must meet minimum)
```

---

## 8. Threat Model

### 8.1. Assumed Adversary Capabilities

| Capability | Assumption |
|------------|------------|
| Computational | Up to 49% of sequential computation |
| Economic | Up to 49% of total bond |
| Network | Can delay messages, not forge signatures |
| Time | Cannot accelerate VDF computation |

### 8.2. Out-of-Scope Threats

| Threat | Reason |
|--------|--------|
| Quantum computing | Ed25519 vulnerable; future upgrade path exists |
| Key theft | User responsibility |
| Social engineering | User responsibility |
| Implementation bugs | Mitigated through testing and audits |

---

## 9. Equivocation Detection

### 9.1. Detection Mechanism

```rust
struct EquivocationProof {
    producer: PublicKey,
    slot: u32,
    block_hash_1: Hash,
    block_hash_2: Hash,
    // Both blocks signed by producer for same slot
}
```

### 9.2. Detection Flow

```
1. Node receives block for (producer, slot)
2. Check if different block already seen for (producer, slot)
3. If yes, construct EquivocationProof
4. Create SlashProducer transaction
5. Broadcast to network
6. On inclusion: producer bond burned, removed from set
```

---

## 10. Key Management

### 10.1. Key Types

| Key Type | Purpose | Storage |
|----------|---------|---------|
| Node key | P2P identity | `~/.doli/{network}/node.key` |
| Producer key | Block signing | User-specified path |
| Wallet keys | Transaction signing | `~/.doli/wallet.json` |

### 10.2. Key Security Recommendations

1. **Producer keys**: Store on dedicated, airgapped machine
2. **Wallet keys**: Use hardware wallet or encrypted storage
3. **Backups**: Store encrypted backups in multiple locations
4. **Rotation**: Rotate keys periodically (requires re-registration)

---

## 11. Privacy Considerations

### 11.1. On-Chain Privacy

| Data | Visibility |
|------|------------|
| Transaction amounts | Public |
| Sender/receiver addresses | Public (pseudonymous) |
| Block producer | Public |
| IP addresses | Not stored on-chain |

### 11.2. Network Privacy

| Data | Visibility |
|------|------------|
| IP addresses | Visible to connected peers |
| Transaction origin | Can be inferred by first-seen peer |

**Recommendations:**
- Use new addresses for each transaction
- Consider Tor for network privacy (not built-in)

---

## 12. Incident Response

### 12.1. Critical Vulnerability

1. Notify maintainers privately
2. Prepare patch
3. Coordinate disclosure with major operators
4. Release patched version
5. Public disclosure after deployment

### 12.2. Network Attack

1. Identify attack vector
2. Coordinate with operators
3. Deploy mitigations
4. Post-mortem analysis

---

## 13. Security Assumptions

The security of DOLI relies on:

1. **Cryptographic assumptions**
   - BLAKE3 is collision resistant
   - Ed25519 is unforgeable
   - Class group discrete log is hard

2. **Network assumptions**
   - Partial synchrony (bounded message delays)
   - Honest majority of computation capacity

3. **Economic assumptions**
   - Rational actors prefer profit over destruction
   - Bond loss is effective deterrent

---

*Security model version: 1.0*

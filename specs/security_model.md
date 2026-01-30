# DOLI Security

This document describes the security model, threat analysis, cryptographic foundations, and implementation security measures of the DOLI protocol.

## Table of Contents

1. [Security Model](#1-security-model)
2. [Threat Model](#2-threat-model)
3. [Cryptographic Security](#3-cryptographic-security)
4. [Economic Security](#4-economic-security)
5. [Consensus Security](#5-consensus-security)
6. [Implementation Security](#6-implementation-security)
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

1. **Break cryptographic assumptions**: Ed25519, BLAKE3, class group DLP
2. **Accelerate sequential computation**: VDFs require inherently sequential work
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
| Cheap identity creation | Bond requirement (1000 coins initially) |
| Identity accumulation | Registration difficulty scales with demand |

#### 2.2.3 Consensus Attacks

| Attack | Mitigation |
|--------|------------|
| Nothing-at-stake | Bond slashing for equivocation |
| Grinding | Selection seed derived from previous block hash |
| Time manipulation | Slot anchored to VDF-proven timestamp |

#### 2.2.4 Network Attacks

| Attack | Mitigation |
|--------|------------|
| Eclipse attack | Multiple peer connections; peer reputation |
| DoS on producers | Producer rotation; multiple active producers |
| Transaction censorship | Fee market; producer competition |

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
HASH("BLK" || data)   // Block VDF input
HASH("REG" || data)   // Registration VDF input
HASH("SEED" || data)  // Producer selection seed
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

**Construction**: Iterated hash chain using SHA-256 with dynamic calibration

**Security Assumptions**:
- Sequentiality of hash chain computation (no parallel speedup)
- Preimage resistance of SHA-256

**Parameters**:
| Parameter | Value | Rationale |
|-----------|-------|-----------|
| Target Time | ~700ms | Heartbeat proof of presence |
| Iterations | ~10,000,000 | Calibrated to achieve ~700ms |

**Properties**:
- **Sequentiality**: Cannot be parallelized
- **Verification**: Recompute the entire chain (O(T))
- **Unique output**: Given input, only one valid output exists
- **Fixed iterations**: Currently uses network-specific fixed iterations (~10M for mainnet/testnet)

**Note**: A dynamic calibration module exists in the codebase but is not currently
used for block production. Block iterations are fixed per network configuration.

**Security Considerations**:
1. Input includes domain tag ("DOLI_HEARTBEAT_V1") for separation
2. Calibration bounds prevent extreme values (min: 100K, max: 100M iterations)
3. Combined with Epoch Lookahead for grinding prevention

#### 3.3.2 Registration VDF: Wesolowski Class Groups

**Construction**: Wesolowski VDF over imaginary quadratic class groups with 2048-bit discriminant

**Security Assumptions**:
- Unknown group order (requires factoring 2048-bit discriminant)
- Hardness of computing discrete logs in class groups
- Low-order assumption for proof security

**Parameters**:
| Parameter | Value | Rationale |
|-----------|-------|-----------|
| T_REGISTER_BASE | 600M iterations (~10 min) | Anti-Sybil protection |
| T_REGISTER_CAP | 86.4B iterations (~24 hrs) | Prevents network closure |
| Discriminant bits | 2048 | ~112-bit security |

**Properties**:
- **Sequentiality**: x^(2^t) requires ~t squarings in unknown-order group
- **Efficient verification**: O(log t) using Wesolowski proof
- **ASIC resistance**: No known efficient ASIC design for class group operations
- **Dynamic scaling**: Difficulty increases with registered producer count

### 3.4 Cryptographic Constants

```rust
// All domain tags are unique to prevent cross-protocol attacks
pub const SIGN_DOMAIN: &[u8] = b"DOLI_SIGN_V1";
pub const ADDRESS_DOMAIN: &[u8] = b"DOLI_ADDR_V1";
pub const TX_DOMAIN: &[u8] = b"DOLI_TX_V1";
pub const BLOCK_DOMAIN: &[u8] = b"DOLI_BLOCK_V1";
pub const VDF_DOMAIN: &[u8] = b"DOLI_VDF_V1";
pub const VDF_BLOCK_DOMAIN: &[u8] = b"DOLI_VDF_BLOCK_V1";
pub const VDF_REGISTER_DOMAIN: &[u8] = b"DOLI_VDF_REGISTER_V1";
pub const SEED_DOMAIN: &[u8] = b"SEED";

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

**Bond Schedule**:
```
Era 0 (years 0-4):   1,000 coins
Era 1 (years 4-8):     700 coins (30% reduction)
Era 2 (years 8-12):    490 coins
Era 3 (years 12-16):   343 coins
...
```

### 4.2 Slashing Conditions

| Violation | Penalty | Proof |
|-----------|---------|-------|
| Double production | 100% bond burned | Two signed blocks for same slot |
| Invalid block | None (slot lost) | Block rejected by network |
| Repeated inactivity | Removal from set | Miss consecutive slots |

**Important**: Slashing is reserved ONLY for double production (equivocation). This is the only offense that cannot happen by accident.

Invalid blocks are NOT slashed - the natural penalty (losing the slot and its reward) is sufficient. Following Bitcoin's philosophy: the network rejects bad blocks, you wasted your time, end of story.

Slashed bonds are **burned**, not redistributed, to prevent:
- Incentivizing false accusations
- Collusion between accusers and validators
- Gaming the slashing mechanism

### 4.3 Economic Attack Costs

To control the network, an attacker would need to:

1. **Register majority of producers**: Requires (N/2 + 1) * VDF_TIME sequential computation
2. **Maintain bonds**: Requires (N/2 + 1) * BOND_AMOUNT capital at risk
3. **Risk detection**: Equivocation leads to permanent bond loss

**Cost Analysis** (Era 0, assuming 100 active producers):
- Minimum 51 registrations: 51 * 10 minutes = 8.5 hours sequential time
- Bond requirement: 51 * 1000 = 51,000 coins at risk
- Potential loss if detected: 51,000 coins

### 4.4 Time-Based Economics

Unlike capital-based systems, time cannot be:
- Borrowed or leveraged
- Accumulated faster through spending
- Transferred between parties

This creates a fundamental limit on how fast identities can be created, regardless of financial resources.

---

## 5. Consensus Security

### 5.1 Chain Selection

The chain selection rule (slot > height > hash) ensures:

1. **Time coverage**: Chains covering more slots are preferred
2. **Density**: Among equal slot coverage, denser chains win
3. **Determinism**: Hash comparison provides final tiebreaker

**Security Properties**:
- Cannot be manipulated by content grinding (slot is time-derived)
- Favors honest chains that follow timing rules
- Provides unique canonical chain at any point

### 5.2 Producer Selection

```
seed = HASH("SEED" || prev_hash || slot)
score(producer) = HASH(seed || producer_pubkey)
selected = argmin(score)
```

**Security Properties**:
- **Unpredictable until prev_block finalized**: Seed depends on prev_hash
- **Deterministic**: All honest nodes compute same result
- **Uniform distribution**: HASH output is uniformly distributed
- **Unbiasable**: Producer cannot influence their score for future slots

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

During bootstrap (first 10,080 blocks):

| Risk | Mitigation |
|------|------------|
| Single party dominance | Lowest hash wins ties |
| No economic stake | Bootstrap phase is time-limited |
| Racing attacks | VDF still required for each block |

---

## 6. Implementation Security

### 6.1 Constant-Time Operations

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

### 6.2 Integer Overflow Protection

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

### 6.3 Input Validation

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

### 6.4 Memory Safety

- Written in Rust, providing memory safety guarantees
- No unsafe code in core cryptographic operations
- Bounds checking on all array accesses
- Automatic cleanup of sensitive data via Drop trait

### 6.5 Serialization Security

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
- Class group VDFs have no known efficient ASIC design
- T parameter can be increased via protocol upgrade
- Security degrades gracefully (faster blocks, not broken security)

### 7.3 Network Partition

**Risk**: Network splits could create competing chains

**Mitigations**:
- Longest-slot chain rule provides clear resolution
- VDF ensures time passes even during partition
- Reconciliation upon reconnection is deterministic

### 7.4 Producer Centralization

**Risk**: Small number of producers could dominate

**Mitigations**:
- No economies of scale in VDF computation
- Bond decreases over time (easier entry)
- Inactivity penalties encourage liveness

### 7.5 Producer Selection Grinding (prev_hash manipulation)

**Risk**: A malicious producer of block N-1 could attempt to manipulate `prev_hash` to influence producer selection for slot N.

**Attack Mechanism**:
```
seed_N = HASH("SEED" || prev_hash || slot_N)
score(producer) = HASH(seed_N || producer_pubkey)
```

Since `prev_hash = HASH(block_N-1)`, an attacker who produces block N-1 could:
1. Generate multiple valid block candidates (varying timestamp, tx ordering, etc.)
2. Compute resulting `prev_hash` for each candidate
3. Select the candidate that gives them best `score` for slot N
4. Pre-compute VDF for slot N using the chosen `prev_hash`

**Cost-Benefit Analysis**:

| Factor | Value | Impact |
|--------|-------|--------|
| VDF computation time | ~700ms | Fast heartbeat with Epoch Lookahead |
| Must win slot N-1 first | Probabilistic | Attacker needs prior slot control |
| Benefit | Better position in N | Only probabilistic advantage |
| Detection | None | Attack is indistinguishable from honest behavior |

**Quantitative Assessment**:
- With 100 producers and random selection, probability of winning any slot ≈ 1%
- Grinding N variants improves odds to approximately `1 - (1 - 1/100)^N`
- 60 variants (1 hour grinding): ~45% chance of winning next slot
- But attacker must already control slot N-1 to grind

**Current Mitigations**:
1. **Epoch Lookahead**: Leader selection is deterministic at epoch start, not per-slot
2. **Sequential dependency**: Must win N-1 before grinding for N
3. **Diminishing returns**: Each additional variant provides marginal improvement
4. **No compounding**: Winning slot N doesn't help grind for N+1 (new prev_hash)

**Why This Is Acceptable**:
- Attack requires sustained slot control (hard to achieve)
- Benefit is positional advantage, not consensus violation
- No known practical exploitation in similar systems (Ethereum's RANDAO has analogous properties)
- Class group VDFs have no efficient ASIC design (grinding throughput is hardware-limited)

**Future Considerations**:
- If VDF ASICs emerge, T parameter should be increased proportionally
- Commit-reveal schemes could eliminate grinding but add latency
- VRF-based selection (like Algorand) would eliminate grinding entirely but requires different trust assumptions

**Monitoring Recommendation**:
Track producer win rates. Statistically significant deviation from expected distribution (>3σ) over extended periods could indicate grinding attacks.

**Status**: Accepted risk. Cost/benefit ratio makes attack impractical with current technology. Will revisit if VDF acceleration hardware emerges.

*Documented: 2026-01-25 during security audit*

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
| TBD | Full protocol | TBD | Pending |

**2026-01-25 - VDF Slashing Evidence Fix**:
- **Issue**: Slashing evidence only contained block hashes, not full headers
- **Impact**: Verifiers couldn't validate VDF proofs in slashing claims
- **Fix**: Changed SlashingEvidence to include full BlockHeaders for VDF verification
- **Commit**: `5863805`

**Note**: VDF iterations are currently network-dependent (fixed per network), not
era-dependent. Block production uses ~10M iterations across all eras.

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

**Email**: doli@protonmail.com

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

*Last updated: 2026-01-25*
*DOLI Security Team*
# DOLI Security Checklist

This checklist helps node operators and producers secure their DOLI infrastructure.

## Node Security

### Network Configuration

- [ ] **Firewall enabled** - Only allow necessary ports
  ```bash
  # Allow P2P
  sudo ufw allow 30303/tcp

  # Allow RPC only from trusted IPs (if needed externally)
  sudo ufw allow from 192.168.1.0/24 to any port 8545

  # Allow metrics only from monitoring server
  sudo ufw allow from 10.0.0.5 to any port 9090

  # Enable firewall
  sudo ufw enable
  ```

- [ ] **RPC bound to localhost** - Default is `127.0.0.1:8545`
  ```toml
  # config.toml - GOOD
  [rpc]
  listen_addr = "127.0.0.1:8545"

  # AVOID exposing to all interfaces
  # listen_addr = "0.0.0.0:8545"  # DANGEROUS
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
  curl http://localhost:9090/metrics | grep vdf_compute

  # Should complete in ~700ms (heartbeat VDF)
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
  address = "/dns4/seed1.doli.network/tcp/30303/p2p/..."

  [[bootstrap_nodes]]
  address = "/dns4/seed2.doli.network/tcp/30303/p2p/..."
  ```

- [ ] **Monitor peer diversity**
  ```bash
  curl http://localhost:8545 \
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
   - Withdraw bond (1-week cooldown)
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
curl -s http://localhost:8545 \
  -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}' \
  | grep -o '"peer_count":[0-9]*' | cut -d: -f2

echo -n "Sync status: "
curl -s http://localhost:8545 \
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

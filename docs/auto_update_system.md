# DOLI Auto-Update System

## Complete Technical Specification v3.0

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Maintainer Bootstrap System](#2-maintainer-bootstrap-system)
3. [Governance and Voting System](#3-governance-and-voting-system)
4. [Sybil Resistance Analysis](#4-sybil-resistance-analysis)
5. [Complete Update Timeline](#5-complete-update-timeline)
6. [Automatic Rollback System](#6-automatic-rollback-system)
7. [Hard Fork Support](#7-hard-fork-support)
8. [Security Model](#8-security-model)
9. [CLI Command Reference](#9-cli-command-reference)
10. [RPC Endpoints](#10-rpc-endpoints)
11. [Implementation Reference](#11-implementation-reference)
12. [Comparison with Other Blockchains](#12-comparison-with-other-blockchains)
13. [Frequently Asked Questions](#13-frequently-asked-questions)

---

## 1. Executive Summary

The DOLI auto-update system is a decentralized, cryptographically secure mechanism for coordinating software updates across the network. It balances the need for rapid security patches with democratic governance, ensuring no single party can force malicious updates on the network.

### 1.1 Key Features

- **Emergent maintainers**: First 5 registered producers automatically become maintainers (no hardcoding)
- **On-chain maintainer management**: Add/remove maintainers via 3/5 multisig transactions
- **Transparent updates**: All releases publicly signed and verifiable on-chain
- **Seniority-weighted voting**: 40% threshold to veto, weighted by producer tenure (max 4x at 4 years)
- **Automatic application**: Updates applied after veto period with automatic rollback on failure
- **Multi-signature releases**: 3/5 maintainer signatures required for any release
- **Version enforcement**: Outdated producers paused from block production after grace period
- **Hard fork support**: Optional upgrade-at-height mechanism for breaking protocol changes

### 1.2 Key Constants

```
┌────────────────────────────────┬───────────┬─────────────────────────────────────────┐
│ Constant                       │ Value     │ Description                             │
├────────────────────────────────┼───────────┼─────────────────────────────────────────┤
│ INITIAL_MAINTAINER_COUNT       │ 5         │ First N producers become maintainers    │
│ MAINTAINER_THRESHOLD           │ 3 of 5    │ Signatures needed for any action        │
│ MIN_MAINTAINERS                │ 3         │ Cannot remove below this                │
│ MAX_MAINTAINERS                │ 5         │ Maximum maintainer count                │
│ VETO_PERIOD                    │ 2 epochs (~2h) │ Time for producers to vote on updates   │
│ GRACE_PERIOD                   │ 1 epoch (~1h)  │ Time after approval before enforcement  │
│ VETO_THRESHOLD_PERCENT         │ 40%       │ Weighted percentage needed to reject    │
│ CHECK_INTERVAL                 │ 6 hours   │ How often nodes check for updates       │
│ MAX_SENIORITY_MULTIPLIER       │ 4x        │ Maximum vote weight for seniors         │
│ SENIORITY_MATURITY_YEARS       │ 4         │ Years to reach maximum seniority        │
│ MIN_VOTING_AGE_DAYS            │ 30        │ Minimum days as producer to vote        │
│ CRASH_THRESHOLD                │ 3         │ Consecutive crashes before rollback     │
└────────────────────────────────┴───────────┴─────────────────────────────────────────┘
```

---

## 2. Maintainer Bootstrap System

Unlike other blockchains that hardcode maintainer keys in configuration files, DOLI derives its maintainer set directly from the blockchain. The first 5 registered producers automatically become maintainers.

### 2.1 Automatic Bootstrap

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│                      MAINTAINER BOOTSTRAP (AUTOMATIC)                            │
├─────────────────────────────────────────────────────────────────────────────────┤
│                                                                                 │
│  Block 1:   Registration(Alice)   → Maintainer #1 ✓                             │
│  Block 2:   Registration(Bob)     → Maintainer #2 ✓                             │
│  Block 5:   Registration(Carol)   → Maintainer #3 ✓                             │
│  Block 8:   Registration(Dave)    → Maintainer #4 ✓                             │
│  Block 12:  Registration(Eve)     → Maintainer #5 ✓                             │
│  Block 15:  Registration(Frank)   → Normal producer (maintainer set full)       │
│  Block 20:  Registration(Grace)   → Normal producer                             │
│                                                                                 │
│  MaintainerSet = [Alice, Bob, Carol, Dave, Eve]                                 │
│  Threshold = 3 of 5                                                             │
│                                                                                 │
└─────────────────────────────────────────────────────────────────────────────────┘
```

### 2.2 Why This Design?

| Aspect | Hardcoded Keys | DOLI Bootstrap |
|--------|----------------|----------------|
| Source of truth | External config file | Blockchain itself |
| Verification | Trust the config | Anyone can verify |
| Changes | Requires hard fork | On-chain transactions |
| Decentralization | Depends on distribution | Emergent from participation |
| Auditability | Check config matches | Deterministic from chain |

### 2.3 Maintainer Determination

Any node can independently compute the maintainer set by scanning the blockchain:

```rust
pub fn derive_maintainer_set(chain: &Blockchain) -> MaintainerSet {
    let mut maintainers = Vec::new();
    
    for block in chain.blocks() {
        for tx in block.transactions() {
            if tx.tx_type == TxType::Registration && maintainers.len() < 5 {
                maintainers.push(tx.registration_data().public_key);
            }
            
            // Process maintainer changes after bootstrap
            if tx.tx_type == TxType::RemoveMaintainer {
                // Verify 3/5 signatures, then remove
            }
            if tx.tx_type == TxType::AddMaintainer {
                // Verify 3/5 signatures, then add
            }
        }
    }
    
    MaintainerSet::new(maintainers, threshold: 3)
}
```

### 2.4 Maintainer Management Transactions

After bootstrap, the maintainer set can be modified via special transactions requiring 3/5 multisig:

#### 2.4.1 Remove Maintainer

```rust
pub struct RemoveMaintainerData {
    /// Public key of maintainer to remove
    pub target: PublicKey,
    /// Signatures from 3+ current maintainers
    pub signatures: Vec<MaintainerSignature>,
    /// Reason for removal (optional, for transparency)
    pub reason: Option<String>,
}
```

**Constraints:**
- Cannot remove if only 3 maintainers remain (minimum threshold)
- Target must be current maintainer
- Requires 3/5 valid signatures from OTHER maintainers (target cannot sign own removal)

#### 2.4.2 Add Maintainer

```rust
pub struct AddMaintainerData {
    /// Public key of new maintainer
    pub target: PublicKey,
    /// Signatures from 3+ current maintainers
    pub signatures: Vec<MaintainerSignature>,
}
```

**Constraints:**
- Cannot add if already at 5 maintainers (maximum)
- Target must be a registered producer (active or unbonding)
- Target must not already be a maintainer
- Requires 3/5 valid signatures

### 2.5 Edge Cases

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│                              EDGE CASES                                          │
├─────────────────────────────────────────────────────────────────────────────────┤
│                                                                                 │
│  Q: What if a maintainer does Exit (unbonds)?                                   │
│  A: They remain maintainer until explicitly removed via RemoveMaintainer tx.    │
│     This allows graceful transitions - they can sign releases during handover.  │
│                                                                                 │
│  Q: What if a maintainer is slashed?                                            │
│  A: Automatic removal from maintainer set (no 3/5 vote needed).                 │
│     Network security takes precedence.                                          │
│                                                                                 │
│  Q: What if fewer than 5 producers ever register?                               │
│  A: Maintainer set has fewer than 5 members.                                    │
│     Threshold adjusts: 2/3, 2/4, 3/5 (always majority).                         │
│                                                                                 │
│  Q: Can a producer decline maintainer role?                                     │
│  A: No automatic decline. They can immediately propose RemoveMaintainer         │
│     for themselves, but need 3/5 signatures from others.                        │
│                                                                                 │
└─────────────────────────────────────────────────────────────────────────────────┘
```

### 2.6 Maintainer Set State

```rust
pub struct MaintainerSet {
    /// Current maintainer public keys (max 5)
    pub members: Vec<PublicKey>,
    /// Required signatures (always 3 when 5 members)
    pub threshold: usize,
    /// Block height of last change
    pub last_updated: u64,
}

impl MaintainerSet {
    pub fn is_maintainer(&self, pubkey: &PublicKey) -> bool {
        self.members.contains(pubkey)
    }
    
    pub fn can_remove(&self) -> bool {
        self.members.len() > MIN_MAINTAINERS  // > 3
    }
    
    pub fn can_add(&self) -> bool {
        self.members.len() < MAX_MAINTAINERS  // < 5
    }
    
    pub fn verify_multisig(&self, signatures: &[Signature], message: &[u8]) -> bool {
        let valid_count = signatures.iter()
            .filter(|sig| self.is_maintainer(&sig.pubkey))
            .filter(|sig| sig.verify(message))
            .count();
        valid_count >= self.threshold
    }
    
    /// Dynamic threshold based on member count
    pub fn calculate_threshold(member_count: usize) -> usize {
        match member_count {
            1 => 1,
            2 => 2,
            3 => 2,
            4 => 3,
            5 => 3,
            _ => (member_count / 2) + 1,  // Simple majority
        }
    }
}
```

---

## 3. Governance and Voting System

The governance model uses seniority-weighted voting to balance democratic participation with Sybil resistance. This ensures that established, long-term participants have proportionally more influence while still allowing newcomers to participate meaningfully.

### 3.1 Seniority-Weighted Voting

Unlike simple count-based voting (1 producer = 1 vote), DOLI uses seniority weighting that rewards long-term commitment to the network.

#### 3.1.1 Weight Calculation

```
vote_weight = 1.0 + min(years_as_producer, 4) * 0.75

Examples:
┌─────────────────────────┬────────────────────────────────┬─────────┐
│ Producer Age            │ Calculation                    │ Weight  │
├─────────────────────────┼────────────────────────────────┼─────────┤
│ New producer (0 years)  │ 1.0 + 0 * 0.75                 │ 1.00x   │
│ 1 year producer         │ 1.0 + 1 * 0.75                 │ 1.75x   │
│ 2 year producer         │ 1.0 + 2 * 0.75                 │ 2.50x   │
│ 3 year producer         │ 1.0 + 3 * 0.75                 │ 3.25x   │
│ 4+ year producer        │ 1.0 + 4 * 0.75                 │ 4.00x   │
└─────────────────────────┴────────────────────────────────┴─────────┘
```

**Key insight**: After 4 years, all producers reach the same maximum weight. The seniority advantage is temporary, not permanent. This prevents oligarchy while still providing meaningful Sybil resistance during the network's growth phase.

#### 3.1.2 Relationship with Bond Units

The voting weight is based on **time** (seniority), not **stake** (bond units). This is intentional:

| System | Weight Based On | Implication |
|--------|-----------------|-------------|
| Block production | Bond units | More stake = more blocks = more rewards |
| Governance voting | Seniority | Time commitment = more influence |

This separation ensures that wealthy newcomers cannot immediately dominate governance, while still allowing them to earn proportional block rewards.

#### 3.1.3 Minimum Voting Age

To prevent flash Sybil attacks, producers must be active for at least **30 days** before their votes count. This creates a significant cost barrier: an attacker would need to fund and maintain fake producers for a month before gaining any voting power.

#### 3.1.4 Veto Threshold Calculation

```
veto_weight  = sum(vote_weight for each veto vote)
total_weight = sum(vote_weight for all eligible producers)
veto_percent = (veto_weight * 100) / total_weight

if veto_percent >= 40%: REJECTED
if veto_percent <  40%: APPROVED
```

**Example with 10 producers (mixed seniority):**

```
┌────────────┬───────┬────────┬─────────┬──────────────┐
│ Producer   │ Years │ Weight │ Vote    │ Contribution │
├────────────┼───────┼────────┼─────────┼──────────────┤
│ Producer A │ 5     │ 4.00x  │ VETO    │ 4.00         │
│ Producer B │ 3     │ 3.25x  │ VETO    │ 3.25         │
│ Producer C │ 2     │ 2.50x  │ APPROVE │ 0            │
│ Producer D │ 2     │ 2.50x  │ APPROVE │ 0            │
│ Producer E │ 1     │ 1.75x  │ VETO    │ 1.75         │
│ Producer F │ 1     │ 1.75x  │ APPROVE │ 0            │
│ Producer G │ 0.5   │ 1.375x │ APPROVE │ 0            │
│ Producer H │ 0.5   │ 1.375x │ APPROVE │ 0            │
│ Producer I │ 0.1   │ 1.075x │ VETO    │ 1.075        │
│ Producer J │ 0.1   │ 1.075x │ APPROVE │ 0            │
└────────────┴───────┴────────┴─────────┴──────────────┘

Total weight: 20.625
Veto weight:  10.075 (A + B + E + I)
Veto percent: 10.075 / 20.625 * 100 = 48.8%

Result: 48.8% >= 40% → REJECTED
```

### 3.2 Vote Lifecycle

#### 3.2.1 Vote Submission

Producers submit votes via RPC or CLI. Votes are cryptographically signed and broadcast via gossip to all nodes.

```rust
pub struct VoteMessage {
    pub version: String,           // "1.0.1"
    pub vote: Vote,                // Approve or Veto
    pub producer_pubkey: [u8; 32], // Producer's public key
    pub timestamp: u64,            // Unix timestamp
    pub signature: [u8; 64],       // Ed25519 signature
}

pub enum Vote {
    Approve,  // Allow the update (or abstain - same effect)
    Veto,     // Block the update
}
```

#### 3.2.2 Vote Changing

Unlike count-based systems, producers **CAN change their vote** during the veto period. This allows reaction to new information (e.g., discovery of a backdoor in the update).

- Each new vote from the same producer replaces their previous vote
- Only the latest vote (by timestamp) counts at deadline
- All vote history is preserved for transparency

#### 3.2.3 Offline Producers

Producers that are offline (not producing blocks) can still vote. The voting system uses gossip propagation, not block production. This ensures:

- Maintenance windows don't forfeit voting rights
- Network issues don't disenfranchise producers
- Only the 30-day minimum age matters, not current online status

#### 3.2.4 Vote Finalization

Votes are counted at the exact moment the veto period expires. The result is deterministic: any node can independently verify the outcome by replaying all votes received before the deadline.

---

## 4. Sybil Resistance Analysis

The combination of bond requirements, seniority weighting, and minimum voting age creates strong Sybil resistance.

### 4.1 Attack Cost Analysis

```
┌──────────────────────────┬──────────────────────┬───────────────────┬─────────────────────┐
│ Attack Vector            │ Cost                 │ Effectiveness     │ Mitigation          │
├──────────────────────────┼──────────────────────┼───────────────────┼─────────────────────┤
│ Register many producers  │ N × bond amount      │ Low (1x weight)   │ Bond requirement    │
│ Flash Sybil (same day)   │ N × bond             │ Zero              │ 30-day minimum age  │
│ Sustained Sybil (1 mo)   │ N × bond × 30 days   │ Low (1.0x weight) │ Seniority weighting │
│ Long-term Sybil (4 yrs)  │ N × bond × 4 years   │ Equal to veterans │ Time prohibitive    │
└──────────────────────────┴──────────────────────┴───────────────────┴─────────────────────┘
```

### 4.2 Why This Works

1. **Economic barrier**: Each fake producer requires a real bond deposit
2. **Time barrier**: 30-day minimum before any voting power
3. **Asymmetric power**: Veterans have 4x the voting power of newcomers
4. **Convergence**: After 4 years, legitimate producers reach parity

**Critical point**: To block a legitimate security update, an attacker would need 40% of the total weighted voting power. With seniority weighting, this requires either massive capital (many producers) or multi-year planning (few high-weight producers). Both are impractical for most attack scenarios.

### 4.3 The 4-Year Convergence

```
Year 0   Year 1   Year 2   Year 3   Year 4   Year 5+
  │        │        │        │        │        │
  ▼        ▼        ▼        ▼        ▼        ▼
 1.0x    1.75x    2.50x    3.25x    4.00x    4.00x
  │        │        │        │        │        │
  └────────┴────────┴────────┴────────┴────────┘
              Seniority advantage period
                                       │
                                       ▼
                              All producers equal
```

The "privilege" of early adopters has an expiration date. After 4 years, anyone who joined can match the voting power of the founders. This is NOT a permanent oligarchy.

### 4.4 Maintainer vs Producer Governance

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│                    TWO-LAYER GOVERNANCE MODEL                                    │
├─────────────────────────────────────────────────────────────────────────────────┤
│                                                                                 │
│  MAINTAINERS (5 people)                                                         │
│  ├── Role: Propose and sign releases                                            │
│  ├── Power: 3/5 needed to publish update                                        │
│  ├── Selection: First 5 producers + on-chain changes                            │
│  └── Cannot: Force update without community consent                             │
│                                                                                 │
│  PRODUCERS (unlimited)                                                          │
│  ├── Role: Review and veto releases                                             │
│  ├── Power: 40% weighted vote to reject                                         │
│  ├── Selection: Anyone who bonds and registers                                  │
│  └── Cannot: Propose releases (only react)                                      │
│                                                                                 │
│  BALANCE:                                                                       │
│  ├── Maintainers propose → Producers approve/veto                               │
│  ├── Small group for efficiency → Large group for legitimacy                    │
│  └── Technical expertise → Democratic oversight                                 │
│                                                                                 │
└─────────────────────────────────────────────────────────────────────────────────┘
```

---

## 5. Complete Update Timeline

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│                          COMPLETE UPDATE TIMELINE                                │
├─────────────────────────────────────────────────────────────────────────────────┤
│                                                                                 │
│  DAY 0: RELEASE PUBLISHED                                                       │
│  ├── Maintainers build and test binaries for all platforms                      │
│  ├── 3 of 5 maintainers sign the release offline                                │
│  ├── Release published to GitHub and mirrors                                    │
│  └── All nodes display mandatory notification                                   │
│                                                                                 │
│  DAYS 0-7: VETO PERIOD                                                          │
│  ├── Producers review changelog and code changes                                │
│  ├── Community discussion on forums/Discord                                     │
│  ├── Producers submit votes (can change until deadline)                         │
│  └── Real-time veto percentage displayed on all nodes                           │
│                                                                                 │
│  DAY 7: RESOLUTION                                                              │
│  ├── Weighted votes tallied at exact deadline                                   │
│  ├── If veto >= 40%: Update REJECTED, discarded                                 │
│  └── If veto <  40%: Update APPROVED, grace period begins                       │
│                                                                                 │
│  HOUR 2-3: GRACE PERIOD (1 epoch, ~1h)                                          │
│  ├── Approved update downloaded and verified                                    │
│  ├── Operators can manually apply early: doli-node update apply                 │
│  └── Outdated nodes can still produce blocks                                    │
│                                                                                 │
│  DAY 9+: ENFORCEMENT ACTIVE                                                     │
│  ├── Nodes below required version: production PAUSED                            │
│  ├── Outdated nodes can still sync, serve RPC, relay transactions               │
│  └── Update and restart to resume production                                    │
│                                                                                 │
│  Total notice before enforcement: 9 DAYS                                        │
│                                                                                 │
└─────────────────────────────────────────────────────────────────────────────────┘
```

### 5.1 Producer Notifications

Producers receive automatic notifications through three channels:

1. **Banner on ANY CLI command** while update is pending
2. **`doli-node update status`** for full details
3. **Periodic log messages** (every 6 hours)

The notification content changes based on the current state:

#### State 1: VOTING PERIOD (Days 0-7) - Not Yet Voted

**Banner (on any CLI command):**
```
╔═════════════════════════════════════════════════════════════════════════════════════╗
║  ⚠️  UPDATE 1.0.1  |  5d left  |  doli-node update vote --veto --key <key.json>     ║
╚═════════════════════════════════════════════════════════════════════════════════════╝
```

**Full status (`doli-node update status`):**
```
╔══════════════════════════════════════════════════════════════════╗
║                    ⚠️  UPDATE PENDING - VOTE NOW                  ║
╠══════════════════════════════════════════════════════════════════╣
║                                                                  ║
║  VERSION                                                         ║
║  Current: 1.0.0                                                  ║
║  New:     1.0.1                                                  ║
║                                                                  ║
║  CHANGELOG                                                       ║
║    - Security fix for VDF verification bypass                    ║
║    - Performance improvement in block propagation                ║
║    - Fix memory leak in peer connection handler                  ║
║                                                                  ║
║  MAINTAINER SIGNATURES                                           ║
║  ✓ Alice (maintainer #1)                                         ║
║  ✓ Bob (maintainer #2)                                           ║
║  ✓ Carol (maintainer #3)                                         ║
║  3/5 signatures verified ✓                                       ║
║                                                                  ║
║  VOTING                                                          ║
║  Veto:        15.5% of 40% threshold                             ║
║  Time left:   5 days, 12 hours                                   ║
║  Projection:  WILL PASS                                          ║
║                                                                  ║
║  YOUR PRODUCER                                                   ║
║  Your vote:   NOT VOTED YET                                      ║
║  Your weight: 2.5x (2 years seniority)                           ║
║                                                                  ║
╠══════════════════════════════════════════════════════════════════╣
║  To veto:    doli-node update vote --veto --key <key.json>       ║
║  To approve: doli-node update vote --approve --key <key.json>    ║
╚══════════════════════════════════════════════════════════════════╝
```

#### State 2: VOTING PERIOD - Already Voted

**Banner:**
```
╔═════════════════════════════════════════════════════════════════════════════════════╗
║  ⚠️  UPDATE 1.0.1  |  Veto: 15%/40%  |  5d left  |  You voted: VETO ✓               ║
╚═════════════════════════════════════════════════════════════════════════════════════╝
```

#### State 3: GRACE PERIOD (Days 7-9)

**Banner:**
```
╔═════════════════════════════════════════════════════════════════════════════════════╗
║  ✅  UPDATE 1.0.1 APPROVED  |  36h left  |  doli-node update apply                  ║
╚═════════════════════════════════════════════════════════════════════════════════════╝
```

#### State 4: PRODUCTION PAUSED (Day 9+, not updated)

**Banner:**
```
╔═════════════════════════════════════════════════════════════════════════════════════╗
║  🚫  PRODUCTION PAUSED - outdated  |  doli-node update apply                        ║
╚═════════════════════════════════════════════════════════════════════════════════════╝
```

#### State 5: UPDATE REJECTED

**Banner:**
```
╔═════════════════════════════════════════════════════════════════════════════════════╗
║  ❌  UPDATE 1.0.1 REJECTED by community  |  No action required                      ║
╚═════════════════════════════════════════════════════════════════════════════════════╝
```

#### State 6: ROLLBACK OCCURRED

**Banner:**
```
╔═════════════════════════════════════════════════════════════════════════════════════╗
║  ⚠️  ROLLBACK: 1.0.1 failed, reverted to 1.0.0  |  doli-node update status          ║
╚═════════════════════════════════════════════════════════════════════════════════════╝
```

#### State 7: HARD FORK PENDING

**Banner:**
```
╔═════════════════════════════════════════════════════════════════════════════════════╗
║  🔴  HARD FORK 2.0.0  |  12d to activation  |  doli-node update apply               ║
╚═════════════════════════════════════════════════════════════════════════════════════╝
```

#### State 8: UP TO DATE

**Banner:** None (no banner shown when up to date)

### 5.2 Version Enforcement: "No Update = No Produce"

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│                      "NO ACTUALIZAS = NO PRODUCES"                               │
├─────────────────────────────────────────────────────────────────────────────────┤
│                                                                                 │
│  PRINCIPLE: If your node is a security hole for the network,                    │
│             you shouldn't be producing blocks.                                  │
│                                                                                 │
│  THIS IS NOT PUNISHMENT. It's network protection.                               │
│                                                                                 │
├─────────────────────────────────────────────────────────────────────────────────┤
│                                                                                 │
│  What outdated nodes CAN do:          What outdated nodes CANNOT do:            │
│  ✅ Sync the chain                    ❌ Produce blocks                          │
│  ✅ Serve RPC requests                ❌ Earn rewards                            │
│  ✅ Relay transactions                                                          │
│  ✅ Validate blocks                                                             │
│  ✅ Vote on future updates                                                      │
│                                                                                 │
└─────────────────────────────────────────────────────────────────────────────────┘
```

---

## 6. Automatic Rollback System

A critical addition to ensure network stability: automatic rollback when updates cause node failures.

### 6.1 Crash Detection Watchdog

The node process is monitored by a lightweight watchdog that detects repeated crashes after an update:

```rust
pub struct UpdateWatchdog {
    last_update_version: Option<String>,
    last_update_time: Option<Timestamp>,
    crash_count: u32,
    crash_window: Duration,  // 1 hour
}

impl UpdateWatchdog {
    pub fn should_rollback(&self) -> bool {
        self.crash_count >= CRASH_THRESHOLD  // 3
            && self.within_crash_window()
            && self.recently_updated()
    }
}
```

### 6.2 Rollback Trigger Conditions

1. Node crashes **3+ times** within 1 hour of update application
2. Node fails to reach sync within 5 minutes of restart
3. Node fails health checks (RPC unresponsive, peers disconnected)

### 6.3 Rollback Process

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│                          AUTOMATIC ROLLBACK PROCESS                              │
├─────────────────────────────────────────────────────────────────────────────────┤
│                                                                                 │
│  [Crash detected]                                                               │
│        │                                                                        │
│        ▼                                                                        │
│  crash_count++                                                                  │
│        │                                                                        │
│        ▼                                                                        │
│  ┌─────────────────────┐                                                        │
│  │ crash_count >= 3 && │──No──► [Restart normally]                              │
│  │ within_window?      │                                                        │
│  └──────────┬──────────┘                                                        │
│             │ Yes                                                               │
│             ▼                                                                   │
│  ┌─────────────────────┐                                                        │
│  │ 1. Stop node        │                                                        │
│  │ 2. Copy backup      │  doli-node.backup → doli-node                          │
│  │ 3. Clear state      │                                                        │
│  │ 4. Log rollback     │  "ROLLBACK: Reverted to {version} due to {reason}"     │
│  │ 5. Restart node     │                                                        │
│  │ 6. Alert operator   │  Webhook notification (optional)                       │
│  └─────────────────────┘                                                        │
│             │                                                                   │
│             ▼                                                                   │
│  [Node running on previous version]                                             │
│                                                                                 │
└─────────────────────────────────────────────────────────────────────────────────┘
```

### 6.4 Post-Rollback Behavior

- Node continues operating on previous version
- Update marked as "failed locally" (not network-wide rejection)
- Operator notified via logs and optional webhook
- Manual intervention required to retry update
- Node can still produce blocks (if previous version meets requirements)

---

## 7. Hard Fork Support

While most updates are backward-compatible, some protocol changes require coordinated hard forks. The system includes an optional upgrade-at-height mechanism.

### 7.1 When Hard Forks Are Needed

- Changes to block structure or validation rules
- Changes to consensus algorithm parameters (e.g., BOND_UNIT, MAX_FALLBACK_RANK)
- State migration or database format changes
- Cryptographic algorithm upgrades

### 7.2 Upgrade-at-Height Mechanism

Hard fork releases include an activation height in the release metadata:

```json
{
    "version": "2.0.0",
    "binary_sha256": "abc123...",
    "hard_fork": true,
    "activation_height": 1000000,
    "min_version_at_height": "2.0.0",
    "changelog": "Protocol upgrade: new block format",
    "signatures": [...]
}
```

### 7.3 Hard Fork Timeline

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│                            HARD FORK TIMELINE (~30 days)                         │
├─────────────────────────────────────────────────────────────────────────────────┤
│                                                                                 │
│  Day 0        Day 7              Day 9                          Day ~30         │
│    │            │                  │                               │            │
│    ▼            ▼                  ▼                               ▼            │
│  Release     Veto period       Grace ends                    Activation        │
│  published   ends              (if approved)                 height reached    │
│    │            │                  │                               │            │
│    └────────────┴──────────────────┴───────────────────────────────┤            │
│    │◄── 2 epochs ─►│◄── 1 epoch ──►│◄────── ~21 days ─────────────►│            │
│                                                                   │            │
│                                                     At activation_height:       │
│                                                     ├── New rules take effect   │
│                                                     ├── Scheduler recalculates  │
│                                                     └── Old nodes fork off      │
│                                                                                 │
└─────────────────────────────────────────────────────────────────────────────────┘
```

### 7.4 Scheduler Recalculation on Hard Fork

If a hard fork changes scheduler parameters:

```rust
// At activation_height, scheduler params change:
// Before: BOND_UNIT = 100 DOLI, MAX_FALLBACK_RANK = 9
// After:  BOND_UNIT = 50 DOLI, MAX_FALLBACK_RANK = 14

fn on_block_applied(&mut self, block: &Block) {
    if block.height == HARD_FORK_ACTIVATION_HEIGHT {
        // Recalculate scheduler with new parameters
        let new_params = ConsensusParams::for_version("2.0.0");
        self.scheduler = DeterministicScheduler::new(
            self.producer_set.active_producers(),
            new_params.bond_unit(),
            new_params.max_fallback_rank(),
        );
        info!("Hard fork activated: scheduler recalculated");
    }
}
```

### 7.5 Soft Update vs Hard Fork Comparison

```
┌─────────────────────┬─────────────────────┬─────────────────────┐
│ Aspect              │ Soft Update         │ Hard Fork           │
├─────────────────────┼─────────────────────┼─────────────────────┤
│ Backward compatible │ Yes                 │ No                  │
│ Old nodes can sync  │ Yes                 │ No (fork off)       │
│ Activation          │ Immediate (grace)   │ At specific height  │
│ Veto period         │ 2 epochs (~2h)      │ 2 epochs (~2h)      │
│ Total notice        │ ~3 hours            │ ~30 days            │
│ Rollback possible   │ Yes (automatic)     │ No (chain diverged) │
│ Network split risk  │ None                │ Yes (if not ready)  │
└─────────────────────┴─────────────────────┴─────────────────────┘
```

---

## 8. Security Model

### 8.1 Threat Analysis

```
┌────────────────────┬─────────────────────────┬─────────────────────────┬──────────────┐
│ Threat             │ Attack Vector           │ Mitigation              │ Risk Level   │
├────────────────────┼─────────────────────────┼─────────────────────────┼──────────────┤
│ Rogue maintainer   │ Signs backdoored        │ Requires 3/5            │ Low          │
│                    │ binary                  │ signatures              │ (collusion)  │
├────────────────────┼─────────────────────────┼─────────────────────────┼──────────────┤
│ Key compromise     │ Attacker signs          │ Still need 2 more       │ None         │
│ (1 key)            │ releases                │ keys                    │              │
├────────────────────┼─────────────────────────┼─────────────────────────┼──────────────┤
│ Key compromise     │ Attacker signs          │ Still need 1 more       │ None         │
│ (2 keys)           │ releases                │ key                     │              │
├────────────────────┼─────────────────────────┼─────────────────────────┼──────────────┤
│ Key compromise     │ Attacker signs          │ Community can veto      │ Medium       │
│ (3 keys)           │ releases                │ within 2 epochs (~2h)   │              │
├────────────────────┼─────────────────────────┼─────────────────────────┼──────────────┤
│ Sybil veto         │ Block legitimate        │ Seniority weighting     │ Low          │
│ attack             │ updates                 │ + bond + 30-day min     │              │
├────────────────────┼─────────────────────────┼─────────────────────────┼──────────────┤
│ Fake maintainer    │ Claim maintainer        │ On-chain verification   │ None         │
│                    │ status                  │ from blockchain         │              │
├────────────────────┼─────────────────────────┼─────────────────────────┼──────────────┤
│ Mirror compromise  │ Serve malicious         │ SHA-256 hash            │ None         │
│                    │ binary                  │ verification            │              │
├────────────────────┼─────────────────────────┼─────────────────────────┼──────────────┤
│ Rollback attack    │ Force old vulnerable    │ Version comparison      │ None         │
│                    │ version                 │ (no downgrades)         │              │
└────────────────────┴─────────────────────────┴─────────────────────────┴──────────────┘
```

### 8.2 Defense in Depth

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│                              DEFENSE LAYERS                                      │
├─────────────────────────────────────────────────────────────────────────────────┤
│                                                                                 │
│  Layer 1: CRYPTOGRAPHIC                                                         │
│  ├── 3/5 multisig for releases                                                  │
│  ├── SHA-256 binary verification                                                │
│  ├── Ed25519 signatures on all messages                                         │
│  └── On-chain maintainer verification                                           │
│                                                                                 │
│  Layer 2: GOVERNANCE                                                            │
│  ├── 40% weighted veto threshold                                                │
│  ├── 2-epoch mandatory review period (~2h)                                      │
│  ├── Vote changing allowed (react to new info)                                  │
│  └── Transparent maintainer set (derived from chain)                            │
│                                                                                 │
│  Layer 3: ECONOMIC                                                              │
│  ├── Bond requirement for producers                                             │
│  ├── Seniority weighting (max 4x)                                               │
│  └── 30-day minimum voting age                                                  │
│                                                                                 │
│  Layer 4: OPERATIONAL                                                           │
│  ├── Automatic rollback on failure                                              │
│  ├── Backup preservation before update                                          │
│  └── Health monitoring and alerting                                             │
│                                                                                 │
└─────────────────────────────────────────────────────────────────────────────────┘
```

---

## 9. CLI Command Reference

### 9.1 Update Management

```bash
# Check for available updates
doli-node update check

# Show detailed update status
doli-node update status

# Apply approved update (after veto period)
doli-node update apply

# Force apply (bypasses approval check, NOT veto period)
doli-node update apply --force

# Manual rollback to backup
doli-node update rollback

# Verify release signatures
doli-node update verify --version 1.0.1
```

### 9.2 Voting (Producers Only)

```bash
# Vote to VETO (block) an update
doli-node update vote --veto --version 1.0.1 --key /path/to/producer.json

# Vote to APPROVE an update
doli-node update vote --approve --version 1.0.1 --key /path/to/producer.json

# View current vote status for a version
doli-node update votes --version 1.0.1
```

### 9.3 Maintainer Management

```bash
# View current maintainer set
doli-node maintainer list

# Output:
# Maintainer Set (5/5)
# ┌───┬────────────────┬─────────────────┬────────────────┐
# │ # │ Public Key     │ Producer Since  │ Status         │
# ├───┼────────────────┼─────────────────┼────────────────┤
# │ 1 │ doli1abc...    │ Block 1         │ Active         │
# │ 2 │ doli1def...    │ Block 2         │ Active         │
# │ 3 │ doli1ghi...    │ Block 5         │ Active         │
# │ 4 │ doli1jkl...    │ Block 8         │ Unbonding      │
# │ 5 │ doli1mno...    │ Block 12        │ Active         │
# └───┴────────────────┴─────────────────┴────────────────┘
# Threshold: 3 of 5 signatures required

# Propose removing a maintainer (requires 3/5 signatures)
doli-node maintainer remove --target doli1jkl... --key /path/to/maintainer.json

# Propose adding a maintainer (requires 3/5 signatures)
doli-node maintainer add --target doli1xyz... --key /path/to/maintainer.json

# Sign a pending maintainer change proposal
doli-node maintainer sign --proposal-id 12345 --key /path/to/maintainer.json
```

### 9.4 Node Run Options

```bash
# Run with auto-updates enabled (default)
doli-node run --network mainnet

# Disable auto-updates entirely
doli-node run --network mainnet --no-auto-update

# Notify only (check but don't apply)
doli-node run --network mainnet --update-notify-only

# Disable automatic rollback
doli-node run --network mainnet --no-auto-rollback

# Full production setup
doli-node run \
  --network mainnet \
  --producer \
  --producer-key /path/to/producer.json \
  --rpc-bind 0.0.0.0:28545
```

---

## 10. RPC Endpoints

### 10.1 getMaintainerSet

```json
// Request
{
  "jsonrpc": "2.0",
  "method": "getMaintainerSet",
  "params": {},
  "id": 1
}

// Response
{
  "jsonrpc": "2.0",
  "result": {
    "maintainers": [
      {
        "pubkey": "doli1abc...",
        "registered_at_block": 1,
        "is_active_producer": true
      },
      {
        "pubkey": "doli1def...",
        "registered_at_block": 2,
        "is_active_producer": true
      }
      // ... 3 more
    ],
    "threshold": 3,
    "last_change_block": 50000
  },
  "id": 1
}
```

### 10.2 getUpdateStatus

```json
// Request
{
  "jsonrpc": "2.0",
  "method": "getUpdateStatus",
  "params": {},
  "id": 1
}

// Response
{
  "jsonrpc": "2.0",
  "result": {
    "current_version": "1.0.0",
    "pending_update": {
      "version": "1.0.1",
      "changelog": "Security fix for VDF verification",
      "maintainer_signatures": [
        {"pubkey": "doli1abc...", "verified": true},
        {"pubkey": "doli1def...", "verified": true},
        {"pubkey": "doli1ghi...", "verified": true}
      ],
      "veto_percent": 15.5,
      "veto_weight": 45.25,
      "total_weight": 291.50,
      "time_remaining_secs": 432000,
      "status": "voting",
      "hard_fork": false
    }
  },
  "id": 1
}
```

### 10.3 submitVote

```json
// Request
{
  "jsonrpc": "2.0",
  "method": "submitVote",
  "params": {
    "version": "1.0.1",
    "vote": "veto",
    "producer_pubkey": "abc123...",
    "timestamp": 1704067200,
    "signature": "def456..."
  },
  "id": 1
}

// Response
{
  "jsonrpc": "2.0",
  "result": {
    "status": "accepted",
    "message": "Vote submitted and broadcast",
    "your_weight": 3.25,
    "replaces_previous": true
  },
  "id": 1
}
```

### 10.4 submitMaintainerChange

```json
// Request
{
  "jsonrpc": "2.0",
  "method": "submitMaintainerChange",
  "params": {
    "action": "remove",
    "target_pubkey": "doli1jkl...",
    "signatures": [
      {"pubkey": "doli1abc...", "signature": "..."},
      {"pubkey": "doli1def...", "signature": "..."},
      {"pubkey": "doli1ghi...", "signature": "..."}
    ],
    "reason": "Inactive for 6 months"
  },
  "id": 1
}

// Response
{
  "jsonrpc": "2.0",
  "result": {
    "status": "accepted",
    "tx_hash": "0x123...",
    "new_maintainer_count": 4
  },
  "id": 1
}
```

---

## 11. Implementation Reference

### 11.1 File Locations

```
┌─────────────────────────────────┬─────────────────────────────────────────────┐
│ Component                       │ Path                                        │
├─────────────────────────────────┼─────────────────────────────────────────────┤
│ Maintainer set management       │ crates/core/src/maintainer.rs               │
│ Maintainer transactions         │ crates/core/src/transaction.rs              │
│ Maintainer validation           │ crates/core/src/validation.rs               │
│ Core updater library            │ crates/updater/src/lib.rs                   │
│ Seniority voting                │ crates/updater/src/seniority.rs             │
│ Vote tracking                   │ crates/updater/src/vote.rs                  │
│ Watchdog (rollback)             │ crates/updater/src/watchdog.rs              │
│ Hard fork logic                 │ crates/updater/src/hardfork.rs              │
│ Binary download                 │ crates/updater/src/download.rs              │
│ Binary verification             │ crates/updater/src/verify.rs                │
│ Update application              │ crates/updater/src/apply.rs                 │
│ Node integration                │ bins/node/src/updater.rs                    │
│ CLI commands                    │ bins/cli/src/update.rs                      │
│ Maintainer CLI                  │ bins/cli/src/maintainer.rs                  │
│ RPC methods                     │ crates/rpc/src/update.rs                    │
│ Gossip topics                   │ crates/network/src/gossip.rs                │
└─────────────────────────────────┴─────────────────────────────────────────────┘
```

### 11.2 Key Data Structures

```rust
/// Maintainer set derived from blockchain
pub struct MaintainerSet {
    pub members: Vec<PublicKey>,
    pub threshold: usize,
    pub last_updated: u64,
}

/// Transaction to modify maintainer set
pub enum MaintainerTxType {
    RemoveMaintainer = 13,
    AddMaintainer = 14,
}

pub struct MaintainerChangeData {
    pub target: PublicKey,
    pub signatures: Vec<MaintainerSignature>,
    pub reason: Option<String>,
}

/// Producer seniority for voting
pub struct ProducerSeniority {
    pub pubkey: PublicKey,
    pub registration_height: u64,
    pub years_active: f64,
    pub vote_weight: f64,
}

/// Release metadata
pub struct Release {
    pub version: String,
    pub binary_sha256: [u8; 32],
    pub binary_url_template: String,
    pub changelog: String,
    pub published_at: u64,
    pub signatures: Vec<MaintainerSignature>,
    pub hard_fork: Option<HardForkInfo>,
}

/// Hard fork activation info
pub struct HardForkInfo {
    pub activation_height: u64,
    pub min_version: String,
    pub consensus_changes: Vec<String>,
}

/// Update watchdog state
pub struct UpdateWatchdog {
    pub last_update_version: Option<String>,
    pub last_update_time: Option<u64>,
    pub crash_count: u32,
    pub crash_timestamps: Vec<u64>,
}
```

### 11.3 Constants

```rust
// Maintainer management
pub const INITIAL_MAINTAINER_COUNT: usize = 5;
pub const MAINTAINER_THRESHOLD: usize = 3;
pub const MIN_MAINTAINERS: usize = 3;
pub const MAX_MAINTAINERS: usize = 5;

// Timing
pub const VETO_PERIOD: Duration = Duration::from_secs(2 * 3600);          // 2 epochs (~2h)
pub const GRACE_PERIOD: Duration = Duration::from_secs(3600);             // 1 epoch (~1h)
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);       // 6 hours

// Thresholds
pub const VETO_THRESHOLD_PERCENT: u8 = 40;

// Seniority
pub const MAX_SENIORITY_MULTIPLIER: f64 = 4.0;
pub const SENIORITY_MATURITY_YEARS: f64 = 4.0;
pub const MIN_VOTING_AGE_DAYS: u64 = 30;

// Rollback
pub const CRASH_THRESHOLD: u32 = 3;
pub const CRASH_WINDOW: Duration = Duration::from_secs(3600);             // 1 hour

// Distribution
pub const GITHUB_API_URL: &str = "https://api.github.com/repos/doli-network/doli/releases/latest";
pub const FALLBACK_MIRROR: &str = "https://releases.doli.network";
```

---

## 12. Comparison with Other Blockchains

```
┌─────────────────────┬────────────┬────────────┬────────────┬─────────────────┐
│ Feature             │ Bitcoin    │ Ethereum   │ Solana     │ DOLI            │
├─────────────────────┼────────────┼────────────┼────────────┼─────────────────┤
│ Maintainer source   │ Core devs  │ EF + devs  │ Foundation │ First 5         │
│                     │ (social)   │ (social)   │ (central)  │ producers       │
├─────────────────────┼────────────┼────────────┼────────────┼─────────────────┤
│ Maintainer changes  │ Social     │ Social     │ Internal   │ On-chain        │
│                     │ consensus  │ consensus  │ decision   │ 3/5 multisig    │
├─────────────────────┼────────────┼────────────┼────────────┼─────────────────┤
│ Update mechanism    │ BIP        │ EIP +      │ Foundation │ Auto-update     │
│                     │ process    │ hard fork  │ push       │ + veto          │
├─────────────────────┼────────────┼────────────┼────────────┼─────────────────┤
│ Voting system       │ Miner      │ None       │ None       │ Seniority-      │
│                     │ signaling  │ (social)   │            │ weighted        │
├─────────────────────┼────────────┼────────────┼────────────┼─────────────────┤
│ Veto power          │ 95%        │ Social     │ None       │ 40% weighted    │
│                     │ threshold  │ consensus  │            │                 │
├─────────────────────┼────────────┼────────────┼────────────┼─────────────────┤
│ Time to update      │ Months to  │ Weeks to   │ Hours      │ 9 days          │
│                     │ years      │ months     │            │ minimum         │
├─────────────────────┼────────────┼────────────┼────────────┼─────────────────┤
│ Automatic apply     │ No         │ No         │ Yes        │ Yes             │
│                     │            │            │            │ (with veto)     │
├─────────────────────┼────────────┼────────────┼────────────┼─────────────────┤
│ Automatic rollback  │ No         │ No         │ No         │ Yes             │
├─────────────────────┼────────────┼────────────┼────────────┼─────────────────┤
│ Sybil resistance    │ Hashpower  │ Stake      │ Stake      │ Seniority +     │
│                     │            │            │            │ bond + time     │
└─────────────────────┴────────────┴────────────┴────────────┴─────────────────┘
```

**DOLI's advantages**:
1. **No hardcoded authority**: Maintainers emerge from blockchain participation
2. **Transparent transitions**: All maintainer changes are on-chain and verifiable
3. **Democratic oversight**: Producers can veto any release
4. **Operational efficiency**: Auto-update with rollback prevents stale nodes
5. **Sybil resistant**: Time-weighted voting prevents plutocracy and attacks

---

## 13. Frequently Asked Questions

### Q: How are the initial maintainers chosen?

**Automatically.** The first 5 producers to register become maintainers. There's no pre-selection, no governance vote, no configuration file. The blockchain itself determines who the maintainers are.

### Q: Can maintainers force an update without community consent?

**No.** Even with 3/5 maintainer signatures, the community has 2 epochs (~2 hours) to review and veto. If 40% of weighted voting power objects, the update is rejected. Maintainers propose; the community disposes.

### Q: How do I verify who the maintainers are?

```bash
doli-node maintainer list
# Shows current maintainers derived from blockchain

doli-node maintainer verify --pubkey doli1abc...
# Verifies if a specific key is a maintainer
```

Any node can independently verify by scanning registration transactions from genesis.

### Q: What if a maintainer goes rogue or loses their keys?

The other maintainers can remove them with a 3/5 vote:
1. Prepare RemoveMaintainer transaction
2. Collect 3+ signatures from other maintainers
3. Submit transaction to chain
4. Maintainer set updates automatically

### Q: Can I become a maintainer?

If there's a vacancy (fewer than 5 maintainers), existing maintainers can add you:
1. You must be a registered producer
2. 3/5 current maintainers must sign an AddMaintainer transaction
3. Transaction is submitted to chain
4. You become a maintainer

### Q: What happens if my node crashes after an update?

The watchdog automatically rolls back to the previous version after 3 crashes within 1 hour. You can also manually rollback with `doli-node update rollback`. Your backup is always preserved.

### Q: Why is voting weight based on seniority, not stake?

Separation of concerns:
- **Block production** rewards capital commitment (stake)
- **Governance** rewards long-term participation (time)

This prevents wealthy newcomers from immediately controlling governance while still allowing them to earn proportional block rewards.

### Q: What's the difference between "approve" and not voting?

Functionally the same. The system uses **veto-based** governance:
- Updates pass by default unless blocked
- Only VETO votes count toward the 40% threshold
- Abstaining = implicit approval

This prevents "voter apathy" from blocking important security updates.

### Q: Can I change my vote?

**Yes.** You can change your vote at any time during the 2-epoch veto period. Only your latest vote (by timestamp) counts at the deadline. This allows reaction to new information discovered during review.

### Q: What if fewer than 5 producers ever register?

The maintainer set will have fewer members, and the threshold adjusts proportionally:
- 3 maintainers → 2/3 required
- 4 maintainers → 3/4 required
- 5 maintainers → 3/5 required

The network can still function with as few as 1 maintainer, though this is not recommended for security.

### Q: Can a hard fork change the maintainer set?

Yes, but it's not necessary. Hard forks can change consensus parameters, block format, etc. Maintainer changes should use the normal on-chain process (AddMaintainer/RemoveMaintainer transactions).

---

## Document Information

- **Version**: 3.0
- **Last Updated**: February 2026
- **Status**: Production Specification

### Changes from v2.0

- ✅ Added maintainer bootstrap system (first 5 producers)
- ✅ Added on-chain maintainer management (Add/Remove transactions)
- ✅ Clarified relationship between voting weight (seniority) and block production (bonds)
- ✅ Added maintainer CLI commands and RPC endpoints
- ✅ Added edge case handling for maintainer set
- ✅ Updated security model for on-chain maintainer verification
- ✅ Added hard fork scheduler recalculation documentation
- ✅ Expanded FAQ with maintainer-related questions

---

*DOLI Protocol - Decentralized, Democratic, Secure*

# DOLI Auto-Update System

## Complete Technical Specification v2.0

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Governance and Voting System](#2-governance-and-voting-system)
3. [Sybil Resistance Analysis](#3-sybil-resistance-analysis)
4. [Complete Update Timeline](#4-complete-update-timeline)
5. [Automatic Rollback System](#5-automatic-rollback-system)
6. [Hard Fork Support](#6-hard-fork-support)
7. [Security Model](#7-security-model)
8. [CLI Command Reference](#8-cli-command-reference)
9. [RPC Endpoints](#9-rpc-endpoints)
10. [Implementation Reference](#10-implementation-reference)
11. [Comparison with Other Blockchains](#11-comparison-with-other-blockchains)
12. [Frequently Asked Questions](#12-frequently-asked-questions)

---

## 1. Executive Summary

The DOLI auto-update system is a decentralized, cryptographically secure mechanism for coordinating software updates across the network. It balances the need for rapid security patches with democratic governance, ensuring no single party can force malicious updates on the network.

### 1.1 Key Features

- **Transparent updates**: All releases publicly signed and verifiable on-chain
- **Seniority-weighted voting**: 40% threshold to veto, weighted by producer tenure (max 4x at 4 years)
- **Automatic application**: Updates applied after veto period with automatic rollback on failure
- **Multi-signature releases**: 3/5 maintainer signatures required for any release
- **Version enforcement**: Outdated producers paused from block production after grace period
- **Hard fork support**: Optional upgrade-at-height mechanism for breaking protocol changes

### 1.2 Key Constants

```
┌────────────────────────────┬───────────┬─────────────────────────────────────────┐
│ Constant                   │ Value     │ Description                             │
├────────────────────────────┼───────────┼─────────────────────────────────────────┤
│ VETO_PERIOD                │ 7 days    │ Time for producers to vote on updates   │
│ GRACE_PERIOD               │ 48 hours  │ Time after approval before enforcement  │
│ VETO_THRESHOLD_PERCENT     │ 40%       │ Weighted percentage needed to reject    │
│ REQUIRED_SIGNATURES        │ 3 of 5    │ Maintainer signatures needed            │
│ CHECK_INTERVAL             │ 6 hours   │ How often nodes check for updates       │
│ MAX_SENIORITY_MULTIPLIER   │ 4x        │ Maximum vote weight for seniors         │
│ SENIORITY_MATURITY_YEARS   │ 4         │ Years to reach maximum seniority        │
│ MIN_VOTING_AGE_DAYS        │ 30        │ Minimum days as producer to vote        │
│ CRASH_THRESHOLD            │ 3         │ Consecutive crashes before rollback     │
└────────────────────────────┴───────────┴─────────────────────────────────────────┘
```

---

## 2. Governance and Voting System

The governance model uses seniority-weighted voting to balance democratic participation with Sybil resistance. This ensures that established, long-term participants have proportionally more influence while still allowing newcomers to participate meaningfully.

### 2.1 Seniority-Weighted Voting

Unlike simple count-based voting (1 producer = 1 vote), DOLI uses seniority weighting that rewards long-term commitment to the network.

#### 2.1.1 Weight Calculation

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

#### 2.1.2 Minimum Voting Age

To prevent flash Sybil attacks, producers must be active for at least **30 days** before their votes count. This creates a significant cost barrier: an attacker would need to fund and maintain fake producers for a month before gaining any voting power.

#### 2.1.3 Veto Threshold Calculation

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

### 2.2 Vote Lifecycle

#### 2.2.1 Vote Submission

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

#### 2.2.2 Vote Changing

Unlike count-based systems, producers **CAN change their vote** during the veto period. This allows reaction to new information (e.g., discovery of a backdoor in the update).

- Each new vote from the same producer replaces their previous vote
- Only the latest vote (by timestamp) counts at deadline
- All vote history is preserved for transparency

#### 2.2.3 Vote Finalization

Votes are counted at the exact moment the veto period expires. The result is deterministic: any node can independently verify the outcome by replaying all votes received before the deadline.

---

## 3. Sybil Resistance Analysis

The combination of bond requirements, seniority weighting, and minimum voting age creates strong Sybil resistance.

### 3.1 Attack Cost Analysis

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

### 3.2 Why This Works

1. **Economic barrier**: Each fake producer requires a real bond deposit
2. **Time barrier**: 30-day minimum before any voting power
3. **Asymmetric power**: Veterans have 4x the voting power of newcomers
4. **Convergence**: After 4 years, legitimate producers reach parity

**Critical point**: To block a legitimate security update, an attacker would need 40% of the total weighted voting power. With seniority weighting, this requires either massive capital (many producers) or multi-year planning (few high-weight producers). Both are impractical for most attack scenarios.

### 3.3 The 4-Year Convergence

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

---

## 4. Complete Update Timeline

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
│  DAYS 7-9: GRACE PERIOD (48 hours)                                              │
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

### 4.1 Producer Notifications

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
║  Details: doli-node update status                                ║
║  To veto: doli-node update vote --veto --key <key.json>          ║
║  Approve: doli-node update vote --approve --key <key.json>       ║
╚══════════════════════════════════════════════════════════════════╝
```

#### State 2: VOTING PERIOD (Days 0-7) - Already Voted

**Banner (on any CLI command):**
```
╔═════════════════════════════════════════════════════════════════════════════════════╗
║  ⚠️  UPDATE 1.0.1  |  Veto: 15%/40%  |  5d left  |  You voted: VETO ✓               ║
╚═════════════════════════════════════════════════════════════════════════════════════╝
```

**Full status shows:**
```
║  YOUR PRODUCER                                                   ║
║  Your vote:   VETO ✓                                             ║
║  Your weight: 2.5x (2 years seniority)                           ║
║  Voted at:    2026-01-10 14:32 UTC                               ║
║                                                                  ║
╠══════════════════════════════════════════════════════════════════╣
║  To change vote: doli-node update vote --approve --key <key>     ║
╚══════════════════════════════════════════════════════════════════╝
```

#### State 3: GRACE PERIOD (Days 7-9)

**Banner (on any CLI command):**
```
╔═════════════════════════════════════════════════════════════════════════════════════╗
║  ✅  UPDATE 1.0.1 APPROVED  |  36h left  |  doli-node update apply                  ║
╚═════════════════════════════════════════════════════════════════════════════════════╝
```

**Full status (`doli-node update status`):**
```
╔══════════════════════════════════════════════════════════════════╗
║                ✅  UPDATE APPROVED - UPDATE NOW                   ║
╠══════════════════════════════════════════════════════════════════╣
║                                                                  ║
║  VERSION                                                         ║
║  Current: 1.0.0                                                  ║
║  New:     1.0.1 (approved with 18% veto)                         ║
║                                                                  ║
║  GRACE PERIOD                                                    ║
║  Time left:   36 hours                                           ║
║  Enforcement: 2026-01-17 10:00 UTC                               ║
║                                                                  ║
║  ⚠️  After enforcement, outdated nodes CANNOT produce blocks     ║
║                                                                  ║
╠══════════════════════════════════════════════════════════════════╣
║  To update now: doli-node update apply                           ║
╚══════════════════════════════════════════════════════════════════╝
```

#### State 4: PRODUCTION PAUSED (Day 9+, not updated)

**Banner (on any CLI command):**
```
╔═════════════════════════════════════════════════════════════════════════════════════╗
║  🚫  PRODUCTION PAUSED - outdated  |  doli-node update apply                        ║
╚═════════════════════════════════════════════════════════════════════════════════════╝
```

**Full status (`doli-node update status`):**
```
╔══════════════════════════════════════════════════════════════════╗
║            🚫  PRODUCTION PAUSED - UPDATE REQUIRED                ║
╠══════════════════════════════════════════════════════════════════╣
║                                                                  ║
║  VERSION                                                         ║
║  Your version:     1.0.0 ❌                                       ║
║  Required version: 1.0.1                                         ║
║                                                                  ║
║  STATUS                                                          ║
║  Block production: PAUSED                                        ║
║  Sync:             ✓ Active (block 125,432)                      ║
║  RPC:              ✓ Active                                      ║
║  Rewards:          ❌ Not earning (not producing)                 ║
║                                                                  ║
║  You are missing approximately 2.5 DOLI per hour while paused.   ║
║                                                                  ║
╠══════════════════════════════════════════════════════════════════╣
║  To resume production: doli-node update apply                    ║
╚══════════════════════════════════════════════════════════════════╝
```

#### State 5: UPDATE REJECTED

**Banner (on any CLI command):**
```
╔═════════════════════════════════════════════════════════════════════════════════════╗
║  ❌  UPDATE 1.0.1 REJECTED by community  |  No action required                      ║
╚═════════════════════════════════════════════════════════════════════════════════════╝
```

**Full status (`doli-node update status`):**
```
╔══════════════════════════════════════════════════════════════════╗
║                ❌  UPDATE 1.0.1 REJECTED                          ║
╠══════════════════════════════════════════════════════════════════╣
║                                                                  ║
║  The community vetoed this update.                               ║
║                                                                  ║
║  Final veto: 45% (threshold: 40%)                                ║
║  Your vote:  VETO ✓                                              ║
║                                                                  ║
║  No action required. Your node continues normally.               ║
║                                                                  ║
╠══════════════════════════════════════════════════════════════════╣
║  Current version: 1.0.0 ✓                                        ║
║  Production:      Active ✓                                       ║
╚══════════════════════════════════════════════════════════════════╝
```

#### State 6: ROLLBACK OCCURRED

**Banner (on any CLI command):**
```
╔═════════════════════════════════════════════════════════════════════════════════════╗
║  ⚠️  ROLLBACK: 1.0.1 failed, reverted to 1.0.0  |  doli-node update status          ║
╚═════════════════════════════════════════════════════════════════════════════════════╝
```

**Full status (`doli-node update status`):**
```
╔══════════════════════════════════════════════════════════════════╗
║            ⚠️  ROLLBACK OCCURRED - UPDATE FAILED                  ║
╠══════════════════════════════════════════════════════════════════╣
║                                                                  ║
║  Update 1.0.1 crashed 3 times. Automatically reverted to 1.0.0.  ║
║                                                                  ║
║  Crash log: /var/log/doli/crash-1.0.1.log                        ║
║                                                                  ║
║  STATUS                                                          ║
║  Current version: 1.0.0 (restored from backup)                   ║
║  Production:      Active ✓                                       ║
║                                                                  ║
║  Please report this issue:                                       ║
║  https://github.com/e-weil/doli/issues                           ║
║                                                                  ║
╠══════════════════════════════════════════════════════════════════╣
║  To retry update: doli-node update apply --retry                 ║
╚══════════════════════════════════════════════════════════════════╝
```

#### State 7: HARD FORK PENDING

**Banner (on any CLI command):**
```
╔═════════════════════════════════════════════════════════════════════════════════════╗
║  🔴  HARD FORK 2.0.0  |  12d to activation  |  doli-node update apply               ║
╚═════════════════════════════════════════════════════════════════════════════════════╝
```

**Full status (`doli-node update status`):**
```
╔══════════════════════════════════════════════════════════════════╗
║          🔴  HARD FORK 2.0.0 - UPDATE MANDATORY                   ║
╠══════════════════════════════════════════════════════════════════╣
║                                                                  ║
║  VERSION                                                         ║
║  Current: 1.0.1                                                  ║
║  New:     2.0.0 (HARD FORK)                                      ║
║                                                                  ║
║  ACTIVATION                                                      ║
║  Height:    1,000,000 (current: 950,000)                         ║
║  Estimated: ~12 days (2026-01-27)                                ║
║                                                                  ║
║  ⚠️  CRITICAL: After activation height, nodes on 1.x.x will:     ║
║     - Fork off the main chain                                    ║
║     - Stop syncing new blocks                                    ║
║     - Be unable to produce or validate                           ║
║                                                                  ║
║  CHANGELOG                                                       ║
║    - New block format with extended headers                      ║
║    - Updated VDF parameters                                      ║
║    - State migration for new account structure                   ║
║                                                                  ║
╠══════════════════════════════════════════════════════════════════╣
║  To update: doli-node update apply                               ║
╚══════════════════════════════════════════════════════════════════╝
```

#### State 8: UP TO DATE

**Banner:** None (no banner shown when up to date)

**Full status (`doli-node update status`):**
```
╔══════════════════════════════════════════════════════════════════╗
║                    ✅  NODE UP TO DATE                            ║
╠══════════════════════════════════════════════════════════════════╣
║                                                                  ║
║  Version:    1.0.1 ✓                                             ║
║  Production: Active ✓                                            ║
║                                                                  ║
║  No pending updates.                                             ║
║                                                                  ║
╚══════════════════════════════════════════════════════════════════╝
```

### 4.2 Log Messages

Periodic log messages (every 6 hours while update pending):

```
[2026-01-15 10:00:00] INFO  Synced to block 125,432
[2026-01-15 10:00:00] ⚠️  UPDATE 1.0.1 PENDING | Veto: 15%/40% | 5d left | doli-node update status
[2026-01-15 10:00:05] INFO  Produced block 125,433
```

```
[2026-01-17 10:00:00] INFO  Synced to block 126,000
[2026-01-17 10:00:00] ✅  UPDATE 1.0.1 APPROVED | Grace: 36h left | doli-node update apply
[2026-01-17 10:00:05] INFO  Produced block 126,001
```

```
[2026-01-19 10:00:00] INFO  Synced to block 126,500
[2026-01-19 10:00:00] 🚫  PRODUCTION PAUSED | Outdated 1.0.0 | Required 1.0.1 | doli-node update apply
[2026-01-19 10:00:05] WARN  Skipped block production - node outdated
```

### 4.3 Version Enforcement: "No Update = No Produce"

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
│  ✅ Serve RPC requests                                                          │
│  ✅ Relay transactions                                                          │
│  ✅ Validate blocks                                                             │
│                                                                                 │
└─────────────────────────────────────────────────────────────────────────────────┘
```

### 4.4 Why This Is Fair

| Argument | Response |
|----------|----------|
| "I'm forced to update" | You had 7 days + 48h grace period to veto or object |
| "I don't trust the update" | You had 40% threshold to block it with weighted votes |
| "I didn't have time" | 7 days + 48h = 9 days total notice |
| "It's centralized" | No — the community voted, not the maintainers |
| "Veterans control everything" | No — after 4 years everyone has equal weight |

---

## 5. Automatic Rollback System

A critical addition to ensure network stability: automatic rollback when updates cause node failures.

### 5.1 Crash Detection Watchdog

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

### 5.2 Rollback Trigger Conditions

1. Node crashes **3+ times** within 1 hour of update application
2. Node fails to reach sync within 5 minutes of restart
3. Node fails health checks (RPC unresponsive, peers disconnected)

### 5.3 Rollback Process

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

### 5.4 Post-Rollback Behavior

- Node continues operating on previous version
- Update marked as "failed locally" (not network-wide rejection)
- Operator notified via logs and optional webhook
- Manual intervention required to retry update
- Node can still produce blocks (if previous version meets requirements)

---

## 6. Hard Fork Support

While most updates are backward-compatible, some protocol changes require coordinated hard forks. The system includes an optional upgrade-at-height mechanism.

### 6.1 When Hard Forks Are Needed

- Changes to block structure or validation rules
- Changes to consensus algorithm parameters
- State migration or database format changes
- Cryptographic algorithm upgrades

### 6.2 Upgrade-at-Height Mechanism

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

### 6.3 Hard Fork Timeline

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
│    │            │                  │                               │            │
│    └────────────┴──────────────────┴───────────────────────────────┤            │
│    │◄── 7 days ──►│◄── 48 hrs ──►│◄────── ~21 days ──────────────►│            │
│    │   (veto)     │   (grace)    │         (update window)        │            │
│                                                                   │            │
│                                                     At activation_height:       │
│                                                     ├── New rules take effect   │
│                                                     └── Old nodes fork off      │
│                                                                                 │
└─────────────────────────────────────────────────────────────────────────────────┘
```

### 6.4 Soft Update vs Hard Fork Comparison

```
┌─────────────────────┬─────────────────────┬─────────────────────┐
│ Aspect              │ Soft Update         │ Hard Fork           │
├─────────────────────┼─────────────────────┼─────────────────────┤
│ Backward compatible │ Yes                 │ No                  │
│ Old nodes can sync  │ Yes                 │ No (fork off)       │
│ Activation          │ Immediate (grace)   │ At specific height  │
│ Veto period         │ 7 days              │ 7 days              │
│ Total notice        │ 9 days              │ ~30 days            │
│ Rollback possible   │ Yes (automatic)     │ No (chain diverged) │
│ Network split risk  │ None                │ Yes (if not ready)  │
└─────────────────────┴─────────────────────┴─────────────────────┘
```

---

## 7. Security Model

### 7.1 Threat Analysis

```
┌────────────────────┬─────────────────────────┬─────────────────────────┬──────────────┐
│ Threat             │ Attack Vector           │ Mitigation              │ Risk Level   │
├────────────────────┼─────────────────────────┼─────────────────────────┼──────────────┤
│ Malicious          │ Signs backdoored        │ Requires 3/5            │ Low          │
│ maintainer         │ binary                  │ signatures              │ (collusion)  │
├────────────────────┼─────────────────────────┼─────────────────────────┼──────────────┤
│ Key compromise     │ Attacker signs          │ Still need 2 more       │ None         │
│ (1 key)            │ releases                │ keys                    │              │
├────────────────────┼─────────────────────────┼─────────────────────────┼──────────────┤
│ Key compromise     │ Attacker signs          │ Still need 1 more       │ None         │
│ (2 keys)           │ releases                │ key                     │              │
├────────────────────┼─────────────────────────┼─────────────────────────┼──────────────┤
│ Key compromise     │ Attacker signs          │ Community can veto      │ Medium       │
│ (3 keys)           │ releases                │ within 7 days           │              │
├────────────────────┼─────────────────────────┼─────────────────────────┼──────────────┤
│ Sybil veto         │ Block legitimate        │ Seniority weighting     │ Low          │
│ attack             │ updates                 │ + bond + 30-day min     │              │
├────────────────────┼─────────────────────────┼─────────────────────────┼──────────────┤
│ Mirror             │ Serve malicious         │ SHA-256 hash            │ None         │
│ compromise         │ binary                  │ verification            │              │
├────────────────────┼─────────────────────────┼─────────────────────────┼──────────────┤
│ MITM attack        │ Intercept download      │ Hash verification       │ None         │
│                    │                         │ + HTTPS                 │              │
├────────────────────┼─────────────────────────┼─────────────────────────┼──────────────┤
│ Rollback attack    │ Force old vulnerable    │ Version comparison      │ None         │
│                    │ version                 │ (no downgrades)         │              │
└────────────────────┴─────────────────────────┴─────────────────────────┴──────────────┘
```

### 7.2 Key Security Requirements

| Aspect | Requirement |
|--------|-------------|
| Key generation | Air-gapped system, HSM recommended |
| Key storage | Hardware wallet or encrypted offline |
| Key usage | Sign releases offline, transfer only signatures |
| Key rotation | Emergency procedures documented |

### 7.3 Defense in Depth

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│                              DEFENSE LAYERS                                      │
├─────────────────────────────────────────────────────────────────────────────────┤
│                                                                                 │
│  Layer 1: CRYPTOGRAPHIC                                                         │
│  ├── 3/5 multisig for releases                                                  │
│  ├── SHA-256 binary verification                                                │
│  └── Ed25519 signatures on all messages                                         │
│                                                                                 │
│  Layer 2: GOVERNANCE                                                            │
│  ├── 40% weighted veto threshold                                                │
│  ├── 7-day mandatory review period                                              │
│  └── Vote changing allowed (react to new info)                                  │
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

## 8. CLI Command Reference

### 8.1 Update Management

```bash
# Check for available updates
doli-node update check

# Output (update available):
# Update available: 1.0.0 -> 1.0.1
# Changelog: Security fix for VDF verification bypass
# Veto period: 5 days, 12 hours remaining
# Veto status: 15.5% of 40% threshold

# Show detailed update status
doli-node update status

# Apply approved update (after veto period)
doli-node update apply

# Force apply (bypasses approval check, NOT veto period)
doli-node update apply --force

# Manual rollback to backup
doli-node update rollback
```

### 8.2 Voting (Producers Only)

```bash
# Vote to VETO (block) an update
doli-node update vote --veto --version 1.0.1 --key /path/to/producer.json

# Vote to APPROVE an update
doli-node update vote --approve --version 1.0.1 --key /path/to/producer.json

# Change your vote (replaces previous)
doli-node update vote --approve --version 1.0.1 --key /path/to/producer.json

# View current vote status for a version
doli-node update votes --version 1.0.1

# Output:
# Version: 1.0.1
# Total eligible weight: 125.50
# Veto weight: 45.25 (36.1%)
# Approve weight: 80.25 (63.9%)
# Threshold: 40%
# Status: ON TRACK TO PASS
# Time remaining: 3 days, 8 hours
```

### 8.3 Node Run Options

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

## 9. RPC Endpoints

### 9.1 getUpdateStatus

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

### 9.2 submitVote

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

### 9.3 getVoteHistory

```json
// Request
{
  "jsonrpc": "2.0",
  "method": "getVoteHistory",
  "params": {
    "version": "1.0.1"
  },
  "id": 1
}

// Response
{
  "jsonrpc": "2.0",
  "result": {
    "version": "1.0.1",
    "votes": [
      {
        "producer": "abc123...",
        "vote": "veto",
        "weight": 4.0,
        "years_active": 5.2,
        "timestamp": 1704067200
      },
      {
        "producer": "def456...",
        "vote": "approve",
        "weight": 2.5,
        "years_active": 2.1,
        "timestamp": 1704067300
      }
    ],
    "summary": {
      "total_votes": 2,
      "veto_weight": 4.0,
      "approve_weight": 2.5,
      "abstain_weight": 285.0
    }
  },
  "id": 1
}
```

### 9.4 getProducerSeniority

```json
// Request
{
  "jsonrpc": "2.0",
  "method": "getProducerSeniority",
  "params": {
    "producer": "abc123..."
  },
  "id": 1
}

// Response
{
  "jsonrpc": "2.0",
  "result": {
    "producer": "abc123...",
    "registration_height": 50000,
    "days_active": 1825,
    "years_active": 5.0,
    "vote_weight": 4.0,
    "is_eligible_to_vote": true
  },
  "id": 1
}
```

---

## 10. Implementation Reference

### 10.1 File Locations

```
┌─────────────────────────────────┬─────────────────────────────────────────────┐
│ Component                       │ Path                                        │
├─────────────────────────────────┼─────────────────────────────────────────────┤
│ Core library                    │ crates/updater/src/lib.rs                   │
│ Seniority voting                │ crates/updater/src/seniority.rs             │
│ Vote tracking                   │ crates/updater/src/vote.rs                  │
│ Watchdog (rollback)             │ crates/updater/src/watchdog.rs              │
│ Hard fork logic                 │ crates/updater/src/hardfork.rs              │
│ Binary download                 │ crates/updater/src/download.rs              │
│ Binary verification             │ crates/updater/src/verify.rs                │
│ Update application              │ crates/updater/src/apply.rs                 │
│ Test keys                       │ crates/updater/src/test_keys.rs             │
│ Node integration                │ bins/node/src/updater.rs                    │
│ CLI commands                    │ bins/cli/src/update.rs                      │
│ RPC methods                     │ crates/rpc/src/update.rs                    │
│ Gossip topics                   │ crates/network/src/gossip.rs                │
└─────────────────────────────────┴─────────────────────────────────────────────┘
```

### 10.2 Key Data Structures

```rust
/// Producer seniority information
pub struct ProducerSeniority {
    pub pubkey: PublicKey,
    pub registration_height: u64,
    pub years_active: f64,
    pub vote_weight: f64,
}

impl ProducerSeniority {
    pub fn calculate_weight(&self) -> f64 {
        1.0 + f64::min(self.years_active, 4.0) * 0.75
    }
    
    pub fn is_eligible_to_vote(&self, current_height: u64, blocks_per_day: u64) -> bool {
        let days_active = (current_height - self.registration_height) / blocks_per_day;
        days_active >= MIN_VOTING_AGE_DAYS  // 30
    }
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
}

/// Vote with seniority weight
pub struct WeightedVote {
    pub message: VoteMessage,
    pub producer_seniority: ProducerSeniority,
    pub effective_weight: f64,
}

/// Update watchdog state
pub struct UpdateWatchdog {
    pub last_update_version: Option<String>,
    pub last_update_time: Option<u64>,
    pub crash_count: u32,
    pub crash_timestamps: Vec<u64>,
}
```

### 10.3 Constants

```rust
// Timing
pub const VETO_PERIOD: Duration = Duration::from_secs(7 * 24 * 3600);     // 7 days
pub const GRACE_PERIOD: Duration = Duration::from_secs(48 * 3600);        // 48 hours
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);       // 6 hours

// Thresholds
pub const VETO_THRESHOLD_PERCENT: u8 = 40;
pub const REQUIRED_SIGNATURES: usize = 3;

// Seniority
pub const MAX_SENIORITY_MULTIPLIER: f64 = 4.0;
pub const SENIORITY_MATURITY_YEARS: f64 = 4.0;
pub const MIN_VOTING_AGE_DAYS: u64 = 30;

// Rollback
pub const CRASH_THRESHOLD: u32 = 3;
pub const CRASH_WINDOW: Duration = Duration::from_secs(3600);             // 1 hour

// Distribution
pub const GITHUB_API_URL: &str = "https://api.github.com/repos/e-weil/doli/releases/latest";
pub const FALLBACK_MIRROR: &str = "https://releases.doli.network";
```

---

## 11. Comparison with Other Blockchains

```
┌─────────────────────┬────────────┬────────────┬────────────┬─────────────────┐
│ Feature             │ Bitcoin    │ Ethereum   │ Solana     │ DOLI            │
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
│ Hard fork support   │ Yes        │ Yes        │ Yes        │ Yes             │
│                     │ (manual)   │ (manual)   │ (forced)   │ (at-height)     │
├─────────────────────┼────────────┼────────────┼────────────┼─────────────────┤
│ Sybil resistance    │ Hashpower  │ Stake      │ Stake      │ Seniority +     │
│                     │            │            │            │ bond + time     │
└─────────────────────┴────────────┴────────────┴────────────┴─────────────────┘
```

**DOLI's advantage**: Combines the security review period of Bitcoin/Ethereum with the operational efficiency of Solana, while adding democratic governance that neither has. The seniority system prevents both plutocracy (unlike pure PoS) and Sybil attacks (unlike pure count-based voting).

---

## 12. Frequently Asked Questions

### Q: Can maintainers force an update without community consent?

**No.** Even with 3/5 maintainer signatures, the community has 7 days to review and veto. If 40% of weighted voting power objects, the update is rejected. Maintainers propose; the community disposes.

### Q: What if I disagree with an approved update?

You have several options:
1. Vote to veto during the period (organize other producers)
2. Accept the democratic outcome and update
3. Run with `--no-auto-update` and stay on old version (cannot produce blocks)
4. Fork the network with like-minded participants

### Q: What happens if my node crashes after an update?

The watchdog automatically rolls back to the previous version after 3 crashes within 1 hour. You can also manually rollback with `doli-node update rollback`. Your backup is always preserved.

### Q: Can I run an old version forever?

- **Soft updates**: Yes, but you cannot produce blocks after the grace period. You can still sync, serve RPC, and relay transactions.
- **Hard forks**: No. Your node will fork off and follow a dead chain. You must update before activation height.

### Q: Why is the seniority cap at 4 years?

Four years balances Sybil resistance with preventing permanent oligarchy:
- **Long enough**: Creates meaningful barriers for attackers (must maintain fake producers for years)
- **Short enough**: New participants can eventually reach parity with founders
- **Fair**: Everyone who commits long-term gets equal voice

### Q: What if 3+ maintainer keys are compromised?

The community has 7 days to detect and veto any malicious release. Additionally:
1. Key rotation procedures should be initiated immediately
2. Community alert via all channels (Discord, Twitter, forums)
3. In extreme cases, a new network may need to be launched with new keys

### Q: How do I become a maintainer?

Maintainer positions are determined by community governance. The initial 5 maintainers are selected before mainnet launch. Future changes require:
1. Governance proposal submitted
2. Community discussion period
3. On-chain vote by producers
4. Key ceremony for new maintainer

### Q: What's the difference between "approve" and not voting?

Functionally the same. The system uses **veto-based** governance:
- Updates pass by default unless blocked
- Only VETO votes count toward the 40% threshold
- Abstaining = implicit approval

This prevents "voter apathy" from blocking important security updates.

### Q: Can I change my vote?

**Yes.** Unlike the v1.0 system, you can change your vote at any time during the 7-day veto period. Only your latest vote (by timestamp) counts at the deadline. This allows reaction to new information discovered during review.

### Q: How do I verify a release is legitimate?

```bash
# Check signatures
doli-node update verify --version 1.0.1

# Output shows:
# - All 3+ maintainer signatures
# - SHA-256 hash of binary
# - Signature verification status
# - Whether hash matches downloaded binary
```

---

## Document Information

- **Version**: 2.0
- **Last Updated**: January 2026
- **Status**: Production Specification

### Changes from v1.0

- ✅ Added seniority-weighted voting (replaces count-based)
- ✅ Added 30-day minimum voting age requirement
- ✅ Added vote changing capability during veto period
- ✅ Added automatic rollback with crash detection watchdog
- ✅ Added hard fork support with upgrade-at-height mechanism
- ✅ Expanded security analysis and threat modeling
- ✅ Added comprehensive FAQ section
- ✅ Clarified the 4-year seniority convergence model

---

*DOLI Protocol - Decentralized, Democratic, Secure*

# DOLI Auto-Update System - Complete Technical Documentation

This document covers **every detail** of the DOLI blockchain auto-update mechanism, including governance, CLI commands, update application process, and operational considerations.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Maintainer Keys and Timing](#2-maintainer-keys-and-timing)
3. [Release Creation Process](#3-release-creation-process)
4. [Update Detection](#4-update-detection)
5. [Governance and Veto System](#5-governance-and-veto-system)
6. [Version Enforcement ("No Update = No Produce")](#6-version-enforcement)
7. [CLI Commands](#7-cli-commands)
8. [Update Application Process](#8-update-application-process)
9. [Node Behavior During Updates](#9-node-behavior-during-updates)
10. [Non-Updating Nodes](#10-non-updating-nodes)
11. [Network Compatibility](#11-network-compatibility)
12. [Configuration Options](#12-configuration-options)
13. [Security Considerations](#13-security-considerations)
14. [Troubleshooting](#14-troubleshooting)
15. [Test Scripts](#15-test-scripts)

---

## 1. Overview

The DOLI auto-update system provides:

- **Transparent updates**: All releases publicly signed and verifiable
- **Community veto power**: 40% of producers can block any update
- **Automatic application**: Updates applied after veto period with no manual intervention
- **Cryptographic verification**: 3/5 maintainer signatures required
- **Rollback capability**: Automatic backup before update

### Key Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `VETO_PERIOD` | 7 days | Time for producers to vote on updates |
| `GRACE_PERIOD` | 48 hours | Time after approval before enforcement |
| `VETO_THRESHOLD_PERCENT` | 40% | Percentage of producers needed to reject |
| `REQUIRED_SIGNATURES` | 3 of 5 | Maintainer signatures needed for release |
| `CHECK_INTERVAL` | 6 hours | How often nodes check for updates |

---

## 2. Maintainer Keys and Timing

### When to Create Maintainer Keys

**CRITICAL: Maintainer keys MUST be created and configured BEFORE mainnet genesis.**

```
Timeline:
┌─────────────────────────────────────────────────────────────────────────┐
│  Key Generation    Code Update     Build & Test    Genesis    Mainnet  │
│       │                │                │             │           │    │
│       ▼                ▼                ▼             ▼           ▼    │
│   [Create 5       [Replace         [Verify        [Network    [First  │
│    Ed25519        placeholder      keys are       launches]    block] │
│    keypairs]      keys in          production-                        │
│                   lib.rs]          ready]                             │
└─────────────────────────────────────────────────────────────────────────┘
```

### Key Generation Process

Each of the 5 designated maintainers must:

1. **Generate offline**: Create Ed25519 keypair on air-gapped system
2. **Secure storage**: Store private key in hardware wallet or secure offline storage
3. **Submit public key**: Provide hex-encoded public key for inclusion in code
4. **Never expose**: Private key must NEVER touch internet-connected device

### Current Placeholder Keys

**Location**: `crates/updater/src/lib.rs`

```rust
pub const MAINTAINER_KEYS: [&str; 5] = [
    // PLACEHOLDER KEYS - REPLACE BEFORE MAINNET
    "0000000000000000000000000000000000000000000000000000000000000001",
    "0000000000000000000000000000000000000000000000000000000000000002",
    "0000000000000000000000000000000000000000000000000000000000000003",
    "0000000000000000000000000000000000000000000000000000000000000004",
    "0000000000000000000000000000000000000000000000000000000000000005",
];
```

### Placeholder Detection

The code includes safety checks:

```rust
// Returns true if ANY key starts with "00000000"
pub fn is_using_placeholder_keys() -> bool

// Panics if placeholder keys detected - call during mainnet startup
pub fn assert_production_keys()
```

**Node startup on mainnet MUST call `assert_production_keys()` to prevent running with test keys.**

### Test Keys for Development

For devnet/testnet testing without real maintainer keys:

```bash
# Enable test keys (generates deterministic Ed25519 keypairs)
DOLI_TEST_KEYS=1 doli-node run --network devnet
```

Test keys are generated in `crates/updater/src/test_keys.rs` using deterministic seeds.

---

## 3. Release Creation Process

### Step-by-Step Release Workflow

```
┌──────────────────────────────────────────────────────────────────────────┐
│                        RELEASE CREATION WORKFLOW                          │
├──────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  1. BUILD BINARIES                                                       │
│     ├── cargo build --release (all platforms)                            │
│     ├── linux-x64, linux-arm64                                           │
│     └── macos-x64, macos-arm64                                           │
│                                                                          │
│  2. COMPUTE HASHES                                                       │
│     └── sha256sum doli-node-linux-x64 → binary_sha256                    │
│                                                                          │
│  3. CREATE RELEASE METADATA                                              │
│     └── version, binary_sha256, changelog, published_at                  │
│                                                                          │
│  4. COLLECT SIGNATURES (3 of 5 maintainers)                              │
│     ├── Maintainer 1: signs "1.0.1:abc123..." offline                    │
│     ├── Maintainer 2: signs "1.0.1:abc123..." offline                    │
│     └── Maintainer 3: signs "1.0.1:abc123..." offline                    │
│                                                                          │
│  5. PUBLISH RELEASE                                                      │
│     ├── Upload binaries to CDN                                           │
│     ├── Create latest.json with signatures                               │
│     └── Upload to all mirrors                                            │
│                                                                          │
└──────────────────────────────────────────────────────────────────────────┘
```

### Release JSON Format

```json
{
    "version": "1.0.1",
    "binary_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "binary_url_template": "https://github.com/e-weil/doli/releases/download/v1.0.1/doli-node-{platform}",
    "changelog": "Security fix for VDF verification bypass",
    "published_at": 1704067200,
    "signatures": [
        {
            "public_key": "abc123...",
            "signature": "def456..."
        },
        {
            "public_key": "ghi789...",
            "signature": "jkl012..."
        },
        {
            "public_key": "mno345...",
            "signature": "pqr678..."
        }
    ]
}
```

### Signature Message Format

Maintainers sign the string: `"version:binary_sha256"`

Example: `"1.0.1:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"`

### Release Distribution

**Primary source: GitHub Releases**

```rust
/// GitHub API URL for latest release
pub const GITHUB_API_URL: &str = "https://api.github.com/repos/e-weil/doli/releases/latest";

/// GitHub releases download base URL
pub const GITHUB_RELEASES_URL: &str = "https://github.com/e-weil/doli/releases/download";

/// Fallback update server (if GitHub is unreachable)
pub const FALLBACK_MIRROR: &str = "https://releases.doli.network";
```

**Why GitHub Releases:**

| Advantage | Explanation |
|-----------|-------------|
| **Free** | No hosting costs |
| **Global CDN** | Fast downloads worldwide |
| **99.9%+ Uptime** | Microsoft infrastructure |
| **Verifiable** | Public commit history |
| **API** | `api.github.com/repos/.../releases/latest` |

**Fallback behavior:**
1. Check GitHub API for latest release tag
2. Download `release.json` from that tag
3. If GitHub fails, try `releases.doli.network/latest.json`

---

## 4. Update Detection

### Detection Flow

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         UPDATE DETECTION FLOW                            │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  Every 6 hours (configurable):                                          │
│                                                                         │
│  ┌─────────────┐                                                        │
│  │ Check       │──────► GET api.github.com/repos/.../releases/latest   │
│  │ GitHub API  │        (fallback: releases.doli.network/latest.json)  │
│  └──────┬──────┘                                                        │
│         │                                                               │
│         ▼                                                               │
│  ┌─────────────┐     ┌──────────────────────────────────────────────┐  │
│  │ Parse       │────►│ Version: 1.0.1                               │  │
│  │ Release     │     │ Hash: e3b0c44298fc1c149...                   │  │
│  └──────┬──────┘     │ Signatures: [sig1, sig2, sig3]               │  │
│         │            └──────────────────────────────────────────────┘  │
│         ▼                                                               │
│  ┌─────────────┐                                                        │
│  │ Compare     │     Current: 1.0.0                                     │
│  │ Versions    │────► is_newer_version("1.0.1", "1.0.0") = true        │
│  └──────┬──────┘                                                        │
│         │                                                               │
│         ▼                                                               │
│  ┌─────────────┐                                                        │
│  │ Verify      │────► Check 3/5 valid Ed25519 signatures               │
│  │ Signatures  │                                                        │
│  └──────┬──────┘                                                        │
│         │                                                               │
│         ▼                                                               │
│  ┌─────────────┐                                                        │
│  │ Start Veto  │────► VoteTracker created, 7-day countdown begins      │
│  │ Period      │                                                        │
│  └──────┬──────┘                                                        │
│         │                                                               │
│         ▼                                                               │
│  ┌─────────────┐                                                        │
│  │ Display     │────► MANDATORY notification shown to node operator    │
│  │ Notification│                                                        │
│  └─────────────┘                                                        │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### Mandatory Notification System

When a new update is detected, **ALL nodes** display a prominent notification:

```
╔══════════════════════════════════════════════════════════════════╗
║                    ⚠️  UPDATE PENDING                             ║
╠══════════════════════════════════════════════════════════════════╣
║  Version: 0.1.0 -> 1.0.1                                         ║
║  Veto period: 7 days, 0 hours remaining                          ║
║  Current vetos: 0/100 (0%)                                       ║
║  Threshold to reject: 40%                                        ║
╠══════════════════════════════════════════════════════════════════╣
║  Review changelog and vote if you have objections:               ║
║                                                                  ║
║    doli-node update status         # See full details            ║
║    doli-node update vote --veto --key <producer.json>            ║
║                                                                  ║
╚══════════════════════════════════════════════════════════════════╝
```

**Notification behavior:**
- Shown immediately when update detected
- Repeated every 6 hours as a reminder
- Persisted across node restarts (saved to `pending_update.json`)
- Contains countdown, veto count, and how to vote

**Philosophy:** Instead of requiring producers to pay to vote, the system:
- Publishes the update for all to see
- Notifies all operators automatically
- Gives 7 days for review
- If no veto threshold reached → community trusts → update passes

### Version Comparison

Semantic versioning comparison (major.minor.patch):

```rust
pub fn is_newer_version(new: &str, current: &str) -> bool {
    // "1.0.1" > "1.0.0" → true
    // "2.0.0" > "1.9.9" → true
    // "1.0.0" > "1.0.0" → false (not newer)
    // "1.0.0" > "1.0.1" → false (older)
}
```

**Important**: Downgrades are NOT supported. Only newer versions are considered.

---

## 5. Governance and Veto System

### Veto Period Timeline

```
┌─────────────────────────────────────────────────────────────────────────┐
│                            VETO PERIOD (7 DAYS)                          │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  Day 0          Day 1-6                              Day 7              │
│    │               │                                   │                │
│    ▼               ▼                                   ▼                │
│  Release       Producers vote                      Deadline            │
│  published     (veto or approve)                   reached             │
│    │               │                                   │                │
│    └───────────────┴───────────────────────────────────┤                │
│                                                        │                │
│                                                        ▼                │
│                                              ┌─────────────────┐        │
│                                              │ Count votes     │        │
│                                              │ (weighted)      │        │
│                                              └────────┬────────┘        │
│                                                       │                 │
│                                     ┌─────────────────┴────────────┐   │
│                                     │                              │   │
│                                     ▼                              ▼   │
│                              Veto < 40%                     Veto ≥ 40% │
│                                     │                              │   │
│                                     ▼                              ▼   │
│                              ┌───────────┐                ┌───────────┐│
│                              │ APPROVED  │                │ REJECTED  ││
│                              │ Apply     │                │ Discard   ││
│                              │ update    │                │ update    ││
│                              └───────────┘                └───────────┘│
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### Vote Types

```rust
pub enum Vote {
    Approve,  // Allow the update (or abstain - same effect)
    Veto,     // Block the update
}
```

### Vote Message Structure

```rust
pub struct VoteMessage {
    pub version: String,      // "1.0.1"
    pub vote: Vote,           // Approve or Veto
    pub producer_id: String,  // Producer's public key (hex)
    pub timestamp: u64,       // Unix timestamp
    pub signature: String,    // Signature over "version:vote:timestamp"
}
```

### Weighted Voting (Anti-Sybil)

Votes are weighted by producer seniority:

```
Weight = 1 + sqrt(months_active / 12)
Maximum weight = 4 (capped)

Examples:
- New producer (0 months): weight = 1
- 1 year active: weight = 1 + sqrt(12/12) = 2
- 4 years active: weight = 1 + sqrt(48/12) = 3
- 16 years active: weight = 1 + sqrt(192/12) = 5 → capped to 4
```

This prevents a wealthy attacker from registering many new nodes to block updates.

### Veto Threshold Calculation

```rust
// Weighted calculation (preferred)
pub fn should_reject_weighted(&self, total_weight: u64) -> bool {
    let veto_weight = self.veto_weight();
    let veto_percent = (veto_weight * 100) / total_weight;
    veto_percent >= 40  // VETO_THRESHOLD_PERCENT
}

// Example with 5 producers (total weight 10):
// - 2 junior vetos (weight 1 each) = 2/10 = 20% → APPROVED
// - 1 senior veto (weight 4) = 4/10 = 40% → REJECTED
```

### Vote Deduplication

- Each producer can only vote **once** per version
- Subsequent votes from same producer are **ignored**
- No vote changing allowed after initial submission

---

## 6. Version Enforcement ("No Update = No Produce")

### The Principle

```
┌─────────────────────────────────────────────────────────────────────────┐
│                    "NO ACTUALIZAS = NO PRODUCES"                         │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  PRINCIPLE: If your node is a security hole for the network,           │
│             you shouldn't be producing blocks.                          │
│                                                                         │
│  THIS IS NOT PUNISHMENT. It's network protection.                       │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### Complete Update Timeline

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         COMPLETE UPDATE TIMELINE                         │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  Day 0: UPDATE PUBLISHED                                                │
│         ├── Release signed by 3/5 maintainers                          │
│         ├── ALL nodes show notification:                                │
│         │   "⚠️ Update v1.0.1 pending. Veto period: 7 days."           │
│         └── Changelog displayed for review                              │
│                                                                         │
│  Day 0-7: VETO PERIOD                                                   │
│         ├── Producers can vote to veto                                  │
│         ├── Review changelog and code changes                           │
│         └── If >= 40% veto: UPDATE REJECTED                            │
│                                                                         │
│  Day 7: VETO PERIOD ENDS                                                │
│         ├── If < 40% veto: UPDATE APPROVED                             │
│         └── Grace period begins                                         │
│                                                                         │
│  Day 7-9: GRACE PERIOD (48 hours)                                       │
│         ├── Notification: "Update approved. 48h to update."             │
│         ├── Run: doli-node update apply                                 │
│         └── Outdated nodes can still produce (for now)                  │
│                                                                         │
│  Day 9+: ENFORCEMENT ACTIVE                                             │
│         ├── Nodes < required version: PRODUCTION PAUSED                 │
│         ├── Network rejects blocks from outdated producers              │
│         └── Update to resume production                                 │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

**Total notice before enforcement: 9 days**

### How Enforcement Works

When enforcement is active:

1. **Production Check**: Before producing a block, the node checks:
   ```rust
   if current_version < required_version {
       log("PRODUCTION PAUSED: Node outdated");
       return; // Skip block production
   }
   ```

2. **Clear Notification**: Outdated nodes see:
   ```
   ╔══════════════════════════════════════════════════════════════════╗
   ║                    ⚠️  PRODUCTION PAUSED                          ║
   ╠══════════════════════════════════════════════════════════════════╣
   ║                                                                  ║
   ║  Your node (0.1.0) is outdated.                                  ║
   ║  Required version: 1.0.1                                         ║
   ║                                                                  ║
   ║  Network security requires all producers to be updated.          ║
   ║  Block production is paused until update is complete.            ║
   ║                                                                  ║
   ║  To update, run:                                                 ║
   ║    doli-node update apply                                        ║
   ║                                                                  ║
   ║  After updating, production will resume automatically.           ║
   ║                                                                  ║
   ╚══════════════════════════════════════════════════════════════════╝
   ```

3. **Automatic Resume**: After running `doli-node update apply`, production resumes immediately with no manual intervention.

### Why This Is Fair

| Argument | Response |
|----------|----------|
| "I'm forced to update" | You had 7 days + 48h grace period to veto or object |
| "I don't trust the update" | You had 40% threshold to block it |
| "I didn't have time" | 7 days + 48h = 9 days total notice |
| "It's centralized" | No — the community voted, not the maintainers |

### What Outdated Nodes Can Still Do

Even with enforcement active, outdated nodes:

- ✅ **Can sync the chain** (receive and validate blocks)
- ✅ **Can serve RPC requests** (wallets still work)
- ✅ **Can relay transactions** (mempool functions normally)
- ❌ **Cannot produce blocks** (paused until updated)

### Comparison with Other Blockchains

| Blockchain | Forces updates? | How |
|------------|-----------------|-----|
| Bitcoin | ❌ No | Old nodes eventually on minority fork |
| Ethereum | ⚠️ Partial | Hard fork heights, no active enforcement |
| Solana | ✅ Yes | Foundation controls, validators follow |
| **DOLI** | ✅ **Yes, democratically** | 40% can veto, but if approved, all update |

---

## 7. CLI Commands

### Update Management Commands

#### Check for Updates

```bash
doli-node update check
```

**Output (update available):**
```
Update available: 1.0.0 -> 1.0.1
Changelog: Security fix for VDF verification bypass
Veto period: 7 days remaining
```

**Output (no update):**
```
Already on latest version (1.0.1)
```

#### Show Update Status

```bash
doli-node update status
```

**Output (pending update):**
```
Pending Update: v1.0.1
Changelog: Security fix for VDF verification bypass

Veto Status:
  Vetos: 3 (30%)
  Threshold: 40%
  Time remaining: 5d 12h

Status: ON TRACK TO PASS
```

**Output (no pending):**
```
No pending updates
Current version: 1.0.0
```

#### Vote on Update (Producers Only)

```bash
# Vote to VETO (block) the update
DOLI_VOTE_VERSION=1.0.1 doli-node update vote --veto --key /path/to/producer.json

# Vote to APPROVE the update
DOLI_VOTE_VERSION=1.0.1 doli-node update vote --approve --key /path/to/producer.json
```

**Output:**
```
Vote created for version 1.0.1: VETO
Producer: abc123def456...
Timestamp: 1704067200
Signature: 789xyz...

Submit via RPC:
curl -X POST http://localhost:28545 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"submitVote","params":{"vote":{...}},"id":1}'
```

#### Apply Approved Update

```bash
# Apply an approved update (after veto period ends)
doli-node update apply

# Force apply even if not explicitly approved (not recommended)
# NOTE: --force only bypasses the approval check, NOT the veto period!
# The 7-day veto period CANNOT be bypassed for security reasons.
doli-node update apply --force
```

**Security: Veto Period Protection**

The `apply_update` function enforces two security checks:
1. **Veto period must be over** (7 days) - CANNOT be bypassed, even with `--force`
2. **Update must be approved** - Can be bypassed with `--force` (not recommended)

This design prevents producers from applying potentially malicious updates before the community has had time to review and veto. The `--force` flag exists for edge cases where approval tracking failed, but the veto period is always enforced.

**Output (success):**
```
╔══════════════════════════════════════════════════════════════════╗
║                    Applying Update                               ║
╠══════════════════════════════════════════════════════════════════╣
║  Current version: 1.0.0                                          ║
║  Target version:  1.0.1                                          ║
╚══════════════════════════════════════════════════════════════════╝

Downloading 1.0.1...

╔══════════════════════════════════════════════════════════════════╗
║                    ✅ UPDATE SUCCESSFUL                          ║
╠══════════════════════════════════════════════════════════════════╣
║                                                                  ║
║  Version: 1.0.0 → 1.0.1                                          ║
║  Status: Production enabled                                      ║
║                                                                  ║
║  Your node is now up to date.                                    ║
║  Restarting node to apply changes...                             ║
║                                                                  ║
╚══════════════════════════════════════════════════════════════════╝
```

**Output (not approved):**
```
Update v1.0.1 is not yet approved.
Veto period: 5 days remaining.

Use --force to apply anyway (not recommended).
```

**Output (veto period still active):**
```
Cannot apply update v1.0.1: veto period still active (120h remaining)

Update v1.0.1 is still in veto period. The community must have the
opportunity to review and veto. Time remaining: 120 hours.

This is a security feature and cannot be bypassed.
```

Note: This error appears even with `--force`. The veto period is a critical security measure.

### Node Run Options

```bash
# Full options
doli-node run \
  --network mainnet \
  --no-auto-update \           # Disable automatic updates
  --update-notify-only \       # Check but don't apply
  --producer \
  --producer-key /path/to.json

# Minimal (auto-updates enabled by default)
doli-node run --network mainnet
```

### RPC Endpoints

#### getNodeInfo

```bash
curl -X POST http://localhost:28545 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getNodeInfo","params":{},"id":1}'
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "result": {
    "version": "1.0.0",
    "network": "mainnet",
    "peerId": "12D3KooW...",
    "peerCount": 42,
    "platform": "linux",
    "arch": "x86_64"
  },
  "id": 1
}
```

#### submitVote

```bash
curl -X POST http://localhost:28545 \
  -H 'Content-Type: application/json' \
  -d '{
    "jsonrpc":"2.0",
    "method":"submitVote",
    "params":{
      "vote":{
        "version":"1.0.1",
        "vote":"veto",
        "producerId":"abc123...",
        "timestamp":1704067200,
        "signature":"def456..."
      }
    },
    "id":1
  }'
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "result": {
    "status": "submitted",
    "message": "Vote submitted and broadcast to network"
  },
  "id": 1
}
```

#### getUpdateStatus

```bash
curl -X POST http://localhost:28545 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getUpdateStatus","params":{},"id":1}'
```

---

## 8. Update Application Process

### Application Steps

```
┌─────────────────────────────────────────────────────────────────────────┐
│                      UPDATE APPLICATION PROCESS                          │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  1. DOWNLOAD BINARY                                                     │
│     ├── Detect platform (linux-x64, macos-arm64, etc.)                  │
│     ├── Download from URL: .../doli-node-{platform}                     │
│     ├── Timeout: 5 minutes                                              │
│     └── Retry from mirrors on failure                                   │
│                                                                         │
│  2. VERIFY HASH                                                         │
│     ├── Compute SHA-256 of downloaded bytes                             │
│     ├── Compare with expected hash from release                         │
│     └── FAIL if mismatch (abort update)                                 │
│                                                                         │
│  3. BACKUP CURRENT BINARY                                               │
│     ├── Locate running executable                                       │
│     ├── Copy to: doli-node.backup                                       │
│     └── Preserve for rollback                                           │
│                                                                         │
│  4. INSTALL NEW BINARY                                                  │
│     ├── Write to temporary: doli-node.new                               │
│     ├── Set executable permissions (chmod 755)                          │
│     └── Atomic rename to replace original                               │
│                                                                         │
│  5. RESTART NODE                                                        │
│     ├── Unix: exec() replaces current process (same PID)                │
│     ├── Windows: spawn new process, exit current                        │
│     └── Node restarts with same arguments                               │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### Is Compilation Required?

**NO** - Updates are pre-compiled binaries:

- Maintainers compile binaries for all platforms
- Nodes download platform-specific binaries
- No Rust toolchain required on node machines
- No source code downloaded

### Is It All At Once?

**YES** - Binary replacement is atomic:

```rust
// Atomic rename - either succeeds completely or fails
fs::rename(&temp_path, target).await?;
```

- No partial updates possible
- Either old version or new version runs
- Never a mix of old/new code

### Restart Behavior

```
┌────────────────────────────────────────────────────────────────┐
│                     RESTART SEQUENCE                            │
├────────────────────────────────────────────────────────────────┤
│                                                                │
│  [Old Binary Running]                                          │
│         │                                                      │
│         ▼                                                      │
│  apply_update() completes                                      │
│         │                                                      │
│         ▼                                                      │
│  restart_node() called                                         │
│         │                                                      │
│         ▼                                                      │
│  ┌─────────────────────────────────────────────────────────┐  │
│  │ Unix: exec(new_binary, args)                            │  │
│  │   - Process image replaced in-place                     │  │
│  │   - Same PID, same file descriptors closed              │  │
│  │   - New binary starts immediately                       │  │
│  └─────────────────────────────────────────────────────────┘  │
│         │                                                      │
│         ▼                                                      │
│  [New Binary Running]                                          │
│         │                                                      │
│         ▼                                                      │
│  - Loads chain state from disk                                 │
│  - Resumes normal operation                                    │
│  - Reconnects to peers                                         │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

### Rollback

If update fails or causes issues:

```rust
pub async fn rollback() -> Result<()> {
    let current = current_binary_path()?;
    let backup = backup_path()?;  // doli-node.backup

    // Copy backup back to original location
    fs::copy(&backup, &current).await?;
}
```

Manual rollback:
```bash
# Stop node
systemctl stop doli-node

# Restore backup
cp /usr/local/bin/doli-node.backup /usr/local/bin/doli-node

# Restart
systemctl start doli-node
```

---

## 9. Node Behavior During Updates

### Downtime Analysis

| Phase | Duration | Node Status | Network Impact |
|-------|----------|-------------|----------------|
| Download | 1-60s | Running | None |
| Verification | <1s | Running | None |
| Backup | <1s | Running | None |
| Install | <1s | Running | None |
| Restart | 5-30s | **OFFLINE** | Peers timeout |
| Recovery | 1-5s | Syncing | Catching up |

**Total expected downtime: 5-30 seconds**

### What Happens to In-Flight Operations

#### Block Production

- If restart happens during your slot → **slot missed**
- Other producers continue normally
- Next assigned slot will be produced
- No slashing for missed slots due to updates

#### Transactions in Mempool

- Mempool is **in-memory only**
- Transactions **lost** on restart
- Users should resubmit if not confirmed
- Consider: Implement mempool persistence (future enhancement)

#### P2P Connections

- All connections **dropped** on restart
- Peers see connection timeout
- Reconnection automatic on restart
- DHT rediscovery takes 10-30 seconds

#### RPC Clients

- RPC server **stops** during restart
- Clients get connection refused
- Clients should retry with backoff
- No persistent sessions

### Notification/Messages

The node logs update activity:

```
INFO  doli_node::updater: New update available: 1.0.0 -> 1.0.1 (veto period: 7 days)
INFO  doli_node::updater: Changelog: Security fix for VDF verification bypass
DEBUG doli_node::updater: Update 1.0.1 pending: 15% veto, 120h remaining
INFO  doli_node::updater: Update 1.0.1 APPROVED (20% veto, threshold 40%)
INFO  updater::apply: Applying update to version 1.0.1
INFO  updater::apply: Downloading 1.0.1 for linux-x64
INFO  updater::apply: Downloaded 45000000 bytes from primary
INFO  updater::apply: Binary hash verified: e3b0c44298fc...
INFO  updater::apply: Backing up current binary to /usr/local/bin/doli-node.backup
INFO  updater::apply: Update to 1.0.1 applied successfully
INFO  updater::apply: Node will restart to apply changes
```

---

## 10. Non-Updating Nodes

### Scenarios

#### A. Auto-Updates Disabled (`--no-auto-update`)

```bash
doli-node run --network mainnet --no-auto-update
```

Behavior:
- Node **never checks** for updates
- Runs current version indefinitely
- Operator must update manually
- **No automatic protection** from security fixes

#### B. Notify-Only Mode (`--update-notify-only`)

```bash
doli-node run --network mainnet --update-notify-only
```

Behavior:
- Checks for updates every 6 hours
- Logs when update available
- **Never applies** update automatically
- Operator must update manually

#### C. Simply Not Restarting

If node is running during update application window:
- Update applied, restart triggered
- If systemd/supervisor restarts: new version runs
- If no process manager: node stays down until manual start

### Network Compatibility

**Old and new versions CAN coexist:**

```
┌─────────────────────────────────────────────────────────────────────────┐
│                    MIXED VERSION NETWORK                                 │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│   Node A (v1.0.0)  ◄───────────────────────►  Node B (v1.0.1)          │
│        │                                            │                   │
│        │           Block Gossip                     │                   │
│        │           (compatible)                     │                   │
│        │                                            │                   │
│        │           Transaction Relay                │                   │
│        │           (compatible)                     │                   │
│        │                                            │                   │
│        ▼                                            ▼                   │
│   ┌─────────┐                                  ┌─────────┐             │
│   │ Produces│                                  │ Produces│             │
│   │ blocks  │                                  │ blocks  │             │
│   └─────────┘                                  └─────────┘             │
│        │                                            │                   │
│        └─────────── Same chain ─────────────────────┘                   │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### What Old Nodes Experience

| Aspect | Behavior |
|--------|----------|
| Block validation | Works normally |
| Block production | Works normally |
| Transaction processing | Works normally |
| Peer connections | Compatible |
| Consensus participation | Full participation |
| Rewards | Received normally |
| Security fixes | **NOT applied** |

### Risks of Not Updating

1. **Security vulnerabilities** - Unpatched exploits
2. **Bug exposure** - Known issues not fixed
3. **Performance issues** - Optimizations missed
4. **Future incompatibility** - Eventually may diverge

### No Forced Updates

**IMPORTANT**: There is NO mechanism to force nodes to update.

- No "upgrade height" that rejects old versions
- No version field in blocks
- No consensus rule requiring specific version
- Operators have full control

---

## 11. Network Compatibility

### Version Compatibility Matrix

| Old Version | New Version | Blocks Compatible | Transactions Compatible | State Compatible |
|-------------|-------------|-------------------|------------------------|------------------|
| 1.0.0 | 1.0.1 | YES | YES | YES |
| 1.0.x | 1.1.x | YES | YES | YES |
| 1.x.x | 2.x.x | YES* | YES* | YES* |

*Unless major version introduces breaking changes (documented in changelog)

### No Hard Fork Support

Current design does **NOT** support hard forks:

- No "upgrade at height X" mechanism
- No version-gated consensus rules
- No state migration framework

**Breaking protocol changes require launching a new network.**

### P2P Protocol

- Same libp2p protocol across versions
- Gossipsub topics unchanged
- Block/transaction formats backward compatible
- No protocol negotiation

---

## 12. Configuration Options

### UpdateConfig Structure

```rust
pub struct UpdateConfig {
    pub enabled: bool,              // Default: true
    pub notify_only: bool,          // Default: false
    pub check_interval_secs: u64,   // Default: 21600 (6 hours)
    pub custom_url: Option<String>, // Default: None (use mirrors)
}
```

### Configuration via CLI

```bash
# Disable auto-updates entirely
doli-node run --no-auto-update

# Enable notifications only (no automatic application)
doli-node run --update-notify-only

# Both can be combined with other flags
doli-node run \
  --network mainnet \
  --no-auto-update \
  --producer \
  --producer-key /path/to/key.json
```

### Environment Variables

| Variable | Purpose | Example |
|----------|---------|---------|
| `DOLI_TEST_KEYS` | Enable test maintainer keys | `DOLI_TEST_KEYS=1` |
| `DOLI_VOTE_VERSION` | Version to vote on | `DOLI_VOTE_VERSION=1.0.1` |

### Custom Update Server

For private networks or air-gapped environments:

```rust
UpdateConfig {
    custom_url: Some("https://internal.mycompany.com/doli-updates".to_string()),
    ..Default::default()
}
```

Server must host `latest.json` at the root path.

---

## 13. Security Considerations

### Key Security

| Aspect | Requirement |
|--------|-------------|
| Key generation | Air-gapped system only |
| Key storage | Hardware wallet or encrypted offline |
| Key usage | Sign releases offline, transfer signatures |
| Key compromise | Requires 3/5 keys - single compromise not fatal |

### Release Verification

- **SHA-256 hash** prevents binary tampering
- **3/5 signatures** prevent unauthorized releases
- **Mirror fallback** prevents single point of failure
- **Version comparison** prevents downgrades

### Attack Scenarios

| Attack | Mitigation |
|--------|------------|
| Compromised mirror | Hash verification fails |
| 1 key compromised | Need 3/5 signatures |
| 2 keys compromised | Need 3/5 signatures (still safe) |
| Sybil veto attack | Weighted voting by seniority |
| Malicious update | 40% veto threshold |

### Recommendations

1. **Mainnet**: Enable auto-updates for security patches
2. **Producers**: Review updates during veto period
3. **Operators**: Monitor update announcements
4. **Maintainers**: Secure key storage, offline signing

---

## 14. Troubleshooting

### Common Issues

#### Update Not Detected

```bash
# Check connectivity to GitHub API
curl -s https://api.github.com/repos/e-weil/doli/releases/latest | jq .tag_name

# Check fallback mirror
curl -v https://releases.doli.network/latest.json

# Check node logs for update check errors
grep "update" /var/log/doli-node.log
```

#### Signature Verification Failed

```
ERROR: Release 1.0.1 has invalid signatures: InsufficientSignatures { found: 2, required: 3 }
```

Cause: Release not properly signed by maintainers.
Action: Contact maintainers or wait for proper release.

#### Hash Mismatch

```
ERROR: Binary hash mismatch: expected abc123..., got def456...
```

Cause: Download corrupted or tampering detected.
Action: Retry download, check network, report if persistent.

#### Update Rejected

```
WARN: Update 1.0.1 REJECTED (45% veto >= 40% threshold)
```

Cause: Community vetoed the update.
Action: No action needed - update discarded automatically.

#### Rollback Needed

```bash
# If new version has issues
systemctl stop doli-node
cp /usr/local/bin/doli-node.backup /usr/local/bin/doli-node
systemctl start doli-node
```

---

## 15. Test Scripts

### Available Test Scripts

| Script | Purpose | Duration |
|--------|---------|----------|
| `scripts/test_governance_scenarios.sh` | All vote threshold scenarios | ~2 min |
| `scripts/test_autoupdate_e2e.sh` | Complete E2E update flow | ~2 min |
| `scripts/test_12node_governance.sh` | Multi-node era progression | ~20 min |

### Running Tests

```bash
# Test all governance scenarios
./scripts/test_governance_scenarios.sh

# Test end-to-end auto-update flow
./scripts/test_autoupdate_e2e.sh

# Test with custom test directory
TEST_DIR=/tmp/my-test ./scripts/test_autoupdate_e2e.sh
```

### Test Coverage

| Functionality | Test Script | Status |
|---------------|-------------|--------|
| Version reporting | test_autoupdate_e2e.sh | TESTED |
| Vote submission | test_governance_scenarios.sh | TESTED |
| Approval flow (< 40%) | test_governance_scenarios.sh | TESTED |
| Rejection flow (>= 40%) | test_governance_scenarios.sh | TESTED |
| Vote propagation | test_autoupdate_e2e.sh | TESTED |
| Node sync stability | test_autoupdate_e2e.sh | TESTED |
| Signature verification | Unit tests | TESTED |
| Hash verification | Unit tests | TESTED |
| Binary download | Manual/integration | TESTED |
| Binary installation | Not safe to test | DOCUMENTED |
| Node restart | Not safe to test | DOCUMENTED |
| Rollback | Manual testing only | DOCUMENTED |

---

## Appendix A: File Locations

| Component | Path |
|-----------|------|
| Core updater library | `crates/updater/src/lib.rs` |
| Vote tracking | `crates/updater/src/vote.rs` |
| Binary download/verify | `crates/updater/src/download.rs` |
| Update application | `crates/updater/src/apply.rs` |
| Test keys | `crates/updater/src/test_keys.rs` |
| Node integration | `bins/node/src/updater.rs` |
| CLI entry point | `bins/node/src/main.rs` |
| RPC methods | `crates/rpc/src/methods.rs` |
| Gossip topics | `crates/network/src/gossip.rs` |

## Appendix B: Constants Reference

```rust
// Timing
pub const VETO_PERIOD: Duration = Duration::from_secs(7 * 24 * 3600);  // 7 days
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);    // 6 hours

// Thresholds
pub const VETO_THRESHOLD_PERCENT: u8 = 40;    // 40% veto to reject
pub const REQUIRED_SIGNATURES: usize = 3;     // 3 of 5 maintainers

// GitHub (primary source)
pub const GITHUB_REPO: &str = "e-weil/doli";
pub const GITHUB_API_URL: &str = "https://api.github.com/repos/e-weil/doli/releases/latest";
pub const GITHUB_RELEASES_URL: &str = "https://github.com/e-weil/doli/releases/download";

// Fallback (if GitHub unreachable)
pub const FALLBACK_MIRROR: &str = "https://releases.doli.network";
```

## Appendix C: Devnet Parameters

For faster testing on devnet:

| Parameter | Mainnet | Devnet |
|-----------|---------|--------|
| Veto period | 7 days | 60 blocks (~1 min) |
| Slot duration | 60 seconds | 1 second |
| Check interval | 6 hours | Configurable |
| Test keys | NO | YES (`DOLI_TEST_KEYS=1`) |

---

*Document last updated: January 2026*
*DOLI Protocol Version: 0.1.0*

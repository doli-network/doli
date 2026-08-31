# DOLI Auto-Update System

## Complete Technical Specification v3.0

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Maintainer Bootstrap System](#2-maintainer-bootstrap-system)
3. [Governance and Voting System](#3-governance-and-voting-system)
4. [Sybil Resistance Analysis](#4-sybil-resistance-analysis)
5. [Complete Update Timeline](#5-complete-update-timeline)
6. [Rollback (MANUAL — the automatic system is NOT implemented)](#6-rollback-manual--the-automatic-system-is-not-implemented)
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
- **Count-based veto voting**: 40% of ACTIVE PRODUCERS can veto an update (one producer, one vote — no seniority or stake weighting; see section 3)
- **Automatic application**: Updates applied after the veto period. There is NO automatic rollback on failure — see section 6
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
│ VETO_PERIOD                    │ 5 min *   │ Time for producers to vote on updates   │
│ GRACE_PERIOD                   │ 2 min *   │ Time after approval before enforcement  │
│ VETO_THRESHOLD_PERCENT         │ 40%       │ % of ACTIVE PRODUCERS needed to reject  │
│ CHECK_INTERVAL                 │ 10 min *  │ How often nodes check for updates       │
│ CRASH_THRESHOLD                │ 3         │ Consecutive crashes before rollback     │
└────────────────────────────────┴───────────┴─────────────────────────────────────────┘
```

**\* Early-network values** (since v1.1.13). These accelerated timings are appropriate for the current small maintainer set. They will be extended as the network grows (target: 7-day veto, 48-hour grace, 6-hour check). Devnet uses further-accelerated values: 1 min veto, 30 sec grace, 10 sec check. Values are set in `crates/core/src/network_params.rs` and configurable on devnet via `DOLI_VETO_PERIOD_SECS`, `DOLI_GRACE_PERIOD_SECS`.

---

## 2. Maintainer Bootstrap System

Unlike other blockchains that hardcode maintainer keys in configuration files, DOLI derives its maintainer set directly from the blockchain. Each network has its own independent maintainer set, and a host that has one uses it — the compiled arrays below are read ONLY by a host that has never established an on-chain set, plus the CLI. They are not a fallback (INC-I-172 F1).

**Maintainers are NOT producers on either network.** Genesis seeded the set from the first 5 registered producers, but both networks have since rotated it to five signing-only wallets — mainnet under INC-I-175/196 at h=331_457, testnet under INC-I-196. The old keys' private halves are committed at `testnet/keys/producer_{1..5}.json`, so anyone with the repository could have signed a release for a host resolving them. Do not re-couple the producer and maintainer roles.

- **Mainnet**: N1-N12 are producers only. Maintainers are 5 separate signing-only keys.
- **Testnet**: NT1-NT12 are producers only. Maintainers are 5 separate signing-only keys.

Bootstrap keys are hardcoded per-network in `BOOTSTRAP_MAINTAINER_KEYS_MAINNET` and `BOOTSTRAP_MAINTAINER_KEYS_TESTNET` (`crates/updater/src/constants.rs`).

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

The pseudocode above is the DESIGN. `crates/core/src/maintainer/derivation.rs` holds two
derivations, and **the node calls only one of them**:

* **`derive_canonical_maintainer_set` — the one the node actually calls.** It seats the
  first 5 producer registrations under the **canonical total order
  `(registered_at ASC, pubkey_bytes ASC)`** — not reader-enumeration order — so two nodes
  seating from the same producer set seat byte-identical keys. Its input is the **live
  `ProducerSet`**, not block history. At and above
  `maintainer_derivation_activation_height` the seed is **one-shot**
  (`maintainer_seed_is_done`, `bins/node/src/node/periodic.rs`); after the first seed the
  root changes only through governance transactions applied in `governance.rs`.
* **`derive_maintainer_set` — the replay-complete derivation.** Genesis seed plus every
  governance action up to height `H`, read through a `BlockchainReader`. It is a correct
  pure function and is covered by tests, but as of M2 it has **zero production callers**
  (`grep -rn derive_maintainer_set bins crates --include="*.rs"` returns only the
  `crates/core/src/lib.rs` re-export and test files). Wiring it into the node needs a real
  `BlockchainReader` over the block store; that is **M3 work, not done**.

**Operational consequence — do NOT delete `maintainer_state.bin` on a node whose chain is
intact.** Because the one-shot guard is a function of that file alone, an absent file
re-arms the seed and the node re-bootstraps from **live producer state at the current
height**. A maintainer key that governance legitimately REMOVED comes back, silently, with
no warning. This is measured, not theoretical (INC-I-172 M2 QA report, PROBE-1:
`removed_key_back=true`). It is **not a regression** — before M2 the root was re-derived on
*every* block, which was equal or worse — but it is an open residual, tracked as **R1** in
`docs/.workflow/inc-i-172-M3-scope.md`.

What DOES converge today:

| Recovery path | Converges? | Why |
|---|---|---|
| Full data-dir wipe + **full resync from genesis** | YES | Every block is re-applied, so `process_transaction_governance` re-executes the whole governance history |
| Delete `maintainer_state.bin` only, chain intact | **NO** | Re-seeds from live producer state — R1 |
| Full data-dir wipe + **snap sync** | **NO** | Governance below the snapshot floor is never replayed, and the node does not fail closed — R3 |

REQ-172-010 ("the trust root SHOULD be derivable from block history alone, **without
trusting `maintainer_state.bin`**") is therefore **M2 partial**, not closed. REQ-172-005's
convergence criterion holds for the full-sync and backfill paths only.

### 2.4 Maintainer Management Transactions

After bootstrap, the maintainer set can be modified via special transactions requiring 3/5 multisig:

> **INC-I-172 M2 — "3 of 5" means 3 DISTINCT signers.** Before M2 the verifier counted
> signature ENTRIES, so three copies of ONE maintainer key satisfied a 3-of-5 threshold
> (AUDIT-P0-010). At and above `maintainer_derivation_activation_height`
> (mainnet `172_000`, testnet `127_200`, devnet `0`) each maintainer contributes at most
> one to the count. Below that height the historical entry-counting result is reproduced
> exactly, because activation acceptance is consensus history. Independently of the gate,
> a set that is empty or carries a zero threshold now authorizes **nothing** at any
> height (AUDIT-P1-010 / FM-02).

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

### 2.6 Maintainer Set Persistence

The maintainer set is persisted as `MaintainerState` in `maintainer_state.bin` inside the node's data directory. This avoids re-deriving from genesis on every restart.

```
MaintainerState {
    version: u32,                // On-disk format tag, serialized FIRST (MAINTAINER_STATE_VERSION)
    set: MaintainerSet,          // Current members, threshold, last_updated
    last_derived_height: u64,    // Block height at which this was last modified
}
```

**Lifecycle:**
1. On startup: loaded from disk. A MISSING file is a fresh node (empty default). A file
   that does not decode, or carries an unknown `version`, is a FATAL startup error that
   names the file — never a silent empty trust root (INC-I-172 F5).
2. On each applied block: the genesis seed is attempted from the first 5 producers.
   At and above `maintainer_derivation_activation_height` it is **ONE-SHOT** — it fires
   only while the root has never been seeded (`members` empty AND
   `last_derived_height == 0`) and uses the canonical total order. Below that height the
   historical behavior is preserved: the seed re-fires whenever the set has fewer than 5
   members, which silently REVERTED a successful `RemoveMaintainer` on the next block
   (~10 s). That is the AUDIT-P1-013 defect M2 closes forward-only; it is not fixed
   retroactively because the root decides `ProtocolActivation` acceptance, which is
   consensus-visible.
3. On MaintainerAdd/Remove tx: updated immediately, persisted to disk
4. On release verification: `maintainer_trust_root_fn` resolves a `TrustRoot` from
   `MaintainerState` (`bins/node/src/updater/trust_root_wiring.rs`)

**Key lookup is O(1)** — reads 3-5 members, regardless of producer count.

### 2.7 Signature Verification Flow

```
Release published on GitHub
    ↓
UpdateService checks for new release (every check_interval)
    ↓
Downloads SIGNATURES.json (3+ signatures)
    ↓
maintainer_trust_root_fn() resolves the TrustRoot (INC-I-172 F1):
    ├─ members non-empty                       → TrustRoot::on_chain(members, set.threshold)
    ├─ members empty AND last_derived_height=0 → TrustRoot::bootstrap(network)
    │                                            (this node NEVER had an on-chain set)
    └─ members empty AND last_derived_height>0 → TrustRoot::on_chain(vec![], threshold)
                                                 → UNUSABLE → FAILS CLOSED.
                                                 There is NO fallback to the compiled keys.
    ↓
verify_release_with_trust_root():
    1. root.is_usable()? (threshold >= 1 AND keys.len() >= threshold)
       → if not: error! + UpdateError::TrustRootUnavailable. STOP. No fallback.
    2. DISTINCT-SIGNER count (covenant k-of-n shape, conditions/eval.rs):
       for each key in root.keys():
           for each signature entry: if it matches this key and verifies → +1, break
       (three entries from ONE key therefore count as ONE signer)
    ↓
    valid_count >= root.threshold()? → Verified ✓ else InsufficientSignatures
```

The same `TrustRoot` is re-resolved and re-checked immediately before install — by
`UpdateService::auto_apply` on the automatic path and by `apply_update`, which takes the
root as a **required parameter**, on the manual `doli-node update apply` path (F2). A key
revoked during the veto period therefore invalidates an already-staged update on both
paths instead of authorising it (F7(a)). Revocation that cannot reach the manual apply
is not revocation.

### 2.x Artifact binding (INC-I-172 F1)

A valid signature proves the maintainers signed *something*. It does not say **what is
being installed**. Every path that writes a binary therefore calls
`verify_release_artifact` (`crates/updater/src/install_gate.rs`), which checks four links
and blocks on any break:

| Link | Check |
|---|---|
| L1 | `SIGNATURES.json.version` equals the release tag being installed (modulo a leading `v`) |
| L2 | `SIGNATURES.json.checksums_sha256` equals `sha256(the CHECKSUMS.txt actually fetched)` — recomputed from the bytes, never read from a derived field |
| L3 | distinct valid signers ≥ `root.threshold()` |
| L4 | `sha256(tarball)` equals the per-platform hash parsed from **that** verified CHECKSUMS.txt |

Without L1 and L2 the check is circular: both operands of the signed message
`"{version}:{checksums_sha256}"` would be read out of the same file that carries the
signatures, so a verbatim copy of any past genuine `SIGNATURES.json` would authorise an
arbitrary tarball while every other check still passed.

---

## 3. Governance and Voting System

> **Corrected 2026-08-10 (INC-I-172 F8).** Everything this section previously
> described as "seniority-weighted voting" — a bond x seniority weight, a 4-year
> multiplier curve, a 30-day minimum voting age, and vote changing — was never
> reachable from production code. The functions existed but their only callers were
> `#[cfg(test)]`. They were deleted so the code and this document agree. What is
> written below is what the code does.

Veto voting is **count-based**: one registered producer, one vote. There is no
seniority weighting, no bond weighting and no minimum voting age.

### 3.1 Veto Threshold Calculation

```
veto_count      = number of DISTINCT producers that voted VETO
total_producers = number of active producers
veto_percent    = (veto_count * 100) / total_producers

if veto_percent >= 40%: REJECTED
if veto_percent <  40%: APPROVED
```

Implemented by `VoteTracker::should_reject` / `veto_percent`
(`crates/updater/src/vote.rs`) and `calculate_veto_result`
(`crates/updater/src/verification.rs`). `VETO_THRESHOLD_PERCENT` is 40.

**Example with 10 active producers:** 4 producers vote VETO -> 40% -> REJECTED.
3 producers vote VETO -> 30% -> APPROVED. Producers that do not vote count as
neither: the denominator is the ACTIVE PRODUCER count, not the votes cast, so
abstaining has the same effect as approving.

### 3.1.1 Sybil resistance, honestly stated

The only barrier to acquiring a veto vote is the bond required to register as a
producer. There is no time-based barrier in code. Do not budget risk against one.

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

**Not supported.** `VoteTracker::record_vote` returns `false` and keeps the first
vote if a producer has already voted either way. A producer that votes APPROVE and
then discovers a problem cannot switch to VETO. Vote deliberately.

#### 3.2.3 Offline Producers

Producers that are offline (not producing blocks) can still vote. The voting system uses gossip propagation, not block production. This ensures:

- Maintenance windows don't forfeit voting rights
- Network issues don't disenfranchise producers
- There is no minimum producer age; any producer the node recognises as active can vote

#### 3.2.4 Vote Finalization

Votes are counted at the exact moment the veto period expires. The result is deterministic: any node can independently verify the outcome by replaying all votes received before the deadline.

---

## 4. Sybil Resistance Analysis

> **Corrected 2026-08-10 (INC-I-172 F8).** The analysis that used to live here priced
> the attack against a seniority-weighted vote and a 30-day minimum voting age. Neither
> exists in code. The corrected analysis is below and it is weaker — that is the point
> of correcting it.

### 4.1 What actually resists a Sybil veto

One barrier: the bond required to register a producer. To reach the 40% veto threshold
an attacker must register enough producers to hold 40% of the ACTIVE producer count and
bond each of them. Registration also has to survive `ACTIVATION_DELAY` before the
producer is active. There is no seniority multiplier and no minimum voting age, so a
producer bonded today has exactly the same veto power as one bonded four years ago.

### 4.2 What does NOT resist it

- **Seniority weighting** — deleted; never executed.
- **30-day minimum voting age** — `is_eligible_to_vote` had zero callers; deleted.
- **A 7-day review window** — the veto period is `VETO_PERIOD` (5 minutes) or the
  network-specific `UpdateParams::veto_period_secs`. Report the configured value.

### 4.3 The real ceiling on this control

The veto protects against a maintainer quorum publishing an update the community
rejects *in the window it is given*. It does not protect against a compromised
maintainer quorum publishing a release that the community has no time to review, and
it never protected against a whale: bonding is the only cost.

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
│  ├── Power: 40% of active producers vote to reject (head count)                 │
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
│  T+0: RELEASE PUBLISHED                                                         │
│  ├── Maintainers build and test binaries for all platforms                      │
│  ├── 3 of 5 maintainers sign SIGNATURES.json offline                            │
│  ├── Release published to GitHub and mirrors                                    │
│  └── All nodes display mandatory notification                                   │
│                                                                                 │
│  T+0 to T+5min: VETO PERIOD *                                                   │
│  ├── Producers review changelog and code changes                                │
│  ├── Community discussion on forums/Discord                                     │
│  ├── Producers submit votes (can change until deadline)                         │
│  └── Real-time veto percentage displayed on all nodes                           │
│                                                                                 │
│  T+5min: RESOLUTION *                                                            │
│  ├── Veto head count tallied at exact deadline                                  │
│  ├── If veto >= 40%: Update REJECTED, discarded                                 │
│  └── If veto <  40%: Update APPROVED, grace period begins                       │
│                                                                                 │
│  T+5min to T+7min: GRACE PERIOD (2 min) *                                       │
│  ├── Approved update downloaded and verified                                    │
│  ├── Operators can manually apply early: doli-node update apply                 │
│  └── Outdated nodes can still produce blocks                                    │
│                                                                                 │
│  T+7min+: ENFORCEMENT ACTIVE                                                    │
│  ├── Nodes below required version: production PAUSED                            │
│  ├── Outdated nodes can still sync, serve RPC, relay transactions               │
│  └── Update and restart to resume production                                    │
│                                                                                 │
│  * Early-network timings (v1.1.13+). Will extend to 7d/48h as network grows.   │
│  Total notice before enforcement: ~7 MINUTES (early network)                    │
│                                                                                 │
└─────────────────────────────────────────────────────────────────────────────────┘
```

### 5.1 Producer Notifications

Producers receive automatic notifications through three channels:

1. **Banner on ANY CLI command** while update is pending
2. **`doli-node update status`** for full details
3. **Periodic log messages** (every check interval — currently 10 minutes)

The notification content changes based on the current state:

#### State 1: VOTING PERIOD (T+0 to T+5min) - Not Yet Voted

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

#### State 3: GRACE PERIOD (T+5min to T+7min)

**Banner:**
```
╔═════════════════════════════════════════════════════════════════════════════════════╗
║  ✅  UPDATE 1.0.1 APPROVED  |  36h left  |  doli-node update apply                  ║
╚═════════════════════════════════════════════════════════════════════════════════════╝
```

#### State 4: PRODUCTION PAUSED (T+7min+, not updated)

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

## 6. Rollback (MANUAL — the automatic system is NOT implemented)

> **STATUS: NOT IMPLEMENTED (INC-I-172 M1 security audit, AUDIT-P1-014).**
> Everything in 6.1-6.3 below is a DESIGN, not a description of running code.
> `crates/updater/src/watchdog.rs` exists but has **zero production callers**: nothing in
> `bins/node` or `bins/cli` constructs `UpdateWatchdog` or calls
> `check_and_maybe_rollback()`, and `UpdateConfig::auto_rollback` is written in three
> places and read in none. **No DOLI node rolls back automatically.** If a release
> crashes a node, the node stays down until an operator intervenes on that host — and
> auto-update is enabled by default, so a bad release reaches the fleet unattended.
>
> What DOES exist: `apply_update` writes `{binary}.backup` before installing, and
> `doli-node update rollback` restores it. That is the whole rollback story today, and
> it is manual. See 6.5.

### 6.1 Crash Detection Watchdog (DESIGN — not wired)

The design monitors the node process with a lightweight watchdog that detects repeated
crashes after an update. **No such monitoring runs today.**

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

### 6.2 Rollback Trigger Conditions (DESIGN — not wired)

1. Node crashes **3+ times** within 1 hour of update application
2. Node fails to reach sync within 5 minutes of restart
3. Node fails health checks (RPC unresponsive, peers disconnected)

### 6.3 Rollback Process (DESIGN — not wired)

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

### 6.4 Post-Rollback Behavior (DESIGN — not wired)

- Node continues operating on previous version
- Update marked as "failed locally" (not network-wide rejection)
- Operator notified via logs and optional webhook
- Manual intervention required to retry update
- Node can still produce blocks (if previous version meets requirements)

### 6.5 What actually happens today

1. `apply_update` copies the running binary to `{binary}.backup` before installing.
2. If the new release is bad, **nothing detects it**. The node crash-loops or misbehaves
   until an operator notices.
3. The operator rolls back by hand:

   ```bash
   systemctl stop doli-mainnet-nodeN
   cp ~/.doli/doli-node.backup $(which doli-node)
   systemctl start doli-mainnet-nodeN
   ```

   or `doli-node update rollback`, which does the same restore.
4. `--no-auto-rollback` is accepted for backward compatibility and does nothing.

Wiring the watchdog is tracked as remediation for AUDIT-P1-014; until it lands, do not
plan an upgrade on the assumption that a bad release self-heals.

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
│  T+0          T+5min             T+7min                         T+~30d *       │
│    │            │                  │                               │            │
│    ▼            ▼                  ▼                               ▼            │
│  Release     Veto period       Grace ends                    Activation        │
│  published   ends              (if approved)                 height reached    │
│    │            │                  │                               │            │
│    └────────────┴──────────────────┴───────────────────────────────┤            │
│    │◄── 5 min ──►│◄── 2 min ───►│◄────── ~30 days ──────────────►│            │
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
│ Veto period         │ 5 min *             │ 5 min *             │
│ Total notice        │ ~7 min *            │ ~30 days            │
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
│ (3 keys)           │ releases                │ within veto period      │              │
├────────────────────┼─────────────────────────┼─────────────────────────┼──────────────┤
│ Sybil veto         │ Block legitimate        │ Registration bond ONLY  │ Medium       │
│ attack             │ updates                 │ (no weighting, no age)  │              │
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
│  ├── 40% veto threshold, by producer head count                                 │
│  ├── Mandatory veto review period (5 min early network *)                       │
│  ├── One vote per producer, first vote is final                                 │
│  └── Transparent maintainer set (derived from chain)                            │
│                                                                                 │
│  Layer 3: ECONOMIC                                                              │
│  ├── Bond requirement for producers                                             │
│  └── (no seniority weighting, no minimum voting age — deleted INC-I-172)        │
│                                                                                 │
│  Layer 4: OPERATIONAL                                                           │
│  ├── Backup preservation before update (the ONLY one of these implemented)      │
│  ├── Automatic rollback on failure — NOT IMPLEMENTED (AUDIT-P1-014, see §6)     │
│  └── Health monitoring and alerting — NOT IMPLEMENTED                           │
│                                                                                 │
└─────────────────────────────────────────────────────────────────────────────────┘
```

---

## 9. CLI Command Reference

DOLI has three upgrade paths:

| Path | Binary | Purpose | Signatures required? |
|------|--------|---------|---------------------|
| **Operator upgrade** | `doli upgrade` | Manual, operator-driven. Downloads from GitHub, installs both `doli` + `doli-node`, restarts a service. | **Yes — install ABORTS on failure** (INC-I-172 F6) |
| **Auto-update** | `doli-node update` | Autonomous. Node checks for updates, verifies 3/5 maintainer sigs, respects veto period, applies automatically. | Yes (3/5 required) |
| **Legacy node upgrade** | `doli-node upgrade` | Manual. Downloads from GitHub and installs `doli-node` only. | **No — checksum only, NO signature check** |

Use `doli upgrade` for planned rolling upgrades. Use `doli-node update` / auto-update for autonomous operation.

> **All five install paths are gated.** `doli upgrade`, `doli-node upgrade`,
> `doli-node update apply`, `apply_update` and the auto-updater each verify maintainer
> signatures **bound to the artifact** before anything is extracted, backed up or
> written, and each aborts on failure or on an absent `SIGNATURES.json`.
>
> On a **producer host, prefer `doli-node upgrade`**: it resolves the trust root from
> that host's on-chain `maintainer_state.bin` (INC-I-172 F3). The `doli` CLI can only
> use the compile-time bootstrap root — it is not the node host and has no chain state.

### 9.1 Operator Upgrade (`doli upgrade`)

For planned upgrades from a machine that is not a node. Downloads the release tarball
from GitHub, **verifies maintainer signatures from `SIGNATURES.json` bound to that exact
tarball (§2.x L1-L4) and aborts before installing anything if any link breaks or the file
is absent**, installs both `doli` and `doli-node` binaries via atomic rename, and
restarts the specified systemd service. Its trust root is the compile-time bootstrap
set.

```bash
# Upgrade to latest version
doli upgrade --yes --service doli-mainnet-node3

# Upgrade to specific version
doli upgrade --version 1.1.11 --yes --service doli-mainnet-node5

# Custom doli-node path (required on servers where doli-node is not in the fallback chain)
doli upgrade --yes --doli-node-path ~/repos/doli/target/release/doli-node --service doli-mainnet-node1

# With sudo (required on N4/N5 where binaries are in /opt/)
sudo /opt/doli/target/release/doli upgrade --yes --service doli-mainnet-node4
```

**Flags:**

| Flag | Required? | Description |
|------|-----------|-------------|
| `--version <VER>` | No | Target version (default: latest GitHub release) |
| `--yes` | No | Skip confirmation prompt |
| `--doli-node-path <PATH>` | Depends | Path to `doli-node` binary. Required if not in: `which doli-node`, `/usr/local/bin/doli-node`, or `/opt/doli/target/release/doli-node` |
| `--service <SERVICE>` | Recommended | Systemd service to restart. **Critical** on multi-node servers (omegacortex has N1+N2+N6) |

**How it works:**
1. Fetches release metadata from GitHub (`doli-network/doli`)
2. Downloads platform tarball (auto-detects linux x86_64 / darwin aarch64)
3. Verifies SHA-256 checksum from `CHECKSUMS.txt`
4. Checks maintainer signatures (informational warning — does **not** block the upgrade)
5. Installs `doli` binary (to its own path via `current_exe()`)
6. Installs `doli-node` binary (auto-detected or `--doli-node-path`)
7. Restarts the specified `--service`

**Note:** If the binary is already at the target version, `doli upgrade` prints "Already up to date" and exits without restarting the service. To restart a service with an already-updated binary, use `sudo systemctl restart <service>` directly.

For the full per-server command reference and upgrade sequence, see the ops runbook (`.claude/skills/doli-ops/SKILL.md`, Section 3.8).

### 9.2 Auto-Update Management (`doli-node update`)

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

### 9.3 Voting (Producers Only)

```bash
# Vote to VETO (block) an update
doli-node update vote --veto --key /path/to/producer.json

# Vote to APPROVE an update
doli-node update vote --approve --key /path/to/producer.json

# View current vote status for a version
doli-node update votes --version 1.0.1
```

### 9.4 Maintainer Management

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

### 9.5 Node Run Options

```bash
# Run with auto-updates enabled (default)
doli-node run --network mainnet

# Disable auto-updates entirely
doli-node run --network mainnet --no-auto-update

# Notify only (check but don't apply)
doli-node run --network mainnet --update-notify-only

# Accepted but a NO-OP: no automatic rollback exists (AUDIT-P1-014, see section 6)
doli-node run --network mainnet --no-auto-rollback

# Full production setup
doli-node run \
  --network mainnet \
  --producer \
  --producer-key /path/to/producer.json \
  --rpc-bind 0.0.0.0 --rpc-port 28500
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
│ Maintainer set management       │ crates/core/src/maintainer/                 │
│ Maintainer transactions         │ crates/core/src/transaction.rs              │
│ Maintainer validation           │ crates/core/src/validation.rs               │
│ Core updater library            │ crates/updater/src/lib.rs                   │
│ Network-aware update params     │ crates/updater/src/params.rs                │
│ Release trust root (fail-closed)│ crates/updater/src/trust_root.rs            │
│ Vote tracking                   │ crates/updater/src/vote.rs                  │
│ Watchdog (rollback) NOT WIRED   │ crates/updater/src/watchdog.rs              │
│ Hard fork logic                 │ crates/updater/src/hardfork.rs              │
│ Binary download                 │ crates/updater/src/download.rs              │
│ Binary verification             │ crates/updater/src/verification.rs          │
│ Update application              │ crates/updater/src/apply.rs                 │
│ Node integration                │ bins/node/src/updater/ (mod, service, cli)  │
│ CLI commands                    │ bins/cli/src/cmd_upgrade.rs                 │
│ Maintainer CLI                  │ bins/cli/src/cmd_governance.rs               │
│ RPC methods                     │ crates/rpc/src/methods/governance.rs        │
│ Gossip topics                   │ crates/network/src/gossip/mod.rs            │
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
    RemoveMaintainer = 11,
    AddMaintainer = 12,
}

pub struct MaintainerChangeData {
    pub target: PublicKey,
    pub signatures: Vec<MaintainerSignature>,
    pub reason: Option<String>,
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

/// Update watchdog state — NOT WIRED, zero production callers (AUDIT-P1-014, see §6)
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
// Early-network values (v1.1.13+). Set in network_params.rs, not hardcoded here.
// Mainnet/Testnet: veto=5min, grace=2min, check=10min
// Devnet: veto=1min, grace=30s, check=10s
// Target (mature network): veto=7 days, grace=48 hours, check=6 hours
pub const VETO_PERIOD: Duration = Duration::from_secs(5 * 60);            // 5 minutes
pub const GRACE_PERIOD: Duration = Duration::from_secs(3600);             // 1 hour (fallback; network_params overrides to 2 min)
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);       // 6 hours (fallback; network_params overrides to 10 min)

// Thresholds
pub const VETO_THRESHOLD_PERCENT: u8 = 40;

// Rollback
pub const CRASH_THRESHOLD: u32 = 3;
pub const CRASH_WINDOW: Duration = Duration::from_secs(3600);             // 1 hour

// Distribution
pub const GITHUB_API_URL: &str = "https://api.github.com/repos/doli-network/doli/releases/latest";
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
│ Voting system       │ Miner      │ None       │ None       │ Producer head   │
│                     │ signaling  │ (social)   │            │ count           │
├─────────────────────┼────────────┼────────────┼────────────┼─────────────────┤
│ Veto power          │ 95%        │ Social     │ None       │ 40% of active   │
│                     │ threshold  │ consensus  │            │                 │
├─────────────────────┼────────────┼────────────┼────────────┼─────────────────┤
│ Time to update      │ Months to  │ Weeks to   │ Hours      │ ~7 min *        │
│                     │ years      │ months     │            │ (early network) │
├─────────────────────┼────────────┼────────────┼────────────┼─────────────────┤
│ Automatic apply     │ No         │ No         │ Yes        │ Yes             │
│                     │            │            │            │ (with veto)     │
├─────────────────────┼────────────┼────────────┼────────────┼─────────────────┤
│ Automatic rollback  │ No         │ No         │ No         │ No (not impl.)  │
├─────────────────────┼────────────┼────────────┼────────────┼─────────────────┤
│ Sybil resistance    │ Hashpower  │ Stake      │ Stake      │ Producer bond   │
│                     │            │            │            │ only            │
└─────────────────────┴────────────┴────────────┴────────────┴─────────────────┘
```

**DOLI's advantages**:
1. **No hardcoded authority**: Maintainers emerge from blockchain participation
2. **Transparent transitions**: All maintainer changes are on-chain and verifiable
3. **Democratic oversight**: Producers can veto any release
4. **Operational efficiency**: Auto-update with rollback prevents stale nodes
5. **Honest limits**: the veto is a producer head count whose only Sybil barrier is the registration bond — see section 4

---

## 13. Frequently Asked Questions

### Q: How are the initial maintainers chosen?

**Automatically.** The first 5 producers to register become maintainers. There's no pre-selection, no governance vote, no configuration file. The blockchain itself determines who the maintainers are.

### Q: Can maintainers force an update without community consent?

**No.** Even with 3/5 maintainer signatures, the community has the veto period (the CONFIGURED `veto_period_secs`; 5 minutes on the current network) to review and veto. If 40% of ACTIVE PRODUCERS object, the update is rejected. Maintainers propose; the community disposes. Note the honest limit: 5 minutes is not a meaningful review window, and the veto is a head count with no Sybil resistance beyond the bond.

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

There is NO automatic rollback (AUDIT-P1-014): `UpdateWatchdog` is written but never called, so nothing detects a post-update crash. Roll back manually with `doli-node update rollback`, which restores the `{binary}.backup` written before the install. Your backup is always preserved.

### Q: Is voting weight based on seniority?

No. It was documented that way for a long time, but the weighting code was never reachable from production and was deleted in INC-I-172. Every active producer has exactly one veto vote.

### Q: What's the difference between "approve" and not voting?

Functionally the same. The system uses **veto-based** governance:
- Updates pass by default unless blocked
- Only VETO votes count toward the 40% threshold
- Abstaining = implicit approval

This prevents "voter apathy" from blocking important security updates.

### Q: Can I change my vote?

**Yes.** You can change your vote at any time during the veto period. Only your latest vote (by timestamp) counts at the deadline. This allows reaction to new information discovered during review.

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

- **Version**: 3.1
- **Last Updated**: August 2026
- **Status**: Production Specification

### Changes from v3.0 (INC-I-172 M1)

- ✅ Removed the seniority-weighted veto, the 4x multiplier curve, the 30-day minimum
  voting age and vote changing. None of them executed: their only callers were tests.
  The veto is a producer head count and always was.
- ✅ Release verification now runs against a resolved `TrustRoot` that FAILS CLOSED.
  An on-chain maintainer set that exists and is empty no longer falls back to the
  compile-time bootstrap keys.
- ✅ Signature counting is now by DISTINCT SIGNER: three entries from one key count as one.
- ✅ Veto and grace deadlines are measured from the node-local `first_notified_at`,
  never from the unsigned `Release::published_at`.
- ✅ `doli upgrade` and `doli-node update verify` now ABORT on verification failure.

### Changes from v2.0

- ✅ Added maintainer bootstrap system (first 5 producers)
- ✅ Added on-chain maintainer management (Add/Remove transactions)
- ✅ Added maintainer CLI commands and RPC endpoints
- ✅ Added edge case handling for maintainer set
- ✅ Updated security model for on-chain maintainer verification
- ✅ Added hard fork scheduler recalculation documentation
- ✅ Expanded FAQ with maintainer-related questions

---

*DOLI Protocol - Decentralized, Democratic, Secure*

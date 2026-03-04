---
name: auto-update
description: Use this skill when implementing, fixing, or completing the DOLI auto-update system. Covers maintainer governance, seniority+stake weighted voting, watchdog/rollback, hard fork support, storage persistence, CLI commands, and E2E testing. Triggers on "auto-update", "update system", "maintainer", "governance", "veto", "seniority", "watchdog", "hard fork", "rollback".
version: 1.0.0
---

# DOLI Auto-Update Implementation Skill

## Architecture Overview

The auto-update system enables transparent, community-governed binary updates with veto power.

```
Flow: Release published → Veto period → Voting → Approved/Rejected → Grace period → Enforcement
      (3/5 maintainer sigs)  (7 days mainnet)  (bonds+seniority weighted)     (48h)
```

### Core Principle: Vote Weight = Staked Bonds × Seniority Multiplier

A producer's governance power combines economic stake (skin in the game) with time commitment (loyalty). Neither alone is sufficient — a whale with 100 bonds registered yesterday should not outweigh a veteran with 2 bonds who has been securing the network for 3 years.

```
vote_weight = bond_count × seniority_multiplier

seniority_multiplier = 1.0 + min(years_active, 4) × 0.75
  Year 0: 1.0x    (new producer, weight = bonds × 1.0)
  Year 1: 1.75x   (weight = bonds × 1.75)
  Year 2: 2.50x   (weight = bonds × 2.50)
  Year 3: 3.25x   (weight = bonds × 3.25)
  Year 4+: 4.0x   (weight = bonds × 4.0, capped)

Examples:
  2-bond producer, 4 years active:  2 × 4.0 = 8.0 weight
  10-bond producer, 0 years active: 10 × 1.0 = 10.0 weight
  10-bond producer, 4 years active: 10 × 4.0 = 40.0 weight
  1-bond producer, 2 years active:  1 × 2.5 = 2.5 weight
```

This means a Sybil attacker creating 100 fresh 1-bond producers gets weight 100 (100 × 1.0), while 25 veteran 1-bond producers at 4 years get weight 100 (25 × 4.0) — equal power with 4× fewer nodes and 4× less cost, but 4 years of proven commitment.

---

## Current Implementation Status

### ✅ DONE — Do not rewrite
| Component | Location | Lines | Notes |
|-----------|----------|-------|-------|
| `UpdateParams` | `crates/updater/src/lib.rs` | 195-300 | Network-aware params, veto/grace periods |
| `Release`, `MaintainerSignature` | `crates/updater/src/lib.rs` | 307-340 | Release struct with signatures |
| `VersionEnforcement` | `crates/updater/src/lib.rs` | 422-470 | Enforcement deadline calculation |
| `VoteTracker` (weighted) | `crates/updater/src/vote.rs` | Full file | Weighted veto/approval, `should_reject_weighted()` |
| `VoteMessage` | `crates/updater/src/vote.rs` | 21-85 | Signed vote messages |
| `apply_update`, `backup_current`, `rollback` | `crates/updater/src/apply.rs` | Full file | Binary apply/backup/rollback |
| `download_binary`, `verify_hash` | `crates/updater/src/download.rs` | Full file | Download and hash verify |
| `test_keys` | `crates/updater/src/test_keys.rs` | Full file | Devnet test maintainer keys |
| `MaintainerSet` | `crates/core/src/maintainer.rs` | Full 716 lines | Full implementation with multisig |
| `TxType::AddMaintainer/RemoveMaintainer` | `crates/core/src/transaction.rs` | Types 11,12 | Transaction types defined |
| Maintainer tx validation | `crates/core/src/validation.rs` | 1783+ | Structural validation |
| Network params | `crates/core/src/network_params.rs` | 375-415 | All update timing params |
| VOTES_TOPIC gossip | `crates/network/src/gossip.rs` | 24, 99 | Vote gossip topic subscribed |
| `UpdateService` | `bins/node/src/updater.rs` | 390-720 | Periodic check, download, veto tracking |
| `PendingUpdate` | `bins/node/src/updater.rs` | 39-115 | JSON persistence of pending updates |
| `display_update_notification` | `bins/node/src/updater.rs` | 116-388 | Banner display with colors |
| RPC `submitVote` | `crates/rpc/src/methods.rs` | ~213 | Broadcasts via gossip |
| RPC `getMaintainerSet` | `crates/rpc/src/methods.rs` | ~215 | Derives from producer set |

### ⚠️ NEEDS MODIFICATION — Change existing code
| What | Where | Change Needed |
|------|-------|---------------|
| Vote forwarding | `bins/node/src/node.rs` | Must connect gossip votes to UpdateService via `vote_tx` channel |
| `getUpdateStatus` RPC | `crates/rpc/src/methods.rs` | Returns placeholder. Must query actual UpdateService state |

### ✅ PREVIOUSLY MISSING — Now implemented
| Component | Location | Notes |
|-----------|----------|-------|
| `calculate_vote_weight()` | `crates/updater/src/lib.rs:241` | Uses `bond_count × seniority_multiplier` |
| `watchdog.rs` | `crates/updater/src/watchdog.rs` | Crash detection and auto-rollback |
| `hardfork.rs` | `crates/updater/src/hardfork.rs` | Upgrade-at-height mechanism |
| `storage/maintainer.rs` | `crates/storage/src/maintainer.rs` | Persist maintainer set |
| `storage/update.rs` | `crates/storage/src/update.rs` | Persist update state, votes |
| CLI `update` subcommand | `bins/cli/src/main.rs` | check, status, vote, votes, apply, rollback |
| CLI `maintainer` subcommand | `bins/cli/src/main.rs` | list |
| CLI `protocol` subcommand | `bins/cli/src/main.rs` | sign, activate |

### ❌ MISSING — Must implement
| Component | Location | Milestone |
|-----------|----------|-----------|
| E2E test: veto flow | `scripts/test_update_veto.sh` | M11 |
| E2E test: rollback | `scripts/test_rollback.sh` | M11 |
| E2E test: hard fork | `scripts/test_hard_fork.sh` | M11 |

---

## Implementation Milestones (execute in order)

### M1: Vote Weight = Bonds × Seniority

**Goal:** Change vote weight from seniority-only to bonds × seniority.

**File: `crates/updater/src/lib.rs`**

Replace `calculate_vote_weight()` (line 259):
```rust
// BEFORE: seniority only
pub fn calculate_vote_weight(&self, blocks_active: u64) -> f64 {
    let years = blocks_active as f64 / self.seniority_step_blocks as f64;
    let capped_years = years.min(4.0);
    1.0 + capped_years * 0.75
}

// AFTER: bonds × seniority
pub fn calculate_vote_weight(&self, bond_count: u32, blocks_active: u64) -> f64 {
    let years = blocks_active as f64 / self.seniority_step_blocks as f64;
    let capped_years = years.min(4.0);
    let seniority_multiplier = 1.0 + capped_years * 0.75;
    bond_count as f64 * seniority_multiplier
}
```

**Update all callers of `calculate_vote_weight()`** to pass `bond_count`. The bond count comes from `ProducerInfo.bond_count` in `crates/storage/src/producer.rs`.

**Update `VoteTracker` weight population** wherever weights are set. The weight for each producer must be:
```rust
let weight = params.calculate_vote_weight(producer.bond_count, blocks_active);
// Store as u64: multiply by 100 for 2-decimal precision
let weight_u64 = (weight * 100.0) as u64;
weights.insert(producer_id, weight_u64);
```

**Tests:** Verify:
- 1 bond, 0 years → weight 100 (1.0 × 100)
- 10 bonds, 0 years → weight 1000 (10.0 × 100)
- 2 bonds, 4 years → weight 800 (8.0 × 100)
- 10 bonds, 4 years → weight 4000 (40.0 × 100)
- Veto with 40% of total weight triggers rejection

**Gate:** `cargo test -p updater` passes.

---

### M2: Connect Vote Forwarding

**Goal:** Votes received via gossip reach the UpdateService.

**File: `bins/node/src/node.rs` (line ~1081)**

The TODO says: "Forward to updater service via vote_tx channel". The `UpdateService` already has `vote_sender()` returning an `mpsc::Sender<VoteMessage>`. Wire it:

1. In `Node` struct, store `vote_tx: Option<mpsc::Sender<VoteMessage>>`
2. When `spawn_update_service()` is called, capture the vote_tx
3. In the `NetworkEvent::NewVote` handler:
```rust
NetworkEvent::NewVote(vote_data) => {
    if let Ok(vote_msg) = serde_json::from_slice::<VoteMessage>(&vote_data) {
        if let Some(ref vote_tx) = self.vote_tx {
            let _ = vote_tx.try_send(vote_msg);
        }
    }
}
```

**Gate:** Votes received via gossip are processed by UpdateService (verify in logs).

---

### M3: Watchdog and Auto-Rollback

**Goal:** Detect crashes after update and auto-rollback.

**New file: `crates/updater/src/watchdog.rs`**

```rust
pub struct UpdateWatchdog {
    state_file: PathBuf,
    network: Network,
}

pub struct WatchdogState {
    pub last_update_version: Option<String>,
    pub last_update_time: Option<u64>,
    pub crash_timestamps: Vec<u64>,
}
```

**Logic:**
1. On startup after update: record in watchdog state file
2. On clean shutdown: clear crash history
3. On startup after crash (no clean shutdown marker):
   - Record crash timestamp
   - Count crashes within `network.crash_window_secs()` (default 3600s mainnet, 60s devnet)
   - If count >= `network.crash_threshold()` (default 3): trigger `rollback()`
4. After rollback: broadcast notification, clear state

**Integrate with `apply.rs`:**
- Before `apply_update()`: call `watchdog.record_update(version)`
- In node startup: call `watchdog.check_and_maybe_rollback()`

**Persist state:** JSON file at `{data_dir}/watchdog_state.json`

**Gate:** Test simulates 3 crashes within window → rollback triggered.

---

### M4: Hard Fork Support

**New file: `crates/updater/src/hardfork.rs`**

```rust
pub struct HardForkInfo {
    pub activation_height: u64,
    pub min_version: String,
    pub consensus_changes: Vec<String>,
}
```

**Logic:**
1. Release can include `hard_fork: Option<HardForkInfo>`
2. If hard fork: node MUST update before `activation_height`
3. At `activation_height`: consensus rules change (e.g., new tx types, parameter changes)
4. Nodes on old version stop producing at `activation_height` (safety measure)
5. Banner shows countdown: "Hard fork in X blocks — update required"

**Integration point:** In `node.rs` block application, check if current height reaches any pending hard fork activation. If node version < `min_version`, stop producing and show error.

**Gate:** Test with devnet hard fork at height 100 — old-version nodes stop producing.

---

### M5: Storage Persistence

**New files:**
- `crates/storage/src/maintainer.rs` — Persist `MaintainerSet` derived from chain
- `crates/storage/src/update.rs` — Persist pending releases, votes, update history

**Why needed:** Currently, MaintainerSet is derived on-the-fly and VoteTracker is in-memory only. Restart = lost votes.

**`maintainer.rs`:**
- Cache derived MaintainerSet with `last_derived_height`
- On restart: load cache, derive forward from `last_derived_height` to chain tip
- Column family: `cf_maintainer_state`

**`update.rs`:**
- Store pending releases, vote state per version, applied update history
- Column families: `cf_pending_releases`, `cf_votes`, `cf_update_history`

**Gate:** Restart node → votes and pending releases survive.

---

### M6: `getUpdateStatus` RPC

**File: `crates/rpc/src/methods.rs`**

Replace the placeholder at line ~704:
```rust
async fn get_update_status(&self) -> Result<Value, RpcError> {
    // Query actual UpdateService state via shared Arc<RwLock<UpdateState>>
    // Return: pending release, veto count, veto %, grace deadline, enforcement status
}
```

The `UpdateService` needs to expose its state via a shared `Arc<RwLock<UpdateState>>` that the RPC server can read.

**Gate:** `curl getUpdateStatus` returns real data.

---

### M7: CLI Commands

**Add to `bins/cli/src/main.rs`** (no separate files needed for now):

**Update commands:**
```
doli update check                              # Check for available updates
doli update status                             # Show pending update, veto status, deadlines
doli update vote --veto --version X.Y.Z        # Vote to veto
doli update vote --approve --version X.Y.Z     # Vote to approve
doli update votes --version X.Y.Z              # Show current votes
doli update apply                              # Manually apply approved update
doli update rollback                           # Manual rollback
```

**Maintainer commands:**
```
doli maintainer list                           # List current maintainers
doli maintainer add --target <pubkey> --key <signer_wallet>
doli maintainer remove --target <pubkey> --key <signer_wallet>
```

Implementation: Each command calls the corresponding RPC endpoint. Vote commands sign with the producer wallet key.

**Gate:** Each command works against a running devnet node.

---

### M8: E2E Tests

**Scripts:**

`scripts/test_update_veto.sh`:
1. Start 5-node devnet
2. Publish test release (signed with test keys)
3. Have 3/5 producers vote veto (>40%)
4. Verify release is REJECTED
5. Publish another release
6. Have 1/5 vote veto (<40%)
7. Verify release is APPROVED after veto period

`scripts/test_rollback.sh`:
1. Start devnet
2. Apply test update
3. Kill node 3 times within crash window
4. Verify rollback triggered automatically
5. Verify node returns to previous version

`scripts/test_hard_fork.sh`:
1. Start devnet
2. Publish hard fork release at activation_height=50
3. Update 4/5 nodes
4. Verify updated nodes continue producing at height 50
5. Verify non-updated node stops producing at height 50

**Gate:** All scripts pass on fresh devnet.

---

## Devnet Testing: Real Servers, Accelerated Time

**CRITICAL:** All E2E tests run against REAL devnet nodes (not mocks). Devnet parameters are designed so a full update lifecycle completes in ~5 minutes.

### Devnet Timing (from `network_params.rs`)

```
Slot duration:          10 seconds
Blocks per year:        144 blocks (~24 minutes real time)
Seniority step:         144 blocks (= 1 "year", ~24 min)
Seniority maturity:     576 blocks (= 4 "years", ~96 min)
Veto period:            60 seconds (6 blocks)
Grace period:           30 seconds (3 blocks)
Min voting age:         60 seconds (6 blocks)
Update check interval:  10 seconds (1 block)
Crash window:           60 seconds (6 blocks)
Crash threshold:        3 crashes
```

### Complete Update Lifecycle on Devnet (~5 minutes)

```
T+0:00  Start 5-node devnet, wait for genesis (24 blocks = 4 min)
T+4:00  Genesis complete, all nodes producing
T+4:10  Publish test release (signed with test_keys)
T+4:10  Veto period starts (60 seconds)
T+4:20  Producers cast votes via CLI or RPC
T+5:10  Veto period ends → APPROVED or REJECTED
T+5:10  If approved: grace period starts (30 seconds)
T+5:40  Grace period ends → enforcement begins
T+5:40  Nodes that haven't updated get enforcement warning
```

### Seniority on Devnet

With `blocks_per_year = 144` and `slot_duration = 10`:
- 1 devnet "year" = 144 blocks = 24 minutes real time
- Seniority multiplier reaches 1.75x after 24 min
- Seniority multiplier reaches 4.0x (max) after 96 min
- Min voting age = 60s = 6 blocks (producers can vote almost immediately)

For testing bond×seniority weighting, create producers at different times:
```
T+0:    Register producers A,B,C (genesis, will have seniority by test time)
T+4:00  Genesis ends, A/B/C have been active ~24 blocks
T+4:10  Register producer D (new, seniority ≈ 0)
T+4:20  Compare weights:
        A (1 bond, 24 blocks active): 1 × ~1.125 = 1.125
        D (10 bonds, 1 block active): 10 × ~1.005 = 10.05
        D has higher weight (economic stake dominates early)
```

To test seniority dominance, run devnet for 30+ minutes:
```
T+30:00  A (1 bond, ~180 blocks = 1.25 years): 1 × 1.9375 = 1.94
         D (10 bonds, ~156 blocks = 1.08 years): 10 × 1.81 = 18.1
         D still dominates (needs ~4 years for A's seniority to matter)
```

### E2E Test Procedures (Real Devnet)

**`scripts/test_update_veto.sh`:**
```bash
#!/bin/bash
set -e

# Phase 1: Start clean devnet
doli-node devnet stop 2>/dev/null || true
doli-node devnet clean 2>/dev/null || true
doli-node devnet init --nodes 5
doli-node devnet start

echo "Waiting for genesis (24 blocks × 10s = ~4 minutes)..."
sleep 250  # 4 min + buffer

# Verify all nodes are healthy
for i in $(seq 0 4); do
  HEIGHT=$(curl -s http://127.0.0.1:$((28545+i)) -X POST \
    -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq -r '.result.bestHeight')
  echo "Node $i height: $HEIGHT"
  if [ "$HEIGHT" -lt 24 ]; then echo "FAIL: Node $i not past genesis"; exit 1; fi
done

# Phase 2: Publish test release (uses test_keys for devnet signing)
echo "Publishing test release v99.0.0..."
# The release is published via RPC or file drop — agent implements this
# Release must be signed by 3/5 test maintainer keys

# Phase 3: Wait for veto period to start, then cast veto votes
sleep 15  # Let release propagate

echo "Casting 3/5 veto votes (should exceed 40% threshold)..."
for i in 0 1 2; do
  doli -r http://127.0.0.1:$((28545+i)) \
    -w ~/.doli/devnet/keys/producer_$i.json \
    update vote --veto --version 99.0.0
  sleep 2
done

# Phase 4: Wait for veto period to end
echo "Waiting for veto period (60s)..."
sleep 65

# Phase 5: Verify REJECTED
STATUS=$(curl -s http://127.0.0.1:28545 -X POST \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getUpdateStatus","params":{},"id":1}')
echo "Update status: $STATUS"
# Verify status shows REJECTED

echo "TEST PASSED: Veto with >40% weight rejected the update"
```

**`scripts/test_update_approve.sh`:**
```bash
#!/bin/bash
set -e

# Uses same devnet from veto test (or fresh)
echo "Publishing test release v99.1.0..."
# Publish release

sleep 15  # Propagation

echo "Casting 1/5 veto votes (should NOT exceed 40%)..."
doli -r http://127.0.0.1:28545 \
  -w ~/.doli/devnet/keys/producer_0.json \
  update vote --veto --version 99.1.0

echo "Waiting for veto period (60s)..."
sleep 65

# Verify APPROVED
STATUS=$(curl -s http://127.0.0.1:28545 -X POST \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getUpdateStatus","params":{},"id":1}')
echo "Update status: $STATUS"
# Verify status shows APPROVED

echo "Waiting for grace period (30s)..."
sleep 35

# Verify enforcement active
echo "TEST PASSED: Update approved with <40% veto, enforcement active"
```

**`scripts/test_rollback.sh`:**
```bash
#!/bin/bash
set -e

# Start fresh devnet
doli-node devnet stop 2>/dev/null || true
doli-node devnet clean 2>/dev/null || true
doli-node devnet init --nodes 3
doli-node devnet start
sleep 250  # Genesis

# Simulate: apply a "test update" to node 0
# Then crash it 3 times within 60s crash window
echo "Simulating 3 crashes within crash window (60s)..."
for i in 1 2 3; do
  echo "Crash $i/3..."
  kill -9 $(pgrep -f "node0.*doli-node") 2>/dev/null || true
  sleep 5
  # Restart node 0 (watchdog should detect crash on startup)
  doli-node --network devnet --data-dir ~/.doli/devnet/data/node0 run \
    --producer --producer-key ~/.doli/devnet/keys/producer_0.json \
    --p2p-port 50303 --rpc-port 28545 \
    --chainspec ~/.doli/devnet/chainspec.json --yes \
    > ~/.doli/devnet/logs/node0.log 2>&1 &
  sleep 10
done

# Verify rollback was triggered
if grep -q "rollback\|Rollback\|ROLLBACK" ~/.doli/devnet/logs/node0.log; then
  echo "TEST PASSED: Watchdog triggered rollback after 3 crashes"
else
  echo "TEST FAILED: No rollback detected"
  exit 1
fi
```

**`scripts/test_hard_fork.sh`:**
```bash
#!/bin/bash
set -e

# Start fresh devnet
doli-node devnet stop 2>/dev/null || true
doli-node devnet clean 2>/dev/null || true
doli-node devnet init --nodes 5
doli-node devnet start
sleep 250  # Genesis

CURRENT_HEIGHT=$(curl -s http://127.0.0.1:28545 -X POST \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq -r '.result.bestHeight')
FORK_HEIGHT=$((CURRENT_HEIGHT + 30))  # ~5 minutes from now

echo "Publishing hard fork release at height $FORK_HEIGHT..."
# Publish release with hard_fork.activation_height = $FORK_HEIGHT

# Update nodes 0-3 (leave node 4 on old version)
echo "Updating nodes 0-3..."
# Apply update to 4 nodes

echo "Waiting for activation height $FORK_HEIGHT..."
while true; do
  H=$(curl -s http://127.0.0.1:28545 -X POST \
    -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq -r '.result.bestHeight')
  echo "Height: $H / $FORK_HEIGHT"
  if [ "$H" -ge "$FORK_HEIGHT" ]; then break; fi
  sleep 10
done

# Verify: nodes 0-3 still producing, node 4 stopped
for i in $(seq 0 3); do
  if grep -q "Produced block.*height.*$FORK_HEIGHT" ~/.doli/devnet/logs/node$i.log; then
    echo "Node $i: PRODUCING (correct)"
  fi
done

if grep -q "Hard fork.*stopping\|version.*required" ~/.doli/devnet/logs/node4.log; then
  echo "Node 4: STOPPED (correct - old version)"
  echo "TEST PASSED: Hard fork activation works"
else
  echo "TEST FAILED: Node 4 should have stopped"
  exit 1
fi
```

### Test Weight Verification Script

```bash
#!/bin/bash
# test_vote_weights.sh — Verify bonds × seniority weighting on real devnet

set -e

# Start devnet with 5 genesis producers (1 bond each)
doli-node devnet stop 2>/dev/null || true
doli-node devnet clean 2>/dev/null || true
doli-node devnet init --nodes 5
doli-node devnet start
sleep 250  # Genesis

# Create whale producer (10 bonds)
doli -w ~/.doli/devnet/keys/whale.json new
WHALE_PH=$(doli -w ~/.doli/devnet/keys/whale.json info | grep "Pubkey Hash" | sed 's/.*: //')
doli -r http://127.0.0.1:28545 -w ~/.doli/devnet/keys/producer_0.json send "$WHALE_PH" 11
sleep 12
doli -r http://127.0.0.1:28545 -w ~/.doli/devnet/keys/whale.json producer register -b 10

# Wait for activation
sleep 110

echo "=== Vote Weight Comparison ==="
echo "Genesis producers (1 bond, ~25 blocks active):"
echo "  seniority_mult = 1.0 + min(25/144, 4) × 0.75 ≈ 1.13"
echo "  weight = 1 × 1.13 = 1.13"
echo ""
echo "Whale producer (10 bonds, ~1 block active):"
echo "  seniority_mult = 1.0 + min(1/144, 4) × 0.75 ≈ 1.005"
echo "  weight = 10 × 1.005 = 10.05"
echo ""
echo "Expected: whale has ~9x the voting power of each genesis producer"
echo "Combined 5 genesis = 5.65, whale = 10.05 → whale has ~64% of total weight"
echo "This means whale ALONE can veto (>40%) — correct economic incentive"

# Publish test release and have whale veto
# Verify: whale's veto alone exceeds 40% threshold
```

---

## Parameters Reference (verified against `network_params.rs`)

| Parameter | Mainnet | Testnet | Devnet | Devnet Real Time |
|-----------|---------|---------|--------|------------------|
| Slot duration | 10s | 10s | 10s | — |
| Blocks per year | 3,153,600 | 3,153,600 | 144 | ~24 minutes |
| Veto period | 7 days | 7 days | 60s | 1 minute |
| Grace period | 48 hours | 48 hours | 30s | 30 seconds |
| Min voting age | 30 days | 30 days | 60s | 1 minute |
| Check interval | 6 hours | 6 hours | 10s | 10 seconds |
| Crash threshold | 3 | 3 | 3 | — |
| Crash window | 1 hour | 1 hour | 60s | 1 minute |
| Seniority step | 3.15M blocks | 3.15M blocks | 144 blocks | ~24 minutes |
| Seniority maturity | 12.6M blocks | 12.6M blocks | 576 blocks | ~96 minutes |
| Veto threshold | 40% | 40% | 40% | — |
| Maintainer threshold | 3/5 | 3/5 | 3/5 | — |
| **Full lifecycle** | **~7.5 days** | **~7.5 days** | **~2.5 min** | **veto + grace** |

### Environment Variable Overrides (devnet only)

All devnet params can be overridden via environment variables for faster testing:
```bash
DOLI_VETO_PERIOD_SECS=30        # Cut veto to 30s
DOLI_GRACE_PERIOD_SECS=15       # Cut grace to 15s
DOLI_MIN_VOTING_AGE_SECS=10     # Vote almost immediately
DOLI_CRASH_WINDOW_SECS=30       # Tighter crash detection
DOLI_UPDATE_CHECK_INTERVAL_SECS=5  # Check every 5s
```

## Anti-Patterns

- ❌ Seniority-only vote weight (ignores economic stake)
- ❌ Bond-only vote weight (enables plutocracy, ignores commitment)
- ❌ Hardcoded maintainer keys (must derive from chain)
- ❌ In-memory-only vote state (lost on restart)
- ❌ Manual rollback as the only recovery (must auto-detect crashes)
- ❌ Hard forks without version gating (old nodes fork the chain)
- ❌ Placeholder RPC responses (users need real data)

## Pro-Patterns

- ✅ `bonds × seniority_multiplier` — balances stake and commitment
- ✅ Deterministic maintainer derivation from genesis (any node can verify)
- ✅ Persistent vote state (survives restart)
- ✅ Watchdog with automatic rollback (no operator needed)
- ✅ Hard fork activation by height (deterministic across all nodes)
- ✅ Network-parameterized timing (fast devnet, production mainnet)
- ✅ All operations scale to 150K producers

## Testing Checklist

### Unit Tests (must pass before E2E)
- [ ] `cargo test -p updater` — all pass
- [ ] `cargo test -p doli-core` — maintainer tests pass
- [ ] `cargo clippy` — no warnings
- [ ] Vote weight: 10-bond/0-year = 10.0, 2-bond/4-year = 8.0, 1-bond/0-year = 1.0
- [ ] Veto at 40% weighted triggers rejection
- [ ] Seniority multiplier caps at 4.0x after seniority_maturity_blocks

### E2E Tests (real devnet, ~10 minutes total)
- [ ] `scripts/test_update_veto.sh` — 3/5 veto rejects update (real nodes, 60s veto period)
- [ ] `scripts/test_update_approve.sh` — 1/5 veto approves update, enforcement activates after 30s grace
- [ ] `scripts/test_vote_weights.sh` — whale (10 bonds) outweighs 5 genesis producers combined
- [ ] `scripts/test_rollback.sh` — 3 crashes in 60s triggers automatic rollback (no manual intervention)
- [ ] `scripts/test_hard_fork.sh` — old-version node stops at activation height
- [ ] Votes persist across node restart (kill node, restart, votes still counted)
- [ ] CLI commands (`doli update status`, `doli update vote`, `doli maintainer list`) work against running devnet
- [ ] All tests complete in < 15 minutes total on real devnet

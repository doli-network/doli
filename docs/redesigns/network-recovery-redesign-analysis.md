# Network Recovery Redesign — Requirements Analysis

> Analyst output for omega-redesign. Date: 2026-04-21.

## 1. Problem Statement

DOLI has 6 individual recovery tools (fork-monitor, emergency-halt, createCheckpoint, seed backup restore, node-heal, snap sync) but **no anti-poisoning mechanism**. The single most dangerous moment in recovery is when freshly restored seeds rejoin the P2P network — non-recovered nodes immediately gossip fork blocks to them, silently undoing the restore.

**Pattern from 10+ incidents:** Every fork cascade recovery follows the same painful manual sequence of ~20 SSH commands across 5+ servers, with the critical risk that a single still-running forked node can poison every freshly restored seed.

## 2. Current Recovery Procedure (As-Is)

### Manual 8-Phase Hostile Recovery
1. **Detect**: fork-monitor.sh alerts (or operator notices)
2. **Halt**: SSH to each server, run emergency-halt.sh (calls pauseProduction on each node)
3. **Identify**: SSH to each seed, check getGuardianStatus → find last healthy checkpoint
4. **Select**: Manually compare checkpoint heights across 3 seeds, pick the best one
5. **Stop seeds**: systemctl stop on all 3 seed servers
6. **Restore**: cp -r checkpoint → data directory on each seed
7. **Restart seeds**: systemctl start — **DANGER ZONE: non-recovered producers gossip fork blocks**
8. **Recover producers**: One by one, stop → wipe → restart → snap sync from seeds

**Pain points:**
- ~20 SSH commands across 5 servers
- Phase 7 is the critical vulnerability — no protection against poisoning
- No way to identify "best" checkpoint programmatically
- No verification that all seeds converged on the same state after restore
- No coordination between seed restores (done one-at-a-time)

## 3. Anti-Poisoning Gap (Core Architectural Weakness)

**Today's protection:** Procedural only — "stop all producers before restoring seeds."

**What's missing:** When a seed restarts after checkpoint restore, it accepts P2P connections and gossip from ANY peer. If N3 is still running on the fork chain, it gossips fork blocks to seed1 immediately. The seed has no way to:
- Reject blocks from non-recovered nodes
- Distinguish "healthy" from "forked" peers
- Enter a quarantine state during recovery

**Simplest fix identified:** A `recovery_mode` flag on the Node struct. When set via RPC, the node drops all inbound blocks (gossip and sync) while remaining available for RPC queries and snap-sync serving. This is LOCAL node policy — zero consensus/protocol changes.

## 4. Capability Inventory

### Detection
| Tool | Type | What it does |
|------|------|-------------|
| fork-monitor.sh | Script | Polls all nodes, detects fork/offline/behind, Telegram alerts |
| getGuardianStatus | RPC | Returns production state, last checkpoint, last healthy checkpoint |

### Containment
| Tool | Type | What it does |
|------|------|-------------|
| pauseProduction | RPC | Stops block production (node still gossips) |
| resumeProduction | RPC | Resumes block production |
| emergency-halt.sh | Script | Calls pauseProduction on all configured nodes |
| emergency-resume.sh | Script | Calls resumeProduction on all configured nodes |

### Backup
| Tool | Type | What it does |
|------|------|-------------|
| createCheckpoint | RPC | Manual RocksDB checkpoint |
| --auto-checkpoint N | Flag | Auto-checkpoint every N blocks, keeps last 5, with health.json |

### Recovery
| Tool | Type | What it does |
|------|------|-------------|
| Manual cp -r | Procedure | Copy checkpoint to data dir |
| node-heal.sh | Script (branch) | rsync producer from healthy source, preserves signed_slots.db |
| Snap sync | Protocol | Auto-sync from peers when starting with empty data |
| UTXO self-heal | Code (branch) | Detects utxo_store/state_db mismatch on startup, auto-repairs |

### Missing
| Gap | Impact |
|-----|--------|
| **Anti-poisoning** | Freshly restored seeds accept fork blocks from non-recovered nodes |
| **Recovery mode** | No way to quarantine a seed during restore |
| **Checkpoint selection** | No programmatic way to find "best" checkpoint across seeds |
| **State verification** | No way to verify all seeds converged post-restore |

## 5. Requirements

### MUST (Blocks deployment)

| ID | Requirement | Acceptance Criteria |
|----|-------------|-------------------|
| M1 | **Recovery Mode RPC** — `enterRecoveryMode` and `exitRecoveryMode` RPC methods on the Node | Calling enterRecoveryMode causes the node to DROP all inbound blocks (gossip + sync). Node remains available for RPC and snap-sync serving. exitRecoveryMode resumes normal block acceptance. |
| M2 | **Block rejection in recovery mode** — All block intake paths (gossip, sync engine, RPC submit) must check recovery_mode before processing | No block can be applied to the chain while recovery_mode=true. Verified by test: set recovery mode, submit block via gossip → block is silently dropped. |
| M3 | **Non-persistent flag** — recovery_mode is in-memory only (AtomicBool). Node restart clears it. | Prevents permanent lockout. A stuck node can always be recovered by restarting it. |
| M4 | **RPC available in recovery mode** — All read RPCs and snap-sync serving continue working | External producers and other seeds can still query chain state and snap-sync while the node is in recovery mode. |
| M5 | **Seed recovery script** — A single script that orchestrates: SSH to all 3 seeds → stop → restore from best checkpoint → start in recovery mode → verify state convergence → exit recovery mode | Operator runs one command. Script reports success/failure for each seed. All 3 seeds end at the same height+hash. |
| M6 | **Operator runbook** — Step-by-step recovery procedure documented | Includes: when to trigger, what to verify, how to handle partial failures, how to recover producers after seeds are up |
| M7 | **Security: RPC restriction** — enterRecoveryMode/exitRecoveryMode restricted to localhost or admin token | Cannot be called by arbitrary network peers. Seeds bind to 0.0.0.0 — unauthenticated recovery mode toggle would be a DoS vector. |
| M8 | **UTXO self-heal on startup** — Port INC-I-027 fix to main | When utxo_store length mismatches state_db after checkpoint restore, auto-rebuild from state_db. No manual intervention needed. |

### SHOULD (High value, next iteration)

| ID | Requirement | Acceptance Criteria |
|----|-------------|-------------------|
| S1 | **Best checkpoint selection** — Script queries all 3 seeds' getGuardianStatus, picks the highest healthy checkpoint | No manual SSH + comparison needed. Script outputs: "Best checkpoint: seed2 at h=30377, hash=9af2f0f7" |
| S2 | **State convergence verification** — After restore, script queries all 3 seeds and confirms same height+hash | Script fails loudly if any seed diverges after restore |
| S3 | **Producer recovery docs** — Document the producer recovery procedure: stop → wipe data → restart → snap sync | External operators can self-recover without contacting Antonio |
| S4 | **node-heal.sh port to main** — Port the node-heal.sh script from feature branch | Operators can heal individual producers with one command |

### COULD (Nice to have)

| ID | Requirement | Acceptance Criteria |
|----|-------------|-------------------|
| C1 | **Recovery target broadcast** — Seeds in recovery mode expose the recovery height+hash via RPC | External operators can query any seed to discover "what state should I converge to?" |

### WON'T (Explicitly out of scope)

| ID | Reason |
|----|--------|
| W1 | **Automated detection-to-recovery** — Too risky given cascade history. Human decides. |
| W2 | **External producer orchestration** — External operators recover independently via snap sync |

## 6. Architecture Constraints

1. **Seeds are source of truth** — all recovery flows FROM seeds
2. **Manual operation** — operator decides when to trigger recovery
3. **3 physical servers** — recovery script must work over SSH
4. **Preserve slashing protection** — never delete signed_slots.db
5. **UTXO self-heal** — utxo_store auto-rebuilds from state_db on mismatch
6. **Anti-poisoning invariant** — a seed in recovery mode MUST NOT accept any block from any peer
7. **Snap-sync continues** — producers must be able to snap-sync from seeds in recovery mode
8. **No consensus/protocol changes** — recovery mode is local node policy only
9. **Existing RPC security model** — recovery RPCs follow same auth pattern as pauseProduction

## 7. Unverified Assumptions (Architect Must Confirm)

| ID | Assumption | Risk if wrong |
|----|-----------|---------------|
| A1 | `handle_new_block()` is the single gate for all inbound blocks | If blocks enter via another path, recovery mode has a bypass |
| A2 | Snap sync serves snapshots from state_db, not from block application | If snap sync requires block acceptance, recovery mode breaks snap serving |
| A3 | Existing `pauseProduction` RPC uses localhost-only or admin auth | If it doesn't, recovery mode RPCs need a new auth mechanism |
| A4 | `--auto-checkpoint` health.json contains enough info to pick "best" checkpoint | If not, checkpoint selection needs additional metadata |

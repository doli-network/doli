# Domain Diagnosis Report: INC-I-083 Post-Snap Fork-Recovery Deadlock

**INC_ID:** INC-I-083
**RUN_ID:** 345
**Date:** 2026-05-19
**Synthesizer:** blockchain-domain-synthesizer

---

## Problem Profile

- **What happened**: After deploying a 14-commit fix batch (INC-I-078/079/080/081/082 + AH re-pin) to the 18-node local testnet, 5-8 nodes entered a permanent fork-recovery deadlock within ~2 hours of natural operation. The fleet split into two advancing clusters plus multiple frozen nodes stuck in `Syncing:Headers` with `sync_fails` climbing into the 200-360 range.
- **When**: 2026-05-19 20:16 (deploy) through 22:51+ (snapshot). Deadlocks emerged ~21:30-22:00.
- **Affected nodes**: 5-8 frozen (n2, n3, n7, n10, n13, n14; seed and n8/n16 on separate fork). n3/n14 later self-recovered via snap sync.
- **Chain**: DOLI PoS, 10s slots, 18-node local testnet, 14 producers.

---

## Domain Relevance Matrix

| Domain | Relevance | Top Hypothesis | Confidence | Key Finding |
|--------|-----------|----------------|------------|-------------|
| Fork | HIGH | Natural tip race at h=110360 amplified by sparse height-index "header desert" across snap-synced fleet | conf(0.65, measured) | Two producers built valid blocks on h=110359; minority-fork nodes (n3/n10) stranded; 13/18 nodes lack blocks at h=110389 in their height index |
| Connectivity | LOW | Recovery path exhaustion, not peer starvation (full 17-peer mesh on every node) | conf(0.70, measured) | All nodes have 17 transport peers; gossip mesh exists; problem is sync state machine, not connectivity |
| Parameters | HIGH | Three independent gates in `request_genesis_resync()` block snap-sync dispatch: `--no-snap-sync` + non-emergency reason; `SNAP_ATTEMPTS_MAX=3` with no reset; `confirmed_height_floor` | conf(0.70, measured) | Coordinator correctly classifies SnapSync; dispatch refuses it at Gate 4/5/1 for every frozen node |
| Code | HIGH | `classify()` has no escalation path for dead-fork nodes with `last_applied_secs > 60` and `gap < 500`; `signal_stuck_fork()` is dead code; `height_fallback_attempted` is one-shot | conf(0.65, measured) | Rule 1 unreachable (recently_synced=false), Rule 2 unreachable (gap<500, no rollback exhaustion), Rule 3 catches all and returns HeaderFirstSync forever |

---

## Domain Classification

**Primary domain**: CODE (the classify-to-dispatch gap is the structural root cause)
**Presenting domain**: FORK (fleet split into competing chain tips is what the operator sees)
**Contributing domain**: PARAMETERS (three snap-sync gates make the code gap unrecoverable)
**Upstream trigger domain**: FORK (natural tip race at h=110360 + sparse height indexes)

---

## Cross-Domain Causal Chain

This incident has a layered cross-domain causal chain. The presenting symptom is a fleet fork (FORK domain), but the fork is a natural PoS event. The inability to RECOVER from the fork is where the real problem lives, and that crosses CODE and PARAMETERS.

```
CROSS-DOMAIN CAUSAL CHAIN:

LAYER 1 — TRIGGER (Fork domain):
  Natural tip race at h=110360. Two producers (2d27fdcc6a24 and b5d98316008d)
  both built valid blocks on parent h=110359. Standard PoS behavior. n3/n10
  chose the minority branch. Seed-cluster (seed/n8/n16) landed on a third fork
  variant at h=110384+.
  
  Amplified by: "Header desert" — 13/18 nodes have sparse height indexes
  (snap-synced nodes lack blocks below their snap horizon). GetHeadersByHeight
  returns empty for the heights frozen nodes need.

LAYER 2 — STRUCTURAL DEFECT (Code domain):
  classify() in recovery.rs has a coverage hole:
  - Rule 1 (ShallowRollback): UNREACHABLE — requires recently_synced()
    (last_applied < 60s). Dead-fork nodes fail this after 60s.
  - Rule 2 (SnapSync): UNREACHABLE — requires rollback_exhausted (never
    triggered because Rule 1 never fires), OR large_gap >= 500 (gaps are
    25-55), OR deep_fork_confirmed (counter oscillation from reset_empty_headers
    prevents consistent accumulation).
  - Rule 3 (HeaderFirstSync): ALWAYS matches — catches all dead-fork nodes
    and returns HeaderFirstSync forever.
  - signal_stuck_fork() fires but has ZERO production callers — dead signal.
  - height_fallback_attempted is one-shot — once chain-break occurs, never
    retries height-based headers.

  Result: classify() returns HeaderFirstSync on every tick. The node is locked
  in a loop that cannot succeed (hash-based GetHeaders returns empty because
  peers don't have the forked tip; height-based returns chain-break because
  canonical prev_hash differs from forked local_hash).

LAYER 3 — RECOVERY BLOCKADE (Parameters domain):
  Even when deep_fork_confirmed DOES eventually become true (some nodes reach
  it after 300s+ via the STALE_TIP_SECS path), the snap-sync dispatch is
  blocked by three independent gates:

  Gate 4 (production_gate.rs:662): --no-snap-sync + non-emergency reason
    -> Blocks n9, n10, n11, n12. CoordinatorSnapEscalation is NOT in the
       emergency list. The coordinator's correct classification is overridden.

  Gate 5 (production_gate.rs:681): snap_attempts >= 3, unconditional
    -> Blocks n7. Three attempts consumed during fleet divergence when
       snap-sync quorum was unachievable. Counter NEVER resets when gap < 50.

  Gate 1 (production_gate.rs:622): confirmed_height_floor > 0 + non-emergency
    -> Blocks n13. Prior snap sync set a floor that now prevents re-snapping.

  Result: Every frozen node has its snap-sync recovery path blocked by a
  parameter/configuration gate. The code gap (Layer 2) puts them in
  HeaderFirstSync, and the parameter gates (Layer 3) prevent escalation to
  the only recovery mechanism that works.

DIRECTION OF CAUSATION:
  Fork (trigger, natural)
    -> Code defect (classify() coverage hole traps nodes in HeaderFirstSync)
      -> Parameters (snap-sync gates block the only escape)
        -> PERMANENT DEADLOCK (no remaining code path can recover the node)

PRIMARY DOMAIN: CODE (structural defect in recovery state machine)
PRESENTING DOMAIN: FORK (fleet split is what operator sees)
CONTRIBUTING DOMAIN: PARAMETERS (gates make the code defect unrecoverable)
```

### Cascade Matrix (per blockchain-invariants.md)

| Layer | Finding | Domain | Evidence Quality |
|-------|---------|--------|-----------------|
| **Trigger** | Natural tip race at h=110360 (two valid blocks on same parent) | Fork | measured |
| **Amplifier** | Sparse height-index "header desert" (13/18 nodes return empty for needed heights) + classify() coverage hole traps nodes in HeaderFirstSync | Fork + Code | measured |
| **Consequence** | 5-8 nodes permanently frozen; fleet split into 2-3 competing chains; sync_fails climbing to 360 | Fork + Code + Params | measured |
| **Detection** | Health logs show sync_fails climbing, state="Syncing:Headers", last_applied climbing | -- | measured |
| **Structural** | Recovery state machine has no escalation path for dead-fork nodes with last_applied > 60s and gap < 500 | Code | measured |

### Root Cause Minimality Test

**Candidate trigger**: Natural tip race at h=110360.

**Alternate scenario 1**: Network partition (21:30 n6/n7/n8 partition test) puts nodes on minority chain. Same cascade: node ends up on dead fork, recently_synced expires, classify() returns HeaderFirstSync forever, snap gates block recovery. **Cascade REPRODUCES.** -> Tip race is a MITIGATION, not the root.

**Alternate scenario 2**: Restart stagger (20:16 deploy) causes nodes to sync to different tips. Same cascade: stale tip, recently_synced expires, same code path. **Cascade REPRODUCES.** -> Deploy event is a MITIGATION.

**Alternate scenario 3**: A single producer emits an invalid block that half the fleet accepts. Same cascade: minority fork forms, same code path. **Cascade REPRODUCES.**

**Conclusion**: The natural tip race is NOT the root cause. It is a trigger -- any event that puts a node on a minority fork produces the same cascade. The **root cause** is the classify() coverage hole in recovery.rs: the absence of an escalation path for dead-fork nodes (not recently_synced, gap < 500). If this hole were fixed, ALL of the above triggers would self-resolve via ShallowRollback or SnapSync escalation.

**Secondary root**: The parameter gates (--no-snap-sync override, SNAP_ATTEMPTS_MAX with no reset, confirmed_height_floor with no emergency bypass for CoordinatorSnapEscalation) are independently necessary for the deadlock to be PERMANENT. If classify() correctly escalated to SnapSync AND the dispatch honored it, the deadlock would resolve. Both the code gap and the parameter gates must be present for the cascade.

---

## Convergence Matrix

```
                              Fork    Conn    Params   Code
recently_synced() gap:          -       Y        -       Y    -> 2/4 (Code+Conn)
classify() coverage hole:       -       -        Y       Y    -> 2/4 (Code+Params)
snap-sync gate blockade:        -       Y        Y       Y    -> 3/4 (Conn+Params+Code)
header desert / sparse index:   Y       Y        -       -    -> 2/4 (Fork+Conn)
signal_stuck_fork() dead:       -       -        -       Y    -> 1/4 (Code only)
height_fallback one-shot:       -       -        -       Y    -> 1/4 (Code only)
natural tip race trigger:       Y       Y        Y       Y    -> 4/4 (all agree)
INC-I-081 NOT causal:           -       -        -       Y    -> 1/4 (Code only, but no contradiction from others)
ProducerSet consistent:         Y       -        Y       -    -> 2/4 (Fork+Params)
```

**Key convergence**: 3/4 domains independently identify the snap-sync gate blockade as a critical factor in making the deadlock permanent. The fork, connectivity, and parameters investigators all identify `--no-snap-sync`, `SNAP_ATTEMPTS_MAX=3`, and the emergency-reason list as blocking recovery. The code investigator traces the exact code paths.

**Independence verification**:
- Fork investigator reached the "header desert" finding via RPC block-availability queries across all 18 nodes (measured).
- Connectivity investigator reached snap-sync exhaustion via log analysis of individual node recovery attempts (measured).
- Parameters investigator reached the three-gate finding via source code tracing of production_gate.rs gates (measured).
- Code investigator reached the classify() coverage hole via exhaustive code path enumeration (measured).

These are INDEPENDENT reasoning paths using different evidence sources (RPC data, logs, source code, source code + log correlation). Convergence is genuine.

---

## Contradictions

### Contradiction 1: Connectivity claims LOW relevance but identifies critical --no-snap-sync finding

**Disagreement**: The connectivity investigator rates domain relevance as LOW ("the problem is NOT that nodes cannot communicate") but then provides the critical finding that `--no-snap-sync` on n9/n10/n11/n12 is the structural difference between n3 (recovered) and n10 (permanently stuck). The connectivity investigator calls this "recovery path exhaustion, not connectivity" — but the `--no-snap-sync` finding is arguably the most actionable parameter finding in the entire investigation.

**Resolution**: The connectivity investigator is CORRECT that this is not a connectivity problem (all nodes have 17 peers, full mesh). The `--no-snap-sync` finding is a PARAMETER/CONFIGURATION observation that the connectivity investigator surfaced while ruling out their own domain. The finding belongs to the PARAMETERS domain, not connectivity. The LOW relevance rating for connectivity is accurate. The cross-domain signal was properly flagged by the connectivity investigator ("For Parameters/Tuning Investigator"). **RESOLVED — no contradiction, just cross-domain signal routing.**

### Contradiction 2: Code investigator says empty_count oscillation prevents escalation; Parameters investigator says STALE_TIP_SECS IS reachable

**Disagreement**: Code (H2) says `reset_empty_headers()` creates counter-oscillation preventing `consecutive_empty_headers >= 10`. Parameters (Key Evidence #4) says `STALE_TIP_SECS=300` IS reachable from inside HeaderFirstSync because Rule 2 evaluates `deep_fork_confirmed = (empty_count >= 10 && last_applied_secs >= 300)` — and confirms it fires.

**Resolution**: Both are partially correct. The code investigator's H2 kill test found that some nodes (n14, n7) DO eventually reach 10+ empties via the `dispatch.rs:96` path, despite the oscillation. The code investigator marked H2 as "PARTIALLY KILLED." The parameters investigator confirms the path is reachable. The resolution: for nodes stuck long enough (> 300s), the STALE_TIP_SECS path IS reached, classify() DOES return SnapSync — but then the DISPATCH gates (Gate 4/5/1) block execution. This means the code-level classify() gap is partially bridged by the STALE_TIP_SECS timeout, but the parameter-level dispatch gates create the permanent blockade. **RESOLVED — both correct at different stages of the same pipeline. Classify gap is partially bridged; dispatch gap is not.**

### Contradiction 3: Fork investigator says "some nodes spontaneously recover" vs Code investigator says "NO automatic code path can recover"

**Disagreement**: Fork investigator (Key Evidence #5) documents that n3, n7, n9, n12, n14 recovered between snapshots. Code investigator (Definitive Answer #2) says there is NO automatic code path for recovery.

**Resolution**: The fork investigator notes "recovery appears to depend on which peers the node happens to connect to." The connectivity investigator provides the answer: n3 recovered via SNAP SYNC at 22:01:04 (snap sync was NOT disabled on n3, and it had attempts remaining). n3's twin n10 had `--no-snap-sync` and remained stuck. The "spontaneous" recoveries are nodes where snap sync was available and succeeded. The code investigator's statement is correct for nodes where all recovery paths are exhausted. **RESOLVED — the recoveries are snap-sync successes on nodes that still had snap-sync available; the permanently stuck nodes are those where snap-sync is blocked by configuration or exhaustion.**

No UNRESOLVED contradictions remain.

---

## Root Cause

The root cause of INC-I-083 is a **structural coverage hole in the recovery state machine's `classify()` function** (recovery.rs:252-363) combined with **three independent snap-sync dispatch gates** (production_gate.rs:614-688) that block the only viable recovery mechanism.

When a node ends up on a minority fork (a routine PoS event), the following cascade occurs:

1. The node's local tip hash is unrecognized by canonical-chain peers (headers return empty or chain-break).
2. Within 60 seconds, `recently_synced()` becomes false, permanently disabling Rule 1 (ShallowRollback) in `classify()`.
3. Rule 2 (SnapSync) is unreachable for moderate forks (gap < 500, no prior rollback attempts to exhaust).
4. Rule 3 (HeaderFirstSync) matches and is returned every tick — a loop that cannot succeed because both hash-based and height-based header requests fail on a dead fork.
5. For nodes that do eventually reach SnapSync classification (via STALE_TIP_SECS after 300s+), the dispatch is blocked by: (a) `--no-snap-sync` flag with `CoordinatorSnapEscalation` not listed as emergency reason, (b) `SNAP_ATTEMPTS_MAX=3` with no reset mechanism, or (c) `confirmed_height_floor` blocking non-emergency snaps.
6. `signal_stuck_fork()` fires at the right moment but is dead code — no production consumer reads the flag.

The result is a permanent deadlock with no automatic recovery path. The node remains in `Syncing:Headers` with `sync_fails` climbing indefinitely until manual intervention (data wipe + re-snap).

This is a **code-level structural defect** (no escalation path for dead-fork + not-recently-synced + moderate-gap nodes) made **irrecoverable by parameter/configuration choices** (snap-sync gates). The fork trigger is incidental — any event creating a minority fork reproduces the cascade.

---

## Causal Chain

| # | Item | Domain | Derived? | Derivation |
|---|------|--------|----------|------------|
| 1 | Natural tip race at h=110360 — two valid blocks on same parent (h=110359) | Fork | NO (measured) | RPC queries confirmed two producers built on h=110359; standard PoS behavior |
| 2 | n3/n10 accept minority fork branch; seed-cluster lands on third variant | Fork | YES | Minority fork chosen based on arrival order; seed snap-synced onto fork at 21:44 |
| 3 | Sparse height indexes — 13/18 nodes return empty for GetHeadersByHeight at needed heights | Fork | YES (measured) | Snap-synced nodes lack blocks below snap horizon; handler breaks on first missing height |
| 4 | Hash-based GetHeaders returns empty (peers don't have forked tip hash) | Code | YES (measured) | Forked tip abandoned by majority; no peer has it in canonical chain |
| 5 | Height-based GetHeadersByHeight returns chain-break (canonical prev_hash != forked local_hash) | Code | YES (measured) | n10 log: `Chain break: prev_hash=c9ea87806bec expected=0b2750dcb31e valid_so_far=0` |
| 6 | `last_applied_secs` exceeds 60s; `recently_synced()` returns false | Code | YES (measured) | No blocks applied on dead fork; n7 log shows last_applied=1351s |
| 7 | Rule 1 (ShallowRollback) in classify() becomes UNREACHABLE | Code | YES (measured) | recovery.rs:304 requires recently_synced(); frozen nodes fail this |
| 8 | Rule 3 (HeaderFirstSync) returned every tick | Code | YES (measured) | n7 log: `action=HeaderFirstSync gap=44 last_applied=1351s` repeating |
| 9 | `reset_empty_headers()` oscillates counter, slowing escalation to Rule 2 | Code | YES (measured) | periodic.rs:624 resets counter each tick |
| 10 | After 300s+, some nodes reach SnapSync via STALE_TIP_SECS path in Rule 2 | Code+Params | YES (measured) | n10 log: `[COORDINATOR] action=SnapSync gap=55 last_applied=1471s` |
| 11 | Gate 4 blocks --no-snap-sync nodes (CoordinatorSnapEscalation not emergency) | Params | YES (measured) | n10 log: `REFUSED: snap sync disabled (reason: CoordinatorSnapEscalation)` |
| 12 | Gate 5 blocks snap-exhausted nodes (3/3 attempts, no reset when gap < 50) | Params | YES (measured) | n7 log: `REFUSED: snap attempts exhausted (3/3)` |
| 13 | Gate 1 blocks floor-set nodes (confirmed_height_floor > 0, non-emergency) | Params | YES (measured) | n13 log: `REFUSED: confirmed_height_floor=101100` |
| 14 | `signal_stuck_fork()` fires but has zero production consumers | Code | YES (measured) | grep confirms no caller of `take_stuck_fork_signal()` in bins/node/src |
| 15 | Node remains in HeaderFirstSync loop permanently — no remaining recovery path | Code+Params | YES (measured) | All frozen nodes: state="Syncing:Headers", sync_fails climbing, zero height advance |

---

## Routing Recommendation

### PRIMARY DOMAIN ROUTING

```
Domain:          CODE (recovery state machine)
Route to:        /omega-doctor (code bug pipeline) OR /omega-swarm --fix
Reasoning:       The structural defect is in classify() — it needs an escalation
                 path for dead-fork nodes. Three specific code fixes needed:
                 (1) Add SnapSync escalation for nodes with last_applied > STALE_TIP_SECS
                     and gap > 0, even without recently_synced or rollback_exhaustion
                 (2) Wire signal_stuck_fork() to a production consumer that triggers
                     rollback or snap escalation
                 (3) Make height_fallback_attempted resettable (not one-shot)
Cross-domain:    After code fix, PARAMETERS domain needs:
                 (a) Add CoordinatorSnapEscalation to the emergency reason list
                     (production_gate.rs:614-619) — the coordinator's classification
                     should override --no-snap-sync when the node is provably stuck
                 (b) Add snap_attempts reset mechanism when gap drops below threshold
                     then re-grows (cleanup.rs:475-494)
                 (c) Consider lowering SNAP_ATTEMPTS_MAX reset threshold from 50 to 20
                     or adding time-based reset (e.g., reset after 10 minutes of stuck)
```

### Fix Order

1. **CODE first**: Fix classify() escalation path. This is the structural defect. Without it, parameter changes alone cannot resolve the deadlock (they only unblock snap dispatch; the classification must first reach SnapSync).
2. **PARAMETERS second**: Add CoordinatorSnapEscalation to emergency list; add snap_attempts reset. These make the code fix effective for all node configurations.
3. **FORK (no fix needed)**: The natural tip race is routine PoS behavior. The header desert (sparse height indexes) is a consequence of snap sync, not fixable without changing the snap-sync block retention policy. After code+parameter fixes, nodes will self-recover from minority forks via snap sync escalation.
4. **CONNECTIVITY (no fix needed)**: Full mesh connectivity is working. The gossipsub `mesh_n_low=20 > 17` is a cosmetic mismatch for this fleet size but not causal.
5. **OPERATIONAL**: Remove `--no-snap-sync` from n9-n12 launchd plists, or ensure the code fix makes snap override work for CoordinatorSnapEscalation. Restart frozen nodes after code fix deployment.

### Recommended Command Sequence

```
/omega-doctor --incident INC-I-083    # Code fix: classify() escalation + dead signal wiring
/omega-p2p --fix                      # Parameter fix: emergency list + snap reset
```

---

## Quality Gate

```
DOMAIN SYNTHESIS QUALITY GATE
Domain reports completed:           4/4
Domain relevance distribution:      [Fork:HIGH, Conn:LOW, Params:HIGH, Code:HIGH]
Primary domain:                     CODE (recovery state machine classify() coverage hole)
Presenting domain:                  FORK (fleet split into competing chain tips)
Cross-domain causation:             YES (Fork -> Code -> Parameters -> permanent deadlock)
Convergence on root cause:          3/4 domains (Code + Params + Conn all identify snap-sync
                                    gate blockade as the persistence mechanism; Code + Params
                                    identify classify() gap as the structural defect)
Evidence independence:              VERIFIED (Fork: RPC queries; Conn: log analysis; Params:
                                    source code tracing; Code: exhaustive path enumeration —
                                    four distinct evidence sources)
Contradictions found:               3
Contradictions resolved:            3/3
COMPROMISED flag:                   NO (all reports >= 127 lines with required sections)
Routing recommendation:             /omega-doctor --incident INC-I-083 (code first),
                                    then /omega-p2p --fix (parameters second)
```

# Security Audit Report: Business Logic & State Integrity (INC-I-139 M6)

## Attack Perspective
Business-logic / state-integrity of the M6 sync-admission refactor. Central question:
can an attacker manipulate order/timing/peer-controllable values to reach an invalid
snap-admission state or a permanent wedge, and is the "bit-for-bit inert" claim on
the RC-2 `threshold 10→50` change actually true across ALL reachable admission inputs?
Scope: `decision.rs`, `dispatch.rs`, `production_gate.rs`, `block_lifecycle.rs`,
`cleanup.rs` under `crates/network/src/sync/manager/`.

## What I Don't Understand
1. The G3 stuck-fork → coordinator Rule 1b → ShallowRollback recovery for the
   minor-fork regime (gap 4..49) lives outside the M6 diff (`cleanup.rs:637`,
   `recovery.rs`); I verified it is *reached* but did not re-audit that it *completes*.
   Not an M6 regression regardless (path unchanged by this diff).
2. `is_deep_fork_detected()` is `pub fn` — I confirmed no in-repo live caller, but a
   downstream/external consumer (or a future re-wire) is outside what I can see.

## Attack Surface Map
| Entry Point | Data Source | Trust Level | Flows To | Dangerous Operation |
|-------------|------------|-------------|----------|---------------------|
| peer `best_height` | lying/Sybil peer | untrusted | `gap = best_height.saturating_sub(local_height)` | snap admission (state wipe) |
| empty header responses | withholding peer | untrusted | `consecutive_empty_headers` → dispatch.rs:102 funnel | `request_genesis_resync` |
| peer count | Sybil | untrusted | `peers.len() >= 3` guard | snap-retry reset / waits |
| `--no-snap-sync` | operator | trusted | `disable_snap_sync()` → threshold=u64::MAX | Gate 4 |

## Findings

No P0/P1/P2 findings. The refactor moves admission in the tightening (safer)
direction and does not open a new attacker-reachable state-wipe path.

### SEC-LOGIC-001: RC-2 `threshold 10→50` — "bit-for-bit inert" claim VERIFIED — P3 (informational) — conf(0.7, observed)
- **Location:** `block_lifecycle.rs:506-509` (`enable_snap_sync`), `production_gate.rs:750`
- **Vulnerability Class:** N/A (claim-verification; would be CWE-670 if false)
- **Data Flow:** `request_genesis_resync(emergency) → enable_snap_sync() → snap.threshold=50`
- **Evidence:** Exhaustive read-site enumeration of `snap.threshold` in non-test code
  yields exactly THREE reads: (a) `decision.rs:163` `snap_allowed = self.snap.threshold < u64::MAX`
  (sentinel only — 10 and 50 both `< u64::MAX`); (b) `block_lifecycle.rs:261` inside a
  `warn!` format string (no control effect); (c) `dispatch.rs:262` is a comment. All four
  numeric gap comparators were re-homed to `SNAP_SYNC_GAP_MIN` and no longer read the
  value. Gate 5 reads `snap.attempts`, not `threshold`. Snap state is in-memory only
  (types.rs has no serialization of `snap`), so no 10-vs-50 divergence survives restart.
- **False Positive Check:** Searched for (i) any numeric read of `snap.threshold` as a
  floor — none remain; (ii) persistence/serialization — none; (iii) config path setting a
  non-{50,MAX,10} value — none (only knob is `no_snap_sync: bool` → u64::MAX). Claim holds.
- **Impact:** None. 10 and 50 are observationally identical in every reachable path.
- **Remediation:** None required. (Cosmetic: emergency-enable permanently overrides an
  operator's `--no-snap-sync` by leaving threshold enabled — pre-existing, identical
  pre/post-M6, not introduced here.)

### SEC-LOGIC-002: `threshold.min(10)` removal — bit-identical only because no config sets threshold<10 — P3 — conf(0.7, observed)
- **Location:** `dispatch.rs:263` (`min_height = self.local_height + 10`)
- **Vulnerability Class:** CWE-697 (incorrect comparison, latent)
- **Data Flow:** pre: `local_height + snap.threshold.min(10)`; post: `local_height + 10`
- **Evidence:** `snap.threshold` is only ever assigned 50 (default/`enable_snap_sync`),
  `u64::MAX` (`disable_snap_sync`), or formerly 10 (emergency). For every one of those,
  `.min(10) == 10`, so the removal is bit-identical across all *currently* reachable inputs.
- **False Positive Check:** Verified no CLI/config plumbs an arbitrary `snap.threshold`
  (grep of bins/ + config.rs: only `no_snap_sync` bool exists). So divergence is
  unreachable today. Finding is latent, not live.
- **Impact:** None today. If a future config knob ever allows `threshold < 10`, the
  GetStateRoot peer-quality filter would silently narrow. Peer-filter only, not admission.
- **Remediation:** Keep `+10` as an intentional literal (already done); if `threshold`
  ever becomes operator-tunable again, re-audit this decoupling.

### SEC-LOGIC-003: RC-1b re-homing at `is_deep_fork_detected` is inert (dead caller); behavior flips only if re-wired — P3 — conf(0.7, observed)
- **Location:** `production_gate.rs:822`
- **Vulnerability Class:** N/A (dead-path observation)
- **Data Flow:** pre `gap > snap.threshold(50)` → post `gap > SNAP_SYNC_GAP_MIN(500)`
- **Evidence:** Whole-repo search: `is_deep_fork_detected` has NO live caller
  (only defined + referenced in comments/tests). For gap 51-500 with a close peer the
  return value FLIPS false→true post-M6, but with no consumer this is unobservable.
- **False Positive Check:** Confirmed absence of caller in `crates/` and `bins/`.
- **Impact:** None currently. Flagged so a future re-wire is aware the semantics changed.
- **Remediation:** None. If re-activated, confirm the 500 floor is the intended gate.

## Race / Ordering Analysis (Concern #2 — RESOLVED, no wedge)
- `start_sync()` reads `local_height` at the top and never `.await`s; it runs under
  exclusive `&mut self`. Block application (h==0→h==1) cannot interleave. No intra-call
  race between the fresh-node-wait (decision.rs:173) and discv5-grace (decision.rs:204)
  blocks — both see the same `local_height`.
- A node that becomes h==1 mid-lifecycle cleanly EXITS both wait regimes (both are now
  `local_height == 0`-gated, RC-1c). Stale `fresh_node_wait_start` / `discv5_peer_grace_deadline`
  are read only inside `h==0` blocks and are self-healing (elapsed>60s falls through;
  expired deadline clears itself). No inconsistent wait state, no wedge.
- RC-1c (adding `local_height == 0` to the discv5-grace block) strictly REMOVES parking
  for h>0 nodes → they proceed to header-first sync → liveness-improving, not regressing.

## Integer-Edge Analysis (Concern #3 — RESOLVED)
- `gap == 500` boundary: all four sites preserve `>` (verified decision.rs:177,208;
  cleanup.rs:492; production_gate.rs:822). Operator is bit-preserved; only the constant
  moved 50→500. gap==500 → false at every site, both pre and post. No off-by-one.
- `saturating_sub`: every gap uses `best_height.saturating_sub(local_height)`; a peer
  reporting `best_height < local_height` saturates to 0 → no admission. No underflow.
- INV-SYNC-011 boundary parity: dispatch minor-fork guard uses `gap < MINOR_FORK_GAP_MAX(50)`,
  so gap==50 is snap-eligible and gap 4..49 is parked — consistent with the invariant's
  "gaps < 50" wording.

## Liveness / Wedge Analysis (Concern #4 — RESOLVED against INV-SYNC-011)
- A real deep-fork node at gap 51-499 STILL recovers: 10+ empty headers → dispatch.rs:102;
  `gap<=3` no, `gap<50` no → falls through to `request_genesis_resync(GenesisFallbackEmptyHeaders)`
  (an emergency reason → floor-exempt, disable-exempt, `enable_snap_sync()`), sets
  `needs_genesis_resync` → next `start_sync` `should_snap = needs_genesis_resync` → snaps.
  The emergency funnel is gap-independent, so raising the wait/retry floors 50→500 does
  NOT wedge this class. No liveness regression found.
- Gaps < 50 for h>0 remain snap-UNreachable except via corroborated evidence
  (empty-headers ≥10 requires gap≥50; ApplyFailures requires gap>50). INV-SYNC-011 preserved.

## Static Analysis Patterns
| Pattern | Files Matched | Risk | Notes |
|---------|--------------|------|-------|
| `> self.snap.threshold` (gap comparator) | 0 in src (tests only) | clean | fully re-homed (RC-1b complete) |
| `snap.threshold` numeric read | decision.rs:163 (sentinel), block_lifecycle.rs:261 (log) | clean | no floor read remains |
| `self.snap.threshold =` | block_lifecycle.rs:497(MAX),508(50) | clean | only {50, u64::MAX}; emergency 10 removed |
| `is_deep_fork_detected` live callers | 0 | info | dead path (SEC-LOGIC-003) |
| snap-state serialization | 0 | clean | in-memory only; no restart divergence |

## Cross-Perspective Signals
- **Rate-governor (INV-SYNC-009 / injection-DoS auditor):** RC-1c makes h>0 nodes fall
  through to header-first sync instead of parking. Confirm the INC-I-120 outbound
  request governor still bounds that path — not re-verified here (out of lane).
- **Fork-onto-wrong-chain (crypto/consensus auditor):** h==0 snap admission still trusts
  a peer-inflated `gap`; mitigated by `consensus_target_hash()` majority guard
  (decision.rs:236). Pre-existing (INC-I-081), unchanged by M6, but worth confirming the
  majority-hash quorum threshold is Sybil-resistant.

## Gaps
- Did not execute the tests (`tests_inc_i139.rs`); findings are by source trace + full
  read-site enumeration, not runtime measurement (`observed`, not `measured`).
- Did not re-audit the minor-fork ShallowRollback completion (out of M6 diff scope).

## Summary
- P0: 0
- P1: 0
- P2: 0
- P3: 3 (all informational/latent; no live exploit path)

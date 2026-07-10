# Security Audit Report: Configuration / Resource-Exhaustion / DoS Surface — INC-I-139 M6

## Attack Perspective
Configuration, resource-exhaustion, and request-storm (DoS) surface of the M6 sync-admission
refactor. Central question: does the demotion of `snap.threshold` to a sentinel, the four
gap-comparator re-homings (50→`SNAP_SYNC_GAP_MIN`=500), the `h==0` discv5-grace gate, and the
`10→50` emergency-enable value alter **outbound request-issuance frequency, retry cadence, or the
snap-attempt/rate limits** in a way that widens a resource-exhaustion or self-amplified
request-storm surface (INC-I-120 / INV-SYNC-009)? Confirm no default is loosened.

## What I Don't Understand
1. Whether `is_deep_fork_detected()` is intended to remain a live consumer in a future milestone
   — it is currently a `pub fn` with **zero callers** anywhere in `crates/` or `bins/`, so its
   comparator change (production_gate.rs:822) has no runtime effect today. If a later change wires
   it back in, the re-homed threshold becomes live and should be re-audited.
2. The exact discovery cadence of discv5 random walks in production — I bound the discv5-grace
   removal by the fixed 30s grace window, but the real distribution of "peers found per second"
   determines how much sooner an h>0 node actually starts header-first (upper bound ≤30s).

## Attack Surface Map
| Entry Point | Data Source | Trust Level | Flows To | Dangerous Operation |
|-------------|------------|-------------|----------|---------------------|
| peer `best_height` → `gap` | lying/Sybil peers | untrusted | cleanup.rs:492, decision.rs:177/208, production_gate.rs:822 | snap-retry reset / header-first entry (request issuance) |
| `self.peers.len()` | Sybil peer count | untrusted | cleanup.rs:492 `>=3`, decision.rs:162 `>=3` | snap admission gate (unchanged by M6) |
| network startup (`set_discv5_peer_grace(30)`) | local config | trusted | decision.rs:204 discv5-grace park | delays/permits header-first request issuance |
| outbound sync req (all classes) | internal state machine | n/a | command_handling.rs:78 governor chokepoint | GetHeaders/GetBodies rate-governed (untouched by M6) |

## Findings

### SEC-CONFIG-001: h>0 nodes skip discv5 grace and enter governed header-first up to 30s sooner — P3 — conf(0.6, observed)
- **Location:** `crates/network/src/sync/manager/sync_engine/decision.rs:204-208` (RC-1c adds `self.local_height == 0`)
- **Vulnerability Class:** CWE-400 (resource consumption) — evaluated, does NOT materialize.
- **Data Flow:** node restart at h>0 with <3 peers + peer-reported gap → pre-M6 `discv5_peer_grace_deadline` (set once at startup via `startup.rs:287 set_discv5_peer_grace(30)`, height-agnostic) caused an early `return` (park ≤30s) → post-M6 the `h==0` gate excludes h>0 nodes → immediate fall-through to header-first `GetHeaders` issuance.
- **Evidence:** `set_discv5_peer_grace(30)` is called unconditionally at network start regardless of height (`bins/node/src/node/startup.rs:285-288`); the discv5-grace block previously matched any height, now only `local_height == 0`. So a restarting h>0 node no longer parks.
- **False Positive Check:** Searched for whether the earlier header-first entry (a) bypasses the INC-I-120 governor and (b) creates a tight retry loop. (a) DEAD: `GetHeaders` is `is_rate_governed() == true` (`crates/network/src/protocols/sync.rs:197-199`) and funnels through the single governor chokepoint `command_handling.rs:78-99`, which is **untouched by M6** (`git diff` empty on `command_handling.rs`, `rate_limit.rs`, `sync.rs`). (b) DEAD: the churn cadence is set by the 30s stuck-sync timeout (cleanup.rs:269-273) and `idle_behind_retries` (cleanup.rs:512-544), neither changed by M6 — the park removal only shifts the FIRST header-first start earlier by ≤30s; it does not raise per-unit-time request rate.
- **Impact:** An h>0 node issues its first (rate-governed) header request up to 30s earlier than pre-M6. One-time phase shift, not amplification. This is the intended post-DC-1 behavior (an h>0 node never uses snap peers, so parking for them was pointless). No storm surface widening.
- **Remediation:** None required. Behavior is correct and governed. Optionally add a regression test asserting an h>0 node with <3 peers transitions Idle→DownloadingHeaders without parking, and that the transition still respects the governor.

### SEC-CONFIG-002: snap-retry attempt reset can fire while snap is operator-disabled (inert) — P3 — conf(0.6, observed)
- **Location:** `crates/network/src/sync/manager/cleanup.rs:487-507`
- **Vulnerability Class:** CWE-665 (improper initialization) — cosmetic side effect, no exploitable impact.
- **Data Flow:** pre-M6 `gap > self.snap.threshold`; when `--no-snap-sync` sets `threshold=u64::MAX`, `gap > u64::MAX` is never true → attempt counter never reset while disabled. Post-M6 `gap > SNAP_SYNC_GAP_MIN(500)` can be true even when snap is disabled → `snap.attempts = 0` + `snap.blacklisted_peers.clear()` execute.
- **Evidence:** cleanup.rs:492 `if gap > super::recovery::thresholds::SNAP_SYNC_GAP_MIN && self.peers.len() >= 3 { ... self.snap.attempts = 0; self.snap.blacklisted_peers.clear(); }`.
- **False Positive Check:** Searched whether resetting `attempts` while disabled can trigger an actual snap request. DEAD: the reset issues **no** request; the next snap attempt is gated by `snap_allowed = self.snap.threshold < u64::MAX` (decision.rs:163, false when disabled) and by `request_genesis_resync` Gate 4 (`threshold == u64::MAX && !is_emergency → REFUSED`, production_gate.rs:732-739). So the counter reset is unobservable while disabled.
- **Impact:** None exploitable. At most once per 30s a counter is zeroed and a small HashSet cleared. No request issued, no snap started. Not a DoS vector.
- **Remediation:** Optional hardening: keep the pre-M6 disabled-guard behavior by early-returning this block when `snap.threshold == u64::MAX`, to preserve exact parity. Not security-required.

## Static Analysis Patterns
| Pattern | Files Matched | Risk | Notes |
|---------|--------------|------|-------|
| `gap > ...SNAP_SYNC_GAP_MIN` (re-homed comparators) | cleanup.rs:492, decision.rs:177/208, production_gate.rs:822 | P3 | All four raise the trigger bar from old default 50 → 500 (STRICTER, "fewer, not more"). No default loosened. |
| `snap.threshold` reads (numeric vs sentinel) | decision.rs:163 (`< u64::MAX`), production_gate.rs:630/732/740 (`== u64::MAX`), block_lifecycle.rs:261 (log only) | P3 | No remaining numeric gap-floor read. RC-2 `10→50` is bit-for-bit inert: both `< u64::MAX`; only observable diff is one log value at block_lifecycle.rs:261. |
| `min_height = local_height + ...` (peer-quality filter) | dispatch.rs:261 | P3 | `threshold.min(10)` was always 10 (threshold ∈ {50, 10, u64::MAX}, all ≥10) → literal `+10` is bit-identical. GetStateRoot fan-out unchanged. |
| governor chokepoint / rate limiter | command_handling.rs:78, rate_limit.rs, protocols/sync.rs | P0-if-changed | `git diff HEAD` EMPTY — INC-I-120 governor + `is_rate_governed()` classification untouched by M6. INV-SYNC-009 preserved. |
| peer-count gates `self.peers.len() >= 3` | cleanup.rs:492, decision.rs:162, production_gate.rs:821 | P2-if-changed | Unchanged by M6. Sybil peer-count interplay not widened. |

## Sybil / Amplification Assessment
A Sybil peer set controls `best_height` (→ gap) and `peers.len()`. Post-M6:
- To trigger the snap-retry reset (cleanup.rs:492), a Sybil set must now claim `gap > 500` (was
  `> 50`) — **harder** to satisfy, so amplification is strictly reduced, not increased.
- Even when triggered, the reset issues no request; real snap requests still pass
  `request_genesis_resync` Gates 2/3/5 (no-concurrent, `MAX_CONSECUTIVE_RESYNCS = 5`,
  `attempts >= 3`) — all "ALL reasons, NO exceptions" per the RC-2 taxonomy comment
  (production_gate.rs:661-664).
- Header-first `GetHeaders` (the path h>0 nodes enter sooner) is per-peer + global rate-governed
  at the unchanged chokepoint.
Conclusion: no new Sybil-reachable request-amplification surface. All admission thresholds moved
in the stricter direction.

## Cross-Perspective Signals
1. **(Logic/admission auditor)** `production_gate.rs:822 is_deep_fork_detected()` has **zero
   callers** in `crates/` and `bins/` (only comment/definition references). Its comparator change
   is dead today. If ever re-wired, note the semantic shift: gaps in (50, 500] with ≥10 empty
   headers + a close peer would now be classified as deep-fork where pre-M6 they early-returned
   `false`. This is the INV-SYNC-011 "corroborated deep-fork evidence" exception territory — worth
   the logic auditor confirming it stays within the invariant if the function is reactivated.
2. **(Logic/admission auditor)** The `10→50` inertness claim (RC-2) is fully load-bearing on there
   being **no numeric read** of `snap.threshold`. I verified all current reads are sentinel
   (`==`/`< u64::MAX`) or log-only. Any FUTURE code that reintroduces a numeric `snap.threshold`
   comparison would silently break this inertness and re-open the INC-I-139 bare-gap class — worth
   an invariant/lint guard.

## Gaps
- I did not exhaustively trace every downstream state transition after header-first entry for the
  h>0 discv5-grace case under adversarial peer churn; I bounded it structurally via the unchanged
  30s stuck-timeout + `idle_behind_retries` cadence and the untouched governor rather than a live
  multi-node run. A gauntlet scenario (h>0 node restart with 1-2 slow-arriving peers) would raise
  confidence from `observed` to `measured`.
- Timing-based confidence ceiling: I could not run the node to measure actual request rates.

## Summary
- P0: 0 findings
- P1: 0 findings
- P2: 0 findings
- P3: 2 findings (both evaluated to inert/intended; no exploitable DoS surface)

**Verdict:** M6 does not widen the resource-exhaustion or request-storm surface. All four
comparator re-homings raise the trigger bar (50→500, stricter), the discv5 `h==0` gate only makes
h>0 nodes start an already-rate-governed header-first path ≤30s sooner (governor chokepoint
untouched — INV-SYNC-009 intact), the dispatch `+10` filter is bit-identical, and the `10→50`
emergency-enable is bit-for-bit inert (no numeric read of `snap.threshold` remains). No default is
loosened.

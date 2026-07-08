# Requirements: INC-I-138 Empty-Headers Stall + Spurious Snap Sync

## Scope
- `crates/network/src/sync/manager/sync_engine/response.rs` (empty-headers handler)
- `crates/network/src/sync/manager/sync_engine/dispatch.rs` (height-fallback dispatch)
- `crates/network/src/sync/manager/recovery.rs` (RecoveryCoordinator classifier)
- `crates/network/src/sync/manager/cleanup.rs` (stuck-fork detector)
- `bins/node/src/node/periodic.rs` (recovery evidence reporting)
- `bins/node/src/node/validation_checks.rs` (GetHeaders serving path)

## Summary (plain language)

On a fresh 6-node local testnet (v6.23.9), node n5 synced normally to h=36 (an epoch-1 boundary), then stalled for 325 seconds. Its GetHeaders requests consistently returned 0 headers from peers, and the INC-I-012 GetHeadersByHeight fallback never engaged. After 325 seconds, the recovery coordinator escalated to a spurious SnapSync at gap=28 (far below the SNAP_SYNC_GAP_MIN=500 threshold).

Three interacting defects in the empty-headers response handler, the height-based fallback dispatch, and the recovery coordinator combine to create a stall loop that only resolves via an overly-aggressive snap sync.

## Architecture Context

### Module Boundaries
- **response.rs** (SyncManager): handles incoming sync responses, classifies empty headers as fork evidence or gossip timing, reports evidence to RecoveryCoordinator. Depends on: ForkState counters, peer status table. Depended by: cleanup.rs (reads consecutive_empty_headers), periodic.rs (reads consecutive_empty_headers for coordinator evidence reporting).
- **dispatch.rs** (SyncManager): creates outbound sync requests. Reads ForkState flags (use_height_based_headers) and resets consecutive_empty_headers when dispatching height-based requests.
- **recovery.rs** (RecoveryCoordinator): accumulates evidence in a time-windowed VecDeque, classifies aggregate evidence into RecoveryAction. Depends on: RecoveryContext (built by periodic.rs from SyncManager state). Depended by: periodic.rs (executes the returned RecoveryAction).
- **cleanup.rs** (SyncManager): periodic housekeeping, stuck-sync detection. Reads consecutive_empty_headers for the stuck-fork signal (guardrail G3: >300s + >=3 empties).
- **periodic.rs** (Node): orchestrates recovery by reporting evidence to coordinator, calling classify_and_dispatch(), and executing the returned action.
- **validation_checks.rs** (Node): serves inbound GetHeaders/GetHeadersByHeight via hash-to-height and height-to-hash canonical indexes.

### Data Flows Through Affected Area
- Outbound: periodic.rs tick -> SyncManager.next_request() (dispatch.rs) -> NetworkCommand::RequestSync -> peer
- Inbound: peer response -> SyncManager.handle_response() (response.rs) -> handle_headers_response() -> evidence reported to RecoveryCoordinator (recovery.rs)
- Classification: periodic.rs tick -> SyncManager.classify_and_dispatch() -> RecoveryCoordinator.classify() -> RecoveryAction -> periodic.rs executes action
- Serving: peer's on_sync_request() (network_events.rs:300) -> handle_sync_request() (validation_checks.rs:962) -> reads chain_state + block_store canonical index -> SyncResponse

### Architectural Constraints & Invariants
- **INV-SYNC-009**: Every outbound sync request funnels through the governor. Recovery/canonical classes are exempt.
- **INV-FORK-001**: A sustained divergent stall MUST raise a stuck-fork signal wired to a real recovery action.
- **PM-005**: Outbound governor (10/peer + 60/s global). Constants NOT derived from live peer count.
- **PM-006**: Busy-backoff blacklists peer + sets Idle.
- **PM-007**: Recovery escalation ladder; deep_fork_confirmed escalates to SnapSync.
- **PM-009**: Inbound serving limit (24/interval; interval = production timer tick = 1s).

### Blast Radius
- **Direct**: response.rs (empty-headers handler), dispatch.rs (height-fallback path), recovery.rs (Rule 2 deep_fork_confirmed)
- **Indirect**: cleanup.rs (stuck-fork signal depends on consecutive_empty_headers), periodic.rs (evidence reporting reads consecutive_empty_headers), production_gate.rs (can_produce reads sync state set by recovery actions)

### Brittleness Check
- Signals detected: 3/5
  1. Cross-module blast radius: fix touches response.rs evidence reporting + recovery.rs gap guard + dispatch.rs counter reset -- 3 files across 2 modules
  2. Invariant gaps: no invariant prevents the coordinator from accumulating "gossip timing" evidence as fork evidence; no invariant prevents deep_fork_confirmed from triggering SnapSync at gap < SNAP_SYNC_GAP_MIN
  3. Contract absence: no explicit contract between the response handler's gap classification and the coordinator's evidence intake -- the response handler reports evidence BEFORE it classifies the response, and the coordinator has no way to distinguish "gossip timing" evidence from real fork evidence
- Verdict: BRITTLE

## What I Don't Understand
1. Why n5 does not receive h=37 via gossip after applying h=36. Gossip delivery should be unaffected by header-first sync state. The 325-second gossip silence is unexplained by this code trace alone. This may be a separate gossip mesh issue or an epoch-boundary-specific processing delay.
2. Whether the serving peers genuinely have `best_height <= 36` throughout the stall or are returning empty for a different reason. The code trace confirms the serving path is correct -- it returns empty only when `start_height >= best_height` or hash lookup fails. If serving peers are at h=64, GetHeaders(290d4942) MUST return headers.

## Hypotheses Traced to Code

### H1: Serving-side hash lookup fails for epoch-boundary blocks
**DISPROVED.** The serving path (validation_checks.rs:978-1006) uses `block_store.get_height_by_hash(&start_hash)` which looks up CF_HASH_TO_HEIGHT. This index is populated by `set_canonical_chain()` (writes.rs:102-161), called at apply_block/mod.rs:280 after every block application. The index is populated by walking backwards from tip_hash -- once h=36 is applied on a serving peer, its hash is in CF_HASH_TO_HEIGHT permanently. No epoch-boundary special case exists. The serving code returns empty only when (a) hash lookup fails or (b) `start_height >= best_height` (validation_checks.rs:1011). The latter occurs when the serving peer's chain_state.best_height equals start_height -- i.e., the peer has nothing after the requested hash.

### H2: Serving limiter returns empty instead of busy
**DISPROVED.** PM-009 (network_events.rs:308-322) returns `SyncResponse::Error("busy: sync serving limit reached")` when the limit is exceeded, NOT empty headers. The 69 "busy" responses in the evidence are correctly attributed to PM-009. The 152 count=0 responses come from the actual GetHeaders serving code at validation_checks.rs:1030 returning `SyncResponse::Headers(vec![])`. These are genuine empty responses, not suppressed busy responses.

### H3: Requester-side defect -- GetHeadersByHeight fallback never engages + counter-reset cycle
**CONFIRMED.** Three interacting defects:

#### Defect 1: Evidence reported before classification (response.rs:252-274)

The empty-headers handler reports EmptyHeaders evidence to the coordinator at **line 262** (via `self.recovery.report(EmptyHeaders { peer, gap })`), then checks `gap <= 3` at **line 264** and returns early as "gossip timing." The coordinator receives EmptyHeaders evidence for responses that the sync engine correctly classified as benign. Over 325 seconds, 152+ such reports accumulate in the coordinator's evidence window.

```
response.rs:252  self.fork.consecutive_empty_headers += 1;
response.rs:262  self.recovery.report(EmptyHeaders { peer, gap });  // <-- reports BEFORE gap check
response.rs:264  if gap <= 3 && self.local_height > 0 {             // <-- gap check AFTER report
response.rs:273      self.set_state(SyncState::Idle, "small_gap_wait_gossip");
response.rs:274      return;  // exits without reaching fork detection paths
```

The per-peer gap at line 203 (`peer_height.saturating_sub(self.local_height)`) uses cached peer status from the sync peer table. Status updates happen every 30 seconds (periodic.rs:771). During the early stall phase, cached peer heights show ~36 (same as n5), making gap=0 even after peers have actually advanced. The gap <= 3 guard suppresses fork detection for most responses.

#### Defect 2: Counter-reset cycle (response.rs:290-309 + dispatch.rs:83-84)

When gap > 3 eventually appears (after a 30-second status refresh), `consecutive_empty_headers >= 3` triggers the anti-cascade guard (response.rs:290-309). Within the 60-second `recently_synced` window (since h=36 was applied), the guard fires:

```
response.rs:290  let recently_synced = self.network.last_block_applied.elapsed() < Duration::from_secs(60);
response.rs:292  if recently_synced && gap < 50 {
response.rs:302      self.fork.consecutive_empty_headers = 0;    // <-- RESETS counter
response.rs:304      self.fork.use_height_based_headers = true;
```

The dispatch then resets the counter a second time:

```
dispatch.rs:72   if self.fork.use_height_based_headers {
dispatch.rs:83       self.fork.use_height_based_headers = false;
dispatch.rs:84       self.fork.consecutive_empty_headers = 0;     // <-- SECOND reset
```

This creates a cycle: GetHeaders -> empty (gap<=3) -> counter=1,2,... -> gap>3 + counter>=3 + recently_synced -> anti-cascade resets counter=0 + use_height=true -> dispatch resets counter=0 -> GetHeadersByHeight -> (if also empty) -> counter=1 -> back to GetHeaders. The counter never accumulates past 3 because the anti-cascade or dispatch always resets it.

Consequences:
- `cleanup.rs:637` stuck-fork signal requires `consecutive_empty_headers >= 3` -- never fires
- `periodic.rs:681` coordinator evidence reporting requires `empty_headers >= 3` -- never fires via this path
- `dispatch.rs:96` deep fork redirect requires `consecutive_empty_headers >= 10` -- never fires

#### Defect 3: GetHeadersByHeight fallback unreachable (response.rs:218-226)

The INC-I-012 F1 height-based fallback at response.rs:218-226 requires EITHER `post_snap_recovery` (AwaitingCanonicalBlock phase) OR `empty_headers_stuck` (consecutive >= 2 AND snap.attempts >= 3). Node n5 synced normally (not via snap), so:
- `post_snap_recovery` = false (n5 is in Normal recovery phase)
- `snap_exhausted` = false (snap.attempts = 0 < 3)
- `empty_headers_stuck` = false

The fallback never engages. It was designed exclusively for post-snap and snap-exhaustion scenarios.

#### Defect 4: Gap-blind deep_fork_confirmed (recovery.rs:382-389)

The coordinator's Rule 2 triggers SnapSync without a gap guard:

```
recovery.rs:382  let deep_fork_confirmed = deep_fork > 0
recovery.rs:383      || (empty_count >= 10 && ctx.last_applied_secs >= thresholds::STALE_TIP_SECS);
recovery.rs:385  if (rollback_exhausted || large_gap || deep_fork_confirmed)
recovery.rs:386      && ctx.snap_attempts < thresholds::SNAP_ATTEMPTS_MAX
recovery.rs:387      && ctx.peer_count >= thresholds::SNAP_MIN_PEERS
recovery.rs:388  {
recovery.rs:389      return RecoveryAction::SnapSync;
```

`deep_fork_confirmed` is OR-ed with `large_gap` (which DOES check `gap >= SNAP_SYNC_GAP_MIN`) and `rollback_exhausted`. When deep_fork_confirmed fires, the gap is not checked. With 10+ EmptyHeaders evidence from gap=0 "gossip timing" responses (Defect 1) and 300s stale tip, the coordinator triggers SnapSync at gap=28, bypassing SNAP_SYNC_GAP_MIN=500.

## Probable Cause

The stall is caused by a feedback loop between three protection mechanisms:

1. The response handler reports EmptyHeaders evidence to the coordinator BEFORE classifying the response as "gossip timing" (per-peer gap <= 3). False evidence accumulates.
2. The counter-reset cycle (anti-cascade + dispatch) prevents consecutive_empty_headers from reaching thresholds needed for the sync engine's own recovery paths (stuck-fork signal, deep fork redirect).
3. After 300+ seconds, the coordinator's gap-blind deep_fork_confirmed fires SnapSync at gap=28, which is the only exit from the stall -- but it's disproportionate (snap sync for a 28-block gap on a 64-block chain).

The stall persists because the INC-I-012 height-based fallback is structurally unreachable for non-snap-synced nodes, and the sync engine's own fork recovery paths are blocked by the counter-reset cycle.

## Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|-------------------|
| REQ-138-001 | Report EmptyHeaders evidence to the coordinator ONLY after gap classification confirms it is not gossip timing | Must | - [ ] EmptyHeaders evidence NOT reported when per-peer gap <= 3 AND network-wide gap <= 3<br>- [ ] EmptyHeaders evidence IS reported when per-peer gap > 3 OR network-wide gap > 3<br>- [ ] GS-002 gauntlet: no spurious snap-sync on 6-node epoch-boundary stall |
| REQ-138-002 | deep_fork_confirmed must require gap >= a minimum threshold before triggering SnapSync | Must | - [ ] deep_fork_confirmed adds `ctx.gap() >= MINOR_FORK_GAP_MAX` guard (gap >= 50)<br>- [ ] SnapSync NOT triggered at gap=28 on a 64-block chain<br>- [ ] Large-gap snap sync (gap >= 500) still works unchanged |
| REQ-138-003 | The dispatch height-fallback path must NOT reset consecutive_empty_headers when the height-based request was triggered by M-RC12-full empty-response handling | Should | - [ ] consecutive_empty_headers is NOT reset at dispatch.rs:84 when the flag was set by the M-RC12 path (line 344)<br>- [ ] consecutive_empty_headers IS reset when the flag was set by the INC-I-012 F1 post-snap path (line 236) |
| REQ-138-004 | The INC-I-012 F1 height-based fallback should also be reachable for non-snap-synced nodes stuck with empty headers | Could | - [ ] Height-based fallback triggers when consecutive_empty_headers >= 2 regardless of snap.attempts<br>- [ ] Guard: only once per sync cycle (height_fallback_attempted check preserved) |

## Acceptance Criteria (detailed)

### REQ-138-001: Evidence gating
- [ ] Given a GetHeaders response returning empty headers from a peer whose cached height equals local_height (gap=0), when the network-wide gap is also <= 3, then NO EmptyHeaders evidence is reported to the RecoveryCoordinator
- [ ] Given a GetHeaders response returning empty headers from a peer whose cached height equals local_height (gap=0), when the network-wide gap is > 3 (stale peer status), then EmptyHeaders evidence IS reported with the network-wide gap value
- [ ] Given a GetHeaders response returning empty headers with per-peer gap > 3, then EmptyHeaders evidence IS reported as before (no behavioral change for real fork evidence)
- [ ] GS-002 assertion: on a 6-node testnet with fresh genesis, no node triggers snap sync at the epoch-1 boundary (h=36)

### REQ-138-002: Gap guard on deep_fork_confirmed
- [ ] Given 10+ EmptyHeaders evidence AND 300+ seconds stale tip AND gap=28, when the coordinator classifies, then it returns ShallowRollback (gap < 50) or HeaderFirstSync (gap < 500), NOT SnapSync
- [ ] Given 10+ EmptyHeaders evidence AND 300+ seconds stale tip AND gap=600, when the coordinator classifies, then it returns SnapSync (gap >= 500, large_gap path)
- [ ] Given deep_fork > 0 evidence AND gap=100, when the coordinator classifies, then it returns ShallowRollback or HeaderFirstSync, NOT SnapSync (explicit DeepForkSuspected at small gap is also guarded)
- [ ] Existing test `deep_fork_suspected_triggers_snap` updated to use gap >= 500

### REQ-138-003: Selective counter reset
- [ ] Given use_height_based_headers set by M-RC12-full (response.rs:344), when dispatch.rs dispatches the height-based request, then consecutive_empty_headers is NOT reset to 0
- [ ] Given use_height_based_headers set by INC-I-012 F1 post-snap path (response.rs:236), when dispatch.rs dispatches the height-based request, then consecutive_empty_headers IS reset to 0 (preserving existing behavior for post-snap recovery)

### REQ-138-004: Broaden height fallback reachability
- [ ] Given a non-snap-synced node with consecutive_empty_headers >= 2, when GetHeaders returns empty, then the height-based fallback is attempted (regardless of snap.attempts)
- [ ] Given a node that already attempted height-based fallback this cycle, when another empty response arrives, then height-based fallback is NOT re-attempted (height_fallback_attempted guard preserved)

## Failure-Mode Matrix

| Mode | Source | Current Behavior | Fixed Behavior |
|------|--------|-----------------|----------------|
| Small network (N=6), epoch boundary stall | INC-I-138 | Empty headers with gap=0 accumulate as coordinator evidence; counter-reset cycle prevents sync engine recovery; deep_fork_confirmed fires SnapSync at gap=28 | Evidence gated by gap classification (REQ-138-001); deep_fork_confirmed guarded by gap >= 50 (REQ-138-002); no spurious snap sync |
| Fresh genesis, all nodes at same height | INC-I-017 | Empty headers from peers at same height correctly classified as gossip timing (gap <= 3) | Unaffected -- evidence NOT reported when both per-peer AND network-wide gap <= 3 |
| Post-snap sync, local_hash unrecognized | INC-I-012 | INC-I-012 F1 height-based fallback fires; dispatch resets consecutive_empty_headers | F1 path unaffected (post_snap_recovery = true bypasses the gap check); dispatch reset preserved for F1 path (REQ-138-003) |
| Fleet-wide fork, genuine deep fork | INC-I-090/INC-I-120 | 10+ empty headers from peers with gap > 3; deep_fork_confirmed fires SnapSync correctly | Unaffected -- real fork evidence has gap > 3, passes evidence gate |
| Busy storm on small network | INC-I-120/GS-008 | 31% busy rate from PM-009; PM-006 blacklists busy peers | Unaffected -- fix does not touch PM-005/PM-006/PM-009 |
| Stale peer status (gap=0 blind spot) | INC-I-111 | Mass status refresh after 60s stale tip; coordinator StaleTip evidence at gap=0 | Unaffected -- evidence gating uses network-wide gap as fallback when per-peer gap <= 3 |
| Snap-synced node, ECON_EPOCH_OVERFLOW | INC-I-118 | Post-snap UTXO backend conversion; height-based fallback fires | Unaffected -- INC-I-012 F1 path uses post_snap_recovery condition, not affected by evidence gate |
| Scale mismatch (mainnet 30+ producers) | INC-I-137 | Governor constants calibrated for 27+ peers; N=6 causes self-starvation | Partially mitigated -- reduced false evidence prevents premature coordinator escalation, but governor constants remain fixed |

## Impact Analysis

### Existing Code Affected
- `response.rs:262` (evidence report line): moved after gap classification -- Risk: low (behavioral change is the fix)
- `recovery.rs:382-383` (deep_fork_confirmed): added gap guard -- Risk: medium (must not break large-gap SnapSync)
- `dispatch.rs:83-84` (counter reset): conditional on flag origin -- Risk: medium (must preserve INC-I-012 F1 semantics)
- `response.rs:218-226` (INC-I-012 F1): broadened trigger condition -- Risk: low (additive, does not change existing paths)

### What Breaks If This Changes
- **recovery.rs gap guard too restrictive**: If deep_fork_confirmed requires gap >= SNAP_SYNC_GAP_MIN (500), nodes in a genuine deep fork with gap=100 and 10+ empties would NOT escalate to snap sync. Use MINOR_FORK_GAP_MAX (50) instead to keep the ShallowRollback path available for medium gaps.
- **Evidence gating too aggressive**: If EmptyHeaders is never reported for gap <= 3, nodes in a real fork where status is stale (gap appears 0 but is actually > 3) would miss early evidence. Mitigated by using network-wide gap as fallback.

### Regression Risk Areas
- INC-I-012 post-snap recovery path (height-based fallback must still work)
- INC-I-120 stuck-fork rollback path (StuckFork evidence must still accumulate)
- INC-I-090 finality-guarded ShallowRollback (unchanged, but verify tests pass)
- Anti-cascade guard (response.rs:290-309) -- still fires for recently_synced + gap < 50

## Traceability Matrix

| Requirement ID | Priority | Test IDs | Architecture Section | Implementation Module |
|---------------|----------|----------|---------------------|---------------------|
| REQ-138-001 | Must | (filled by test-writer) | response.rs evidence reporting | (filled by developer) |
| REQ-138-002 | Must | (filled by test-writer) | recovery.rs Rule 2 | (filled by developer) |
| REQ-138-003 | Should | (filled by test-writer) | dispatch.rs counter reset | (filled by developer) |
| REQ-138-004 | Could | (filled by test-writer) | response.rs F1 fallback | (filled by developer) |

## Triage Verdict

```
TRIAGE: DEEP
Confidence: 0.90
Rationale: 3+ interacting components (response.rs evidence reporting, recovery.rs
classifier, dispatch.rs counter reset) across 2 crate boundaries (sync_engine,
recovery). Counter-reset cycle is a PM-006/PM-007/M-RC12 interaction bug -- not
a single-line defect. deep_fork_confirmed gap-blindness is architecturally
distinct from the counter-reset cycle. Both require separate fixes with separate
test coverage. Resumed incident (INC-I-138 registered after swarm diagnosis)
adds prior-context complexity.
```

## Assumptions

| # | Assumption (technical) | Explanation (plain language) | Confirmed |
|---|----------------------|---------------------------|-----------|
| 1 | Serving peers return empty because their chain_state.best_height <= start_height at request time | The serving code is correct -- it returns empty when it genuinely has no blocks after the requested hash | Yes (code trace) |
| 2 | Per-peer gap is stale because status updates happen every 30s | On a fresh network, peers advance faster than status refreshes, making cached per-peer heights misleading | Yes (periodic.rs:771) |
| 3 | The 325-second stall is not caused by gossip delivery failure | n5's gossip mesh should still deliver blocks; the gossip silence is unexplained by this code trace | No (needs investigation) |
| 4 | The counter-reset cycle prevents stuck-fork threshold permanently | anti-cascade + dispatch resets keep consecutive_empty_headers below 3 | Yes (code trace) |

## Identified Risks
- **Gossip delivery unknown**: Assumption #3 is unconfirmed. If n5 stops receiving gossip blocks at h=36 (e.g., due to epoch-boundary gossipsub mesh reconfiguration), fixing the sync path alone would not prevent the stall -- gossip would need a separate fix.
- **Test coverage gap**: No existing integration test covers the 6-node epoch-boundary fresh-genesis scenario. GS-002 gauntlet assertion needs to be implemented.

## Out of Scope (Won't)
- Recalibrating PM-009 serving limit for small networks (N=6) -- separate concern, not root cause
- Investigating gossip delivery failure at epoch boundaries -- separate incident if confirmed
- Making governor constants derive from live peer count -- architectural change beyond this fix scope

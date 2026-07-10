# Security Audit Report — INC-I-139 M6 (Sync-Admission Refactor: RC-1 Sentinel Demotion + RC-2 Emergency-Enable Taxonomy)

## Scope
`crates/network/src/sync/manager/` M6 diff (uncommitted working tree vs HEAD): `sync_engine/decision.rs`, `sync_engine/dispatch.rs`, `production_gate.rs`, `block_lifecycle.rs`, `cleanup.rs`. Five independent auditor perspectives: injection, auth, crypto, logic, config. Brief: `docs/.workflow/security-audit-brief-M6.md`.

## GATE VERDICT

**GATE VERDICT: PROCEED — zero M6-INTRODUCED P0 or P1 findings. Maximum M6-introduced severity is P3 (latent/inert, zero runtime effect). The single P2 (AUDIT-P2-001, one-way `--no-snap-sync` latch) is PRE-EXISTING behavior that M6 preserves bit-for-bit (verified via `git log -S "snap.threshold = 10"` → commit 39d500e7) and does not block this gate.**

## Summary
- **P0 (Critical):** 0
- **P1 (High):** 0
- **P2 (Medium):** 1 — PRE-EXISTING (not M6-introduced)
- **P3 (Low):** 4 — all M6-introduced, all latent/inert/intended (zero live exploit path)
- **Speculative (conf <0.7, manual review):** 3 — all PRE-EXISTING
- **Total:** 5 graded + 3 speculative unique findings (10 raw findings across 5 reports, deduplicated)
- **Systemic patterns:** 1
- **Verified-safe claims:** 5 (including the load-bearing "10→50 bit-for-bit inert" claim, 5/5 convergence)

## Findings Table

| ID | Title | Severity | M6-introduced? | Convergence | Evidence (file:line) |
|----|-------|----------|----------------|-------------|----------------------|
| AUDIT-P2-001 | Emergency `enable_snap_sync()` is a one-way latch: first emergency permanently defeats `--no-snap-sync` | P2 | **NO — pre-existing** (pre-M6 `threshold=10` was identically persistent; `git log -S` → 39d500e7) | 3/5 (auth graded; injection, logic independently noted) | `production_gate.rs:740-751`, `block_lifecycle.rs:496-509`, `bins/node/src/node/init.rs:696` |
| AUDIT-P3-001 | `is_deep_fork_detected()` 50→500 comparator re-home is a semantic INVERSION — but sits in repo-wide DEAD CODE (zero callers) | P3 | YES (latent; zero runtime effect) | **4/5** (injection, crypto, logic, config) | `production_gate.rs:787-854` (changed :822) |
| AUDIT-P3-002 | Snap-retry attempt reset now fires while snap is operator-disabled (parity deviation vs pre-M6; inert — no request issued, admission still sentinel-gated) | P3 | YES (inert) | 1/5 (config; synthesizer-verified) | `cleanup.rs:487-507` (changed :492) |
| AUDIT-P3-003 | `threshold.min(10)` → literal `+10` in peer-quality filter: bit-identical today; latent divergence only if threshold ever becomes operator-tunable <10 | P3 | YES (bit-identical for all reachable inputs) | 4/5 (logic graded; config, crypto, auth verified identical) | `dispatch.rs:261-263` |
| AUDIT-P3-004 | RC-1c: h>0 nodes skip discv5 grace, enter header-first ≤30s sooner — intended, fully rate-governed (INC-I-120 chokepoint untouched) | P3 | YES (intended, benign) | 3/5 (injection+logic raised; config resolved DEAD) | `decision.rs:204-208`, governor at `command_handling.rs:78-99` (0 lines in M6 diff) |

## Findings

### P2: Medium

#### AUDIT-P2-001: Emergency `enable_snap_sync()` is a one-way latch — first emergency permanently defeats `--no-snap-sync`
- **Location:** `crates/network/src/sync/manager/production_gate.rs:740-751` (Gate 4 emergency branch), `crates/network/src/sync/manager/block_lifecycle.rs:496-509`
- **Introduced by M6:** **NO — PRE-EXISTING.** Pre-M6 the same Gate-4 branch executed `self.snap.threshold = 10` with the identical "Temporarily enable" comment and no restore (auth auditor confirmed via `git log -S "snap.threshold = 10"` → 39d500e7; synthesizer re-verified the history hit). RC-2 preserves this bit-for-bit. Injection (SEC-INJECTION-002 note) and logic (SEC-LOGIC-001 note) independently reached the same pre-existing classification.
- **Vulnerability Class:** CWE-696 (Incorrect Behavior Order) / privilege-persistence — authorization override that never reverts
- **Data Flow:** operator `--no-snap-sync` (`init.rs:696` → `disable_snap_sync()` → `threshold=u64::MAX`) → attacker withholds headers + inflates `best_height` (gap≥50, `consecutive_empty_headers≥10`, `dispatch.rs:102-151`) → `request_genesis_resync(GenesisFallbackEmptyHeaders)` [emergency] → Gate 4 calls `enable_snap_sync()` → `threshold=50` **with no code path anywhere restoring `u64::MAX`** → thereafter non-emergency forward-large-gap reasons (`CoordinatorSnapEscalation`/`StuckSyncLargeGap`, `production_gate.rs:685-689`) pass Gate 4, and `decision.rs:163 snap_allowed` is true forever.
- **Evidence:** Synthesizer-verified: only writer of `u64::MAX` is `disable_snap_sync()` (`block_lifecycle.rs:497`), whose sole non-test caller is `init.rs:696`; only caller of `enable_snap_sync()` is `production_gate.rs:750`. No restore in `set_post_recovery_grace` (`production_gate.rs:615-623`) or anywhere else. `snap.attempts` resets on every applied block (`block_lifecycle.rs:151,307,371`; `cleanup.rs:503`), so Gate 5 does not durably re-close.
- **Confidence:** conf(0.8, converged) — 3/5 auditors, independent evidence paths (auth: full write/read trace + git archeology; injection: read-site enumeration; logic: read-site enumeration + persistence check)
- **Impact:** A single attacker-inducible emergency (both inputs inside the stated trust boundary) permanently converts a `--no-snap-sync` node into snap-enabled; subsequent non-emergency paths can wipe state the operator forbade. The RC-2 taxonomy comment ("bypass-operator-disable = emergency ONLY") holds per-request but not as a durable operator guarantee.
- **Open FP caveat (auth):** if node-level orchestration force-restarts the process after every genesis resync, config reload re-disables snap and the latch self-heals — not resolvable within the 5-file scope. Downgrades severity if confirmed.
- **Remediation:** Make the emergency enable scoped — snapshot prior `threshold` before `enable_snap_sync()` and restore it in `set_post_recovery_grace()`/on recovery completion, OR gate the emergency snap on a per-recovery boolean instead of a persistent field mutation. Note the M6 doc-comment "symmetric to `disable_snap_sync()`" (`block_lifecycle.rs:500`) is misleading: disable is never called to re-close the door.
- **Test Strategy:** unit test — `disable_snap_sync()`, fire one emergency `request_genesis_resync`, complete recovery, then assert a subsequent non-emergency `CoordinatorSnapEscalation` is REFUSED (currently passes Gate 4 — test fails, proving the latch).

### P3: Low

#### AUDIT-P3-001: `is_deep_fork_detected()` 50→500 re-home is a semantic inversion inside dead code
- **Location:** `crates/network/src/sync/manager/production_gate.rs:787-854` (changed comparator at :822)
- **Introduced by M6:** YES — latent only; **zero runtime effect** (dead code)
- **Vulnerability Class:** CWE-561 (Dead Code) / latent CWE-670 (always-incorrect control flow if re-wired)
- **Data Flow:** peer `best_height` → gap; withheld headers → `consecutive_empty_headers` → `is_deep_fork_detected()` early-exit `gap > SNAP_SYNC_GAP_MIN` (:822) → (no consumer)
- **Evidence:** **4/5 convergence, independently verified:** injection, crypto, logic each ran repo-wide caller searches; config confirmed in cross-signal. Synthesizer re-verified: `grep -rn is_deep_fork_detected crates/ bins/` returns ONLY the definition (`production_gate.rs:787`) and two doc-comments (`block_lifecycle.rs:729`, `production_gate.rs:40`) — zero invocations. Unlike the other three re-homed sites, this comparator's direction INVERTS meaning: `gap > threshold → return "not deep fork"`, so 50→500 WIDENS the deep-fork-eligible window from (12,50] to (12,500] — the exact INC-I-139 recurrence class — if ever re-wired. The live deep-fork emergency path (`cleanup.rs:445`) is untouched by M6.
- **Confidence:** conf(0.9, converged) — 4 independent caller searches + synthesizer verification
- **Impact:** None at runtime. Latent: a future re-wire inherits a 10x-wider floor-bypassing emergency-snap window for header-withholding + Sybil-height attackers, without a review gate.
- **Remediation:** Delete `is_deep_fork_detected()` (unreferenced), or annotate `#[allow(dead_code)]` with an explicit note that the 500 gate widens the deep-fork window and MUST be reconciled with `MINOR_FORK_GAP_MAX(50)`/INV-SYNC-011 before any re-wiring (crypto auditor recommends the 50 short-circuit if retained).
- **Test Strategy:** compile-time: remove `pub`, let `dead_code` lint force the decision; or a source-scan test (pattern of `tests_inc_i139_m6.rs`) asserting no live caller exists.

#### AUDIT-P3-002: Snap-retry attempt reset fires while snap is operator-disabled (parity deviation, inert)
- **Location:** `crates/network/src/sync/manager/cleanup.rs:487-507` (comparator at :492)
- **Introduced by M6:** YES — the one site where M6 is NOT behavior-preserving for `--no-snap-sync` nodes, but the deviation is inert
- **Vulnerability Class:** CWE-665 (improper state handling) — cosmetic
- **Data Flow:** peer-reported gap > 500 + `peers.len() ≥ 3` → `snap.attempts = 0` + `snap.blacklisted_peers.clear()` even when `threshold == u64::MAX` (pre-M6: `gap > u64::MAX` never true → block never executed while disabled)
- **Evidence:** Config auditor's direct code cite; synthesizer verified no `snap.threshold` comparator remains at the site (re-homed to `SNAP_SYNC_GAP_MIN`). Inertness verified: the reset issues no request; next snap attempt still requires `snap_allowed = threshold < u64::MAX` (`decision.rs:163`, false while disabled) and Gate 4 (`production_gate.rs:732-739`) refuses non-emergency while disabled.
- **Confidence:** conf(0.7, observed) — single auditor, raised from 0.6 after synthesizer re-verification of the comparator and both downstream sentinels
- **Impact:** None exploitable — a counter zeroed and a small HashSet cleared at most once per 30s. Flagged because it falsifies a blanket "behavior preserved bit-for-bit" statement at this one site.
- **Remediation:** Optional parity hardening — early-return the block when `snap.threshold == u64::MAX`.
- **Test Strategy:** unit test — disabled node, gap=600, 3 peers → assert `snap.attempts`/blacklist untouched (fails post-M6 without the guard, documents the accepted deviation otherwise).

#### AUDIT-P3-003: `threshold.min(10)` → literal `+10` decoupling — bit-identical today, latent if threshold becomes tunable
- **Location:** `crates/network/src/sync/manager/sync_engine/dispatch.rs:261-263`
- **Introduced by M6:** YES — bit-identical for all reachable inputs
- **Vulnerability Class:** CWE-697 (latent incorrect comparison)
- **Data Flow:** pre: `local_height + snap.threshold.min(10)`; post: `local_height + 10` (GetStateRoot peer-quality filter only — not admission)
- **Evidence:** 4/5 verified identical: threshold ∈ {50, u64::MAX, formerly 10} — `.min(10) == 10` for all (crypto: `50.min(10)==10`; logic: no config plumbs arbitrary threshold, only `no_snap_sync: bool`; config: fan-out unchanged; auth: FP-check). 
- **Confidence:** conf(0.9, converged)
- **Impact:** None today. If a future knob allows `threshold < 10`, the peer-quality filter silently stops narrowing with it.
- **Remediation:** None required; re-audit this decoupling if `snap.threshold` ever becomes operator-tunable again.

#### AUDIT-P3-004: RC-1c — h>0 nodes skip discv5 grace, start rate-governed header-first ≤30s sooner (intended)
- **Location:** `crates/network/src/sync/manager/sync_engine/decision.rs:204-208`; governor chokepoint `command_handling.rs:78-99` (0 lines in M6 diff)
- **Introduced by M6:** YES — intended behavior change, security-benign
- **Vulnerability Class:** CWE-400 (evaluated, does NOT materialize)
- **Data Flow:** h>0 restart with <3 peers → pre-M6 parked ≤30s on `discv5_peer_grace_deadline` (armed height-agnostically at `startup.rs:285-288`) → post-M6 `h==0` gate excludes it → immediate header-first `GetHeaders`
- **Evidence:** Raised as a DoS question by injection and logic (cross-signals); config resolved both branches DEAD: (a) `GetHeaders` is `is_rate_governed()` (`protocols/sync.rs:197-199`) through the untouched INC-I-120 chokepoint; (b) retry cadence (30s stuck-timeout `cleanup.rs:269-273`, `idle_behind_retries` :512-544) unchanged — one-time ≤30s phase shift, no rate amplification. Crypto independently: `should_snap` for h>0 still requires `needs_genesis_resync` → strictly LESS snapping.
- **Confidence:** conf(0.75, converged) — cross-perspective question raised and resolved with cited evidence
- **Impact:** None; closes an h>0 pointless-park stall (liveness improvement). INV-SYNC-009 preserved.
- **Remediation:** None required. Optional regression test: h>0 node, <3 peers → Idle→DownloadingHeaders without parking, governor still applied.

## Verified-Safe Claims (audit passed — not findings)

| # | Claim | Convergence | Confidence |
|---|-------|-------------|-----------|
| V1 | **RC-2 "10→50 bit-for-bit inert": TRUE.** No numeric gap-floor read of `snap.threshold` survives — all non-test reads are sentinel (`decision.rs:163 < u64::MAX`; `production_gate.rs:630/732/740 == u64::MAX`) or log-only (`block_lifecycle.rs:261`). Both 10 and 50 satisfy `< u64::MAX`; only observable diff is one log value. | **5/5** (injection, auth, crypto, logic, config — each ran an independent read-site enumeration) + synthesizer grep re-verification | conf(0.95, converged) |
| V2 | Live snap-admission integrity guards untouched by M6: `consensus_target_hash()` plurality guard (`decision.rs:44-70`, on EVERY snap path), `confirmed_height_floor` Gate 1 (`production_gate.rs:692`), finality guard (`recovery.rs:325-373`, INC-I-081), INC-I-120 rate governor (`command_handling.rs`/`rate_limit.rs`/`sync.rs` — git diff empty) | 2/5 primary (crypto, config), consistent with all | conf(0.85, converged) |
| V3 | `needs_genesis_resync` setter is single-authority (`production_gate.rs:773`), behind Gates 1-5; M6 adds no new setter, loosens no gate | 2/5 (injection, crypto) | conf(0.8, converged) |
| V4 | No race/wedge/off-by-one: `start_sync` never awaits mid-decision; gap==500 boundary preserves `>` at all 4 sites; `saturating_sub` prevents underflow; snap state is in-memory only (no restart divergence) | 1/5 (logic, dedicated analysis) | conf(0.7, observed) |
| V5 | INV-SYNC-011 and INV-SYNC-009 are not weakened by M6; all comparator moves are in the stricter direction (50→500) or wait/retry-only | 4/5 directional agreement | conf(0.85, converged) |

## Speculative Findings (low-confidence, requires manual review — ALL PRE-EXISTING, none block this gate)

#### SPEC-001: `AllPeersBlacklistedDeepFork` emergency snap reachable at gap as low as 13 — possible INV-SYNC-011 tension (PRE-EXISTING)
- **Location:** `cleanup.rs:445-453` (guard `gap > 12`; gap from peer-influenceable `network.network_tip_height`); classified emergency → floor-exempt + disable-exempt
- **Source:** auth cross-signal + crypto attack-surface map; NOT in M6 diff at that line. Emerged from the resolved contradiction below. Whether blacklist-exhaustion constitutes "corroborated deep-fork evidence" under INV-SYNC-011's exception clause is interpretive. conf(0.6, observed) on the path existing; violation claim unresolved. **Recommend a dedicated review of this pre-existing path.**

#### SPEC-002: `consensus_target_hash()` accepts plurality with only `count >= 2` peers, not strict majority — Sybil-resistance question (PRE-EXISTING)
- **Location:** `decision.rs:44-70`; 2/5 signals (crypto, logic). During an active fork with fragmented honest peers, 2+ coordinated Sybils on one forged `(height,hash)` pair could win the plurality. This is the ultimate forged-snapshot guard. conf(0.5, inferred) — depends on peer-table admission model, outside M6 scope.

#### SPEC-003: Gate 5 attempt limiter is soft — `snap.attempts` resets on every applied block and every 30s (PRE-EXISTING)
- **Location:** `block_lifecycle.rs:151/307/371`, `cleanup.rs:503`; 1/5 (auth DoS signal). An intermittently-serving peer could keep re-arming attempts; interacts with AUDIT-P2-001's latch. conf(0.5, inferred).

## Systemic Patterns

### SYS-001: M6's "inert" guarantees are convention-enforced (absence-of-readers), not compiler/lint-enforced
- **Affected findings:** AUDIT-P3-001, AUDIT-P3-002, AUDIT-P3-003, V1
- **Description:** The load-bearing safety property — "no code reads `snap.threshold` as a gap floor" — holds only because no reader currently exists. Any future numeric read silently re-opens the INC-I-139 bare-gap class (config cross-signal #2); the dead `is_deep_fork_detected()` and the decoupled `+10` are two more places where correctness depends on nobody re-wiring/re-plumbing without re-audit. A partial drift-guard already exists (`tests_inc_i139_m6.rs` source-scan asserting no `> self.snap.threshold` substring in decision.rs).
- **Impact:** Latent regression class, zero current exposure.
- **Remediation (systemic):** extend the source-scan drift-guard to ALL manager files (not only decision.rs); delete the dead function; register an invariant record (suggest INV-SYNC-0NN: "snap.threshold is a pure enable/disable sentinel; any numeric comparator read is a regression") linked to the drift-guard test.

## Convergence Analysis

```
                                   Injection  Auth  Crypto  Logic  Config
A: dead-code 50→500 inversion          Y       -      Y       Y      Y(sig)   4/5
B: 10→50 bit-for-bit inert             Y       Y      Y       Y      Y        5/5
C: one-way --no-snap-sync latch        Y(note) Y(P2)  -       Y(note) -       3/5
D: min(10)→+10 bit-identical           -       Y(fp)  Y       Y      Y        4/5
E: RC-1c grace-skip benign/governed    Y(sig)  -      Y       Y      Y(P3)    4/5 (resolved by config)
F: reset-while-disabled deviation      -       -      -       -      Y        1/5 (synth-verified)
```

**Independence checks (all clusters):** each converging auditor performed its own repo-wide grep/read-site enumeration or directional trace within its own lane — no shared intermediate artifact; conclusions cite the same code because the code is the shared subject, not because evidence was copied. True convergence — confidence boosts applied. Cluster B additionally re-verified by the synthesizer (grep of all non-test `snap.threshold` reads: exactly `decision.rs:163`, `production_gate.rs:630/732/740`, `block_lifecycle.rs:261` log-only, writes at `block_lifecycle.rs:497/508`). Cluster A re-verified by synthesizer (zero invocations of `is_deep_fork_detected` repo-wide). Cluster C's pre-existing classification anchored on git archeology (39d500e7), re-verified by synthesizer.

## Contradictions

**1 found, 1 resolved (code fact), residual interpretation escalated to SPEC-001.**

- **Logic vs Auth on sub-50 snap reachability.** Logic's liveness analysis claimed "gaps < 50 for h>0 remain snap-UNreachable except via corroborated evidence (empty-headers ≥10 requires gap≥50; ApplyFailures requires gap>50)". Auth (corroborated by crypto's attack-surface map) cited `cleanup.rs:445-453`: `AllPeersBlacklistedDeepFork` is emergency-classed with only a `gap > 12` guard. **Resolution:** auth's evidence is stronger (specific line-cited guard; crypto independently mapped the same flow); logic's enumeration simply omitted the blacklist-exhaustion reason. The gap>12 emergency path EXISTS. Whether it satisfies INV-SYNC-011's "corroborated deep-fork evidence" exception (all-peers-blacklisted is itself evidence of a kind) is interpretive → SPEC-001, manual review. **Both sides agree the path is PRE-EXISTING and outside the M6 diff — no impact on the gate.**

No other contradictions: all 5 auditors agree on the 10→50 inertness (5/5), the dead-code status of `is_deep_fork_detected` (4/5 checked, 0 dissent), and the pre-existing classification of the latch (3/5 checked, 0 dissent).

## Coverage Gaps
- **Downstream recovery-coordinator wiring** (`periodic.rs`, `node.rs` consumption of `needs_genesis_resync`; whether a post-resync process restart neutralizes the AUDIT-P2-001 latch) — outside the 5-file scope for all auditors; flagged by injection, auth, crypto. Material to P2-001's real-world severity.
- **Runtime measurement:** all findings are `observed` (static trace); none `measured`. All auditors self-capped at conf(0.7) individually; convergence lifts merged confidence. Config recommends a gauntlet scenario (h>0 restart, 1-2 slow peers) to lift AUDIT-P3-004 to measured.
- **Peer-table admission/scoring model** (whether plurality ≈ majority for SPEC-002) — upstream of the diff, uncovered by design.
- No auditor report was suspiciously thin; all 5 delivered full attack-surface maps, FP checks, and gap statements.

## Synthesis Quality Gate

```
SYNTHESIS QUALITY GATE
Auditors completed:           5/5
Total raw findings:           10 (before dedup, incl. verification records)
Total unique findings:        8 (5 graded + 3 speculative) + 5 verified-safe claims
Convergence clusters:         6
Contradictions found:         1
Contradictions resolved:      1/1 (code fact; interpretation escalated to SPEC-001 manual review)
Attack perspectives covered:  injection, auth, crypto, logic, config
Attack perspectives thin:     none
Systemic patterns detected:   1 (SYS-001)
M6-introduced max severity:   P3
Pre-existing max severity:    P2 (AUDIT-P2-001) — does not block gate
GATE:                         PROCEED
```

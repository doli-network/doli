# Security Audit Report: Authorization & Operator-Intent Bypass (INC-I-139 M6)

## Attack Perspective
Whether the M6 sync-admission refactor (RC-1 sentinel demotion of `snap.threshold` +
RC-2 `threshold=10` → `enable_snap_sync()`=50) lets an attacker-controllable input or a
mis-classified `RecoveryReason` perform a history-wiping SnapSync the operator forbade
via `--no-snap-sync`. Focus: the three-capability taxonomy in `request_genesis_resync`
(bypass-floor / bypass-operator-disable / rate-attempt limits) and Gates 1–5.

## What I Don't Understand
1. Whether operators treat `--no-snap-sync` as "never wipe history under any
   circumstance" or as "prefer header-first, but emergency override is acceptable."
   The code comment (production_gate.rs:731) asserts the latter; I found no spec
   statement confirming operator expectation. This determines whether the emergency
   override is a feature or a bypass.
2. Whether any external supervisor restarts the node after recovery (which would reload
   `--no-snap-sync` from config and re-set `threshold=u64::MAX`). If recovery always
   ends in a process restart, the latch in Finding 002 self-heals; I found no such
   restart in the sync-manager code, but it could live in node-level orchestration.

## Attack Surface Map
| Entry Point | Data Source | Trust Level | Flows To | Dangerous Operation |
|-------------|------------|-------------|----------|---------------------|
| Peer `best_height` | lying/Sybil peer | untrusted | `gap` in dispatch.rs:109, cleanup.rs:441 | selects emergency `RecoveryReason` |
| Empty header responses | withholding peer | untrusted | `consecutive_empty_headers` → dispatch.rs:102, cleanup.rs:436 | GenesisFallback / AllPeersBlacklisted emergency |
| Peer count | Sybil peers | untrusted | `enough_peers>=3` decision.rs:162, cleanup.rs:440 | snap quorum eligibility |
| `request_genesis_resync(reason)` | internal enum | trusted classification | Gate 4 (production_gate.rs:732-751) | `enable_snap_sync()` → state wipe |

## Findings

### SEC-AUTH-001: RC-2 10→50 value change is inert for admission — operator-disable NOT weakened by the value change — conf(0.7, observed)
- **Location:** `production_gate.rs:750`, `block_lifecycle.rs:505-509`; all reads at `production_gate.rs:630,732,740`, `decision.rs:163`, `block_lifecycle.rs:261`
- **Vulnerability Class:** N/A — negative finding (verification of "bit-for-bit inert" claim)
- **Data Flow:** `enable_snap_sync()` → `snap.threshold=50` → read ONLY as `== u64::MAX` / `< u64::MAX` sentinel, never as a gap comparator.
- **Evidence:** Exhaustive grep of every non-test read of `snap.threshold` (whole `crates/network/src/sync` + `bins/node/src`): production_gate.rs:630/732/740 use `== u64::MAX`; decision.rs:163 uses `< u64::MAX`; block_lifecycle.rs:261 is a log-only read. NO site does `gap > snap.threshold`. The four former gap-comparator reads are re-homed to the named constant `SNAP_SYNC_GAP_MIN` (decision.rs:177,208; cleanup.rs:492; production_gate.rs:822) and the peer-quality filter uses literal `local_height + 10` (dispatch.rs:263). Therefore 10 vs 50 is unobservable to every admission decision — both are simply "< u64::MAX = enabled."
- **False Positive Check:** Searched for any comparison `gap {>,>=,<} snap.threshold` and any exact `== 10`/`== 50` gate — none found. The dispatch minor-fork guard uses the SEPARATE constant `MINOR_FORK_GAP_MAX=50` (dispatch.rs:136), not `snap.threshold`; a coincidental numeric collision, not a read of the field.
- **Impact:** None. The RC-2 numeric change does NOT open a new operator-disable bypass. The "bit-for-bit inert" claim holds across all reachable admission inputs.
- **Remediation:** None required for the value change. (See 002 for the persistence concern the refactor left unaddressed.)

### SEC-AUTH-002: Emergency `enable_snap_sync()` is a one-way latch — first emergency permanently defeats `--no-snap-sync` for ALL subsequent (incl. non-emergency) reasons — conf(0.7, observed)
- **Location:** `production_gate.rs:740-751` (Gate 4 emergency branch), `block_lifecycle.rs:496-509` (`enable_snap_sync` / `disable_snap_sync`)
- **Vulnerability Class:** CWE-696 (Incorrect Behavior Order) / CWE-privilege-persistence — an authorization override that does not revert.
- **Data Flow:** operator sets `--no-snap-sync` (init.rs:696 → `disable_snap_sync()` → `threshold=u64::MAX`) → attacker withholds headers + inflates `best_height` so `gap>=50` and `consecutive_empty_headers>=10` (dispatch.rs:102-151) → `request_genesis_resync(GenesisFallbackEmptyHeaders)` (emergency) → Gate 4 emergency branch calls `enable_snap_sync()` → `threshold=50` **with no code path anywhere that restores `u64::MAX`** → thereafter Gate 4's `snap.threshold == u64::MAX` is `false` forever AND `decision.rs:163 snap_allowed = threshold < u64::MAX` is `true` forever.
- **Evidence:** Only writer of `u64::MAX` is `disable_snap_sync()` (block_lifecycle.rs:497), called solely at init (init.rs:696). No caller of `disable_snap_sync` exists outside init/tests (verified by whole-repo grep). No post-recovery restore in `set_post_recovery_grace` (production_gate.rs:615-623) or elsewhere. Consequently, after one emergency, a later NON-emergency forward-large-gap reason (`CoordinatorSnapEscalation` / `StuckSyncLargeGap`, classified `is_forward_large_gap` but NOT emergency, production_gate.rs:685-689) sails through Gate 4 (line 732 now false) — a SnapSync the operator disabled. `snap.attempts` is reset to 0 on any applied block (block_lifecycle.rs:151,307,371; cleanup.rs:503), so Gate 5 does not durably re-close the door either.
- **False Positive Check:** Searched for (a) any restore of `threshold=u64::MAX` after recovery, (b) a node-level restart that reloads `--no-snap-sync` post-recovery, (c) a scoped/temporary enable. Found none in sync-manager scope. Residual FP: if node-level orchestration force-restarts the process after every genesis resync, config reload re-disables snap (see "What I Don't Understand" #2) — this would downgrade the finding. Not resolved within the 5-file scope.
- **Impact:** The taxonomy's documented invariant "bypass-operator-disable = emergency ONLY" (production_gate.rs:664) holds per-request only for a node that has never tripped an emergency. It is a one-way latch: a single attacker-inducible emergency (empty-header withholding + inflated height, both in the stated trust boundary) permanently converts a `--no-snap-sync` node into a snap-enabled node, after which non-emergency and normal `should_snap` paths (decision.rs:164-167) can wipe state. The taxonomy is enforced as documented at the instant of the request but NOT as a durable operator guarantee.
- **NOT M6-introduced (honesty):** Pre-M6 the same line was `self.snap.threshold = 10` (confirmed via `git log -S`), equally persistent with the same "Temporarily enable" comment and no restore. RC-2 preserves this bit-for-bit. So M6 does not OPEN this path — but the refactor (which advertises `enable_snap_sync` as "symmetric to `disable_snap_sync`", block_lifecycle.rs:500) was the natural place to make the enable genuinely scoped/temporary and did not. The "symmetric" framing is misleading: disable is never called to close the door.
- **Remediation:** Make the emergency enable scoped: snapshot the prior `threshold` before `enable_snap_sync()` and restore it in `set_post_recovery_grace()` / on recovery completion; OR gate the emergency snap on a per-recovery boolean rather than a persistent field mutation, so `--no-snap-sync` is re-asserted once the emergency resync completes.

## Static Analysis Patterns
| Pattern | Files Matched | Risk | Notes |
|---------|--------------|------|-------|
| `gap {>,>=,<} snap.threshold` | 0 non-test | P0 if present | Clean — confirms RC-1 gap-floor de-homing complete |
| reads of `snap.threshold` | 5 non-test (3 sentinel `==MAX`, 1 sentinel `<MAX`, 1 log) | — | All sentinel/log; value is dead semantics (001) |
| writers of `threshold=u64::MAX` | 1 (`disable_snap_sync`, init-only) | P2 | No post-recovery restore → latch (002) |
| `is_emergency` matches! variants | 3 typed enums, fixed call sites | P1 | No peer data flows into the classification itself; peer data only steers WHICH call site fires |

## Cross-Perspective Signals
- **INV-SYNC-011 tension (injection/logic lane):** `AllPeersBlacklistedDeepFork` is classified EMERGENCY (bypasses Gate 4 + floor) but its call-site guard is only `gap > 12` (cleanup.rs:445-453), well below `MINOR_FORK_GAP_MAX=50`, and `gap` is derived from `network.network_tip_height` (gossip/peer-reported, attacker-influenceable). This is an emergency SnapSync reachable at gap as low as 13 — potential INV-SYNC-011 ("SnapSync unreachable for gaps < 50 except via corroborated deep-fork evidence") violation. Pre-existing (not in M6 diff at that line); flagging for the injection/logic auditor.
- **DoS (INC-I-120 lane):** `snap.attempts` resets to 0 on every applied block (block_lifecycle.rs:151/307/371) and cleanup.rs:503 resets after 30s — Gate 5 (attempt limiter) is soft. Combined with the 002 latch, a peer that intermittently supplies one block then resumes withholding could keep re-arming snap attempts. Relevant to the rate-governor auditor (INV-SYNC-009).

## Gaps
- Could not determine (within the 5-file scope) whether node-level orchestration restarts the process after a genesis resync, which would neutralize the 002 latch. Requires reading `bins/node/src/node/` recovery/restart flow — out of assigned scope.
- Did not measure/reproduce at runtime; findings are code-traced (`observed`), not `measured`. Ceiling conf(0.7) per auditor protocol.

## Summary
- P0: 0 findings
- P1: 0 findings
- P2: 1 finding (SEC-AUTH-002 — pre-existing operator-disable latch, preserved by M6)
- P3: 0 findings
- Negative verification: SEC-AUTH-001 — RC-2 10→50 value change confirmed inert; no new operator-disable bypass introduced by M6.

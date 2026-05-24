# Prompt Refinement — INC-I-089

Original:
INC-I-089: producer self-fork on restart due to gossip-vs-scheduler race window

SYMPTOM (observed 2026-05-24 on local testnet during rolling restart N1→N5→seed, v6.21.20 → v6.22.0):
- n1 (first node restarted) self-forked at h=22090 immediately after coming back online.
- n1 stayed on its private fork for ~5 minutes producing 4 blocks alone (h=22090..22093, all `from=self`, producer=n1's pubkey).
- Recovery infrastructure self-healed: FinalityGuard refused ShallowRollback (correct, per INC-I-081), coordinator escalated to SnapSync at gap=26 / last_applied=304s, `[RECOVERY] Genesis resync ACCEPTED: CoordinatorSnapEscalation`, n1 snap-synced to canonical root and caught up.
- Fleet (n2, n3, n4, n5, seed) was unaffected — canonical chain never stalled.
- n2-n5/seed restarts did NOT self-fork. Only n1 did.

ROOT CAUSE (confirmed from `getForkDiagnostic` ledger + n1 logs):
A race between gossip-arrival of the canonical block at the next height and the local scheduler firing for the producer's own slot. Sequence on n1:
1. n1 boots, peers handshake, receives canonical h=22089 (`ac41fbbd236b`) from n3 and applies it. Local tip = h=22089.
2. BEFORE canonical h=22090 (`caa25ba75ebb`) gossip arrives and is applied, n1's scheduler fires for slot 260212 (n1's scheduled producer slot).
3. n1's production loop builds on top of its local tip (h=22089) and produces h=22090 `cdc7c42ddbc0`.
4. Canonical h=22090 arrives microseconds later — same parent (h=22089) but different content. n1 classifies it as Orphan (1374 `ForkBlockReceived: Orphan` events follow).
5. n1 continues self-producing at slots 260216, 260220, 260224 on its private fork until coordinator escalates to SnapSync.

Probability per producer restart ≈ (online-window ~5s) / (5-producer slot rotation ~40s) ≈ 12.5%. With 5 producers in a rolling restart, ~50% chance per deploy that ONE producer self-forks.

THE FIX (per architecture analysis already completed in prior session):
Add a post-restart production lockout in `crates/network/src/sync/manager/production_gate.rs`. The exact gate mechanism ALREADY EXISTS for the snap-sync path (`[SNAP_SYNC] Production gated: awaiting first canonical gossip block`). Extend it to also engage at process startup.

Lockout entry: set on process start (in `Node::new()` or first sync-manager init).
Lockout unlock — ANY of:
  (a) received ≥1 gossip block from a non-self peer AND local tip is parent/that-block/descendant. Reuse the snap-path hook that clears `awaiting first canonical gossip block`.
  (b) Safety timer: ≥N slots elapsed since process start, N=3 default (~30s). Required for single-producer / no-peer scenarios.

While locked, `try_produce_block()` early-returns. Slot is simply skipped (valid producer behavior).

THREE-QUESTION CHECKLIST (per INC-I-075 protocol):
1. User-submittable tx triggers path? NO.
2. Producer-action/attestation pattern triggers? NO.
3. Bit-identical to old behavior for all reachable inputs? NO — producer may skip up to N self-slots in first N×slot_duration after startup. Skipping is valid; no consensus rule affected.

Therefore: NO NetworkParams activation height. NO CURRENT_PROTOCOL_VERSION bump. NO EPOCH_STATE_FORMAT_VERSION bump. NO HardForkSchedule entry. Rolling-deploy safe. Patch bump only (6.22.0 → 6.22.1).

CONSTRAINTS:
- TDD: write failing test FIRST. Test reproduces race deterministically — mock "process restarts at slot S = local producer's scheduled slot, canonical block at next height not yet arrived". Observe: producer skips slot, no `from=self` block built, gate clears after gossip-from-peer OR after N-slot safety timer.
- DO NOT touch CURRENT_PROTOCOL_VERSION, EPOCH_STATE_FORMAT_VERSION, NetworkParams activation heights, HardForkSchedule, chainspec.
- Reuse existing `awaiting first canonical gossip block` mechanism — no parallel gate.
- Safety-timer default (N=3 slots) must be a named constant next to other production-gate constants.
- Single-producer / no-peer testnet MUST still produce blocks after N-slot safety unlock. Add test for this.
- Follow "After Every Modification": build + clippy + fmt + tests, deploy to local testnet (rolling restart seed+N1-N5), verify NO self-fork via `getForkDiagnostic`.

KEY FILES:
- `crates/network/src/sync/manager/production_gate.rs` — canonical home for the fix
- `bins/node/src/node/production.rs` — `try_produce_block()` consumer
- `bins/node/src/node/init.rs` — where to set lockout on startup
- Existing snap-sync gate hook (search `Production gated: awaiting first canonical gossip block`)
- `crates/network/src/sync/manager/recovery.rs` — FinalityGuard context (do NOT modify)

DELIVERABLES: Failing test → fix in production_gate.rs → all existing tests pass → commit with three-question checklist → local testnet rolling-restart verification → open INC-I-089 in memory.db.

Author: `--author "Antonio Lozada <antonio@omegacortex.ai>"`.

Anchors detected:
- "ROOT CAUSE (confirmed from ...)" → PRESERVE — user explicitly states this was diagnosed in prior session with ledger + log evidence; reframing would discard validated work.
- "THE FIX (per architecture analysis already completed in prior session)" → PRESERVE AS HYPOTHESIS — treat as the user's proposed design with very high prior confidence (architect signed off previously), but the failing test MUST be written from symptoms/root-cause, NOT from the fix design (per Law #1 + output-contract.md sequencing).
- "KEY FILES (entry points for investigation)" → PRESERVE — these are scope-limiting guidance, not blinding anchors. Investigator should still verify the gate file exists where claimed.
- "probability ≈ 12.5%" → PRESERVE — quantitative claim about race window; useful for test design (the test must force the race deterministically, not rely on probability).

Domain context preserved:
- [terminal] n1 self-forked at h=22090 producing h=22090..22093 all `from=self`
- [terminal] 1374 `ForkBlockReceived: Orphan` events on n1
- [terminal] Recovery path: FinalityGuard refuse → CoordinatorSnapEscalation at gap=26/last_applied=304s → `[RECOVERY] Genesis resync ACCEPTED`
- [terminal] Fleet (n2-n5, seed) unaffected — only n1 hit the race
- [git] commits c05e02e3 (new) vs fc71a5ad (old) — both v6.22.0, no consensus/serialization delta
- [code] Existing snap-sync gate emits `[SNAP_SYNC] Production gated: awaiting first canonical gossip block`
- ⚠️ CONSTRAINT: NO CURRENT_PROTOCOL_VERSION bump, NO EPOCH_STATE_FORMAT_VERSION bump, NO NetworkParams activation height, NO HardForkSchedule entry, NO chainspec change.
- ⚠️ CONSTRAINT: Reuse existing snap-path gate hook — do not introduce parallel gate.
- ⚠️ CONSTRAINT: Safety-timer N=3 slots default MUST be a named constant.
- ⚠️ CONSTRAINT: Single-producer / no-peer scenario must still produce after N-slot safety unlock (separate test required).
- ⚠️ CONSTRAINT: TDD strict — failing test BEFORE any fix code is written or fix plan presented.
- ⚠️ CONSTRAINT: All commits authored as `Antonio Lozada <antonio@omegacortex.ai>`.

Regression context: INC-I-089 is a NEW class of bug at startup; not a regression of a previously-fixed bug. Baseline commits c05e02e3 vs fc71a5ad show no consensus delta — the race window has existed across versions but only manifests during rolling restart. No regression-required flag triggered.

Refined:
Investigate and fix INC-I-089: on producer restart, a race exists between (a) gossip-delivery of the canonical block at height H+1 and (b) the local scheduler firing for the producer's own slot, allowing the just-restarted producer to build height H+1 on its stale local tip and self-fork.

Investigation directive (omega-doctor):
1. VERIFY the structural claims in the prompt before accepting them:
   - Does `crates/network/src/sync/manager/production_gate.rs` exist and house the snap-sync gate?
   - Where exactly does the existing `awaiting first canonical gossip block` log emit from?
   - How does `try_produce_block()` in `bins/node/src/node/production.rs` currently consult the gate?
   - How is the gate constructed during `Node::new()` / sync-manager init?
2. CONFIRM root cause hypothesis against the code (forward reasoning from startup → first scheduler fire → production attempt).
3. DESIGN the lockout as a strict extension of the snap-sync gate (not a parallel mechanism). Document the unlock-trigger reuse precisely.
4. TDD SEQUENCE (non-negotiable):
   a. Test Writer FIRST. Produce Output Contract Checklist for the gate's `should_produce()` (or equivalent) decision function. Enumerate ALL outputs × paths × input partitions:
      - Output: bool (produce / skip) + structured reason
      - Paths: locked-at-startup, unlocked-by-gossip-from-peer, unlocked-by-safety-timer, snap-sync-path (must remain functional), single-producer-no-peer (unlocks only via timer)
      - Input partitions per path: with/without prior gossip, at/before/after safety timer, gossip from self vs peer, gossip aligned vs non-aligned with local tip
   b. Test MUST FAIL with current code (proves race is real).
   c. Fix in `production_gate.rs` + minimal wiring. Test MUST PASS.
   d. All existing tests still pass (esp. snap-sync gate tests).
5. CONSTRAINTS gate the fix: any drift toward bumping protocol/epoch versions or adding activation height is REJECTED (explicit user prohibition).
6. Verify via local testnet rolling restart + `getForkDiagnostic` showing zero new `ForkBlockReceived: Orphan` events on restarted nodes.
7. Commit message MUST include explicit answers to the three-question checklist.

Triage hint: This is a localized fix to a single subsystem (production gate) with extremely well-bounded symptoms, validated root cause, and an existing analogous mechanism to extend. Strong FAST-path candidate UNLESS verification reveals the prompt's structural claims are wrong (e.g., `production_gate.rs` doesn't exist or the snap-sync gate is structured differently than claimed) — in which case DEEP path is required to map the real architecture.

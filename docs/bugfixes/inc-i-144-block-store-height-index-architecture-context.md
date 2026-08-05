# INC-I-144 — Architecture Context: Block-Store Height-Index Fossils

**Pipeline step**: /omega-diagnose Step 2.5 (focused structural analysis of confirmed root cause)
**Incident**: INC-I-144 (RUN_ID=468) — mainnet block-store integrity divergence, seed2 h7222-7227
**Upstream input**: `docs/.workflow/diagnosis-report.md` (VERDICT conf(0.95, measured)), `docs/.workflow/evidence-assembly.md`
**Mode**: READ-ONLY structural analysis. No design, no milestones, no new specs. This document answers ONE question: **why does the architecture allow this class of bug?**

---

## 1. Verified Structural Map

### 1.1 Index mutation authority — writer/deleter inventory (verified by workspace grep + file reads, basis=measured)

The canonical by-height projection (`CF_HEIGHT_INDEX` + `CF_HASH_TO_HEIGHT`) has **six live writer entry points across three crates and ZERO live delete paths**:

| # | Writer | Location | Mechanism | Linkage-verified? |
|---|--------|----------|-----------|-------------------|
| W1 | `apply_block` → `set_canonical_chain` | `bins/node/src/node/apply_block/mod.rs:306` → `crates/storage/src/block_store/writes.rs:102` | tip-down healing walk, first-match break (:111-118), snap_horizon floor (:129-135) | YES (walks prev_hash) |
| W2 | `backfillFromPeer` → `put_block_canonical` | `crates/rpc/src/methods/backfill.rs:418` → `writes.rs:78` | direct per-height index write, missing-heights-only | NO (BLAKE3 checksum only) |
| W3 | Archive import → `put_block_canonical` | `crates/storage/src/archiver.rs:439` | direct per-height index write | NO (genesis_hash check only) |
| W4 | Checkpoint restore → `put_block_canonical` | `bins/node/src/operations/restore.rs:355` | direct per-height index write | NO |
| W5 | Snap anchor → `seed_canonical_index` | `bins/node/src/node/fork_recovery.rs:377`, `bins/node/src/node/init.rs:418,455` → `writes.rs:172` | single anchor entry + sets `snap_horizon` floor | N/A (single entry) |
| W6 | Offline reindex → `rebuild_canonical_index` | `bins/node/src/operations/chain.rs:25,311` → `writes.rs:196` | full clear + tip→genesis header walk | YES — **the only deleter, offline-only** |

**Deleters on any live path (rollback_one_block, execute_reorg rollback phase, fork_recovery, snap install): ZERO.** Independently verified two ways: (a) full read of `rollback.rs` + reorg rollback phase `block_handling.rs:533-844` — no block-store index calls; (b) workspace grep for all four index-writing methods — no call site in any rollback/rewind path. Both methods agree (Redundant Verification satisfied).

### 1.2 The module's own documentation contradicts itself (basis=measured)

`writes.rs:100`: *"This is the ONLY method that writes to height_index/hash_to_height"* (about `set_canonical_chain`). False in the same file: `put_block_canonical` (writes.rs:78-91), `seed_canonical_index` (writes.rs:172-186), and `rebuild_canonical_index` (writes.rs:196+) all write both CFs. The invariant's contract (`writes.rs:17-19`: index reflects canonical chain only) lives in a comment that is already drifted from the code **inside its own module**. This is direct evidence for the missing-ownership explanation (S1 below): nobody owns the writer set, so nobody noticed it fragmenting.

### 1.3 Data flow around the root cause

```
                     APPLY (symmetric)                ROLLBACK (asymmetric)
  block ──> apply_block() ──┬─> chain_state/utxo/producer   rollback_one_block()/execute_reorg
                            └─> set_canonical_chain()  ──┬─> chain_state/utxo/producer (undo)
                                 [W1: heals index]       └─> index: UNTOUCHED → fossils

  snap jump: apply_snap_snapshot() ──> seed_canonical_index() [W5]
             sets snap_horizon → W1's healing walk PERMANENTLY fenced out of pre-anchor range

  fill: backfillFromPeer [W2] / archive [W3] — missing-heights-only, occupied fossils skipped

  readers (12 consumers, see diagnosis blast table): RPC by-height, P2P serving,
  archiver, integrity scan, rewards lookback, AND rollback.rs:79 / block_handling.rs:585
  (best_hash/common-ancestor resolution — latent self-fork, INC-I-008 family)
```

Consensus 3-state (ChainState/UtxoSet/ProducerSet) never derives from this index — which is why state roots stayed byte-identical while the projection diverged. The defect class lives entirely in the projection layer; its severity comes from the two *control-plane* readers (rollback.rs:79, block_handling.rs:585) that feed the projection back into consensus decisions.

---

## 2. Architecture Constraint Table (memory.db via evidence-assembly, + git history)

| # | Prior approach | Result | Structural lesson (what NOT to conclude/design) |
|---|---------------|--------|------------------------------------------------|
| C1 | INC-I-008: apply_block skip-guard fix for stale hash_to_height | resolved (narrow trigger) | Patching one CONSUMER of a stale index does not fix the class — it recurred (this incident) |
| C2 | INC-I-025 (`25b200a7`): snap_horizon floor in set_canonical_chain | resolved snap-anchor crash — **created the permanence contributor here** | Locally-correct guards on the healing walk can revoke the liveness assumption other code depends on; any floor relaxation must preserve the "anchor header absent" crash protection it was built for |
| C3 | Partner v6.7.5 height-occupied ingestion guard (`23093519`) | rejected as symptom patch | Ingestion-time guards don't fix the write/rollback asymmetry |
| C4 | INC-I-041 (`38f10fc4`): atomic_replace reads utxo_store not stale state_db | resolved | The PROVEN medicine for this exact shape: make the rewind path mutate/read the derived structure atomically. Block-index equivalent was never applied |
| C5 | INC-I-083 + misfiled N12 findings (under INC-I-107 record): height-index corruption post-ShallowRollback, state correct | still open, no fix | The class recurs in NORMAL post-bootstrap operation (h≈321k) — not a bootstrap-only fluke; any fix must cover standalone ShallowRollback, not just reorg |
| C6 | backfill repair mode (`131183d7`): tip-anchored first-match reverse scan | shipped, RPC-only | Tip-anchored detection is structurally sandwich-blind; also the ONLY orphan-purging code is manually invoked — self-cleaning was never wired into the automatic paths |
| C7 | Incremental chain commitment (`883e3c52`) → replaced by periodic full scan (`defe1416`) | incremental abandoned as unreliable | Precedent that this projection's integrity machinery has already failed once and been rebuilt — detectors here must be simple and full-walk, not clever/incremental |

Active invariants constraining any fix: INV-SYNC-002/006/007, INV-SYNC-001/004/008 (finality-guard family), INV-ARCHIVER-001, INV-GUARD-002. **None governs by-height-index cleanup — confirmed invariant gap** (evidence-assembly §Active Invariants).

---

## 3. Structural Explanations — Explorer/Skeptic/Analogist Loop

Five candidate explanations for "why does the architecture allow this class of bug", attacked against the diagnosis evidence ([E1]-[E7]) and the constraint table. Two loop passes run; second pass merged/refined survivors.

### S1 — Missing invariant OWNERSHIP — conf(0.85, observed) — SURVIVES (load-bearing)
The canonical-only contract exists solely as a comment (`writes.rs:17-19`); no module owns it, no INV-* codifies it, no test enforces it. Mutation is split across 3 crates (storage internal walk, node rollback/reorg orchestration, RPC backfill, archiver), and §1.2 shows the module's own doc already lost track of its writer set.
*Skeptic attack*: "comments-as-contracts is universal; why did THIS one fail?" — Answer: because the contract has a **liveness half** ("stale entries get rewritten later") that spans crates: storage provides the healing walk, but only the node layer knows whether a walk will ever traverse a range. A contract whose enforcement spans a crate boundary with no owner on either side is unenforced by construction. Attack survives as refinement, not elimination.

### S2 — Asymmetric state transitions — conf(0.9, measured) that the asymmetry exists; refined by S3 — SURVIVES (mechanism layer)
Apply mutates {consensus-state, index}; rollback mutates {consensus-state} only ([E1], [E6], §1.1). No single "chain mutation" abstraction forces both directions through one API.
*Skeptic attack*: "was this omission or design?" — The early-exit comment (`writes.rs:96-98`: *"a 10-block reorg only writes 10 entries"*) proves the designer explicitly modeled reorgs: cleanup was **intended** to happen by last-write-wins overwrite during the winning branch's re-apply walk. So the asymmetry is not raw omission — it is a deliberate **lazy-heal (eventually-consistent) design** whose compensation runs in the opposite direction's code path. That reframing hands the causal weight to S3. conf(0.8, observed) that lazy-heal was the intent.

### S3 — Self-healing-by-convention, silently invalidated by feature interaction — conf(0.9, observed) — SURVIVES (primary explanation)
Correctness of the lazy-heal design depends on an unstated liveness precondition: *every rolled-back range is eventually traversed by a canonical apply-walk*. Three later, each-locally-correct features revoked it:
1. **snap_horizon floor** (INC-I-025, `25b200a7`) — correct for its purpose (anchor header absent → walk crashes, `writes.rs:130`), but forbids healing below the anchor even on nodes with full history → fossils become permanent ([E2]).
2. **Standalone ShallowRollback** (recovery ladder, INC-I-139/143 era) — rollback with NO paired re-apply; the design assumed rollback only occurs inside a reorg immediately followed by applying the better branch. C5 (N12 at h≈321k) proves this trigger fires post-bootstrap.
3. **Missing-only per-height fill** (backfill [E3], W2-W4) — fills gaps but skips occupied fossil heights, sealing the sandwich.
*Skeptic attack*: "feature interaction is a description, not a cause — the cause is that the precondition was never written down." — Correct: S3 collapses into S1 at the root. The pair {S1 ownership gap, S3 revoked liveness} is the two-level answer: the precondition was unowned (S1), so nothing stopped later features from revoking it (S3).

### S4 — Presence-based tooling / missing canonicity oracle — conf(0.85, observed) — SURVIVES (invisibility layer, downstream)
Every detector answers "is there a block at h" (missingCount, ensure_blocks_present, FORK_GUARD checks, checkpoint health) or anchors at the tip (backfill repair reverse scan, C6); nothing answers "is index[h] on the canonical chain". The only canonicity-restoring tool (W6 reindex) is offline. ([E3], [E4]).
*Skeptic attack*: "this explains non-detection, not creation" — sustained. S4 is eliminated as a *sole* explanation but survives as the layer that turned a transient defect into a 12-day-silent permanent one, and it is independently actionable.

### S5 — Blind-trust readers — conf(0.8, observed) — MERGED into S1
`get_block_by_height` has an implicit canonicity contract that 12 consumers assume (`queries.rs:171-177` is a correct lookup over wrong content — diagnosis counter-hypothesis 2). This is the read-side face of the same unowned contract; not independently load-bearing. Notably two of the blind readers are the rollback/reorg paths themselves (rollback.rs:79, block_handling.rs:585) — the defect's own creation path consumes its own output, which is what upgrades this from cosmetic to latent-consensus-hazard.

### Loop verdict (pass 2)

No single candidate survives as sole explanation; the class exists because of a **three-layer conjunction**, each layer independently verified:

| Layer | Explanation | conf |
|-------|------------|------|
| Creation | Lazy-heal index design: apply-side overwrite is the only cleanup; rollback intentionally writes nothing (S2 refined by S3) | conf(0.9, measured) |
| Permanence | Unowned liveness precondition revoked by three locally-correct later features: snap floor, standalone ShallowRollback, missing-only fill (S3+S1) | conf(0.9, observed) |
| Invisibility | No canonicity oracle anywhere in the runtime; all detection presence-based or tip-anchored (S4+S5) | conf(0.85, observed) |

**One-sentence answer**: the height index is a materialized view with insert-triggers but no delete-triggers, whose repair liveness was guaranteed only by convention, whose contract lives in a drifted comment spanning three crates with six writers and zero owners, and whose consumers — including the rollback path itself — have no way to ask whether what they read is canonical.

---

## 4. Pattern Matches (Analogist)

- **Recurring in-project shape (3rd+ occurrence, diagnosis Shape-Recurrence=RECURS)**: "derived structure not maintained atomically with the primary state mutation, downstream consumers trust it blindly" — INC-I-041 (state_db UTXO view), INC-I-118 (in-memory UtxoSet post-snap), INC-I-008/025/083/N12 (this index). The INC-I-041 fix (make the rewind path structure-consistent) is the proven in-project medicine for the shape.
- **Known anti-pattern**: *materialized view without invalidation path* — incremental refresh on insert, nothing on delete; equivalently a saga with a compensating action defined for one direction only.
- **Industry parallel**: Bitcoin Core maintains its height→block mapping (`chainActive`/`CChain::SetTip`) symmetrically — `SetTip` runs on both ConnectTip and DisconnectTip, so a disconnected block's height entry is removed in the same step that rewinds the tip. The missing abstraction here is exactly a symmetric set-tip: DOLI has the connect half (W1) and never built the disconnect half.
- **Complexity anti-pattern check (for the fix phase)**: C1/C3/C6 show this area's history is consumer-side and ingestion-side patches — additive guards at ever-more call sites. The structural lesson is subtractive: one symmetric mutation at the two rewind sites retires the need for per-consumer guards.

---

## 5. Invariant Gaps (structural observations — NOT new specs)

Flagged for the invariant-protocol step of the fix pipeline (Level 2+ incident → mandatory INV + regression test; Level 3 → monitoring signal):

1. **INV-STORAGE-00X (candidate, from diagnosis residual)**: `height_index`/`hash_to_height` MUST be mutated atomically with every `chain_state` rewind; a rolled-back height MUST NOT retain its index entry. Regression test shape: rollback N blocks → assert `get_hash_by_height` returns None (or the redirected canonical hash) for every rewound height; FAIL on current code.
2. **Writer-set ownership observation**: the index currently has 6 live writers (§1.1), 3 of which (W2/W3/W4) bypass the linkage-verified walk entirely. Any minted invariant should name the permitted writer set explicitly, or the next backfill-style feature re-opens the hole. (Also: fix the false claim at `writes.rs:100`.)
3. **Canonicity-oracle observation**: the periodic commitment fold (`periodic.rs:1278-1305`) already reads every block per height; a prev-linkage check inside that existing loop is the natural home for the Level-3 monitoring signal (detects fossils regardless of how they are created — covers unknown future creation paths, per C7's "simple full-walk over clever incremental" precedent).
4. **Snap-floor scope observation**: the floor's own justification (`writes.rs:130`: anchor header never persisted) is narrower than its implementation (unconditional height comparison). The floor could condition on actual header absence without violating INC-I-025's protection — noted as the structural basis for the diagnosis's DEFENSE-IN-DEPTH #1.
5. **Misfiled institutional memory**: the N12 height-index findings (entries 975-977) sit under the unrelated INC-I-107 record — should be re-linked to INC-I-144 at close-out so the recurrence chain is queryable.

---

## 6. Structural Fix Direction (direction only — design/implementation belongs to the fix pipeline)

Ranked by the diagnosis's break-the-chain analysis and the constraint table; the root-cause fix is the symmetric-mutation one, everything else is defense-in-depth:

1. **Root cause (breaks [E1], INC-I-041 medicine)**: rollback/reorg rewind deletes (or canonically rewrites) `height_index[target+1 ..= old_tip]` + matching `hash_to_height` entries in the same operation that rewinds chain_state — at both sites: `rollback_one_block` and `execute_reorg`'s rollback phase. Converts fail-silent-wrong → fail-visible-missing (every existing presence detector then sees the hole until re-apply heals it).
2. **Permanence (breaks [E2])**: relax snap_horizon floor from unconditional to "stop when parent header actually missing" (§5.4).
3. **Invisibility (breaks [E4])**: prev-linkage check piggybacked on the existing periodic commitment fold.
4. **Propagation/repair (breaks [E3])**: sandwich-capable divergence finder (ranged-commitment binary search over existing `from_height`/`up_to_height` params) + prev-linkage validation on backfill ingest per INV-SYNC-006.

━━━ RESOURCE COST
CPU: Event-driven only. Rollback-time index deletion: ≤100 batch deletes per rollback event (MAX_CUMULATIVE_ROLLBACK=50, rollback.rs:57, × 2 CFs); steady-state delta ~0 (basis: inferred from measured cap). Prev-linkage scan: one 32-byte hash compare per height inside the existing periodic fold loop that already fetches each block (periodic.rs:1278-1305); steady-state delta ~0 (basis: observed — loop and fetches already exist). Sandwich finder: operator-invoked only, O(log n) ranged commitment calls (basis: inferred).
Memory: +≤~4KB transient WriteBatch per rollback event (≤50 heights × 2 CFs × ~40B entries); zero resident growth (basis: inferred).
IO: +1 RocksDB WriteBatch write per rollback event; 0 additional reads in the periodic scan (blocks already read for the commitment fold) (basis: observed).
Network: 0 steady-state; divergence finder adds operator-invoked ranged verifyChainIntegrity traffic only (basis: inferred).
Disk: net NEGATIVE — removes stale index entries; RocksDB compaction absorbs tombstones (basis: inferred).
Latency: rollback path +<1ms per event (batched deletes inside an already-heavyweight recovery operation); hot apply/serve paths untouched (basis: inferred).
Inevitability: INEVITABLE — any correct fix for this class must either mutate the index on rewind (this cost) or verify canonicity at read time; there is no zero-cost correctness.
Cheaper alternative: NONE-EXISTS for the creation-layer fix. The read-side alternative (verify prev-linkage inside get_block_by_height on every call) costs one extra header read per lookup on the hot RPC/P2P serving path — strictly more expensive at steady state.
Why this proposal anyway: pays a bounded, event-driven cost on an already-rare recovery path to convert a fail-silent consensus-adjacent hazard into a fail-visible gap, mirroring the proven INC-I-041 fix shape and retiring the need for further per-consumer guards.
━━━

Separately: mainnet cleanup of existing fossils is an operational task (offline `doli-node reindex` after header-completeness verification, per diagnosis §"Cleaning the 3 mainnet seeds"), not a code fix — the code fix prevents recurrence but does not repair seed1/seed2/seed3 in place.

---

## 7. Quality Audit

```
━━━ DESIGN DECISION QUALITY AUDIT (structural-analysis mode) ━━━
Structural explanations evaluated:     5 (S1-S5)
  basis=measured:                      2 (S2 asymmetry; §1.1/§1.2 writer inventory + doc contradiction)
  basis=observed:                      3 (S1, S3, S4 — code comments, git history, constraint table)
  basis=assumed:                       0
Eliminated as sole explanation:        S4, S5 (downstream); none fabricated to survive
Survivor confidence range:             0.85-0.90 — discriminated, not flat
Constraint table entries used:         7 (C1-C7) — all cross-referenced in attacks
Redundant verification:                apply-only-index claim confirmed by 2 independent methods (file reads + workspace grep)
━━━ SIMPLICITY AUDIT ━━━
Subtraction explored:                  yes — "remove the height index" rejected (12 live consumers);
                                       "remove the snap floor instead" retained as defense-in-depth only
                                       (doesn't stop fossil creation or the rollback.rs:79 latent hazard)
Fix-direction complexity cost:         0 new modules; 2 existing functions gain a delete step, 1 existing
                                       loop gains a compare, 1 existing RPC gains a search mode
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Appendix — memory.db rows (for orchestrator insertion; Bash unavailable in this agent context)

```sql
INSERT INTO incident_entries (incident_id, entry_type, content, agent, run_id) VALUES
('INC-I-144','discovery','STRUCTURAL: height index has 6 live writer entry points across 3 crates (apply set_canonical_chain, backfill/restore/archiver put_block_canonical, snap seed_canonical_index) and ZERO live delete paths; only deleter is offline rebuild_canonical_index. writes.rs:100 doc claim "ONLY method that writes" is false within its own file — direct evidence of unowned writer set.','architect',468),
('INC-I-144','discovery','STRUCTURAL VERDICT (3-layer, conf 0.85-0.9): (1) creation = intentional lazy-heal design — rollback writes nothing, cleanup relied on winning-branch re-apply overwrite (writes.rs:96-98 early-exit comment proves intent); (2) permanence = unowned liveness precondition revoked by 3 locally-correct features (INC-I-025 snap floor, standalone ShallowRollback, missing-only backfill); (3) invisibility = no runtime canonicity oracle, all detectors presence/tip-anchored. Same shape as INC-I-041/118 — 3rd+ recurrence of derived-structure-not-atomic-with-rewind.','architect',468),
('INC-I-144','discovery','STRUCTURAL: industry parallel — Bitcoin Core SetTip runs on BOTH ConnectTip and DisconnectTip; DOLI built only the connect half. Fix direction: symmetric index mutation at rollback_one_block + execute_reorg rewind (INC-I-041 medicine), event-driven cost only (≤100 batch deletes/rollback, steady-state ~0). Invariant gap confirmed: no INV-* governs by-height-index cleanup; candidate INV-STORAGE-00X minted in report. N12 findings misfiled under INC-I-107 — relink at close-out.','architect',468);
```

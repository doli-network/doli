# Fork Lifecycle Redesign — Analyst Scoping

**Incident:** INC-I-204 (escalated) · **Run:** 541 · **Mode:** proposal-only, read-only
**Scope:** how forks come to exist (production/error paths) and how nodes recover (rollback → reorg → sync → snap).
**Author:** analyst · **Date:** 2026-09-01

---

## 0. Prerequisite gap (declared, not silently absorbed)

The mandate named five upstream artifacts. **Four do not exist on disk:**

| Artifact | Status |
|---|---|
| `docs/.workflow/prompt-refinement.md` | PRESENT — read |
| `docs/.workflow/domain-diagnosis-report.md` | **ABSENT** |
| `docs/.workflow/evidence-assembly.md` | **ABSENT** |
| `docs/.workflow/domain-investigation-{fork,connectivity,parameters,code}.md` | **ABSENT** (all four) |

`docs/.workflow/` was verified by directory listing. The only INC-I-204-era file is `prompt-refinement.md`.

**Consequence for this document:** the INC-I-204 causal chain reached me only as a *summary* inside the task prompt.
Per PRIOR-KNOWLEDGE-GATE I treated every claim in it as a **hypothesis** and re-verified it against source and
`.omega/memory.db`. Verification results are marked `[VERIFIED]`, `[REFINED]`, or `[UNVERIFIED]` throughout.
Nothing in the Requirements section rests on an `[UNVERIFIED]` claim.

**Tooling note (recorded limitation, not a blocker):** the code graph was tried first as protocol requires —
`graphify explain rollback_one_block --graph graphify-out/graph.json` returned **degree 2** (only `Result`
[references] and `Node` [method]). This is the known, memory-recorded graphify blind spot for Rust receiver
calls (`self.method()`). Blast radius below is therefore grep-derived and labelled as such.

---

## 1. Context

DOLI has had four incidents of one shape in 180 days (INC-I-081 → INC-I-147 → INC-I-190 → INC-I-204). The
diagnosis synthesizer ruled that the next recurrence after a fix routes to redesign. This is that route.

The database sweep changes the framing of the problem. This is not a recurring bug in a subsystem:

> **107 of 192 incidents (55.7%) in `.omega/memory.db` match `fork|reorg|rollback|snap|sync|finality|wedge|divergence`.**
> The matches span 2026-03-19 (INC-001) to 2026-08-31 (INC-I-204) — the entire life of the project, with no
> quiet quarter.

Fork-lifecycle failure is **the dominant failure class of the system**, not an intermittent defect. Any redesign
scoped as "fix the INC-I-204 chain" is scoped too small; any redesign scoped as "replace consensus" is scoped
too large and is explicitly excluded below.

Two prior attempts at exactly this already happened and did **not** end the class — this is the single most
important calibration fact in this document:

- **`crates/network/src/sync/manager/mod.rs`** carries hotspot risk `critical`, `times_touched=23`, with the
  description *"M1-M3 redesign: 67→25 fields, 14→3 states, 11→4 production checks. Structurally simplified."*
  A structural simplification of the sync manager has already been executed. Forks continued.
- **`production_gate.rs`** likewise: *"M2 redesign: 590→130 lines. 4 checks replace 11 layers."* (`times_touched=18`).

Decision 34 (INC-I-120) records the reason plainly: *"55+ prior patch attempts (INC-I-040) prove symptom-patching
does not hold."* Decision 35 (INC-I-139) records the shape that *did* work — **consolidation by subtraction**,
which "kills 4-incident recurrence class INC-I-005/033/138/139", with *"parameter band-aid (threshold 50→500)
explicitly WONT-listed."*

**Calibration:** simplifying the sync manager is not the lever. Two rounds of it are already in the tree. The
lever has to be something those rounds did not touch — and §4 argues it is *authority over "what is my canonical
branch"*, which no round has ever consolidated.

---

## 2. Database Genealogy

### 2.1 The four-incident spine

| Inc | Date | What it changed | What it did **not** change | How the next one got in |
|---|---|---|---|---|
| **INC-I-081** | 2026-05-18 | 5 commits. `calculate_epoch_rewards` → `Result` + abort slot (INV-PROD-001). Sync bugs 1-4: ShallowRollback finality guard (INV-SYNC-001), `plan_reorg` real-height fallback (INV-SYNC-002), `try_apply_direct_successor` (INV-SYNC-003), `clear_finality_if_below_tip` backstop (INV-SYNC-004). | The **production** error arm. Bug 5 (snap-source canonical filter, INV-SYNC-005) **DEFERRED — still deferred today**. | The INV-SYNC-004 backstop it *added* became the finality-erasure mechanism INC-I-147 exploited. |
| **INC-I-147** | 2026-07-31 | Nine defects, four-stage cascade. Mempool/builder parity (D1/D2). `plan_reorg` real-height gate behind `inc_i_147_activation_height` (INV-SYNC-012). | **D3 was not removed**: `production/mod.rs` still calls `rollback_one_block` on *any* error incl. pre-mutation rejects. INV-PROD-002 was written but deliberately **not enforced**. Six sibling context-parity gaps left armed ("7 of 18 context fields are block-validation-only"). | The fix is **AH-gated** — dormant below the height. INC-I-204 wedged at h=77777, **below** testnet AH 80_700. |
| **INC-I-190** | 2026-08-28 | F1 attestation weight from local ProducerSet; F2 depth-2 finality; F3 AH-gated floor bound (INV-EPOCH-005). | The ladder itself ("ladder files zero-diff vs v6.24.1"). Snap sync remained the only terminal. | Ladder unchanged → INC-I-204 hits the same absorbing state from a different trigger. |
| **INC-I-204** | 2026-08-31 | *(open)* 9 of 18 testnet nodes wedged at h=77777 since 2026-08-24; FINALITY_GUARD refusal counts n6=43, n17=53, seed=32, n1=0. | — | — |

### 2.2 The genealogy's own verdict

Each fix **added a guard**. No fix ever **removed an authority**. The count of independent components that can
answer "what is my canonical branch" only ever went up. INV-SYNC-012 says this out loud:

> *"RELATED: INV-SYNC-004 (INC-I-081) constrains the OTHER operand of the same guard... **both operands of the
> plan_reorg finality comparison have now had independent defects, which is itself a signal that the comparison
> mixes units.**"*

That sentence, written by a previous investigation, is the redesign thesis. §4 develops it.

### 2.3 The contradiction at the centre — **the single most important finding**

> ⚠ **CONTRADICTION NAMED.** The INC-I-204 summary indicts the BLOCK_POISON rollback as the DIVERGENCE step.
> The INC-I-190 record credits the *same* code as the only thing that saved half the fleet.

INC-I-190 `root_cause`, verbatim:

> *"14 nodes snapped (n1-n10, n12, seed1-3) leaving a permanent 49-block body hole 314592-314640; **13 escaped
> (n11, ivan, santiago, 6 jorge, 4 folsi) via the BLOCK_POISON ADDBOND_CAP_EXCEEDED unconditional
> rollback_one_block() which bypasses the finality guard.**"*

INV-PROD-002 encodes the consequence as a **mandatory sequencing caveat**:

> *"This invariant MUST NOT be enforced in code before a replacement wedge-escape ships... The rollback it
> forbids is currently the fleet's ONLY escape from the finality wedge — measured 6/6 producers escaped via
> their own poison rollback while the 2 nodes that never rolled back NEVER escaped."*

with a self-correction appended 2026-08-04 that is itself important:

> *"CORRECTED: the word PERMANENT was wrong. Both non-rollback nodes DID escape... via genesis-resync then snap
> sync... The caveat still stands (escape is catastrophic, not free) but must not be justified by the false
> claim that no escape exists."*

**Resolution of the contradiction:** both records are true and they are not in conflict once the mechanism is
read precisely (see §4.1 `[REFINED]`). The poison arm is a **finality-guard bypass**. A bypass is a fork factory
when the tip was healthy and an escape hatch when the tip was wedged, because *nothing in the code distinguishes
those two cases*. That is the defect — not the rollback, and not the guard.

**Binding consequence:** removing the poison rollback **first** converts every poison event into a fleet-wide
wedge escapable only through history-destroying snap sync. Order is a hard constraint (REQ-FORK-012), not a
preference.

### 2.4 Failed approaches — do not re-propose

Only 5 rows exist in `failed_approaches` and none are fork-domain, but two carry design filters that bind here:

| Filter | Source | Binding rule for this redesign |
|---|---|---|
| **Frozen-history wire break** | INC-I-176 M2.5 | *"UNGATED wire-format break on consensus-visible FROZEN HISTORY... a node on the new binary cannot sync past a block containing an old-shape tx, in BOTH deploy directions. **A synchronized deploy does NOT repair it — the block is re-validated on every full sync from genesis.**"* Any redesign touching block/header/tx shape must be AH-gated *and* keep the old decoder forever. |
| **Filter F7 — no adversary-advanceable bound** | INC-I-176 R6 tombstone | An approach dies if it introduces *"a bound term advanceable by a ≥threshold holder"*. Killed two proposals 5/5 and 3/3. Any new fork-choice or recovery bound must be checked against F7. |

From `decisions`, three more binding precedents:

- **d.27 / d.28 (INC-I-054):** never bump `CURRENT_PROTOCOL_VERSION` for non-EpochState changes; never move a
  crossed AH. Both caused fork cascades.
- **d.29 (INC-I-054):** *"`rebuild_epoch_state_from_blocks()` is architecturally unsafe and must be replaced"* —
  it scans local blocks, and snap-synced nodes have incomplete history. **Still unreplaced.** This is a live
  landmine directly under the redesign's feet: snap sync is the escape, and the escape leaves nodes unable to
  safely rebuild.
- **d.35 (INC-I-139):** consolidation **by subtraction** is the shape that has actually killed a recurrence class
  in this codebase. Parameter tuning is WONT-listed.

### 2.5 The mainnet deploy event the user cited — corrected

The user's framing was: *a deploy restart made ~99% of structural nodes snap-sync and fork.* The DB record
**contradicts the causal half** of that and the correction matters, because it removes "deploys" as the lever:

> INC-I-190 `root_cause`: *"**NOT caused by the v6.25.0 deploy** (5.5-8.8h earlier; ladder files zero-diff vs
> v6.24.1)."*

The actual trigger was a same-height fork at h=314591 (22daab44 slot 317069 vs f2be9d1a slot 317070, both
children of b4cd8130) where the winning sibling **self-finalized 223 ms after apply** from its own carried
attestation bitfield. The fleet then refused reorg past finalized 314591 **for 18 minutes against a 50-block-heavier
branch**, stalled 52 slots, crossed the gap=50 cliff, and exited through snap sync.

**Therefore:** hardening the deploy procedure would not have prevented it. The deploy was coincident, not causal.
The lever is the ladder and the finality authority. (Deploy discipline remains required for *other* reasons —
INV-8 synchronized deploy, INC-I-062 — but it is out of scope here.)

---

## 3. Capability Inventory (PRIOR-KNOWLEDGE-GATE)

Enumerated from source before any "the system lacks X" claim. **The system does not lack recovery primitives —
it has 20+ and they disagree.**

### 3.1 Recovery ladder — `RecoveryAction`, 6 variants
`crates/network/src/sync/manager/recovery.rs:113` — `None`, `SiblingFetch{height}`, `ShallowRollback{depth}`,
`HeaderFirstSync`, `SnapSync`, `GenesisResync`.

### 3.2 Sync state machine — 3 states / 5 phases
`crates/network/src/sync/manager/types.rs:85` — `SyncState`: `Idle`, `Syncing{phase,started_at}`, `Synchronized`.
`SyncPhase`: `DownloadingHeaders`, `DownloadingBodies`, `ProcessingBlocks`, `SnapCollecting`, `SnapDownloading`.
`RecoveryPhase` (types.rs:351): `Normal`, `ResyncInProgress`, `PostRecoveryGrace{..}`, `AwaitingCanonicalBlock{..}`.

### 3.3 Rollback / rewind primitives — 4 distinct
1. `Node::rollback_one_block` — `bins/node/src/node/rollback.rs:10` (434 LOC). Doc-comment: *"Unconditionally
   roll back 1 block for fork recovery. **No preconditions — it just rolls back.**"*
2. `Node::execute_reorg` — `bins/node/src/node/block_handling.rs` (own undo loop, ~686-996).
3. Undo-based path (`cf_undo`, `UNDO_KEEP_DEPTH = 100`, `crates/core/src/consensus/constants.rs:265`).
4. Legacy rebuild-from-blocks fallback (guarded by `ensure_blocks_present`, INV-SYNC-015; FORK_GUARD refusal at
   `rollback.rs:188`).
Plus `maintainer_rewind/` as a coupled side-channel (INC-I-174).

### 3.4 Reorg planner entry points — 2
`plan_reorg` (`crates/network/src/sync/reorg/mod.rs`, 622 LOC) and `check_reorg_weighted` (mod.rs:306).
**Both consume the same synthetic per-process height**; only `plan_reorg` received the INC-I-147 real-height fix.
INV-SYNC-012: *"KNOWN REMAINING SITE... `check_reorg_weighted` uses the same synthetic height with `unwrap_or(0)`
and NO `get_height` fallback — strictly worse."*

### 3.5 Escape hatches — 4
Poison rollback (`production/mod.rs:630`); `SiblingFetch` (INC-I-143 D4, bounded by `SIBLING_FETCH_MAX`);
`wedge_escape.rs` FORK_GUARD retention (193 LOC, single fn `retain_sibling_and_try_escape`); `GenesisResync`.

### 3.6 Finality signals — 3 stores, 1 computation
`FinalityTracker` (`crates/core/src/finality.rs`, 438 LOC, `CONFIRMATION_DEPTH = 2`, fields `last_finalized`,
`pending`, `early_attestations`) — the computation.
`ReorgHandler.last_finality_height` (`reorg/mod.rs:229/234/242`) — a **cache** with its own setter/clearer.
`SyncManager::clear_finality_if_below_tip` (`block_lifecycle.rs:496`) — a forwarder.
Plus `SEED_CONFIRMATION_DEPTH = 6` (`constants.rs:241`) — a *second, different* depth for seeds.

### 3.7 Guard rails / thresholds — 8
All verified in `recovery.rs:220-245`: `MIN_MINOR_FORK_EVIDENCE = 2`, `MINOR_FORK_GAP_MAX = 50`,
`SNAP_SYNC_GAP_MIN = 500`, `SHALLOW_ROLLBACK_MAX = 10`, `SNAP_ATTEMPTS_MAX = 3`, `SNAP_MIN_PEERS = 3`,
`STALE_TIP_SECS = 300`, `SIBLING_FETCH_MAX = 3` (paced by `ACTION_COOLDOWN` 30s ≈ 90s of bounded fetching).
Plus `MAX_CUMULATIVE_ROLLBACK = 50` (`rollback.rs`, local const) and `MIN_PRODUCERS_FLOOR = 3`.

**Note on the absorbing state:** `SNAP_ATTEMPTS_MAX = 3` and `SNAP_MIN_PEERS = 3` are both small. A node that
burns 3 snap attempts is barred from Rule 2 **forever** (INV-SYNC-011: attempts never reset), which is the most
likely reason the INC-I-204 nodes did not escape once their gap grew past 500 (see §7.1).

### 3.8 Checkpoint / backup / repair — 6
`--auto-checkpoint N` (`bins/node/src/run.rs:175`, `config.rs:72`); Seed Guardian RPC checkpoint
(`scripts/seed-backup.sh`); block archiver (`crates/storage/src/archiver.rs`, 8 pub fns);
`backfillFromPeer` + `backfillStatus` + `verifyChainIntegrity` (`crates/rpc/src/methods/backfill.rs`);
snap sync (`snap_sync.rs`, 394 LOC); operator wipe+rsync (out-of-band, no code).

### 3.9 What the inventory licenses me to claim

- ✅ **Valid:** "the system has no *single* authority on canonical branch" — verified, 3 finality stores + 2
  reorg planners + a guard-bypassing rollback.
- ✅ **Valid:** "no rung of the ladder terminates in the 50..500 gap band without degradation" — verified §4.3.
- ❌ **Invalid and NOT claimed:** "the system lacks recovery mechanisms", "lacks checkpoints", "lacks a fork
  detector", "lacks finality". All exist. The problem is plurality and disagreement, not absence.

---

## 4. Architecture Context

### 4.1 The production error arm — `[REFINED]`

`bins/node/src/node/production/mod.rs:619-668`, read in full.

The summary claim *"BLOCK_POISON arm retracts an already-gossiped valid tip"* is **directionally right but
mechanically imprecise**, and the precision changes what a fix must do:

```
let block_hash = block.hash();
info!("[BLOCK_PRODUCED] ...");
match self.apply_block(block.clone(), ValidationMode::Light).await {
    Ok(()) => {}                    // ← "Success — proceed to broadcast"
    Err(e) => { ... rollback_one_block() ... return Ok(()); }
}
```

`apply_block` runs **before** broadcast. On the error path the function returns early, so **the failed block is
never gossiped by this node**. The block being retracted is therefore *not* the failed block — it is the
**parent**, i.e. the previously-applied, already-gossiped, possibly-finalized tip. This only bites when
`apply_block` rejects *before* mutating state (INC-I-147 D3: *"calls `rollback_one_block` on ANY error including
pre-mutation rejects"*), because then there is nothing to undo and the rollback consumes a healthy block.

**Why the precision matters:** a fix framed as "don't broadcast poisoned blocks" addresses nothing — the code
already doesn't. The correct frame is **"a production-side failure must not consume a rollback budget it did not
create"**, which is REQ-FORK-002.

### 4.2 Unit-mismatch inventory — the core defect

Every row is a place where **the same question is answered by different components using different data**.

| # | Question | Component A | Component B | Mismatch | Evidence |
|---|---|---|---|---|---|
| **U1** | *What height is this ancestor?* | `plan_reorg`: real chain height (post-AH) | `plan_reorg` pre-AH + `check_reorg_weighted`: `BlockWeight.height`, a **per-process counter** | `H_syn = H_real − I`, `I` = boot-dependent | INV-SYNC-012; measured: block at real 57067 recorded as **267** by seed, **25897** by n7, 5.6 ms apart |
| **U2** | *Am I allowed to roll back?* | `RecoveryCoordinator` ShallowRollback: finality-guarded, strict `<` | `rollback_one_block`: **"No preconditions"**, and *clears* finality (`rollback.rs:318`) | Two rollback doors, opposite guard regimes | §3.3, §4.1 |
| **U3** | *Where is finality?* | `FinalityTracker` (core) — computes | `ReorgHandler.last_finality_height` — caches, independently settable/clearable | Cache can be erased while the truth stands | INV-FINALITY-001: *"the finality marker was ERASED to None by the poison rollback... making both monotonicity demands vacuous"* |
| **U4** | *How deep is finality?* | `CONFIRMATION_DEPTH = 2` | `SEED_CONFIRMATION_DEPTH = 6` | Two depths, one chain | `finality.rs:19`, `constants.rs:241` |
| **U5** | *Who should I sync from?* | `best_peer`: `status.best_height > local_height` + blacklist | *(nothing checks branch/ancestry)* | **Height-only vs branch-aware** — a wedged node happily picks peers on its own fork | `decision.rs:81-108`, read in full |
| **U6** | *Is my tip canonical?* | production gate `AwaitingCanonicalBlock` (INV-CONSENSUS-089) | sync `StuckFork` evidence | Two independent detectors, no shared verdict | `types.rs:351`, `recovery.rs:104` |

`[VERIFIED]` U5, verbatim — the whole predicate:

```rust
status.best_height > self.local_height
    && !self.fork.header_blacklisted_peers.contains_key(pid)
```

No ancestry term, no "does this peer's chain contain my tip", no branch identity. Confirmed branch-blind.

**Synthesis.** U1-U6 are six instances of one structure: *no component owns the canonical-branch question, so
each recomputes it from whatever data is locally cheap.* Guards added by INC-I-081/147/190 each constrained **one
operand of one comparison**. The class survives because the *comparisons themselves* are unowned.

### 4.3 The recovery ladder is a cliff — `[VERIFIED]`

`crates/network/src/sync/manager/recovery.rs:384-484`, read in full.

```
StuckFork ∧ 0 < gap < 50 ∧ shallow_rollback_count < MAX
   ├─ ¬rollback_refused ────────────────► ShallowRollback{1}        (terminates)
   ├─ rollback_refused ∧ sibling < MAX ─► SiblingFetch{local_height} (terminates IFF sibling arrives)
   └─ sibling budget exhausted ─────────► fall through ↓
Rule 2: (rollback_exhausted ∨ gap ≥ 500 ∨ deep_fork_confirmed)
        ∧ snap_attempts < MAX ∧ peer_count ≥ MIN ──► SnapSync   (terminates, DESTROYS HISTORY)
Rule 3: gap>0 ∧ gap<500, or stale_tip ─────────────► HeaderFirstSync (does NOT terminate — see below)
Rule 4: apply_fails ≥ 5 ∧ snap_exhausted ─────────► GenesisResync
        else ─────────────────────────────────────► None
```

**The absorbing state.** A node with `tip == finality` (so rollback is correctly refused), `gap` in the band
**50 ≤ gap < 500**, and an exhausted sibling budget matches **no terminating rung**:

- ShallowRollback: excluded, `gap < 50` fails.
- Rule 2 `large_gap`: excluded, `gap < 500` fails. `deep_fork_confirmed` needs `gap ≥ 50` **and** `empty_count ≥ 10`
  **and** stale tip — reachable, but gated behind `snap_attempts < SNAP_ATTEMPTS_MAX`, and INV-SYNC-011 states
  *"snap.attempts is never reset by any admission or redirect path"* — so a node that already burned its 3 attempts
  is permanently excluded from Rule 2.
- Rule 3 fires (`medium_gap`) → `HeaderFirstSync` → issued against **branch-blind `best_peer`** (U5) → peers on the
  other branch return empty headers for the local tip hash → `consecutive_empty_headers` climbs → loop.
- Rule 4: needs `apply_fails ≥ 5`; a node that receives *nothing* applies nothing and fails nothing. Excluded.

The node's only exit is to **fall further behind** until `gap ≥ 500`, and even that is barred once snap attempts
are spent. This is exactly INC-I-190's measured path: *"stalled 52 slots, crossed the MINOR_FORK_GAP_MAX=50 cliff
and exited via emergency snap-sync door (b)"*, and it is consistent with INC-I-204's nine nodes sitting for seven
days (first refusal 2026-08-24T17:47:31Z; n6=43, n17=53, seed=32 refusals).

> **Structural statement of the class:** *recovery requires the node's condition to get **worse** before any rung
> terminates, and the only rung that reliably terminates **destroys archival history**.*

That single sentence is what a redesign must falsify. Every prior fix moved a threshold or added a guard *inside*
this shape; none changed the shape.

### 4.4 Why the INC-I-147 fix was dormant — `[VERIFIED]` arithmetic

`crates/core/src/network_params/defaults.rs`: mainnet `inc_i_147_activation_height = 129_500` (:260), testnet
`80_700` (:492), devnet `0` (:737).

- INC-I-204 wedged at **h = 77_777**. `77_777 < 80_700` → **pre-activation branch** → `plan_reorg` used the
  synthetic per-process height (`reorg/mod.rs:527-541`) → structural veto. The fix existed in the binary and was
  switched off by height.
- Mainnet tip ≈ **344_090** > 129_500 → **post-activation**. The U1 half of the INC-I-204 mechanism is **not armed
  on mainnet today**.

**Two consequences.** (1) INC-I-204 is *not* a straight mainnet risk via U1 — do not over-escalate. (2) The
generalizable defect is real and unfixed: **AH-gating creates a dormant window in which the old, known-broken
behavior is still the live behavior.** `check_reorg_weighted` (U1, second planner) is ungated everywhere.

### 4.5 Module boundaries and dependency direction

```
bins/node/src/node/
  production/mod.rs  ──apply_block──► apply_block/    ──┐
        │ :630 rollback_one_block (UNGUARDED)           │ owns state mutation
        ▼                                               │
  rollback.rs (434) ──clear_finality_if_below_tip──►    │
  block_handling.rs ── execute_reorg (own undo loop)    │
  fork_recovery.rs (778, 5 fns) ── apply_snap_snapshot ─┘
  wedge_escape.rs (193, 1 fn) ── FORK_GUARD retention
  periodic.rs:831 ── the OTHER rollback_one_block caller (ladder dispatch)
        ▲
        │ RecoveryAction
crates/network/src/sync/manager/
  recovery.rs (1299) ── classify → RecoveryAction        ← the ladder
  sync_engine/decision.rs ── best_peer (branch-blind), start_sync
  block_lifecycle.rs (finality forwarder), snap_sync.rs (394), cleanup.rs
crates/network/src/sync/reorg/mod.rs (622) ── plan_reorg + check_reorg_weighted + finality CACHE
crates/core/src/finality.rs (438) ── FinalityTracker (the actual computation)
```

**Direction:** `core` ← `network` ← `bins/node`. `bins/node` owns state mutation; `network` owns the recovery
decision; `core` owns the finality computation. **The pathology:** the finality *cache* and the reorg *planners*
live in `network`, while the finality *truth* lives in `core`, and the most consequential rollback lives in
`bins/node` and honours neither. Authority runs orthogonal to the dependency graph.

### 4.6 Blast radius (grep-derived; graph blind per §0)

**Direct — production callers of `rollback_one_block` (exactly 2):**
`bins/node/src/node/production/mod.rs:630` (poison arm, unguarded) and `bins/node/src/node/periodic.rs:831`
(ladder dispatch, guarded upstream). Every other hit in the tree is a test or a doc-comment — verified by
filtering to `bins/node/src` + `crates/*/src`.

**Indirect (consume rollback/reorg/finality outcomes):** `apply_block/` (dir), `block_handling.rs`,
`maintainer_rewind/` (INC-I-174 coupling), `crates/storage/src/block_store/writes.rs:199` (rolled-back-height
rule), `snapshot.rs` (state root), `rewards.rs:1218` (FORK_GUARD token), `metrics.rs:276-303`
(`FORK_GUARD_REFUSALS`, only `site="producer_rebuild"` instrumented), `crates/rpc/src/methods/backfill.rs`.

**Regression surface — `v_regression_map`, fork/sync invariant families:**

| Invariant | Tests | Canaries | | Invariant | Tests | Canaries |
|---|---|---|---|---|---|---|
| INV-SYNC-011 | 23 | 1 | | INV-SYNC-001 | 1 | 1 |
| INV-SYNC-008 | 4 | 1 | | INV-SYNC-002 | 1 | 1 |
| INV-SYNC-007 | 4 | 2 | | INV-SYNC-003 | 1 | **0** |
| INV-PROD-004 | 4 | 2 | | INV-SYNC-004 | 1 | **0** |
| INV-CONSENSUS-089 | 5 | 1 | | INV-SYNC-012 | 3 | **0** |
| INV-PROD-002 | 3 | 3 | | INV-SYNC-013 | 1 | **0** |
| INV-PROD-003 | 3 | **0** | | INV-SYNC-010 | 2 | **0** |
| INV-SYNC-015 | 3 | 1 | | **INV-SYNC-006** | **0** | 1 |
| INV-FORK-001 | 2 | 1 | | INV-FINALITY-001 | 1 | 1 |
| INV-EPOCH-005 | 2 | 1 | | INV-SYNC-014 | 1 | 1 |
| INV-SYNC-009 | 2 | 1 | | INV-PROD-001 | 1 | 2 |

**Coverage gaps that bound the redesign:** INV-SYNC-006 (chain continuity / gapped stores) has **zero regression
tests** and is exactly the invariant the snap-sync escape violates. INV-SYNC-012 (the unit-mismatch invariant, the
thesis of this redesign) has **zero canaries** — a regression would be silent in production. These are inputs to
REQ-FORK-010.

### 4.7 Brittleness check

```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 5/5
 1. Cross-module blast radius — YES. bins/node (production, rollback, block_handling,
    fork_recovery, wedge_escape) + crates/network (recovery, decision, reorg, snap_sync,
    block_lifecycle) + crates/core (finality). No direct dependency links the poison arm to
    the ladder that must compensate for it.
 2. Invariant gaps — YES. No module enforces "exactly one component decides the canonical
    branch". INV-SYNC-012 names the mixing but no owner enforces it; check_reorg_weighted
    still violates it.
 3. Data flow reversal — YES. Finality truth is computed in core, cached in network, and
    erased from bins/node (rollback.rs:318) — the erase flows opposite to the dependency
    direction.
 4. Shared mutable state — YES. last_finality_height has 3 writers across 3 crates
    (set_last_finality_height, clear_finality_if_below_tip ×2 sites) with no single owner.
 5. Contract absence — YES. production/mod.rs and the RecoveryCoordinator both invoke
    rollback with no shared contract about who may retract applied state; the guard regime
    is an accident of call site.
Verdict: BRITTLE
━━━━━━━━━━━━━━━━━━━━━━━━━
```

**5/5. This is architectural, not a code bug.** Consistent with the diagnosis synthesizer's routing to redesign.

---

## 5. Load-Bearing Behaviors — DO NOT "FIX" THESE

Behaviors that read as bugs in a log but are correct, deliberate, or currently indispensable.

| # | Behavior | Looks like | Actually is | Evidence |
|---|---|---|---|---|
| **LB-1** | FINALITY_GUARD refusing sub-finality ShallowRollback | The cause of INC-I-204's wedge | **Correct and mandatory.** Refusing is the *safe* outcome. The bug is the absence of a next rung, not the refusal. | INV-SYNC-001/004/008; code comment: *"Guard is UNCHANGED (strict `<`, INC-I-090) — do NOT loosen it."* |
| **LB-2** | Strict `<` (not `≤`) in the guard | Off-by-one | **Deliberate fencepost fix.** Rolling back *to* finality is legal; only *below* violates. A `≤` blocks legal 1-block forks. | INV-SYNC-008 (INC-I-090) |
| **LB-3** | AH-gating the INC-I-147 fix instead of activating at 0 | Cowardice; cause of the dormant window | **Correct INV-12 discipline.** Activating at 0 *"would reinterpret already-validated history under the new fork-choice rule."* Testnet AH pinned 80_700 at live tip 80_544 — a deliberate ~150-block lead. | INC-I-147 resolution; CLAUDE.md #0 rule |
| **LB-4** | Poison rollback bypassing the finality guard | The fork factory (INC-I-204) | **Currently the fleet's only wedge escape** (13/27 nodes, INC-I-190). Load-bearing *until* replaced — INV-PROD-002 forbids enforcing its removal first. | §2.3 |
| **LB-5** | Snap sync existing at all | The thing that destroys history | **Required** for genuinely-far-behind and fresh nodes. The defect is snap being reachable as a *fork* remedy, not its existence. | INV-SYNC-011/015 |
| **LB-6** | `snap.attempts` never reset by any admission path | A leak that permanently bars snap | **Deliberate.** Prevents unbounded snap retry loops. | INV-SYNC-011 (explicit) |
| **LB-7** | `MAX_CUMULATIVE_ROLLBACK = 50` + refuse-rollback-to-genesis | Arbitrary caps that block recovery | **Deliberate erosion guards** — stop cascading rollbacks eroding the chain to genesis. | `rollback.rs` Fix 3 / Fix 4 |
| **LB-8** | Producer refusing to build on a loaded-from-disk tip until a peer block arrives | Startup stall | **INV-CONSENSUS-089**, protection level 3 — prevents self-fork on restart (INC-I-089). | INV-CONSENSUS-089 |
| **LB-9** | `SiblingFetch` being non-destructive and bounded | A rung that does nothing | **INC-I-143 D4 fix.** Returning `None` there was the measured 454-refusal livelock on seed1. | `recovery.rs:397-415` |
| **LB-10** | `calculate_epoch_rewards` refusing on a gapped store, aborting the slot | A producer that stops producing | **INV-PROD-001.** Emitting the block instead is a consensus violation that cascades fleet-wide. | INC-I-081 |

**LB-4 is the one to watch.** It is the only load-bearing behavior that is *also* a root cause. It must be
**replaced, not removed**, and the replacement must land **first** (REQ-FORK-012).

---

## 6. Requirements

IDs `REQ-FORK-NNN`. MoSCoW. These define **what the redesign must satisfy** — they deliberately do **not**
specify a solution. Per SSF, the architect proposes ONE mechanism against these; this document does not present a
menu.

### Must

| ID | Requirement | Priority | Acceptance Criteria |
|---|---|---|---|
| **REQ-FORK-001** | **Behavior preservation — honest-majority convergence.** Under an honest majority with normal connectivity, every node converges to one canonical chain with bit-identical 3-state. | Must | - [ ] Given a healthy fleet, when N blocks are produced, then all nodes report identical `(height→stateRoot, csHash, psHash, utxoHash)`<br>- [ ] INV-SYNC-007 regression suite (4 tests) stays green<br>- [ ] Gauntlet passes ≥ its current baseline |
| **REQ-FORK-002** | **Error paths must not retract published state.** A production-side failure must never consume a rollback of a block it did not itself apply. | Must | - [x] Given `apply_block` returns Err **before** any state mutation, when the poison arm runs, then local tip is **unchanged**<br>- [x] Given the failed block mutated state, when the arm runs, then exactly that block is undone<br>- [x] `last_finality_height` is **never** cleared as a side effect of a production failure (INV-FINALITY-001)<br>- [x] FAIL→PASS reproduction test exists and fails before the change |
| **REQ-FORK-003** | **Finality never rolled back on mainnet post-AH.** No path may undo a block at or below the finalized height. | Must | - [ ] Strict `<` guard preserved verbatim (LB-1/LB-2)<br>- [ ] Every rollback entry point — including `rollback_one_block` — is subject to *one* finality decision<br>- [ ] INV-SYNC-001/004/008 + INV-FINALITY-001 regression tests green<br>- [ ] Given a finalized block, when any of the 4 rollback primitives is invoked below it, then it is refused and the refusal is counted |
| **REQ-FORK-004** | **Snap sync remains available for genuinely-far-behind and fresh nodes.** | Must | - [ ] Bootstrap (`h=0`) and genesis-window (door c) admission unchanged<br>- [ ] `gap ≥ SNAP_SYNC_GAP_MIN` admission for genuinely-behind nodes preserved<br>- [ ] INV-SYNC-011 (23 tests) + INV-SYNC-015 green |
| **REQ-FORK-005** | **No regression against the 13 active fork/sync invariants.** | Must | - [ ] Each of INV-SYNC-001/002/003/004/006/007/008/009/010/011/012/013/014/015, INV-FORK-001, INV-FINALITY-001, INV-PROD-001/002/003/004, INV-CONSENSUS-089, INV-EPOCH-005 is listed as *preserved*, *strengthened*, or *superseded-with-successor* — no silent drops<br>- [ ] Any superseded invariant gets a written successor before code lands |
| **REQ-FORK-006** | **Per-change consensus-visibility analysis (INV-12).** Every change answers the three questions in its commit. | Must | - [ ] For each change: Q1 user-submittable tx reaches path? Q2 producer-action/attestation reaches it? Q3 bit-identical for ALL reachable inputs?<br>- [ ] `(Q1∨Q2) ∧ ¬Q3` ⇒ activation height assigned, in `NetworkParams`, never a global const<br>- [ ] No crossed AH is moved (d.28); no `CURRENT_PROTOCOL_VERSION` bump unless EpochState format changes (d.27) |
| **REQ-FORK-007** | **Rolling-deploy compatible for node-local changes.** | Must | - [ ] Each change classified node-local vs block-content-visible<br>- [ ] Block-content changes ⇒ synchronized deploy (INV-8 / INC-I-062), stated explicitly<br>- [ ] Mixed-version fleet (old+new) converges in test — deploy must not itself fork the fleet, given ~30 external producers make stop-all impossible |
| **REQ-FORK-008** | **No wire-format break on frozen history.** | Must | - [ ] Any block/header/tx shape change is AH-gated **and** retains the old decoder permanently<br>- [ ] Given a pre-change block in history, when a new binary full-syncs from genesis, then it validates (both deploy directions)<br>- [ ] Explicitly checked against the INC-I-176 M2.5 failed approach |
| **REQ-FORK-012** | **Ordering constraint — escape before enforcement.** The poison-rollback bypass (LB-4) may not be closed until a replacement wedge-escape is live. | Must | - [ ] Replacement escape ships and is proven on testnet **before** any change enforcing INV-PROD-002<br>- [ ] Given a wedged node with `tip == finality`, when the replacement is active, then it recovers **without** snap sync and **without** the poison bypass<br>- [ ] Migration plan states the order explicitly |

### Should — the structural properties that would end the CLASS

| ID | Requirement | Priority | Acceptance Criteria |
|---|---|---|---|
| **REQ-FORK-009** | **One canonical-branch authority.** Exactly one component answers "what is my canonical branch, and where is finality". All others query it. | Should | - [ ] Enumerate every current answerer (§4.2 U1-U6) and show each either delegates or is deleted<br>- [ ] `last_finality_height` has exactly **one** writer<br>- [ ] `check_reorg_weighted` and `plan_reorg` consume the **same** height source (closes the INV-SYNC-012 known-remaining site)<br>- [ ] No comparison mixes a per-process counter with a chain-global quantity — enforced by a test, not a comment |
| **REQ-FORK-010** | **A recovery ladder whose every rung terminates.** No reachable (gap, finality, budget) combination yields a non-terminating action. | Should | - [ ] Exhaustive state-space enumeration over (gap band × `tip==finality` × each budget exhausted) with the terminating rung named for **every** cell<br>- [ ] The 50 ≤ gap < 500 ∧ `tip==finality` ∧ sibling-exhausted ∧ snap-exhausted cell has a **named, non-lossy** terminal (today: none — §4.3)<br>- [ ] No rung requires the node's condition to **degrade** to become eligible<br>- [ ] INV-SYNC-006 gains regression tests (currently **0**) |
| **REQ-FORK-011** | **Escapes that never cost archival history.** Fork recovery must not be a reason to discard blocks a node already holds. | Should | - [ ] Given a forked node with complete history, when it recovers, then no body hole is created (cf. INC-I-190's permanent 314592-314640 hole)<br>- [ ] Snap sync is reachable for *behind-ness*, not as a *fork* remedy<br>- [ ] `verifyChainIntegrity` reports no new gap ranges after a recovery drill |
| **REQ-FORK-013** | **Branch-aware peer selection.** Sync source choice must consider which branch a peer is on, not height alone. | Should | - [ ] `best_peer` (or successor) has an ancestry/branch predicate in addition to height<br>- [ ] Given a wedged node and peers split across two branches, when it selects a source, then it does not pick a peer that cannot serve headers connecting to canonical<br>- [ ] Does not regress INC-I-014 load distribution or INC-I-017 thundering-herd |
| **REQ-FORK-014** | **Dormant-window discipline.** AH-gating must not leave known-broken behavior as the live behavior for long unmonitored windows. | Should | - [ ] Each AH-gated change states its dormant window and what runs during it<br>- [ ] A monitoring signal fires if the pre-activation path is exercised on a live network (INC-I-204 sat in one for 7 days undetected)<br>- [ ] INV-SYNC-012 gains a canary (currently **0**) |
| **REQ-FORK-015** | **Non-foreclosure.** The simpler structure must not block the known evolution path nor re-create the dead-end being escaped. | Should | - [ ] Works at thousands of producers with 10s slots (CLAUDE.md Law 3)<br>- [ ] Works for ~30 external producers that cannot be stop-all coordinated<br>- [ ] Introduces no bound term advanceable by a ≥threshold holder (**filter F7**)<br>- [ ] Does not depend on `rebuild_epoch_state_from_blocks`, already ruled architecturally unsafe (d.29) |
| **REQ-FORK-016** | **Observability of the class.** A wedge must be visible before an operator notices it. | Should | - [ ] FINALITY_GUARD refusals exported beyond the single instrumented `site="producer_rebuild"`<br>- [ ] A sustained-refusal + non-advancing-tip condition raises an alert<br>- [ ] Given the INC-I-204 signature (n6=43, n17=53, seed=32 refusals over days), when replayed, then it alerts within minutes |

### Could

| ID | Requirement | Priority | Acceptance Criteria |
|---|---|---|---|
| **REQ-FORK-017** | Consolidate the 4 rollback primitives (§3.3) toward one guarded implementation. | Could | - [ ] Count of independent rollback code paths strictly decreases<br>- [ ] Undo/legacy-rebuild distinction preserved (INV-SYNC-015) |
| **REQ-FORK-018** | Reconcile the two finality depths (`CONFIRMATION_DEPTH=2` vs `SEED_CONFIRMATION_DEPTH=6`, U4). | Could | - [ ] Either justified as intentionally distinct, in writing, or unified<br>- [ ] Consensus-visibility assessed per REQ-FORK-006 |
| **REQ-FORK-019** | Retire `wedge_escape.rs` / `SiblingFetch` if REQ-FORK-010 makes them redundant. | Could | - [ ] Only after the ladder is proven terminating (subtraction, per d.35) |

### Won't (this iteration — bounded explicitly)

| ID | Excluded | Why |
|---|---|---|
| **REQ-FORK-020** | Consensus algorithm replacement (PoS→other, fork-choice rule replacement) | Blast radius exceeds the class; would invalidate all frozen history |
| **REQ-FORK-021** | BLS attestation verification redesign | INC-I-178 is its own track; active hotfix on whitepaper §10.3 claims |
| **REQ-FORK-022** | Oracle / DeFi subsystems | Frozen at `u64::MAX`; unrelated |
| **REQ-FORK-023** | Genesis reset of any network | CLAUDE.md #0 rule; forward-only activation is mandatory |
| **REQ-FORK-024** | Parameter re-tuning as the primary remedy (e.g. moving 50 or 500) | d.35 WONT-listed it explicitly; d.34 records 55+ failed patch attempts |
| **REQ-FORK-025** | Deploy-procedure hardening as the fork remedy | §2.5 — INC-I-190 was **not** deploy-caused; would fix a non-cause |
| **REQ-FORK-026** | Replacing `rebuild_epoch_state_from_blocks` (d.29) | Real and unsafe, but its own incident; must not be bundled |

### Traceability Matrix

| Requirement | Priority | Invariants touched | Test IDs | Architecture § | Impl module |
|---|---|---|---|---|---|
| REQ-FORK-001 | Must | INV-SYNC-007 | *(test-writer)* | *(architect)* | *(developer)* |
| REQ-FORK-002 | Must | INV-PROD-002, INV-FINALITY-001 | `bins/node/tests/it/inc_i_204_m42_poison_containment.rs` (11: `req_fork_002_ac1_a_*`, `req_fork_002_ac2_b_*`, `req_fork_002_ac2_b2_*`, `req_fork_002_c_*`, `req_fork_002_ac3_d_*`, `req_fork_002_ac3_e_*`, `req_fork_002_f_*`, `req_fork_002_g_*`, `req_fork_002_h_*`, `req_fork_002_i_*`, `req_fork_002_j_*`); `bins/node/tests/it/inc_i_204_m42_poison_contract_pins.rs` (3: `rollback_authority_carries_*`, `refused_not_authorized_is_distinct_*`, `poison_containment_outcomes_are_zero_initialised_*`) — 14 total, all RED pre-fix; plan in `docs/.workflow/inc-i-204-M4.2-test-plan.md` | M4.2 | `rollback_authority` @ `bins/node/src/node/rollback_authority.rs`; `poison` @ `bins/node/src/node/production/poison.rs`; `rollback` @ `bins/node/src/node/rollback.rs`; `metrics` @ `bins/node/src/metrics.rs` |
| REQ-FORK-003 | Must | INV-SYNC-001/004/008, INV-FINALITY-001 | | | |
| REQ-FORK-004 | Must | INV-SYNC-011/015 | | | |
| REQ-FORK-005 | Must | all 22 listed | | | |
| REQ-FORK-006 | Must | INV-12 (checklist) | | | |
| REQ-FORK-007 | Must | INV-8 | | | |
| REQ-FORK-008 | Must | — (failed-approach filter) | | | |
| REQ-FORK-009 | Should | INV-SYNC-012, INV-FINALITY-001 | | | |
| REQ-FORK-010 | Should | INV-FORK-001, INV-SYNC-006 | `tests_inc_i204_m3.rs` (4: `m3_incident_shape_*`, `c6_every_cell_*`, `c6_enumeration_is_not_vacuous_*`, `m3_wedged_is_exitable_*`); `tests_inc_i204_m3_rungs.rs` (5: `rung_rule1_*`, `rung_rule1b_*`, `rung_rule2_*`, `rung_rule3_*`, `rung_rule4_*`); `tests_inc_i204_m3_traps.rs` (13: `t1_*` x2, `t5_*`, `t6_*`, `t8_*`, `t11_*`, `c3_*` x2, `inv_sync_006_*` x2, `b_f1_*`, `m3_classify_*`, `m3_cell_fixtures_*`) — 22 total, 9 RED pre-fix; ledger in `docs/.workflow/test-plan-M3.md` | M3 | `crates/network/src/sync/manager/recovery.rs`, `bins/node/src/node/periodic.rs` |
| REQ-FORK-011 | Should | INV-SYNC-006/007 | | | |
| REQ-FORK-012 | Must | INV-PROD-002 (sequencing caveat) | | | |
| REQ-FORK-013 | Should | INV-SYNC-005 (deferred), INV-SYNC-009 | | | |
| REQ-FORK-014 | Should | INV-SYNC-012 | | | |
| REQ-FORK-015 | Should | — (filter F7) | | | |
| REQ-FORK-016 | Should | INV-FORK-001 | | | |

---

## 7. What I Don't Understand (mandatory disclosure)

Gaps in my understanding become gaps in requirements. Stated plainly:

1. **Why INC-I-204's nine nodes never escaped in seven days.** §4.3 derives an absorbing state that *fits*, but I
   did not measure those nodes' `snap_attempts`, `sibling_fetch_attempts`, or observed `gap` trajectory. The gap
   should have grown past 500 as the fleet advanced. The leading hypothesis is now sharper: `SNAP_ATTEMPTS_MAX = 3`
   and INV-SYNC-011 states attempts are **never reset**, so three failed snaps bar Rule 2 permanently — after
   which Rule 3 `HeaderFirstSync` loops forever against branch-blind `best_peer`. **This is the single
   highest-value measurement before design** and I flag it `[UNVERIFIED]`: I did not read those nodes' counters.
2. **Whether `check_reorg_weighted` is reachable on the wedge path.** INV-SYNC-012 says it measured **0 rejects**
   on all specimens vs 1017 for `plan_reorg`, i.e. off the path — but it is strictly worse code. I do not know if
   it is dead, rare, or merely unmeasured in that window.
3. **The real cost of branch-aware peer selection** (REQ-FORK-013). Ancestry checks may need extra round trips;
   I have not modelled CPU/mem/IO, which CLAUDE.md requires before any proposal is approved.
4. **Whether one canonical-branch authority is achievable without a consensus-visible change.** REQ-FORK-009 is
   filed as Should precisely because I cannot yet answer this. If unifying the height source changes fork choice
   for any reachable input, it needs an AH — and then LB-3's dormant-window problem recurs.
5. **The interaction with `maintainer_rewind`** (INC-I-174). It couples to both rollback paths; I read only its
   doc-comments.
6. **Whether the finality *cache* can simply be deleted** in favour of querying `FinalityTracker` directly. That
   would collapse U3 outright, but I do not know why the cache exists — performance, or async-boundary necessity.

I did not read `specs/network-recovery-architecture.md` or `specs/sync-snap-admission-architecture.md` in full
(context budget). Both exist and are relevant; the architect should treat them as required reading and I may have
duplicated or contradicted them.

---

## 8. Specs Drift Detected

| Location | Claim | Reality | Action |
|---|---|---|---|
| `CLAUDE.md` Map — Code | *"fork recovery (**9 functions**)"* → `fork_recovery.rs` | **5** functions: `handle_completed_fork_recovery`, `try_trigger_fork_recovery`, `try_apply_cached_chain`, `apply_snap_snapshot`, `try_apply_direct_successor` | Correct to 5 |
| `CLAUDE.md` Map — Code | `fork recovery integration tests (11)` / `rollback_one_block()` at `node/rollback.rs` | Path correct; `rollback.rs` is 434 LOC | No action |
| `docs/.workflow/` | Redesign mandate cites 6 upstream artifacts | 4 of 6 absent (§0) | Regenerate or drop the references |
| `INV-SYNC-005` | Status `deferred-revision` since 2026-05-18 | Still deferred; snap-source canonical filter never landed. Directly relevant to REQ-FORK-013 | Re-open with the reframe already written in the invariant |

---

## 9. Assumptions

| # | Assumption (technical) | Plain language | Confirmed |
|---|---|---|---|
| 1 | INC-I-204's `[E1]`-`[E7]` evidence chain exists somewhere I could not read | The detailed evidence behind the verdict is missing from disk; I rebuilt what I could from code + DB | **No** — needs user |
| 2 | Mainnet tip ≈ 344,090 (from prompt-refinement) is current | Mainnet is past the INC-I-147 activation height, so that specific veto is not armed there | **No** — not re-measured (read-only, no mainnet SSH) |
| 3 | The 13 nodes that escaped in INC-I-190 did so *solely* via the poison bypass | The escape hatch reading rests on the incident record, not my own measurement | **No** — from DB record |
| 4 | `SHALLOW_ROLLBACK_MAX=10`, `SIBLING_FETCH_MAX=3`, `SNAP_ATTEMPTS_MAX=3`, `SNAP_MIN_PEERS=3`, `STALE_TIP_SECS=300`, `MIN_MINOR_FORK_EVIDENCE=2` | The exact recovery budgets | **Yes** — read from `recovery.rs:220-245` |
| 5 | No fork-lifecycle code changed between my read and the design session | Analysis is a snapshot of `fix/inc-i-196-trust-root-unbrick` | Yes (read-only session) |

---

## 10. Identified Risks

| Risk | Mitigation |
|---|---|
| **Removing the poison bypass before a replacement escape ships** converts poison events into fleet-wide wedges escapable only via history-destroying snap sync | REQ-FORK-012 makes ordering a Must; INV-PROD-002 already forbids it |
| **A third structural simplification of the sync manager** repeats two prior rounds that did not end the class | §1 calibration; the lever must be *authority consolidation* (REQ-FORK-009), not field/state reduction |
| **AH-gating the fix re-creates a dormant window** — exactly how INC-I-204 got in below testnet 80_700 | REQ-FORK-014 (dormant-window discipline + canary) |
| **Consensus-visible change deployed rolling** forks the fleet mid-deploy | REQ-FORK-006/007; ~30 external producers make stop-all impossible |
| **Wire-format change breaks frozen history in both directions** | REQ-FORK-008; INC-I-176 M2.5 precedent |
| **INV-SYNC-006 has zero regression tests** and is the invariant the snap escape violates | REQ-FORK-010 requires tests before change |
| **Scope creep into BLS / consensus / oracle** | REQ-FORK-020/021/022 Won't-listed |
| **`rebuild_epoch_state_from_blocks` (d.29, unsafe) sits under the snap escape** | REQ-FORK-026 routes it out; REQ-FORK-015 forbids depending on it |

---

## 11. Open Questions for the User

Ordered by how much they change the design. **Q1 and Q2 are blocking.**

1. **[BLOCKING] Should we measure the nine wedged testnet nodes before designing?** §7.1 — their `snap_attempts`,
   `sibling_fetch_attempts` and gap trajectory would confirm or refute the absorbing-state derivation that
   REQ-FORK-010 is built on. They appear to still be wedged (read-only RPC, no restart). **Designing without this
   risks building against a hypothesis.**
2. **[BLOCKING] Is "one canonical-branch authority" (REQ-FORK-009) in scope, or is the goal narrower — make the
   ladder terminate (REQ-FORK-010) and leave the plural authorities alone?** This is the single biggest scope
   fork. §4.2 argues the plural authority *is* the class; a ladder-only fix is smaller, rolling-safe, and likely
   AH-free, but on the evidence would leave the class alive.
3. **The four missing upstream artifacts (§0)** — regenerate the diagnosis/evidence chain, or proceed on this
   document plus the DB record?
4. **Is a permanent body hole ever acceptable?** REQ-FORK-011 assumes no. If archival completeness is negotiable
   for non-archival nodes, the solution space widens considerably.
5. **May the redesign consume an activation height on mainnet?** Pinning one is a separate decision session per
   HC-6 / INC-I-075. If the answer is "no AH this cycle", every Must must be satisfiable node-locally.
6. **Confirm the correction in §2.5** — the deploy did **not** cause INC-I-190. Your framing named the deploy as
   the cause. If you have evidence the DB lacks, it changes REQ-FORK-025 from Won't to in-scope.

---

## 12. Summary (plain language)

Forks are not an occasional bug in DOLI; they are 56% of everything that has ever gone wrong. Four fixes in six
months each added a *guard*, and none removed an *authority* — so today several different parts of the code each
decide, from different data, what the true chain is. They disagree, and the disagreement is the fork.

Recovery has the same shape problem. There is a ladder of six recovery actions, but in a specific and reachable
situation — the node is exactly at its finalized height, moderately behind, and has used its cheap options — no
rung applies. The node's only way out is to fall *further* behind until it qualifies for the emergency option,
and that option throws away chain history permanently. Recovery requires getting worse first, and the only
reliable exit is lossy.

One thing must not be rushed. The piece of code most responsible for *creating* forks is also the only thing that
*rescued* half the fleet last time. It has to be replaced, in that order — never simply deleted.

---

*Analyst scoping only. No solution is proposed here by design (SSF): the architect proposes ONE mechanism against
these requirements.*

# INC-I-136 — Guardian Checkpoint System Defects (must-fix before it can be trusted)

**Status:** F1–F6 **IMPLEMENTED & TESTED** (RUN 446, 2026-07-07, uncommitted pending review). F7 deferred to a separate ticket. See §7 Implementation Status.
**Severity:** Critical (the guardian failed to provide a usable recovery point during the exact failure mode it exists for).
**Discovered:** 2026-06-30 → 2026-07-07, during a mainnet fleet halt + multi-way chain shatter.
**Outcome of the incident:** Recovery from local node state was impossible. Decision taken: **restart from genesis**. This document exists so the guardian is fixed to 100% before the next chain accumulates value.

> One-line summary: **The guardian is a periodic RocksDB snapshotter with a cosmetic health label. It does not verify that the state it snapshots is healthy, complete, or canonical — so during an isolation/fork event it produces a rolling series of unusable checkpoints and rotates away the last good one.**

---

## 1. What we expected vs. what happened

**Expectation (per the guardian skill & key principles):** "Auto-checkpoint is the safety net… `last_healthy_checkpoint` answers *what was the last known-good state?*" We expected to `getGuardianStatus` → read `last_healthy_checkpoint` → restore → recover.

**Reality during the incident:**
- All 3 mainnet seeds were down. Every surviving checkpoint on every seed was tagged `healthy=false`, `peers=0`, `peers_agreeing=0`.
- The checkpoints that survived were all captured *inside* the corrupt/frozen window; the last pre-incident good state had been rotated out.
- The surviving checkpoints could not even serve their own tip's blocks (missing bodies) and could not be rolled back (missing undo data).
- On restore, the UTXO set doubled in memory, making every `getStateRootDebug` comparison meaningless.

Net: the guardian produced **zero** usable recovery anchors for the situation it is specifically designed to cover.

---

## 2. Evidence (from the live incident)

### 2.1 All checkpoints unhealthy, isolated
Checkpoint `health.json` across all 3 seeds:
```
ai1 seed: h487115  hash=6476b744…  healthy=False  peers=0  agreeing=0   (×5, all same)
ai2 seed: h487118  hash=ac17037b…  healthy=False  peers=0  agreeing=0   (×4, latest)
          h487120  hash=345381f9…  healthy=False  peers=0  agreeing=0   (×1, oldest / pre-freeze)
ai3 seed: h487109  hash=7b33a201…  healthy=False  peers=0  agreeing=0   (×5, all same)
```
`healthy=false` on **every** checkpoint, driven purely by `peers=0`.

### 2.2 Retention rotated away the good state
Each seed keeps only the last 5 checkpoints. During a multi-day freeze all 5 slots filled with frozen-window snapshots. Only seed2's **oldest** checkpoint (`h487120`, dated Jun 26, pre-freeze) survived — everything newer was corrupt.

### 2.3 Checkpoints captured an incomplete block store
On restore + boot, both seed2 checkpoints logged:
```
[STARTUP] Body gap at h=487116 (tip=487118). Undoing 19 blocks to h=487099.
[STARTUP] No undo data at h=487117 — rebuilding UTXO set from state_db (avoiding partial mutation leak)
```
The block bodies for ~487100–487120 were absent from **both the checkpoint store and the archive**, and no undo data existed to roll back cleanly. A checkpoint that cannot serve its tip and cannot roll back is not a recovery point.

### 2.4 UTXO rebuild doubled the set
```
init:              [UTXO] state_db-backed UTXO set: 32888 entries      (sane, persisted)
after rebuild:     getStateRootDebug → utxoCount = 65776               (= 2 × 32888)
```
Observed identically on every node that hit the rebuild path (32886→…, 32890→…, all ~2×). This corrupts `stateRoot`/`utxoHash`/`utxoCount`, which is exactly the data the documented recovery gate (`getStateRootDebug` cross-seed convergence) depends on.

### 2.5 The archive — not the guardian — was the only trustworthy record
The immutable archive (`--archive-to`, append-only `.block` + `.blake3`) agreed cross-seed down to ~486900; the RocksDB stores diverged/were corrupt much lower. Recovery intelligence came from the archive, not from any guardian checkpoint.

---

## 3. Root-cause defects (each is independently fixable)

Code reference: `bins/node/src/node/periodic.rs` (auto-checkpoint block, ~lines 940–1030).

### DEFECT 1 — Checkpoint creation is NOT health-gated
Creation fires on a pure height cadence:
```rust
if current_height >= self.last_checkpoint_height + interval { … create … }   // line ~944
```
`healthy` is computed *after* creation and written to `health.json` as metadata only (lines ~980–999). Nothing prevents snapshotting a forked/gappy/corrupt state.

### DEFECT 2 — The health signal is unusable exactly when needed
```rust
let point_healthy = peer_count > 0 && peers_agreeing == peer_count && unique_hashes <= 1;  // line ~981
let healthy = point_healthy || window_healthy;
```
An **isolated** seed has `peer_count == 0` → `point_healthy` is impossible, and after ~10 min of isolation `window_healthy` is false too → **every** checkpoint is tagged unhealthy regardless of whether the underlying state is actually correct. Isolation is the primary trigger for needing recovery, and it is precisely when the health tag goes dark. `last_healthy_checkpoint` therefore returns nothing.

### DEFECT 3 — Rotation can evict the last good checkpoint
```rust
if dirs.len() > 5 { for old in &dirs[..dirs.len()-5] { remove_dir_all(old.path()); } }  // line ~1024
```
Keep-last-5-by-height, with no protection for the last **healthy** checkpoint. A sustained incident (hours–days) overwrites every pre-incident anchor.

### DEFECT 4 — No block-store completeness / undo-data validation at checkpoint time
The snapshot copies whatever RocksDB currently holds. It never asserts (a) contiguous block bodies up to tip, or (b) undo data present for at least the rollback window. Result: "checkpoints" that cannot serve their tip or roll back (§2.3).

### DEFECT 5 — UTXO rebuild-from-state_db doubles the in-memory count
(Not in `periodic.rs` — in the startup rebuild path, `bins/node/src/node/init.rs`, the "No undo data → rebuilding UTXO set from state_db" branch.)

**Corrected root cause (ground-truthed during INC-I-136 analysis — the original "appends instead of replacing" was directionally right but mechanistically wrong):** The RocksDB data is never duplicated. The `utxo_count` is an in-memory `AtomicU64` shadow counter (initialized by scanning `cf_utxo` on open). The rebuild branch runs `utxo_set.clear()` — which is a **no-op for the RocksDb variant** (`crates/storage/src/utxo/set.rs`) — then re-inserts every existing UTXO via `state_db.iter_utxos()` → `insert_utxo()`. `insert_utxo()` did `utxo_count.fetch_add(1)` **unconditionally**, even on an upsert of an existing key (`crates/storage/src/state_db/writes.rs`). Re-inserting all N existing keys therefore added N to the counter: `N → 2N` (observed 32888 → 65776). The RocksDB keys are correct; only the atomic counter — and thus `utxo_len()`, `getStateRootDebug.utxoCount`, and the recovery convergence gate — is poisoned. **Fix targets the counter (make `insert_utxo` idempotent on upsert) + skips the pointless rebuild loop when RocksDb-backed.**

---

## 4. Required fixes (definition of "100% proper")

| ID | Fix | Acceptance criterion |
|----|-----|----------------------|
| **F1** | **Gate creation on a validated-state predicate**, not just tag it. Only mark a checkpoint `healthy` when state is provably good; optionally still snapshot unhealthy ones but never treat them as anchors. | An isolated-but-internally-consistent node can still produce a `healthy` checkpoint; a forked/gappy node cannot. |
| **F2** | **Peer-independent validity signal.** Health must not depend solely on live peers. Add self-checks: block-store contiguity to tip, undo data present for rollback window, epoch-state loads, state-root self-consistent. Distinguish `isolated` (peer agreement unknown) from `forked/corrupt` (known bad). | `healthy=true` achievable with `peers=0` when state is self-consistent. |
| **F3** | **Immunize the last-known-healthy checkpoint(s) from rotation.** Track healthy and unhealthy checkpoints separately; never `remove_dir_all` the most recent healthy one, even if outside the last-5 window. | After an arbitrarily long incident, the last pre-incident healthy checkpoint still exists on disk. |
| **F4** | **Validate block-store completeness + undo-data at checkpoint time.** Refuse to tag `healthy` (and log loudly) if bodies are non-contiguous to tip or undo data is missing for the rollback window. | No `healthy` checkpoint can exist that fails to serve its own tip or roll back. |
| **F5** | **Deeper retention horizon.** Keep last-5 recent **plus** a spaced set (e.g. 1/day for N days) **plus** last-known-healthy. | A multi-day incident cannot rotate away all pre-incident anchors. |
| **F6** | **Fix the UTXO rebuild doubling.** The rebuild path must replace/clear the in-memory set before repopulating, with a post-rebuild assertion `in_memory_count == state_db_persisted_count`. | After the rebuild path fires, `getStateRootDebug.utxoCount` equals the init-time persisted count; state root is valid. |
| **F7** | **Archive as first-class recovery source.** Since no clean state snapshot is guaranteed to survive, guardian recovery tooling must be able to (a) treat the immutable archive (`.block`+`.blake3`) as the canonical-block source of truth, and (b) reconstruct state by replaying genesis→anchor from the archive. | Documented, tested "reconstruct state from archive to height H" path. |

---

## 5. Design principle to encode

The guardian must move from **"periodically snapshot whatever is on disk and label it"** to **"only vouch for a recovery anchor whose state is validated as complete, self-consistent, and — where determinable — canonical; and never let rotation destroy the last vouched anchor."**

A checkpoint's job is to answer *"restore me to a state I can prove is good."* Until F1–F6 land, the guardian cannot answer that during the failure modes (isolation, fork, freeze) it is meant to protect against. F7 is the backstop for when even a good checkpoint doesn't survive.

---

## 6. Related / follow-on tickets

- **F6** (UTXO rebuild doubling) is a standalone binary bug worth its own regression test even independent of the guardian.
- Cross-reference lessons in `.claude/skills/guardian/reference/procedures.md` (Procedures 2/2c and key principles #15, #25 already warn that `healthy:true` is peer-dependent and that cross-seed `getStateRootDebug` convergence is mandatory — this incident is the empirical proof and the basis for F1–F4).
- Add regression tests: (a) isolated node produces a healthy checkpoint (F1/F2); (b) rotation preserves last healthy across >5 newer unhealthy (F3); (c) checkpoint with a body gap is never tagged healthy (F4); (d) rebuild-from-state_db count parity (F6).

---

## 7. Implementation Status (RUN 446, 2026-07-07)

Implemented via `/omega-doctor --incident=INC-I-136`, FAST path (diagnosis complete), TDD. **Uncommitted — pending review.** No consensus rules / block validation / wire protocol touched → no activation height, standard binary upgrade.

| Fix | Milestone | Status | Where | Test evidence |
|-----|-----------|--------|-------|---------------|
| **F6** UTXO rebuild counter doubling | M1 | ✅ Done | `state_db/writes.rs` (`insert_utxo` idempotent on upsert), `utxo/set.rs` (`is_rocksdb`), `init.rs` (skip rebuild loop when RocksDb-backed) | `storage::state_db::tests::test_m1_*` (6) |
| **F1+F2** Health-gate + peer-independent validity | M2 | ✅ Done | `checkpoint_health.rs` (`decide_checkpoint_health`), wired in `periodic.rs`; `health.json` gains `isolated`+`self_consistent` | `checkpoint_health` unit (14) + `inc_i_136_checkpoint_health_test` (14) |
| **F4** Block-store completeness / undo validation | M2 | ✅ Done | `block_store/queries.rs` (`has_contiguous_bodies`), `state_db/undo.rs` (`has_undo_data`); self_consistent = contiguity ∧ undo over `UNDO_KEEP_DEPTH`(100) window | `storage test_m2_contiguous` (7) + `test_m2_undo` (7) |
| **F3+F5** Rotation immunity + deeper retention | M3 | ✅ Done | `checkpoint_health.rs` (`select_checkpoint_evictions` — keep top-5 ∪ most-recent healthy), wired in `periodic.rs` rotation (reads each `health.json`) | `checkpoint_health::tests::test_m3_*` (7) |
| **F7** Archive-based state reconstruction | — | ⛔ Deferred | Feature, not bugfix; depends on F1–F6; needs its own ticket | — |

**Verification:** storage lib 245/245, doli-node lib 52/52, integration 14/14 — all green; `cargo build`/`clippy -D warnings`/`fmt --check` clean. Full analysis: `docs/bugfixes/inc-i-136-analysis.md`.

**Remaining follow-ups (not blocking):** (a) `self_consistent` currently checks contiguity+undo over the rollback window; adding an explicit "epoch state loads" assertion is noted as `TODO(INC-I-136 F4)` in `periodic.rs`. (b) F7 archive-reconstruction ticket. (c) A new spec home for the checkpoint subsystem (none exists today).

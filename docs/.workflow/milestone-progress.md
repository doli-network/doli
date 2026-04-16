# Milestone Progress — INC-I-034 Redesign Implementation

**RUN_ID**: redesign-fix-inc-i-034-2026-04-16
**Branch**: synmgrefactor
**Spec**: `specs/scheduler-state-architecture.md`
**Locked decisions**: SAME HF / RUNTIME PERIODIC / SHRINK after audit

---

## ⚠️ REFRESHED against HEAD 2026-04-16 (supersedes original 23-milestone plan)

The original milestone list was generated from the synthesis spec, which captured pre-deletion state from the analyst's inventory. Reconciliation against HEAD (commits on `synmgrefactor`) reveals most INC-I-035 Phase 1 work is ALREADY DONE. The actual pending work is concentrated on the 4 cross-boundary RCs that caused today's cascade (RC#9-12) plus the 3 locked-choice deliverables.

### Already completed on `synmgrefactor` (do NOT redo)

| Prior M# | Commit | What it did |
|---|---|---|
| M1 EpochState struct | `42740269` | Unified 7 scattered epoch fields |
| M2/M4/M5/M7 follow-up | `3d267217` | Accumulator persistence, cached_scheduler and dead-method removal |
| M5 cached_scheduler deletion | `4331d797` | 9 lines, pure deletion, conf 0.85 measured |
| M6 excluded_producers deletion | `81a3d6f7` | Vestigial field + 8 coupling sites |
| atomic_replace preserves scheduler meta | `1725fcc4` | RC#3 addressed |
| Rollback rebuilds scheduler state | `f677e9b5` | RC#7 addressed |
| Snap sync attestation filter | `674f066e` | RC#4 filter portion addressed |
| compute_scheduler_root observability | `557afc6d` | Divergence detection (NOT state-root inclusion yet) |
| Recovery Coordinator phase 1+2 | `0b685137`, `a4b3dedc` | New sync arch, shadow-integrated |
| Discriminate "behind" vs "forked" | `42fe7982` | **Partial RC#12 fix** |
| EpochState audit cleanup | `47cb679f`, `db545880` | 6 audit findings, accumulator persistence per block |

### ACTUALLY PENDING — the real remaining work

| ID | Name | File:line | LOC | Confidence | Status |
|----|------|-----------|-----|-----------|--------|
| **M-RC9** | Fix silent `vec![]` trigger | `bins/node/src/node/rewards.rs:55-63` | -9 +30 | conf(0.85, measured by today's cascade) | **IN PROGRESS** (first) |
| M-RC10 | Eliminate apply-after-reject | `bins/node/src/node/block_handling.rs:260` | -15 +20 | conf(0.85, measured) | PENDING (auto-continue) |
| M-RC11 | FORK_GUARD with backfill | `bins/node/src/node/block_handling.rs:90` (FORK_GUARD log) and `block_handling.rs:311` (`execute_reorg`, the actual switch site); bug at `block_handling.rs:399-413` (`unwrap_or(genesis_hash)`) | -10 +40 | conf(0.85, measured) | TEST WRITTEN (FAIL on HEAD) — `bins/node/tests/m_rc11_fork_guard_backfill_regression.rs::test_b_deeper_reorg_with_missing_ancestor_preserves_invariant` |
| M-RC12-full | Complete asymmetric blacklist (42fe7982 was partial) | `crates/network/src/sync/manager/sync_engine/response.rs:222-344` | -20 +35 | conf(0.7, partial fix exists) | PENDING |
| M-Choice1 | State-root inclusion of EpochState (HF gated) | `crates/storage/src/snapshot.rs:22-57` + `crates/updater/src/hardfork.rs` | +40 | conf(0.7) | PENDING |
| M-Choice2 | RUNTIME PERIODIC integrity check (background task) | `bins/node/src/node/periodic.rs` | +60 | conf(0.7) | PENDING |
| M-Choice3 | `doli-cli repair-chain` subcommand | `bins/cli/src/cmd_chain.rs` | +80 | conf(0.65) | COMPLETE (local, pending commit) — 15/15 unit tests pass, see `docs/bugfixes/inc-i-034-m-choice3-chain-repair.md` |

**Net remaining scope**: ~+205 LOC added, -54 LOC removed. Much smaller than the original 23-milestone plan because INC-I-035 work already landed.

### Critical path to stop the cascade

Fixing **M-RC9 alone** stops the cascade structurally (no producer can emit a wrong-count block). Adding M-RC10 + M-RC11 closes the amplifiers. That's the minimum viable trigger-removal trio — ~60 LOC net delta, one session of focused work.

State-root inclusion (M-Choice1) is the SAFETY NET — it makes divergence detectable at block apply time. Deploy after the trigger-removal trio passes testnet validation.

---

## Original 23-milestone plan (historical — superseded, preserved for reference)

---

## Phase 1 — Pre-activation (no consensus change, additive only)

Phase 1 ships in one release. All milestones are safe to deploy without HardForkSchedule activation. Cross-version 6.13.28 / 6.13.29-synmgr coexistence is preserved.

| ID | Name | Modules | Type | LOC delta | Confidence | Status |
|----|------|---------|------|-----------|-----------|--------|
| M1 | Add `EpochSnapshot` struct + `CF_EPOCH_SNAPSHOTS` column family | `crates/storage/src/epoch_snapshot.rs` (new), `crates/storage/src/state_db/keys.rs` | additive | +150 | conf(0.75, converged) | PENDING |
| M2 | Shadow-mode compute at epoch boundary | `bins/node/src/node/apply_block/post_commit.rs` | additive | +50 | conf(0.75, converged) | PENDING |
| M3 | Add `EpochSlice`, `BlockOutcome`, `BlacklistDecision` types (no behavior change) | `crates/core/src/epoch_slice.rs` (new), `crates/network/src/sync/types.rs` | additive | +60 | conf(0.7, converged) | PENDING |
| M4 | Wire-format extensions (Optional fields, backward-compatible) | `crates/network/src/sync/manager/snap_sync.rs`, `crates/network/src/protocols/status.rs` (CURRENT_PROTOCOL_VERSION bump) | additive | +30 | conf(0.7, converged) | PENDING |
| **M5** | **Delete `cached_scheduler` field + 8 sites** | `bins/node/src/node/mod.rs`, 8 caller sites | **PURE DELETION** | **−9** | **conf(0.85, measured)** | **PENDING — RECOMMENDED FIRST** |
| M6 | Delete `excluded_producers` field + coupling sites + `rebuild_excluded_from_headers()` | `bins/node/src/node/mod.rs`, `bins/node/src/node/apply_block/post_commit.rs`, others | PURE DELETION | −120 | conf(0.8, measured) | PENDING |
| M7 | Delete dead convenience methods | `bins/node/src/node/mod.rs` (height, best_hash, save_state, state_reset_recovery, is_active_producer, last_active_status_epoch) | PURE DELETION | −70 | conf(0.7, measured) | PENDING |
| M8 | Ship `doli-cli repair-chain` command | `bins/cli/src/cmd_chain.rs` (new subcommand) | additive | +80 | conf(0.65, inferred) | PENDING |
| M9 | Add `HardForkSchedule::EPOCH_SNAPSHOT_HF` entry (activation height TBD by ops) | `crates/updater/src/hardfork.rs` | additive | +20 | conf(0.7, converged) | PENDING |

**Phase 1 net**: ~+390 LOC, **-199 LOC**, **+191 LOC net**. Reversible. Deployable on testnet then mainnet without HF coordination.

---

## Phase 2 — HF Activation (consensus-changing — REQUIRES TESTNET VALIDATION FIRST)

Phase 2 ships in one release with `HardForkSchedule::EPOCH_SNAPSHOT_HF` activated at a future epoch boundary ≥2h after fleet upgrade verification. **All Phase 2 milestones bundle into ONE atomic activation.** Pre-HF peers partition off at activation.

| ID | Name | Modules | LOC delta | Status |
|----|------|---------|-----------|--------|
| M10 | State-root format: include `H(EpochSnapshot)` | `crates/storage/src/snapshot.rs:22-57` | +20 | PENDING (Phase 2) |
| M11 | W1-only writer + `producer_at()` pure function | `bins/node/src/node/production/scheduling.rs:457-461`, `crates/core/src/validation/producer.rs:272-298` | -30 +40 | PENDING (Phase 2) |
| M12 | `rewards.rs` rewrite — read `qualifier_set`, no block_store scan | `bins/node/src/node/rewards.rs:39-200, 464-621, 525-542, 55-63` | -250 +60 | PENDING (Phase 2) |
| M13 | Block-store invariant + RUNTIME PERIODIC integrity check | `bins/node/src/node/periodic.rs`, new background task | +60 | PENDING (Phase 2) |
| M14 | FORK_GUARD with backfill — `advance_chain_state` gated on `ensure_blocks_present()` | `bins/node/src/node/fork_recovery.rs`, `bins/node/src/node/block_handling.rs` | -10 +30 | PENDING (Phase 2) |
| M15 | `BlockOutcome` refactor of `block_handling.rs` (apply/reject atomicity via type-state) | `bins/node/src/node/block_handling.rs` | -100 +60 | PENDING (Phase 2) |
| M16 | `BlacklistDecision` asymmetric in `sync_engine/response.rs` | `crates/network/src/sync_engine/response.rs` | -20 +35 | PENDING (Phase 2) |
| M17 | Snap-sync payload: `EpochSnapshot` becomes required | `crates/network/src/sync/manager/snap_sync.rs`, `bins/node/src/node/fork_recovery.rs:772-777` (delete W4 RAM-only) | -30 +20 | PENDING (Phase 2) |
| M18 | `atomic_replace()` nuclear branch deletion + surgical writes | `crates/storage/src/state_db/writes.rs:130-217`, all callers | -25 +15 | PENDING (Phase 2) |
| M19 | Sanity halt (one epoch dual-read divergence check) | `bins/node/src/node/apply_block/post_commit.rs` | +25 | PENDING (Phase 2) |

**Phase 2 net**: -465 LOC, +365 LOC, **-100 LOC**. Consensus-changing. Requires testnet pass + fleet upgrade window.

---

## Phase 3 — Post-activation cleanup (no consensus change, deletions only)

Phase 3 ships ≥1 release after Phase 2 activation, after Phase 1 shadow-mode legacy paths confirmed unused for ≥3 epochs.

| ID | Name | Modules | LOC delta | Status |
|----|------|---------|-----------|--------|
| M20 | Delete shadow-mode legacy paths (rebuild_epoch_state_from_blocks body, W9 auto-heal, W11 startup fallback) | `bins/node/src/node/rewards.rs:464-636`, `bins/node/src/node/block_handling.rs:269-272`, `bins/node/src/node/init.rs:784-801` | -195 | PENDING (Phase 3) |
| M21 | Delete CF_META scheduler keys (META_EPOCH_PRODUCER_LIST, META_ACTIVE_PRODUCTION_LIST, META_EPOCH_ATTESTED_SET, META_EPOCH_ATTESTATION_ACCUM, META_EPOCH_BLOCKS_PRODUCED, META_EPOCH_BOND_SNAPSHOT) | `crates/storage/src/state_db/keys.rs`, all readers | -40 | PENDING (Phase 3) |
| M22 | Delete Phase 2 sanity-check halt code | `bins/node/src/node/apply_block/post_commit.rs` | -25 | PENDING (Phase 3) |
| M23 | RPC-consumer audit + DeterministicScheduler shrink (605 → 50 lines) | `crates/core/src/scheduler.rs`, RPC consumers | -555 +50 | PENDING (Phase 3, gated on audit) |

**Phase 3 net**: **-815 LOC**. Pure cleanup.

---

## Cumulative

- Phase 1: net **+191 LOC** (additive — building the new infrastructure)
- Phase 2: net **-100 LOC** (replacement)
- Phase 3: net **-815 LOC** (cleanup)
- **TOTAL: net -724 LOC** (more than the synthesizer's −1120 estimate is conservative; some Phase 1 additions are shadow-mode that gets removed in Phase 3)
- Note: synthesizer's −1120 figure double-counted Phase 1 additions that are removed in Phase 3. Real net after Phase 3 is around -700 to -1100 depending on shadow-mode scaffolding extent.

---

## Critical sequencing rules (DO NOT VIOLATE)

1. **M9 before Phase 2**: HardForkSchedule entry must exist on at least 1 release before Phase 2 ships, so all nodes agree on activation height.
2. **M8 before Phase 2 activation**: `doli-cli repair-chain` must be available so operators of santiago/ivan/seed3 can backfill block_store gaps before HALT_PRODUCTION fires at activation.
3. **M2 + M3 + M4 before any Phase 2 milestone**: shadow mode requires the new types and CF to exist.
4. **Phase 1 dual-read divergence = 0 for ≥3 epochs on testnet** before Phase 2 ships to mainnet (REQ-REDESIGN-001 byte-identical state-root test).
5. **Phase 3 gated on**: Phase 2 sanity-check halt did not fire for ≥3 epochs after activation.
6. **M23 gated on**: RPC consumer grep returns zero matches for `DeterministicScheduler::*` outside crates/core/src/scheduler.rs itself.

---

## Recommended execution order (within Phase 1)

The 9 Phase 1 milestones can largely run independently, but the recommended order minimizes risk:

1. **M5** — `cached_scheduler` deletion (smallest, highest confidence, builds team confidence)
2. **M7** — dead methods deletion (compiler-verified safe)
3. **M6** — `excluded_producers` deletion (largest pure deletion, requires careful review)
4. **M3** — new types added (no behavior change)
5. **M1** — `EpochSnapshot` struct + CF
6. **M2** — shadow-mode compute (depends on M1)
7. **M4** — wire-format additions (depends on M3)
8. **M9** — HardForkSchedule entry
9. **M8** — `doli-cli repair-chain` (independent; can run anytime before Phase 2)

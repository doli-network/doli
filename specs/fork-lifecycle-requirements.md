<!--
OUTPUT CONTRACT: N/A — requirements document (not a test file)
INPUT PARTITIONS: N/A — requirements document (not a test file)
-->

# Requirements: Fork Lifecycle Redesign (INC-I-204) — M6 scope adjudication

> Origin: INC-I-204 — nine testnet nodes wedged at h=77777 for seven days.
> Full requirement catalogue: `docs/redesigns/fork-lifecycle-redesign-analysis.md`
> (REQ-FORK-001..019, lines 440-505).
> Architecture + milestone plan: `specs/fork-lifecycle-architecture.md`.
> M6 design adjudication: `docs/.workflow/inc-i-204-M6-design-brief.md`.

This file records the **M6 milestone scope decision only**. It exists so the M6
requirement row (`specs/fork-lifecycle-architecture.md:286`) has a machine-readable
priority for every id it names. It does not restate the full catalogue.

---

## M6 scope

| ID | Requirement | Priority (M6) | Acceptance criteria |
|---|---|---|---|
| **REQ-FORK-004** | Snap sync stops being reachable as a FORK remedy; admission narrows to bootstrap (`h=0`), the genesis window, genuine behind-ness (`gap >= SNAP_SYNC_GAP_MIN`) and the operator flag. Snap remains available for genuinely-far-behind and fresh nodes. | **Must** | - [ ] Bootstrap (`h=0`) and genesis-window admission unchanged<br>- [ ] `gap >= SNAP_SYNC_GAP_MIN` admission preserved (LB-5)<br>- [ ] No cell carrying fork evidence at `gap < 500` admits `SnapSync` or `GenesisResync`<br>- [ ] `--no-snap-sync` survives an emergency genesis-resync request<br>- [ ] INV-SYNC-011 (23 tests) + INV-SYNC-015 green, LB-6 untouched |
| **REQ-FORK-017 (Won't)** | Consolidate the 4 rollback primitives toward one guarded implementation. | **Won't** (deferred, was Could) | Deferred out of M6 — see refusal R2 below. |
| **REQ-FORK-019 (Won't)** | Retire `wedge_escape.rs` / `SiblingFetch` if REQ-FORK-010 makes them redundant. | **Won't** (deferred, was Could) | Deferred out of M6 — see refusal R3 below. |

---

## Refusals (evidence, not preference)

Both refused ids are priority **Could** in the source catalogue. They are recorded
**Won't for M6** — deferred with a named precondition, not cancelled. Neither may be
re-scoped into a milestone until its precondition is met and stated.

### R2 — REQ-FORK-017 (`execute_reorg` undo-loop consolidation)

Trap **T7** is listed on M6's own row: *"bundle rollback consolidation with door
removal (collapses the two-release ordering)"*. M6 **is** the door removal, so
consolidating in the same release is the trap verbatim.

Independently: `execute_reorg` is the executor that M4.1's audited `forceReorgTo`
escape runs through, and the escape exists to rewind a node whose finality state is
what wedged it (`the_escape_never_mutates_the_finality_tracker`). Adding a finality
veto inside `execute_reorg` would veto the only exit from the measured cell.

**Precondition to re-scope:** one release of separation after M6's door removal has
shipped and settled.

### R3 — REQ-FORK-019 (`SiblingFetch` / `wedge_escape.rs` retirement)

Trap **T11**: *"delete wedge_escape/SiblingFetch before C-6 passes (recreates
INC-I-143's 454-refusal livelock)"*. LB-9 permits only supersession-with-successor.

C-6's deterministic half is green
(`c6_every_cell_of_the_recovery_state_space_has_a_named_terminating_rung`), but the
**C-12 live drill has never run end-to-end** — carried as a residual from M4.1/M4.2:
the fleet runs v6.26.1, which answers `forceReorgTo` with `-32601`, and all 18 testnet
nodes sit at the same height, so the recorded cell is empty.

**Precondition to re-scope:** C-12 live drill executed end-to-end against a fleet that
answers `forceReorgTo`.

---

## Traceability (M6)

| Requirement | Priority | Tests | Invariants |
|---|---|---|---|
| REQ-FORK-004 | Must | `m6_d1_rollback_exhausted_at_a_minor_gap_must_not_reach_snap`, `m6_d1_deep_fork_confirmed_shape_must_not_reach_snap`, `m6_lock_snap_boundary_499_is_not_snap_and_500_is`, `m6_lock_large_gap_door_still_gated_by_attempts_and_peer_quorum`, `m6_d2_wedge_shape_with_apply_failures_must_wedge_not_wipe`, `m6_lock_non_forked_broken_node_still_reaches_genesis_resync`, `m6_d3_empty_headers_at_gap_50_must_park_not_resync`, `m6_d3_empty_headers_at_gap_600_must_park_not_resync`, `m6_lock_gap_le_3_gossip_wait_and_its_only_reset_are_unchanged`, `m6_lock_park_arms_above_gap_3_never_reset_the_evidence_counter`, `m6_d4_emergency_under_no_snap_sync_is_honored_without_re_enabling_snap`, `m6_d4_honored_emergency_does_not_open_the_snap_admission_door`, `m6_lock_non_emergency_under_no_snap_sync_is_still_refused`, `shallow_rollback_exhausted_escalates_to_snap` (reversed), `m6_no_fork_shaped_cell_reaches_a_lossy_action`, `m6_census_enumeration_is_not_vacuous`, `m6_lossy_admission_census`, `m6_pin_the_mirror_answers_every_read_the_finality_tracker_would`, `m6_pin_the_only_mirror_tracker_divergence_is_the_inv_sync_004_backstop`, `m6_pin_set_last_finality_height_is_monotone_above_the_gate`, `m6_pin_finality_guard_backstop_still_clears_after_a_rollback_below_tip`, `m6_pin_fork_choice_gate_is_still_dormant_on_mainnet_and_testnet` | INV-SYNC-004, INV-SYNC-011, INV-SYNC-015, INV-FINALITY-001, LB-5, LB-6 |
| REQ-FORK-017 | Won't (M6) | none — refused, see R2 | INV-SYNC-015 (must stay green when it does land) |
| REQ-FORK-019 | Won't (M6) | none — refused, see R3 | LB-9, INC-I-143 livelock bound |

//! INC-I-204 M5 — the ONE branch-weight authority, its dormant window, and derived finality.
//!
//! TDD RED, EXPECTED: this file does not compile against HEAD. `weigh_branches`,
//! `WeightVerdict`, `ForkChoiceFinality` and `record_fork_block_with_height` do not
//! exist yet. The BEHAVIOURAL red — tests that compile today and fail today — is
//! `tests_m5_red_witness.rs`.
//!
//! WHY THIS FILE EXISTS. Three rules decide "is this branch better than mine" and they
//! disagree on a weight TIE (`tests_m5_red_witness.rs` W1/W1b prove it). M5 replaces
//! them with one, behind an activation height. An activation height is only worth its
//! cost if BOTH halves hold:
//!
//!   below the gate — old and new must be INDISTINGUISHABLE, or a mixed fleet splits
//!                    at the first tie (trap T9, hard filter C-22);
//!   at/above it    — one rule, for every door.
//!
//! The first half cannot be tested by inspection, and "we did not mean to change it"
//! is not evidence. It is tested here against `tests_m5_common`'s transcription of the
//! pre-M5 source lines, driven over a weight vector that includes the incident's own
//! `(10390, 10390)` and both hash orderings.
//!
//! Sibling file: `tests_m5_finality_authority.rs` (the finality half — T1, ancestry,
//! INV-SYNC-012, LRU eviction).

// OUTPUT CONTRACT — ENUMERATION-CHECKLIST for the three functions this file owns.
//
//   F1: ReorgHandler::weigh_branches(&self, our_tip, cand_tip, finality) -> WeightVerdict
//       O1: return — the verdict. THE observable.
//       (no mutable params; `&self`; no receiver mutation; no persistent store writes —
//        `crates/network` has no doli-storage dependency, S10)
//       PATHS:
//         P1 heavier   cand_w > our_w
//         P2 lighter   cand_w < our_w
//         P3 tie/pre   cand_w == our_w, local_tip_height <  AH   -> TieKeep, always
//         P4 tie/post  cand_w == our_w, local_tip_height >= AH   -> TieSwitch iff cand < our
//       INPUT PARTITIONS: WEIGHT_VECTOR (10 weight pairs incl. the exact tie, the
//         incident pair, a zero-zero tie, and a near-overflow tie) x {cand hash lower,
//         cand hash higher} x {GATE_DORMANT, GATE_ACTIVE}.
//       MATRIX: 1 output x 10 x 2 x 2 = 40 cells, all driven by the table tests.
//
//   F2: ReorgHandler::record_fork_block_with_height(&mut self, hash, prev, weight, real_h) -> ()
//       O1: return — unit.
//       O2: receiver — block_weights[hash].height            <- THE observable
//       O3: receiver — block_weights[hash].accumulated_weight
//       O4: receiver — block_weights[hash].prev_hash
//       O5: receiver — current_chain_weight (must NOT move: fork path)
//       O6: receiver — recent_blocks / block_parents / lru_order membership
//       PATHS: P1 real_h >= AH -> store real_h ; P2 real_h < AH -> legacy parent_height+1
//       INPUT PARTITIONS: {parent absent from block_weights (process start),
//         parent present} x {GATE_ACTIVE, GATE_DORMANT}
//       MATRIX: 6 outputs x 4 cells; O2 asserted in all four, O3-O6 in the two that
//         could regress INC-I-012 (the fork-weighs-against-itself bug).
//
//   F3: ForkChoiceFinality::effective_finality(&self) -> Option<u64>
//       O1: return. Pure; no other channel exists.
//       PATHS: P1 finalized_height None -> None ; P2 Some(f) -> Some(min(f, local_tip))
//       INPUT PARTITIONS: tip above f, tip == f, tip below f, tip 0, f 0, f u64::MAX.

use super::tests_m5_common::*;
use super::*;

// ===========================================================================
// T9 — THE BINDING GATE. Below the activation height, old and new must be
// indistinguishable. Design brief S3.
// ===========================================================================

/// REQ-FORK-006 / T9 — Decision: a failure means a node on the new binary answers a
/// weight tie differently from a node on the old one BELOW the gate, which splits the
/// live mainnet and testnet fleets during the whole dormant window; the milestone would
/// have to be withdrawn, not patched.
///
/// `weigh_branches` is the successor of the wedge-escape short-circuit
/// (`wedge_escape.rs:119-132`). Below the gate it must reproduce that rule exactly:
/// `cand_w <= our_w` keeps our tip. In particular it must NEVER return `TieSwitch`
/// below the gate — that verdict has no pre-M5 counterpart on this path.
#[test]
fn t9_pre_ah_weigh_branches_matches_the_wedge_escape_give_up_rule() {
    for &(our_w, cand_w) in WEIGHT_VECTOR {
        for cand_lower in [true, false] {
            let f = two_branch(our_w, cand_w, cand_lower, GATE_DORMANT);
            let verdict =
                f.handler
                    .weigh_branches(&f.our_tip, &f.cand_tip, fin(None, None, TIP_REAL_H));

            let switches = matches!(verdict, WeightVerdict::Heavier);
            assert_eq!(
                switches,
                legacy_weight_switch(our_w, cand_w),
                "T9 dormant window: weigh_branches({our_w}, {cand_w}, cand_lower={cand_lower}) \
                 gave {verdict:?}, which switches={switches}. wedge_escape.rs:132 gives \
                 up whenever cand_w <= our_w. A node on the old binary and a node on \
                 the new one must be indistinguishable below the gate."
            );
            assert_ne!(
                verdict,
                WeightVerdict::TieSwitch,
                "T9: TieSwitch has NO pre-M5 counterpart on the wedge-escape path — the \
                 old binary never switches on a tie there. Producing it below the gate \
                 IS the mixed-fleet split. ({our_w}, {cand_w}, cand_lower={cand_lower})"
            );
        }
    }
}

/// REQ-FORK-006 / T9 — Decision: a failure means the gossip door's equal-weight
/// tie-break moved below the gate, changing which of two equal branches every node
/// picks off gossip — the highest-traffic fork-choice path there is.
///
/// `check_reorg_weighted`'s own documented rule (`mod.rs:344-363`, lower hash wins,
/// `>= current_tip` refused) is preserved VERBATIM for that entry point below the gate,
/// including its absence of a zero-weight guard.
#[test]
fn t9_pre_ah_check_reorg_weighted_keeps_its_verbatim_equal_weight_hash_reject() {
    for &(our_w, cand_w) in WEIGHT_VECTOR {
        for cand_lower in [true, false] {
            let f = two_branch(our_w, cand_w, cand_lower, GATE_DORMANT);
            // The gossiped sibling: chain_weight(ancestor) is 0, so its producer weight
            // IS the candidate branch weight.
            let block = fork_block_ordered(f.ancestor, f.our_tip, cand_lower);
            let switches = f
                .handler
                .check_reorg_weighted(
                    &block,
                    f.our_tip,
                    cand_w,
                    |h| f.heights.get(h).copied(),
                    fin(None, None, TIP_REAL_H),
                )
                .is_some();

            assert_eq!(
                switches,
                legacy_gossip_switch(our_w, cand_w, &block.hash(), &f.our_tip),
                "T9 dormant window: check_reorg_weighted({our_w}, {cand_w}, \
                 cand_lower={cand_lower}) must reproduce mod.rs:336-363 exactly. Note \
                 that rule has NO `weight > 0` guard on its tie-break, unlike \
                 should_reorg_by_weight_with_tiebreak — the zero-zero row of the vector \
                 is where that difference bites."
            );
        }
    }
}

/// REQ-FORK-006 / T9 — Decision: a failure means M5 silently re-pointed the third rule
/// at the new authority below the gate; since that method has ZERO production callers
/// (measured run 542), the right move is to DELETE it in M6, and a silent behaviour
/// change would hide that decision instead of forcing it.
#[test]
fn t9_pre_ah_should_reorg_by_weight_with_tiebreak_is_untouched() {
    for &(our_w, cand_w) in WEIGHT_VECTOR {
        for cand_lower in [true, false] {
            let f = two_branch(our_w, cand_w, cand_lower, GATE_DORMANT);
            assert_eq!(
                f.handler
                    .should_reorg_by_weight_with_tiebreak(&f.cand_tip, &f.our_tip),
                legacy_tiebreak_method_switch(our_w, cand_w, &f.cand_tip, &f.our_tip),
                "mod.rs:284-294 must be byte-identical below the gate ({our_w}, \
                 {cand_w}, cand_lower={cand_lower})"
            );
        }
    }
}

/// REQ-FORK-006 / T9 — ANTI-VACUITY for the three tests above.
///
/// Decision: a failure means the gate is inert — either the new authority never
/// diverges from the old one (so the activation height buys nothing and M5 should not
/// have one), or `local_tip_height` is not actually wired as the gate input. Both are
/// fatal to the milestone's justification, and both would leave the three parity tests
/// above passing while proving nothing.
///
/// Same fixture, two gate values: the ONLY row whose verdict may move is the tie.
#[test]
fn t9_the_gate_is_the_only_thing_that_changes_the_tie_answer() {
    let mut tie_rows_that_moved = 0usize;

    for &(our_w, cand_w) in WEIGHT_VECTOR {
        for cand_lower in [true, false] {
            let dormant = two_branch(our_w, cand_w, cand_lower, GATE_DORMANT);
            let active = two_branch(our_w, cand_w, cand_lower, GATE_ACTIVE);
            let f = fin(None, None, TIP_REAL_H);

            let before = dormant
                .handler
                .weigh_branches(&dormant.our_tip, &dormant.cand_tip, f);
            let after = active
                .handler
                .weigh_branches(&active.our_tip, &active.cand_tip, f);

            if our_w == cand_w {
                if before != after {
                    tie_rows_that_moved += 1;
                }
            } else {
                assert_eq!(
                    before, after,
                    "the gate must not touch a NON-tie row: ({our_w}, {cand_w}, \
                     cand_lower={cand_lower}) went {before:?} -> {after:?}. Only the \
                     tie is allowed to differ across the height."
                );
            }
        }
    }

    assert!(
        tie_rows_that_moved > 0,
        "ANTI-VACUITY: no tie row changed its verdict across the activation height. \
         Either the gate is not wired to finality.local_tip_height, or the post-AH rule \
         is identical to the pre-AH one — in which case M5 needs no activation height \
         and the three parity tests above are asserting nothing."
    );
}

/// REQ-FORK-006 — Decision: a failure means the ONE rule is not the rule the brief
/// specified, so the wedge escape, the gossip door and fork recovery would still be
/// able to disagree above the gate — the milestone's entire purpose.
///
/// At and above the gate: strictly heavier wins; an exact tie goes to the LOWER hash;
/// everything else keeps our tip. One rule, every door.
#[test]
fn t9_post_ah_weigh_branches_is_the_one_rule() {
    for &(our_w, cand_w) in WEIGHT_VECTOR {
        for cand_lower in [true, false] {
            let f = two_branch(our_w, cand_w, cand_lower, GATE_ACTIVE);
            let verdict =
                f.handler
                    .weigh_branches(&f.our_tip, &f.cand_tip, fin(None, None, TIP_REAL_H));

            let expected = if cand_w > our_w {
                WeightVerdict::Heavier
            } else if cand_w < our_w {
                WeightVerdict::Lighter
            } else if cand_lower {
                WeightVerdict::TieSwitch
            } else {
                WeightVerdict::TieKeep
            };

            assert_eq!(
                verdict, expected,
                "post-AH one rule: ({our_w}, {cand_w}, cand_lower={cand_lower}). The \
                 tie goes to the lower hash with NO zero-weight exception — the zero-zero \
                 row is where the pre-M5 rules disagreed with each other, and the \
                 unified rule must answer it once."
            );
        }
    }
}

/// REQ-FORK-006 / INV-SYNC-012 — Decision: a failure means the gate input is not a
/// chain-derived height. `weigh_branches` is gated on `finality.local_tip_height`, which
/// the caller derives from the local chain tip; if it were gated on anything
/// process-derived, M5 would reproduce INC-I-147's unit mismatch in its own gate and the
/// activation height would fire at a different block on every node.
#[test]
fn the_weigh_branches_gate_input_is_the_local_tip_height() {
    const GATE: u64 = 500;
    let f = two_branch(INCIDENT_WEIGHT, INCIDENT_WEIGHT, true, GATE);

    let below = f
        .handler
        .weigh_branches(&f.our_tip, &f.cand_tip, fin(None, None, GATE - 1));
    let at = f
        .handler
        .weigh_branches(&f.our_tip, &f.cand_tip, fin(None, None, GATE));

    assert_eq!(
        below,
        WeightVerdict::TieKeep,
        "one block BELOW the gate the legacy give-up rule still binds"
    );
    assert_eq!(
        at,
        WeightVerdict::TieSwitch,
        "AT the gate the unified rule binds. The transition must be driven by \
         local_tip_height and by nothing else — never by BlockWeight.height, which is a \
         per-process counter (INV-SYNC-012)."
    );
}

// ===========================================================================
// Contradiction 2 — the fork-recording writer finally learns the real height.
// ===========================================================================

/// REQ-FORK-006 / INV-SYNC-012 — Decision: a failure means every block recorded on a
/// competing branch still carries a per-process counter above the gate, so the unified
/// finality comparison would be fed the same poisoned unit that caused INC-I-204; the
/// rest of M5 would be correct and still not fix the incident.
///
/// Successor to `tests_m5_red_witness.rs` W2.
#[test]
fn post_ah_record_fork_block_with_height_stores_the_real_chain_height() {
    let mut handler = ReorgHandler::with_activation_heights(0, GATE_ACTIVE);
    // Absent from block_weights: this process booted above genesis.
    let parent = crypto::hash::hash(b"m5_rfbwh_unrecorded_parent");
    let block = crypto::hash::hash(b"m5_rfbwh_fork_block");

    handler.record_fork_block_with_height(block, parent, INCIDENT_WEIGHT, TIP_REAL_H);

    let stored = handler.get_block_weight(&block).expect("O6: tracked");
    assert_eq!(
        stored.height, TIP_REAL_H,
        "O2: above the gate the caller's REAL chain height is authoritative, exactly as \
         record_block_with_height already does (mod.rs:146-159)"
    );
    assert_eq!(
        stored.accumulated_weight, INCIDENT_WEIGHT,
        "O3: absent parent contributes 0; the weight accounting is unchanged"
    );
    assert_eq!(stored.prev_hash, parent, "O4");
    assert_eq!(
        handler.current_weight(),
        0,
        "O5: the FORK path must never move current_chain_weight, or the next comparison \
         weighs the fork against itself and always rejects (mod.rs:161-166)"
    );
    assert!(handler.knows_block(&block), "O6");
    assert_eq!(handler.get_parent(&block), Some(parent), "O6");
}

/// REQ-FORK-006 / T9 — Decision: a failure means the fork-recording writer changed
/// behaviour below the gate. `BlockWeight.height` feeds the pre-AH finality comparison,
/// so a different stored height below the gate is a different fork choice below the
/// gate — the mixed-fleet split, arriving through the writer instead of the reader.
#[test]
fn pre_ah_record_fork_block_with_height_is_byte_identical_to_record_fork_block() {
    let parent = crypto::hash::hash(b"m5_rfbwh_pre_parent");
    let block = crypto::hash::hash(b"m5_rfbwh_pre_block");

    let mut gated = ReorgHandler::with_activation_heights(0, GATE_DORMANT);
    gated.record_fork_block_with_height(block, parent, INCIDENT_WEIGHT, TIP_REAL_H);

    let mut legacy = ReorgHandler::with_activation_heights(0, GATE_DORMANT);
    legacy.record_fork_block(block, parent, INCIDENT_WEIGHT);

    let g = gated.get_block_weight(&block).expect("tracked");
    let l = legacy.get_block_weight(&block).expect("tracked");

    assert_eq!(
        (g.height, g.accumulated_weight, g.prev_hash),
        (l.height, l.accumulated_weight, l.prev_hash),
        "below the gate the new writer must be indistinguishable from record_fork_block \
         (legacy derived height {}, NOT the real {TIP_REAL_H})",
        l.height
    );
    assert_eq!(
        g.height, 1,
        "and the legacy value is specifically `parent_height + 1` off an absent parent \
         (mod.rs:194) — pinned as a literal so 'byte-identical' cannot be satisfied by \
         two implementations that are both wrong in the same new way"
    );
    assert_eq!(gated.current_weight(), legacy.current_weight(), "O5 parity");
}

/// REQ-FORK-006 — Decision: a failure means the gate input for the WRITER is not the
/// real height, so a node would cross this gate at a different block than its peers.
/// The gate input must match `record_block_with_height`'s (`mod.rs:153`).
#[test]
fn the_record_fork_block_gate_input_is_the_real_height_argument() {
    const GATE: u64 = 1_000;
    let parent = crypto::hash::hash(b"m5_rfbwh_gate_parent");

    let mut h = ReorgHandler::with_activation_heights(0, GATE);
    let below = crypto::hash::hash(b"m5_rfbwh_gate_below");
    let at = crypto::hash::hash(b"m5_rfbwh_gate_at");
    h.record_fork_block_with_height(below, parent, 1, GATE - 1);
    h.record_fork_block_with_height(at, parent, 1, GATE);

    assert_eq!(
        h.get_block_weight(&below).expect("tracked").height,
        1,
        "below the gate: legacy derived height"
    );
    assert_eq!(
        h.get_block_weight(&at).expect("tracked").height,
        GATE,
        "at the gate: the real height"
    );
}

// ===========================================================================
// Derived finality — the successor to the erasable mirror. Brief S12, INV-SYNC-004.
// ===========================================================================

/// REQ-FORK-006 / INV-SYNC-004 successor — Decision: a failure means the successor does
/// not reproduce the mirror's RELEASE semantics, so a node that rolls back below
/// finality would either stay wedged (guard never releases) or lose the guard entirely
/// (INV-SYNC-008 violated). Both are how INC-I-081 and INC-I-090 happened.
#[test]
fn effective_finality_is_the_min_of_the_checkpoint_and_the_local_tip() {
    let h = crypto::hash::hash(b"m5_eff_finalized");

    assert_eq!(
        fin(Some(100), Some(h), 200).effective_finality(),
        Some(100),
        "tip above the checkpoint: the checkpoint binds"
    );
    assert_eq!(
        fin(Some(100), Some(h), 100).effective_finality(),
        Some(100),
        "tip exactly at the checkpoint"
    );
    assert_eq!(
        fin(Some(100), Some(h), 50).effective_finality(),
        Some(50),
        "tip BELOW the checkpoint: the tip binds. This is the release the mirror \
         performed by ERASING itself (clear_finality_if_below_tip, mod.rs:254-264); the \
         successor performs it by clamping, with no erasable state."
    );
    assert_eq!(fin(Some(0), Some(h), 0).effective_finality(), Some(0));
    assert_eq!(
        fin(Some(u64::MAX), Some(h), 7).effective_finality(),
        Some(7)
    );
}

/// REQ-FORK-006 / INV-FINALITY-001 — Decision: a failure means the guard can be switched
/// OFF by a rollback, which is exactly the erasure that made both monotonicity demands
/// of INV-FINALITY-001 vacuous at h=57067 (INC-I-147 / INC-I-081 Bug 4). The whole point
/// of replacing the mirror is that this state is unreachable.
#[test]
fn effective_finality_never_returns_to_none_once_something_has_finalized() {
    let h = crypto::hash::hash(b"m5_eff_never_none");
    for tip in [0u64, 1, 49, 99, 100, 101, u64::MAX] {
        assert!(
            fin(Some(100), Some(h), tip).effective_finality().is_some(),
            "INV-FINALITY-001 strengthened: with a checkpoint at 100 and local tip \
             {tip}, effective finality must still be Some. The mirror answered None \
             here for every tip below 100, leaving the sub-finality refusal DISARMED \
             until a new finalization re-armed it."
        );
    }
}

/// REQ-FORK-006 — Decision: a failure means the guard fires before anything has
/// finalized, refusing legal reorgs on a fresh or never-finalized node — the state
/// every node is in at process start, and the reason `plan_reorg` gates on the ANCESTOR
/// height rather than on the finality height (`mod.rs:553-557`).
#[test]
fn effective_finality_is_none_when_nothing_has_finalized() {
    assert_eq!(ForkChoiceFinality::default().effective_finality(), None);
    assert_eq!(fin(None, None, 900_000).effective_finality(), None);
    assert_eq!(
        fin(None, Some(crypto::hash::hash(b"m5_orphan_hash")), 900_000).effective_finality(),
        None,
        "a hash with no height finalizes nothing — the height is what the numeric \
         pre-filter reads, and a half-populated checkpoint must not arm the guard"
    );
}

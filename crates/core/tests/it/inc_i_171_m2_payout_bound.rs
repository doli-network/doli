//! INC-I-171 M2 — `check_withdrawal_payout_bound`: the gate, the guard, the bound and
//! the error surface.
//!
// covers: crates/core/src/validation/vesting.rs, crates/core/src/validation/error.rs
//!
//! Requirements: **REQ-VEST-001** (Must — payout bounded by penalized net + non-Bond
//! value), **REQ-VEST-003** (Must — the rule is dormant behind its own frozen height),
//! **REQ-VEST-004** (Must — deterministic, order-independent, no panic),
//! **REQ-VEST-006** (Must — the tier comes from params, and an unusable param is an
//! error rather than a division panic). Pins INV-VEST-003.
//!
//! TDD RED, EXPECTED: this module does not compile against the tree at HEAD —
//! `doli_core::validation::vesting` and the three `ValidationError` variants below do
//! not exist. That compile failure is the red, exactly as
//! `inc_i_208_activation_height.rs` documents for itself.
//!
//! GUARD ORDER IS PINNED, NOT LEFT OPEN. The activation gate is INSIDE the predicate and
//! runs FIRST: below the activation height, or at/above the disable height, the fn
//! returns `Ok(())` without ever reading `quarter_slots`. So a zero quarter is `Ok` while
//! the rule is dormant and `Err` once it is armed.
//! See [`req_vest_003_the_dormant_gate_returns_before_the_quarter_guard`].

// OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//
//   F1: check_withdrawal_payout_bound(payout, spent_bonds, non_bond_input_total,
//         block_slot, quarter_slots, height, activation_height, disable_height)
//         -> Result<(), ValidationError>
//       PURE free fn. Mutable params: NONE (`spent_bonds` is a shared slice).
//       Receiver/self mutation: NONE. Persistent store writes: NONE.
//       O1: Ok(()) vs Err(_) — the verdict.
//       O2: the Err VARIANT (WithdrawalPayoutExceedsVestedNet | VestingQuarterInvalid).
//       O3: the four fields of WithdrawalPayoutExceedsVestedNet
//           (payout, bound, bond_inputs, non_bond_value).
//       O4: the one field of VestingQuarterInvalid (quarter_slots).
//       PATHS:
//         P-BELOW    height < activation_height              -> Ok, nothing else read
//         P-DISABLED height >= disable_height                -> Ok, nothing else read
//         P-BADQ     quarter_slots == 0 || > u32::MAX        -> Err(VestingQuarterInvalid)
//         P-OK       payout <= bound                         -> Ok
//         P-OVER     payout >  bound                         -> Err(WithdrawalPayout…)
//   F2: ValidationError::error_code(&self) -> &'static str   (PURE)
//       O5: the stable string for each of the three new variants.
//   F3: ValidationError::to_structured_json(&self) -> Value  (PURE)
//       O6: the field keys emitted for each of the three new variants.
//   MATRIX: O1 x every path; O2 x {P-BADQ, P-OVER}; O3 x P-OVER; O4 x P-BADQ;
//           O5,O6 x each of the three variants — every cell is asserted below.
//   INPUT PARTITIONS: bonds {empty, 3 mixed tiers in 3 permutations, 4 x u64::MAX};
//     quarter_slots {0, 1, 100, u32::MAX, u32::MAX+1}; height {AH-1, AH, mid,
//     DISABLE-1, DISABLE, DISABLE+1}; payout {0, bound-k, bound, bound+1, u64::MAX}.
//   NOT CLAIMED HERE: the per-bond arithmetic (`inc_i_171_m2_penalty_golden.rs`) and
//     the shipped params (`inc_i_171_m2_activation_height.rs`).

use doli_core::validation::vesting::check_withdrawal_payout_bound;
use doli_core::validation::ValidationError;

const Q: u64 = 100;
const AH: u64 = 1_000;
const DISABLE: u64 = 2_000;
/// Strictly inside `[AH, DISABLE)`, so the rule is armed.
const ARMED: u64 = 1_500;

const BLOCK_SLOT: u32 = 250;

/// Three bonds in three different tiers with three different amounts, so a permutation
/// cannot be neutralised by symmetry. Nets under `Q = 100` at slot 250:
/// 1_000_007 age 250 -> 25% -> 750_006; 500_003 age 100 -> 50% -> 250_002;
/// 777 age 1 -> 75% -> 195.
const BONDS: [(u64, u32); 3] = [(1_000_007, 0), (500_003, 150), (777, 249)];
const NON_BOND: u64 = 42;
/// 750_006 + 250_002 + 195 + 42. A literal, so the assertions do not re-derive the
/// value from the function under test.
const EXPECTED_BOUND: u64 = 1_000_245;

/// Every argument except the four that a given test varies.
fn check(
    payout: u64,
    bonds: &[(u64, u32)],
    non_bond: u64,
    height: u64,
) -> Result<(), ValidationError> {
    check_withdrawal_payout_bound(payout, bonds, non_bond, BLOCK_SLOT, Q, height, AH, DISABLE)
}

fn expect_over(result: Result<(), ValidationError>) -> (u64, u64, u32, u64) {
    match result {
        Err(ValidationError::WithdrawalPayoutExceedsVestedNet {
            payout,
            bound,
            bond_inputs,
            non_bond_value,
        }) => (payout, bound, bond_inputs, non_bond_value),
        other => panic!("expected WithdrawalPayoutExceedsVestedNet, got {other:?}"),
    }
}

// ===========================================================================
// The bound itself.
// ===========================================================================

/// REQ-VEST-001 — Decision: a failure at `payout == bound` says the rule is `<` and
/// rejects the exact honest payout; a failure at `bound + 1` says the rule does not bind
/// at all, which is the whole of INC-I-171. The four field assertions are the operator's
/// only evidence of WHY a block was rejected — a wrong `bound` sends the next
/// investigation to the wrong producer.
#[test]
fn req_vest_001_payout_equal_to_the_bound_is_ok_and_one_over_is_rejected() {
    assert!(
        check(EXPECTED_BOUND, &BONDS, NON_BOND, ARMED).is_ok(),
        "the rule is `payout <= bound`: the exact bound is the honest CLI's payout"
    );

    let (payout, bound, bond_inputs, non_bond_value) =
        expect_over(check(EXPECTED_BOUND + 1, &BONDS, NON_BOND, ARMED));
    assert_eq!(
        payout,
        EXPECTED_BOUND + 1,
        "the rejected payout is reported verbatim"
    );
    assert_eq!(bound, EXPECTED_BOUND, "the computed ceiling is reported");
    assert_eq!(
        bond_inputs, 3,
        "the COUNT of spent Bond inputs (u32), not their value"
    );
    assert_eq!(
        non_bond_value, NON_BOND,
        "the non-Bond input total is reported so an operator can split the ceiling"
    );
}

/// REQ-VEST-001 — Decision: a failure says the rule is `==`, which forbids voluntary
/// over-burning. A producer that pays itself less than the ceiling destroys value in the
/// network's favour; rejecting that would break every wallet that rounds down or pays a
/// fee out of the withdrawal.
#[test]
fn req_vest_001_over_burning_below_the_bound_is_accepted() {
    for payout in [0u64, 1, EXPECTED_BOUND / 2, EXPECTED_BOUND - 1] {
        assert!(
            check(payout, &BONDS, NON_BOND, ARMED).is_ok(),
            "payout {payout} is below the ceiling {EXPECTED_BOUND} and must be accepted"
        );
    }
}

/// REQ-VEST-001 — Decision: a failure says non-Bond inputs are dropped from the ceiling,
/// which rejects every withdrawal that pays its fee from an ordinary coin input, or that
/// they are double-counted, which raises the ceiling above the value actually spent.
#[test]
fn req_vest_001_with_no_bond_inputs_the_bound_is_the_non_bond_input_total() {
    assert!(
        check(NON_BOND, &[], NON_BOND, ARMED).is_ok(),
        "no Bond inputs: the ceiling is exactly the non-Bond input total"
    );

    let (_, bound, bond_inputs, non_bond_value) =
        expect_over(check(NON_BOND + 1, &[], NON_BOND, ARMED));
    assert_eq!(bound, NON_BOND);
    assert_eq!(bond_inputs, 0, "an empty slice reports zero Bond inputs");
    assert_eq!(non_bond_value, NON_BOND);
}

/// REQ-VEST-004 / INV-VEST-003 — Decision: a failure says the ceiling depends on input
/// ORDER. Two nodes that iterate the same UTXO set in different orders would then reach
/// different verdicts on the same block — a fork with no rule change and no attacker.
/// The natural way to introduce it is to sum first and truncate once; per-bond
/// truncation is what makes the fold order-free.
#[test]
fn req_vest_004_the_bound_is_invariant_under_input_permutation() {
    let permutations: [[(u64, u32); 3]; 3] = [
        [BONDS[0], BONDS[1], BONDS[2]],
        [BONDS[2], BONDS[0], BONDS[1]],
        [BONDS[1], BONDS[2], BONDS[0]],
    ];

    for (i, bonds) in permutations.iter().enumerate() {
        assert!(
            check(EXPECTED_BOUND, bonds, NON_BOND, ARMED).is_ok(),
            "permutation {i}: the exact bound must still be accepted"
        );
        let (_, bound, bond_inputs, _) =
            expect_over(check(EXPECTED_BOUND + 1, bonds, NON_BOND, ARMED));
        assert_eq!(
            bound, EXPECTED_BOUND,
            "permutation {i}: the reported ceiling must be the same integer for every \
             ordering of the same multiset"
        );
        assert_eq!(
            bond_inputs, 3,
            "permutation {i}: the count is order-free too"
        );
    }
}

// ===========================================================================
// Overflow and saturation.
// ===========================================================================

/// REQ-VEST-004 — Decision: a failure is either an overflow panic in consensus
/// validation or, worse, a WRAPPED ceiling. Four fully vested `u64::MAX` bonds sum to
/// roughly 2^66; a u64 accumulator wraps to `u64::MAX - 3` and would then reject the
/// honest full-value payout of `u64::MAX`. This test fails on a wrapping implementation
/// even in release, where no panic would fire.
#[test]
fn req_vest_004_u128_accumulation_saturates_instead_of_wrapping() {
    // Every bond fully vested (age 3Q with Q = 100), so each net is u64::MAX itself.
    let bonds = [(u64::MAX, 0u32); 4];
    let fully_vested_slot: u32 = 3 * Q as u32;

    let result = check_withdrawal_payout_bound(
        u64::MAX,
        &bonds,
        0,
        fully_vested_slot,
        Q,
        ARMED,
        AH,
        DISABLE,
    );
    assert!(
        result.is_ok(),
        "the sum must saturate at u64::MAX, not wrap to u64::MAX - 3: {result:?}"
    );
}

/// REQ-VEST-004 — Decision: a failure is an add-overflow panic when a withdrawal spends
/// an ordinary coin input alongside a saturated Bond total. A plain `+` panics in debug
/// and wraps in release; the wrap produces a TINY ceiling and rejects an honest payout.
#[test]
fn req_vest_004_non_bond_total_is_added_with_saturation() {
    let bonds = [(u64::MAX, 0u32); 4];
    let fully_vested_slot: u32 = 3 * Q as u32;

    let result = check_withdrawal_payout_bound(
        u64::MAX,
        &bonds,
        12_345,
        fully_vested_slot,
        Q,
        ARMED,
        AH,
        DISABLE,
    );
    assert!(
        result.is_ok(),
        "an already-saturated ceiling plus a non-zero non-Bond total stays u64::MAX: \
         {result:?}"
    );
}

// ===========================================================================
// The quarter guard (REQ-VEST-006).
// ===========================================================================

/// REQ-VEST-006 — Decision: a failure is a divide-by-zero PANIC inside block validation.
/// `withdrawal_penalty_rate_with_quarter` divides by `quarter_slots` unguarded
/// (`constants.rs:431`), so a zero param crashes every node that validates a withdrawal
/// rather than rejecting one block.
#[test]
fn req_vest_006_quarter_slots_zero_is_an_error_not_a_division_panic() {
    let result = check_withdrawal_payout_bound(1, &BONDS, 0, BLOCK_SLOT, 0, ARMED, AH, DISABLE);
    assert_eq!(
        result,
        Err(ValidationError::VestingQuarterInvalid { quarter_slots: 0 }),
        "a zero quarter must be a typed rejection carrying the offending value"
    );
}

/// REQ-VEST-006 — Decision: a failure at `u32::MAX + 1` says the u64 param is narrowed
/// to u32 by a silent `as` cast, so 2^32 would read as quarter 0 and panic; a failure at
/// `u32::MAX` says the valid side of the boundary was made exclusive and a legitimate
/// param is refused. The ladder takes `Slot = u32` (`types.rs:13`), so `u32::MAX` is the
/// largest representable quarter and it is INCLUSIVE.
#[test]
fn req_vest_006_the_quarter_upper_bound_is_inclusive_at_u32_max() {
    let too_big = u32::MAX as u64 + 1;
    assert_eq!(
        check_withdrawal_payout_bound(1, &BONDS, 0, BLOCK_SLOT, too_big, ARMED, AH, DISABLE),
        Err(ValidationError::VestingQuarterInvalid {
            quarter_slots: too_big
        }),
        "a quarter that does not fit in Slot must be rejected, never truncated"
    );

    // Q = u32::MAX: age 0 is quarter 0, the 75% tier. 101 -> penalty 75 -> net 26.
    let widest = u32::MAX as u64;
    assert!(
        check_withdrawal_payout_bound(26, &[(101, 0)], 0, 0, widest, ARMED, AH, DISABLE).is_ok(),
        "u32::MAX is a representable quarter and must be accepted"
    );
    let (_, bound, _, _) = expect_over(check_withdrawal_payout_bound(
        27,
        &[(101, 0)],
        0,
        0,
        widest,
        ARMED,
        AH,
        DISABLE,
    ));
    assert_eq!(
        bound, 26,
        "anti-vacuity: at u32::MAX the predicate really evaluated the ladder rather \
         than short-circuiting"
    );
}

// ===========================================================================
// The activation gate (REQ-VEST-003).
// ===========================================================================

/// REQ-VEST-003 — Decision: a failure says the guard order is inverted. The gate is
/// INSIDE the predicate and runs FIRST, so a dormant rule never reads `quarter_slots`
/// and never rejects anything. An implementation that validates the param before the
/// gate turns a bad `.env` on one node into block rejections at every height, below the
/// activation height included — exactly the class of self-inflicted split the frozen
/// gate exists to prevent. THE EXPECTATION IS `Ok(())`; do not invert this.
#[test]
fn req_vest_003_the_dormant_gate_returns_before_the_quarter_guard() {
    assert!(
        check_withdrawal_payout_bound(u64::MAX, &BONDS, 0, BLOCK_SLOT, 0, AH - 1, AH, DISABLE)
            .is_ok(),
        "below the activation height a zero quarter is not even looked at"
    );
    assert!(
        check_withdrawal_payout_bound(u64::MAX, &BONDS, 0, BLOCK_SLOT, 0, DISABLE, AH, DISABLE)
            .is_ok(),
        "at or above the disable height a zero quarter is not even looked at"
    );
}

/// REQ-VEST-003 — Decision: a failure below the activation height means the shipped
/// binary is NOT byte-identical to its predecessor and replaying history diverges
/// (INV-VEST-007); a failure at or above the disable height means the in-protocol undo
/// does not disarm, leaving a binary revert — which partitions ~30 auto-update
/// producers — as the only escape.
#[test]
fn req_vest_003_the_rule_is_dormant_outside_the_window() {
    for height in [0u64, 1, AH - 1, DISABLE, DISABLE + 1, u64::MAX] {
        assert!(
            check(u64::MAX, &BONDS, NON_BOND, height).is_ok(),
            "height {height} is outside [{AH}, {DISABLE}): a payout far above the \
             ceiling must still be accepted"
        );
    }
}

/// REQ-VEST-003 — Decision: a failure at `AH` says the lower edge is exclusive, so the
/// rule arms one block late and the pinned height in the release notes is a lie; a
/// failure at `DISABLE - 1` says the window closes one block early. Both edges are
/// consensus history the moment mainnet crosses them (INC-I-054).
#[test]
fn req_vest_003_the_window_is_inclusive_at_activation_and_exclusive_at_disable() {
    for height in [AH, AH + 1, ARMED, DISABLE - 1] {
        assert!(
            check(EXPECTED_BOUND + 1, &BONDS, NON_BOND, height).is_err(),
            "height {height} is inside [{AH}, {DISABLE}) and must reject an over-value \
             payout"
        );
        assert!(
            check(EXPECTED_BOUND, &BONDS, NON_BOND, height).is_ok(),
            "height {height}: the honest payout stays valid while the rule is armed"
        );
    }
}

// ===========================================================================
// The error surface.
// ===========================================================================

/// REQ-VEST-001 — Decision: a failure says an error CODE changed. The node bails with
/// `"[{}] …", e.error_code()`, so these strings are the operator-facing contract and
/// the dimension every alert and log filter matches on. `WithdrawalBondExtraDataMalformed`
/// is constructed but not yet raised anywhere: M3 (INV-VEST-013, fail closed on a
/// malformed spent Bond) is its first caller, and M2 only pins that it exists.
#[test]
fn req_vest_001_the_three_new_error_codes_are_the_stable_econ_strings() {
    assert_eq!(
        ValidationError::WithdrawalPayoutExceedsVestedNet {
            payout: 2,
            bound: 1,
            bond_inputs: 1,
            non_bond_value: 0,
        }
        .error_code(),
        "ECON_WITHDRAWAL_PAYOUT_EXCEEDS_NET"
    );
    assert_eq!(
        ValidationError::VestingQuarterInvalid { quarter_slots: 0 }.error_code(),
        "ECON_VESTING_QUARTER_INVALID"
    );
    assert_eq!(
        ValidationError::WithdrawalBondExtraDataMalformed {
            input_index: 3,
            len: 2,
        }
        .error_code(),
        "ECON_WITHDRAWAL_BOND_EXTRA_DATA_MALFORMED"
    );
}

/// REQ-VEST-001 — Decision: a failure says a variant was added to the enum but not to
/// the `to_structured_json` match, so the RPC surface reports the code with none of its
/// terms and an operator cannot tell an over-value withdrawal from a mis-set param
/// without reading the node's own logs. `AddBondCapExceeded` (`error.rs:431,519,727`) is
/// the shape being mirrored.
#[test]
fn req_vest_001_structured_json_carries_every_term_of_each_new_error() {
    let over = ValidationError::WithdrawalPayoutExceedsVestedNet {
        payout: 9,
        bound: 8,
        bond_inputs: 2,
        non_bond_value: 7,
    }
    .to_structured_json();
    assert_eq!(over["error_code"], "ECON_WITHDRAWAL_PAYOUT_EXCEEDS_NET");
    assert_eq!(over["payout"], 9);
    assert_eq!(over["bound"], 8);
    assert_eq!(over["bond_inputs"], 2);
    assert_eq!(over["non_bond_value"], 7);

    let bad_quarter =
        ValidationError::VestingQuarterInvalid { quarter_slots: 0 }.to_structured_json();
    assert_eq!(bad_quarter["error_code"], "ECON_VESTING_QUARTER_INVALID");
    assert_eq!(bad_quarter["quarter_slots"], 0);

    let malformed = ValidationError::WithdrawalBondExtraDataMalformed {
        input_index: 3,
        len: 2,
    }
    .to_structured_json();
    assert_eq!(
        malformed["error_code"],
        "ECON_WITHDRAWAL_BOND_EXTRA_DATA_MALFORMED"
    );
    assert_eq!(malformed["input_index"], 3);
    assert_eq!(malformed["len"], 2);
}

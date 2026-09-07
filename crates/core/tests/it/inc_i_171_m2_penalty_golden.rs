//! INC-I-171 M2 — `penalized_bond_net`: the per-bond vesting arithmetic.
//!
// covers: crates/core/src/validation/vesting.rs, crates/core/src/consensus/constants.rs:430-437, bins/cli/src/cmd_producer/common.rs:33-38
//!
//! Requirements: **REQ-VEST-001** (Must — a payout is bounded by the penalized net of
//! the Bond inputs it spends), **REQ-VEST-004** (Must — integer-only, no panic, no
//! overflow) and **REQ-VEST-009** (Should — the released CLI stays valid across
//! activation). Pins INV-VEST-004 and the groundwork for INV-VEST-005.
//!
//! TDD RED, EXPECTED: this module does not compile against the tree at HEAD —
//! `doli_core::validation::vesting` does not exist. That compile failure is the red,
//! exactly as `inc_i_208_activation_height.rs` documents for itself.
//!
//! WHY ROUNDING ORDER IS THE WHOLE FILE. The bound the node enforces and the payout the
//! released CLI computes must agree to the base unit, or every honest withdrawal is
//! rejected at the activation height. The CLI truncates the PENALTY and subtracts
//! (`bins/cli/src/cmd_producer/common.rs:33-38`); a node that truncates the NET instead
//! is off by one on 101 @ 75% (26 vs 25).
//!
//! CLI PARITY IS A RE-DERIVATION, NOT A CALL. `compute_fifo_breakdown` is `pub(super)`,
//! so no test crate can reach it. [`cli_honest_net`] re-derives the expression inline.
//! Follow-up: M7 (REQ-VEST-002) makes the CLI call `penalized_bond_net`, and the
//! re-derivation can be deleted then.

// OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//
//   F1: penalized_bond_net(amount, creation_slot, block_slot, quarter_slots) -> u64
//       PURE free fn. Mutable params: NONE. Receiver/self mutation: NONE. Persistent
//       store writes: NONE. (Declared, not left unmentioned.)
//       O1: the u64 return value — the only observable output.
//       PATHS (ladder at constants.rs:430-437, plus the age computation):
//         P-Q1  quarters == 0   -> pct 75
//         P-Q2  quarters == 1   -> pct 50
//         P-Q3  quarters == 2   -> pct 25
//         P-Q4  quarters >= 3   -> pct 0
//         P-SAT block_slot < creation_slot -> saturating age 0 -> P-Q1
//   MATRIX: O1 x {P-Q1, P-Q2, P-Q3, P-Q4, P-SAT} — every cell is asserted below.
//   INPUT PARTITIONS on `amount`: 101 (the golden, truncation-visible at 75%),
//     1_000_007 (truncation-visible at 75/50/25), u64::MAX (a u64 multiply wraps),
//     and four-tier CLI-parity sets.
//   NOT CLAIMED HERE: the payout bound, the activation gate and the error surface
//     (`inc_i_171_m2_payout_bound.rs`); the shipped params
//     (`inc_i_171_m2_activation_height.rs`).

use doli_core::validation::vesting::penalized_bond_net;

/// A small quarter so a whole ladder fits in readable slot numbers. Deliberately none of
/// the shipped values (mainnet 3_153_600 / testnet 2_160 / devnet 60).
const Q: u32 = 100;

/// Truncation-visible at 75%, 50% and 25%: every tier's penalty has a non-zero fraction,
/// so a rounding change moves the expected net.
const AMOUNT: u64 = 1_000_007;

/// REQ-VEST-001 — Decision: a failure says the implementation truncates the NET instead
/// of the PENALTY. 101 @ 75% is 26 penalty-first and 25 net-first; the released CLI pays
/// 26, so a net-first node rejects every honest Q1 withdrawal from the activation height
/// onward. Golden vector of `specs/vesting-penalty-consensus-architecture.md:259`.
#[test]
fn req_vest_001_penalty_first_truncation_101_at_75_nets_26() {
    assert_eq!(
        penalized_bond_net(101, 0, 0, Q),
        26,
        "penalty = 101*75/100 = 75 (truncated from 75.75), net = 101 - 75 = 26. A \
         net-first implementation returns 101*25/100 = 25 and rejects the honest CLI."
    );
}

/// REQ-VEST-001 — Decision: a failure says a quarter boundary moved. `quarters = age /
/// quarter_slots` (constants.rs:430-437) makes each boundary INCLUSIVE at its lower
/// edge: age == Q is already Q2. An off-by-one charges an honest producer a whole tier.
#[test]
fn req_vest_001_each_quarter_boundary_selects_the_expected_tier() {
    // creation_slot is 0 throughout, so `block_slot` IS the age.
    for (block_slot, pct, expected_net) in [
        (Q - 1, 75u8, 250_002u64),
        (Q, 50, 500_004),
        (2 * Q - 1, 50, 500_004),
        (2 * Q, 25, 750_006),
        (3 * Q - 1, 25, 750_006),
        (3 * Q, 0, AMOUNT),
    ] {
        assert_eq!(
            penalized_bond_net(AMOUNT, 0, block_slot, Q),
            expected_net,
            "age {block_slot} with quarter {Q} is the {pct}% tier: \
             penalty = {AMOUNT}*{pct}/100, net = {AMOUNT} - penalty"
        );
    }
}

/// REQ-VEST-001 — Decision: a failure says `creation_slot = 0` is read as "unknown" or
/// "never vests" instead of "created at genesis". Every bonded producer on the live
/// testnet, and mainnet's genesis producers, hold `creationSlot: 0` bonds; if those
/// never vest, activation confiscates 75% of the founding stake forever.
#[test]
fn req_vest_001_genesis_bond_creation_slot_zero_ages_from_genesis() {
    assert_eq!(
        penalized_bond_net(AMOUNT, 0, 3 * Q, Q),
        AMOUNT,
        "a genesis bond at age 3Q is fully vested: pct 0, net == amount"
    );
    assert_ne!(
        penalized_bond_net(AMOUNT, 0, 3 * Q - 1, Q),
        AMOUNT,
        "anti-vacuity: one slot earlier is still the 25% tier, so the assertion above \
         cannot pass because the fn ignores its inputs"
    );
}

/// REQ-VEST-004 — Decision: a failure is a `u32` subtract-overflow panic inside
/// consensus validation. A reorg can re-mine a withdrawal into an EARLIER slot than the
/// one that stamped the Bond, so `block_slot < creation_slot` is reachable on a chain
/// that reorganises at all — a panic there halts the node instead of rejecting a block.
#[test]
fn req_vest_004_block_slot_before_creation_slot_saturates_to_age_zero() {
    assert_eq!(
        penalized_bond_net(101, 500, 100, Q),
        26,
        "age saturates to 0, which is the 75% tier"
    );
    assert_eq!(
        penalized_bond_net(101, u32::MAX, 0, Q),
        26,
        "the widest possible underflow still saturates to age 0"
    );
}

/// REQ-VEST-004 — Decision: a failure is either an overflow panic (debug) or a WRAPPED
/// bound (release) on `amount * 75`. `u64::MAX * 75` does not fit in u64, so the
/// multiply must happen in u128. A wrapped bound is the worse outcome: it silently
/// raises the ceiling and lets an over-value withdrawal through.
#[test]
fn req_vest_004_u64_max_amount_does_not_overflow_at_any_tier() {
    // 0.75 * (2^64 - 1) truncates to 13_835_058_055_282_163_711, so the net is 2^62.
    assert_eq!(
        penalized_bond_net(u64::MAX, 0, 0, Q),
        1u64 << 62,
        "u64::MAX at the 75% tier nets exactly 2^62 when the multiply is widened to u128"
    );
    assert_eq!(
        penalized_bond_net(u64::MAX, 0, 3 * Q, Q),
        u64::MAX,
        "u64::MAX fully vested nets itself — no truncation, no saturation"
    );
}

/// The released CLI's payout arithmetic, re-derived verbatim from
/// `bins/cli/src/cmd_producer/common.rs:33-38`:
///
/// ```text
/// let pct = entry.penalty_pct;
/// let penalty = (entry.amount * pct as u64) / 100;
/// let net = entry.amount - penalty;
/// ```
///
/// Re-derived, not called: `compute_fifo_breakdown` is `pub(super)`. Callers must keep
/// `amount * 75` inside u64 — the CLI itself would overflow otherwise.
fn cli_honest_net(amount: u64, pct: u8) -> u64 {
    let penalty = (amount * pct as u64) / 100;
    amount - penalty
}

/// REQ-VEST-009 — Decision: a failure says the consensus bound and the released CLI
/// disagree on at least one tier, so an unmodified wallet builds a withdrawal the
/// network rejects at the activation height — an unannounced mandatory CLI upgrade for
/// every producer. INV-VEST-005 groundwork. Slack must be EXACTLY 0: the two
/// expressions are the same expression, only widened to u128 on the node side.
#[test]
fn req_vest_009_bound_equals_the_released_cli_payout_at_every_tier() {
    // Small enough that the CLI's u64 multiply cannot overflow; each amount leaves a
    // non-zero truncation remainder at at least one tier.
    const BONDS: [u64; 4] = [1_000_007, 3, 999_999_999, 250_001];

    for (block_slot, pct) in [(0u32, 75u8), (Q, 50), (2 * Q, 25), (3 * Q, 0)] {
        let node_bound: u64 = BONDS
            .iter()
            .map(|amount| penalized_bond_net(*amount, 0, block_slot, Q))
            .sum();
        let cli_payout: u64 = BONDS
            .iter()
            .map(|amount| cli_honest_net(*amount, pct))
            .sum();

        assert_eq!(
            node_bound, cli_payout,
            "tier {pct}%: the consensus bound and the released CLI payout must be the \
             same integer, not merely close"
        );
    }
}

/// REQ-VEST-009 — Decision: a failure says the bound sits BELOW what the honest CLI
/// pays for at least one amount, which rejects an honest withdrawal. This is the
/// direction the consensus rule actually needs and it survives if the exact-equality
/// test above is ever relaxed; a net-first implementation fails here too, because
/// net-first truncation always returns the smaller net.
#[test]
fn req_vest_009_the_bound_never_falls_below_the_released_cli_payout() {
    for amount in [
        1u64,
        3,
        99,
        101,
        1_000_007,
        250_001,
        999_999_999,
        4_000_000_001,
    ] {
        for (block_slot, pct) in [(0u32, 75u8), (Q, 50), (2 * Q, 25), (3 * Q, 0)] {
            assert!(
                penalized_bond_net(amount, 0, block_slot, Q) >= cli_honest_net(amount, pct),
                "amount {amount} at tier {pct}%: the bound must never be below the \
                 payout the released CLI computes for the same bond"
            );
        }
    }
}

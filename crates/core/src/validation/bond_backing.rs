//! Bond-backing rule — a declared `bond_count` must be backed by locked value.
//!
//! # The hole this closes (exploited on mainnet)
//!
//! A producer's selection weight (its share of block slots and of every
//! epoch's reward pool) is its `bond_count`. Before this rule, `bond_count`
//! was taken verbatim from the `Registration` / `AddBond` transaction and was
//! **never cross-checked against the collateral the transaction actually
//! locked**. Registration only checked `total_locked >= one bond unit` and
//! `1 <= bond_count <= MAX_BONDS_PER_PRODUCER`; nothing tied the two together.
//!
//! An attacker locked a single bond unit (10 DOLI on mainnet) while declaring
//! `bond_count = 3000` (the cap) and was scheduled and paid as if they had
//! staked 30 000 DOLI — a ~3000x weight amplification from a 10 DOLI stake.
//!
//! # The rule
//!
//! At and after `bond_backing_activation_height`, for both `Registration` and
//! `AddBond`:
//!
//! ```text
//! declared_bond_count * bond_unit == locked_total
//! ```
//!
//! with **checked** multiplication (an overflow is treated as a mismatch, not a
//! panic and not a wrap). `locked_total` is the sum of the transaction's Bond
//! output amounts. Because [`check_bond_outputs`](super::unbonding::check_bond_outputs)
//! independently forces every Bond output to be worth exactly one `bond_unit`,
//! equality here is the same as "declared count == number of Bond outputs",
//! but we assert the value relation directly so the rule stands on its own.
//!
//! ## Why equality, not `>=`
//!
//! `>=` ("you may lock more than you declare") would let a producer under-count
//! their own weight — harmless to others — but it also silently tolerates a
//! `bond_unit` that does not divide `locked_total`, leaving stranded value and
//! an ambiguous count. `<=` is the exploit itself. Equality is the only
//! relation under which the on-chain weight equals the collateral exactly, in
//! both directions, with no remainder. It also matches the storage model: the
//! epoch flush creates exactly `locked_total / bond_unit` bond entries.
//!
//! # Activation gating (INC-I-075 three-question gate)
//!
//! Q1 YES (Registration/AddBond are user-submittable), Q2 YES (the paired epoch
//! clamp mutates the ProducerSet), Q3 NO (post-AH rejects txs previously
//! accepted, and the clamp changes producer weights) ⇒ an activation height is
//! REQUIRED. Pre-activation this function returns `Ok(())` unconditionally so
//! replaying historical blocks stays bit-identical.

use crate::validation::ValidationError;

/// Verify that `declared_bond_count` is exactly backed by `locked_total` at the
/// network's `bond_unit`, height-gated by `activation_height`.
///
/// * **Pre-activation** (`height < activation_height`): always `Ok(())`
///   (replay safety — no consensus change below the gate).
/// * **Post-activation**: `Ok(())` iff
///   `declared_bond_count.checked_mul(bond_unit) == Some(locked_total)`.
///   A multiply overflow (only reachable with an absurd `bond_unit`) is a
///   mismatch, never a panic or a wrap.
///
/// `bond_count == 0` and `bond_count > MAX_BONDS_PER_PRODUCER` are rejected by
/// the existing structural checks before this runs; a zero here still fails the
/// equality for any non-zero `locked_total`, so the rule is self-contained.
#[inline]
pub fn check_bond_backing(
    declared_bond_count: u32,
    bond_unit: u64,
    locked_total: u64,
    height: u64,
    activation_height: u64,
) -> Result<(), ValidationError> {
    // Pre-activation gate dominates — historical blocks replay unchanged.
    if height < activation_height {
        return Ok(());
    }

    // Checked multiply. An overflow is impossible for any real input
    // (`MAX_BONDS_PER_PRODUCER * bond_unit` << `u64::MAX`), so it can only come
    // from an adversarial/absurd `bond_unit`; we treat it as a mismatch with a
    // `u64::MAX` sentinel rather than panicking or wrapping. Kept as an
    // explicit `match` (not `unwrap_or`) so the arithmetic is genuinely checked
    // and cannot be folded into a silent saturating multiply.
    let expected_locked = match u64::from(declared_bond_count).checked_mul(bond_unit) {
        Some(v) => v,
        None => {
            return Err(ValidationError::BondBackingMismatch {
                declared_bond_count,
                bond_unit,
                locked_total,
                expected_locked: u64::MAX,
            });
        }
    };

    if expected_locked == locked_total {
        Ok(())
    } else {
        Err(ValidationError::BondBackingMismatch {
            declared_bond_count,
            bond_unit,
            locked_total,
            expected_locked,
        })
    }
}

/// Sum of a transaction's Bond-output amounts (saturating), the value a
/// Registration/AddBond actually locks. Mirrors the apply path, which creates
/// bond entries from the same Bond outputs.
pub fn locked_bond_total(tx: &crate::transaction::Transaction) -> u64 {
    use crate::transaction::OutputType;
    tx.outputs
        .iter()
        .filter(|o| o.output_type == OutputType::Bond)
        .map(|o| o.amount)
        .fold(0u64, |acc, a| acc.saturating_add(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNIT: u64 = 1_000_000_000; // mainnet bond_unit (10 DOLI)
    const AH: u64 = 1_000;

    #[test]
    fn accepts_exact_backing() {
        // 3000 bonds backed by 3000 * unit locked.
        assert!(check_bond_backing(3000, UNIT, 3000 * UNIT, AH, AH).is_ok());
        // single-bond producer.
        assert!(check_bond_backing(1, UNIT, UNIT, AH, AH).is_ok());
    }

    #[test]
    fn rejects_the_exploit() {
        // The on-chain attack: declare 3000, lock only one unit.
        let err = check_bond_backing(3000, UNIT, UNIT, AH, AH).unwrap_err();
        match err {
            ValidationError::BondBackingMismatch {
                declared_bond_count,
                locked_total,
                expected_locked,
                ..
            } => {
                assert_eq!(declared_bond_count, 3000);
                assert_eq!(locked_total, UNIT);
                assert_eq!(expected_locked, 3000 * UNIT);
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn rejects_overdeclare_by_one() {
        assert!(check_bond_backing(2, UNIT, UNIT, AH, AH).is_err());
    }

    #[test]
    fn rejects_underdeclare() {
        // Locking more than declared is also a mismatch (no remainder tolerated).
        assert!(check_bond_backing(1, UNIT, 2 * UNIT, AH, AH).is_err());
    }

    #[test]
    fn rejects_non_multiple_locked() {
        assert!(check_bond_backing(1, UNIT, UNIT + 1, AH, AH).is_err());
    }

    #[test]
    fn overflow_is_a_mismatch_not_a_panic() {
        // declared * unit overflows u64 -> sentinel u64::MAX -> mismatch.
        let err = check_bond_backing(u32::MAX, u64::MAX, 0, AH, AH).unwrap_err();
        match err {
            ValidationError::BondBackingMismatch {
                expected_locked, ..
            } => assert_eq!(expected_locked, u64::MAX),
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn pre_activation_is_a_noop() {
        // Below the gate the exploit tx passes this function (replay safety);
        // the paired epoch clamp is what neutralizes already-registered cheats.
        assert!(check_bond_backing(3000, UNIT, UNIT, AH - 1, AH).is_ok());
    }

    #[test]
    fn boundary_height_is_enforced() {
        // The gate is `>=`: at exactly the activation height the rule is live.
        assert!(check_bond_backing(3000, UNIT, UNIT, AH, AH).is_err());
    }
}

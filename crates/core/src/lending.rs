//! Lending mathematics.
//!
//! All computations use integer arithmetic (u64 values, u128 intermediates).

use crate::types::Amount;

/// Slots per year (10-second slots, ~365.25 days)
pub const SLOTS_PER_YEAR: u64 = 3_155_760;

/// Compute accrued interest.
/// `interest = principal * rate_bps * elapsed_slots / (10000 * SLOTS_PER_YEAR)`
pub fn compute_interest(principal: Amount, rate_bps: u16, elapsed_slots: u64) -> Amount {
    if principal == 0 || rate_bps == 0 || elapsed_slots == 0 {
        return 0;
    }
    let n = (principal as u128) * (rate_bps as u128) * (elapsed_slots as u128);
    let d = 10000u128 * (SLOTS_PER_YEAR as u128);
    (n / d) as Amount
}

/// Total debt = principal + accrued interest.
pub fn compute_total_debt(principal: Amount, rate_bps: u16, elapsed_slots: u64) -> Amount {
    principal + compute_interest(principal, rate_bps, elapsed_slots)
}

/// LTV in basis points: `ltv = debt * 10000 / collateral_value`.
pub fn compute_ltv_bps(debt: Amount, collateral_value: Amount) -> u16 {
    if collateral_value == 0 {
        return u16::MAX;
    }
    let ltv = ((debt as u128) * 10000) / (collateral_value as u128);
    ltv.min(u16::MAX as u128) as u16
}

/// Check if liquidatable: `collateral_value * 10000 < debt * liquidation_ratio_bps`.
pub fn is_liquidatable(debt: Amount, collateral_value: Amount, liquidation_ratio_bps: u16) -> bool {
    let lhs = (collateral_value as u128) * 10000;
    let rhs = (debt as u128) * (liquidation_ratio_bps as u128);
    lhs < rhs
}

/// Collateral value in DOLI using TWAP fixed-point price (<<64).
pub fn collateral_value_from_twap(collateral_amount: Amount, twap_price_fixed: u128) -> Amount {
    (((collateral_amount as u128) * twap_price_fixed) >> 64) as Amount
}

/// Verify creation LTV is within maximum.
/// Returns `Ok(ltv)` if within bounds, `Err(ltv)` if over.
pub fn verify_creation_ltv(
    principal: Amount,
    collateral_value: Amount,
    max_ltv_bps: u16,
) -> Result<u16, u16> {
    let ltv = compute_ltv_bps(principal, collateral_value);
    if ltv > max_ltv_bps {
        Err(ltv)
    } else {
        Ok(ltv)
    }
}

/// Compute depositor's share of interest earned.
/// `share = total_interest * depositor_amount / total_deposits`
pub fn compute_depositor_earnings(
    depositor_amount: Amount,
    total_deposits: Amount,
    total_interest_earned: Amount,
) -> Amount {
    if total_deposits == 0 {
        return 0;
    }
    ((depositor_amount as u128) * (total_interest_earned as u128) / (total_deposits as u128))
        as Amount
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interest_one_year() {
        // 5% on 1000 DOLI (in base units: 1000 * 10^8 = 100_000_000_000)
        let principal = 100_000_000_000u64; // 1000 DOLI
        let rate_bps = 500; // 5%
        let elapsed = SLOTS_PER_YEAR;
        let interest = compute_interest(principal, rate_bps, elapsed);
        // Expected: 1000 * 0.05 = 50 DOLI = 5_000_000_000
        assert_eq!(interest, 5_000_000_000);
    }

    #[test]
    fn test_interest_one_day() {
        let principal = 100_000_000_000u64; // 1000 DOLI
        let rate_bps = 500; // 5%
        let slots_per_day = 8640; // 86400 seconds / 10
        let interest = compute_interest(principal, rate_bps, slots_per_day);
        // ~1000 * 0.05 / 365.25 = ~0.1369 DOLI = ~13_689_253 units
        // Allow small rounding: integer division truncates
        assert!(interest > 13_000_000 && interest < 14_000_000);
    }

    #[test]
    fn test_interest_zero_cases() {
        assert_eq!(compute_interest(0, 500, SLOTS_PER_YEAR), 0);
        assert_eq!(compute_interest(100_000_000_000, 0, SLOTS_PER_YEAR), 0);
        assert_eq!(compute_interest(100_000_000_000, 500, 0), 0);
    }

    #[test]
    fn test_total_debt() {
        let principal = 100_000_000_000u64;
        let rate_bps = 500;
        let elapsed = SLOTS_PER_YEAR;
        let debt = compute_total_debt(principal, rate_bps, elapsed);
        assert_eq!(debt, 105_000_000_000);
    }

    #[test]
    fn test_ltv_calculation() {
        // 100 debt / 150 collateral = 66.67% = 6666 bps
        let ltv = compute_ltv_bps(100, 150);
        assert_eq!(ltv, 6666);
    }

    #[test]
    fn test_ltv_zero_collateral() {
        assert_eq!(compute_ltv_bps(100, 0), u16::MAX);
    }

    #[test]
    fn test_liquidation_healthy() {
        // Collateral 150, debt 100, liquidation ratio 150% (15000 bps)
        // 150 * 10000 = 1_500_000 vs 100 * 15000 = 1_500_000 => NOT liquidatable (equal)
        assert!(!is_liquidatable(100, 150, 15000));
    }

    #[test]
    fn test_liquidation_unhealthy() {
        // Collateral 140, debt 100, liquidation ratio 150%
        // 140 * 10000 = 1_400_000 < 100 * 15000 = 1_500_000 => liquidatable
        assert!(is_liquidatable(100, 140, 15000));
    }

    #[test]
    fn test_liquidation_at_threshold() {
        // Exact boundary: collateral 120, debt 100, ratio 120%
        // 120 * 10000 = 1_200_000 vs 100 * 12000 = 1_200_000 => NOT liquidatable
        assert!(!is_liquidatable(100, 120, 12000));
        // Just below: collateral 119
        // 119 * 10000 = 1_190_000 < 100 * 12000 = 1_200_000 => liquidatable
        assert!(is_liquidatable(100, 119, 12000));
    }

    #[test]
    fn test_collateral_value_from_twap() {
        // Price = 2.0 in fixed-point: 2 << 64 = 2 * 18446744073709551616
        let price_fp = 2u128 << 64;
        let value = collateral_value_from_twap(500, price_fp);
        assert_eq!(value, 1000);
    }

    #[test]
    fn test_verify_creation_ltv_ok() {
        // 50% LTV passes 66.67% max
        let result = verify_creation_ltv(50, 100, 6667);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 5000);
    }

    #[test]
    fn test_verify_creation_ltv_rejected() {
        // 71% LTV fails 66.67% max
        let result = verify_creation_ltv(71, 100, 6667);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), 7100);
    }

    #[test]
    fn test_interest_accrual_makes_position_liquidatable() {
        // Start: 100 principal, 150 collateral, 150% ratio => healthy
        let principal = 100_000_000_000u64; // 100 DOLI principal
        let collateral_value = 150_000_000_000u64; // 150 DOLI collateral value
        assert!(!is_liquidatable(principal, collateral_value, 15000));

        // After many years at 50% interest, debt >> collateral
        let debt = compute_total_debt(principal, 5000, SLOTS_PER_YEAR * 5);
        // debt = 100 + 100*5*0.5 = 350 DOLI
        assert!(is_liquidatable(debt, collateral_value, 15000));
    }

    #[test]
    fn test_depositor_earnings_proportional() {
        // Alice deposited 300, Bob deposited 700 (total 1000). Interest earned: 100.
        let alice_share = compute_depositor_earnings(300, 1000, 100);
        let bob_share = compute_depositor_earnings(700, 1000, 100);
        assert_eq!(alice_share, 30);
        assert_eq!(bob_share, 70);
    }

    #[test]
    fn test_depositor_earnings_zero_pool() {
        assert_eq!(compute_depositor_earnings(100, 0, 50), 0);
    }

    #[test]
    fn test_collateral_output_roundtrip() {
        use crate::transaction::Output;
        let pool_id = crypto::Hash::from_bytes([0xAA; 32]);
        let borrower = crypto::Hash::from_bytes([0xBB; 32]);
        let asset_id = crypto::Hash::from_bytes([0xCC; 32]);
        let output = Output::collateral(500, pool_id, borrower, 100, 500, 42, 15000, asset_id);
        let meta = output.collateral_metadata().unwrap();
        assert_eq!(meta.pool_id, pool_id);
        assert_eq!(meta.borrower_hash, borrower);
        assert_eq!(meta.principal, 100);
        assert_eq!(meta.interest_rate_bps, 500);
        assert_eq!(meta.creation_slot, 42);
        assert_eq!(meta.liquidation_ratio_bps, 15000);
        assert_eq!(meta.collateral_asset_id, asset_id);
    }

    #[test]
    fn test_lending_deposit_output_roundtrip() {
        use crate::transaction::Output;
        let pool_id = crypto::Hash::from_bytes([0xDD; 32]);
        let depositor = crypto::Hash::from_bytes([0xEE; 32]);
        let output = Output::lending_deposit(1000, pool_id, depositor, 99);
        let meta = output.lending_deposit_metadata().unwrap();
        assert_eq!(meta.lending_pool_id, pool_id);
        assert_eq!(meta.deposit_slot, 99);
        assert_eq!(output.amount, 1000);
        assert_eq!(output.pubkey_hash, depositor);
    }
}

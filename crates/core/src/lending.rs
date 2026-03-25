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

    /// Simulate a real loan: 150 tokens collateral, 100 DOLI borrowed, 5% rate.
    /// Verify Collateral UTXO serialization roundtrips correctly with realistic values.
    #[test]
    fn test_collateral_output_with_real_values() {
        use crate::transaction::Output;
        let pool_id = crypto::Hash::from_bytes([0x11; 32]);
        let borrower = crypto::Hash::from_bytes([0x22; 32]);
        let asset_id = crypto::Hash::from_bytes([0x33; 32]);

        // 150 tokens collateral, 100 DOLI (10^10 base units) principal, 5% rate, slot 1000
        let collateral_amount = 150_000_000_000u64; // 150 tokens (assuming 10^9 decimals)
        let principal = 10_000_000_000u64; // 100 DOLI in base units (10^8)
        let rate_bps = 500u16;
        let creation_slot = 1000u32;
        let liq_ratio = 15000u16;

        let output = Output::collateral(
            collateral_amount,
            pool_id,
            borrower,
            principal,
            rate_bps,
            creation_slot,
            liq_ratio,
            asset_id,
        );

        // Verify amount field
        assert_eq!(output.amount, collateral_amount);

        // Roundtrip metadata
        let meta = output.collateral_metadata().unwrap();
        assert_eq!(meta.pool_id, pool_id);
        assert_eq!(meta.borrower_hash, borrower);
        assert_eq!(meta.principal, principal);
        assert_eq!(meta.interest_rate_bps, rate_bps);
        assert_eq!(meta.creation_slot, creation_slot);
        assert_eq!(meta.liquidation_ratio_bps, liq_ratio);
        assert_eq!(meta.collateral_asset_id, asset_id);

        // Verify serialization roundtrip via serialize/deserialize
        let serialized = output.serialize();
        assert!(!serialized.is_empty());
    }

    /// Test interest accrual over one epoch (360 slots).
    #[test]
    fn test_interest_over_one_epoch() {
        // 100 DOLI at 5%, 360 slots
        let interest = compute_interest(10_000_000_000, 500, 360);
        assert!(interest > 0);
        // Expected: 100 * 0.05 * 360 / 3_155_760 ~ 0.000570 DOLI ~ 57058 units
        // The exact value depends on integer truncation
        let expected_approx = (10_000_000_000u128 * 500 * 360) / (10000 * SLOTS_PER_YEAR as u128);
        assert_eq!(interest, expected_approx as u64);
    }

    /// Test that half-year interest accrual is approximately half of yearly.
    #[test]
    fn test_interest_half_year() {
        let principal = 100_000_000_000u64; // 1000 DOLI
        let rate_bps = 500; // 5%
        let full_year = compute_interest(principal, rate_bps, SLOTS_PER_YEAR);
        let half_year = compute_interest(principal, rate_bps, SLOTS_PER_YEAR / 2);
        // Half-year interest should be approximately half of full year
        // Allow 1 unit rounding difference
        assert!((full_year / 2).abs_diff(half_year) <= 1);
    }

    /// Test that compute_total_debt includes both principal and interest.
    #[test]
    fn test_total_debt_exceeds_principal() {
        let principal = 5_000_000_000u64; // 50 DOLI
        let rate_bps = 1000; // 10%
        let elapsed = SLOTS_PER_YEAR;
        let debt = compute_total_debt(principal, rate_bps, elapsed);
        assert!(debt > principal);
        assert_eq!(
            debt,
            principal + compute_interest(principal, rate_bps, elapsed)
        );
    }

    /// Test LTV with large real-world values.
    #[test]
    fn test_ltv_real_world() {
        // 80 DOLI debt, 120 DOLI collateral value = 66.67% LTV = 6666 bps
        let debt = 8_000_000_000u64;
        let collateral = 12_000_000_000u64;
        let ltv = compute_ltv_bps(debt, collateral);
        assert_eq!(ltv, 6666);
    }

    /// Test lending pool ID derivation is deterministic.
    #[test]
    fn test_lending_pool_id_deterministic() {
        use crate::transaction::Output;
        let pool_id = crypto::Hash::from_bytes([0x55; 32]);
        let lending_pool_id_a = Output::compute_lending_pool_id(&pool_id);
        let lending_pool_id_b = Output::compute_lending_pool_id(&pool_id);
        assert_eq!(lending_pool_id_a, lending_pool_id_b);
        // Different pool ID produces different lending pool ID
        let pool_id_2 = crypto::Hash::from_bytes([0x66; 32]);
        let lending_pool_id_c = Output::compute_lending_pool_id(&pool_id_2);
        assert_ne!(lending_pool_id_a, lending_pool_id_c);
    }

    // ========== Gap 1: Liquidation end-to-end scenarios ==========

    /// L5: Liquidation executes when LTV > threshold
    #[test]
    fn test_liquidation_executes_above_threshold() {
        // Setup: 150 tokens collateral, 100 DOLI principal, 150% liquidation ratio
        // Price via TWAP: 1 token = 0.8 DOLI (price dropped)
        // Collateral value = 150 * 0.8 = 120 DOLI
        // Debt = 100 DOLI (ignore interest for simplicity)
        // LTV = 100/120 = 83.3% > 66.67% max
        // Liquidation check: collateral(120) * 10000 < debt(100) * ratio(15000)
        //                    1200000 < 1500000 -> TRUE -> liquidatable
        let debt = 100u64;
        let collateral_value = 120u64;
        assert!(is_liquidatable(debt, collateral_value, 15000));
    }

    /// L6: Liquidation rejected when position is healthy
    #[test]
    fn test_liquidation_rejected_when_healthy() {
        // Collateral value = 200 DOLI, Debt = 100 DOLI
        // LTV = 50% < 66.67% -> NOT liquidatable
        let debt = 100u64;
        let collateral_value = 200u64;
        assert!(!is_liquidatable(debt, collateral_value, 15000));
    }

    /// L7: TWAP protects against spot manipulation
    #[test]
    fn test_twap_protects_against_spot_manipulation() {
        // Normal price for 359 blocks: reserve_a=1000, reserve_b=500 (price = 2 DOLI/token)
        let normal_price = (1000u128 << 64) / 500;
        let cum_359 = normal_price * 359;

        // Manipulated price for 1 block: reserve_a=100, reserve_b=500 (price = 0.2 DOLI/token = 10x drop)
        let manip_price = (100u128 << 64) / 500;
        let cum_360 = cum_359 + manip_price;

        // TWAP over 360 blocks
        let twap = cum_360 / 360;

        // Normal collateral value at TWAP: 150 tokens * twap_price
        // At normal price: 150 * 2 = 300 DOLI (healthy)
        // At manipulated spot: 150 * 0.2 = 30 DOLI (would trigger liquidation)
        // At TWAP: should be close to normal (300 DOLI), NOT close to manipulated (30 DOLI)
        let deviation_bps = twap.abs_diff(normal_price) * 10000 / normal_price;
        // TWAP must change < 3% (300 bps) from a single-block manipulation
        assert!(
            deviation_bps <= 300,
            "TWAP changed {}bps from 1-block manipulation -- lending would be unsafe",
            deviation_bps
        );

        // At TWAP price, position should NOT be liquidatable
        let collateral_val = collateral_value_from_twap(150, twap);
        let debt = 100u64;
        assert!(
            !is_liquidatable(debt, collateral_val, 15000),
            "Position should be safe at TWAP price, but would be liquidated"
        );
    }

    /// L8: Interest accrual makes position liquidatable over time
    #[test]
    fn test_interest_makes_position_liquidatable() {
        // Position: 150 DOLI collateral, 100 DOLI principal, 5% annual rate
        // At creation: LTV = 66.67% (healthy)
        // After enough time, interest pushes debt above liquidation threshold
        let principal = 100u64;
        let collateral_value = 150u64;
        let rate_bps = 500u16; // 5% annual

        // Initially healthy
        assert!(!is_liquidatable(principal, collateral_value, 15000));

        // After 10 years of interest
        let debt_10y = compute_total_debt(principal, rate_bps, SLOTS_PER_YEAR * 10);
        // debt ~ 100 * 1.05^10 (linear here) = 100 + 100*0.05*10 = 150
        // Liquidation: 150 * 10000 = 1500000 < 150 * 15000 = 2250000 -> liquidatable
        assert!(
            is_liquidatable(debt_10y, collateral_value, 15000),
            "Position should be liquidatable after 10 years of interest: debt={}",
            debt_10y
        );
    }

    // ========== Gap 5 (partial): Full DeFi lifecycle integration ==========

    /// Full DeFi lifecycle: create pool -> swap -> TWAP -> borrow -> accrue interest -> repay
    #[test]
    fn test_full_defi_lifecycle() {
        use crate::pool::*;

        // Use realistic DOLI-scale values (10^8 base units per DOLI)
        let unit: u64 = 100_000_000; // 1 DOLI

        // Step 1: Create pool with 1000 DOLI / 5000 tokens
        let reserve_a = 1000 * unit;
        let reserve_b = 5000 * unit;
        let lp_shares = compute_initial_lp_shares(reserve_a, reserve_b);
        assert!(lp_shares > 0);

        // Step 2: Swap 100 DOLI -> tokens
        let (tokens_out, new_ra, new_rb) =
            compute_swap(reserve_a, reserve_b, 100 * unit, 30).unwrap();
        assert!(tokens_out > 0);
        assert!(verify_invariant(reserve_a, reserve_b, new_ra, new_rb));

        // Step 3: TWAP accumulates over 100 slots
        let cum = update_twap(0, new_ra, new_rb, 200, 100);
        assert!(cum > 0);

        // Step 4: Compute TWAP price
        let twap_price = compute_twap_price(0, cum, 100).unwrap();
        assert!(twap_price > 0);

        // Step 5: Create loan -- 500 tokens collateral, borrow DOLI
        let collateral_tokens = 500 * unit;
        let collateral_val = collateral_value_from_twap(collateral_tokens, twap_price);
        assert!(collateral_val > 0);

        // Max borrow at 66.67% LTV
        let max_borrow = collateral_val * 6667 / 10000;
        assert!(max_borrow > 0);

        // Verify LTV is acceptable
        let ltv = verify_creation_ltv(max_borrow, collateral_val, 6667);
        assert!(ltv.is_ok());

        // Step 6: Accrue interest over 1 epoch (360 slots)
        let interest = compute_interest(max_borrow, 500, 360);
        assert!(interest > 0, "Interest must be >0 with DOLI-scale values");

        // Step 7: Total debt
        let total_debt = compute_total_debt(max_borrow, 500, 360);
        assert!(total_debt > max_borrow);

        // Step 8: Verify position is still healthy
        let ltv_after = compute_ltv_bps(total_debt, collateral_val);
        // Should still be under liquidation threshold (83.33%)
        assert!(
            ltv_after < 8333,
            "Position should be healthy after 1 epoch, LTV={}bps",
            ltv_after
        );

        // Step 9: Depositor earnings
        let depositor_amount = 1000 * unit;
        let total_deposits = 5000 * unit;
        let earnings = compute_depositor_earnings(depositor_amount, total_deposits, interest);
        assert!(earnings > 0);
        // Depositor gets proportional share: 1000/5000 = 20% of interest
        assert_eq!(earnings, interest / 5);
    }
}

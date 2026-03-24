//! AMM pool mathematics.
//!
//! All computations use integer arithmetic only (u64 values, u128 intermediates).
//! No floating point. No external dependencies.

use crate::types::Amount;

/// Compute swap output using x*y=k constant product formula.
///
/// `dx` = input amount (before fee)
/// `fee_bps` = fee in basis points (e.g. 30 = 0.3%)
/// Returns `(dy, reserve_a_new, reserve_b_new)` or None if invalid.
pub fn compute_swap(
    reserve_a: Amount,
    reserve_b: Amount,
    dx: Amount,
    fee_bps: u16,
) -> Option<(Amount, Amount, Amount)> {
    if dx == 0 || reserve_a == 0 || reserve_b == 0 {
        return None;
    }
    // dx_eff = dx * (10000 - fee) / 10000
    let dx_eff = (dx as u128) * (10000 - fee_bps as u128) / 10000;
    if dx_eff == 0 {
        return None;
    }
    // dy = reserve_b * dx_eff / (reserve_a + dx_eff)
    let numerator = (reserve_b as u128) * dx_eff;
    let denominator = (reserve_a as u128) + dx_eff;
    let dy = (numerator / denominator) as Amount;
    if dy == 0 || dy >= reserve_b {
        return None;
    }
    let reserve_a_new = reserve_a + dx; // full dx goes in (fee stays in pool)
    let reserve_b_new = reserve_b - dy;
    Some((dy, reserve_a_new, reserve_b_new))
}

/// Compute LP shares to mint for initial liquidity deposit.
/// Returns sqrt(amount_a * amount_b) using integer square root.
pub fn compute_initial_lp_shares(amount_a: Amount, amount_b: Amount) -> Amount {
    isqrt(amount_a as u128 * amount_b as u128) as Amount
}

/// Compute LP shares to mint for subsequent liquidity deposit.
/// shares = min(da * total / ra, db * total / rb)
pub fn compute_lp_shares(
    amount_a: Amount,
    amount_b: Amount,
    reserve_a: Amount,
    reserve_b: Amount,
    total_shares: Amount,
) -> Option<Amount> {
    if reserve_a == 0 || reserve_b == 0 || total_shares == 0 {
        return None;
    }
    let shares_a = (amount_a as u128) * (total_shares as u128) / (reserve_a as u128);
    let shares_b = (amount_b as u128) * (total_shares as u128) / (reserve_b as u128);
    let shares = shares_a.min(shares_b) as Amount;
    if shares == 0 {
        return None;
    }
    Some(shares)
}

/// Compute assets returned when burning LP shares.
/// da = shares * ra / total, db = shares * rb / total
pub fn compute_remove_liquidity(
    shares: Amount,
    reserve_a: Amount,
    reserve_b: Amount,
    total_shares: Amount,
) -> Option<(Amount, Amount)> {
    if total_shares == 0 || shares == 0 || shares > total_shares {
        return None;
    }
    let da = ((shares as u128) * (reserve_a as u128) / (total_shares as u128)) as Amount;
    let db = ((shares as u128) * (reserve_b as u128) / (total_shares as u128)) as Amount;
    Some((da, db))
}

/// Update TWAP cumulative price.
///
/// Uses u128 fixed-point: price = reserve_a << 64 / reserve_b.
/// Accumulates: cum' = cum + price * elapsed_slots.
pub fn update_twap(
    cumulative: u128,
    reserve_a: Amount,
    reserve_b: Amount,
    current_slot: u32,
    last_slot: u32,
) -> u128 {
    if reserve_b == 0 || current_slot <= last_slot {
        return cumulative;
    }
    let elapsed = (current_slot - last_slot) as u128;
    let price = ((reserve_a as u128) << 64) / (reserve_b as u128);
    cumulative.saturating_add(price.saturating_mul(elapsed))
}

/// Compute TWAP price over a window from two cumulative snapshots.
/// Returns price as u128 fixed-point (>> 64 to get integer ratio).
pub fn compute_twap_price(cum_start: u128, cum_end: u128, window_slots: u32) -> Option<u128> {
    if window_slots == 0 || cum_end < cum_start {
        return None;
    }
    Some((cum_end - cum_start) / window_slots as u128)
}

/// Verify the constant product invariant holds after a swap.
/// new_k >= old_k (fees make it grow).
pub fn verify_invariant(
    old_reserve_a: Amount,
    old_reserve_b: Amount,
    new_reserve_a: Amount,
    new_reserve_b: Amount,
) -> bool {
    let old_k = (old_reserve_a as u128) * (old_reserve_b as u128);
    let new_k = (new_reserve_a as u128) * (new_reserve_b as u128);
    new_k >= old_k
}

/// Integer square root (Newton's method).
fn isqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========== Group 1 — Invariant (7 tests) ==========

    #[test]
    fn test_swap_invariant_holds() {
        let (dy, ra_new, rb_new) = compute_swap(1000, 1000, 100, 30).unwrap();
        // dy should be ~90.66 (with 0.3% fee)
        assert!(dy > 0);
        assert!(dy < 100); // can't get more out than k allows
        assert!(verify_invariant(1000, 1000, ra_new, rb_new));
    }

    #[test]
    fn test_swap_invariant_100_random() {
        let mut ra: u64 = 1_000_000;
        let mut rb: u64 = 1_000_000;
        for i in 0..100 {
            let dx = ((i * 7 + 13) % 1000 + 1) as u64; // pseudo-random 1..1000
            let direction = i % 2 == 0;
            if direction {
                if let Some((_dy, ra_n, rb_n)) = compute_swap(ra, rb, dx, 30) {
                    assert!(verify_invariant(ra, rb, ra_n, rb_n));
                    ra = ra_n;
                    rb = rb_n;
                }
            } else if let Some((_dy, rb_n, ra_n)) = compute_swap(rb, ra, dx, 30) {
                assert!(verify_invariant(ra, rb, ra_n, rb_n));
                ra = ra_n;
                rb = rb_n;
            }
        }
        // After 100 swaps with fees, k should have grown
        assert!((ra as u128) * (rb as u128) > 1_000_000u128 * 1_000_000u128);
    }

    #[test]
    fn test_swap_fee_grows_reserves() {
        let (_, ra1, rb1) = compute_swap(1000, 1000, 100, 30).unwrap();
        // With fee, k grows
        let old_k = 1000u128 * 1000;
        let new_k = ra1 as u128 * rb1 as u128;
        assert!(new_k > old_k);
    }

    #[test]
    fn test_swap_price_impact_scales() {
        // Small swap: low impact
        let (dy_small, _, _) = compute_swap(1000, 1000, 10, 30).unwrap();
        let price_small = (10.0 * 1000.0) / (dy_small as f64 * 1000.0); // effective price ratio

        // Large swap: high impact
        let (dy_large, _, _) = compute_swap(1000, 1000, 500, 30).unwrap();
        let price_large = (500.0 * 1000.0) / (dy_large as f64 * 1000.0);

        // Larger swap = worse price (higher ratio)
        assert!(price_large > price_small);
        // 500 into 1000/1000 pool should give < 500 out
        assert!(dy_large < 500);
    }

    #[test]
    fn test_swap_zero_amount_rejected() {
        assert!(compute_swap(1000, 1000, 0, 30).is_none());
        assert!(compute_swap(0, 1000, 100, 30).is_none());
        assert!(compute_swap(1000, 0, 100, 30).is_none());
    }

    #[test]
    fn test_swap_both_directions_invariant() {
        let ra = 1000u64;
        let rb = 2000u64;
        // Swap A -> B
        let (dy, ra1, rb1) = compute_swap(ra, rb, 100, 30).unwrap();
        // Swap B -> A with exactly dy
        let (dx_back, rb2, ra2) = compute_swap(rb1, ra1, dy, 30).unwrap();
        // After round-trip, reserves should be >= original (fees grow k)
        assert!(verify_invariant(ra, rb, ra2, rb2));
        // We get back less than we put in (fees)
        assert!(dx_back < 100);
    }

    #[test]
    fn test_swap_large_never_drains() {
        let (dy, ra_new, rb_new) = compute_swap(1000, 1000, 999, 30).unwrap();
        // reserve_b never reaches 0
        assert!(rb_new > 0);
        // amount_out < input (curve prevents draining)
        assert!(dy < 999);
        // Invariant holds
        assert!(verify_invariant(1000, 1000, ra_new, rb_new));
    }

    // ========== Group 2 — Liquidity (4 tests) ==========

    #[test]
    fn test_add_liquidity_proportional() {
        let shares = compute_lp_shares(100, 200, 1000, 2000, 707).unwrap();
        // 100/1000 = 10%, so shares ~ 70 (10% of 707)
        assert!(shares > 60 && shares < 80);
    }

    #[test]
    fn test_remove_liquidity_roundtrip() {
        let initial_shares = compute_initial_lp_shares(1000, 1000);
        let (da, db) =
            compute_remove_liquidity(initial_shares, 1000, 1000, initial_shares).unwrap();
        // Should get back everything (no fees yet)
        assert_eq!(da, 1000);
        assert_eq!(db, 1000);
    }

    #[test]
    fn test_first_liquidity_sqrt() {
        let shares = compute_initial_lp_shares(1000, 500);
        // sqrt(1000 * 500) = sqrt(500000) ~ 707
        assert!((706..=708).contains(&shares));
    }

    #[test]
    fn test_remove_more_than_owned_fails() {
        assert!(compute_remove_liquidity(1001, 1000, 1000, 1000).is_none());
    }

    // ========== Group 3 — TWAP (5 tests) ==========

    #[test]
    fn test_twap_accumulates_correctly() {
        let cum = update_twap(0, 1000, 1000, 10, 0);
        // price = 1000 << 64 / 1000 = 1 << 64
        // cum = (1 << 64) * 10
        let expected = (1u128 << 64) * 10;
        assert_eq!(cum, expected);
    }

    #[test]
    fn test_twap_resistant_to_single_block() {
        // 359 blocks at price 1:1
        let cum_359 = update_twap(0, 1000, 1000, 359, 0);
        // 1 block at manipulated price 10:1
        let cum_360 = update_twap(cum_359, 10000, 1000, 360, 359);
        // TWAP over 360 blocks
        let twap = compute_twap_price(0, cum_360, 360).unwrap();
        // Normal price = 1 << 64
        let normal_price = 1u128 << 64;
        // TWAP should be very close to normal (within 3%)
        let diff = twap.abs_diff(normal_price);
        let pct = (diff * 100) / normal_price;
        assert!(
            pct <= 3,
            "TWAP changed {}% from single block manipulation",
            pct
        );
    }

    #[test]
    fn test_twap_u128_no_overflow() {
        // Large reserves, many slots — saturating arithmetic prevents panic
        let cum = update_twap(0, u64::MAX / 2, 1, 1_000_000, 0);
        // Should not panic — u128 handles it via saturation
        assert!(cum > 0);
        // Extreme values saturate to u128::MAX rather than panicking
        let cum_extreme = update_twap(0, u64::MAX / 2, 1, u32::MAX, 0);
        assert!(cum_extreme > 0);

        // Moderate-large values that don't saturate still accumulate correctly
        let cum_a = update_twap(0, 1_000_000_000, 1_000, 100, 0);
        let cum_b = update_twap(cum_a, 1_000_000_000, 1_000, 200, 100);
        assert!(cum_b > cum_a);
        // Both windows had the same price, so second half equals first half
        assert_eq!(cum_b, cum_a * 2);
    }

    #[test]
    fn test_twap_window_shorter_than_age() {
        // Pool exists for 100 slots
        let cum_50 = update_twap(0, 1000, 500, 50, 0);
        let cum_100 = update_twap(cum_50, 1000, 500, 100, 50);
        // Query TWAP for last 50 slots only
        let twap_50 = compute_twap_price(cum_50, cum_100, 50).unwrap();
        // Should equal the price during those 50 slots
        let expected = (1000u128 << 64) / 500;
        assert_eq!(twap_50, expected);
    }

    #[test]
    fn test_twap_price_equals_spot_when_constant() {
        // Constant price for 360 slots
        let cum = update_twap(0, 2000, 1000, 360, 0);
        let twap = compute_twap_price(0, cum, 360).unwrap();
        let spot = (2000u128 << 64) / 1000;
        assert_eq!(twap, spot);
    }

    // ========== Group 4 — Batching (3 tests) ==========

    #[test]
    fn test_batch_order_deterministic() {
        use crypto::Hash;
        let mut hashes = [
            Hash::from_bytes([0x03; 32]),
            Hash::from_bytes([0x01; 32]),
            Hash::from_bytes([0x02; 32]),
        ];
        hashes.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        assert_eq!(hashes[0], Hash::from_bytes([0x01; 32]));
        assert_eq!(hashes[1], Hash::from_bytes([0x02; 32]));
        assert_eq!(hashes[2], Hash::from_bytes([0x03; 32]));
    }

    #[test]
    fn test_batch_two_swaps_sequential() {
        // First swap
        let (dy1, ra1, rb1) = compute_swap(1000, 1000, 50, 30).unwrap();
        // Second swap against updated reserves
        let (dy2, ra2, rb2) = compute_swap(ra1, rb1, 50, 30).unwrap();
        // Second swap gets less output (price moved)
        assert!(dy2 < dy1);
        // Both maintain invariant
        assert!(verify_invariant(1000, 1000, ra1, rb1));
        assert!(verify_invariant(ra1, rb1, ra2, rb2));
    }

    #[test]
    fn test_batch_slippage_second_swap_fails() {
        // First swap moves the price
        let (_, ra1, rb1) = compute_swap(1000, 1000, 200, 30).unwrap();
        // Second swap expects original price output
        let (dy2, _, _) = compute_swap(ra1, rb1, 100, 30).unwrap();
        // Output at original price would be ~90
        let expected_at_original = compute_swap(1000, 1000, 100, 30).unwrap().0;
        // Second swap gets less due to price movement
        assert!(dy2 < expected_at_original);
    }

    // ========== Group 5 — Security (2 tests) ==========

    #[test]
    fn test_pool_invariant_tampered_rejected() {
        // Tampered reserves that violate x*y=k
        assert!(!verify_invariant(1000, 1000, 1100, 800));
        // 1100 * 800 = 880000 < 1000000 = 1000 * 1000 -> INVALID
    }

    #[test]
    fn test_twap_manipulation_single_block() {
        // 359 blocks at price ratio 1:1 (reserve_a=1000, reserve_b=1000)
        let normal_price = (1000u128 << 64) / 1000;
        let cum_359 = normal_price * 359;

        // 1 block with 10x manipulation (reserve_a=10000, reserve_b=1000)
        let manip_price = (10000u128 << 64) / 1000;
        let cum_360 = cum_359 + manip_price;

        // TWAP over full 360 blocks
        let twap = cum_360 / 360;

        // Deviation from normal price
        let diff = twap.abs_diff(normal_price);
        // Deviation in basis points
        let pct_x100 = (diff * 10000) / normal_price;
        // 1/360 slots with 10x spike -> ~2.5% deviation. Must be < 3% (300 bps).
        // Over a full epoch, a single-block manipulation has limited impact.
        assert!(
            pct_x100 <= 300,
            "TWAP changed {:.2}% from single-block manipulation (must be < 3%)",
            pct_x100 as f64 / 100.0
        );
        // Also verify the deviation is non-trivial (manipulation did have *some* effect)
        assert!(
            pct_x100 > 0,
            "TWAP should show some deviation from manipulation"
        );
    }

    // ========== Group 6 — Output constructors (2 tests) ==========

    #[test]
    fn test_pool_output_roundtrip() {
        use crypto::Hash;
        let asset_b = Hash::from_bytes([0xBB; 32]);
        let pool_id = crate::transaction::Output::compute_pool_id(&Hash::ZERO, &asset_b);

        let output =
            crate::transaction::Output::pool(pool_id, asset_b, 1000, 2000, 707, 0, 100, 30, 100);

        assert_eq!(output.output_type, crate::OutputType::Pool);
        assert_eq!(output.amount, 0);
        assert_eq!(
            output.extra_data.len(),
            crate::transaction::POOL_METADATA_SIZE
        );

        let meta = output.pool_metadata().unwrap();
        assert_eq!(meta.pool_id, pool_id);
        assert_eq!(meta.asset_b_id, asset_b);
        assert_eq!(meta.reserve_a, 1000);
        assert_eq!(meta.reserve_b, 2000);
        assert_eq!(meta.total_lp_shares, 707);
        assert_eq!(meta.cumulative_price, 0);
        assert_eq!(meta.last_update_slot, 100);
        assert_eq!(meta.fee_bps, 30);
        assert_eq!(meta.creation_slot, 100);
        assert_eq!(meta.status, 0);
    }

    #[test]
    fn test_lp_share_output_roundtrip() {
        use crypto::Hash;
        let pool_id = Hash::from_bytes([0xAA; 32]);
        let owner = Hash::from_bytes([0xBB; 32]);

        let output = crate::transaction::Output::lp_share(500, pool_id, owner);

        assert_eq!(output.output_type, crate::OutputType::LPShare);
        assert_eq!(output.amount, 500);
        assert_eq!(output.pubkey_hash, owner);

        let recovered_pool_id = output.lp_share_metadata().unwrap();
        assert_eq!(recovered_pool_id, pool_id);
    }
}

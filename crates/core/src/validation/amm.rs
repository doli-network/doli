//! AMM value-conservation validation (INC-I-096).
//!
//! Single shared function implementing per-asset balance equations
//! (E1 DOLI, E2 token_b, E3 LP supply), proportional reserve binding
//! for LP operations, k-invariant for Swap, and FM-S11 asset_id
//! cross-check. Called by both consensus and mempool (M2/M3).
//!
//! This module does NOT touch `is_native_amount()` or system-wide
//! conservation. It operates on resolved `&[Output]` slices so both
//! mempool (`UtxoSet`) and consensus (`UtxoProvider`) callers can
//! construct the data without trait dependencies.

use crypto::Hash;

use crate::pool;
use crate::transaction::{Output, OutputType, PoolMetadata, TxType};
use crate::types::Amount;

use super::ValidationError;

/// Result of a successful AMM conservation check.
/// `doli_surplus` is the DOLI left over after conservation (usable as fee by mempool).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmmConservationResult {
    pub doli_surplus: u64,
}

/// Pool-aware, input-bound value conservation for AMM tx types.
///
/// `consumed_outputs`: resolved input UTXOs in tx.inputs order (input[0] = old Pool).
/// `tx_outputs`: tx.outputs (output[0] = new Pool).
///
/// Handles: Swap, AddLiquidity, RemoveLiquidity.
/// CreatePool is validated separately (RC-B in utxo.rs).
pub fn verify_amm_conservation(
    tx_type: TxType,
    consumed_outputs: &[Output],
    tx_outputs: &[Output],
) -> Result<AmmConservationResult, ValidationError> {
    // --- Extract old and new pool metadata (FILTER-8: no silent fallback) ---
    let old_pool = consumed_outputs
        .first()
        .and_then(|o| o.pool_metadata())
        .ok_or_else(|| {
            ValidationError::InvalidPool(
                "consumed_outputs[0] must be a Pool UTXO with valid metadata".into(),
            )
        })?;

    let new_pool = tx_outputs
        .first()
        .and_then(|o| o.pool_metadata())
        .ok_or_else(|| {
            ValidationError::InvalidPool(
                "tx_outputs[0] must be a Pool UTXO with valid metadata".into(),
            )
        })?;

    let asset_b_id = old_pool.asset_b_id;
    let pool_id = old_pool.pool_id;

    // --- FM-S11: every FungibleAsset output must carry pool's asset_b_id ---
    for (i, out) in tx_outputs.iter().enumerate() {
        if out.output_type == OutputType::FungibleAsset {
            if let Some((fa_asset_id, _, _)) = out.fungible_asset_metadata() {
                if fa_asset_id != asset_b_id {
                    return Err(ValidationError::InvalidPool(format!(
                        "FM-S11: FungibleAsset output[{}] has asset_id {:?} but pool \
                         asset_b_id is {:?}; cross-pool token counterfeiting rejected",
                        i, fa_asset_id, asset_b_id
                    )));
                }
            }
        }
    }

    // --- Per-asset value extractors (u128 to avoid overflow) ---
    let doli_value = |o: &Output| -> u128 {
        if o.output_type.is_native_amount() {
            o.amount as u128
        } else if o.output_type == OutputType::Pool {
            if let Some(pm) = o.pool_metadata() {
                pm.reserve_a as u128
            } else {
                0
            }
        } else {
            0
        }
    };

    let token_b = |o: &Output| -> u128 {
        if o.output_type == OutputType::FungibleAsset {
            if let Some((fa_id, _, _)) = o.fungible_asset_metadata() {
                if fa_id == asset_b_id {
                    return o.amount as u128;
                }
            }
            0
        } else if o.output_type == OutputType::Pool {
            if let Some(pm) = o.pool_metadata() {
                pm.reserve_b as u128
            } else {
                0
            }
        } else {
            0
        }
    };

    // LP extractor: DO NOT fold Pool.total_lp into this. LP supply is handled
    // by E3 separately using old_pool.total_lp_shares and new_pool.total_lp_shares.
    let lp = |o: &Output| -> u128 {
        if o.output_type == OutputType::LPShare {
            if let Some(lp_pool_id) = o.lp_share_metadata() {
                if lp_pool_id == pool_id {
                    return o.amount as u128;
                }
            }
            0
        } else {
            0
        }
    };

    // --- Compute sums ---
    let sum_doli_in: u128 = consumed_outputs.iter().map(doli_value).sum();
    let sum_doli_out: u128 = tx_outputs.iter().map(doli_value).sum();
    let sum_token_in: u128 = consumed_outputs.iter().map(token_b).sum();
    let sum_token_out: u128 = tx_outputs.iter().map(token_b).sum();
    let sum_lp_in: u128 = consumed_outputs.iter().map(&lp).sum();
    let sum_lp_out: u128 = tx_outputs.iter().map(&lp).sum();

    // --- E1: DOLI conservation (>= absorbs fee + floor dust) ---
    if sum_doli_in < sum_doli_out {
        return Err(ValidationError::InsufficientFunds {
            inputs: sum_doli_in as Amount,
            outputs: sum_doli_out as Amount,
        });
    }
    let doli_surplus = (sum_doli_in - sum_doli_out) as u64;

    // --- E2: token_b conservation ---
    if sum_token_in < sum_token_out {
        return Err(ValidationError::InvalidLiquidity(format!(
            "token_b conservation violated: sum_in={} < sum_out={} \
             (token created from nothing)",
            sum_token_in, sum_token_out
        )));
    }

    // --- E3: LP-supply EXACT bind ---
    // new_total_lp + sum_lp(consumed) == old_total_lp + sum_lp(tx_outputs)
    // This binds new_total_lp to ACTUAL consumed LPShare inputs (P5/T10 fix).
    let lhs = new_pool.total_lp_shares as u128 + sum_lp_in;
    let rhs = old_pool.total_lp_shares as u128 + sum_lp_out;
    if lhs != rhs {
        return Err(ValidationError::InvalidLiquidity(format!(
            "LP supply mismatch: new_total_lp({}) + consumed_lp({}) = {} != \
             old_total_lp({}) + minted_lp({}) = {}",
            new_pool.total_lp_shares, sum_lp_in, lhs, old_pool.total_lp_shares, sum_lp_out, rhs
        )));
    }

    // --- Type-specific checks ---
    match tx_type {
        TxType::Swap => {
            check_swap(&old_pool, &new_pool)?;
        }
        TxType::RemoveLiquidity => {
            check_remove_liquidity(&old_pool, &new_pool, &asset_b_id, tx_outputs)?;
        }
        TxType::AddLiquidity => {
            check_add_liquidity(&old_pool, &new_pool)?;
        }
        _ => {
            return Err(ValidationError::InvalidPool(format!(
                "verify_amm_conservation called with unsupported tx_type {:?}",
                tx_type
            )));
        }
    }

    Ok(AmmConservationResult { doli_surplus })
}

/// Swap: k-invariant (independent guard).
fn check_swap(old_pool: &PoolMetadata, new_pool: &PoolMetadata) -> Result<(), ValidationError> {
    if !pool::verify_invariant(
        old_pool.reserve_a,
        old_pool.reserve_b,
        new_pool.reserve_a,
        new_pool.reserve_b,
    ) {
        return Err(ValidationError::InvalidSwap(format!(
            "k-invariant violated: old_k={} > new_k={}",
            (old_pool.reserve_a as u128) * (old_pool.reserve_b as u128),
            (new_pool.reserve_a as u128) * (new_pool.reserve_b as u128),
        )));
    }
    Ok(())
}

/// RemoveLiquidity: proportional reserve binding (OPTION A).
fn check_remove_liquidity(
    old_pool: &PoolMetadata,
    new_pool: &PoolMetadata,
    asset_b_id: &Hash,
    tx_outputs: &[Output],
) -> Result<(), ValidationError> {
    // shares_burned = old_total_lp - new_total_lp (safe: E3 ensures consistency)
    let old_total = old_pool.total_lp_shares;
    let new_total = new_pool.total_lp_shares;

    if old_total < new_total {
        return Err(ValidationError::InvalidLiquidity(
            "RemoveLiquidity: new_total_lp > old_total_lp".into(),
        ));
    }
    let shares_burned = old_total - new_total;

    if shares_burned == 0 {
        return Err(ValidationError::InvalidLiquidity(
            "RemoveLiquidity: zero shares burned".into(),
        ));
    }

    // Proportional max via builder math
    let (da_max, db_max) = pool::compute_remove_liquidity(
        shares_burned,
        old_pool.reserve_a,
        old_pool.reserve_b,
        old_total,
    )
    .ok_or_else(|| {
        ValidationError::InvalidLiquidity(
            "RemoveLiquidity: compute_remove_liquidity returned None \
             (degenerate pool state)"
                .into(),
        )
    })?;

    // Reserve delta checks (old >= new guarded by E1/E2 already, but be safe)
    let da_actual = old_pool.reserve_a.saturating_sub(new_pool.reserve_a);
    let db_actual = old_pool.reserve_b.saturating_sub(new_pool.reserve_b);

    if da_actual > da_max {
        return Err(ValidationError::InvalidLiquidity(format!(
            "RemoveLiquidity proportional binding violated: reserve_a \
             delta {} > proportional max {} for {} shares burned out of \
             {} total",
            da_actual, da_max, shares_burned, old_total
        )));
    }
    if db_actual > db_max {
        return Err(ValidationError::InvalidLiquidity(format!(
            "RemoveLiquidity proportional binding violated: reserve_b \
             delta {} > proportional max {} for {} shares burned out of \
             {} total",
            db_actual, db_max, shares_burned, old_total
        )));
    }

    // User token output binding: sum of FungibleAsset outputs (skip output[0])
    // with asset_id == asset_b_id must be <= db_actual
    let user_token_out: u128 = tx_outputs
        .iter()
        .skip(1) // skip output[0] (new Pool)
        .filter(|o| o.output_type == OutputType::FungibleAsset)
        .filter_map(|o| {
            o.fungible_asset_metadata()
                .filter(|(id, _, _)| id == asset_b_id)
                .map(|_| o.amount as u128)
        })
        .sum();

    if user_token_out > db_actual as u128 {
        return Err(ValidationError::InvalidLiquidity(format!(
            "RemoveLiquidity token output binding violated: user_token_out \
             {} > reserve_b delta {} (T8 token-inflation guard)",
            user_token_out, db_actual
        )));
    }

    Ok(())
}

/// AddLiquidity: proportional LP minting binding (OPTION A / FILTER-7).
fn check_add_liquidity(
    old_pool: &PoolMetadata,
    new_pool: &PoolMetadata,
) -> Result<(), ValidationError> {
    let old_total = old_pool.total_lp_shares;
    let new_total = new_pool.total_lp_shares;

    if new_total < old_total {
        return Err(ValidationError::InvalidLiquidity(
            "AddLiquidity: new_total_lp < old_total_lp".into(),
        ));
    }
    let minted = new_total - old_total;

    if new_pool.reserve_a < old_pool.reserve_a {
        return Err(ValidationError::InvalidLiquidity(
            "AddLiquidity: new_reserve_a < old_reserve_a".into(),
        ));
    }
    if new_pool.reserve_b < old_pool.reserve_b {
        return Err(ValidationError::InvalidLiquidity(
            "AddLiquidity: new_reserve_b < old_reserve_b".into(),
        ));
    }

    let da_added = new_pool.reserve_a - old_pool.reserve_a;
    let db_added = new_pool.reserve_b - old_pool.reserve_b;

    // Can't mint more LP than proportional to reserves added
    let max_lp = pool::compute_lp_shares(
        da_added,
        db_added,
        old_pool.reserve_a,
        old_pool.reserve_b,
        old_total,
    )
    .ok_or_else(|| {
        ValidationError::InvalidLiquidity(
            "AddLiquidity: compute_lp_shares returned None \
             (degenerate pool state)"
                .into(),
        )
    })?;

    if minted > max_lp {
        return Err(ValidationError::InvalidLiquidity(format!(
            "AddLiquidity: minted {} LP shares exceeds proportional max \
             {} (da={}, db={}, old_ra={}, old_rb={}, old_total={})",
            minted, max_lp, da_added, db_added, old_pool.reserve_a, old_pool.reserve_b, old_total
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conditions::Condition;
    use crate::transaction::Output;
    use crypto::Hash;

    // Helper constants
    fn asset_b() -> Hash {
        Hash::from_bytes([0xBB; 32])
    }
    fn pool_id() -> Hash {
        Output::compute_pool_id(&Hash::ZERO, &asset_b(), 30)
    }
    fn user_pkh() -> Hash {
        Hash::from_bytes([0x55; 32])
    }

    fn make_token_out(amount: Amount, asset_id: Hash) -> Output {
        Output::fungible_asset(
            amount,
            user_pkh(),
            asset_id,
            1_000_000,
            "TKN",
            &Condition::Signature(user_pkh()),
        )
        .expect("fungible asset output must encode")
    }

    // ===== Valid RemoveLiquidity =====
    // Pool: 1000/2000/1000, burn 500 shares -> da=500, db=1000
    // Outputs: [new_pool(500,1000,500), doli_out(500), tokens(1000), fee_change(998)]
    // Consumed: [old_pool, lp_share(500), doli_funding(1000)]
    // doli_surplus = (1000+1000) - (500+500+998) = 2
    #[test]
    fn valid_remove_liquidity() {
        let consumed = vec![
            Output::pool(pool_id(), asset_b(), 1000, 2000, 1000, 0, 100, 30, 100),
            Output::lp_share(500, pool_id(), user_pkh()),
            Output::normal(1000, user_pkh()),
        ];
        let outputs = vec![
            Output::pool(pool_id(), asset_b(), 500, 1000, 500, 0, 101, 30, 100),
            Output::normal(500, user_pkh()),
            make_token_out(1000, asset_b()),
            Output::normal(998, user_pkh()),
        ];
        let res = verify_amm_conservation(TxType::RemoveLiquidity, &consumed, &outputs);
        assert!(res.is_ok(), "valid RemoveLiquidity rejected: {:?}", res);
        assert_eq!(res.unwrap().doli_surplus, 2);
    }

    // ===== 1-share drain (T3) =====
    #[test]
    fn one_share_drain_rejected() {
        let consumed = vec![
            Output::pool(pool_id(), asset_b(), 1000, 2000, 1000, 0, 100, 30, 100),
            Output::lp_share(1, pool_id(), user_pkh()),
            Output::normal(2, user_pkh()),
        ];
        let outputs = vec![
            Output::pool(pool_id(), asset_b(), 500, 1998, 999, 0, 101, 30, 100),
            Output::normal(500, user_pkh()),
            make_token_out(2, asset_b()),
        ];
        let res = verify_amm_conservation(TxType::RemoveLiquidity, &consumed, &outputs);
        assert!(res.is_err(), "1-share drain must be rejected");
        let msg = format!("{:?}", res.unwrap_err());
        assert!(
            msg.contains("proportional"),
            "error must mention 'proportional', got: {}",
            msg
        );
    }

    // ===== Token drain (T4) =====
    #[test]
    fn token_drain_rejected() {
        let consumed = vec![
            Output::pool(pool_id(), asset_b(), 1000, 2000, 1000, 0, 100, 30, 100),
            Output::lp_share(500, pool_id(), user_pkh()),
            Output::normal(2, user_pkh()),
        ];
        let outputs = vec![
            Output::pool(pool_id(), asset_b(), 500, 500, 500, 0, 101, 30, 100),
            Output::normal(500, user_pkh()),
            make_token_out(1500, asset_b()),
        ];
        let res = verify_amm_conservation(TxType::RemoveLiquidity, &consumed, &outputs);
        assert!(res.is_err(), "token drain must be rejected");
    }

    // ===== T10 underburn: LPShare input=1, new_total_lp=0 =====
    #[test]
    fn t10_underburn_drain_rejected() {
        let consumed = vec![
            Output::pool(pool_id(), asset_b(), 1000, 2000, 1000, 0, 100, 30, 100),
            Output::lp_share(1, pool_id(), user_pkh()),
            Output::normal(1002, user_pkh()),
        ];
        let outputs = vec![
            Output::pool(pool_id(), asset_b(), 0, 0, 0, 0, 101, 30, 100),
            Output::normal(1000, user_pkh()),
            make_token_out(2000, asset_b()),
        ];
        let res = verify_amm_conservation(TxType::RemoveLiquidity, &consumed, &outputs);
        assert!(res.is_err(), "T10 underburn drain must be rejected");
        let msg = format!("{:?}", res.unwrap_err());
        assert!(
            msg.contains("LP supply mismatch"),
            "expected LP supply mismatch, got: {}",
            msg
        );
    }

    // ===== Rounding remainder =====
    #[test]
    fn rounding_remainder_accepted() {
        let consumed = vec![
            Output::pool(pool_id(), asset_b(), 1000, 2001, 1000, 0, 100, 30, 100),
            Output::lp_share(333, pool_id(), user_pkh()),
            Output::normal(2, user_pkh()),
        ];
        let outputs = vec![
            Output::pool(pool_id(), asset_b(), 667, 1335, 667, 0, 101, 30, 100),
            Output::normal(333, user_pkh()),
            make_token_out(666, asset_b()),
        ];
        let res = verify_amm_conservation(TxType::RemoveLiquidity, &consumed, &outputs);
        assert!(
            res.is_ok(),
            "rounding remainder remove must be accepted: {:?}",
            res
        );
    }

    // ===== Valid Swap A->B =====
    #[test]
    fn valid_swap_a_to_b() {
        let consumed = vec![
            Output::pool(pool_id(), asset_b(), 1000, 1000, 707, 0, 100, 30, 100),
            Output::normal(100, user_pkh()),
        ];
        let outputs = vec![
            Output::pool(pool_id(), asset_b(), 1100, 910, 707, 0, 101, 30, 100),
            make_token_out(90, asset_b()),
        ];
        let res = verify_amm_conservation(TxType::Swap, &consumed, &outputs);
        assert!(res.is_ok(), "valid A->B swap rejected: {:?}", res);
    }

    // ===== Valid Swap B->A with DOLI fee-change =====
    #[test]
    fn valid_swap_b_to_a_with_fee_change() {
        let consumed = vec![
            Output::pool(pool_id(), asset_b(), 1000, 1000, 707, 0, 100, 30, 100),
            make_token_out(100, asset_b()),
            Output::normal(100, user_pkh()),
        ];
        let outputs = vec![
            Output::pool(pool_id(), asset_b(), 910, 1100, 707, 0, 101, 30, 100),
            Output::normal(90, user_pkh()),
            Output::normal(98, user_pkh()),
        ];
        let res = verify_amm_conservation(TxType::Swap, &consumed, &outputs);
        assert!(
            res.is_ok(),
            "valid B->A swap with fee-change rejected: {:?}",
            res
        );
        assert_eq!(res.unwrap().doli_surplus, 2);
    }

    // ===== Swap B->A over-extraction (new_k < old_k) =====
    #[test]
    fn swap_b_to_a_over_extraction_rejected() {
        let consumed = vec![
            Output::pool(pool_id(), asset_b(), 1000, 1000, 707, 0, 100, 30, 100),
            make_token_out(100, asset_b()),
            Output::normal(200, user_pkh()),
        ];
        let outputs = vec![
            Output::pool(pool_id(), asset_b(), 800, 1100, 707, 0, 101, 30, 100),
            Output::normal(200, user_pkh()),
        ];
        let res = verify_amm_conservation(TxType::Swap, &consumed, &outputs);
        assert!(res.is_err(), "swap over-extraction must be rejected");
        let msg = format!("{:?}", res.unwrap_err());
        assert!(
            msg.contains("k-invariant") || msg.contains("InvalidSwap"),
            "expected k-invariant rejection, got: {}",
            msg
        );
    }

    // ===== Valid AddLiquidity =====
    #[test]
    fn valid_add_liquidity() {
        let consumed = vec![
            Output::pool(pool_id(), asset_b(), 1000, 1000, 707, 0, 100, 30, 100),
            Output::normal(500, user_pkh()),
            make_token_out(500, asset_b()),
        ];
        let outputs = vec![
            Output::pool(pool_id(), asset_b(), 1500, 1500, 1060, 0, 101, 30, 100),
            Output::lp_share(353, pool_id(), user_pkh()),
        ];
        let res = verify_amm_conservation(TxType::AddLiquidity, &consumed, &outputs);
        assert!(res.is_ok(), "valid AddLiquidity rejected: {:?}", res);
    }

    // ===== AddLiquidity excess minting =====
    #[test]
    fn add_liquidity_excess_minting_rejected() {
        let consumed = vec![
            Output::pool(pool_id(), asset_b(), 1000, 1000, 707, 0, 100, 30, 100),
            Output::normal(500, user_pkh()),
            make_token_out(500, asset_b()),
        ];
        // max = min(500*707/1000, 500*707/1000) = 353. Minting 500.
        let outputs = vec![
            Output::pool(pool_id(), asset_b(), 1500, 1500, 1207, 0, 101, 30, 100),
            Output::lp_share(500, pool_id(), user_pkh()),
        ];
        let res = verify_amm_conservation(TxType::AddLiquidity, &consumed, &outputs);
        assert!(res.is_err(), "AddLiquidity excess minting must be rejected");
    }

    // ===== FM-S11: foreign asset_id in RemoveLiquidity =====
    #[test]
    fn fms11_foreign_asset_id_rejected_in_remove() {
        let foreign_asset = Hash::from_bytes([0xCC; 32]);
        let consumed = vec![
            Output::pool(pool_id(), asset_b(), 1000, 2000, 1000, 0, 100, 30, 100),
            Output::lp_share(500, pool_id(), user_pkh()),
            Output::normal(1000, user_pkh()),
        ];
        let outputs = vec![
            Output::pool(pool_id(), asset_b(), 500, 1000, 500, 0, 101, 30, 100),
            Output::normal(500, user_pkh()),
            make_token_out(1000, foreign_asset),
            Output::normal(998, user_pkh()),
        ];
        let res = verify_amm_conservation(TxType::RemoveLiquidity, &consumed, &outputs);
        assert!(res.is_err(), "FM-S11: foreign asset_id must be rejected");
        let msg = format!("{:?}", res.unwrap_err());
        assert!(
            msg.contains("FM-S11"),
            "error must mention FM-S11, got: {}",
            msg
        );
    }

    // ===== FM-S11: foreign asset_id in Swap =====
    #[test]
    fn fms11_foreign_asset_id_rejected_in_swap() {
        let foreign_asset = Hash::from_bytes([0xCC; 32]);
        let consumed = vec![
            Output::pool(pool_id(), asset_b(), 1000, 1000, 707, 0, 100, 30, 100),
            Output::normal(100, user_pkh()),
        ];
        let outputs = vec![
            Output::pool(pool_id(), asset_b(), 1100, 910, 707, 0, 101, 30, 100),
            make_token_out(90, foreign_asset),
        ];
        let res = verify_amm_conservation(TxType::Swap, &consumed, &outputs);
        assert!(res.is_err(), "FM-S11: foreign asset_id must be rejected");
    }

    // ===== None pool metadata input =====
    #[test]
    fn none_pool_metadata_input_rejected() {
        let consumed = vec![Output::normal(1000, user_pkh())];
        let outputs = vec![Output::pool(
            pool_id(),
            asset_b(),
            500,
            1000,
            500,
            0,
            101,
            30,
            100,
        )];
        let res = verify_amm_conservation(TxType::Swap, &consumed, &outputs);
        assert!(res.is_err(), "non-Pool input[0] must be rejected");
    }

    // ===== None pool metadata output =====
    #[test]
    fn none_pool_metadata_output_rejected() {
        let consumed = vec![
            Output::pool(pool_id(), asset_b(), 1000, 1000, 707, 0, 100, 30, 100),
            Output::normal(100, user_pkh()),
        ];
        let outputs = vec![Output::normal(1090, user_pkh())];
        let res = verify_amm_conservation(TxType::Swap, &consumed, &outputs);
        assert!(res.is_err(), "non-Pool output[0] must be rejected");
    }

    // ===== E1: DOLI conservation failure =====
    #[test]
    fn doli_conservation_violation_rejected() {
        let consumed = vec![
            Output::pool(pool_id(), asset_b(), 1000, 1000, 707, 0, 100, 30, 100),
            Output::normal(50, user_pkh()),
        ];
        let outputs = vec![
            Output::pool(pool_id(), asset_b(), 1100, 900, 707, 0, 101, 30, 100),
            Output::normal(50, user_pkh()),
        ];
        let res = verify_amm_conservation(TxType::Swap, &consumed, &outputs);
        assert!(res.is_err(), "DOLI over-creation must be rejected");
    }

    // ===== E2: token_b conservation failure =====
    #[test]
    fn token_b_conservation_violation_rejected() {
        let consumed = vec![
            Output::pool(pool_id(), asset_b(), 1000, 1000, 707, 0, 100, 30, 100),
            Output::normal(100, user_pkh()),
        ];
        let outputs = vec![
            Output::pool(pool_id(), asset_b(), 1100, 900, 707, 0, 101, 30, 100),
            make_token_out(200, asset_b()),
        ];
        let res = verify_amm_conservation(TxType::Swap, &consumed, &outputs);
        assert!(res.is_err(), "token_b over-creation must be rejected");
    }

    // ===== E3: Swap LP supply change =====
    #[test]
    fn swap_lp_supply_change_rejected() {
        let consumed = vec![
            Output::pool(pool_id(), asset_b(), 1000, 1000, 707, 0, 100, 30, 100),
            Output::normal(100, user_pkh()),
        ];
        let outputs = vec![
            Output::pool(pool_id(), asset_b(), 1100, 910, 800, 0, 101, 30, 100),
            make_token_out(90, asset_b()),
        ];
        let res = verify_amm_conservation(TxType::Swap, &consumed, &outputs);
        assert!(res.is_err(), "Swap changing LP supply must be rejected");
    }

    // ===== RemoveLiquidity: user token output exceeds reserve_b delta =====
    #[test]
    fn remove_liquidity_token_output_exceeds_delta() {
        let consumed = vec![
            Output::pool(pool_id(), asset_b(), 1000, 2000, 1000, 0, 100, 30, 100),
            Output::lp_share(500, pool_id(), user_pkh()),
            Output::normal(1000, user_pkh()),
            make_token_out(1, asset_b()),
        ];
        let outputs = vec![
            Output::pool(pool_id(), asset_b(), 500, 1000, 500, 0, 101, 30, 100),
            Output::normal(500, user_pkh()),
            make_token_out(1001, asset_b()),
            Output::normal(998, user_pkh()),
        ];
        let res = verify_amm_conservation(TxType::RemoveLiquidity, &consumed, &outputs);
        assert!(
            res.is_err(),
            "RemoveLiquidity token output exceeding delta must be rejected"
        );
    }
}

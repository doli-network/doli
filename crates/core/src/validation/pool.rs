//! Validation rules for AMM pool transactions.
//!
//! Structural validation only — no UTXO state needed.
//! Invariant (x*y=k) and reserve checks happen in validation/utxo.rs (UTXO-context validation).

use crate::consensus::MINIMUM_LIQUIDITY;
use crate::transaction::{OutputType, Transaction, POOL_MAX_FEE_BPS};
use crate::validation::error::ValidationError;

/// Validate a CreatePool transaction structure.
pub(crate) fn validate_create_pool(tx: &Transaction) -> Result<(), ValidationError> {
    // Must have inputs (funding the pool)
    if tx.inputs.is_empty() {
        return Err(ValidationError::InvalidPool(
            "CreatePool requires at least one input".to_string(),
        ));
    }

    // Must have exactly 2 outputs: Pool UTXO + LPShare for creator
    // Optionally a 3rd output for change
    if tx.outputs.len() < 2 {
        return Err(ValidationError::InvalidPool(
            "CreatePool requires at least 2 outputs (Pool + LPShare)".to_string(),
        ));
    }

    // First output must be Pool type
    if tx.outputs[0].output_type != OutputType::Pool {
        return Err(ValidationError::InvalidPool(
            "first output must be Pool type".to_string(),
        ));
    }

    // Validate pool metadata is decodable
    let pool_meta = tx.outputs[0].pool_metadata().ok_or_else(|| {
        ValidationError::InvalidPool("invalid pool metadata in output 0".to_string())
    })?;

    // Reserves must be > 0
    if pool_meta.reserve_a == 0 || pool_meta.reserve_b == 0 {
        return Err(ValidationError::InvalidPool(
            "initial reserves must be greater than zero".to_string(),
        ));
    }

    // Fee must be reasonable
    if pool_meta.fee_bps > POOL_MAX_FEE_BPS {
        return Err(ValidationError::InvalidPool(format!(
            "fee {} bps exceeds maximum {} bps",
            pool_meta.fee_bps, POOL_MAX_FEE_BPS
        )));
    }

    // Asset order: asset_a (DOLI = Hash::ZERO) must be < asset_b
    if pool_meta.asset_b_id <= crypto::Hash::ZERO {
        return Err(ValidationError::InvalidPool(
            "asset_b must be a non-zero FungibleAsset ID (DOLI is always asset_a)".to_string(),
        ));
    }

    // D2 (AMM Foundations M2, 2026-05-25): pool_id MUST equal the
    // canonical derivation BLAKE3(POOL_ID_DOMAIN || fee_bps_le || sort(a,b)).
    // A forged pool_id would let an attacker mint a Pool UTXO at an
    // arbitrary address, breaking the per-(pair, fee_bps) singleton
    // invariant (INV-DEFI-010 generalised by M2) once the AMM apply_block
    // consumer enforces uniqueness by pool_id at state level. We anchor
    // the invariant at validation so the same rule fires for every node.
    let expected_pool_id = crate::transaction::Output::compute_pool_id(
        &crypto::Hash::ZERO,
        &pool_meta.asset_b_id,
        pool_meta.fee_bps,
    );
    if pool_meta.pool_id != expected_pool_id {
        return Err(ValidationError::InvalidPool(format!(
            "pool_id mismatch: declared={}, expected={} (must be BLAKE3 of \
             POOL_ID_DOMAIN || fee_bps_le || sort(asset_a, asset_b))",
            pool_meta.pool_id, expected_pool_id
        )));
    }

    // Second output must be LPShare
    if tx.outputs[1].output_type != OutputType::LPShare {
        return Err(ValidationError::InvalidPool(
            "second output must be LPShare type".to_string(),
        ));
    }

    // LPShare must reference the same pool
    let lp_pool_id = tx.outputs[1].lp_share_metadata().ok_or_else(|| {
        ValidationError::InvalidPool("invalid LPShare metadata in output 1".to_string())
    })?;
    if lp_pool_id != pool_meta.pool_id {
        return Err(ValidationError::InvalidPool(
            "LPShare pool_id must match Pool output".to_string(),
        ));
    }

    // D1 (AMM Foundations M3, 2026-05-25): MINIMUM_LIQUIDITY first-deposit
    // lock invariant. Mirrors Uniswap v2: the gap between Pool.total_lp_shares
    // and the creator's LPShare.amount must equal exactly MINIMUM_LIQUIDITY,
    // and those locked shares are NEVER materialised as a spendable UTXO. The
    // threshold (1000) makes the first-deposit inflation attack 1:1 cost/payoff
    // (Adversarial Capital A4/A7).
    //
    // Failure modes (both → ValidationError::AmmMinimumLiquidity):
    //   (a) declared_total < MINIMUM_LIQUIDITY (under-minted; nothing to lock)
    //   (b) creator_share + MINIMUM_LIQUIDITY != declared_total (creator
    //       claims more than they paid for, OR claims less than allowed and
    //       leaves part of the lock unaccounted; both shapes break the
    //       proportional-minting invariant downstream).
    let creator_share = tx.outputs[1].amount;
    let declared_total = pool_meta.total_lp_shares;
    if declared_total < MINIMUM_LIQUIDITY
        || creator_share.checked_add(MINIMUM_LIQUIDITY) != Some(declared_total)
    {
        return Err(ValidationError::AmmMinimumLiquidity {
            declared_total,
            creator_share,
            minimum_liquidity: MINIMUM_LIQUIDITY,
        });
    }

    // Initial TWAP state
    if pool_meta.cumulative_price != 0 {
        return Err(ValidationError::InvalidPool(
            "initial cumulative_price must be 0".to_string(),
        ));
    }

    // Remaining outputs (if any) must be Normal (DOLI change) or FungibleAsset (token change)
    for (i, output) in tx.outputs.iter().enumerate().skip(2) {
        if output.output_type != OutputType::Normal
            && output.output_type != OutputType::FungibleAsset
        {
            return Err(ValidationError::InvalidPool(format!(
                "output {} must be Normal or FungibleAsset type (change), got {:?}",
                i, output.output_type
            )));
        }
    }

    Ok(())
}

/// Validate a Swap transaction structure.
pub(crate) fn validate_swap(tx: &Transaction) -> Result<(), ValidationError> {
    // Must have inputs (at least: pool UTXO + swapper's funding UTXO)
    if tx.inputs.len() < 2 {
        return Err(ValidationError::InvalidSwap(
            "Swap requires at least 2 inputs (pool + funding)".to_string(),
        ));
    }

    // Must have at least 2 outputs: new Pool UTXO + swapper's output
    if tx.outputs.len() < 2 {
        return Err(ValidationError::InvalidSwap(
            "Swap requires at least 2 outputs (pool + swap result)".to_string(),
        ));
    }

    // First output must be Pool type (updated reserves)
    if tx.outputs[0].output_type != OutputType::Pool {
        return Err(ValidationError::InvalidSwap(
            "first output must be Pool type".to_string(),
        ));
    }

    // Validate pool metadata
    tx.outputs[0].pool_metadata().ok_or_else(|| {
        ValidationError::InvalidSwap("invalid pool metadata in output 0".to_string())
    })?;

    Ok(())
}

/// Validate an AddLiquidity transaction structure.
pub(crate) fn validate_add_liquidity(tx: &Transaction) -> Result<(), ValidationError> {
    // Inputs: pool UTXO + provider's assets
    if tx.inputs.len() < 2 {
        return Err(ValidationError::InvalidLiquidity(
            "AddLiquidity requires at least 2 inputs".to_string(),
        ));
    }

    // Outputs: updated Pool + new LPShare + optional change
    if tx.outputs.len() < 2 {
        return Err(ValidationError::InvalidLiquidity(
            "AddLiquidity requires at least 2 outputs (pool + LPShare)".to_string(),
        ));
    }

    if tx.outputs[0].output_type != OutputType::Pool {
        return Err(ValidationError::InvalidLiquidity(
            "first output must be Pool type".to_string(),
        ));
    }

    if tx.outputs[1].output_type != OutputType::LPShare {
        return Err(ValidationError::InvalidLiquidity(
            "second output must be LPShare type".to_string(),
        ));
    }

    Ok(())
}

/// Validate a RemoveLiquidity transaction structure.
pub(crate) fn validate_remove_liquidity(tx: &Transaction) -> Result<(), ValidationError> {
    // Inputs: pool UTXO + LP shares to burn
    if tx.inputs.len() < 2 {
        return Err(ValidationError::InvalidLiquidity(
            "RemoveLiquidity requires at least 2 inputs".to_string(),
        ));
    }

    // Outputs: updated Pool + returned assets + optional change
    if tx.outputs.is_empty() {
        return Err(ValidationError::InvalidLiquidity(
            "RemoveLiquidity requires at least 1 output (updated pool)".to_string(),
        ));
    }

    if tx.outputs[0].output_type != OutputType::Pool {
        return Err(ValidationError::InvalidLiquidity(
            "first output must be Pool type".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::{Input, Output, Transaction, TxType};
    use crypto::Hash;

    fn dummy_pool_tx() -> Transaction {
        let asset_b = Hash::from_bytes([0xBB; 32]);
        let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);
        // D1: total = 707 (sqrt-style) + MINIMUM_LIQUIDITY (1000) = 1707;
        // creator receives 707; the 1000 gap is the permanently locked
        // first-deposit lock.
        let pool_output = Output::pool(pool_id, asset_b, 1000, 2000, 1707, 0, 100, 30, 100);
        let lp_output = Output::lp_share(707, pool_id, Hash::from_bytes([0x01; 32]));

        Transaction {
            version: 1,
            tx_type: TxType::CreatePool,
            inputs: vec![Input::new(Hash::from_bytes([0xFF; 32]), 0)],
            outputs: vec![pool_output, lp_output],
            extra_data: vec![],
        }
    }

    #[test]
    fn test_create_pool_valid() {
        let tx = dummy_pool_tx();
        assert!(validate_create_pool(&tx).is_ok());
    }

    #[test]
    fn test_create_pool_no_inputs_fails() {
        let mut tx = dummy_pool_tx();
        tx.inputs.clear();
        assert!(validate_create_pool(&tx).is_err());
    }

    #[test]
    fn test_create_pool_no_outputs_fails() {
        let mut tx = dummy_pool_tx();
        tx.outputs.clear();
        assert!(validate_create_pool(&tx).is_err());
    }

    #[test]
    fn test_create_pool_wrong_first_output_fails() {
        let mut tx = dummy_pool_tx();
        tx.outputs[0] = Output::normal(1000, Hash::ZERO);
        assert!(validate_create_pool(&tx).is_err());
    }

    #[test]
    fn test_create_pool_zero_reserves_fails() {
        let asset_b = Hash::from_bytes([0xBB; 32]);
        let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);
        let pool_output = Output::pool(pool_id, asset_b, 0, 2000, 707, 0, 100, 30, 100);
        let lp_output = Output::lp_share(707, pool_id, Hash::from_bytes([0x01; 32]));

        let tx = Transaction {
            version: 1,
            tx_type: TxType::CreatePool,
            inputs: vec![Input::new(Hash::from_bytes([0xFF; 32]), 0)],
            outputs: vec![pool_output, lp_output],
            extra_data: vec![],
        };
        let err = validate_create_pool(&tx).unwrap_err();
        assert!(err.to_string().contains("greater than zero"));
    }

    #[test]
    fn test_create_pool_excessive_fee_fails() {
        let asset_b = Hash::from_bytes([0xBB; 32]);
        // fee_bps=1500 here mirrors what the Pool UTXO will stamp below; the
        // fee-exceeds-maximum check fires before the pool_id integrity check.
        let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 1500);
        let pool_output = Output::pool(pool_id, asset_b, 1000, 2000, 707, 0, 100, 1500, 100); // 15% fee
        let lp_output = Output::lp_share(707, pool_id, Hash::from_bytes([0x01; 32]));

        let tx = Transaction {
            version: 1,
            tx_type: TxType::CreatePool,
            inputs: vec![Input::new(Hash::from_bytes([0xFF; 32]), 0)],
            outputs: vec![pool_output, lp_output],
            extra_data: vec![],
        };
        let err = validate_create_pool(&tx).unwrap_err();
        assert!(err.to_string().contains("exceeds maximum"));
    }

    #[test]
    fn test_create_pool_lp_amount_mismatch_fails() {
        // Post-M3 (D1): the LPShare amount must equal
        // (total_lp_shares - MINIMUM_LIQUIDITY). Here total=1707 implies the
        // creator must get exactly 707, but they claim 500 → reject with
        // AmmMinimumLiquidity (creator-share-mismatch failure mode).
        let asset_b = Hash::from_bytes([0xBB; 32]);
        let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);
        let pool_output = Output::pool(pool_id, asset_b, 1000, 2000, 1707, 0, 100, 30, 100);
        let lp_output = Output::lp_share(500, pool_id, Hash::from_bytes([0x01; 32])); // wrong: 500+1000 != 1707

        let tx = Transaction {
            version: 1,
            tx_type: TxType::CreatePool,
            inputs: vec![Input::new(Hash::from_bytes([0xFF; 32]), 0)],
            outputs: vec![pool_output, lp_output],
            extra_data: vec![],
        };
        let err = validate_create_pool(&tx).unwrap_err();
        assert!(matches!(err, ValidationError::AmmMinimumLiquidity { .. }));
        assert!(err.to_string().contains("[ERRTX-AMM002]"));
    }

    #[test]
    fn test_swap_valid() {
        let asset_b = Hash::from_bytes([0xBB; 32]);
        let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);
        let pool_output = Output::pool(pool_id, asset_b, 1100, 910, 707, 0, 101, 30, 100);
        let swap_output = Output::normal(90, Hash::from_bytes([0x02; 32]));

        let tx = Transaction {
            version: 1,
            tx_type: TxType::Swap,
            inputs: vec![
                Input::new(Hash::from_bytes([0xAA; 32]), 0), // pool UTXO
                Input::new(Hash::from_bytes([0xBB; 32]), 0), // funding
            ],
            outputs: vec![pool_output, swap_output],
            extra_data: vec![],
        };
        assert!(validate_swap(&tx).is_ok());
    }

    #[test]
    fn test_swap_insufficient_inputs_fails() {
        let asset_b = Hash::from_bytes([0xBB; 32]);
        let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);
        let pool_output = Output::pool(pool_id, asset_b, 1100, 910, 707, 0, 101, 30, 100);

        let tx = Transaction {
            version: 1,
            tx_type: TxType::Swap,
            inputs: vec![Input::new(Hash::from_bytes([0xAA; 32]), 0)], // only 1
            outputs: vec![pool_output],
            extra_data: vec![],
        };
        assert!(validate_swap(&tx).is_err());
    }

    #[test]
    fn test_swap_b2a_valid_structure() {
        // B→A swap: tokens go in, DOLI comes out
        // The DOLI balance check is exempted for Swap TxType
        let asset_b = Hash::from_bytes([0xBB; 32]);
        let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);
        // Old pool: 1100 DOLI, 454M tokens
        // After b2a swap of 50M tokens: reserve_a decreases, reserve_b increases
        let new_pool = Output::pool(pool_id, asset_b, 991312418, 504669456, 707, 0, 101, 30, 100);
        let doli_output = Output::normal(108687582, Hash::from_bytes([0x02; 32])); // DOLI to swapper

        let tx = Transaction {
            version: 1,
            tx_type: TxType::Swap,
            inputs: vec![
                Input::new(Hash::from_bytes([0xAA; 32]), 0), // pool UTXO
                Input::new(Hash::from_bytes([0xBB; 32]), 0), // token input
                Input::new(Hash::from_bytes([0xCC; 32]), 0), // fee input
            ],
            outputs: vec![new_pool, doli_output],
            extra_data: Vec::new(),
        };
        assert!(validate_swap(&tx).is_ok());
    }

    #[test]
    fn test_pool_id_deterministic_for_same_pair_and_fee() {
        // Post-M2: pool_id is per (pair, fee_bps). Same (pair, fee_bps)
        // tuple MUST produce the same pool_id. Different fee tiers on the
        // same pair are intentionally distinct (covered in
        // crates/core/tests/pool_id_fee_bps.rs::pool_id_distinct_per_fee_tier).
        let asset_b = Hash::from_bytes([0xBB; 32]);
        let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);
        let pool_id_2 = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);
        assert_eq!(
            pool_id, pool_id_2,
            "same (pair, fee_bps) must produce same pool_id"
        );
    }
}

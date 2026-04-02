//! Validation rules for NFT fractionalization transactions.
//!
//! Structural validation only -- UTXO-level checks (input type, already-fractionalized)
//! happen in `validate_transaction_with_utxos` and apply_block.

use crate::transaction::{Output, OutputType, Transaction};
use crate::validation::error::ValidationError;

/// Validate a FractionalizeNft transaction (structural checks).
///
/// Rules enforced:
/// 1. At least 1 input (NFT + optional fee inputs)
/// 2. At least 2 outputs (fractionalized NFT + fraction tokens)
/// 3. Output 0 must be NFT type
/// 4. Output 0 must have valid fractionalization metadata at the end of extra_data
/// 5. Output 1 must be FungibleAsset type
/// 6. Fraction asset_id must match BLAKE3("DOLI_FRAC" || token_id)
/// 7. Shares must be > 0 and consistent between output 0 metadata and output 1
pub(crate) fn validate_fractionalize_nft(tx: &Transaction) -> Result<(), ValidationError> {
    // 1. Must have at least 1 input
    if tx.inputs.is_empty() {
        return Err(ValidationError::InvalidFractionalization(
            "must have at least 1 input".to_string(),
        ));
    }

    // 2. Must have at least 2 outputs
    if tx.outputs.len() < 2 {
        return Err(ValidationError::InvalidFractionalization(
            "must have at least 2 outputs (fractionalized NFT + fraction tokens)".to_string(),
        ));
    }

    // 3. Output 0 must be NFT type
    if tx.outputs[0].output_type != OutputType::NFT {
        return Err(ValidationError::InvalidFractionalization(
            "output 0 must be UniqueAsset (NFT) type".to_string(),
        ));
    }

    // 4. Output 0 must have valid fractionalization metadata
    let (frac_asset_id, total_shares) =
        tx.outputs[0].fractionalization_metadata().ok_or_else(|| {
            ValidationError::InvalidFractionalization(
                "output 0 missing valid fractionalization metadata".to_string(),
            )
        })?;

    // Must also have valid underlying NFT metadata
    let (token_id, _content_hash) = tx.outputs[0].nft_metadata().ok_or_else(|| {
        ValidationError::InvalidFractionalization("output 0 missing valid NFT metadata".to_string())
    })?;

    // 5. Output 1 must be FungibleAsset type
    if tx.outputs[1].output_type != OutputType::FungibleAsset {
        return Err(ValidationError::InvalidFractionalization(
            "output 1 must be FungibleAsset type".to_string(),
        ));
    }

    // 6. Fraction asset_id must match BLAKE3("DOLI_FRAC" || token_id)
    let expected_asset_id = Output::fraction_asset_id(&token_id);
    if frac_asset_id != expected_asset_id {
        return Err(ValidationError::InvalidFractionalization(
            "fraction asset_id does not match BLAKE3(DOLI_FRAC || token_id)".to_string(),
        ));
    }

    // Verify output 1's asset_id matches
    let (out1_asset_id, out1_supply, _ticker) =
        tx.outputs[1].fungible_asset_metadata().ok_or_else(|| {
            ValidationError::InvalidFractionalization(
                "output 1 has invalid FungibleAsset metadata".to_string(),
            )
        })?;

    if out1_asset_id != expected_asset_id {
        return Err(ValidationError::InvalidFractionalization(
            "output 1 asset_id does not match expected fraction asset_id".to_string(),
        ));
    }

    // 7. Shares must be > 0 and consistent
    if total_shares == 0 {
        return Err(ValidationError::InvalidFractionalization(
            "total_shares must be greater than zero".to_string(),
        ));
    }

    if tx.outputs[1].amount != total_shares {
        return Err(ValidationError::InvalidFractionalization(format!(
            "output 1 amount ({}) must equal total_shares ({})",
            tx.outputs[1].amount, total_shares
        )));
    }

    if out1_supply != total_shares {
        return Err(ValidationError::InvalidFractionalization(format!(
            "output 1 total_supply ({}) must equal total_shares ({})",
            out1_supply, total_shares
        )));
    }

    Ok(())
}

/// Validate a RedeemNft transaction (structural checks).
///
/// Rules enforced:
/// 1. At least 2 inputs (fractionalized NFT + fraction tokens)
/// 2. At least 1 output (unlocked NFT)
/// 3. Output 0 must be NFT type WITHOUT fractionalization metadata
pub(crate) fn validate_redeem_nft(tx: &Transaction) -> Result<(), ValidationError> {
    // 1. Must have at least 2 inputs
    if tx.inputs.len() < 2 {
        return Err(ValidationError::InvalidRedemption(
            "must have at least 2 inputs (fractionalized NFT + fraction tokens)".to_string(),
        ));
    }

    // 2. Must have at least 1 output
    if tx.outputs.is_empty() {
        return Err(ValidationError::InvalidRedemption(
            "must have at least 1 output (unlocked NFT)".to_string(),
        ));
    }

    // 3. Output 0 must be NFT type
    if tx.outputs[0].output_type != OutputType::NFT {
        return Err(ValidationError::InvalidRedemption(
            "output 0 must be UniqueAsset (NFT) type".to_string(),
        ));
    }

    // Output 0 must NOT be fractionalized (it should be unlocked)
    if tx.outputs[0].is_fractionalized() {
        return Err(ValidationError::InvalidRedemption(
            "output 0 must not have fractionalization metadata (NFT should be unlocked)"
                .to_string(),
        ));
    }

    // Output 0 must have valid NFT metadata
    if tx.outputs[0].nft_metadata().is_none() {
        return Err(ValidationError::InvalidRedemption(
            "output 0 missing valid NFT metadata".to_string(),
        ));
    }

    Ok(())
}

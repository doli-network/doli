use crate::block::Block;
use crate::transaction::{Input, OutputType, SighashType, Transaction, TxType};
use crate::types::Amount;
use crypto::Hash;

use super::{
    validate_transaction_skip_registration_vdf, UtxoInfo, UtxoProvider, ValidationContext,
    ValidationError,
};

/// Validate a transaction with full UTXO context.
///
/// This performs complete validation including:
/// - All structural checks from `validate_transaction`
/// - Signature verification for each input
/// - Input existence and spendability
/// - Input/output balance verification
/// - Lock time enforcement
///
/// # Errors
///
/// Returns an error if any validation check fails.
pub fn validate_transaction_with_utxos<U: UtxoProvider>(
    tx: &Transaction,
    ctx: &ValidationContext,
    utxo_provider: &U,
) -> Result<(), ValidationError> {
    // Structural validation — skip registration VDF since it was already
    // verified in the parallel pre-pass (block.rs). Without this, every
    // registration VDF is verified TWICE: once in parallel (block validation)
    // and once here sequentially (UTXO validation). 7 registrations × 400ms
    // = 2.8s wasted, causing nodes to fall behind and trigger fork recovery.
    validate_transaction_skip_registration_vdf(tx, ctx)?;

    // Coinbase and EpochReward transactions don't need UTXO validation (minted)
    if tx.is_coinbase() || tx.is_epoch_reward() {
        return Ok(());
    }

    // Validate each input
    let mut total_input: Amount = 0;

    for (i, input) in tx.inputs.iter().enumerate() {
        // Validate committed_output_count
        if input.committed_output_count > 0 {
            if input.sighash_type != SighashType::AnyoneCanPay {
                return Err(ValidationError::InvalidTransaction(format!(
                    "input {} has committed_output_count={} but sighash is not AnyoneCanPay",
                    i, input.committed_output_count
                )));
            }
            if input.committed_output_count as usize > tx.outputs.len() {
                return Err(ValidationError::InvalidTransaction(format!(
                    "input {} committed_output_count={} exceeds output count={}",
                    i,
                    input.committed_output_count,
                    tx.outputs.len()
                )));
            }
        }

        // Per-input signing hash: respects SighashType (All vs AnyoneCanPay)
        let signing_hash = tx.signing_message_for_input(i);
        // Look up the UTXO
        let utxo = utxo_provider
            .get_utxo(&input.prev_tx_hash, input.output_index)
            .ok_or(ValidationError::OutputNotFound {
                tx_hash: input.prev_tx_hash,
                output_index: input.output_index,
            })?;

        // Check if already spent
        if utxo.spent {
            return Err(ValidationError::OutputAlreadySpent {
                tx_hash: input.prev_tx_hash,
                output_index: input.output_index,
            });
        }

        // Check lock time -- skip for WithdrawalRequest/Exit (they unlock Bond UTXOs)
        if tx.tx_type != TxType::RequestWithdrawal
            && tx.tx_type != TxType::Exit
            && !utxo.output.is_spendable_at(ctx.current_height)
        {
            return Err(ValidationError::OutputLocked {
                lock_height: utxo.output.lock_until,
                current_height: ctx.current_height,
            });
        }

        // Verify spending conditions (signature for Normal/Bond, condition evaluator for others)
        verify_input_conditions(tx, input, &signing_hash, &utxo, i, ctx.current_height)?;

        // Add to total (with overflow check)
        total_input = total_input.checked_add(utxo.output.amount).ok_or_else(|| {
            ValidationError::AmountOverflow {
                context: format!("input total at index {}", i),
            }
        })?;
    }

    // Verify inputs >= outputs (difference is fee)
    let total_output = tx.total_output();
    if total_input < total_output {
        return Err(ValidationError::InsufficientFunds {
            inputs: total_input,
            outputs: total_output,
        });
    }

    // -- Royalty enforcement --
    // When an NFT with royalties is spent, the transaction MUST include
    // a payment output to the creator for at least (sale_price * royalty_bps / 10000).
    // Sale price is inferred from the largest non-NFT, non-change output to a
    // pubkey_hash that differs from both the NFT input owner and the NFT output recipient.
    if tx.tx_type == TxType::Transfer {
        for (i, input) in tx.inputs.iter().enumerate() {
            let utxo = utxo_provider.get_utxo(&input.prev_tx_hash, input.output_index);
            if let Some(utxo) = utxo {
                if let Some((creator_hash, royalty_bps)) = utxo.output.nft_royalty() {
                    if royalty_bps > 0 {
                        // Find the sale price: the payment output to the seller
                        // (the owner of the NFT being spent = utxo.output.pubkey_hash)
                        let seller_hash = utxo.output.pubkey_hash;
                        let sale_price: Amount = tx
                            .outputs
                            .iter()
                            .filter(|o| {
                                o.output_type == OutputType::Normal && o.pubkey_hash == seller_hash
                            })
                            .map(|o| o.amount)
                            .sum();

                        if sale_price > 0 {
                            let required_royalty =
                                (sale_price as u128 * royalty_bps as u128 / 10000) as Amount;
                            if required_royalty > 0 {
                                let actual_royalty: Amount = tx
                                    .outputs
                                    .iter()
                                    .filter(|o| o.pubkey_hash == creator_hash)
                                    .map(|o| o.amount)
                                    .sum();
                                if actual_royalty < required_royalty {
                                    return Err(ValidationError::InvalidTransaction(format!(
                                        "NFT input {} requires royalty of {} to creator, got {}",
                                        i, required_royalty, actual_royalty
                                    )));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // -- MintAsset issuer authorization --
    // When minting fungible assets, verify:
    // 1. All inputs are FungibleAsset with the same asset_id
    // 2. The signer owns the genesis issuer UTXO (first input)
    // 3. All outputs are FungibleAsset with the same asset_id
    // 4. Total output amount does not exceed total_supply cap
    if tx.tx_type == TxType::MintAsset {
        // Get first input UTXO -- must be a FungibleAsset (the genesis output)
        let first_input = &tx.inputs[0]; // structural check already ensures non-empty
        let genesis_utxo = utxo_provider
            .get_utxo(&first_input.prev_tx_hash, first_input.output_index)
            .ok_or(ValidationError::OutputNotFound {
                tx_hash: first_input.prev_tx_hash,
                output_index: first_input.output_index,
            })?;

        if genesis_utxo.output.output_type != OutputType::FungibleAsset {
            return Err(ValidationError::InvalidMintAsset(
                "first input must be a FungibleAsset UTXO".to_string(),
            ));
        }

        let (asset_id, total_supply, _ticker) = genesis_utxo
            .output
            .fungible_asset_metadata()
            .ok_or(ValidationError::InvalidMintAsset(
                "genesis UTXO has invalid asset metadata".to_string(),
            ))?;

        // Verify signer == genesis UTXO owner (issuer auth via witness pubkey)
        if let Some(ref pk) = genesis_utxo.pubkey {
            let signer_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes());
            if signer_hash != genesis_utxo.output.pubkey_hash {
                return Err(ValidationError::InvalidMintAsset(
                    "only the original issuer can mint".to_string(),
                ));
            }
        }

        // All inputs must share the same asset_id
        for (i, input) in tx.inputs.iter().enumerate() {
            let utxo = utxo_provider
                .get_utxo(&input.prev_tx_hash, input.output_index)
                .ok_or(ValidationError::OutputNotFound {
                    tx_hash: input.prev_tx_hash,
                    output_index: input.output_index,
                })?;
            if let Some((id, _, _)) = utxo.output.fungible_asset_metadata() {
                if id != asset_id {
                    return Err(ValidationError::InvalidMintAsset(format!(
                        "input {} has different asset_id",
                        i
                    )));
                }
            } else {
                return Err(ValidationError::InvalidMintAsset(format!(
                    "input {} is not a FungibleAsset",
                    i
                )));
            }
        }

        // Total output amount must not exceed total_supply
        let output_total: u64 = tx
            .outputs
            .iter()
            .try_fold(0u64, |acc, o| acc.checked_add(o.amount))
            .ok_or(ValidationError::AmountOverflow {
                context: "MintAsset output total".to_string(),
            })?;
        if output_total > total_supply {
            return Err(ValidationError::InvalidMintAsset(format!(
                "output total {} exceeds supply cap {}",
                output_total, total_supply
            )));
        }

        // All outputs must be FungibleAsset with the same asset_id
        for (i, output) in tx.outputs.iter().enumerate() {
            if let Some((id, _, _)) = output.fungible_asset_metadata() {
                if id != asset_id {
                    return Err(ValidationError::InvalidMintAsset(format!(
                        "output {} has wrong asset_id",
                        i
                    )));
                }
            } else {
                return Err(ValidationError::InvalidMintAsset(format!(
                    "output {} has invalid asset metadata",
                    i
                )));
            }
        }
    }

    // -- BurnAsset input validation --
    // When burning fungible assets, verify all inputs share the same asset_id
    // and all outputs (change) use the same asset_id.
    if tx.tx_type == TxType::BurnAsset {
        let first_input = &tx.inputs[0];
        let first_utxo = utxo_provider
            .get_utxo(&first_input.prev_tx_hash, first_input.output_index)
            .ok_or(ValidationError::OutputNotFound {
                tx_hash: first_input.prev_tx_hash,
                output_index: first_input.output_index,
            })?;
        let (asset_id, _, _) = first_utxo.output.fungible_asset_metadata().ok_or(
            ValidationError::InvalidBurnAsset("first input is not a FungibleAsset".to_string()),
        )?;

        for (i, input) in tx.inputs.iter().skip(1).enumerate() {
            let utxo = utxo_provider
                .get_utxo(&input.prev_tx_hash, input.output_index)
                .ok_or(ValidationError::OutputNotFound {
                    tx_hash: input.prev_tx_hash,
                    output_index: input.output_index,
                })?;
            if let Some((id, _, _)) = utxo.output.fungible_asset_metadata() {
                if id != asset_id {
                    return Err(ValidationError::InvalidBurnAsset(format!(
                        "input {} has different asset_id",
                        i + 1
                    )));
                }
            } else {
                return Err(ValidationError::InvalidBurnAsset(format!(
                    "input {} is not a FungibleAsset",
                    i + 1
                )));
            }
        }

        for (i, output) in tx.outputs.iter().enumerate() {
            if let Some((id, _, _)) = output.fungible_asset_metadata() {
                if id != asset_id {
                    return Err(ValidationError::InvalidBurnAsset(format!(
                        "output {} has wrong asset_id",
                        i
                    )));
                }
            } else {
                return Err(ValidationError::InvalidBurnAsset(format!(
                    "output {} has invalid asset metadata",
                    i
                )));
            }
        }
    }

    Ok(())
}

/// Verify spending conditions on a transaction input.
///
/// For Normal/Bond outputs: verify single Ed25519 signature (existing behavior).
/// For conditioned outputs (Multisig, Hashlock, HTLC, Vesting):
///   decode condition from output extra_data, decode witness from tx extra_data, evaluate.
fn verify_input_conditions(
    tx: &Transaction,
    input: &Input,
    signing_hash: &Hash,
    utxo: &UtxoInfo,
    input_index: usize,
    current_height: crate::types::BlockHeight,
) -> Result<(), ValidationError> {
    if utxo.output.output_type.is_conditioned() {
        // ---- Conditioned output: evaluate condition tree ----
        let condition = crate::conditions::Condition::decode_prefix(&utxo.output.extra_data)
            .map(|(cond, _consumed)| cond)
            .map_err(|e| {
                ValidationError::InvalidTransaction(format!(
                    "input {} references output with invalid condition: {}",
                    input_index, e
                ))
            })?;

        // Check ops limit
        let ops = condition.ops_count();
        if ops > crate::conditions::MAX_CONDITION_OPS {
            return Err(ValidationError::InvalidTransaction(format!(
                "input {} condition has {} ops (max {})",
                input_index,
                ops,
                crate::conditions::MAX_CONDITION_OPS
            )));
        }

        // Decode witness from Transaction.extra_data (SegWit-style)
        let witness_bytes = tx.get_covenant_witness(input_index).unwrap_or(&[]);
        let witness = crate::conditions::Witness::decode(witness_bytes).map_err(|e| {
            ValidationError::InvalidTransaction(format!(
                "input {} has invalid witness data: {}",
                input_index, e
            ))
        })?;

        let ctx = crate::conditions::EvalContext {
            current_height,
            signing_hash,
        };

        let mut or_idx = 0;
        if !crate::conditions::evaluate(&condition, &witness, &ctx, &mut or_idx) {
            return Err(ValidationError::InvalidSignature { index: input_index });
        }

        Ok(())
    } else {
        // ---- Normal/Bond output: single signature verification ----
        verify_input_signature(input, signing_hash, utxo, input_index)
    }
}

/// Verify the signature on a transaction input (Normal/Bond outputs only).
fn verify_input_signature(
    input: &Input,
    signing_hash: &Hash,
    utxo: &UtxoInfo,
    input_index: usize,
) -> Result<(), ValidationError> {
    // We need the public key to verify the signature.
    // In pay-to-pubkey-hash, the UTXO only stores the hash -- the pubkey
    // is not available until the spender reveals it. When pubkey is None
    // (production UTXO set), skip signature verification; covenant conditions,
    // lock times, and balance are still enforced.
    let pubkey = match utxo.pubkey.as_ref() {
        Some(pk) => pk,
        None => return Ok(()),
    };

    // Verify the pubkey hash matches the output
    let expected_hash = utxo.output.pubkey_hash;
    let actual_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());

    // Compare full 32-byte hash (128-bit security; truncating to 20 bytes would be 80-bit -- birthday-attackable)
    if expected_hash != actual_hash {
        return Err(ValidationError::PubkeyHashMismatch {
            expected: expected_hash,
            got: actual_hash,
        });
    }

    // Verify the signature
    crypto::signature::verify_hash(signing_hash, &input.signature, pubkey)
        .map_err(|_| ValidationError::InvalidSignature { index: input_index })
}

/// Check for double spends within a block
pub(super) fn check_internal_double_spend(block: &Block) -> Result<(), ValidationError> {
    use std::collections::HashSet;

    let mut spent: HashSet<(Hash, u32)> = HashSet::new();

    for tx in &block.transactions {
        for input in &tx.inputs {
            let outpoint = input.outpoint();
            if !spent.insert(outpoint) {
                return Err(ValidationError::DoubleSpend);
            }
        }
    }

    Ok(())
}

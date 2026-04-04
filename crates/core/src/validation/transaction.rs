use crate::consensus::TOTAL_SUPPLY;
use crate::transaction::{max_extra_data_size, Output, OutputType, Transaction, TxType};
use crate::types::Amount;
use crypto::Hash;

use super::registration::{validate_registration_data, validate_registration_data_skip_vdf};
use super::tx_types::{
    validate_add_bond_data, validate_burn_asset, validate_claim_bond_data, validate_claim_data,
    validate_delegate_bond_data, validate_epoch_reward_data, validate_exit_data,
    validate_maintainer_change_data, validate_mint_asset, validate_protocol_activation_data,
    validate_revoke_delegation_data, validate_slash_data, validate_slash_data_skip_vdf,
    validate_withdrawal_request_data,
};
use super::{ValidationContext, ValidationError};

/// Validate a regular transaction (structural validation only).
///
/// This performs all checks that don't require UTXO access:
/// - Version check
/// - Non-empty inputs and outputs
/// - Positive output amounts
/// - No amount overflow in outputs
/// - Type-specific validation (registration data)
///
/// For full validation including signatures and balances,
/// use `validate_transaction_with_utxos`.
pub fn validate_transaction(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    // 1. Version check
    if tx.version != 1 {
        return Err(ValidationError::InvalidVersion(tx.version));
    }

    // 2. Must have at least one input (unless coinbase, exit, claim, claim_bond, slash,
    //    add_bond, request_withdrawal, or epoch_reward)
    //    Note: add_bond requires inputs but handles its own error for specificity
    if tx.inputs.is_empty()
        && !tx.is_coinbase()
        && !tx.is_exit()
        && !tx.is_claim_reward()
        && !tx.is_claim_bond()
        && !tx.is_slash_producer()
        && !tx.is_add_bond()
        && !tx.is_request_withdrawal()
        && tx.tx_type != TxType::ClaimWithdrawal
        && tx.tx_type != TxType::MintAsset
        && tx.tx_type != TxType::BurnAsset
        && !tx.is_epoch_reward()
        && !tx.is_delegate_bond()
        && !tx.is_revoke_delegation()
        && !tx.is_registration()
        && !tx.is_maintainer_change()
        && !tx.is_protocol_activation()
    // Registration/maintainer/protocol txs handle their own input validation
    {
        return Err(ValidationError::InvalidTransaction(format!(
            "[ERRTX001] transaction must have inputs (tx_type={:?})",
            tx.tx_type
        )));
    }

    // 3. Must have at least one output (unless exit, slash_producer, add_bond, or request_withdrawal)
    //    Note: epoch_reward requires exactly one output but handles its own errors
    if tx.outputs.is_empty()
        && !tx.is_exit()
        && !tx.is_slash_producer()
        && !tx.is_add_bond()
        && !tx.is_request_withdrawal()
        && tx.tx_type != TxType::ClaimWithdrawal
        && tx.tx_type != TxType::MintAsset
        && tx.tx_type != TxType::BurnAsset
        && !tx.is_epoch_reward()
        && !tx.is_delegate_bond()
        && !tx.is_revoke_delegation()
        && !tx.is_registration()
        && !tx.is_maintainer_change()
        && !tx.is_protocol_activation()
    // Registration/maintainer/protocol txs handle their own output validation
    {
        return Err(ValidationError::InvalidTransaction(format!(
            "[ERRTX002] transaction must have outputs (tx_type={:?})",
            tx.tx_type
        )));
    }

    // 4. Validate all outputs
    let total_output = validate_outputs(&tx.outputs, ctx)?;

    // 5. Total output must not exceed max supply
    if total_output > TOTAL_SUPPLY {
        return Err(ValidationError::AmountExceedsSupply {
            amount: total_output,
            max: TOTAL_SUPPLY,
        });
    }

    // 6. Type-specific validation
    match tx.tx_type {
        TxType::Transfer => {
            // Transfer transactions should have empty extra_data
            // (or use it for memo, but we don't validate that)
        }
        TxType::Registration => {
            validate_registration_data(tx, ctx)?;
        }
        TxType::Exit => {
            validate_exit_data(tx)?;
        }
        TxType::ClaimReward => {
            validate_claim_data(tx)?;
        }
        TxType::ClaimBond => {
            validate_claim_bond_data(tx)?;
        }
        TxType::SlashProducer => {
            validate_slash_data(tx, ctx)?;
        }
        TxType::Coinbase => {
            // Coinbase validation is handled at the block level
            // (must be first tx, amount must match block reward, etc.)
        }
        TxType::AddBond => {
            validate_add_bond_data(tx)?;
        }
        TxType::RequestWithdrawal => {
            validate_withdrawal_request_data(tx)?;
        }
        TxType::ClaimWithdrawal => {
            // Reserved — DO NOT REUSE discriminant 9. Tombstone for wire compat.
            return Err(ValidationError::InvalidClaimWithdrawal(
                "ClaimWithdrawal is not supported".to_string(),
            ));
        }
        TxType::MintAsset => {
            validate_mint_asset(tx)?;
        }
        TxType::BurnAsset => {
            validate_burn_asset(tx)?;
        }
        TxType::EpochReward => {
            validate_epoch_reward_data(tx)?;
        }
        TxType::RemoveMaintainer => {
            validate_maintainer_change_data(tx)?;
        }
        TxType::AddMaintainer => {
            validate_maintainer_change_data(tx)?;
        }
        TxType::DelegateBond => {
            validate_delegate_bond_data(tx)?;
        }
        TxType::RevokeDelegation => {
            validate_revoke_delegation_data(tx)?;
        }
        TxType::ProtocolActivation => {
            validate_protocol_activation_data(tx)?;
        }
        TxType::CreatePool => {
            super::pool::validate_create_pool(tx)?;
        }
        TxType::Swap => {
            super::pool::validate_swap(tx)?;
        }
        TxType::AddLiquidity => {
            super::pool::validate_add_liquidity(tx)?;
        }
        TxType::RemoveLiquidity => {
            super::pool::validate_remove_liquidity(tx)?;
        }
        TxType::CreateLoan => {
            super::lending::validate_create_loan(tx)?;
        }
        TxType::RepayLoan => {
            super::lending::validate_repay_loan(tx)?;
        }
        TxType::LiquidateLoan => {
            super::lending::validate_liquidate_loan(tx)?;
        }
        TxType::LendingDeposit => {
            super::lending::validate_lending_deposit(tx)?;
        }
        TxType::LendingWithdraw => {
            super::lending::validate_lending_withdraw(tx)?;
        }
        TxType::FractionalizeNft => {
            super::fractionalize::validate_fractionalize_nft(tx)?;
        }
        TxType::RedeemNft => {
            super::fractionalize::validate_redeem_nft(tx)?;
        }
    }

    Ok(())
}

/// Same as `validate_transaction` but skips VDF verification for Registration
/// and SlashProducer TXs.
///
/// Used when VDFs have already been verified in parallel (block.rs).
/// All other validation (bond amounts, BLS PoP, registration chain, structural
/// slash checks) still runs.
pub fn validate_transaction_skip_registration_vdf(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    match tx.tx_type {
        TxType::Registration => {
            if tx.version != 1 {
                return Err(ValidationError::InvalidVersion(tx.version));
            }
            let total_output = validate_outputs(&tx.outputs, ctx)?;
            if total_output > TOTAL_SUPPLY {
                return Err(ValidationError::AmountExceedsSupply {
                    amount: total_output,
                    max: TOTAL_SUPPLY,
                });
            }
            validate_registration_data_skip_vdf(tx, ctx)?;
            Ok(())
        }
        TxType::SlashProducer => {
            if tx.version != 1 {
                return Err(ValidationError::InvalidVersion(tx.version));
            }
            validate_slash_data_skip_vdf(tx, ctx)?;
            Ok(())
        }
        _ => validate_transaction(tx, ctx),
    }
}

/// Validate transaction outputs and compute safe total.
///
/// Returns the total output amount, or an error if any output is invalid
/// or if the total would overflow.
pub(super) fn validate_outputs(
    outputs: &[Output],
    ctx: &ValidationContext,
) -> Result<Amount, ValidationError> {
    let mut total: Amount = 0;
    let mut seen_nft_token_ids: std::collections::HashSet<[u8; 32]> =
        std::collections::HashSet::new();

    for (i, output) in outputs.iter().enumerate() {
        // Amount must be positive.
        // Pool outputs exempt: reserves tracked in extra_data.
        // Non-native types (FungibleAsset, LPShare, Collateral) carry token
        // units / LP shares — zero is also prohibited for them.
        if output.amount == 0 && output.output_type != OutputType::Pool {
            return Err(ValidationError::InvalidTransaction(format!(
                "[ERRTX003] output {} has zero amount (type={:?})",
                i, output.output_type
            )));
        }

        // Amount must not exceed max DOLI supply individually (native only).
        // Non-native types may hold token supplies larger than TOTAL_SUPPLY.
        if output.output_type.is_native_amount() && output.amount > TOTAL_SUPPLY {
            return Err(ValidationError::AmountExceedsSupply {
                amount: output.amount,
                max: TOTAL_SUPPLY,
            });
        }

        // Accumulate only native DOLI amounts for the TOTAL_SUPPLY check.
        if output.output_type.is_native_amount() {
            total = total.checked_add(output.amount).ok_or_else(|| {
                ValidationError::AmountOverflow {
                    context: format!("output total at index {}", i),
                }
            })?;
        }

        // Validate extra_data size limit (era-aware: doubles every ~4 years)
        let max_data = max_extra_data_size(ctx.current_height);
        if output.extra_data.len() > max_data {
            return Err(ValidationError::InvalidTransaction(format!(
                "[ERRTX004] output {} extra_data exceeds max size ({} > {} at height {})",
                i,
                output.extra_data.len(),
                max_data,
                ctx.current_height,
            )));
        }

        // Validate output type consistency
        match output.output_type {
            OutputType::Normal => {
                // Normal outputs should have lock_until = 0
                if output.lock_until != 0 {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX005] normal output {} has non-zero lock_until={}",
                        i, output.lock_until
                    )));
                }
                // Normal outputs must not carry extra_data
                if !output.extra_data.is_empty() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX006] normal output {} has non-empty extra_data ({} bytes)",
                        i,
                        output.extra_data.len()
                    )));
                }
            }
            OutputType::Bond => {
                // Bond outputs must have a future lock time
                if output.lock_until == 0 {
                    return Err(ValidationError::InvalidBond(format!(
                        "bond output {} has zero lock_until",
                        i
                    )));
                }
                // Bond outputs must carry exactly 4 bytes of extra_data (creation_slot)
                if output.extra_data.len() != 4 {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX007] bond output {} has {} bytes extra_data, expected 4 (creation_slot)",
                        i,
                        output.extra_data.len()
                    )));
                }
            }
            // Covenant output types: validate activation and condition encoding
            OutputType::Multisig
            | OutputType::Hashlock
            | OutputType::HTLC
            | OutputType::Vesting => {
                // Activation height gating: reject conditioned outputs before activation
                let activation = ctx.params.covenants_activation_height(&ctx.network);
                if ctx.current_height < activation {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX008] conditioned output {} (type={:?}) rejected: covenants activate at height {}, current={}",
                        i, output.output_type, activation, ctx.current_height
                    )));
                }
                if output.extra_data.is_empty() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX009] conditioned output {} (type={:?}) has empty extra_data",
                        i, output.output_type
                    )));
                }
                // Validate condition decodes successfully
                match crate::conditions::Condition::decode(&output.extra_data) {
                    Ok(cond) => {
                        // Guard conditions require separate activation height
                        if cond.contains_guard() {
                            let guard_activation =
                                ctx.params.guards_activation_height(&ctx.network);
                            if ctx.current_height < guard_activation {
                                return Err(ValidationError::InvalidTransaction(format!(
                                    "[ERRTX011] output {} contains guard condition: guards activate at height {}",
                                    i, guard_activation
                                )));
                            }
                        }
                    }
                    Err(e) => {
                        return Err(ValidationError::InvalidTransaction(format!(
                            "[ERRTX010] conditioned output {} (type={:?}) has invalid condition: {}",
                            i, output.output_type, e
                        )));
                    }
                }
            }
            OutputType::NFT => {
                let activation = ctx.params.covenants_activation_height(&ctx.network);
                if ctx.current_height < activation {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX011] NFT output {} rejected: covenants activate at height {}, current={}",
                        i, activation, ctx.current_height
                    )));
                }
                if output.extra_data.is_empty() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX012] NFT output {} has empty extra_data",
                        i
                    )));
                }
                if let Err(e) = crate::conditions::Condition::decode_prefix(&output.extra_data) {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX013] NFT output {} has invalid condition: {}",
                        i, e
                    )));
                }
                if output.nft_metadata().is_none() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX014] NFT output {} has invalid or missing NFT metadata ({} bytes extra_data)",
                        i, output.extra_data.len()
                    )));
                }
                // Reject duplicate NFT token_ids within the same transaction
                if let Some((token_id, _)) = output.nft_metadata() {
                    if !seen_nft_token_ids.insert(*token_id.as_bytes()) {
                        return Err(ValidationError::InvalidTransaction(format!(
                            "[ERRTX015] duplicate NFT token_id in output {}",
                            i
                        )));
                    }
                }
            }
            OutputType::FungibleAsset => {
                let activation = ctx.params.covenants_activation_height(&ctx.network);
                if ctx.current_height < activation {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX016] FungibleAsset output {} rejected: covenants activate at height {}, current={}",
                        i, activation, ctx.current_height
                    )));
                }
                if output.extra_data.is_empty() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX017] FungibleAsset output {} has empty extra_data",
                        i
                    )));
                }
                if let Err(e) = crate::conditions::Condition::decode_prefix(&output.extra_data) {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX018] FungibleAsset output {} has invalid condition: {}",
                        i, e
                    )));
                }
                if output.fungible_asset_metadata().is_none() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX019] FungibleAsset output {} has invalid or missing asset metadata ({} bytes extra_data)",
                        i, output.extra_data.len()
                    )));
                }
            }
            OutputType::BridgeHTLC => {
                let activation = ctx.params.covenants_activation_height(&ctx.network);
                if ctx.current_height < activation {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX020] BridgeHTLC output {} rejected: covenants activate at height {}, current={}",
                        i, activation, ctx.current_height
                    )));
                }
                if output.extra_data.is_empty() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX021] BridgeHTLC output {} has empty extra_data",
                        i
                    )));
                }
                if let Err(e) = crate::conditions::Condition::decode_prefix(&output.extra_data) {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX022] BridgeHTLC output {} has invalid condition: {}",
                        i, e
                    )));
                }
                if output.bridge_htlc_metadata().is_none() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX023] BridgeHTLC output {} has invalid or missing bridge metadata ({} bytes extra_data)",
                        i, output.extra_data.len()
                    )));
                }
            }
            OutputType::Pool => {
                if output.extra_data.len() < crate::transaction::POOL_METADATA_SIZE {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX024] Pool output {} has invalid extra_data size: {} < {}",
                        i,
                        output.extra_data.len(),
                        crate::transaction::POOL_METADATA_SIZE
                    )));
                }
                if output.pool_metadata().is_none() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX025] Pool output {} has invalid or undecodable metadata ({} bytes extra_data)",
                        i, output.extra_data.len()
                    )));
                }
            }
            OutputType::LPShare => {
                if output.extra_data.len() < crate::transaction::LP_SHARE_METADATA_SIZE {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX026] LPShare output {} has invalid extra_data size: {} < {}",
                        i,
                        output.extra_data.len(),
                        crate::transaction::LP_SHARE_METADATA_SIZE
                    )));
                }
                if output.lp_share_metadata().is_none() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX027] LPShare output {} has invalid metadata ({} bytes extra_data)",
                        i,
                        output.extra_data.len()
                    )));
                }
            }
            OutputType::Collateral => {
                if output.extra_data.len() < crate::transaction::COLLATERAL_METADATA_SIZE {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX028] Collateral output {} has invalid extra_data size: {} < {}",
                        i,
                        output.extra_data.len(),
                        crate::transaction::COLLATERAL_METADATA_SIZE
                    )));
                }
                if output.collateral_metadata().is_none() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX029] Collateral output {} has invalid or undecodable metadata ({} bytes extra_data)",
                        i, output.extra_data.len()
                    )));
                }
            }
            OutputType::LendingDeposit => {
                if output.extra_data.len() < crate::transaction::LENDING_DEPOSIT_METADATA_SIZE {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX030] LendingDeposit output {} has invalid extra_data size: {} < {}",
                        i,
                        output.extra_data.len(),
                        crate::transaction::LENDING_DEPOSIT_METADATA_SIZE
                    )));
                }
                if output.lending_deposit_metadata().is_none() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "[ERRTX031] LendingDeposit output {} has invalid or undecodable metadata ({} bytes extra_data)",
                        i, output.extra_data.len()
                    )));
                }
            }
        }

        // Pubkey hash must not be zero (except for burn address)
        if output.pubkey_hash == Hash::ZERO {
            return Err(ValidationError::InvalidTransaction(format!(
                "[ERRTX032] output {} has zero pubkey_hash (type={:?}, amount={})",
                i, output.output_type, output.amount
            )));
        }
    }

    Ok(total)
}

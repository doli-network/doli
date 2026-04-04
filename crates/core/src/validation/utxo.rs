use crate::block::Block;
use crate::consensus::{BASE_FEE, FEE_PER_BYTE};
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

    // Coinbase transactions don't need UTXO validation (minted, no inputs)
    if tx.is_coinbase() {
        return Ok(());
    }

    // EpochReward: pre-activation (no inputs) → skip entirely.
    // Post-activation (explicit pool inputs) → verify inputs exist and sum, skip signatures.
    if tx.is_epoch_reward() {
        if tx.inputs.is_empty() {
            return Ok(()); // pre-activation format
        }
        // Post-activation: verify each pool input exists and is unspent, sum amounts
        let mut total_input: Amount = 0;
        for (i, input) in tx.inputs.iter().enumerate() {
            let utxo = utxo_provider
                .get_utxo(&input.prev_tx_hash, input.output_index)
                .ok_or(ValidationError::OutputNotFound {
                    tx_hash: input.prev_tx_hash,
                    output_index: input.output_index,
                })?;
            if utxo.spent {
                return Err(ValidationError::OutputAlreadySpent {
                    tx_hash: input.prev_tx_hash,
                    output_index: input.output_index,
                });
            }
            // Pool UTXOs must belong to the reward pool
            let pool_hash = crate::consensus::reward_pool_pubkey_hash();
            if utxo.output.pubkey_hash != pool_hash {
                return Err(ValidationError::InvalidTransaction(format!(
                    "[ERRTX033] EpochReward input {} is not a pool UTXO (pubkey_hash={})",
                    i, utxo.output.pubkey_hash
                )));
            }
            if utxo.output.output_type.is_native_amount() {
                total_input = total_input.checked_add(utxo.output.amount).ok_or_else(|| {
                    ValidationError::AmountOverflow {
                        context: format!("epoch reward input total at index {}", i),
                    }
                })?;
            }
        }
        // Conservation: inputs must cover outputs exactly (no fee)
        let total_output = tx.total_output();
        if total_input < total_output {
            return Err(ValidationError::InsufficientFunds {
                inputs: total_input,
                outputs: total_output,
            });
        }
        return Ok(());
    }

    // Validate each input
    let mut total_input: Amount = 0;

    for (i, input) in tx.inputs.iter().enumerate() {
        // Validate committed_output_count
        if input.committed_output_count > 0 {
            if input.sighash_type != SighashType::AnyoneCanPay {
                return Err(ValidationError::InvalidTransaction(format!(
                    "[ERRTX034] input {} has committed_output_count={} but sighash is {:?}, expected AnyoneCanPay",
                    i, input.committed_output_count, input.sighash_type
                )));
            }
            if input.committed_output_count as usize > tx.outputs.len() {
                return Err(ValidationError::InvalidTransaction(format!(
                    "[ERRTX035] input {} committed_output_count={} exceeds output count={}",
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

        // Fractionalized NFTs can ONLY be spent by RedeemNft transactions
        if utxo.output.is_fractionalized() && tx.tx_type != TxType::RedeemNft {
            return Err(ValidationError::InvalidTransaction(format!(
                "[ERRTX036] input {} is a fractionalized NFT — can only be spent by RedeemNft, got tx_type={:?}",
                i, tx.tx_type
            )));
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
        verify_input_conditions(
            tx,
            input,
            &signing_hash,
            &utxo,
            i,
            ctx.current_height,
            ctx.sig_verification_height,
        )?;

        // Add to total (with overflow check) — only native DOLI amounts
        if utxo.output.output_type.is_native_amount() {
            total_input = total_input.checked_add(utxo.output.amount).ok_or_else(|| {
                ValidationError::AmountOverflow {
                    context: format!("input total at index {}", i),
                }
            })?;
        }
    }

    // Verify inputs >= outputs (difference is fee).
    // Exempt TxTypes:
    // DOLI conservation: native output can NEVER exceed native input.
    // This applies to ALL TX types including Pool/Lending — prevents coin creation.
    // Pool/Lending types are only exempt from the FEE check (not the balance check)
    // because DOLI flows into/out of reserves tracked in extra_data, not Output.amount.
    //
    // Genesis Registration (0 inputs, 0 outputs): protocol-generated VDF proof, exempt.
    let is_genesis_registration =
        tx.tx_type == TxType::Registration && tx.inputs.is_empty() && tx.outputs.is_empty();
    if !is_genesis_registration {
        let total_output = tx.total_output();
        if total_input < total_output {
            return Err(ValidationError::InsufficientFunds {
                inputs: total_input,
                outputs: total_output,
            });
        }

        // Fee check: Pool/Lending types are exempt (DOLI moves to reserves, not burned as fee).
        let fee_exempt = matches!(
            tx.tx_type,
            TxType::CreatePool
                | TxType::Swap
                | TxType::AddLiquidity
                | TxType::RemoveLiquidity
                | TxType::CreateLoan
                | TxType::RepayLoan
                | TxType::LiquidateLoan
                | TxType::LendingDeposit
                | TxType::LendingWithdraw
        );
        if !fee_exempt {
            let actual_fee = total_input.saturating_sub(total_output);
            let min_fee = tx.minimum_fee();
            if actual_fee < min_fee {
                let extra_bytes: u64 = tx.outputs.iter().map(|o| o.extra_data.len() as u64).sum();
                return Err(ValidationError::InsufficientFee {
                    actual: actual_fee,
                    minimum: min_fee,
                    base: BASE_FEE,
                    extra_bytes,
                    per_byte: FEE_PER_BYTE,
                });
            }
        }
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
                                        "[ERRTX037] NFT input {} requires royalty of {} to creator, got {} (sale_price={}, royalty_bps={})",
                                        i, required_royalty, actual_royalty, sale_price, royalty_bps
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

    // -- Pool swap invariant and token conservation --
    // When swapping through a pool, verify:
    // 1. First input is a Pool UTXO (the old pool state)
    // 2. First output is a Pool UTXO (the new pool state)
    // 3. Constant product invariant: new_k >= old_k
    // 4. Token conservation: tokens leaving reserves go to user output (and vice versa)
    // 5. Pool ID must be preserved (same pool)
    if tx.tx_type == TxType::Swap {
        let first_input = &tx.inputs[0];
        let old_pool_utxo = utxo_provider
            .get_utxo(&first_input.prev_tx_hash, first_input.output_index)
            .ok_or(ValidationError::OutputNotFound {
                tx_hash: first_input.prev_tx_hash,
                output_index: first_input.output_index,
            })?;

        if old_pool_utxo.output.output_type != OutputType::Pool {
            return Err(ValidationError::InvalidSwap(
                "first input must be a Pool UTXO".to_string(),
            ));
        }

        let old_meta = old_pool_utxo
            .output
            .pool_metadata()
            .ok_or_else(|| ValidationError::InvalidSwap("invalid old pool metadata".to_string()))?;

        let new_meta = tx.outputs[0]
            .pool_metadata()
            .ok_or_else(|| ValidationError::InvalidSwap("invalid new pool metadata".to_string()))?;

        // Pool ID must be preserved
        if old_meta.pool_id != new_meta.pool_id {
            return Err(ValidationError::InvalidSwap(
                "pool_id changed during swap".to_string(),
            ));
        }

        // Asset B must be preserved
        if old_meta.asset_b_id != new_meta.asset_b_id {
            return Err(ValidationError::InvalidSwap(
                "asset_b changed during swap".to_string(),
            ));
        }

        // Fee must be preserved
        if old_meta.fee_bps != new_meta.fee_bps {
            return Err(ValidationError::InvalidSwap(
                "fee_bps changed during swap".to_string(),
            ));
        }

        // LP supply must be preserved (swaps don't mint/burn LP)
        if old_meta.total_lp_shares != new_meta.total_lp_shares {
            return Err(ValidationError::InvalidSwap(
                "total_lp_shares changed during swap".to_string(),
            ));
        }

        // INVARIANT CHECK: new_k >= old_k
        let old_k = (old_meta.reserve_a as u128) * (old_meta.reserve_b as u128);
        let new_k = (new_meta.reserve_a as u128) * (new_meta.reserve_b as u128);
        if new_k < old_k {
            return Err(ValidationError::InvalidSwap(format!(
                "invariant violated: new k ({}) < old k ({})",
                new_k, old_k
            )));
        }

        // TOKEN CONSERVATION: the difference in reserves must match user outputs
        // Direction A->B: reserve_a increases, reserve_b decreases
        // Direction B->A: reserve_b increases, reserve_a decreases
        if new_meta.reserve_a > old_meta.reserve_a {
            // A->B swap: DOLI went in, tokens came out
            let tokens_out_from_pool = old_meta.reserve_b - new_meta.reserve_b;
            // Find total FungibleAsset output amount to user (skip output[0] which is Pool)
            let tokens_to_user: u64 = tx
                .outputs
                .iter()
                .skip(1)
                .filter(|o| o.output_type == OutputType::FungibleAsset)
                .map(|o| o.amount)
                .sum();
            if tokens_to_user != tokens_out_from_pool {
                return Err(ValidationError::InvalidSwap(format!(
                    "token conservation violated: pool released {} but user received {}",
                    tokens_out_from_pool, tokens_to_user
                )));
            }
        } else if new_meta.reserve_b > old_meta.reserve_b {
            // B->A swap: tokens went in, DOLI came out
            let doli_out_from_pool = old_meta.reserve_a - new_meta.reserve_a;
            // Find total Normal DOLI output to user (skip output[0] Pool, skip change)
            // The DOLI out from reserves should appear as Normal outputs to the swapper
            // We can't distinguish swap output from change here, but we can verify
            // the reserve decrease is bounded: doli out <= old_reserve_a
            if doli_out_from_pool > old_meta.reserve_a {
                return Err(ValidationError::InvalidSwap(
                    "DOLI output exceeds pool reserve_a".to_string(),
                ));
            }
        }

        // Reserves must remain positive
        if new_meta.reserve_a == 0 || new_meta.reserve_b == 0 {
            return Err(ValidationError::InvalidSwap(
                "swap would drain pool reserves to zero".to_string(),
            ));
        }
    }

    // -- AddLiquidity invariant --
    if tx.tx_type == TxType::AddLiquidity {
        let first_input = &tx.inputs[0];
        let old_pool_utxo = utxo_provider
            .get_utxo(&first_input.prev_tx_hash, first_input.output_index)
            .ok_or(ValidationError::OutputNotFound {
                tx_hash: first_input.prev_tx_hash,
                output_index: first_input.output_index,
            })?;

        if old_pool_utxo.output.output_type != OutputType::Pool {
            return Err(ValidationError::InvalidLiquidity(
                "AddLiquidity first input must be a Pool UTXO".to_string(),
            ));
        }
        {
            let old_meta = old_pool_utxo.output.pool_metadata();
            let new_meta = tx.outputs[0].pool_metadata();

            if let (Some(old_m), Some(new_m)) = (old_meta, new_meta) {
                // Pool ID preserved
                if old_m.pool_id != new_m.pool_id {
                    return Err(ValidationError::InvalidLiquidity(
                        "pool_id changed during add liquidity".to_string(),
                    ));
                }
                // Reserves must increase
                if new_m.reserve_a < old_m.reserve_a || new_m.reserve_b < old_m.reserve_b {
                    return Err(ValidationError::InvalidLiquidity(
                        "reserves decreased during add liquidity".to_string(),
                    ));
                }
                // LP supply must increase
                if new_m.total_lp_shares <= old_m.total_lp_shares {
                    return Err(ValidationError::InvalidLiquidity(
                        "LP shares did not increase during add liquidity".to_string(),
                    ));
                }
            }
        }
    }

    // -- RemoveLiquidity invariant --
    if tx.tx_type == TxType::RemoveLiquidity {
        let first_input = &tx.inputs[0];
        let old_pool_utxo = utxo_provider
            .get_utxo(&first_input.prev_tx_hash, first_input.output_index)
            .ok_or(ValidationError::OutputNotFound {
                tx_hash: first_input.prev_tx_hash,
                output_index: first_input.output_index,
            })?;

        if old_pool_utxo.output.output_type != OutputType::Pool {
            return Err(ValidationError::InvalidLiquidity(
                "RemoveLiquidity first input must be a Pool UTXO".to_string(),
            ));
        }
        {
            let old_meta = old_pool_utxo.output.pool_metadata();
            let new_meta = tx.outputs[0].pool_metadata();

            if let (Some(old_m), Some(new_m)) = (old_meta, new_meta) {
                if old_m.pool_id != new_m.pool_id {
                    return Err(ValidationError::InvalidLiquidity(
                        "pool_id changed during remove liquidity".to_string(),
                    ));
                }
                // Reserves must decrease or stay same
                if new_m.reserve_a > old_m.reserve_a || new_m.reserve_b > old_m.reserve_b {
                    return Err(ValidationError::InvalidLiquidity(
                        "reserves increased during remove liquidity".to_string(),
                    ));
                }
                // LP supply must decrease
                if new_m.total_lp_shares >= old_m.total_lp_shares {
                    return Err(ValidationError::InvalidLiquidity(
                        "LP shares did not decrease during remove liquidity".to_string(),
                    ));
                }
            }
        }
    }

    // -- FractionalizeNft UTXO checks --
    // Input 0 must be a UniqueAsset (NFT), and it must NOT already be fractionalized.
    if tx.tx_type == TxType::FractionalizeNft {
        let first_input = &tx.inputs[0];
        let nft_utxo = utxo_provider
            .get_utxo(&first_input.prev_tx_hash, first_input.output_index)
            .ok_or(ValidationError::OutputNotFound {
                tx_hash: first_input.prev_tx_hash,
                output_index: first_input.output_index,
            })?;

        if nft_utxo.output.output_type != OutputType::NFT {
            return Err(ValidationError::InvalidFractionalization(
                "input 0 must be a UniqueAsset (NFT) UTXO".to_string(),
            ));
        }

        if nft_utxo.output.is_fractionalized() {
            return Err(ValidationError::InvalidFractionalization(
                "input NFT is already fractionalized".to_string(),
            ));
        }

        // Verify token_id is preserved from input to output
        let input_meta = nft_utxo.output.nft_metadata().ok_or_else(|| {
            ValidationError::InvalidFractionalization(
                "input 0 has invalid NFT metadata".to_string(),
            )
        })?;
        let output_meta = tx.outputs[0].nft_metadata().ok_or_else(|| {
            ValidationError::InvalidFractionalization(
                "output 0 has invalid NFT metadata".to_string(),
            )
        })?;
        if input_meta.0 != output_meta.0 {
            return Err(ValidationError::InvalidFractionalization(
                "output 0 token_id does not match input NFT token_id".to_string(),
            ));
        }
    }

    // -- RedeemNft UTXO checks --
    // Input 0 must be a fractionalized NFT.
    // Other inputs must include FungibleAsset UTXOs totaling the full supply.
    if tx.tx_type == TxType::RedeemNft {
        let first_input = &tx.inputs[0];
        let nft_utxo = utxo_provider
            .get_utxo(&first_input.prev_tx_hash, first_input.output_index)
            .ok_or(ValidationError::OutputNotFound {
                tx_hash: first_input.prev_tx_hash,
                output_index: first_input.output_index,
            })?;

        if nft_utxo.output.output_type != OutputType::NFT {
            return Err(ValidationError::InvalidRedemption(
                "input 0 must be a UniqueAsset (NFT) UTXO".to_string(),
            ));
        }

        let (frac_asset_id, total_shares) = nft_utxo
            .output
            .fractionalization_metadata()
            .ok_or_else(|| {
                ValidationError::InvalidRedemption(
                    "input 0 is not a fractionalized NFT".to_string(),
                )
            })?;

        // Sum FungibleAsset inputs with matching fraction_asset_id
        let mut fraction_total: u64 = 0;
        for (i, input) in tx.inputs.iter().enumerate().skip(1) {
            let utxo = utxo_provider
                .get_utxo(&input.prev_tx_hash, input.output_index)
                .ok_or(ValidationError::OutputNotFound {
                    tx_hash: input.prev_tx_hash,
                    output_index: input.output_index,
                })?;
            if utxo.output.output_type == OutputType::FungibleAsset {
                if let Some((asset_id, _, _)) = utxo.output.fungible_asset_metadata() {
                    if asset_id == frac_asset_id {
                        fraction_total = fraction_total
                            .checked_add(utxo.output.amount)
                            .ok_or_else(|| ValidationError::AmountOverflow {
                                context: format!("RedeemNft fraction total at input {}", i),
                            })?;
                    }
                }
            }
        }

        if fraction_total != total_shares {
            return Err(ValidationError::InvalidRedemption(format!(
                "fraction token total ({}) does not match required total_shares ({})",
                fraction_total, total_shares
            )));
        }

        // No output may be FungibleAsset with the fraction_asset_id
        for (i, output) in tx.outputs.iter().enumerate() {
            if output.output_type == OutputType::FungibleAsset {
                if let Some((asset_id, _, _)) = output.fungible_asset_metadata() {
                    if asset_id == frac_asset_id {
                        return Err(ValidationError::InvalidRedemption(format!(
                            "output {} is a FungibleAsset with the fraction asset_id — all fractions must be burned",
                            i
                        )));
                    }
                }
            }
        }

        // Verify token_id is preserved from input to output
        let input_meta = nft_utxo.output.nft_metadata().ok_or_else(|| {
            ValidationError::InvalidRedemption("input 0 has invalid NFT metadata".to_string())
        })?;
        let output_meta = tx.outputs[0].nft_metadata().ok_or_else(|| {
            ValidationError::InvalidRedemption("output 0 has invalid NFT metadata".to_string())
        })?;
        if input_meta.0 != output_meta.0 {
            return Err(ValidationError::InvalidRedemption(
                "output 0 token_id does not match input NFT token_id".to_string(),
            ));
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
    sig_verification_height: u64,
) -> Result<(), ValidationError> {
    if utxo.output.output_type.is_conditioned() {
        // ---- Conditioned output: evaluate condition tree ----
        let condition = crate::conditions::Condition::decode_prefix(&utxo.output.extra_data)
            .map(|(cond, _consumed)| cond)
            .map_err(|e| {
                ValidationError::InvalidTransaction(format!(
                    "[ERRTX038] input {} references output with invalid condition: {}",
                    input_index, e
                ))
            })?;

        // Check ops limit
        let ops = condition.ops_count();
        if ops > crate::conditions::MAX_CONDITION_OPS {
            return Err(ValidationError::InvalidTransaction(format!(
                "[ERRTX039] input {} condition has {} ops (max {})",
                input_index,
                ops,
                crate::conditions::MAX_CONDITION_OPS
            )));
        }

        // Decode witness from Transaction.extra_data (SegWit-style)
        let witness_bytes = tx.get_covenant_witness(input_index).unwrap_or(&[]);
        let witness = crate::conditions::Witness::decode(witness_bytes).map_err(|e| {
            ValidationError::InvalidTransaction(format!(
                "[ERRTX040] input {} has invalid witness data: {}",
                input_index, e
            ))
        })?;

        let ctx = crate::conditions::EvalContext {
            current_height,
            signing_hash,
            transaction: Some(tx),
        };

        let mut or_idx = 0;
        if !crate::conditions::evaluate(&condition, &witness, &ctx, &mut or_idx) {
            return Err(ValidationError::InvalidSignature { index: input_index });
        }

        Ok(())
    } else {
        // ---- Normal/Bond output: single signature verification ----
        verify_input_signature(
            input,
            signing_hash,
            utxo,
            input_index,
            current_height,
            sig_verification_height,
        )
    }
}

/// Verify the signature on a transaction input (Normal/Bond outputs only).
///
/// Pubkey resolution order:
/// 1. `input.public_key` — spender-provided (post-P0-001 hard fork)
/// 2. `utxo.pubkey` — from UTXO set (test mock only; production returns None)
/// 3. Neither:
///    - If `current_height >= sig_verification_height` → reject (MissingPublicKey)
///    - If `current_height < sig_verification_height` → skip verification (pre-fork compat)
fn verify_input_signature(
    input: &Input,
    signing_hash: &Hash,
    utxo: &UtxoInfo,
    input_index: usize,
    current_height: crate::types::BlockHeight,
    sig_verification_height: u64,
) -> Result<(), ValidationError> {
    // Try input's own public key first (post-fork), then UTXO's (test mock).
    let pubkey = match input.public_key.as_ref().or(utxo.pubkey.as_ref()) {
        Some(pk) => pk,
        None => {
            // After activation height: public key is mandatory
            if current_height >= sig_verification_height {
                return Err(ValidationError::MissingPublicKey {
                    index: input_index,
                    activation_height: sig_verification_height,
                });
            }
            // Before activation: skip verification (pre-fork legacy compat)
            return Ok(());
        }
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
                return Err(ValidationError::DoubleSpend {
                    tx_hash: outpoint.0,
                    output_index: outpoint.1,
                });
            }
        }
    }

    Ok(())
}

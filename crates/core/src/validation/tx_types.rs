use crate::transaction::{
    AddBondData, ClaimBondData, ClaimData, ExitData, OutputType, SlashData, Transaction,
    WithdrawalRequestData,
};
use crypto::Hash;

use super::producer::validate_vdf;
use super::{ValidationContext, ValidationError};

/// Validate exit transaction data.
pub(super) fn validate_exit_data(tx: &Transaction) -> Result<(), ValidationError> {
    // Exit must have no inputs (just identifies producer to exit)
    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "[ERRTX041] exit transaction must have no inputs".to_string(),
        ));
    }

    // Exit must have no outputs (bond released after cooldown)
    if !tx.outputs.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "[ERRTX042] exit transaction must have no outputs".to_string(),
        ));
    }

    // Parse and validate exit data from extra_data
    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "[ERRTX043] missing exit data in extra_data".to_string(),
        ));
    }

    // Try to deserialize exit data
    let _exit_data: ExitData = bincode::deserialize(&tx.extra_data).map_err(|e| {
        ValidationError::InvalidTransaction(format!("[ERRTX044] invalid exit data: {}", e))
    })?;

    // Note: Producer state validation (is producer active, not already in cooldown, etc.)
    // is done at the node level where we have access to the producer set

    Ok(())
}

/// Validate claim reward transaction data.
///
/// Structural validation for ClaimReward transactions:
/// - Must have no inputs (rewards come from pending balance)
/// - Must have exactly one output (the claimed amount)
/// - Must have valid claim data identifying the producer
///
/// Note: The actual reward amount validation is done at the node level
/// where we have access to the producer set and their pending_rewards.
pub(super) fn validate_claim_data(tx: &Transaction) -> Result<(), ValidationError> {
    // Claim must have no inputs (rewards come from pending balance, not UTXOs)
    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidClaim(
            "claim transaction must have no inputs".to_string(),
        ));
    }

    // Claim must have exactly one output (the claimed rewards)
    if tx.outputs.len() != 1 {
        return Err(ValidationError::InvalidClaim(
            "claim transaction must have exactly one output".to_string(),
        ));
    }

    // Output must be a normal output (not a bond)
    if tx.outputs[0].output_type != OutputType::Normal {
        return Err(ValidationError::InvalidClaim(
            "claim output must be a normal output".to_string(),
        ));
    }

    // Parse and validate claim data from extra_data
    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidClaim(
            "missing claim data".to_string(),
        ));
    }

    // Try to deserialize claim data
    let _claim_data: ClaimData = bincode::deserialize(&tx.extra_data)
        .map_err(|e| ValidationError::InvalidClaim(format!("invalid claim data: {}", e)))?;

    // Note: The following validations are done at the node level:
    // - Producer exists and is registered
    // - Producer has sufficient pending_rewards for the claimed amount
    // - Signature verification (producer must sign the claim)

    Ok(())
}

/// Validate claim bond transaction data.
///
/// Structural validation for ClaimBond transactions:
/// - Must have no inputs (bond comes from protocol)
/// - Must have exactly one output (the returned bond)
/// - Must have valid claim bond data identifying the producer
///
/// Note: The actual bond amount and exit terms validation is done at the node level
/// where we have access to the producer set and their unbonding status.
pub(super) fn validate_claim_bond_data(tx: &Transaction) -> Result<(), ValidationError> {
    // Claim bond must have no inputs
    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidBondClaim(
            "claim bond transaction must have no inputs".to_string(),
        ));
    }

    // Claim bond must have exactly one output
    if tx.outputs.len() != 1 {
        return Err(ValidationError::InvalidBondClaim(
            "claim bond transaction must have exactly one output".to_string(),
        ));
    }

    // Output must be a normal output (not a bond)
    if tx.outputs[0].output_type != OutputType::Normal {
        return Err(ValidationError::InvalidBondClaim(
            "claim bond output must be a normal output".to_string(),
        ));
    }

    // Parse and validate claim bond data from extra_data
    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidBondClaim(
            "missing claim bond data".to_string(),
        ));
    }

    // Try to deserialize claim bond data
    let _claim_bond_data: ClaimBondData = bincode::deserialize(&tx.extra_data).map_err(|e| {
        ValidationError::InvalidBondClaim(format!("invalid claim bond data: {}", e))
    })?;

    // Note: The following validations are done at the node level:
    // - Producer exists and has status Exited (unbonding complete)
    // - Bond amount matches exit terms (full or early exit penalty applied)
    // - Signature verification (producer must sign the claim)

    Ok(())
}

/// Validate slash producer transaction data.
///
/// Structural validation for SlashProducer transactions:
/// - Must have no inputs
/// - Must have no outputs (bond is burned, not redistributed)
/// - Must have valid slash data with cryptographically verifiable evidence
///
/// Evidence verification is now done here with VDF verification to prevent
/// fabricated evidence attacks. The VDF proves the producer actually created
/// both blocks (since the VDF input includes the producer's public key).
pub(super) fn validate_slash_data(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    // Slash must have no inputs
    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidSlash(
            "slash transaction must have no inputs".to_string(),
        ));
    }

    // Slash must have no outputs (bond is burned)
    if !tx.outputs.is_empty() {
        return Err(ValidationError::InvalidSlash(
            "slash transaction must have no outputs".to_string(),
        ));
    }

    // Parse and validate slash data from extra_data
    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidSlash(
            "missing slash data".to_string(),
        ));
    }

    // Try to deserialize slash data
    let slash_data: SlashData = bincode::deserialize(&tx.extra_data)
        .map_err(|e| ValidationError::InvalidSlash(format!("invalid slash data: {}", e)))?;

    // Validate evidence structure with full cryptographic verification
    // Only double production is slashable - this is the only unambiguously intentional offense
    match &slash_data.evidence {
        crate::transaction::SlashingEvidence::DoubleProduction {
            block_header_1,
            block_header_2,
        } => {
            // 1. Both headers must have the same producer
            if block_header_1.producer != block_header_2.producer {
                return Err(ValidationError::InvalidSlash(
                    "double production evidence must have same producer in both headers"
                        .to_string(),
                ));
            }

            // 2. Both headers must have the same slot
            if block_header_1.slot != block_header_2.slot {
                return Err(ValidationError::InvalidSlash(
                    "double production evidence must have same slot in both headers".to_string(),
                ));
            }

            // 3. Block hashes must be different (otherwise it's not double production)
            if block_header_1.hash() == block_header_2.hash() {
                return Err(ValidationError::InvalidSlash(
                    "double production evidence must have different block hashes".to_string(),
                ));
            }

            // 4. Producer in evidence must match slash_data.producer_pubkey
            if block_header_1.producer != slash_data.producer_pubkey {
                return Err(ValidationError::InvalidSlash(
                    "evidence producer does not match slash target".to_string(),
                ));
            }

            // 5. Verify VDF for header 1 (proves the producer actually created it)
            validate_vdf(block_header_1, ctx.network).map_err(|_| {
                ValidationError::InvalidSlash(
                    "invalid VDF proof in first block header - evidence may be fabricated"
                        .to_string(),
                )
            })?;

            // 6. Verify VDF for header 2 (proves the producer actually created it)
            validate_vdf(block_header_2, ctx.network).map_err(|_| {
                ValidationError::InvalidSlash(
                    "invalid VDF proof in second block header - evidence may be fabricated"
                        .to_string(),
                )
            })?;
        }
        crate::transaction::SlashingEvidence::PriceAttestationEquivocation {
            attestation_1,
            attestation_2,
        } => {
            validate_price_attestation_equivocation_evidence(
                ctx,
                &slash_data.producer_pubkey,
                attestation_1,
                attestation_2,
            )?;
        }
    }

    // Note: The following validations are done at the node level:
    // - Producer exists and is active (not already slashed or exited)
    // - Reporter signature is valid

    Ok(())
}

/// Same as `validate_slash_data` but skips VDF verification for evidence headers.
///
/// Used when slash VDFs have already been verified in parallel (block.rs).
/// All structural checks (same producer, same slot, different hashes) still run.
pub(super) fn validate_slash_data_skip_vdf(
    tx: &Transaction,
    _ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidSlash(
            "slash transaction must have no inputs".to_string(),
        ));
    }
    if !tx.outputs.is_empty() {
        return Err(ValidationError::InvalidSlash(
            "slash transaction must have no outputs".to_string(),
        ));
    }
    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidSlash(
            "missing slash data".to_string(),
        ));
    }
    let slash_data: SlashData = bincode::deserialize(&tx.extra_data)
        .map_err(|e| ValidationError::InvalidSlash(format!("invalid slash data: {}", e)))?;

    match &slash_data.evidence {
        crate::transaction::SlashingEvidence::DoubleProduction {
            block_header_1,
            block_header_2,
        } => {
            if block_header_1.producer != block_header_2.producer {
                return Err(ValidationError::InvalidSlash(
                    "double production evidence must have same producer in both headers"
                        .to_string(),
                ));
            }
            if block_header_1.slot != block_header_2.slot {
                return Err(ValidationError::InvalidSlash(
                    "double production evidence must have same slot in both headers".to_string(),
                ));
            }
            if block_header_1.hash() == block_header_2.hash() {
                return Err(ValidationError::InvalidSlash(
                    "double production evidence must have different block hashes".to_string(),
                ));
            }
            if block_header_1.producer != slash_data.producer_pubkey {
                return Err(ValidationError::InvalidSlash(
                    "evidence producer does not match slash target".to_string(),
                ));
            }
            // VDF verification skipped — already verified in parallel pre-pass
        }
        crate::transaction::SlashingEvidence::PriceAttestationEquivocation {
            attestation_1,
            attestation_2,
        } => {
            // No VDF in PriceAttestation evidence — signatures are
            // the proof. We always verify them; there is no "skip"
            // shortcut equivalent for this variant.
            validate_price_attestation_equivocation_evidence(
                _ctx,
                &slash_data.producer_pubkey,
                attestation_1,
                attestation_2,
            )?;
        }
    }
    Ok(())
}

/// Validate evidence for `SlashingEvidence::PriceAttestationEquivocation`
/// — Phase 2.1 Oracle M7. Spec §1.4.
///
/// Returns `Ok(())` only when ALL of the following hold:
///   - `attestation_1.signer_pubkey == attestation_2.signer_pubkey`
///   - `slash_data.producer_pubkey == that signer_pubkey`
///   - `attestation_1.epoch_number == attestation_2.epoch_number`
///   - `attestation_1.pair_id == attestation_2.pair_id`
///   - `attestation_1.price_cents != attestation_2.price_cents`
///     (otherwise the "two" attestations are identical — not
///     equivocation, just a duplicate that M4 rule 5 already rejects)
///   - both signatures verify against the shared `signer_pubkey`
///     over each attestation's `signing_message()`
///   - `ctx.current_height >= ctx.oracle_activation_height`
///     (defense-in-depth — pre-activation no valid PriceAttestation
///     can exist, but explicitly gating avoids any pathological
///     fabricated-evidence path).
fn validate_price_attestation_equivocation_evidence(
    ctx: &ValidationContext,
    producer_pubkey: &crypto::PublicKey,
    attestation_1: &crate::transaction::PriceAttestationData,
    attestation_2: &crate::transaction::PriceAttestationData,
) -> Result<(), ValidationError> {
    // Activation gate — equivocation evidence is unreachable
    // pre-activation because M4 rejects PriceAttestation submission
    // entirely. Re-check here for defense-in-depth.
    if ctx.current_height < ctx.oracle_activation_height {
        return Err(ValidationError::InvalidSlash(format!(
            "price attestation equivocation evidence rejected: oracle not activated \
             (current_height={} activation_height={})",
            ctx.current_height, ctx.oracle_activation_height
        )));
    }

    // 1. Same signer in both attestations.
    if attestation_1.signer_pubkey != attestation_2.signer_pubkey {
        return Err(ValidationError::InvalidSlash(
            "price attestation equivocation: attestations have different signers".to_string(),
        ));
    }

    // 2. Slash target matches the equivocating signer.
    if attestation_1.signer_pubkey != *producer_pubkey {
        return Err(ValidationError::InvalidSlash(
            "price attestation equivocation: evidence signer does not match slash target"
                .to_string(),
        ));
    }

    // 3. Same epoch — equivocation is per-epoch only.
    if attestation_1.epoch_number != attestation_2.epoch_number {
        return Err(ValidationError::InvalidSlash(
            "price attestation equivocation: attestations are from different epochs".to_string(),
        ));
    }

    // 4. Same pair_id.
    if attestation_1.pair_id != attestation_2.pair_id {
        return Err(ValidationError::InvalidSlash(
            "price attestation equivocation: attestations are for different pairs".to_string(),
        ));
    }

    // 5. Different prices — otherwise it's just a duplicate, not
    //    equivocation. (M4 rule 5 rejects duplicates at validation,
    //    so this would not be a slashable offense anyway.)
    if attestation_1.price_cents == attestation_2.price_cents {
        return Err(ValidationError::InvalidSlash(
            "price attestation equivocation: attestations have identical price_cents".to_string(),
        ));
    }

    // 6. Both signatures must verify against the shared signer.
    crypto::signature::verify_hash(
        &attestation_1.signing_message(),
        &attestation_1.signature,
        &attestation_1.signer_pubkey,
    )
    .map_err(|_| {
        ValidationError::InvalidSlash(
            "price attestation equivocation: first attestation signature invalid".to_string(),
        )
    })?;
    crypto::signature::verify_hash(
        &attestation_2.signing_message(),
        &attestation_2.signature,
        &attestation_2.signer_pubkey,
    )
    .map_err(|_| {
        ValidationError::InvalidSlash(
            "price attestation equivocation: second attestation signature invalid".to_string(),
        )
    })?;

    Ok(())
}

// ==================== Bond Transaction Validation ====================

/// Validate add bond transaction data.
///
/// Structural validation for AddBond transactions:
/// - Must have inputs (paying for bonds)
/// - Must have no outputs (funds become bonds)
/// - Must have valid add bond data with bond count
/// - Input amount must equal bond_count * BOND_UNIT
///
/// Note: Producer existence and max bonds check is done at node level.
pub(super) fn validate_add_bond_data(tx: &Transaction) -> Result<(), ValidationError> {
    // Must have inputs (funds to become bonds)
    if tx.inputs.is_empty() {
        return Err(ValidationError::InvalidAddBond(
            "add bond transaction must have inputs".to_string(),
        ));
    }

    // Must have at least one Bond output (lock/unlock model)
    let bond_outputs: Vec<_> = tx
        .outputs
        .iter()
        .filter(|o| o.output_type == OutputType::Bond)
        .collect();

    if bond_outputs.is_empty() {
        return Err(ValidationError::InvalidAddBond(
            "add bond must have a Bond output".to_string(),
        ));
    }

    // Non-bond outputs must be Normal (for change)
    for output in &tx.outputs {
        if output.output_type != OutputType::Normal && output.output_type != OutputType::Bond {
            return Err(ValidationError::InvalidAddBond(
                "add bond outputs must be Bond or Normal type".to_string(),
            ));
        }
    }

    // Parse add bond data from extra_data
    let bond_data = AddBondData::from_bytes(&tx.extra_data)
        .ok_or_else(|| ValidationError::InvalidAddBond("invalid add bond data".to_string()))?;

    // Bond count must be positive
    if bond_data.bond_count == 0 {
        return Err(ValidationError::InvalidAddBond(
            "bond count must be positive".to_string(),
        ));
    }

    // Note: These validations are done at node level:
    // - Producer is registered
    // - New total doesn't exceed MAX_BONDS_PER_PRODUCER → check_addbond_cap()
    //   (INC-I-080, height-gated at addbond_cap_enforcement_activation_height)
    // - Bond output amount matches bond_count * BOND_UNIT

    Ok(())
}

/// INC-I-080: enforce the per-producer bond cap on AddBond, height-gated.
///
/// Resolves the long-standing comment-only TODO above in
/// `validate_add_bond_data` ("New total doesn't exceed
/// MAX_BONDS_PER_PRODUCER … done at node level"). The node never actually
/// performed it — `ProducerInfo::add_bonds` silently clipped the excess at
/// epoch flush and discarded the orphaned Bond UTXOs (value lost, no signal).
///
/// * **Pre-activation** (`height < activation_height`): returns `Ok(())`
///   unconditionally. The historical clip-at-epoch-flush behavior is
///   preserved so replaying historical blocks stays bit-identical (no
///   consensus change before the activation height).
/// * **Post-activation** (`height >= activation_height`): the AddBond is
///   rejected with [`ValidationError::AddBondCapExceeded`] when
///   `current + pending + requested > MAX_BONDS_PER_PRODUCER`, where
///   `pending` sums the bond counts over the producer's in-flight queued
///   AddBonds. All arithmetic saturates (no overflow panic on adversarial
///   inputs).
///
/// Three-question gate (INC-I-075): Q1=YES (AddBond is user-submittable),
/// Q2=YES (producer-action triggered), Q3=NO (post-AH rejects txs that were
/// previously accepted-then-clipped) ⇒ activation height REQUIRED. The gate
/// makes every node on the same params+height reach the same verdict, so the
/// rejection is consensus-safe under a rolling deploy.
///
/// Unlike the INC-I-078 DelegateBond cap (which *skips* the over-cap tx,
/// leaving the block valid), AddBond must *reject* the carrying block: the
/// Bond output UTXOs are real, and a skip would still orphan them. Rejection
/// at block-apply guarantees "no orphan Bonds" post-activation.
pub fn check_addbond_cap(
    current: u32,
    pending: u32,
    requested: u32,
    height: u64,
    activation_height: u64,
) -> Result<(), ValidationError> {
    // Pre-activation gate dominates — clip path preserved (replay safety).
    if height < activation_height {
        return Ok(());
    }
    let total = current.saturating_add(pending).saturating_add(requested);
    if total > crate::MAX_BONDS_PER_PRODUCER {
        return Err(ValidationError::AddBondCapExceeded {
            current,
            pending,
            requested,
            max: crate::MAX_BONDS_PER_PRODUCER,
        });
    }
    Ok(())
}

/// Validate withdrawal request transaction data.
///
/// Structural validation for RequestWithdrawal transactions:
/// - Must have inputs (Bond UTXOs being consumed -- lock/unlock model)
/// - Must have exactly 1 normal output (payout to destination)
/// - Must have valid withdrawal request data
/// - Output amount must be > 0
/// - Output pubkey_hash must match destination in withdrawal data
///
/// Note: Bond UTXO ownership, producer bond holdings, and FIFO calculation done at node level.
pub(super) fn validate_withdrawal_request_data(tx: &Transaction) -> Result<(), ValidationError> {
    // Must have inputs (Bond UTXOs being unlocked)
    if tx.inputs.is_empty() {
        return Err(ValidationError::InvalidWithdrawalRequest(
            "withdrawal request must have Bond UTXO inputs".to_string(),
        ));
    }

    // Must have exactly 1 output (payout)
    if tx.outputs.len() != 1 {
        return Err(ValidationError::InvalidWithdrawalRequest(
            "withdrawal request must have exactly 1 output".to_string(),
        ));
    }

    let output = &tx.outputs[0];
    if output.output_type != OutputType::Normal {
        return Err(ValidationError::InvalidWithdrawalRequest(
            "withdrawal output must be Normal type".to_string(),
        ));
    }
    if output.amount == 0 {
        return Err(ValidationError::InvalidWithdrawalRequest(
            "withdrawal output amount must be positive".to_string(),
        ));
    }

    // Parse withdrawal data from extra_data
    let withdrawal_data = WithdrawalRequestData::from_bytes(&tx.extra_data).ok_or_else(|| {
        ValidationError::InvalidWithdrawalRequest("invalid withdrawal request data".to_string())
    })?;

    // Bond count must be positive
    if withdrawal_data.bond_count == 0 {
        return Err(ValidationError::InvalidWithdrawalRequest(
            "withdrawal bond count must be positive".to_string(),
        ));
    }

    // Destination must not be zero hash
    if withdrawal_data.destination == Hash::ZERO {
        return Err(ValidationError::InvalidWithdrawalRequest(
            "destination cannot be zero hash".to_string(),
        ));
    }

    // Output destination must match withdrawal data destination
    if output.pubkey_hash != withdrawal_data.destination {
        return Err(ValidationError::InvalidWithdrawalRequest(
            "output destination must match withdrawal data destination".to_string(),
        ));
    }

    // Note: These validations are done at node level:
    // - Producer is registered
    // - Producer has enough bonds to withdraw
    // - Output amount <= FIFO net calculation

    Ok(())
}

/// Validate a MintAsset transaction.
///
/// Rules:
/// - Must have at least one input (issuer proves ownership of the asset's genesis UTXO)
/// - All inputs must be FungibleAsset outputs with the same asset_id
/// - All outputs must be FungibleAsset outputs with the same asset_id
/// - sum(output amounts) >= sum(input amounts) -- the difference is the newly minted supply
/// - The first input must be from the original issuer (creator of the genesis asset UTXO)
pub(super) fn validate_mint_asset(tx: &Transaction) -> Result<(), ValidationError> {
    if tx.inputs.is_empty() {
        return Err(ValidationError::InvalidMintAsset(
            "MintAsset requires at least one input".to_string(),
        ));
    }
    if tx.outputs.is_empty() {
        return Err(ValidationError::InvalidMintAsset(
            "MintAsset requires at least one output".to_string(),
        ));
    }
    // All outputs must be FungibleAsset type
    for (i, output) in tx.outputs.iter().enumerate() {
        if output.output_type != OutputType::FungibleAsset {
            return Err(ValidationError::InvalidMintAsset(format!(
                "output {} must be FungibleAsset type",
                i
            )));
        }
    }
    Ok(())
}

/// Validate a BurnAsset transaction.
///
/// Rules:
/// - Must have at least one input (tokens being burned)
/// - All inputs consumed must be FungibleAsset outputs with the same asset_id
/// - sum(output amounts) < sum(input amounts) -- the difference is provably destroyed
/// - Outputs (if any) must be FungibleAsset with the same asset_id (change back to holder)
/// - No new minting: each output amount must be individually <= input total
pub(super) fn validate_burn_asset(tx: &Transaction) -> Result<(), ValidationError> {
    if tx.inputs.is_empty() {
        return Err(ValidationError::InvalidBurnAsset(
            "BurnAsset requires at least one input".to_string(),
        ));
    }
    // Outputs (if any) must all be FungibleAsset type
    for (i, output) in tx.outputs.iter().enumerate() {
        if output.output_type != OutputType::FungibleAsset {
            return Err(ValidationError::InvalidBurnAsset(format!(
                "output {} must be FungibleAsset type",
                i
            )));
        }
    }
    // Note: the actual supply accounting (inputs > outputs) is enforced by the UTXO
    // balance check in apply_block -- sum(outputs) must be <= sum(inputs) for all tx types.
    Ok(())
}

/// Validate epoch reward transaction data
///
/// Basic validation of EpochReward transactions:
/// - Must have no inputs (minted)
/// - Must have exactly one output
/// - Output must be Normal type
/// - Must have valid EpochRewardData
///
/// NOTE: This is the working automatic push-based reward system.
/// Rewards are distributed automatically at epoch boundaries by the block producer.
pub(super) fn validate_epoch_reward_data(tx: &Transaction) -> Result<(), ValidationError> {
    // Pre-activation: must have no inputs (pool consumed by side-effect).
    // Post-activation: inputs are explicit sorted pool outpoints.
    // Structural validation allows both formats — the height-aware check
    // is in validate_block_economics (Full mode) and validation/utxo.rs.

    // Must have at least one output
    if tx.outputs.is_empty() {
        return Err(ValidationError::InvalidEpochReward(
            "epoch reward must have at least one output".to_string(),
        ));
    }

    // All outputs must be Normal type
    for output in &tx.outputs {
        if output.output_type != OutputType::Normal {
            return Err(ValidationError::InvalidEpochReward(
                "epoch reward outputs must be Normal type".to_string(),
            ));
        }
    }

    Ok(())
}

// ==================== Maintainer Transaction Validation ====================

/// Validate maintainer change transaction data (AddMaintainer/RemoveMaintainer).
///
/// Structural validation for maintainer change transactions:
/// - Must have no inputs (state-only operation)
/// - Must have no outputs (no funds transferred)
/// - Must have valid MaintainerChangeData in extra_data
///
/// Note: Signature verification and maintainer set state checks are done
/// at the node level where we have access to the current maintainer set.
pub(super) fn validate_maintainer_change_data(tx: &Transaction) -> Result<(), ValidationError> {
    use crate::maintainer::MaintainerChangeData;

    // Maintainer changes must have no inputs (state-only operation)
    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidMaintainerChange(
            "maintainer change transaction must have no inputs".to_string(),
        ));
    }

    // Maintainer changes must have no outputs (no funds transferred)
    if !tx.outputs.is_empty() {
        return Err(ValidationError::InvalidMaintainerChange(
            "maintainer change transaction must have no outputs".to_string(),
        ));
    }

    // Must have valid MaintainerChangeData in extra_data
    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidMaintainerChange(
            "missing maintainer change data".to_string(),
        ));
    }

    // Try to deserialize maintainer change data
    let _change_data = MaintainerChangeData::from_bytes(&tx.extra_data).ok_or_else(|| {
        ValidationError::InvalidMaintainerChange(
            "invalid maintainer change data format".to_string(),
        )
    })?;

    // Note: The following validations are done at the node level:
    // - Current maintainer set exists and is valid
    // - Sufficient signatures from current maintainers (threshold check)
    // - Target is not already a maintainer (for Add) or is a maintainer (for Remove)
    // - Adding won't exceed MAX_MAINTAINERS
    // - Removing won't go below MIN_MAINTAINERS

    Ok(())
}

/// Validate DelegateBond transaction data.
///
/// Structural validation:
/// - Must have no inputs (state-only operation)
/// - Must have no outputs
/// - Must have valid DelegateBondData in extra_data
/// - Bond count must be positive
///
/// Note: Producer existence, active status, self-delegation, and
/// sufficient bonds are checked at the node level.
pub(super) fn validate_delegate_bond_data(tx: &Transaction) -> Result<(), ValidationError> {
    use crate::transaction::DelegateBondData;

    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidDelegation(
            "delegate bond must have no inputs".to_string(),
        ));
    }

    if !tx.outputs.is_empty() {
        return Err(ValidationError::InvalidDelegation(
            "delegate bond must have no outputs".to_string(),
        ));
    }

    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidDelegation(
            "missing delegate bond data".to_string(),
        ));
    }

    let data = DelegateBondData::from_bytes(&tx.extra_data).ok_or_else(|| {
        ValidationError::InvalidDelegation("invalid delegate bond data format".to_string())
    })?;

    if data.bond_count == 0 {
        return Err(ValidationError::InvalidDelegation(
            "bond count must be positive".to_string(),
        ));
    }

    Ok(())
}

/// Validate the structural (data-only) shape of a `PriceAttestation`
/// (TxType=16) tx — Phase 2.1 Oracle M4.
///
/// Spec: `specs/oracle-structural-anchored-economics.md` §1.1.
///
/// Checks performed here (no `ValidationContext` access — pure data):
///   - `tx.inputs` is empty (the tx mutates no UTXO state)
///   - `tx.outputs` is empty (OraclePrice UTXO is created at epoch
///     boundary by M6, not by the user tx)
///   - `tx.extra_data` decodes as a 144-byte `PriceAttestationData`
///   - Rule 6 (signature verifies over `signing_message()`) — verified
///     here because the signing pubkey lives inside the payload itself.
///
/// Rules 1 (height gate), 2 (active producer), and 3 (epoch match)
/// require `ValidationContext` and live in the `PriceAttestation` arm of
/// `validate_transaction`. Rules 4 (pool liquidity) and 5 (at-most-one
/// per epoch+pair) require UTXO + block-scope context and land at M6
/// (`apply_block` epoch-boundary aggregator).
pub(super) fn validate_price_attestation_data(
    tx: &crate::transaction::Transaction,
) -> Result<(), ValidationError> {
    use crate::transaction::PriceAttestationData;

    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "price attestation must have no inputs".to_string(),
        ));
    }

    if !tx.outputs.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "price attestation must have no outputs".to_string(),
        ));
    }

    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "missing price attestation data".to_string(),
        ));
    }

    let data = PriceAttestationData::from_bytes(&tx.extra_data).ok_or_else(|| {
        ValidationError::InvalidTransaction("invalid price attestation data format".to_string())
    })?;

    // Rule 6 (spec §1.1): signature verifies against signer_pubkey over
    // signing_message(). Done here because the signing pubkey is part of
    // the data payload — no `ctx` needed.
    crypto::signature::verify_hash(
        &data.signing_message(),
        &data.signature,
        &data.signer_pubkey,
    )
    .map_err(|e| {
        ValidationError::InvalidTransaction(format!(
            "price attestation signature verification failed: {e:?}"
        ))
    })?;

    Ok(())
}

/// Validate RevokeDelegation transaction data.
///
/// Structural validation:
/// - Must have no inputs (state-only operation)
/// - Must have no outputs
/// - Must have valid RevokeDelegationData in extra_data
///
/// Note: Active delegation existence and unbonding delay are
/// checked at the node level.
pub(super) fn validate_revoke_delegation_data(tx: &Transaction) -> Result<(), ValidationError> {
    use crate::transaction::RevokeDelegationData;

    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidDelegation(
            "revoke delegation must have no inputs".to_string(),
        ));
    }

    if !tx.outputs.is_empty() {
        return Err(ValidationError::InvalidDelegation(
            "revoke delegation must have no outputs".to_string(),
        ));
    }

    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidDelegation(
            "missing revoke delegation data".to_string(),
        ));
    }

    let _data = RevokeDelegationData::from_bytes(&tx.extra_data).ok_or_else(|| {
        ValidationError::InvalidDelegation("invalid revoke delegation data format".to_string())
    })?;

    Ok(())
}

// ==================== Protocol Activation Validation ====================

/// Validate protocol activation transaction data.
///
/// Structural validation:
/// - Must have no inputs (state-only operation)
/// - Must have no outputs (no funds transferred)
/// - Must have valid ProtocolActivationData in extra_data
/// - Protocol version must be > 0
/// - Activation epoch must be > 0
/// - At least 1 signature present (full 3/5 check done at node level)
///
/// Note: Maintainer set verification (3/5 multisig), version > current,
/// and epoch > current are checked at the node level where state is available.
pub(super) fn validate_protocol_activation_data(tx: &Transaction) -> Result<(), ValidationError> {
    use crate::maintainer::ProtocolActivationData;

    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidProtocolActivation(
            "protocol activation must have no inputs".to_string(),
        ));
    }

    if !tx.outputs.is_empty() {
        return Err(ValidationError::InvalidProtocolActivation(
            "protocol activation must have no outputs".to_string(),
        ));
    }

    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidProtocolActivation(
            "missing protocol activation data".to_string(),
        ));
    }

    let data = ProtocolActivationData::from_bytes(&tx.extra_data).ok_or_else(|| {
        ValidationError::InvalidProtocolActivation(
            "invalid protocol activation data format".to_string(),
        )
    })?;

    if data.protocol_version == 0 {
        return Err(ValidationError::InvalidProtocolActivation(
            "protocol version must be > 0".to_string(),
        ));
    }

    if data.activation_epoch == 0 {
        return Err(ValidationError::InvalidProtocolActivation(
            "activation epoch must be > 0".to_string(),
        ));
    }

    if data.signatures.is_empty() {
        return Err(ValidationError::InvalidProtocolActivation(
            "at least one maintainer signature required".to_string(),
        ));
    }

    Ok(())
}

/// Validate the structure of a `ZKSettle` transaction.
///
/// Structural checks (no UTXO lookups, no proof verification):
///   1. Height gate — reject if below `ZK_SETTLE_ACTIVATION_HEIGHT`.
///   2. Exactly 1 input (the previous `ZKRollup` UTXO reference).
///   3. At least 1 output, where the **first** output is a `ZKRollup` output.
///   4. The first output's `extra_data` deserializes as `ZkRollupData`.
///   5. The rollup identity fields are within documented caps.
///   6. The optional proof blob in `tx.extra_data` does not exceed `MAX_ZK_PROOF_SIZE`.
///
/// The actual ZK proof is verified in `validate_transaction_with_utxos()`
/// where the input UTXO (and therefore the previous `ZkRollupData`) is available.
pub(super) fn validate_zk_settle_structure(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    use crate::transaction::{ZkRollupData, MAX_VERIFYING_KEY_SIZE, MAX_ZK_PROOF_SIZE};

    // 1. Activation gate. Until a ProtocolActivation tx lowers
    //    ZK_SETTLE_ACTIVATION_HEIGHT, this short-circuits every call.
    if ctx.current_height < crate::validation::zk::ZK_SETTLE_ACTIVATION_HEIGHT {
        return Err(ValidationError::InvalidTransaction(format!(
            "[ERRTX-ZK001] ZKSettle not yet activated (height {} < activation {})",
            ctx.current_height,
            crate::validation::zk::ZK_SETTLE_ACTIVATION_HEIGHT
        )));
    }

    // 2. Exactly one input — the previous ZKRollup UTXO reference.
    if tx.inputs.len() != 1 {
        return Err(ValidationError::InvalidTransaction(format!(
            "[ERRTX-ZK002] ZKSettle must have exactly 1 input, got {}",
            tx.inputs.len()
        )));
    }

    // 3. At least one output, first must be ZKRollup.
    if tx.outputs.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "[ERRTX-ZK003] ZKSettle must have at least 1 output".to_string(),
        ));
    }
    if tx.outputs[0].output_type != OutputType::ZKRollup {
        return Err(ValidationError::InvalidTransaction(format!(
            "[ERRTX-ZK004] ZKSettle first output must be ZKRollup, got {:?}",
            tx.outputs[0].output_type
        )));
    }

    // 4. The first output's extra_data must decode as ZkRollupData.
    let out_data = ZkRollupData::from_bytes(&tx.outputs[0].extra_data).ok_or_else(|| {
        ValidationError::InvalidTransaction(
            "[ERRTX-ZK005] ZKSettle first output extra_data is not a valid ZkRollupData"
                .to_string(),
        )
    })?;

    // 5. Rollup identity caps (defense in depth — the deserializer also enforces).
    if out_data.verifying_key.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "[ERRTX-ZK006] ZkRollupData verifying_key must not be empty".to_string(),
        ));
    }
    if out_data.verifying_key.len() > MAX_VERIFYING_KEY_SIZE {
        return Err(ValidationError::InvalidTransaction(format!(
            "[ERRTX-ZK007] ZkRollupData verifying_key size {} exceeds max {}",
            out_data.verifying_key.len(),
            MAX_VERIFYING_KEY_SIZE
        )));
    }
    if out_data.proof_system_id == crate::validation::proof_system::UNASSIGNED {
        return Err(ValidationError::InvalidTransaction(
            "[ERRTX-ZK008] ZkRollupData proof_system_id is UNASSIGNED (0)".to_string(),
        ));
    }

    // 6. Proof blob size cap. The proof lives in tx.extra_data (see spec §4.2).
    if tx.extra_data.len() > MAX_ZK_PROOF_SIZE {
        return Err(ValidationError::InvalidTransaction(format!(
            "[ERRTX-ZK009] ZKSettle proof size {} exceeds max {}",
            tx.extra_data.len(),
            MAX_ZK_PROOF_SIZE
        )));
    }

    Ok(())
}

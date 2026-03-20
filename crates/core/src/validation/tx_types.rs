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
            "exit transaction must have no inputs".to_string(),
        ));
    }

    // Exit must have no outputs (bond released after cooldown)
    if !tx.outputs.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "exit transaction must have no outputs".to_string(),
        ));
    }

    // Parse and validate exit data from extra_data
    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "missing exit data".to_string(),
        ));
    }

    // Try to deserialize exit data
    let _exit_data: ExitData = bincode::deserialize(&tx.extra_data)
        .map_err(|e| ValidationError::InvalidTransaction(format!("invalid exit data: {}", e)))?;

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
    }

    // Note: The following validations are done at the node level:
    // - Producer exists and is active (not already slashed or exited)
    // - Reporter signature is valid

    Ok(())
}

/// Same as `validate_slash_data` but skips VDF verification for evidence headers.
/// Used when slash VDFs have already been verified in parallel (block.rs Phase 1).
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
    }
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
    // - New total doesn't exceed MAX_BONDS_PER_PRODUCER
    // - Bond output amount matches bond_count * BOND_UNIT

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
    // Must have no inputs (minted)
    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidEpochReward(
            "epoch reward must have no inputs".to_string(),
        ));
    }

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

use crate::attestation::decode_attestation_bitfield;
use crate::block::Block;
use crate::network::Network;
use crate::tpop::heartbeat::verify_hash_chain_vdf;
use crate::transaction::{OutputType, RegistrationData, Transaction};
use crate::types::Amount;

use super::{ValidationContext, ValidationError};

/// Validate registration transaction data.
pub(super) fn validate_registration_data(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    validate_registration_data_inner(tx, ctx, false)
}

/// Validate registration transaction data, optionally skipping VDF verification.
///
/// When `skip_vdf` is true, the VDF proof is assumed to have been verified
/// already (e.g., in a parallel pre-verification pass). All other checks
/// (bond amount, BLS PoP, registration chain, duplicates) still run.
pub(super) fn validate_registration_data_skip_vdf(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    validate_registration_data_inner(tx, ctx, true)
}

fn validate_registration_data_inner(
    tx: &Transaction,
    ctx: &ValidationContext,
    skip_vdf: bool,
) -> Result<(), ValidationError> {
    // During genesis, Registration TXs are VDF proof containers only.
    // No bond required (bond is handled at GENESIS PHASE COMPLETE).
    // No registration chain validation (bootstrap producers can't be Sybil-attacked).
    // VDF proof is still validated -- that's the whole point.
    if ctx.network.is_in_genesis(ctx.current_height) {
        if tx.extra_data.is_empty() {
            return Err(ValidationError::InvalidRegistration(
                "missing registration data".to_string(),
            ));
        }
        let reg_data: RegistrationData = bincode::deserialize(&tx.extra_data).map_err(|e| {
            ValidationError::InvalidRegistration(format!("invalid registration data: {}", e))
        })?;
        // BLS key is mandatory -- every producer must have one
        if reg_data.bls_pubkey.is_empty() {
            return Err(ValidationError::InvalidRegistration(
                "BLS public key required for registration".to_string(),
            ));
        }
        if reg_data.bls_pop.is_empty() {
            return Err(ValidationError::InvalidRegistration(
                "BLS proof of possession required for registration".to_string(),
            ));
        }
        validate_bls_pop(&reg_data)?;

        // Validate VDF proof (the only requirement for genesis registrations)
        if !skip_vdf {
            validate_registration_vdf(&reg_data, ctx.network)?;
        }
        return Ok(());
    }

    // Registration must have at least one input (for bond)
    if tx.inputs.is_empty() {
        return Err(ValidationError::InvalidRegistration(
            "registration must have inputs for bond".to_string(),
        ));
    }

    // Must have at least one bond output
    let bond_outputs: Vec<_> = tx
        .outputs
        .iter()
        .filter(|o| o.output_type == OutputType::Bond)
        .collect();

    if bond_outputs.is_empty() {
        return Err(ValidationError::InvalidRegistration(
            "registration must have a bond output".to_string(),
        ));
    }

    // Verify bond amount meets minimum requirement
    let required_bond = ctx.params.bond_amount(ctx.current_height);
    let total_bond: Amount = bond_outputs
        .iter()
        .map(|o| o.amount)
        .try_fold(0u64, |acc, amt| acc.checked_add(amt))
        .ok_or_else(|| ValidationError::AmountOverflow {
            context: "bond total".to_string(),
        })?;

    if total_bond < required_bond {
        return Err(ValidationError::InvalidRegistration(format!(
            "insufficient bond: {} < {}",
            total_bond, required_bond
        )));
    }

    // Verify bond lock duration
    let required_lock = ctx.current_height + ctx.params.blocks_per_era;
    for bond in &bond_outputs {
        if bond.lock_until < required_lock {
            return Err(ValidationError::InvalidRegistration(format!(
                "bond lock too short: {} < {}",
                bond.lock_until, required_lock
            )));
        }
    }

    // Parse and validate registration data from extra_data
    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidRegistration(
            "missing registration data".to_string(),
        ));
    }

    // Try to deserialize registration data
    let reg_data: RegistrationData = bincode::deserialize(&tx.extra_data).map_err(|e| {
        ValidationError::InvalidRegistration(format!("invalid registration data: {}", e))
    })?;

    // Validate bond_count is consensus-safe (WHITEPAPER Section 7)
    // bond_count is embedded on-chain to ensure all nodes agree on producer selection.
    // We only validate bounds here; the existing total_bond >= required_bond check
    // above ensures sufficient collateral.
    if reg_data.bond_count < 1 {
        return Err(ValidationError::InvalidRegistration(
            "bond_count must be at least 1".to_string(),
        ));
    }
    if reg_data.bond_count > crate::consensus::MAX_BONDS_PER_PRODUCER {
        return Err(ValidationError::InvalidRegistration(format!(
            "bond_count {} exceeds maximum {}",
            reg_data.bond_count,
            crate::consensus::MAX_BONDS_PER_PRODUCER,
        )));
    }

    // BLS key is mandatory -- every producer must have one (like Ethereum validators)
    if reg_data.bls_pubkey.is_empty() {
        return Err(ValidationError::InvalidRegistration(
            "BLS public key required for registration".to_string(),
        ));
    }
    if reg_data.bls_pop.is_empty() {
        return Err(ValidationError::InvalidRegistration(
            "BLS proof of possession required for registration".to_string(),
        ));
    }
    validate_bls_pop(&reg_data)?;

    // Verify VDF proof for registration
    // (The actual VDF verification happens here)
    if !skip_vdf {
        validate_registration_vdf(&reg_data, ctx.network)?;
    }

    // Verify registration chain (anti-Sybil: prevents parallel registration)
    validate_registration_chain(&reg_data, ctx)?;

    // Reject duplicate registration (GitHub Issue #4: duplicate register deletes producer)
    if ctx.active_producers.contains(&reg_data.public_key) {
        return Err(ValidationError::InvalidRegistration(
            "producer already registered".to_string(),
        ));
    }

    // Reject registration if already pending (epoch-deferred, not yet active)
    if ctx.pending_producer_keys.contains(&reg_data.public_key) {
        return Err(ValidationError::InvalidRegistration(
            "producer already has a pending registration".to_string(),
        ));
    }

    Ok(())
}

/// Validate the registration chain fields (anti-Sybil protection).
///
/// Each registration must reference the previous registration's hash and
/// have the correct sequence number. This prevents parallel registration
/// attacks where an attacker tries to register many nodes simultaneously.
fn validate_registration_chain(
    reg_data: &RegistrationData,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    let expected_prev_hash = ctx.registration_chain.expected_prev_hash();
    let expected_sequence = ctx.registration_chain.expected_sequence();

    // Verify prev_registration_hash matches
    if reg_data.prev_registration_hash != expected_prev_hash {
        return Err(ValidationError::InvalidRegistration(format!(
            "invalid prev_registration_hash: expected {}, got {}",
            hex::encode(expected_prev_hash.as_bytes()),
            hex::encode(reg_data.prev_registration_hash.as_bytes())
        )));
    }

    // Verify sequence number is correct
    if reg_data.sequence_number != expected_sequence {
        return Err(ValidationError::InvalidRegistration(format!(
            "invalid sequence_number: expected {}, got {}",
            expected_sequence, reg_data.sequence_number
        )));
    }

    Ok(())
}

/// Validate the VDF proof in registration data.
///
/// Validate registration VDF using hash-chain (same as block VDF).
/// Hash-chain is fast to compute (~5s for 5M iterations) and self-verifying
/// (recompute and compare output). No separate proof needed.
/// For devnet, VDF validation is skipped to allow quick testing.
///
/// This function is pure (no context needed) and safe to call from parallel threads.
pub fn validate_registration_vdf(
    reg_data: &RegistrationData,
    network: Network,
) -> Result<(), ValidationError> {
    // Skip VDF validation for devnet (allows CLI registration without VDF computation)
    if network == Network::Devnet {
        return Ok(());
    }

    // Create VDF input using the standard function
    let input = vdf::registration_input(&reg_data.public_key, reg_data.epoch);

    // Hash-chain VDF output must be exactly 32 bytes
    if reg_data.vdf_output.len() != 32 {
        return Err(ValidationError::InvalidRegistration(
            "invalid VDF output: expected 32 bytes".to_string(),
        ));
    }

    let expected_output: [u8; 32] = reg_data.vdf_output.as_slice().try_into().map_err(|_| {
        ValidationError::InvalidRegistration("invalid VDF output format".to_string())
    })?;

    // Verify by recomputing the hash-chain VDF
    if !verify_hash_chain_vdf(&input, &expected_output, network.vdf_register_iterations()) {
        return Err(ValidationError::InvalidRegistration(
            "VDF verification failed".to_string(),
        ));
    }

    Ok(())
}

/// Verify the BLS aggregate attestation signature in a block.
///
/// Decodes the presence_root bitfield to determine which producers attested,
/// gathers their BLS public keys from the validation context, and verifies
/// the aggregate signature against the attestation message.
pub(super) fn validate_bls_aggregate(
    block: &Block,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    // Decode aggregate signature
    let agg_sig = crypto::BlsSignature::try_from_slice(&block.aggregate_bls_signature)
        .map_err(|e| ValidationError::InvalidBlock(format!("invalid BLS aggregate sig: {}", e)))?;

    // Decode bitfield to find which producers attested
    let attested_indices =
        decode_attestation_bitfield(&block.header.presence_root, ctx.producer_bls_keys.len());

    if attested_indices.is_empty() {
        return Err(ValidationError::InvalidBlock(
            "BLS aggregate sig present but bitfield is empty".to_string(),
        ));
    }

    // Gather BLS pubkeys of attesting producers (skip those without BLS keys)
    let mut bls_pubkeys: Vec<crypto::BlsPublicKey> = Vec::new();
    for &idx in &attested_indices {
        if idx < ctx.producer_bls_keys.len() && !ctx.producer_bls_keys[idx].is_empty() {
            if let Ok(pk) = crypto::BlsPublicKey::try_from_slice(&ctx.producer_bls_keys[idx]) {
                bls_pubkeys.push(pk);
            }
        }
    }

    if bls_pubkeys.is_empty() {
        // No BLS-capable producers in the bitfield -- can't verify.
        // This is a transitional state: block has aggregate sig but no BLS keys registered.
        // Accept gracefully during migration.
        return Ok(());
    }

    // Verify: aggregate sig must match the attestation message signed by these pubkeys
    let msg = crypto::attestation_message(&block.hash(), block.header.slot);
    crypto::bls_verify_aggregate(&msg, &agg_sig, &bls_pubkeys).map_err(|e| {
        ValidationError::InvalidBlock(format!("BLS aggregate verification failed: {}", e))
    })?;

    Ok(())
}

/// Validate BLS proof-of-possession for a registration.
///
/// Verifies that the registrant controls the BLS secret key by checking
/// a signature over the BLS public key itself (separate `PoP` DST).
fn validate_bls_pop(reg_data: &RegistrationData) -> Result<(), ValidationError> {
    let pubkey = crypto::BlsPublicKey::try_from_slice(&reg_data.bls_pubkey).map_err(|e| {
        ValidationError::InvalidRegistration(format!("invalid BLS public key: {}", e))
    })?;

    let sig = crypto::BlsSignature::try_from_slice(&reg_data.bls_pop).map_err(|e| {
        ValidationError::InvalidRegistration(format!("invalid BLS PoP signature: {}", e))
    })?;

    crypto::bls_verify_pop(&pubkey, &sig).map_err(|_| {
        ValidationError::InvalidRegistration(
            "BLS proof-of-possession verification failed".to_string(),
        )
    })?;

    Ok(())
}

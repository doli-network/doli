//! SlashProducer structural validation and DoubleProduction evidence checks.
//!
//! From the INC-I-236 activation height the producer's block signature is the
//! authorship proof for each evidence header; the VDF only proves sequential
//! work over public inputs, so anyone can compute it for any producer.

use bincode::Options;

use crate::block::BlockHeader;
use crate::transaction::{SlashData, SlashingEvidence, Transaction};

use super::block::verify_block_signature;
use super::producer::validate_vdf;
use super::tx_types::validate_price_attestation_equivocation_evidence;
use super::{ValidationContext, ValidationError};

/// Encoded `SlashData` cap from the INC-I-236 AH. Two real headers (<= ~450 B
/// each with at most 3 missed producers) + pubkey + signature are ~1 KiB.
const MAX_SLASH_DATA_BYTES: u64 = 64 * 1024;

fn invalid(msg: &str) -> ValidationError {
    ValidationError::InvalidSlash(msg.to_string())
}

fn signature_rule_active(ctx: &ValidationContext) -> bool {
    ctx.current_height >= ctx.inc_i_236_block_signature_activation_height
}

/// Decode `SlashData` with `bincode::deserialize` semantics; from the AH the
/// input and the decoder are bounded by `MAX_SLASH_DATA_BYTES`.
pub(super) fn decode_slash_data(
    bytes: &[u8],
    ctx: &ValidationContext,
) -> Result<SlashData, ValidationError> {
    let opts = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .allow_trailing_bytes();
    let decoded = if signature_rule_active(ctx) {
        if bytes.len() as u64 > MAX_SLASH_DATA_BYTES {
            return Err(ValidationError::InvalidSlash(format!(
                "slash data is {} bytes (max {})",
                bytes.len(),
                MAX_SLASH_DATA_BYTES
            )));
        }
        opts.with_limit(MAX_SLASH_DATA_BYTES).deserialize(bytes)
    } else {
        opts.deserialize(bytes)
    };
    decoded.map_err(|e| ValidationError::InvalidSlash(format!("invalid slash data: {}", e)))
}

/// Validate a SlashProducer transaction, including evidence VDFs.
pub(super) fn validate_slash_data(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    validate_slash(tx, ctx, true)
}

/// Same as `validate_slash_data` but skips evidence VDFs (verified in the
/// parallel pre-pass in block.rs). Signature and structural checks still run.
pub(super) fn validate_slash_data_skip_vdf(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    validate_slash(tx, ctx, false)
}

fn validate_slash(
    tx: &Transaction,
    ctx: &ValidationContext,
    verify_vdf: bool,
) -> Result<(), ValidationError> {
    if !tx.inputs.is_empty() {
        return Err(invalid("slash transaction must have no inputs"));
    }
    if !tx.outputs.is_empty() {
        return Err(invalid("slash transaction must have no outputs"));
    }
    if tx.extra_data.is_empty() {
        return Err(invalid("missing slash data"));
    }
    let slash_data = decode_slash_data(&tx.extra_data, ctx)?;

    match &slash_data.evidence {
        SlashingEvidence::DoubleProduction {
            block_header_1,
            block_header_2,
        } => validate_double_production(
            &slash_data.producer_pubkey,
            block_header_1,
            block_header_2,
            ctx,
            verify_vdf,
        ),
        SlashingEvidence::PriceAttestationEquivocation {
            attestation_1,
            attestation_2,
        } => validate_price_attestation_equivocation_evidence(
            ctx,
            &slash_data.producer_pubkey,
            attestation_1,
            attestation_2,
        ),
    }
}

fn validate_double_production(
    accused: &crypto::PublicKey,
    h1: &BlockHeader,
    h2: &BlockHeader,
    ctx: &ValidationContext,
    verify_vdf: bool,
) -> Result<(), ValidationError> {
    if h1.producer != h2.producer {
        return Err(invalid(
            "double production evidence must have same producer in both headers",
        ));
    }
    if h1.slot != h2.slot {
        return Err(invalid(
            "double production evidence must have same slot in both headers",
        ));
    }
    if h1.hash() == h2.hash() {
        return Err(invalid(
            "double production evidence must have different block hashes",
        ));
    }
    if h1.producer != *accused {
        return Err(invalid("evidence producer does not match slash target"));
    }

    if signature_rule_active(ctx) {
        for (n, h) in [(1, h1), (2, h2)] {
            if h.genesis_hash != ctx.params.genesis_hash {
                return Err(ValidationError::InvalidSlash(format!(
                    "evidence header {n} is from a different chain (genesis mismatch)"
                )));
            }
            verify_block_signature(h).map_err(|e| {
                ValidationError::InvalidSlash(format!(
                    "evidence header {n} lacks a valid producer signature: {e}"
                ))
            })?;
        }
    }

    if verify_vdf {
        validate_vdf(h1, ctx.network).map_err(|_| {
            invalid("invalid VDF proof in first block header - evidence may be fabricated")
        })?;
        validate_vdf(h2, ctx.network).map_err(|_| {
            invalid("invalid VDF proof in second block header - evidence may be fabricated")
        })?;
    }
    Ok(())
}

//! SlashProducer structural validation and DoubleProduction evidence checks.
//!
//! From the INC-I-236 activation height the producer's block signature is the
//! authorship proof for each evidence header; the VDF only proves sequential
//! work over public inputs, so anyone can compute it for any producer.

use bincode::Options;
use crypto::{Hash, PublicKey, Signature};

use crate::block::BlockHeader;
use crate::network::Network;
use crate::transaction::{SlashData, SlashingEvidence, Transaction, TxType};

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

pub(super) fn slash_rule_active(ctx: &ValidationContext) -> bool {
    signature_rule_active(ctx) && ctx.current_height >= ctx.inc_i_255_slashing_activation_height
}

/// Fee-floor exemption: a SlashProducer whose evidence passes the shared check
/// while the slash rule is active. Evidence VDFs are left to the caller's check.
pub fn slash_fee_exempt(tx: &Transaction, ctx: &ValidationContext) -> bool {
    tx.tx_type == TxType::SlashProducer
        && slash_rule_active(ctx)
        && validate_slash(tx, ctx, false).is_ok()
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
    validate_slash(tx, ctx, true).map(|_| ())
}

/// Same as `validate_slash_data` but skips evidence VDFs (verified in the
/// parallel pre-pass in block.rs). Signature and structural checks still run.
pub(super) fn validate_slash_data_skip_vdf(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    validate_slash(tx, ctx, false).map(|_| ())
}

pub(super) fn validate_slash(
    tx: &Transaction,
    ctx: &ValidationContext,
    verify_vdf: bool,
) -> Result<SlashData, ValidationError> {
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
    if slash_rule_active(ctx) {
        check_canonical(tx, &slash_data)?;
    }

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
        )?,
        SlashingEvidence::PriceAttestationEquivocation {
            attestation_1,
            attestation_2,
        } => validate_price_attestation_equivocation_evidence(
            ctx,
            &slash_data.producer_pubkey,
            attestation_1,
            attestation_2,
        )?,
    }
    Ok(slash_data)
}

fn check_canonical(tx: &Transaction, data: &SlashData) -> Result<(), ValidationError> {
    if !matches!(data.evidence, SlashingEvidence::DoubleProduction { .. }) {
        return Err(invalid("slash evidence must be DoubleProduction"));
    }
    if bincode::serialize(data).ok().as_deref() != Some(tx.extra_data.as_slice()) {
        return Err(invalid("non-canonical slash data encoding"));
    }
    if data.reporter_signature != Signature::default() {
        return Err(invalid("reporter signature must be the default (all zero)"));
    }
    Ok(())
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
    let (hash_1, hash_2) = (h1.hash(), h2.hash());
    if hash_1 == hash_2 {
        return Err(invalid(
            "double production evidence must have different block hashes",
        ));
    }
    if h1.producer != *accused {
        return Err(invalid("evidence producer does not match slash target"));
    }
    if slash_rule_active(ctx) && hash_1.as_bytes() > hash_2.as_bytes() {
        return Err(invalid(
            "double production evidence header order must be ascending by hash",
        ));
    }

    if signature_rule_active(ctx) {
        for (n, h) in [(1, h1), (2, h2)] {
            check_header_origin(n, h, &ctx.params.genesis_hash)?;
        }
    }

    if verify_vdf {
        check_header_vdf(h1, ctx.network, VDF_FIRST)?;
        check_header_vdf(h2, ctx.network, VDF_SECOND)?;
    }
    Ok(())
}

const VDF_FIRST: &str = "invalid VDF proof in first block header - evidence may be fabricated";
const VDF_SECOND: &str = "invalid VDF proof in second block header - evidence may be fabricated";

fn check_header_origin(n: u8, h: &BlockHeader, genesis_hash: &Hash) -> Result<(), ValidationError> {
    if h.genesis_hash != *genesis_hash {
        return Err(ValidationError::InvalidSlash(format!(
            "evidence header {n} is from a different chain (genesis mismatch)"
        )));
    }
    verify_block_signature(h).map_err(|e| {
        ValidationError::InvalidSlash(format!(
            "evidence header {n} lacks a valid producer signature: {e}"
        ))
    })
}

fn check_header_vdf(h: &BlockHeader, network: Network, msg: &str) -> Result<(), ValidationError> {
    validate_vdf(h, network).map_err(|_| invalid(msg))
}

/// The per-header DoubleProduction evidence checks (genesis, producer
/// signature, VDF), in the order `validate_double_production` applies them.
pub fn evidence_header_admissible(
    header: &BlockHeader,
    genesis_hash: &Hash,
    network: Network,
) -> Result<(), ValidationError> {
    check_header_origin(1, header, genesis_hash)?;
    check_header_vdf(header, network, VDF_FIRST)
}

/// `true` iff the canonical SlashData for this pair encodes within
/// `MAX_SLASH_DATA_BYTES`.
pub fn double_production_evidence_fits(
    producer: &PublicKey,
    h1: &BlockHeader,
    h2: &BlockHeader,
) -> bool {
    let data = SlashData::double_production(*producer, h1.clone(), h2.clone());
    bincode::serialized_size(&data).is_ok_and(|n| n <= MAX_SLASH_DATA_BYTES)
}

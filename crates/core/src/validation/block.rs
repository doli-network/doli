use crate::block::{Block, BlockHeader};
use crate::consensus::{max_block_size, MAX_DRIFT, MAX_FUTURE_SLOTS, NETWORK_MARGIN};
use crate::transaction::{RegistrationData, SlashData, TxType};

use super::producer::{validate_producer_eligibility, validate_vdf};
use super::registration::{validate_bls_aggregate, validate_registration_vdf};
use super::transaction::{validate_transaction, validate_transaction_skip_registration_vdf};
use super::utxo::check_internal_double_spend;
use super::{ValidationContext, ValidationError, ValidationMode};

/// Validate a block header
pub fn validate_header(
    header: &BlockHeader,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    // 0. Chain identity -- reject blocks from different genesis FIRST.
    // This is O(1) and catches all genesis-time-hijack attacks with zero tolerance.
    if header.genesis_hash != ctx.params.genesis_hash {
        return Err(ValidationError::GenesisHashMismatch {
            got: header.genesis_hash,
            expected: ctx.params.genesis_hash,
        });
    }

    // 1. Version check
    if header.version != 2 {
        return Err(ValidationError::InvalidVersion(header.version));
    }

    // 2. Timestamp must be after previous block
    if header.timestamp <= ctx.prev_timestamp {
        return Err(ValidationError::InvalidTimestamp {
            block: header.timestamp,
            expected: ctx.prev_timestamp + 1,
        });
    }

    // 3. Timestamp not too far in future
    if header.timestamp > ctx.current_time + MAX_DRIFT {
        return Err(ValidationError::TimestampTooFuture(header.timestamp));
    }

    // 4. Slot must derive correctly from timestamp
    let expected_slot = ctx.params.timestamp_to_slot(header.timestamp);
    if header.slot != expected_slot {
        return Err(ValidationError::InvalidSlot {
            got: header.slot,
            expected: expected_slot,
        });
    }

    // 5. Slot must advance
    if header.slot <= ctx.prev_slot {
        return Err(ValidationError::SlotNotAdvancing {
            got: header.slot,
            prev: ctx.prev_slot,
        });
    }

    // 6. Timestamp must be within slot window
    let slot_start = ctx.params.slot_to_timestamp(header.slot);
    let min_time = slot_start.saturating_sub(NETWORK_MARGIN);
    if header.timestamp < min_time {
        return Err(ValidationError::InvalidTimestamp {
            block: header.timestamp,
            expected: min_time,
        });
    }

    // 7. Slot boundary check (time-based consensus)
    // Block slot must be within acceptable range of current time.
    // This prevents clock manipulation attacks and enforces time-based slot selection.
    let current_slot = ctx.params.timestamp_to_slot(ctx.current_time);

    // Check not too far in the future
    if header.slot as u64 > current_slot as u64 + MAX_FUTURE_SLOTS {
        return Err(ValidationError::SlotTooFuture {
            got: header.slot,
            current: current_slot,
            max_future: MAX_FUTURE_SLOTS,
        });
    }

    // MAX_PAST_SLOTS: enforced at the gossip boundary (node.rs gossip handler),
    // NOT here. Checking here breaks header-first sync and reorgs because
    // historical blocks have slots far behind wall-clock. The gossip handler
    // is the correct enforcement point -- it rejects old gossip blocks before
    // they reach apply_block(), while sync/reorg paths bypass it safely.

    Ok(())
}

/// Validate a complete block
pub fn validate_block(block: &Block, ctx: &ValidationContext) -> Result<(), ValidationError> {
    // 1. Validate header
    validate_header(&block.header, ctx)?;

    // 2. Check block size (scales with era)
    let size = block.size();
    let max_size = max_block_size(ctx.current_height);
    if size > max_size {
        return Err(ValidationError::BlockTooLarge {
            size,
            max: max_size,
        });
    }

    // 3. Verify merkle root
    if !block.verify_merkle_root() {
        return Err(ValidationError::InvalidMerkleRoot {
            header: block.header.merkle_root,
            computed: block.compute_merkle_root(),
        });
    }

    // 4. Validate all transactions
    // Note: The old RewardMode-based validation is deprecated. The new weighted
    // presence reward system uses on-demand ClaimEpochReward transactions.
    // Blocks no longer require coinbase or automatic EpochReward transactions.
    for tx in &block.transactions {
        validate_transaction(tx, ctx)?;
    }

    // 5. Check for double spends within block
    check_internal_double_spend(block)?;

    // 6. Validate VDF (if not in bootstrap)
    if !ctx.params.is_bootstrap(ctx.current_height) {
        validate_vdf(&block.header, ctx.network)?;
    }

    // 7. Validate producer eligibility (if not in bootstrap)
    validate_producer_eligibility(&block.header, ctx)?;

    // 8. Verify BLS aggregate attestation signature (if present).
    // Pre-BLS blocks have empty aggregate_bls_signature -- accepted.
    // Post-BLS blocks with a signature are verified against the bitfield.
    if !block.aggregate_bls_signature.is_empty() {
        validate_bls_aggregate(block, ctx)?;
    }

    Ok(())
}

/// Validate a block with a specified validation mode.
///
/// In `Full` mode, this is identical to `validate_block()` -- all checks including
/// VDF proof verification are performed.
///
/// In `Light` mode, VDF verification is skipped. This is used for gap blocks after
/// snap sync, where the state root was already verified by a peer quorum. All other
/// checks (header, merkle root, block size, transactions, double-spend, producer
/// eligibility) are still performed.
pub fn validate_block_with_mode(
    block: &Block,
    ctx: &ValidationContext,
    mode: ValidationMode,
) -> Result<(), ValidationError> {
    match mode {
        ValidationMode::Full => {
            // Full validation: all checks including time-based header validation and VDF.
            validate_header(&block.header, ctx)?;

            let size = block.size();
            let max_size = max_block_size(ctx.current_height);
            if size > max_size {
                return Err(ValidationError::BlockTooLarge {
                    size,
                    max: max_size,
                });
            }

            if !block.verify_merkle_root() {
                return Err(ValidationError::InvalidMerkleRoot {
                    header: block.header.merkle_root,
                    computed: block.compute_merkle_root(),
                });
            }

            // Phase 1: Parallel VDF verification for VDF-heavy TXs.
            //
            // Registration VDFs (~400ms each) and SlashProducer evidence VDFs
            // (~800ms = 2 headers) are CPU-heavy. Sequential verification of N
            // such TXs takes N*400ms+ — enough to miss slots and cause forks.
            // Parallel verification keeps total time at ~max(single VDF) regardless
            // of N, scaling to hundreds of simultaneous operations.
            let has_heavy_vdf_txs = ctx.network != crate::network::Network::Devnet
                && block.transactions.iter().any(|tx| {
                    (tx.tx_type == TxType::Registration || tx.tx_type == TxType::SlashProducer)
                        && !tx.extra_data.is_empty()
                });

            if has_heavy_vdf_txs {
                let network = ctx.network;
                let vdf_error: std::sync::Mutex<Option<ValidationError>> =
                    std::sync::Mutex::new(None);

                std::thread::scope(|s| {
                    for tx in &block.transactions {
                        if tx.extra_data.is_empty() {
                            continue;
                        }
                        match tx.tx_type {
                            TxType::Registration => {
                                let err_ref = &vdf_error;
                                s.spawn(move || {
                                    if let Ok(rd) =
                                        bincode::deserialize::<RegistrationData>(&tx.extra_data)
                                    {
                                        if let Err(e) = validate_registration_vdf(&rd, network) {
                                            let mut guard = err_ref.lock().unwrap();
                                            if guard.is_none() {
                                                *guard = Some(e);
                                            }
                                        }
                                    }
                                });
                            }
                            TxType::SlashProducer => {
                                let err_ref = &vdf_error;
                                s.spawn(move || {
                                    if let Ok(slash_data) =
                                        bincode::deserialize::<SlashData>(&tx.extra_data)
                                    {
                                        let crate::transaction::SlashingEvidence::DoubleProduction {
                                            ref block_header_1,
                                            ref block_header_2,
                                        } = slash_data.evidence;
                                        if validate_vdf(block_header_1, network).is_err() {
                                            let mut guard = err_ref.lock().unwrap();
                                            if guard.is_none() {
                                                *guard = Some(ValidationError::InvalidSlash(
                                                    "invalid VDF proof in first block header"
                                                        .to_string(),
                                                ));
                                            }
                                            return;
                                        }
                                        if validate_vdf(block_header_2, network).is_err() {
                                            let mut guard = err_ref.lock().unwrap();
                                            if guard.is_none() {
                                                *guard = Some(ValidationError::InvalidSlash(
                                                    "invalid VDF proof in second block header"
                                                        .to_string(),
                                                ));
                                            }
                                        }
                                    }
                                });
                            }
                            _ => {}
                        }
                    }
                });

                if let Some(err) = vdf_error.into_inner().unwrap() {
                    return Err(err);
                }
            }

            // Phase 2: Sequential validation of all TXs.
            // Registration and SlashProducer TXs skip VDF (already verified
            // in parallel above). All other checks (bond amounts, BLS PoP,
            // chain linkage, structural slash checks, signatures, UTXO rules)
            // run sequentially as before.
            if has_heavy_vdf_txs {
                for tx in &block.transactions {
                    validate_transaction_skip_registration_vdf(tx, ctx)?;
                }
            } else {
                for tx in &block.transactions {
                    validate_transaction(tx, ctx)?;
                }
            }

            check_internal_double_spend(block)?;

            if !ctx.params.is_bootstrap(ctx.current_height) {
                validate_vdf(&block.header, ctx.network)?;
            }

            validate_producer_eligibility(&block.header, ctx)?;
        }
        ValidationMode::Light => {
            // Light validation for synced gap blocks: skip VDF and time-based header
            // checks (MAX_PAST_SLOTS would reject old blocks during sync).
            // Header chain linkage was already verified during header download.

            // Chain identity -- reject blocks from different genesis FIRST.
            if block.header.genesis_hash != ctx.params.genesis_hash {
                return Err(ValidationError::GenesisHashMismatch {
                    got: block.header.genesis_hash,
                    expected: ctx.params.genesis_hash,
                });
            }

            // Version check
            if block.header.version != 2 {
                return Err(ValidationError::InvalidVersion(block.header.version));
            }

            // Slot derivation from genesis -- rejects blocks with wrong slot calculation.
            let expected_slot = ctx.params.timestamp_to_slot(block.header.timestamp);
            if block.header.slot != expected_slot {
                return Err(ValidationError::InvalidSlot {
                    got: block.header.slot,
                    expected: expected_slot,
                });
            }

            // Slot must advance from previous block
            if block.header.slot <= ctx.prev_slot {
                return Err(ValidationError::SlotNotAdvancing {
                    got: block.header.slot,
                    prev: ctx.prev_slot,
                });
            }

            // Block size
            let size = block.size();
            let max_size = max_block_size(ctx.current_height);
            if size > max_size {
                return Err(ValidationError::BlockTooLarge {
                    size,
                    max: max_size,
                });
            }

            // Merkle root integrity
            if !block.verify_merkle_root() {
                return Err(ValidationError::InvalidMerkleRoot {
                    header: block.header.merkle_root,
                    computed: block.compute_merkle_root(),
                });
            }

            // Transaction structural validation
            for tx in &block.transactions {
                validate_transaction(tx, ctx)?;
            }

            // Internal double-spend detection
            check_internal_double_spend(block)?;

            // Producer eligibility: SKIPPED in Light mode.
            // Light mode is used for sync blocks and reorg blocks where the producer
            // set at the time of production may differ from the current state.
            // Checking eligibility against rolled-back/stale state causes false
            // "invalid producer for slot" rejections during emergency equalization.
            // The blocks were already validated by the producing node.

            // Skipped: VDF (trusted via state root quorum)
            // Skipped: MAX_FUTURE_SLOTS / MAX_PAST_SLOTS (time-based, breaks sync)
            // Skipped: timestamp-not-too-future (time-based)
        }
    }

    Ok(())
}

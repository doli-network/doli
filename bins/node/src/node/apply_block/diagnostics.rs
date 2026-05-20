//! Diagnostic emit helpers for apply_block — fire-and-forget event recording.

use crypto::Hash;
use doli_core::validation::ValidationMode;
use doli_core::Block;
use storage::diagnostic_ledger::types::BlockProvenance;

/// Emit a BlockRejected diagnostic event (fire-and-forget).
pub(super) fn emit_block_rejected(
    emitter: &dyn storage::diagnostic_ledger::emitter::DiagnosticEmitter,
    block: &Block,
    block_hash: Hash,
    height: u64,
    mode: ValidationMode,
    provenance: &Option<BlockProvenance>,
    reason: &str,
) {
    let _ = emitter.record(storage::diagnostic_ledger::types::DiagnosticEvent {
        event_id: ulid::Ulid::new().to_string(),
        kind: storage::diagnostic_ledger::types::EventKind::BlockRejected,
        timestamp_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        height: Some(height),
        correlation_key: None,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload: storage::diagnostic_ledger::types::EventPayload::BlockRejected {
            slot: block.header.slot,
            block_hash: block_hash.to_hex(),
            producer_pubkey: hex::encode(block.header.producer.as_bytes()),
            from_peer_id: provenance.as_ref().and_then(|p| p.from_peer_id.clone()),
            rejection_reason: reason.to_string(),
            mode: format!("{:?}", mode),
        },
    });
}

/// Emit a BlockApplied diagnostic event (fire-and-forget).
pub(super) fn emit_block_applied(
    emitter: &dyn storage::diagnostic_ledger::emitter::DiagnosticEmitter,
    block: &Block,
    block_hash: Hash,
    height: u64,
    mode: ValidationMode,
    provenance: &Option<BlockProvenance>,
    validation_duration_ms: u64,
) {
    let applied_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let _ = emitter.record(storage::diagnostic_ledger::types::DiagnosticEvent {
        event_id: ulid::Ulid::new().to_string(),
        kind: storage::diagnostic_ledger::types::EventKind::BlockApplied,
        timestamp_ms: applied_at_ms,
        height: Some(height),
        correlation_key: None,
        caused_by_event_id: None,
        is_cascade_origin: false,
        payload: storage::diagnostic_ledger::types::EventPayload::BlockApplied {
            slot: block.header.slot,
            block_hash: block_hash.to_hex(),
            producer_pubkey: hex::encode(block.header.producer.as_bytes()),
            from_peer_id: provenance.as_ref().and_then(|p| p.from_peer_id.clone()),
            received_at_ms: provenance
                .as_ref()
                .map(|p| p.received_at_ms)
                .filter(|&v| v > 0),
            applied_at_ms,
            validation_duration_ms,
            mode: format!("{:?}", mode),
            tx_count: block.transactions.len() as u32,
        },
    });
}

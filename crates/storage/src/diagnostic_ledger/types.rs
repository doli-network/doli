//! Diagnostic event types for fork-observability instrumentation.
//!
//! All types implement `Clone + Debug + Serialize + Deserialize + PartialEq`
//! for bincode round-trip and test assertions. The on-disk format is:
//!
//! ```text
//! [0x01 format_marker][u16 LE schema_version][bincode payload]
//! ```
//!
//! Composite key layout (25 bytes):
//! ```text
//! [event_kind u8][height u64 BE][ulid 16 bytes]
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Format marker for bincode-encoded events.
pub const FORMAT_MARKER_BINCODE: u8 = 0x01;

/// Current schema version for the diagnostic event format.
pub const CURRENT_SCHEMA_VERSION: u16 = 1;

// ---------------------------------------------------------------------------
// EventKind — discriminant enum for event classification and key prefix
// ---------------------------------------------------------------------------

/// Discriminant for diagnostic event types.
///
/// Each variant maps to a unique `u8` used as the first byte of the composite
/// RocksDB key, enabling efficient prefix scans by event category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum EventKind {
    /// A block was successfully validated and applied to the chain state.
    BlockApplied = 1,
    /// A block failed validation and was rejected.
    BlockRejected = 2,
    /// A fork block was received (height already occupied or reorg candidate).
    ForkBlockReceived = 3,
    /// A rollback operation started.
    RollbackStarted = 4,
    /// A rollback operation completed.
    RollbackCompleted = 5,
    /// A chain reorganization was executed.
    ReorgExecuted = 6,
    /// The recovery classifier was invoked.
    RecoveryClassifyCall = 7,
    /// A snap-sync attempt was initiated.
    SnapSyncAttempted = 8,
    /// A snap-sync completed successfully.
    SnapSyncCompleted = 9,
    /// A snap-sync attempt failed.
    SnapSyncFailed = 10,
    /// A chain break (parent-hash mismatch) was detected during header sync.
    ChainBreakDetected = 11,
    /// Periodic heartbeat from the diagnostic writer task.
    WriterHeartbeat = 12,
}

impl EventKind {
    /// Return the `u8` discriminant for use as a RocksDB key prefix.
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

// ---------------------------------------------------------------------------
// EventPayload — per-kind structured data
// ---------------------------------------------------------------------------

/// Kind-specific payload carried by each diagnostic event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EventPayload {
    /// Payload for `EventKind::BlockApplied`.
    BlockApplied {
        slot: u32,
        block_hash: String,
        producer_pubkey: String,
        from_peer_id: Option<String>,
        received_at_ms: Option<u64>,
        applied_at_ms: u64,
        validation_duration_ms: u64,
        mode: String,
        tx_count: u32,
    },
    /// Payload for `EventKind::BlockRejected`.
    BlockRejected {
        slot: u32,
        block_hash: String,
        producer_pubkey: String,
        from_peer_id: Option<String>,
        rejection_reason: String,
        mode: String,
    },
    /// Payload for `EventKind::ForkBlockReceived`.
    ForkBlockReceived {
        block_hash: String,
        block_slot: u32,
        block_height_estimate: Option<u64>,
        producer_pubkey: String,
        from_peer_id: String,
        classification: String,
        fork_kind: Option<String>,
        local_tip_hash: String,
        local_tip_height: u64,
    },
    /// Payload for `EventKind::RollbackStarted`.
    RollbackStarted {
        from_height: u64,
        to_height: u64,
        trigger: String,
        cumulative_depth: u32,
    },
    /// Payload for `EventKind::RollbackCompleted`.
    RollbackCompleted {
        from_height: u64,
        to_height: u64,
        duration_ms: u64,
        success: bool,
    },
    /// Payload for `EventKind::ReorgExecuted`.
    ReorgExecuted {
        old_tip_hash: String,
        new_tip_hash: String,
        rollback_depth: u32,
        applied_count: u32,
        weight_delta: i64,
        trigger_block_hash: String,
        trigger_from_peer_id: Option<String>,
    },
    /// Payload for `EventKind::RecoveryClassifyCall`.
    RecoveryClassifyCall {
        local_height: u64,
        network_tip_height: u64,
        peer_count: u32,
        last_applied_secs: u64,
        shallow_rollback_count: u32,
        snap_attempts: u32,
        last_rollback_local_height: Option<u64>,
        in_grace_period: bool,
        last_finality_height: Option<u64>,
        action_returned: Option<String>,
        rule_matched: Option<String>,
    },
    /// Payload for `EventKind::SnapSyncAttempted`.
    SnapSyncAttempted {
        local_height: u64,
        target_height: u64,
        source_peer_id: String,
    },
    /// Payload for `EventKind::SnapSyncCompleted`.
    SnapSyncCompleted { result: String, duration_ms: u64 },
    /// Payload for `EventKind::SnapSyncFailed`.
    SnapSyncFailed { error: String, duration_ms: u64 },
    /// Payload for `EventKind::ChainBreakDetected`.
    ChainBreakDetected {
        expected_prev_hash: String,
        actual_prev_hash: String,
        header_slot: u32,
        valid_so_far_count: u32,
        from_peer_id: String,
    },
    /// Payload for `EventKind::WriterHeartbeat`.
    WriterHeartbeat {
        events_written_total: u64,
        events_dropped_total: u64,
    },
}

// ---------------------------------------------------------------------------
// CorrelationKey — links causally related events across a fork episode
// ---------------------------------------------------------------------------

/// Groups diagnostic events that belong to the same fork episode.
///
/// When a fork is detected, subsequent rollback / reorg / snap-sync events
/// share the same `CorrelationKey` so the classifier can reconstruct the
/// causal chain. A canonical block with no fork context has all-`None` fields.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CorrelationKey {
    /// The height at which the fork diverged from the canonical chain.
    pub divergence_height: Option<u64>,
    /// Hash of the canonical block at `divergence_height`.
    pub canonical_hash: Option<String>,
    /// Hash of the fork block at `divergence_height`.
    pub fork_hash: Option<String>,
}

// ---------------------------------------------------------------------------
// DiagnosticEvent — the core event record
// ---------------------------------------------------------------------------

/// A single diagnostic event recorded by the observability layer.
///
/// Events are identified by a ULID (`event_id`) for time-ordered uniqueness.
/// The optional `correlation_key` links events belonging to the same fork
/// episode, and `caused_by_event_id` records direct causal predecessors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticEvent {
    /// ULID string — unique, time-ordered identifier.
    pub event_id: String,
    /// Event category.
    pub kind: EventKind,
    /// Wall-clock timestamp in milliseconds since UNIX epoch.
    pub timestamp_ms: u64,
    /// Block height associated with this event, if applicable.
    pub height: Option<u64>,
    /// Fork-episode grouping key (None for canonical-only events).
    pub correlation_key: Option<CorrelationKey>,
    /// ULID of the event that directly caused this one (causal chain).
    pub caused_by_event_id: Option<String>,
    /// Whether this is the first event in a cascade for its correlation key.
    pub is_cascade_origin: bool,
    /// Kind-specific structured payload.
    pub payload: EventPayload,
}

// ---------------------------------------------------------------------------
// BlockProvenance — network origin metadata for M2 apply_block signature
// ---------------------------------------------------------------------------

/// Network origin metadata for a received block.
///
/// Passed as `Option<BlockProvenance>` to `apply_block` in M2 so the emitter
/// can record which peer sent the block and when it arrived.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlockProvenance {
    /// Peer ID of the sender (None for locally-produced blocks).
    pub from_peer_id: Option<String>,
    /// Timestamp (ms since epoch) when the block was received from the network.
    pub received_at_ms: u64,
}

// ---------------------------------------------------------------------------
// ForkType — classifier output variants
// ---------------------------------------------------------------------------

/// Classification result from the deterministic fork-type classifier.
///
/// Named variants cover the known fork patterns observed in DOLI incidents.
/// `Unknown` is the safety valve for novel patterns (C3 convergent decision).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ForkType {
    /// Natural tip race between two producers in adjacent slots.
    TipRaceNatural,
    /// Tip race caused by high network latency.
    TipRaceHighLatency,
    /// Same producer emitted two different blocks for the same slot.
    ProducerEquivocation,
    /// Invalid block at an epoch boundary (reward mismatch, etc.).
    EpochBoundaryInvalid,
    /// Dead tip after snap sync (INC-I-012 pattern).
    PostSnapDeadTip,
    /// Nodes disagree on block validation outcome.
    ValidationDisagreement,
    /// Repeated shallow rollbacks forming a loop.
    RollbackLoop,
    /// Snap-synced to a minority fork peer.
    SnapSyncToMinorityFork,
    /// Novel pattern not matching any known variant.
    Unknown {
        /// Human-readable description of why classification failed.
        reason_unknown: String,
        /// Event IDs that formed the evidence for the unknown classification.
        evidence_event_ids: Vec<String>,
    },
}

// ---------------------------------------------------------------------------
// Classification — full classifier output
// ---------------------------------------------------------------------------

/// Complete classification result for a fork episode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Classification {
    /// The identified fork type.
    pub fork_type: ForkType,
    /// Confidence score in `[0.0, 1.0]`.
    pub confidence: f64,
    /// Event IDs used as evidence for this classification.
    pub evidence_event_ids: Vec<String>,
    /// Suggested remediation action (e.g. "shallow_rollback", "snap_sync").
    pub recommended_action: Option<String>,
    /// Optional structured arguments for the recommended action.
    pub recommended_action_args: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// ForkSummary — aggregate statistics for a diagnostic bundle
// ---------------------------------------------------------------------------

/// Aggregate statistics over the queried fork-event window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForkSummary {
    /// Total fork-related events in the window.
    pub fork_events_in_window: u64,
    /// Fork events grouped by producer public key.
    pub by_producer: HashMap<String, u64>,
    /// Fork events grouped by event kind name.
    pub by_event_kind: HashMap<String, u64>,
    /// Height of the first fork event in the window.
    pub first_fork_height: Option<u64>,
    /// Height of the last fork event in the window.
    pub last_fork_height: Option<u64>,
}

// ---------------------------------------------------------------------------
// BaselineComparison — rate comparison for anomaly detection
// ---------------------------------------------------------------------------

/// Compares the current fork-event rate against a rolling 24h baseline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BaselineComparison {
    /// Fork events per hour in the current query window.
    pub fork_events_per_hour_current: f64,
    /// Fork events per hour averaged over the last 24 hours.
    pub fork_events_per_hour_24h_avg: f64,
    /// Percentage delta: `(current - avg) / avg * 100`.
    pub delta_pct: f64,
}

// ---------------------------------------------------------------------------
// DiagnosticHealth — writer health status
// ---------------------------------------------------------------------------

/// Health status of the diagnostic writer subsystem.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticHealth {
    /// Whether the diagnostic ledger RocksDB is available.
    pub ledger_available: bool,
    /// Total events successfully written since node start.
    pub events_written_total: u64,
    /// Total events dropped (channel overflow) since node start.
    pub events_dropped_total: u64,
    /// Timestamp of the last writer heartbeat (None if no heartbeat yet).
    pub last_heartbeat_ms: Option<u64>,
}

// ---------------------------------------------------------------------------
// DiagnosticBundle — complete RPC response payload
// ---------------------------------------------------------------------------

/// Complete diagnostic bundle returned by `getForkDiagnostic` RPC (M3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticBundle {
    /// Schema version of this bundle.
    pub schema_version: u16,
    /// Peer ID of the node that produced this bundle.
    pub node_peer_id: String,
    /// Timestamp when this bundle was generated.
    pub query_timestamp_ms: u64,
    /// Diagnostic events in the query window.
    pub events: Vec<DiagnosticEvent>,
    /// Aggregate fork statistics.
    pub fork_summary: ForkSummary,
    /// Optional classifier output (None if not enough evidence).
    pub classification: Option<Classification>,
    /// Rate comparison against 24h baseline.
    pub baseline: BaselineComparison,
    /// Writer subsystem health.
    pub health: DiagnosticHealth,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during diagnostic event encoding/decoding.
#[derive(Debug)]
pub enum DecodeError {
    /// The byte slice is too short to contain the header.
    TooShort,
    /// Unknown format marker byte.
    UnknownFormatMarker(u8),
    /// Schema version is from the future and not supported.
    UnsupportedSchemaVersion(u16),
    /// Bincode deserialization failed.
    BincodeError(String),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => write!(f, "byte slice too short for diagnostic header"),
            Self::UnknownFormatMarker(b) => write!(f, "unknown format marker: 0x{:02x}", b),
            Self::UnsupportedSchemaVersion(v) => {
                write!(
                    f,
                    "unsupported schema version: {} (current: {})",
                    v, CURRENT_SCHEMA_VERSION
                )
            }
            Self::BincodeError(e) => write!(f, "bincode decode error: {}", e),
        }
    }
}

impl std::error::Error for DecodeError {}

// ---------------------------------------------------------------------------
// Encode / Decode helpers
// ---------------------------------------------------------------------------

/// Serialize a `DiagnosticEvent` to the on-disk format.
///
/// Layout: `[0x01][schema_version u16 LE][bincode payload]`
pub fn encode_event(event: &DiagnosticEvent) -> Result<Vec<u8>, String> {
    let payload = bincode::serialize(event).map_err(|e| e.to_string())?;
    let mut buf = Vec::with_capacity(1 + 2 + payload.len());
    buf.push(FORMAT_MARKER_BINCODE);
    buf.extend_from_slice(&CURRENT_SCHEMA_VERSION.to_le_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Deserialize a `DiagnosticEvent` from the on-disk format.
///
/// Accepts schema versions `<= CURRENT_SCHEMA_VERSION` (forward-compatible
/// for legacy). Rejects unknown future versions with a graceful `Err`.
pub fn decode_event(bytes: &[u8]) -> Result<DiagnosticEvent, DecodeError> {
    if bytes.len() < 3 {
        return Err(DecodeError::TooShort);
    }
    if bytes[0] != FORMAT_MARKER_BINCODE {
        return Err(DecodeError::UnknownFormatMarker(bytes[0]));
    }
    let version = u16::from_le_bytes([bytes[1], bytes[2]]);
    if version > CURRENT_SCHEMA_VERSION {
        return Err(DecodeError::UnsupportedSchemaVersion(version));
    }
    bincode::deserialize(&bytes[3..]).map_err(|e| DecodeError::BincodeError(e.to_string()))
}

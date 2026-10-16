//! Sync protocol for chain synchronization
//!
//! Request-response protocol for downloading headers and block bodies.

use serde::{Deserialize, Serialize};

use crypto::Hash;
use doli_core::{Block, BlockHeader};

pub use super::sync_codec::SyncCodec;

/// Protocol identifier for sync
pub const SYNC_PROTOCOL: &str = "/doli/sync/1.0.0";

/// INC-I-012 F13: Maximum message size for sync messages.
/// Reduced from 64MB to 16MB. State snapshots are the largest message type;
/// 16MB is generous for any reasonable UTXO set + producer set. Non-snapshot
/// messages (headers, bodies, blocks) are much smaller. The 64MB limit allowed
/// a malicious peer to force 64MB allocation per stream via the length prefix.
pub const MAX_SYNC_SIZE: usize = 16 * 1024 * 1024;

/// M3 [F3]: per-chunk body budget for a state-transfer session. Two orders of
/// magnitude under `MAX_SYNC_SIZE`, so the codec bound is never the binding
/// constraint on a chunk.
pub const STATE_CHUNK_MAX_BYTES: u32 = 1024 * 1024;

/// Concurrent state sessions one node will serve. Each holds a pinned storage
/// read for its lifetime, and a pinned read defers compaction.
pub const MAX_CONCURRENT_STATE_SESSIONS: usize = 4;

/// Wall-clock lifetime of a session, pinned view included.
pub const STATE_SESSION_TTL_SECS: u64 = 120;

/// Idle window after which a session is evicted ahead of its TTL.
pub const STATE_SESSION_IDLE_TIMEOUT_SECS: u64 = 30;

/// Why a state-transfer session cannot be served right now.
///
/// Typed, never `SyncResponse::Error`: the client's error path treats any error
/// as peer misbehaviour and blacklists, and every reason here is retryable.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum StateSessionRefusal {
    /// The serving node is at `MAX_CONCURRENT_STATE_SESSIONS`.
    Busy,
    /// No such session: never issued, evicted, expired, or lost to a restart.
    ManifestExpired,
    /// The serving node refuses to serve state at all (rebuild-in-progress halt).
    Halted(String),
}

/// Sync request types
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SyncRequest {
    /// Request headers starting from a hash
    GetHeaders {
        /// Start from this hash (or genesis if zero)
        start_hash: Hash,
        /// Maximum number of headers to return
        max_count: u32,
    },

    /// Request block bodies by hash
    GetBodies {
        /// Block hashes to fetch
        hashes: Vec<Hash>,
    },

    /// Request a specific block by height
    GetBlockByHeight {
        /// Block height
        height: u64,
    },

    /// Request a specific block by hash
    GetBlockByHash {
        /// Block hash
        hash: Hash,
    },

    /// Request a complete state snapshot at a specific block (snap sync)
    GetStateSnapshot {
        /// Block hash to snapshot at (should be a recent finalized block)
        block_hash: Hash,
    },

    /// Request only the state root hash for cross-peer verification (snap sync)
    GetStateRoot {
        /// Block hash to compute state root for
        block_hash: Hash,
    },

    /// Direct attestation delivery to next block producer (protocol v5+).
    /// Bypasses gossip mesh — causal delivery like orphan chase.
    DirectAttestation {
        /// Serialized attestation bytes
        data: Vec<u8>,
    },

    /// Request headers starting from a height (INC-I-012 F1).
    ///
    /// Used after snap sync when the node's local_hash is from a forked peer
    /// and no canonical peer recognizes it. Height-based lookup bypasses the
    /// hash lookup entirely — the server uses its OWN canonical hash at that
    /// height. The first header returned provides the client with a canonical
    /// hash to anchor subsequent GetHeaders requests.
    GetHeadersByHeight {
        /// Start from this height (returns headers from height+1 onward)
        start_height: u64,
        /// Maximum number of headers to return
        max_count: u32,
    },

    /// M3 [F3]: open a chunked state-transfer session (index 8 — APPENDED).
    GetStateManifest {
        /// Block hash the requester believes is the anchor
        block_hash: Hash,
    },

    /// M3 [F3]: fetch one sorted-key range of a session (index 9 — APPENDED).
    GetStateChunk {
        /// Session the manifest issued
        session_id: u64,
        /// Resume cursor; `None` starts at the first key
        start_key: Option<Vec<u8>>,
        /// Requested body budget. Advisory — the server clamps it.
        max_bytes: u32,
    },

    /// INC-I-248: ask a peer for its maintainer trust root (index 10 — APPENDED).
    GetMaintainerState,
}

/// Sync response types
#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum SyncResponse {
    /// Headers response
    Headers(Vec<BlockHeader>),

    /// Block bodies response
    Bodies(Vec<Block>),

    /// Single block response
    Block(Option<Block>),

    /// Complete state snapshot (snap sync)
    StateSnapshot {
        /// Block this snapshot is valid at
        block_hash: Hash,
        /// Block height at snapshot
        block_height: u64,
        /// Serialized ChainState (bincode)
        chain_state: Vec<u8>,
        /// Serialized UtxoSet (bincode)
        utxo_set: Vec<u8>,
        /// Serialized ProducerSet (bincode)
        producer_set: Vec<u8>,
        /// Merkle root: H(H(chain_state) || H(utxo_set) || H(producer_set))
        state_root: Hash,
        /// Anchor block header (post-activation). Allows receiving node to
        /// persist the header so set_canonical_chain walks never crash.
        #[serde(default)]
        block_header_bytes: Option<Vec<u8>>,
        /// Epoch bond snapshot: serialized (HashMap<Hash, u64>, u64).
        /// Allows receiving node to use the correct bond weights without
        /// recalculating from UTXO (which diverges due to mid-epoch add-bonds).
        #[serde(default)]
        epoch_bond_snapshot_bytes: Option<Vec<u8>>,
        /// Attestation accumulators: serialized (attested_set, attestation_accum, blocks_produced).
        /// Eliminates the 3-epoch convergence window after snap sync.
        #[serde(default)]
        epoch_accumulators_bytes: Option<Vec<u8>>,
        /// Complete EpochState (canonical serialization).
        /// When present, receiver uses this directly (no reconstruction needed).
        /// Backward compatible: older peers send None, receiver falls back.
        #[serde(default)]
        epoch_state_bytes: Option<Vec<u8>>,
    },

    /// State root only, for cross-peer verification (snap sync)
    StateRoot {
        /// Block hash this root is for
        block_hash: Hash,
        /// Block height this root is for (for grouping votes by height)
        block_height: u64,
        /// The computed state root
        state_root: Hash,
    },

    /// Error response
    Error(String),

    /// M3 [F3]: session opened; an O(1) description of an unbounded set (index 6 — APPENDED).
    StateManifest {
        /// Session id to quote on every `GetStateChunk`
        session_id: u64,
        /// Block this pinned view is anchored at
        block_hash: Hash,
        /// Height of that block
        block_height: u64,
        /// Composed three-component state root, identical to the legacy frame's
        state_root: Hash,
        /// BLAKE3 over the whole canonical UTXO image the chunks reconstruct
        utxo_hash: Hash,
        /// Entry count — the LE header the reassembled image starts with
        utxo_count: u64,
        /// Body budget the server will actually honour
        chunk_max_bytes: u32,
        /// Serialized ChainState (bincode)
        chain_state: Vec<u8>,
        /// Serialized ProducerSet (bincode)
        producer_set: Vec<u8>,
        /// Anchor block header, as on `StateSnapshot`
        #[serde(default)]
        block_header_bytes: Option<Vec<u8>>,
        /// Epoch bond snapshot, as on `StateSnapshot`
        #[serde(default)]
        epoch_bond_snapshot_bytes: Option<Vec<u8>>,
        /// Attestation accumulators, as on `StateSnapshot`
        #[serde(default)]
        epoch_accumulators_bytes: Option<Vec<u8>>,
        /// Complete EpochState, as on `StateSnapshot`
        #[serde(default)]
        epoch_state_bytes: Option<Vec<u8>>,
    },

    /// M3 [F3]: one sorted-key range of the pinned image (index 7 — APPENDED).
    StateChunk {
        /// Session this range belongs to
        session_id: u64,
        /// Canonical BODY bytes — no count header, entry-aligned
        body: Vec<u8>,
        /// Cursor for the next range; `None` means the walk is exhausted
        next_key: Option<Vec<u8>>,
    },

    /// M3 [F3]: typed, retryable session refusal (index 8 — APPENDED).
    StateSessionUnavailable {
        /// Session the client asked about
        session_id: u64,
        /// Why it cannot be served
        reason: StateSessionRefusal,
    },

    /// INC-I-248: the peer's maintainer trust root at its tip (index 9 — APPENDED).
    MaintainerStateAt {
        /// Peer tip hash when it answered
        block_hash: Hash,
        /// Peer tip height when it answered
        block_height: u64,
        /// `maintainer_set_digest(set, genesis_hash)` computed by the peer
        digest: Hash,
        /// bincode(`MaintainerSet`); empty set when the peer has none
        set_bytes: Vec<u8>,
        /// The peer's `MaintainerState.last_derived_height`
        last_derived_height: u64,
    },
}

impl SyncRequest {
    pub fn get_headers(start_hash: Hash, max_count: u32) -> Self {
        Self::GetHeaders {
            start_hash,
            max_count,
        }
    }

    pub fn get_bodies(hashes: Vec<Hash>) -> Self {
        Self::GetBodies { hashes }
    }

    pub fn get_block_by_height(height: u64) -> Self {
        Self::GetBlockByHeight { height }
    }

    pub fn get_block_by_hash(hash: Hash) -> Self {
        Self::GetBlockByHash { hash }
    }

    pub fn get_state_snapshot(block_hash: Hash) -> Self {
        Self::GetStateSnapshot { block_hash }
    }

    pub fn get_state_manifest(block_hash: Hash) -> Self {
        Self::GetStateManifest { block_hash }
    }

    pub fn get_state_chunk(session_id: u64, start_key: Option<Vec<u8>>, max_bytes: u32) -> Self {
        Self::GetStateChunk {
            session_id,
            start_key,
            max_bytes,
        }
    }

    pub fn get_state_root(block_hash: Hash) -> Self {
        Self::GetStateRoot { block_hash }
    }

    pub fn get_headers_by_height(start_height: u64, max_count: u32) -> Self {
        Self::GetHeadersByHeight {
            start_height,
            max_count,
        }
    }

    /// INC-I-120: Whether this outbound sync request is subject to the outbound
    /// rate governor at the `NetworkCommand::RequestSync` chokepoint.
    ///
    /// `true`  → bulk-catchup / retry-storm classes. These are what self-
    ///           amplified into the fleet-collapse storm and MUST be throttled.
    /// `false` → recovery + canonical-critical classes. Throttling these is the
    ///           INC-I-049 failure mode (a blunt limiter dropped a canonical
    ///           block → 9-min fork). They bypass the governor entirely (G1):
    ///           - `GetStateSnapshot` / `GetStateRoot` — snap-sync recovery
    ///           - `GetHeadersByHeight` — post-snap canonical anchor recovery
    ///           - `GetBlockByHash` — orphan-chase fetch of a canonical parent
    ///           - `DirectAttestation` — causal push (not a fetch retry)
    pub fn is_rate_governed(&self) -> bool {
        match self {
            SyncRequest::GetHeaders { .. }
            | SyncRequest::GetBodies { .. }
            | SyncRequest::GetBlockByHeight { .. } => true,
            SyncRequest::GetBlockByHash { .. }
            | SyncRequest::GetStateSnapshot { .. }
            | SyncRequest::GetStateRoot { .. }
            | SyncRequest::GetHeadersByHeight { .. }
            | SyncRequest::DirectAttestation { .. }
            | SyncRequest::GetStateManifest { .. }
            | SyncRequest::GetStateChunk { .. }
            | SyncRequest::GetMaintainerState => false,
        }
    }
}

impl SyncResponse {
    /// Returns a human-readable name for the response type (for logging)
    pub fn type_name(&self) -> &'static str {
        match self {
            SyncResponse::Headers(h) => {
                if h.is_empty() {
                    "Headers(empty)"
                } else {
                    "Headers"
                }
            }
            SyncResponse::Bodies(b) => {
                if b.is_empty() {
                    "Bodies(empty)"
                } else {
                    "Bodies"
                }
            }
            SyncResponse::Block(Some(_)) => "Block(Some)",
            SyncResponse::Block(None) => "Block(None)",
            SyncResponse::StateSnapshot { .. } => "StateSnapshot",
            SyncResponse::StateRoot { .. } => "StateRoot",
            SyncResponse::Error(_) => "Error",
            SyncResponse::StateManifest { .. } => "StateManifest",
            SyncResponse::StateChunk { .. } => "StateChunk",
            SyncResponse::StateSessionUnavailable { .. } => "StateSessionUnavailable",
            SyncResponse::MaintainerStateAt { .. } => "MaintainerStateAt",
        }
    }
}

/// Sync protocol definition
#[derive(Clone, Debug)]
pub struct SyncProtocol;

impl AsRef<str> for SyncProtocol {
    fn as_ref(&self) -> &str {
        SYNC_PROTOCOL
    }
}

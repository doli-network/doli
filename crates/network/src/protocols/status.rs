//! Status protocol for peer handshake
//!
//! Used to exchange chain state information between peers during connection.

use async_trait::async_trait;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::request_response;
use libp2p::StreamProtocol;
use serde::{Deserialize, Serialize};

use crypto::{Hash, PublicKey};

/// Protocol identifier for status exchange
pub const STATUS_PROTOCOL: &str = "/doli/status/1.0.0";

/// Current protocol version sent in status handshakes.
///
/// Bump this when the binary introduces changes that affect consensus,
/// block validation, or network protocol compatibility.
///
/// History:
///   1 — original (no version enforcement)
///   2 — version enforcement in status handshake
///   3 — INC-I-026 scheduler fix gated by
///       `NetworkParams::inc_i_026_scheduler_activation_height`. v3 binaries
///       run identical consensus to v2 BEFORE the per-network activation
///       height, and switch to the fork-resilient scheduler AT and AFTER it.
///       Wire-compatible with v2 across the gate (no message format change).
///   4 — INC-I-034 / M-Choice1: EpochState-in-state-root `HardForkSchedule`
///       entry scheduled. v4 binaries carry the
///       `compute_state_root_with_epoch_state` primitive and the
///       `EPOCH_SNAPSHOT_HF` entry; the actual state-root formula change is
///       height-gated via the schedule, NOT via the handshake. v3 peers
///       remain wire-compatible pre-activation. Bumping this constant is
///       the Phase-1 signal to peer-scoring that the binary is capable of
///       crossing the gate — call-site wiring lands in Phase-2.
///   5 — Direct attestation delivery via sync protocol. Nodes with v5+
///       can receive DirectAttestation requests. Senders check peer version
///       before sending — old peers only get gossip.
pub const CURRENT_PROTOCOL_VERSION: u32 = 5;

/// Minimum protocol version accepted from peers.
///
/// Peers reporting a version below this are disconnected immediately.
/// Bump this to partition old nodes off the network after a breaking change.
///
/// Kept at 1 for backward compatibility during INC-I-026 and
/// INC-I-034 / M-Choice1 rollouts. v2/v3 peers remain wire-compatible with
/// v4 because both changes are height-gated (INC-I-026 via
/// `NetworkParams::inc_i_026_scheduler_activation_height`, M-Choice1 via
/// `HardForkSchedule::EPOCH_SNAPSHOT_HF`) rather than handshake-gated. Bump
/// to 4 only AFTER mainnet crosses the M-Choice1 activation height on every
/// deployed network — at that point legacy state-root binaries cannot rejoin
/// safely.
pub const MIN_PEER_PROTOCOL_VERSION: u32 = 1;

/// Maximum message size for status messages (64KB)
const MAX_STATUS_SIZE: usize = 64 * 1024;

/// Status request
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatusRequest {
    /// Protocol version
    pub version: u32,
    /// Network ID (1 = mainnet, 2 = testnet)
    pub network_id: u32,
    /// Genesis hash for chain verification
    pub genesis_hash: Hash,
    /// Producer public key (if this node is a producer)
    /// Used to discover other producers during bootstrap before blocks are exchanged
    #[serde(default)]
    pub producer_pubkey: Option<PublicKey>,
}

/// Status response
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    /// Protocol version
    pub version: u32,
    /// Network ID
    pub network_id: u32,
    /// Genesis hash
    pub genesis_hash: Hash,
    /// Best block height
    pub best_height: u64,
    /// Best block hash
    pub best_hash: Hash,
    /// Best block slot
    pub best_slot: u32,
    /// Producer public key (if this node is a producer)
    /// Used to discover other producers during bootstrap before blocks are exchanged
    #[serde(default)]
    pub producer_pubkey: Option<PublicKey>,
}

impl StatusRequest {
    pub fn new(network_id: u32, genesis_hash: Hash) -> Self {
        Self {
            version: CURRENT_PROTOCOL_VERSION,
            network_id,
            genesis_hash,
            producer_pubkey: None,
        }
    }

    /// Create a status request with producer info for bootstrap discovery
    pub fn with_producer(network_id: u32, genesis_hash: Hash, producer_pubkey: PublicKey) -> Self {
        Self {
            version: CURRENT_PROTOCOL_VERSION,
            network_id,
            genesis_hash,
            producer_pubkey: Some(producer_pubkey),
        }
    }
}

impl StatusResponse {
    pub fn new(
        network_id: u32,
        genesis_hash: Hash,
        best_height: u64,
        best_hash: Hash,
        best_slot: u32,
    ) -> Self {
        Self {
            version: CURRENT_PROTOCOL_VERSION,
            network_id,
            genesis_hash,
            best_height,
            best_hash,
            best_slot,
            producer_pubkey: None,
        }
    }

    /// Create a status response with producer info for bootstrap discovery
    pub fn with_producer(
        network_id: u32,
        genesis_hash: Hash,
        best_height: u64,
        best_hash: Hash,
        best_slot: u32,
        producer_pubkey: PublicKey,
    ) -> Self {
        Self {
            version: CURRENT_PROTOCOL_VERSION,
            network_id,
            genesis_hash,
            best_height,
            best_hash,
            best_slot,
            producer_pubkey: Some(producer_pubkey),
        }
    }
}

/// Status protocol definition
#[derive(Clone, Debug)]
pub struct StatusProtocol;

impl AsRef<str> for StatusProtocol {
    fn as_ref(&self) -> &str {
        STATUS_PROTOCOL
    }
}

/// Codec for status messages
#[derive(Clone, Debug, Default)]
pub struct StatusCodec;

#[async_trait]
impl request_response::Codec for StatusCodec {
    type Protocol = StreamProtocol;
    type Request = StatusRequest;
    type Response = StatusResponse;

    async fn read_request<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
    ) -> std::io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let mut len_bytes = [0u8; 4];
        io.read_exact(&mut len_bytes).await?;
        let len = u32::from_le_bytes(len_bytes) as usize;

        if len > MAX_STATUS_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Status request too large",
            ));
        }

        let mut buf = vec![0u8; len];
        io.read_exact(&mut buf).await?;

        bincode::deserialize(&buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }

    async fn read_response<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
    ) -> std::io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        let mut len_bytes = [0u8; 4];
        io.read_exact(&mut len_bytes).await?;
        let len = u32::from_le_bytes(len_bytes) as usize;

        if len > MAX_STATUS_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Status response too large",
            ));
        }

        let mut buf = vec![0u8; len];
        io.read_exact(&mut buf).await?;

        bincode::deserialize(&buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }

    async fn write_request<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
        req: Self::Request,
    ) -> std::io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let bytes = bincode::serialize(&req)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        let len = (bytes.len() as u32).to_le_bytes();
        io.write_all(&len).await?;
        io.write_all(&bytes).await?;
        io.flush().await?;

        Ok(())
    }

    async fn write_response<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
        resp: Self::Response,
    ) -> std::io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let bytes = bincode::serialize(&resp)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        let len = (bytes.len() as u32).to_le_bytes();
        io.write_all(&len).await?;
        io.write_all(&bytes).await?;
        io.flush().await?;

        Ok(())
    }
}

// =============================================================================
// M-Choice1 — protocol version pins
// =============================================================================
//
// INC-I-034 / M-Choice1. Spec: specs/scheduler-state-architecture.md
// "Migration path — Phase 1: Pre-activation" item 3 (CURRENT_PROTOCOL_VERSION
// bump). Locked 2026-04-16 as CHOICE 1 = SAME HF.
//
// OUTPUT CONTRACT: const CURRENT_PROTOCOL_VERSION: u32
//   O1: value — MUST equal 4 (Phase-1 bump per M-Choice1)
// PATHS: P1 only (compile-time constant)
// MATRIX: 1 output × 1 path = 1 assertion (Test 6)
//
// OUTPUT CONTRACT: const MIN_PEER_PROTOCOL_VERSION: u32
//   O1: value — MUST remain 1 during Phase-1 rollout (height-gated HF keeps
//       v3 peers wire-compatible with v4 binaries)
// PATHS: P1 only (compile-time constant)
// MATRIX: 1 output × 1 path = 1 assertion (Test 7)
#[cfg(test)]
mod m_choice1_protocol_version_tests {
    use super::*;

    /// Test 6 — Phase-1 protocol version bump.
    ///
    /// The state-root formula can change at EPOCH_SNAPSHOT_HF's activation
    /// height. Binaries that carry the new schedule entry and the new
    /// `compute_state_root_with_epoch_state` function MUST advertise a bumped
    /// `CURRENT_PROTOCOL_VERSION` so peer-scoring can observe which binaries
    /// are capable of handling the transition (v3 binaries lack the function
    /// AND the schedule entry — they'd silently fork at activation if they
    /// connected to a v4 mesh that crosses the gate).
    #[test]
    fn test_m_choice1_current_protocol_version_is_4() {
        assert_eq!(
            CURRENT_PROTOCOL_VERSION, 4,
            "M-Choice1: CURRENT_PROTOCOL_VERSION must bump from 3 to 4 when \
             EPOCH_SNAPSHOT_HF is scheduled. Per CLAUDE.md 'After Every \
             Modification' step 3 — signal to peer scoring that this binary \
             may switch state-root formula at the scheduled height."
        );
    }

    /// Test 7 — MIN_PEER_PROTOCOL_VERSION held defensively at 1.
    ///
    /// Phase-1 is a pre-activation rollout — EPOCH_SNAPSHOT_HF activation is
    /// far-future. Raising MIN_PEER_PROTOCOL_VERSION now would immediately
    /// partition v3 peers (INC-I-026 rollout cohort) from the network even
    /// though they remain wire-compatible until the HF boundary. Hold at 1;
    /// bump to 4 (or higher) only AFTER mainnet crosses the activation
    /// height per spec Phase 2 cutover.
    #[test]
    fn test_m_choice1_min_peer_protocol_version_held_at_1() {
        assert_eq!(
            MIN_PEER_PROTOCOL_VERSION, 1,
            "M-Choice1: MIN_PEER_PROTOCOL_VERSION must remain at 1 during \
             Phase-1 rollout. v3 (INC-I-026) peers stay wire-compatible with \
             v4 (M-Choice1) binaries because the state-root change is \
             height-gated via HardForkSchedule::EPOCH_SNAPSHOT_HF, not via \
             the handshake. Bump to 4 only AFTER mainnet crosses the \
             activation height per spec Phase 2 cutover."
        );
    }
}

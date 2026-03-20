//! Sync protocol for chain synchronization
//!
//! Request-response protocol for downloading headers and block bodies.

use async_trait::async_trait;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::request_response;
use libp2p::StreamProtocol;
use serde::{Deserialize, Serialize};

use crypto::Hash;
use doli_core::{Block, BlockHeader};

/// Protocol identifier for sync
pub const SYNC_PROTOCOL: &str = "/doli/sync/1.0.0";

/// Maximum message size for sync messages (64MB for state transfers)
const MAX_SYNC_SIZE: usize = 64 * 1024 * 1024;

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

    /// Request blocks by height range (for efficient backfill)
    GetBlocksByHeightRange {
        /// Starting height (inclusive)
        start_height: u64,
        /// Number of blocks to return (max 500)
        count: u32,
    },

    /// Request state at a checkpoint height (for trusted initial sync).
    /// Only new nodes (height=0) send this. The response is verified against
    /// the hardcoded CHECKPOINT_STATE_ROOT in the binary.
    GetStateAtCheckpoint {
        /// Checkpoint height
        height: u64,
    },
}

/// Sync response types
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SyncResponse {
    /// Headers response
    Headers(Vec<BlockHeader>),

    /// Block bodies response
    Bodies(Vec<Block>),

    /// Single block response
    Block(Box<Option<Block>>),

    /// State at checkpoint (for trusted initial sync).
    /// Contains the full serialized state at a checkpoint height.
    StateAtCheckpoint {
        /// Block hash at the checkpoint
        block_hash: Hash,
        /// Block height
        block_height: u64,
        /// Serialized ChainState (bincode)
        chain_state: Vec<u8>,
        /// Serialized UtxoSet (canonical format)
        utxo_set: Vec<u8>,
        /// Serialized ProducerSet (bincode)
        producer_set: Vec<u8>,
        /// State root for verification
        state_root: Hash,
    },

    /// Error response
    Error(String),
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

    pub fn get_blocks_by_height_range(start_height: u64, count: u32) -> Self {
        Self::GetBlocksByHeightRange {
            start_height,
            count,
        }
    }

    pub fn get_state_at_checkpoint(height: u64) -> Self {
        Self::GetStateAtCheckpoint { height }
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
            SyncResponse::Block(b) if b.is_some() => "Block(Some)",
            SyncResponse::Block(_) => "Block(None)",
            SyncResponse::StateAtCheckpoint { .. } => "StateAtCheckpoint",
            SyncResponse::Error(_) => "Error",
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

/// Codec for sync messages
#[derive(Clone, Debug, Default)]
pub struct SyncCodec;

#[async_trait]
impl request_response::Codec for SyncCodec {
    type Protocol = StreamProtocol;
    type Request = SyncRequest;
    type Response = SyncResponse;

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

        if len > MAX_SYNC_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Sync request too large",
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

        if len > MAX_SYNC_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Sync response too large",
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

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

/// Maximum message size for sync messages (64MB for state snapshots)
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
}

/// Sync response types
#[derive(Clone, Debug, Serialize, Deserialize)]
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

    pub fn get_state_root(block_hash: Hash) -> Self {
        Self::GetStateRoot { block_hash }
    }

    pub fn get_headers_by_height(start_height: u64, max_count: u32) -> Self {
        Self::GetHeadersByHeight {
            start_height,
            max_count,
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

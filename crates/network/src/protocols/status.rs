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
            version: 1,
            network_id,
            genesis_hash,
            producer_pubkey: None,
        }
    }

    /// Create a status request with producer info for bootstrap discovery
    pub fn with_producer(network_id: u32, genesis_hash: Hash, producer_pubkey: PublicKey) -> Self {
        Self {
            version: 1,
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
            version: 1,
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
            version: 1,
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

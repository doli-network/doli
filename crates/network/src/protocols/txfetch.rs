//! Transaction fetch protocol
//!
//! Request-response protocol for fetching full transactions by hash.
//! Used with the announce-request pattern: nodes broadcast tx hashes via gossipsub,
//! then peers request the full transactions they don't have via this protocol.

use async_trait::async_trait;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::request_response;
use libp2p::StreamProtocol;
use serde::{Deserialize, Serialize};

use crypto::Hash;
use doli_core::Transaction;

/// Protocol identifier for transaction fetch
pub const TXFETCH_PROTOCOL: &str = "/doli/txfetch/1.0.0";

/// Maximum message size for tx fetch messages (256KB — enough for ~50 large txs)
const MAX_TXFETCH_SIZE: usize = 256 * 1024;

/// Maximum number of hashes per request
pub const MAX_TXFETCH_HASHES: usize = 50;

/// Transaction fetch request — ask a peer for transactions by hash
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TxFetchRequest {
    /// Transaction hashes to fetch (max 50)
    pub hashes: Vec<Hash>,
}

/// Transaction fetch response — return the transactions we have
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TxFetchResponse {
    /// Transactions found in our mempool (may be fewer than requested)
    pub transactions: Vec<Transaction>,
}

/// Codec for tx fetch messages
#[derive(Clone, Debug, Default)]
pub struct TxFetchCodec;

#[async_trait]
impl request_response::Codec for TxFetchCodec {
    type Protocol = StreamProtocol;
    type Request = TxFetchRequest;
    type Response = TxFetchResponse;

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

        if len > MAX_TXFETCH_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "TxFetch request too large",
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

        if len > MAX_TXFETCH_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "TxFetch response too large",
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

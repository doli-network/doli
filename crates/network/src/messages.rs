//! Network message types

use crypto::Hash;
use doli_core::{Block, BlockHeader, Transaction};
use serde::{Deserialize, Serialize};

/// Network message types
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Message {
    /// Announce new block
    NewBlock(Box<Block>),

    /// Announce new transaction
    NewTransaction(Transaction),

    /// Request blocks by hash
    GetBlocks(Vec<Hash>),

    /// Response with blocks
    Blocks(Vec<Block>),

    /// Request headers by hash range
    GetHeaders { start_hash: Hash, max_count: u32 },

    /// Response with headers
    Headers(Vec<BlockHeader>),

    /// Inventory announcement (type + hashes)
    Inv(InventoryType, Vec<Hash>),

    /// Request data for inventory items
    GetData(InventoryType, Vec<Hash>),

    /// Ping message
    Ping(u64),

    /// Pong response
    Pong(u64),

    /// Status message for handshake
    Status(StatusMessage),
}

/// Inventory item types
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InventoryType {
    Block,
    Transaction,
}

/// Status message for peer handshake
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatusMessage {
    /// Protocol version
    pub version: u32,
    /// Network ID
    pub network_id: u32,
    /// Best block height
    pub best_height: u64,
    /// Best block hash
    pub best_hash: Hash,
    /// Genesis hash
    pub genesis_hash: Hash,
}

impl Message {
    /// Serialize the message
    pub fn serialize(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    /// Deserialize a message
    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        bincode::deserialize(bytes).ok()
    }

    /// Get the message type name
    pub fn type_name(&self) -> &'static str {
        match self {
            Message::NewBlock(_) => "NewBlock",
            Message::NewTransaction(_) => "NewTransaction",
            Message::GetBlocks(_) => "GetBlocks",
            Message::Blocks(_) => "Blocks",
            Message::GetHeaders { .. } => "GetHeaders",
            Message::Headers(_) => "Headers",
            Message::Inv(_, _) => "Inv",
            Message::GetData(_, _) => "GetData",
            Message::Ping(_) => "Ping",
            Message::Pong(_) => "Pong",
            Message::Status(_) => "Status",
        }
    }
}

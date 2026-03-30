use libp2p::gossipsub::{Behaviour as Gossipsub, IdentTopic};

use crypto::Hash;
use doli_core::Transaction;

use super::{
    region_topic, GossipError, ATTESTATION_TOPIC, BLOCKS_TOPIC, HEADERS_TOPIC, HEARTBEATS_TOPIC,
    PRODUCERS_TOPIC, TIER1_BLOCKS_TOPIC, TRANSACTIONS_TOPIC, VOTES_TOPIC,
};

/// Publish a block to the network
pub fn publish_block(gossipsub: &mut Gossipsub, block_data: Vec<u8>) -> Result<(), GossipError> {
    let topic = IdentTopic::new(BLOCKS_TOPIC);
    gossipsub
        .publish(topic, block_data)
        .map_err(|e| GossipError::Publish(format!("block: {}", e)))?;
    Ok(())
}

/// Publish a transaction to the network
pub fn publish_transaction(gossipsub: &mut Gossipsub, tx_data: Vec<u8>) -> Result<(), GossipError> {
    let topic = IdentTopic::new(TRANSACTIONS_TOPIC);
    gossipsub
        .publish(topic, tx_data)
        .map_err(|e| GossipError::Publish(format!("tx: {}", e)))?;
    Ok(())
}

/// Publish a producer announcement to the network
pub fn publish_producer(
    gossipsub: &mut Gossipsub,
    producer_data: Vec<u8>,
) -> Result<(), GossipError> {
    let topic = IdentTopic::new(PRODUCERS_TOPIC);
    gossipsub
        .publish(topic, producer_data)
        .map_err(|e| GossipError::Publish(format!("producer: {}", e)))?;
    Ok(())
}

/// Publish a vote message to the network (for governance veto system)
pub fn publish_vote(gossipsub: &mut Gossipsub, vote_data: Vec<u8>) -> Result<(), GossipError> {
    let topic = IdentTopic::new(VOTES_TOPIC);
    gossipsub
        .publish(topic, vote_data)
        .map_err(|e| GossipError::Publish(format!("vote: {}", e)))?;
    Ok(())
}

/// Publish a heartbeat to the network (for weighted presence rewards)
pub fn publish_heartbeat(
    gossipsub: &mut Gossipsub,
    heartbeat_data: Vec<u8>,
) -> Result<(), GossipError> {
    let topic = IdentTopic::new(HEARTBEATS_TOPIC);
    gossipsub
        .publish(topic, heartbeat_data)
        .map_err(|e| GossipError::Publish(format!("heartbeat: {}", e)))?;
    Ok(())
}

/// Publish a block header to the lightweight headers topic (all tiers)
pub fn publish_header(gossipsub: &mut Gossipsub, header_data: Vec<u8>) -> Result<(), GossipError> {
    let topic = IdentTopic::new(HEADERS_TOPIC);
    gossipsub
        .publish(topic, header_data)
        .map_err(|e| GossipError::Publish(format!("header: {}", e)))?;
    Ok(())
}

/// Publish a block to the Tier 1 dense mesh topic
pub fn publish_tier1_block(
    gossipsub: &mut Gossipsub,
    block_data: Vec<u8>,
) -> Result<(), GossipError> {
    let topic = IdentTopic::new(TIER1_BLOCKS_TOPIC);
    gossipsub
        .publish(topic, block_data)
        .map_err(|e| GossipError::Publish(format!("t1_block: {}", e)))?;
    Ok(())
}

/// Publish an attestation message
pub fn publish_attestation(
    gossipsub: &mut Gossipsub,
    attestation_data: Vec<u8>,
) -> Result<(), GossipError> {
    let topic = IdentTopic::new(ATTESTATION_TOPIC);
    gossipsub
        .publish(topic, attestation_data)
        .map_err(|e| GossipError::Publish(format!("attestation: {}", e)))?;
    Ok(())
}

/// Publish a block to a regional topic (Tier 2 sharding)
pub fn publish_to_region(
    gossipsub: &mut Gossipsub,
    region: u32,
    block_data: Vec<u8>,
) -> Result<(), GossipError> {
    let topic = IdentTopic::new(region_topic(region));
    gossipsub
        .publish(topic, block_data)
        .map_err(|e| GossipError::Publish(format!("region_{}: {}", region, e)))?;
    Ok(())
}

/// Version prefix for batched transaction messages.
/// Must not collide with the first byte of a bincode-serialized Transaction
/// (version field: u32 LE, so 0x01 for v1, 0x02 for v2, etc.).
pub(super) const TX_MSG_BATCH: u8 = 0xBA;

/// Version prefix for transaction hash announcements (announce-request pattern).
/// Nodes broadcast hashes instead of full txs; peers request missing ones via txfetch.
pub const TX_MSG_ANNOUNCE: u8 = 0xAA;

/// Encode a batch of transactions with version prefix.
///
/// Format: `[0x01][u32 count LE][u32 len1 LE][tx1 bytes][u32 len2 LE][tx2 bytes]...`
pub fn encode_tx_batch(transactions: &[Transaction]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.push(TX_MSG_BATCH);
    buf.extend_from_slice(&(transactions.len() as u32).to_le_bytes());
    for tx in transactions {
        let data = tx.serialize();
        buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
        buf.extend_from_slice(&data);
    }
    buf
}

/// Decoded transaction gossip message — dispatched by prefix byte.
pub enum TxGossipMessage {
    /// Full transaction batch (0xBA prefix or legacy single-tx)
    FullBatch(Vec<Transaction>),
    /// Transaction hash announcements (0xAA prefix)
    Announce(Vec<Hash>),
}

/// Decode a transaction gossip message. Handles all formats:
///
/// - `0xAA` prefix → hash announcement batch
/// - `0xBA` prefix → full transaction batch
/// - Other → legacy single-tx bincode deserialization
///
/// Returns `None` on empty input or decode failure.
pub fn decode_tx_gossip(data: &[u8]) -> Option<TxGossipMessage> {
    if data.is_empty() {
        return None;
    }

    match data[0] {
        TX_MSG_ANNOUNCE => decode_tx_announce(data).map(TxGossipMessage::Announce),
        TX_MSG_BATCH => decode_tx_batch(data).map(TxGossipMessage::FullBatch),
        _ => {
            // Legacy single-tx format
            Transaction::deserialize(data).map(|tx| TxGossipMessage::FullBatch(vec![tx]))
        }
    }
}

/// Decode a transaction message. Handles both single (legacy) and batched formats.
///
/// - If the first byte is `0xBA`, decodes as a batch.
/// - Otherwise, attempts legacy single-tx bincode deserialization.
/// - Returns `None` on empty input or decode failure.
pub fn decode_tx_message(data: &[u8]) -> Option<Vec<Transaction>> {
    if data.is_empty() {
        return None;
    }

    if data[0] == TX_MSG_BATCH {
        decode_tx_batch(data)
    } else if data[0] == TX_MSG_ANNOUNCE {
        // Announcement messages are not full txs — return None
        None
    } else {
        // Legacy single-tx format
        Transaction::deserialize(data).map(|tx| vec![tx])
    }
}

/// Decode a batched transaction message (0xBA prefix).
fn decode_tx_batch(data: &[u8]) -> Option<Vec<Transaction>> {
    if data.len() < 5 {
        return None;
    }
    let count = u32::from_le_bytes(data[1..5].try_into().ok()?) as usize;
    if count == 0 || count > 10_000 {
        return None;
    }
    // Cap pre-allocation to prevent OOM from malicious count values.
    // Each tx needs at least 4 bytes (length prefix), so bound by remaining data.
    let max_possible = (data.len() - 5) / 4;
    let mut txs = Vec::with_capacity(count.min(max_possible));
    let mut offset = 5;
    for _ in 0..count {
        if offset + 4 > data.len() {
            return None;
        }
        let len = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
        offset += 4;
        if offset + len > data.len() {
            return None;
        }
        let tx = Transaction::deserialize(&data[offset..offset + len])?;
        txs.push(tx);
        offset += len;
    }
    Some(txs)
}

/// Encode transaction hash announcements.
///
/// Format: `[0xAA][u32 count LE][hash1: 32 bytes][hash2: 32 bytes]...`
pub fn encode_tx_announce(hashes: &[Hash]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 4 + hashes.len() * 32);
    buf.push(TX_MSG_ANNOUNCE);
    buf.extend_from_slice(&(hashes.len() as u32).to_le_bytes());
    for hash in hashes {
        buf.extend_from_slice(hash.as_bytes());
    }
    buf
}

/// Decode transaction hash announcements (0xAA prefix).
fn decode_tx_announce(data: &[u8]) -> Option<Vec<Hash>> {
    if data.len() < 5 || data[0] != TX_MSG_ANNOUNCE {
        return None;
    }
    let count = u32::from_le_bytes(data[1..5].try_into().ok()?) as usize;
    if count == 0 {
        return None;
    }
    // Sanity: don't accept absurdly large counts
    if count > 1000 {
        return None;
    }
    let expected_len = 5 + count * 32;
    if data.len() < expected_len {
        return None;
    }
    let mut hashes = Vec::with_capacity(count);
    let mut offset = 5;
    for _ in 0..count {
        let hash_bytes: [u8; 32] = data[offset..offset + 32].try_into().ok()?;
        hashes.push(Hash::from(hash_bytes));
        offset += 32;
    }
    Some(hashes)
}

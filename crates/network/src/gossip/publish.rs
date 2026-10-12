use crypto::Hash;
use doli_core::Transaction;

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

/// Smallest batch frame: 4-byte length + a 32-byte tx (bincode, empty vecs).
const MIN_TX_FRAME_BYTES: usize = 4 + 32;

/// Decode a batched transaction message (0xBA prefix).
fn decode_tx_batch(data: &[u8]) -> Option<Vec<Transaction>> {
    if data.len() < 5 {
        return None;
    }
    let count = u32::from_le_bytes(data[1..5].try_into().ok()?) as usize;
    if count == 0 || count > 10_000 {
        return None;
    }
    // Reserve `count` only when the payload can hold `count` minimum frames;
    // a shorter payload cannot complete, so it reserves at most one slot.
    let max_fit = (data.len() - 5) / MIN_TX_FRAME_BYTES;
    let mut txs = Vec::with_capacity(if count <= max_fit {
        count
    } else {
        max_fit.min(1)
    });
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

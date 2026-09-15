//! The one canonical UTXO codec.
//!
//! Every consensus byte of `utxo_hash` comes from [`fold`]: a `u64` LE count
//! header, then `key || UtxoEntry::serialize_canonical_bytes()` per entry in
//! ascending key order. Digest, length, range, and the materialised image are
//! all built on it, so no second encoder exists to drift from it.

use std::ops::ControlFlow;

use crypto::Hash;

use super::in_memory::InMemoryUtxoStore;
use super::types::{Outpoint, UtxoEntry};
use crate::StorageError;

/// Canonical key: 32-byte tx hash + 4-byte LE output index.
pub(crate) const KEY_LEN: usize = 36;

/// Canonical entry value without `extra_data` (`UtxoEntry::canonical_byte_size`).
const VALUE_BASE_LEN: usize = 61;

/// Per-entry capacity hint. A hint only — it can never reach the emitted bytes.
const ENTRY_HINT: usize = KEY_LEN + VALUE_BASE_LEN;

/// Sink for the canonical `(key, value)` bytes of one entry.
type EmitBytes<'a> = &'a mut dyn FnMut(&[u8], &[u8]) -> ControlFlow<()>;

/// Sink for one decoded `(key, entry)` row.
pub(crate) type EmitEntry<'a> = &'a mut dyn FnMut(&[u8], &UtxoEntry) -> ControlFlow<()>;

/// What the fold does when a stored value will not decode.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnUndecodable {
    /// Consensus paths: refuse to emit an image that is missing an entry (AP-7).
    Fail,
    /// Pre-M2 skip. Kept only for [`materialize`] until M3 retires it.
    Skip,
}

/// The rows of one UTXO backend, ascending by canonical key, over a pinned view.
pub(crate) trait CanonicalRows {
    /// Capacity hint only. Never a source of emitted bytes or of the header.
    fn size_hint(&self) -> usize;

    /// Rows this view holds. Must count exactly what `for_each_entry(None, ..)`
    /// walks: the header count is derived from it, never from a live counter
    /// that an iteration can desync from (STOR028).
    fn row_count(&self) -> Result<u64, StorageError>;

    /// Feed every entry with `key >= start` to `f`, ascending, stopping on
    /// `Break`. `Err` on an iterator error, and on an undecodable value unless
    /// `policy` is [`OnUndecodable::Skip`].
    fn for_each_entry(
        &self,
        start: Option<&[u8]>,
        policy: OnUndecodable,
        f: EmitEntry<'_>,
    ) -> Result<(), StorageError>;
}

/// Feed `emit` the canonical `(key, value)` bytes of every entry from `start`.
pub(crate) fn fold(
    rows: &dyn CanonicalRows,
    start: Option<&[u8]>,
    policy: OnUndecodable,
    emit: EmitBytes<'_>,
) -> Result<(), StorageError> {
    rows.for_each_entry(start, policy, &mut |key, entry| {
        emit(key, &entry.serialize_canonical_bytes())
    })
}

/// Streaming BLAKE3 over the canonical image, with its byte length.
///
/// Two passes over one pinned view: a streaming hasher cannot back-patch the
/// LE header, and the header must come from the rows the body emits (STOR028).
/// Pass 1 reads keys only; the fail-loud pass 2 can never emit fewer.
pub(crate) fn digest_and_len(rows: &dyn CanonicalRows) -> Result<(Hash, u64), StorageError> {
    let count = rows.row_count()?;
    let mut hasher = crypto::Hasher::new();
    hasher.update(&count.to_le_bytes());

    let mut len = 8u64;
    fold(rows, None, OnUndecodable::Fail, &mut |key, value| {
        hasher.update(key);
        hasher.update(value);
        len += (key.len() + value.len()) as u64;
        ControlFlow::Continue(())
    })?;

    Ok((hasher.finalize(), len))
}

/// Byte length of the canonical image, without materialising it.
pub(crate) fn len(rows: &dyn CanonicalRows) -> Result<u64, StorageError> {
    let mut len = 8u64;
    fold(rows, None, OnUndecodable::Fail, &mut |key, value| {
        len += (key.len() + value.len()) as u64;
        ControlFlow::Continue(())
    })?;
    Ok(len)
}

/// Canonical BODY slice (no header) from the first key `>= start`, with the
/// cursor of the next entry. `None` means the set is exhausted.
///
/// At least one entry is emitted whenever one exists, so a budget smaller than
/// one entry still makes progress instead of looping on an empty chunk.
pub(crate) fn range(
    rows: &dyn CanonicalRows,
    start: Option<&[u8]>,
    max_bytes: usize,
) -> Result<(Vec<u8>, Option<Vec<u8>>), StorageError> {
    let mut body: Vec<u8> = Vec::new();
    let mut next: Option<Vec<u8>> = None;

    fold(rows, start, OnUndecodable::Fail, &mut |key, value| {
        if !body.is_empty() && body.len() + key.len() + value.len() > max_bytes {
            next = Some(key.to_vec());
            return ControlFlow::Break(());
        }
        body.extend_from_slice(key);
        body.extend_from_slice(value);
        ControlFlow::Continue(())
    })?;

    Ok((body, next))
}

/// Materialise the whole canonical image: header then body.
///
/// The remaining wire-bytes caller is `StateSnapshot::create`; M3 moves it to
/// [`range`]. [`OnUndecodable::Skip`] holds its pre-M2 bytes, and the header
/// still counts only what the body emitted (STOR028).
pub(crate) fn materialize(rows: &dyn CanonicalRows) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8 + rows.size_hint() * ENTRY_HINT);
    buf.extend_from_slice(&0u64.to_le_bytes());

    let mut count = 0u64;
    let _ = fold(rows, None, OnUndecodable::Skip, &mut |key, value| {
        buf.extend_from_slice(key);
        buf.extend_from_slice(value);
        count += 1;
        ControlFlow::Continue(())
    });

    buf[..8].copy_from_slice(&count.to_le_bytes());
    buf
}

/// Rebuild an in-memory store from a canonical image (inverse of [`materialize`]).
///
/// Format: `[8-byte LE count] [36-byte outpoint][entry_bytes] ...`.
pub(crate) fn deserialize(bytes: &[u8]) -> Result<InMemoryUtxoStore, StorageError> {
    if bytes.len() < 8 {
        return Err(StorageError::Serialization(format!(
            "[STOR027] UTXO canonical bytes too short: {} bytes (min 8)",
            bytes.len()
        )));
    }
    let count = u64::from_le_bytes(bytes[0..8].try_into().unwrap()) as usize;
    let mut store = InMemoryUtxoStore::new();
    let mut pos = 8;

    for _ in 0..count {
        if pos + KEY_LEN > bytes.len() {
            return Err(StorageError::Serialization(format!(
                "[STOR028] UTXO canonical bytes truncated at outpoint (pos={}, len={})",
                pos,
                bytes.len()
            )));
        }
        let outpoint = Outpoint::from_bytes(&bytes[pos..pos + KEY_LEN]).ok_or_else(|| {
            StorageError::Serialization(format!(
                "[STOR029] invalid outpoint in canonical bytes at pos={}",
                pos
            ))
        })?;
        pos += KEY_LEN;

        if pos + VALUE_BASE_LEN > bytes.len() {
            return Err(StorageError::Serialization(format!(
                "[STOR030] UTXO canonical bytes truncated at entry (pos={}, len={})",
                pos,
                bytes.len()
            )));
        }
        // 0xFFFF marks a u32 extra_data length (NFT payloads above 64 KB).
        let raw_len = u16::from_le_bytes(bytes[pos + 59..pos + 61].try_into().unwrap());
        let (extra_len, header_overhead) = if raw_len == 0xFFFF {
            if pos + 65 > bytes.len() {
                return Err(StorageError::Serialization(
                    "UTXO canonical bytes truncated (u32 length)".to_string(),
                ));
            }
            let len = u32::from_le_bytes(bytes[pos + 61..pos + 65].try_into().unwrap()) as usize;
            (len, 65)
        } else {
            (raw_len as usize, VALUE_BASE_LEN)
        };
        let entry_size = header_overhead + extra_len;
        if pos + entry_size > bytes.len() {
            return Err(StorageError::Serialization(format!(
                "[STOR031] UTXO canonical bytes truncated at extra_data (pos={}, entry_size={}, len={})",
                pos,
                entry_size,
                bytes.len()
            )));
        }
        let entry = UtxoEntry::deserialize_canonical_bytes(&bytes[pos..pos + entry_size])
            .ok_or_else(|| {
                StorageError::Serialization(format!(
                    "[STOR032] invalid UTXO entry in canonical bytes at pos={} (entry_size={})",
                    pos, entry_size
                ))
            })?;
        pos += entry_size;

        store.insert(outpoint, entry);
    }

    Ok(store)
}

/// Canonical key bytes of an outpoint, without a heap allocation.
fn outpoint_key(outpoint: &Outpoint) -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    key[0..32].copy_from_slice(outpoint.tx_hash.as_bytes());
    key[32..KEY_LEN].copy_from_slice(&outpoint.index.to_le_bytes());
    key
}

impl CanonicalRows for InMemoryUtxoStore {
    fn size_hint(&self) -> usize {
        self.len()
    }

    fn row_count(&self) -> Result<u64, StorageError> {
        Ok(self.len() as u64)
    }

    fn for_each_entry(
        &self,
        start: Option<&[u8]>,
        _policy: OnUndecodable,
        f: EmitEntry<'_>,
    ) -> Result<(), StorageError> {
        // A HashMap has no order: the keys at or after the cursor are collected
        // and sorted, never the values.
        let mut rows: Vec<([u8; KEY_LEN], &UtxoEntry)> = self
            .iter()
            .map(|(outpoint, entry)| (outpoint_key(outpoint), entry))
            .filter(|(key, _)| start.is_none_or(|s| key.as_slice() >= s))
            .collect();
        rows.sort_unstable_by_key(|(key, _)| *key);

        for (key, entry) in rows {
            if f(&key, entry).is_break() {
                break;
            }
        }
        Ok(())
    }
}

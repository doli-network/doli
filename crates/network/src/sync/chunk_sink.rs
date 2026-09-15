//! M4 [F3] staged-install seam: where a verified snap-sync chunk body goes.
//!
//! `crates/network` has no `storage` dependency, so the session client cannot
//! reach `StateDb::stage_utxo_bytes`. It calls this trait instead; `bins/node`
//! implements it over the `StateDb` (memory.db decision 130).

use crypto::Hash;

/// Durable landing zone for verified UTXO chunk bodies.
///
/// A body reaches `stage` only after the session accepted the range. Every
/// abandon path calls `clear`: rows left behind are concatenated onto the next
/// peer's stream and match neither manifest.
pub trait UtxoChunkSink: Send + Sync {
    /// Append one verified canonical BODY run (no count header).
    fn stage(&self, body: &[u8]) -> Result<(), String>;

    /// Discard everything staged, including any partial-record residual.
    fn clear(&self) -> Result<(), String>;

    /// Rows currently staged.
    fn staged_len(&self) -> u64;
}

/// What a completed staged session hands the install instead of an image.
///
/// Derived from fields the manifest already carries. It MUST NOT be put on the
/// wire — `m4_wire_unchanged` locks the M3 frame bytes.
#[derive(Clone, Debug, Default)]
pub struct StagedUtxoMarker {
    /// Manifest digest the staged rows must re-derive to (INV-SYNC-007).
    pub utxo_hash: Hash,
    /// Manifest entry count, so a truncated promotion is detectable.
    pub utxo_count: u64,
}

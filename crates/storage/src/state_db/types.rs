//! State database types, constants, and struct definitions

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicU64;

use crypto::Hash;
use doli_core::maintainer::MaintainerSet;
use serde::{Deserialize, Serialize};

use crate::utxo::{Outpoint, UtxoEntry};

/// Reverse diff for a single block — enough to undo all state changes.
///
/// Stored in `cf_undo` keyed by block height. Enables O(rollback_depth) reorgs
/// instead of O(chain_height) rebuild-from-genesis.
///
/// ## This encoding is APPEND-HOSTILE. Do not add a field.
///
/// Bincode is non-self-describing: field order and arity are implied by the type,
/// never carried in the bytes. Appending a field therefore makes every entry written
/// by the previous binary fail to decode, and `StateDb::get_undo` maps that failure to
/// `None` — the same value it returns for "this height was never written". The node
/// then silently takes the rebuild-from-genesis fallback
/// (`bins/node/src/node/rollback.rs`, AUDIT-P1-003 / INC-I-156 territory) and
/// `execute_reorg`'s `all(|h| get_undo(h).is_some())` gate closes for the WHOLE rewind
/// range, for up to `UNDO_KEEP_DEPTH` blocks after each node's restart.
///
/// INC-I-174 M1 needed a per-height maintainer snapshot and deliberately did **not**
/// add it here. It lives in a separate `cf_undo` record under a distinct key prefix
/// ([`MaintainerUndoSnapshot`], `state_db/undo.rs`), which leaves this encoding
/// byte-identical and every pre-upgrade entry readable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoData {
    /// UTXOs that were spent by this block (restore on rollback).
    pub spent_utxos: Vec<(Outpoint, UtxoEntry)>,
    /// UTXOs that were created by this block (delete on rollback).
    pub created_utxos: Vec<Outpoint>,
    /// Serialized ProducerSet snapshot BEFORE this block was applied.
    /// Producer state is complex (bonds, pending updates, epoch boundaries)
    /// so we snapshot instead of tracking individual deltas.
    pub producer_snapshot: Vec<u8>,
    /// Serialized EpochState snapshot BEFORE this block was applied.
    /// Enables O(1) rollback of scheduler state instead of rebuilding from blocks.
    ///
    /// CORRECTED (INC-I-174 M1, measured). This field used to be documented as
    /// "None for blocks created before this field was added (backward compat)".
    /// That claim is FALSE and the `#[serde(default)]` below cannot deliver it:
    /// `#[serde(default)]` only fires for a self-describing format that can report a
    /// MISSING FIELD, and bincode reports EOF instead. An undo entry written before
    /// this field existed does NOT decode under today's `UndoData` — it decodes to an
    /// `Err`, which `get_undo` turns into `None`, which every caller reads as
    /// "no undo data at this height".
    ///
    /// Measured by `crates/storage/tests/inc_i_174_undo_schema.rs`
    /// (`req_174_004_the_existing_serde_default_backward_compat_claim_is_measured`).
    /// The attribute is kept only because removing it would change nothing on the wire;
    /// it must never again be cited as a licence to append a field. See the type-level
    /// note above.
    #[serde(default)]
    pub epoch_state_snapshot: Option<Vec<u8>>,
    /// Legacy field — retained so the field arity of this struct does not move.
    /// Chain commitment is now computed via periodic full scan, not incrementally.
    /// The `#[serde(default)]` here carries the same false promise as the one above:
    /// it does NOT make an older entry readable. Retained, not relied on.
    #[serde(default)]
    pub chain_commitment: Option<[u8; 32]>,
}

/// The maintainer trust root as it stood BEFORE a block that rotates it.
///
/// INC-I-174. The maintainer set is the auto-updater's release-verification trust root.
/// It is node-local — never gossiped, never hashed, absent from
/// `ChainState::serialize_canonical` — so it is NOT part of the consensus state root and
/// this record needs no activation height. But it IS mutated by `AddMaintainer` /
/// `RemoveMaintainer` transactions, and before this record existed nothing could undo
/// that mutation: a reorg that dropped the rotation left the node's install authority
/// permanently diverged from the canonical chain, in memory and on disk.
///
/// ## Why this is a SEPARATE `cf_undo` record and not a field on [`UndoData`]
///
/// See the append-hostility note on [`UndoData`]. Adding a sixth field there would have
/// made every pre-upgrade `cf_undo` entry undecodable, silently dropping rollbacks into
/// the rebuild-from-genesis fallback and closing the reorg gate for up to
/// `UNDO_KEEP_DEPTH` blocks per node. Keyed separately, the two encodings evolve
/// independently and no existing entry is disturbed.
///
/// ## Sizing
///
/// At most `MAX_MAINTAINERS` (5) × 32 B of keys plus two integers — roughly 200 B,
/// four orders of magnitude below the `ProducerSet` snapshot whose per-block write
/// caused the INC-I-071 605 MB `cf_undo` bloat. The record is nevertheless written
/// ONLY for a block that carries a rotation: absence is the "unchanged at this height"
/// sentinel and costs zero bytes.
///
/// ## Every field matters
///
/// `last_derived_height` is not decoration. `Node::maintainer_seed_is_done`
/// (`bins/node/src/node/periodic.rs`) reads "never seeded" as
/// `members.is_empty() && last_derived_height == 0`, and the seed is driven on EVERY
/// applied block. A restore that dropped `last_derived_height` to 0 with an empty set
/// would re-arm the one-shot bootstrap, which re-derives the root from LIVE producer
/// state and RE-ARMS ANY KEY GOVERNANCE REMOVED — the INC-I-172 R1 hazard, strictly
/// worse than the divergence this record exists to fix.
///
/// ## Why the record is SELF-DESCRIBING and BOUND (AUDIT-P1-001 / SYS-001)
///
/// The first shape of this record carried only `set` + `last_derived_height`, and the
/// rewind path authorized it by its KEY — "a snapshot exists at height `h`, and the block
/// now at `h` carries a rotation, therefore restore it". The five-lens M1 security audit
/// converged on that as one structural property (SYS-001): *a new authority record is
/// trusted for its POSITION, never for its CONTENT-authenticity or its BINDING to the
/// block it describes.*
///
/// It is not a theoretical gap. `plan_maintainer_rewind` reads the block through
/// `get_block_by_height` → `CF_HEIGHT_INDEX`, and `BlockStore::put_block_canonical`
/// rewrites that index WITHOUT going through `apply_block` and WITHOUT refreshing this
/// record — on `backfillFromPeer` (an online RPC), `doli-node restore`, the archiver, and
/// `rebuild_canonical_index`. After any of those, a legitimate operator recovery can leave
/// a DIFFERENT, rotation-carrying block at `h` while the record below it still describes
/// the abandoned one. The position check then passes and a member list that exists on NO
/// canonical chain — under INC-I-175, one still holding the publicly leaked bootstrap keys
/// — installs through the SUCCESS exit.
///
/// So the record now answers three questions about ITSELF, and the reader
/// (`bins/node/src/node/maintainer_rewind/binding.rs`) checks all three before a restore:
///
/// * [`Self::magic`] + [`Self::version`] — *was this written by a binary of this
///   generation?* A record from another key family, a truncated value, or a future format
///   is refused instead of being decoded into plausible-looking members.
/// * [`Self::block_hash`] — *is this the record for THIS block?* This is the
///   AUDIT-P1-001 closer, and it holds whatever rewrote the canonical index, because it
///   compares the record against the block itself rather than against the index that
///   pointed at it.
/// * [`Self::set_digest`] — *does the member list still match the one this record was
///   filed with?* Recomputed from `set` and the chain's genesis hash on the restore path,
///   so a set edited in place on disk, or a record lifted from another chain, no longer
///   matches its own record.
///
/// ## What the three checks do NOT do (AUDIT-P3-401)
///
/// They are **staleness and drift detection, not tamper detection.** All three inputs are
/// PUBLIC and none is keyed: `magic`/`version` are compiled constants, `block_hash` is
/// readable from the same data dir the record lives in, and `maintainer_set_digest` is
/// `BLAKE3(domain ‖ genesis_hash ‖ threshold ‖ sorted members)` with no node secret. So the
/// checks detect a FOSSIL record (left behind by an abandoned branch or a rewritten height
/// index), a record from ANOTHER CHAIN, a record for a DIFFERENT BLOCK, a record whose
/// member list was edited in place after capture, and a record written by a different
/// BINARY GENERATION. They do NOT detect deliberate tampering by an actor who can write the
/// data dir: that actor edits the member list and recomputes a matching `block_hash` and
/// `set_digest` in one BLAKE3 call, and the record verifies.
///
/// That residual is ACCEPTED, not overlooked. The same write access reaches
/// `maintainer_state.bin` directly — `crates/storage/src/lib.rs`
/// [`crate::StorageError::MalformedPersistedValue`] documents that file as unsigned and
/// attacker-writable given data-dir access, and it is the LIVE trust root rather than an
/// undo record consulted only across a rewind. Editing it is a strictly shorter path to the
/// same authority, so this record is not the control standing between that actor and the
/// trust root, and a keyed MAC here would move nothing. Do not read these checks as
/// authentication, tamper-proofing or integrity protection against an attacker, and do not
/// retire another control (for example the `TrustRoot::resolve` containment guard) on the
/// strength of them.
///
/// None of this is consensus-visible: the record stays node-local, so there is no
/// activation height, no `*_VERSION` bump and no new column family. Changing the shape is
/// safe on the wire because the 9-byte `0x4D` key family was introduced by INC-I-174 M1
/// itself and has exactly two writers, both new — unlike [`UndoData`], no pre-upgrade
/// entry exists to be broken.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintainerUndoSnapshot {
    /// Format magic — always [`MaintainerUndoSnapshot::MAGIC`].
    pub magic: [u8; 4],
    /// Format version — always [`MaintainerUndoSnapshot::VERSION`].
    pub version: u16,
    /// Hash of the block this record was captured FOR — the binding that makes the
    /// record authority for a BLOCK rather than for a HEIGHT.
    pub block_hash: Hash,
    /// `maintainer_set_digest(set, genesis_hash)` as computed at capture time.
    ///
    /// Stored as raw bytes so `crates/storage` gains no dependency edge on the digest
    /// module: the node computes it at capture and recomputes it at restore.
    pub set_digest: [u8; 32],
    /// The member list and threshold as they stood before the block.
    pub set: MaintainerSet,
    /// `MaintainerState::last_derived_height` as it stood before the block.
    pub last_derived_height: u64,
}

impl MaintainerUndoSnapshot {
    /// ASCII `MUND`. Four bytes, so a value that is not one of these records — a
    /// truncated write, a foreign key family, a hand-edited blob — fails to decode or
    /// fails this check instead of yielding a plausible member list.
    pub const MAGIC: [u8; 4] = *b"MUND";

    /// Version 1 is the INC-I-174 M1 shape. A reader refuses anything else rather than
    /// guessing: this record decides which binary the host installs, so "decode what you
    /// can" is the wrong failure direction.
    pub const VERSION: u16 = 1;

    /// Stamp a new record with the current header. The ONLY constructor used in
    /// production, so `magic`/`version` cannot be forgotten at a capture site.
    pub fn new(
        block_hash: Hash,
        set_digest: [u8; 32],
        set: MaintainerSet,
        last_derived_height: u64,
    ) -> Self {
        Self {
            magic: Self::MAGIC,
            version: Self::VERSION,
            block_hash,
            set_digest,
            set,
            last_derived_height,
        }
    }

    /// True when the header names this exact format generation.
    pub fn header_is_valid(&self) -> bool {
        self.magic == Self::MAGIC && self.version == Self::VERSION
    }
}

/// INC-I-104 M0: hard cap on total memtable budget across all CFs.
/// Shared between `open()` (sets `db_write_buffer_size`) and `metrics()`.
/// Per Failure Analyst C-002: must be >= 32 MB for snap-sync atomic_replace.
pub(super) const DB_WRITE_BUFFER_SIZE_BYTES: u64 = 64 * 1024 * 1024;

// Column family names
pub(super) const CF_UTXO: &str = "cf_utxo";
pub(super) const CF_UTXO_BY_PUBKEY: &str = "cf_utxo_by_pubkey";
pub(super) const CF_PRODUCERS: &str = "cf_producers";
pub(super) const CF_EXIT_HISTORY: &str = "cf_exit_history";
pub(super) const CF_META: &str = "cf_meta";
pub(super) const CF_UNDO: &str = "cf_undo";
/// Phase 1 of UTXO storage consolidation: unique ID index for NFT/Pool/Asset
/// uniqueness checks. Mirrors utxo_store's `unique_id` CF.
/// Key: prefix(1B) + id(32B) -> empty. See `specs/utxo-storage-architecture.md`.
pub(super) const CF_UNIQUE_ID: &str = "cf_unique_id";

// Meta keys
pub(super) const META_CHAIN_STATE: &[u8] = b"chain_state";
pub(super) const META_PENDING_UPDATES: &[u8] = b"pending_updates";
pub(super) const META_LAST_APPLIED: &[u8] = b"last_applied";
pub(super) const META_EPOCH_PRODUCER_LIST: &[u8] = b"epoch_producer_list";
pub(super) const META_ACTIVE_PRODUCTION_LIST: &[u8] = b"active_production_list";
pub(super) const META_EPOCH_ATTESTED_SET: &[u8] = b"epoch_attested_set";
pub(super) const META_EPOCH_ATTESTATION_ACCUM: &[u8] = b"epoch_attestation_accum";
pub(super) const META_EPOCH_BLOCKS_PRODUCED: &[u8] = b"epoch_blocks_produced";
pub(super) const META_EPOCH_BOND_SNAPSHOT: &[u8] = b"epoch_bond_snapshot";
pub(super) const META_EPOCH_STATE: &[u8] = b"epoch_state";
pub(super) const META_EPOCH_STATE_VERSION: &[u8] = b"epoch_state_version";
pub(super) const META_CHAIN_COMMITMENT: &[u8] = b"chain_commitment";
pub(super) const META_CHAIN_COMMITMENT_TIP: &[u8] = b"chain_commitment_tip";
/// D.3 oracle sunset gradient state (warning/halt epoch tracking).
/// Persisted as bincode-serialized `OracleSunsetState`. Local
/// bookkeeping — NOT part of the consensus state root.
pub(super) const META_ORACLE_SUNSET_STATE: &[u8] = b"oracle_sunset_state";
/// AUDIT-P2-001: cached `last_update_height` for the oracle status RPC.
/// Written at every successful `OraclePrice` UTXO insert in the
/// aggregator; read by `getOracleStatus` to avoid an unbounded full-
/// UTXO-set scan on every unauthenticated RPC call. Stored as 8-byte
/// little-endian u64. NOT part of the consensus state root.
pub(super) const META_ORACLE_LAST_UPDATE_HEIGHT: &[u8] = b"oracle_last_update_height";
/// AUDIT-P1-001 (INC-I-156): set immediately BEFORE a destructive
/// rebuild-from-genesis wipes `cf_utxo`, deleted only after the trailing
/// `atomic_replace` succeeds. Its presence at any later moment means the wipe
/// committed but the replay did not finish — the durable ledger is a truncated
/// subset of the chain the persisted `chain_state` claims.
///
/// Stored as 16 bytes: target height (8B LE) ‖ unix start time (8B LE).
/// NOT part of the consensus state root. Deliberately in `CF_META`, which
/// `atomic_replace` does not iterate-delete (`writes.rs:181-186`), so the
/// marker survives the very operation that clears it explicitly — and survives
/// a `systemctl restart` in the middle of the replay window.
pub(super) const META_REBUILD_IN_PROGRESS: &[u8] = b"rebuild_in_progress";

/// Unified state database wrapping a single RocksDB instance.
pub struct StateDb {
    pub(super) db: rocksdb::DB,
    pub(super) utxo_count: AtomicU64,
    /// Shared LRU block cache referenced by every CF. Held on the struct so
    /// `metrics()` can query its real usage via `Cache::get_usage()` instead
    /// of summing per-CF property reads (INC-I-106 root-cause fix).
    pub(super) block_cache: rocksdb::Cache,
    /// Configured capacity of `block_cache` in bytes.
    pub(super) block_cache_capacity_bytes: u64,
}

/// Atomic write batch for a single block application.
///
/// All mutations within a block go into this batch. On `commit()`,
/// the entire batch is written atomically. If the batch is dropped
/// without committing, no changes are persisted.
///
/// Phase 3: BlockBatch is now the **sole** UTXO mutation path during
/// `apply_block`. All reads during block application use the overlay
/// methods (`get_utxo`, `contains_utxo`, `get_utxos_by_pubkey`,
/// `has_unique_id_check`) which check pending state first, then fall
/// through to committed state_db.
pub struct BlockBatch<'a> {
    pub(super) db: &'a StateDb,
    pub(super) batch: rocksdb::WriteBatch,
    pub(super) utxo_delta: i64,
    /// UTXOs added in this batch but not yet committed to DB.
    /// Needed for same-block-spend: TX2 spending an output created by TX1
    /// in the same block won't find it via db.get() (not committed yet).
    pub(super) pending_utxos: HashMap<Outpoint, UtxoEntry>,
    /// Outpoints removed in this batch (to avoid returning spent UTXOs from pending).
    pub(super) spent_in_batch: Vec<Outpoint>,
    /// Unique IDs added in this batch but not yet committed to disk.
    /// Enables same-block NFT/Pool/Asset uniqueness checks without
    /// reading from cf_unique_id (which hasn't been committed yet).
    /// Key: (output_type_discriminant, unique_id_hash).
    /// Phase 1 of UTXO storage consolidation (specs/utxo-storage-architecture.md).
    pub(super) pending_unique_ids: HashSet<(u8, [u8; 32])>,
    /// Unique IDs removed in this batch but not yet committed to disk.
    /// Phase 3: `has_unique_id_check` must return false for IDs that were
    /// spent in the current block, even if they still exist on disk.
    pub(super) removed_unique_ids: HashSet<(u8, [u8; 32])>,
}

/// The consistency canary — stored inside the same WriteBatch as state.
/// If this key exists and matches the chain_state, the DB is consistent.
#[derive(Debug, Clone)]
pub struct LastApplied {
    pub height: u64,
    pub hash: Hash,
    pub slot: u32,
}

impl LastApplied {
    pub(super) const SIZE: usize = 44; // 8 + 32 + 4

    pub(super) fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..8].copy_from_slice(&self.height.to_le_bytes());
        buf[8..40].copy_from_slice(self.hash.as_bytes());
        buf[40..44].copy_from_slice(&self.slot.to_le_bytes());
        buf
    }

    pub(super) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::SIZE {
            return None;
        }
        let height = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
        let hash = Hash::from_bytes(bytes[8..40].try_into().ok()?);
        let slot = u32::from_le_bytes(bytes[40..44].try_into().ok()?);
        Some(Self { height, hash, slot })
    }
}

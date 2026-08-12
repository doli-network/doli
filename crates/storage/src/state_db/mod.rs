//! Unified state database — single RocksDB with atomic WriteBatch per block.
//!
//! Merges UTXO set, producer set, and chain state into one database.
//! A crash at any point leaves the database in a consistent state:
//! either the full batch committed or none of it did.
//!
//! ## Column Families
//!
//! | CF | Key | Value |
//! |----|-----|-------|
//! | `cf_utxo` | Outpoint (36B) | UtxoEntry (bincode) |
//! | `cf_utxo_by_pubkey` | pubkey_hash(32B) ++ outpoint(36B) | 0x00 |
//! | `cf_producers` | pubkey_hash (32B) | ProducerInfo (bincode) |
//! | `cf_exit_history` | pubkey_hash (32B) | exit_height (8B LE) |
//! | `cf_meta` | string key | varies |
//! | `cf_undo` | height (8B LE) | UndoData (bincode) |
//! | `cf_undo` | 0x4D ++ height (9B) | MaintainerUndoSnapshot (bincode) — INC-I-174 |
//! | `cf_unique_id` | prefix(1B) + id(32B) | 0x00 |
//!
//! The two `cf_undo` families are distinguished by key LENGTH, so they can never
//! collide at any height. INC-I-174 chose this over a sixth field on `UndoData`
//! (which would make every pre-upgrade entry undecodable — bincode is
//! non-self-describing) and over a new column family (which a downgraded binary
//! could not open at all, because RocksDB requires every existing CF to be listed
//! at open time).

mod batch;
mod open;
mod queries;
#[cfg(test)]
mod rebuild_marker_tests;
#[cfg(test)]
mod tests;
mod types;
mod undo;
mod writes;

pub use types::{BlockBatch, LastApplied, MaintainerUndoSnapshot, StateDb, UndoData};

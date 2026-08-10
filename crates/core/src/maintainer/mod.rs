//! Maintainer Bootstrap System
//!
//! The DOLI auto-update system uses a decentralized maintainer set derived from
//! the blockchain itself. Unlike other blockchains that hardcode maintainer keys
//! in configuration files, DOLI derives its maintainer set from the first 5
//! registered producers.
//!
//! # Bootstrap Process
//!
//! 1. The first 5 producers to register become automatic maintainers
//! 2. After bootstrap, the maintainer set can be modified via on-chain transactions
//! 3. All changes require a k-of-n quorum of **distinct** current maintainers
//!
//! # Why This Design?
//!
//! | Aspect          | Hardcoded Keys      | DOLI Bootstrap       |
//! |-----------------|---------------------|----------------------|
//! | Source of truth | External config     | Blockchain itself    |
//! | Verification    | Trust the config    | Anyone can verify    |
//! | Changes         | Requires hard fork  | On-chain transactions|
//! | Auditability    | Check config        | Deterministic        |
//!
//! # Security Model
//!
//! - `MaintainerSet::threshold` DISTINCT signers required for any action
//!   (3 for a 4- or 5-member set — see [`set::MaintainerSet::calculate_threshold`])
//! - Minimum 3 maintainers must remain
//! - Maximum 5 maintainers allowed
//! - Slashed producers are automatically removed from maintainer set
//!
//! # INC-I-172 M2 — what the doc used to claim, and what is true now
//!
//! Before M2 this module doc said "3 of 5 signatures required for any action"
//! and advertised "Deterministic" auditability. Neither held:
//!
//! * `verify_multisig` counted signature ENTRIES, so three copies of ONE key
//!   satisfied a 3-of-5 threshold (AUDIT-P0-010). The distinct-signer counter is
//!   now the default; the entry-counting form survives ONLY as
//!   [`set::MaintainerSet::verify_multisig_legacy`], reachable exclusively below
//!   `NetworkParams::maintainer_derivation_activation_height` so replaying
//!   history stays bit-identical.
//! * `calculate_threshold(0)` returned 0, making `valid >= threshold` vacuous on
//!   an empty set (AUDIT-P1-010 / FM-02). It now returns
//!   [`MAINTAINER_THRESHOLD`], and every verifier additionally refuses an
//!   un-authorizable set outright — **UNGATED**, because an empty set has no
//!   legitimate authority to preserve at any height.
//! * The bootstrap derivation walked a `HashMap` and stable-sorted on
//!   `registered_at` alone, so tied genesis producers yielded a per-node random
//!   5-subset (AUDIT-P3-014). [`derive_canonical_maintainer_set`] sorts by the
//!   TOTAL order `(registered_at, pubkey_bytes)` and is the ONE derivation on the
//!   node path AT AND ABOVE the gate.
//!
//! It is NOT the only derivation in tree, and the earlier wording here ("is now
//! the ONE derivation") was false (M2 review F4). Three others survive, all
//! deliberately: the HashMap-ordered stable sort in
//! `bins/node/src/node/periodic.rs` and
//! `apply_block/governance.rs::derive_ad_hoc_maintainer_set`, both reachable
//! ONLY below the gate because they are frozen consensus history; and the
//! read-only CLI display helper `bins/node/src/commands/maintainer.rs`. The
//! `getMaintainerSet` RPC fallback was a fourth until the same review routed it
//! onto [`derive_canonical_maintainer_set`].

mod data;
mod derivation;
mod set;

#[cfg(test)]
mod tests;

pub use data::{MaintainerChangeData, ProtocolActivationData};
pub use derivation::{
    derive_canonical_maintainer_set, derive_maintainer_set, BlockchainReader, MaintainerChange,
};
pub use set::{MaintainerError, MaintainerSet, MaintainerSignature};

// ============================================================================
// Constants
// ============================================================================

/// Number of initial maintainers derived from first N registrations
pub const INITIAL_MAINTAINER_COUNT: usize = 5;

/// Required signatures for any maintainer action (3 of 5)
pub const MAINTAINER_THRESHOLD: usize = 3;

/// Minimum maintainers allowed (cannot remove below this)
pub const MIN_MAINTAINERS: usize = 3;

/// Maximum maintainers allowed (cannot add above this)
pub const MAX_MAINTAINERS: usize = 5;

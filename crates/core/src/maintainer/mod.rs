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

mod authmsg;
mod data;
mod derivation;
mod digest;
mod set;

#[cfg(test)]
mod tests;

pub use authmsg::{
    signing_message, signing_message_at, signing_message_legacy, signing_message_preimage,
    GOLDEN_AUTH_DIGEST_HEX, GOLDEN_AUTH_GENESIS_HASH, GOLDEN_AUTH_IS_ADD, GOLDEN_AUTH_PREIMAGE_HEX,
    GOLDEN_AUTH_TARGET_PUBKEY, GOLDEN_AUTH_VALID_BEFORE,
};
pub use data::{MaintainerChangeData, ProtocolActivationData};
pub use derivation::{
    derive_canonical_maintainer_set, derive_maintainer_set, BlockchainReader, MaintainerChange,
};
pub use digest::maintainer_set_digest;
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

// ----------------------------------------------------------------------------
// INC-I-173 M3 / F5 — maintainer-change payload bounds (AUDIT-P1-001)
//
// `AddMaintainer` / `RemoveMaintainer` are the first FEE-EXEMPT types whose
// payload decoder had no length bound: `MaintainerChangeData::from_bytes` is
// `bincode::deserialize().ok()` over an unbounded `Vec<MaintainerSignature>`
// plus an `Option<String>`. Above `NetworkParams::inc_i_173_activation_height`
// that is a zero-fee unbounded permanent chain write driving O(N) Ed25519
// verifies, re-paid on every future sync.
//
// These bounds are enforced ONLY at and above that height
// (`crates/core/src/validation/tx_types.rs::validate_maintainer_change_data`).
//
// # Why a RESTRICTIVE consensus rule may ride an ALREADY-CROSSED height
//
// (INC-I-173 M3a, AUDIT-P2-001. Read this before moving F5 to its own height,
// and before deploying anything on this branch.)
//
// These bounds only ever REJECT, so on the face of it they are the INC-I-054
// class: a new binary re-validating history could refuse a block the running
// binary accepted. `inc_i_173_activation_height` is `133_000` on testnet and
// the live tip is already past it, so "the gate is in the future" is NOT the
// argument — it is false there.
//
// The argument is that no block on ANY network can contain either type, on
// either side of the gate:
//
// * BELOW the gate — the fee predicate refuses them. Being unmineable IS the
//   INC-I-173 bug.
// * ABOVE the gate — mineability arrives ONLY with INC-I-173 M1, and M1 and
//   these bounds SHIP IN THE SAME BINARY (M3a is built on M1). So the only
//   binary that can mine either type is also a binary that bounds it. A node
//   re-validating the crossed window finds no maintainer transaction there to
//   re-judge.
//
// The invariant that must hold, stated so it can be checked rather than
// assumed: **on every network whose `inc_i_173_activation_height` is already
// crossed, no block in `[inc_i_173_activation_height, tip]` contains an
// `AddMaintainer` or a `RemoveMaintainer` at the moment this code first runs
// there.** Verified on testnet 2026-08-11 by scanning the crossed window
// (zero maintainer transactions; mempool empty; the deployed `doli-node` was
// built ~9h BEFORE M1 was committed and its `getMaintainerSet` publishes no
// digest, so it provably predates both changes).
//
// The one way to break it is OFF-PLAN and is forbidden for an independent
// reason: build and deploy M1 WITHOUT these bounds to a crossed-gate network,
// mine an over-sized payload, then upgrade. M1 alone already requires a
// synchronized stop-all/start-all (INV-8 / INC-I-062), and INC-I-173 M2 re-pins
// the testnet height ABOVE the tip measured immediately before pinning — either
// step alone closes the window. **Never ship M1 to a crossed-gate network
// without F5 in the same binary.**
//
// The three caps are mutually consistent, and that consistency is proven
// BEHAVIOURALLY against the real bincode encoder by
// `req_173_014_maximal_legal_payload_fits_under_the_outer_cap` — never by
// re-deriving bincode's encoding rules here. See the doc comment on
// [`MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES`].
// ----------------------------------------------------------------------------

/// Maximum `extra_data` byte length of an `AddMaintainer` / `RemoveMaintainer`
/// transaction, checked BEFORE the payload is decoded.
///
/// # Why 1024 leaves room for a MAXIMAL legal payload
///
/// A maximal legal `MaintainerChangeData` — the target key, a full
/// [`MAX_MAINTAINER_CHANGE_SIGNATURES`] signature vector and a `reason` of
/// exactly [`MAX_MAINTAINER_CHANGE_REASON_BYTES`] — encodes to **873 bytes**,
/// leaving **151 bytes of headroom** under this cap. If it did not fit, a
/// legitimate 5-of-5 rotation would satisfy both inner caps and still be
/// rejected by this one, i.e. be permanently unmineable — the exact INC-I-173
/// bug class.
///
/// That figure is prose, and prose drifts. What protects it is not this comment
/// but `req_173_014_maximal_legal_payload_fits_under_the_outer_cap`
/// (`crates/core/tests/inc_i_173_m3_payload_bounds.rs`), which builds the
/// maximal payload, runs it through the REAL encoder and asserts the result is
/// `<=` this cap. Add a field to the payload and that test fails, whatever this
/// comment says.
///
/// Historical note (M3 QA iteration 1, OBS-5): the figure was once stated as
/// 785, wrong by 88 bytes in the UNSAFE direction — it dropped the 8-byte length
/// prefix bincode writes ahead of `crypto::PublicKey` and `crypto::Signature`,
/// which are encoded as byte SEQUENCES (40 and 72 bytes, not 32 and 64). A
/// hand-derived restatement of bincode's rules was tried as the fix and then
/// removed at review iteration 1 (F3): it had no production consumer, and a
/// second copy of the encoder's rules kept only to check the first copy is the
/// duplication, not the cure.
pub const MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES: usize = 1024;

/// Maximum number of signature entries in a `MaintainerChangeData`.
///
/// This is [`MAX_MAINTAINERS`] because that is the PRINCIPLED bound, not a
/// magic number: `MaintainerSet::count_distinct_signers` counts a signature
/// only when its `pubkey` is a CURRENT member, and membership is capped at
/// [`MAX_MAINTAINERS`], so signature entry number 6 can never add a distinct
/// signer. Rejecting it removes no capability.
pub const MAX_MAINTAINER_CHANGE_SIGNATURES: usize = MAX_MAINTAINERS;

/// Maximum BYTE length of the optional `reason` string.
///
/// BYTES, never `char`s. `reason` is attacker-chosen free text on a FEE-EXEMPT
/// transaction, so the unit that must be bounded is the unit the chain pays for
/// — bytes written and re-read on every future sync — not user-visible
/// characters. A 256-`char` cap would admit a 1024-byte payload.
pub const MAX_MAINTAINER_CHANGE_REASON_BYTES: usize = 256;

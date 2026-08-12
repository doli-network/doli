//! INC-I-174 M1 / AUDIT-P1-001 — is this `cf_undo` record authority for THIS block?
//!
//! ## The property this file exists to enforce
//!
//! The five-lens M1 security audit converged 5/5 on one structural finding (SYS-001):
//! *a new authority record is trusted for its POSITION, never for its CONTENT-authenticity
//! or its BINDING to the block it describes.* [`super::plan_maintainer_rewind`] used to
//! promote a record to `Restore` on two facts alone — a record exists at height `h`, and
//! the block at `h` carries a rotation-typed transaction. Neither fact is about the record.
//!
//! What this file adds is a BINDING between the record and the block, checked with public,
//! unkeyed inputs. It detects staleness, cross-chain replay and in-place edits; it is NOT
//! authentication and does NOT detect tampering by an actor with data-dir write access. The
//! limit, and why it is accepted, are stated in full on [`check_snapshot_binding`]
//! (AUDIT-P3-401) — read that before citing this file as a defence against an attacker.
//!
//! ## Why the position check is not enough (the reachability that made it a P1)
//!
//! `plan_maintainer_rewind` resolves the block at `h` through
//! `BlockStore::get_block_by_height` → `CF_HEIGHT_INDEX`. Four production paths rewrite
//! that index through `BlockStore::put_block_canonical`, which writes `CF_HEIGHT_INDEX` +
//! `CF_HASH_TO_HEIGHT` and NOTHING else — no `apply_block`, and no refresh of the 9-byte
//! `cf_undo` family:
//!
//! * `crates/rpc/src/methods/backfill.rs` — `backfillFromPeer`, an ONLINE RPC;
//! * `bins/node/src/operations/restore.rs` — `doli-node restore`;
//! * `crates/storage/src/archiver.rs` — archive import;
//! * `crates/storage/src/block_store/writes.rs` — `rebuild_canonical_index`.
//!
//! So a LEGITIMATE operator recovery — routine on this fleet; INC-I-143 was a fleet-wide
//! snap-sync/backfill cascade — can leave a different, rotation-carrying block at `h` while
//! the record below it still describes the abandoned one. No data-dir write is required.
//! The position check then passes and the stale record installs through the SUCCESS exit
//! (`info!` + `maintainer_rewind_count += 1`): this host's release-verification trust root
//! becomes a member list that exists on no canonical chain — under INC-I-175, one still
//! holding the five bootstrap keys whose private halves are public — and the operator is
//! told the rewind succeeded.
//!
//! ## Why the check lives HERE and not in `validate_persisted_set`
//!
//! `storage::validate_persisted_set` is the WELL-FORMEDNESS gate, and it is deliberately
//! ONE function shared by `MaintainerState::load` and by the rewind
//! (REQ-174-SEC-001 AC-4: the two gates may not drift, and the load path must keep an
//! empty/unseeded root bootable). "Is this the record for this block?" is a question the
//! load path cannot even ask — it has no block. Folding it into the shared gate would
//! either break every fresh boot or force a second policy behind a flag, which is the
//! drift the shared gate exists to prevent. The binding check is therefore layered BEFORE
//! it, on the rewind path only.
//!
//! ## Pure by construction
//!
//! [`check_snapshot_binding`] takes the record, the hash of the block that NOW occupies the
//! height, and the genesis hash — no `Node`, no `&mut self`, no I/O. That is what lets the
//! `reason=` token of every refusal branch be pinned by a unit test in this file;
//! `bins/node` has no tracing-capture dev-dependency, so an integration test can only
//! observe the counters.

use super::{reason, MaintainerRewindPlan};

use crypto::Hash;
use doli_core::maintainer::maintainer_set_digest;
use storage::MaintainerUndoSnapshot;

/// Decide whether `snapshot` may be restored for `height`, given the block that NOW
/// occupies that height and the chain it belongs to.
///
/// Returns [`MaintainerRewindPlan::Restore`] only when all three bindings hold, and
/// otherwise the existing [`MaintainerRewindPlan::Unrestorable`] exit — so every refusal
/// is counted and announced by `commit_maintainer_rewind` exactly like the pre-existing
/// ones (REQ-174-005 AC-3, "no silent route").
///
/// The order is cheapest-and-most-fundamental first:
///
/// 1. **header** — was this written by a binary of this format generation? A record that
///    fails here is not one of ours, and reading its `set` field would be reading attacker-
///    or corruption-chosen bytes as a member list.
/// 2. **block hash** — the AUDIT-P1-001 closer. It compares the record against the BLOCK,
///    not against the index that pointed at the block, so it holds whatever rewrote the
///    canonical index.
/// 3. **set digest** — does the member list still match the one this record was filed
///    with? Recomputed from `set` and the genesis hash, so a member list edited in place no
///    longer matches its own record. Bound to the chain (`maintainer_set_digest` mixes the
///    genesis hash), so a record lifted from another network is refused as well.
///
/// A refusal is never a repair. A record that fails any check is discarded whole and the
/// LIVE trust root is kept: a de-duplicated, truncated or "corrected" authority record is
/// still an authority record nobody on the chain chose.
///
/// # What this function does NOT prove (AUDIT-P3-401)
///
/// It is **staleness and drift detection, not tamper detection, and it is not
/// authentication** — hence the name: it checks a BINDING, it does not authenticate anyone.
/// All three inputs are public and none is keyed: `MAGIC`/`VERSION` are compiled constants,
/// `canonical_block_hash` is recomputed from a block in the same data dir as the record, and
/// `maintainer_set_digest` is `BLAKE3(domain ‖ genesis_hash ‖ threshold ‖ sorted members)`
/// over a PUBLIC genesis hash with no node secret anywhere in the preimage.
///
/// So the checks catch: a FOSSIL record whose block is no longer the block at that height
/// (the AUDIT-P1-001 class, reachable with NO data-dir write at all, through the four
/// `put_block_canonical` writers listed above); a record captured for a DIFFERENT BLOCK; a
/// record lifted from ANOTHER CHAIN; a member list edited in place after capture; and a
/// record written by a different BINARY GENERATION. They do NOT catch an actor who can
/// WRITE `cf_undo`: that actor rewrites the member list and recomputes a matching
/// `block_hash` and `set_digest` in one BLAKE3 call, and this function returns `Restore`.
///
/// That residual is ACCEPTED under the existing threat model. The same write access reaches
/// `maintainer_state.bin`, which `crates/storage/src/lib.rs`
/// (`StorageError::MalformedPersistedValue`) documents as unsigned and attacker-writable
/// given data-dir access — and that file is the LIVE trust root, not a record consulted only
/// across a rewind. Editing it is a strictly shorter path to the same authority, so this
/// function is not the control standing between that actor and the trust root. Do not cite
/// these checks as authentication, tamper-proofing or integrity protection against an
/// attacker, and do not retire another control on the strength of them.
pub(super) fn check_snapshot_binding(
    height: u64,
    canonical_block_hash: Hash,
    genesis_hash: &[u8],
    snapshot: MaintainerUndoSnapshot,
) -> MaintainerRewindPlan {
    if !snapshot.header_is_valid() {
        return MaintainerRewindPlan::Unrestorable {
            height,
            token: reason::SNAPSHOT_HEADER_INVALID,
            reason: format!(
                "the undo snapshot at h={height} does not carry this format generation's \
                 header (magic={:?}, version={}); it was written by another binary \
                 generation, or the value is not a maintainer snapshot at all. It is \
                 discarded whole rather than decoded — this record decides which binary \
                 the auto-updater installs on this host",
                snapshot.magic, snapshot.version
            ),
        };
    }

    if snapshot.block_hash != canonical_block_hash {
        return MaintainerRewindPlan::Unrestorable {
            height,
            token: reason::SNAPSHOT_BLOCK_MISMATCH,
            reason: format!(
                "the undo snapshot at h={height} was captured for block {} but the block \
                 now canonical at that height is {}. A `cf_undo` record is authority for \
                 the BLOCK it was captured from, never for the height it is filed under: \
                 `put_block_canonical` (backfillFromPeer, restore, the archiver, \
                 rebuild_canonical_index) rewrites the height index WITHOUT re-running \
                 apply_block and WITHOUT refreshing this record. Restoring it would \
                 install a member list that exists on NO canonical chain. The live trust \
                 root is kept unchanged (AUDIT-P1-001)",
                snapshot.block_hash, canonical_block_hash
            ),
        };
    }

    let recomputed = maintainer_set_digest(&snapshot.set, genesis_hash);
    if recomputed != snapshot.set_digest {
        return MaintainerRewindPlan::Unrestorable {
            height,
            token: reason::SNAPSHOT_DIGEST_MISMATCH,
            reason: format!(
                "the undo snapshot at h={height} carries set_digest={} but its member list \
                 hashes to {} on this chain, so the set was altered after the record was \
                 written (or the record belongs to another chain). The live trust root is \
                 kept unchanged",
                hex::encode(snapshot.set_digest),
                hex::encode(recomputed)
            ),
        };
    }

    MaintainerRewindPlan::Restore {
        height,
        snapshot: Box::new(snapshot),
    }
}

#[cfg(test)]
mod tests {
    //! The `reason=` TOKEN of every branch, pinned directly.
    //!
    //! These are the only tests in the INC-I-174 suite that can name a token: the anchor
    //! is a log LINE, `bins/node` has no tracing-capture dev-dependency, and adding one
    //! would edit a non-test manifest. The integration siblings
    //! (`bins/node/tests/inc_i_174_snapshot_binding.rs`) prove the branch is REACHED
    //! through the real `execute_reorg`; these prove WHICH branch it is. Both halves are
    //! needed — the tokens are the machine-readable half of a security-graded fleet-wide
    //! grep, so a fix that routed every refusal through one token would defeat the signal
    //! while keeping every counter assertion green.

    use super::*;
    use doli_core::maintainer::MaintainerSet;

    const GENESIS: &[u8] = b"test-genesis-hash-32-bytes-long!";

    fn pubkey(seed: u8) -> crypto::PublicKey {
        crypto::PrivateKey::from_bytes([seed; 32]).public_key()
    }

    fn well_formed_set() -> MaintainerSet {
        MaintainerSet::with_members((0..5u8).map(|i| pubkey(0xC0 + i)).collect(), 42)
    }

    /// A record that is CONSISTENT on all three bindings for `block_hash` -- the shape a
    /// genuine capture has, and equally the shape a data-dir writer can manufacture
    /// (AUDIT-P3-401).
    fn consistent(block_hash: Hash) -> MaintainerUndoSnapshot {
        let set = well_formed_set();
        let digest = maintainer_set_digest(&set, GENESIS);
        MaintainerUndoSnapshot::new(block_hash, digest, set, 7)
    }

    fn token(plan: &MaintainerRewindPlan) -> Option<&'static str> {
        match plan {
            MaintainerRewindPlan::Unrestorable { token, .. } => Some(token),
            _ => None,
        }
    }

    #[test]
    fn audit_p1_001_a_record_for_another_block_is_refused_with_its_own_token() {
        let captured_for = Hash::from_bytes([0x11; 32]);
        let now_canonical = Hash::from_bytes([0x22; 32]);
        let plan = check_snapshot_binding(9, now_canonical, GENESIS, consistent(captured_for));

        assert_eq!(
            token(&plan),
            Some(reason::SNAPSHOT_BLOCK_MISMATCH),
            "AUDIT-P1-001: a record captured for a different block must take the \
             Unrestorable exit under its OWN token. Sharing `snapshot_refused` with the \
             well-formedness gate would tell an operator to go looking at a malformed \
             persisted set when the actual cause is that their last backfill/restore \
             re-pointed a height at another block — a different runbook entirely."
        );
        assert!(
            matches!(plan, MaintainerRewindPlan::Unrestorable { height: 9, .. }),
            "the offending height must be reported, or the anchor names no block"
        );
    }

    #[test]
    fn audit_p1_001_control_a_record_for_this_block_is_restored() {
        let block = Hash::from_bytes([0x33; 32]);
        let plan = check_snapshot_binding(9, block, GENESIS, consistent(block));

        match plan {
            MaintainerRewindPlan::Restore { height, snapshot } => {
                assert_eq!(height, 9);
                assert_eq!(snapshot.set.members.len(), 5);
                assert_eq!(snapshot.last_derived_height, 7);
            }
            other => panic!(
                "control: all three bindings hold, so the record MUST restore. A binding \
                 that refuses here has deleted REQ-174-003, the deliverable it was added \
                 to protect. Got {other:?}"
            ),
        }
    }

    #[test]
    fn sys_001_a_record_without_this_generations_header_is_refused() {
        let block = Hash::from_bytes([0x44; 32]);

        let mut wrong_magic = consistent(block);
        wrong_magic.magic = *b"XXXX";
        assert_eq!(
            token(&check_snapshot_binding(3, block, GENESIS, wrong_magic)),
            Some(reason::SNAPSHOT_HEADER_INVALID),
            "SYS-001: a value that is not one of these records must be discarded whole, \
             not decoded into a plausible member list"
        );

        let mut wrong_version = consistent(block);
        wrong_version.version = MaintainerUndoSnapshot::VERSION + 1;
        assert_eq!(
            token(&check_snapshot_binding(3, block, GENESIS, wrong_version)),
            Some(reason::SNAPSHOT_HEADER_INVALID),
            "SYS-001: a future format version is refused, never guessed at. This record \
             decides which binary the host installs, so 'decode what you can' is the wrong \
             failure direction."
        );
    }

    #[test]
    fn sys_001_a_set_edited_after_capture_no_longer_matches_its_own_digest() {
        let block = Hash::from_bytes([0x55; 32]);
        let mut edited = consistent(block);
        edited.set.members[0] = pubkey(0xFE);

        assert_eq!(
            token(&check_snapshot_binding(3, block, GENESIS, edited)),
            Some(reason::SNAPSHOT_DIGEST_MISMATCH),
            "SYS-001: swapping one member keeps the set well-formed, so \
             `validate_persisted_set` accepts it; only the captured digest sees the edit. \
             This pins IN-PLACE EDIT / corruption detection — NOT tamper detection \
             (AUDIT-P3-401): the digest is unkeyed over public inputs, so a writer who can \
             reach `cf_undo` recomputes it and this case passes. That is accepted because \
             the same writer reaches `maintainer_state.bin`, the LIVE root, directly."
        );
    }

    #[test]
    fn sys_001_a_record_lifted_from_another_chain_is_refused() {
        let block = Hash::from_bytes([0x66; 32]);
        let plan = check_snapshot_binding(
            3,
            block,
            b"a-completely-different-genesis!!",
            consistent(block),
        );

        assert_eq!(
            token(&plan),
            Some(reason::SNAPSHOT_DIGEST_MISMATCH),
            "SYS-001: `maintainer_set_digest` mixes the genesis hash precisely because the \
             mainnet and testnet bootstrap key arrays have been byte-identical \
             (AUDIT-P1-016). A record copied between chains must not bind on this one."
        );
    }
}

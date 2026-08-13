//! Deriving the maintainer set from the blockchain.
//!
//! INC-I-172 M2 (F2/F8): the node's genesis seed
//! (`bins/node/src/node/periodic.rs`, at and above the gate) and the full replay
//! ([`derive_maintainer_set`]) seat their members through the SAME canonical
//! total order — [`seated_registrations`], exposed as
//! [`derive_canonical_maintainer_set`] — so a node that walked the chain by a
//! different route reaches the same install trust root.

use crypto::PublicKey;

use super::{MaintainerChangeData, MaintainerSet, INITIAL_MAINTAINER_COUNT};

/// Trait for reading registration transactions from the blockchain
///
/// This is used by `derive_maintainer_set` to scan the chain and build
/// the maintainer set deterministically.
pub trait BlockchainReader {
    /// Get all registration public keys in order (earliest first)
    fn get_registrations_in_order(&self) -> Vec<(u64, PublicKey)>;

    /// Get all maintainer change transactions in order
    fn get_maintainer_changes(&self) -> Vec<(u64, MaintainerChange)>;

    /// Get all slashed producers, each with the height at which the slash landed.
    ///
    /// The height is REQUIRED (INC-I-172 M2 review F6). Without it a slash
    /// cannot be merged into the chronological action stream, and every slash
    /// ends up applied AFTER every Add/Remove — so a slash that chronologically
    /// preceded a legitimate re-add would wrongly undo that re-add.
    fn get_slashed_producers(&self) -> Vec<(u64, PublicKey)>;
}

/// A maintainer change event from the blockchain
#[derive(Clone, Debug)]
pub enum MaintainerChange {
    /// Add a new maintainer
    Add(MaintainerChangeData),
    /// Remove an existing maintainer
    Remove(MaintainerChangeData),
}

/// One entry in the chronological replay stream of [`derive_maintainer_set`].
///
/// Governance changes and slashes are merged into a SINGLE height-ordered
/// stream so that replay order is chronological rather than
/// "all changes, then all slashes".
enum ReplayAction {
    /// An `AddMaintainer` / `RemoveMaintainer` governance action.
    Change(MaintainerChange),
    /// A producer slash, which force-removes the key from the set.
    Slash(PublicKey),
}

/// The first [`INITIAL_MAINTAINER_COUNT`] registrations under the canonical
/// TOTAL order, de-duplicated, as `(pubkey, registered_at)`.
///
/// The order is `(registered_at ASC, pubkey_bytes ASC)`. It is TOTAL: no two
/// distinct keys compare equal, so the result never depends on the order the
/// caller enumerated its registrations in. That is what
/// `producers.all_producers()` (a `HashMap::values()` walk) plus a STABLE
/// `sort_by_key(registered_at)` could not give — every genesis producer is
/// stamped `registered_at == 0`, so the whole set was one tie group and `take(5)`
/// selected a per-node random 5-subset (AUDIT-P3-014).
///
/// A repeated pubkey takes ONE seat: a duplicate would silently cut the effective
/// quorum from 3 distinct keys to 2 while `threshold` stayed 3.
fn seated_registrations(registrations: &[(PublicKey, u64)]) -> Vec<(PublicKey, u64)> {
    let mut ordered: Vec<(PublicKey, u64)> = registrations.to_vec();
    ordered.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
    });

    let mut seated: Vec<(PublicKey, u64)> = Vec::with_capacity(INITIAL_MAINTAINER_COUNT);
    for (pubkey, registered_at) in ordered {
        if seated.len() >= INITIAL_MAINTAINER_COUNT {
            break;
        }
        if seated.iter().any(|(seen, _)| *seen == pubkey) {
            continue;
        }
        seated.push((pubkey, registered_at));
    }
    seated
}

/// The ONE canonical, replayable bootstrap derivation (INC-I-172 M2, REQ-172-005).
///
/// Takes a VALUE slice, never `ProducerInfo`: `crates/core` must keep its
/// no-edge-to-`storage` boundary (C-R4). Callers map
/// `ProducerInfo -> (public_key, registered_at)` at the call site.
///
/// `height` becomes `MaintainerSet::last_updated`. `.members` ORDER is
/// observable — it is serialized verbatim into `maintainer_state.bin` and
/// returned by the `getMaintainerSet` RPC — so "same set, different order" is a
/// real divergence, and the total order above pins it.
pub fn derive_canonical_maintainer_set(
    registrations: &[(PublicKey, u64)],
    height: u64,
) -> MaintainerSet {
    let members = seated_registrations(registrations)
        .into_iter()
        .map(|(pubkey, _)| pubkey)
        .collect();
    MaintainerSet::with_members(members, height)
}

/// Derive the maintainer trust root by REPLAYING block history up to a height.
///
/// The root is a pure function of `(genesis seed, every governance action at a
/// height <= up_to_height)` and of nothing else — never of live producer state.
/// A wiped or backfilled node that replays to the same `up_to_height` therefore
/// reaches the same root as a node that was online throughout, instead of
/// re-bootstrapping to the genesis five and silently re-arming a key that
/// governance removed (REQ-172-010).
///
/// Steps:
/// 1. Seed from the registrations at heights `<= up_to_height`, seated under the
///    canonical total order (same shape as [`derive_canonical_maintainer_set`]).
/// 2. Apply `AddMaintainer` / `RemoveMaintainer` / slash actions in a SINGLE
///    height-ordered stream, so a slash that chronologically preceded a
///    legitimate re-add is applied BEFORE it. The sort is stable and governance
///    changes are enqueued ahead of slashes, so within one height the reader's
///    own order is preserved and a change is applied before a slash at that
///    same height.
///
/// # Height gating (INC-I-172 M2 review F1)
///
/// Every authorization decision uses [`MaintainerSet::verify_multisig_at`] /
/// [`MaintainerSet::verify_multisig_excluding_at`] against the action's OWN
/// height, so each action is judged under the rule that was in force when the
/// fleet accepted it: entry counting below `activation_height`, DISTINCT signers
/// at and above it. Verifying ungated with the post-activation rule would REJECT
/// pre-activation actions the live fleet ACCEPTED, and the replaying node would
/// derive a different root than the node that stayed online — the exact
/// divergence `maintainer_derivation_activation_height` exists to prevent.
///
/// `activation_height` is
/// `NetworkParams::maintainer_derivation_activation_height`, passed as a plain
/// `u64` so `crates/core::maintainer` stays a leaf module.
///
/// # Status
///
/// **Zero production callers today.** Wiring this into the seed path (so that a
/// missing `maintainer_state.bin` replays instead of re-bootstrapping) is
/// INC-I-172 M3 / R1 — see `docs/.workflow/inc-i-172-M3-scope.md`.
///
/// That is a LOAD-BEARING fact, not a note: both signature arms below still build
/// the INC-I-176 LEGACY message unconditionally, so a production caller would make
/// this function disagree with the gated apply path above #22. It is enforced by
/// `crates/core/tests/inc_i_176_m2_derivation_tripwire.rs`, which fails when a
/// production caller appears while the arms are still legacy.
pub fn derive_maintainer_set<R: BlockchainReader>(
    reader: &R,
    up_to_height: u64,
    activation_height: u64,
) -> MaintainerSet {
    // Step 1: Bootstrap from the first 5 registrations at or below the bound,
    // canonically ordered.
    let candidates: Vec<(PublicKey, u64)> = reader
        .get_registrations_in_order()
        .into_iter()
        .filter(|(height, _)| *height <= up_to_height)
        .map(|(height, pubkey)| (pubkey, height))
        .collect();
    let seated = seated_registrations(&candidates);
    // Deterministic seed height: the LAST seated registration under the total
    // order, i.e. the greatest `registered_at` among the seated five.
    let seed_height = seated.iter().map(|(_, h)| *h).max().unwrap_or(0);
    let mut maintainer_set = MaintainerSet::with_members(
        seated.into_iter().map(|(pubkey, _)| pubkey).collect(),
        seed_height,
    );

    // Step 2: one chronological stream of governance changes AND slashes,
    // bounded by `up_to_height`. `sort_by_key` is stable, so equal heights keep
    // the order they were enqueued in: changes first, then slashes.
    let mut actions: Vec<(u64, ReplayAction)> = Vec::new();
    for (height, change) in reader.get_maintainer_changes() {
        if height <= up_to_height {
            actions.push((height, ReplayAction::Change(change)));
        }
    }
    for (height, pubkey) in reader.get_slashed_producers() {
        if height <= up_to_height {
            actions.push((height, ReplayAction::Slash(pubkey)));
        }
    }
    actions.sort_by_key(|(height, _)| *height);

    for (height, action) in actions {
        match action {
            ReplayAction::Change(MaintainerChange::Add(data)) => {
                // Verify under the rule in force AT `height`, not today's rule.
                //
                // INC-I-176 M1a (F14): routed through the ONE owned constructor
                // (`authmsg`) instead of re-deriving the format at this call
                // site. BIT-IDENTICAL to `data.signing_message(true)`, which is
                // now itself a delegate to the same function — no wire change,
                // no behaviour change, and the two arms can no longer drift
                // apart independently.
                //
                // INC-I-176 M2 SHIPPED THE GATE (#22,
                // `inc_i_176_auth_binding_activation_height`) AND THIS LINE DID
                // NOT MOVE. An earlier version of this comment said M2 would
                // convert it; that was wrong and is corrected here (QA F2 /
                // GAP-176-M2-02). M2 wired the ONE production verifier, in
                // `bins/node/src/node/apply_block/governance.rs`. This site
                // stays on the LEGACY arm.
                //
                // Why that is not a live defect, and what would make it one:
                // `derive_maintainer_set` has ZERO production callers. The
                // production paths (`crates/rpc/src/methods/governance.rs`,
                // `bins/node/src/node/periodic.rs`,
                // `crates/updater/src/trust_root.rs`) all call the DIFFERENT
                // function `derive_canonical_maintainer_set`, which seats by
                // registration order and verifies no signatures. Give this
                // function a production caller while this line is still legacy
                // and a replay-derived root would ACCEPT the legacy message and
                // REJECT the bound one above #22 — disagreeing with the apply
                // path, which is the trust-root fragmentation #22 exists to
                // avoid. CLAUDE.md's INC-I-075 lesson is explicit that
                // "currently unused" is never a reason to skip a gate, so the
                // reachability premise is not left as prose: it is enforced by
                // `crates/core/tests/inc_i_176_m2_derivation_tripwire.rs`,
                // which FAILS the moment a production caller appears.
                //
                // Converting this site is REQ-176-041, marked WON'T THIS RUN in
                // `docs/.workflow/milestone-progress.md`. Do not wire it here as
                // a drive-by. When it is done it becomes
                // `signing_message_at(genesis, true, &data.target, valid_before,
                // height, auth_binding_activation_height)` and needs a genesis
                // hash threaded in, which this leaf module does not have today.
                // `valid_before` arrives there as a PLAIN PARAMETER: the
                // payload carries no such field, and will not until M2.5 adds
                // it behind its own activation height and format
                // discriminator. Do not read it off `data` before then.
                let message = super::signing_message_legacy(true, &data.target);
                if maintainer_set.verify_multisig_at(
                    &data.signatures,
                    &message,
                    height,
                    activation_height,
                ) {
                    let _ = maintainer_set.add_maintainer(data.target, height);
                }
            }
            ReplayAction::Change(MaintainerChange::Remove(data)) => {
                // Verify signatures (excluding the target) under the rule in
                // force AT `height`. INC-I-176 M1a (F14): see the `Add` arm —
                // same owned constructor, same bit-identical bytes, same
                // parameter-not-field note. It is ALSO still on the LEGACY arm
                // after M2 for the same reason, is covered by the same tripwire,
                // and is the same deferred REQ-176-041: a conversion that fixed
                // only the `Add` arm would leave this one unbound.
                let message = super::signing_message_legacy(false, &data.target);
                if maintainer_set.verify_multisig_excluding_at(
                    &data.signatures,
                    &message,
                    &data.target,
                    height,
                    activation_height,
                ) {
                    let _ = maintainer_set.remove_maintainer(&data.target, height);
                }
            }
            ReplayAction::Slash(pubkey) => {
                maintainer_set.force_remove_maintainer(&pubkey, height);
            }
        }
    }

    maintainer_set
}

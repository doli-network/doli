//! Well-formedness policy for the persisted maintainer trust root.
//!
//! Split out of [`crate::maintainer`] so the FORMAT concern (magic, version tag, bincode
//! body, atomic save) and the AUTHORITY concern (is this set something a live code path
//! could have produced?) are separately readable. `crate::maintainer` answers "did these
//! bytes decode"; this module answers "may the result become this host's trust root",
//! which is the question AUDIT-P1-019 found nobody was asking.

use std::path::Path;

use doli_core::maintainer::{MaintainerSet, MAX_MAINTAINERS};

use crate::StorageError;

/// Refuse a persisted [`MaintainerSet`] that no live code path could have produced.
///
/// INC-I-172 M2, AUDIT-P1-019. `maintainer_state.bin` is node-local, unsigned and
/// attacker-writable given data-dir access, and M2 promoted it to the SOLE
/// `ProtocolActivation` authority above the maintainer-derivation gate. The decoder used
/// to restore `members` and `threshold` verbatim, so the single property M2 adds —
/// "threshold means DISTINCT signers" — was void on any host whose data directory an
/// adversary can write, silently: nothing was logged, `getMaintainerSet` reported
/// `enforced: true`, and the member list looked the right length.
///
/// The magic and the version tag are FORMAT discrimination, not authenticity. This is
/// the well-formedness check that has to stand in for the authenticity tag the file does
/// not have. It is node-local and not consensus-visible — the file is never gossiped,
/// never hashed, and absent from `ChainState::serialize_canonical` — so it needs NO
/// activation height.
///
/// Three refusals, each closing a distinct quorum collapse. They are applied in this
/// order because the size bound also bounds the cost of the duplicate scan:
///
/// 1. **More members than `MAX_MAINTAINERS`.** Both live derivations truncate to
///    `INITIAL_MAINTAINER_COUNT` (`maintainer/derivation.rs`, `node/periodic.rs`) and
///    `add_maintainer` refuses at the cap, so an over-long list is unreachable — and it
///    is the same collapse with a self-consistent threshold (11 members ⇒ threshold 6,
///    which an adversary supplying 6 keys clears alone).
/// 2. **Duplicate members.** `MaintainerSet::count_distinct_signers` iterates member
///    SLOTS, breaking after the first signature that matches the slot. Nothing bounds
///    the count per KEY, so `[K,K,K,K,K]` with `threshold: 3` is a 1-of-1 wearing a
///    3-of-5 costume. The comment at `set.rs:127-129` is true of the SIGNATURE vector
///    and false of the MEMBER vector; this is where the member vector is enforced.
/// 3. **An unreconciled threshold.** The genuine five with `threshold: 1` needs no key
///    theft at all: it downgrades an honest, freshly rotated quorum, and M1's
///    install-path containment compares KEYS ONLY, so it passes containment and reaches
///    the binary-install path.
///
/// **The EMPTY set is carved out on purpose.** `MaintainerSet::new` persists
/// `threshold: 0` while `calculate_threshold(0)` is `MAINTAINER_THRESHOLD` (3), so a
/// blanket reconciliation would refuse the state every fresh node starts from; and M1
/// deliberately keeps an EMPTIED root loadable (`inc_i_172_command_trust_root_test.rs`
/// `f3_an_emptied_on_chain_set_fails_closed_for_operator_commands`) so an attacked host
/// resolves to an unusable `OnChain` root instead of becoming unbootable. The carve-out
/// costs nothing: `MaintainerSet::is_authorizable` short-circuits on
/// `!members.is_empty()` and `TrustRoot::is_usable` requires `keys.len() >= threshold`,
/// so an empty set authorizes nothing at ANY threshold value.
///
/// Refuse, never repair. A deduplicated or threshold-corrected set is still an
/// ATTACKER-CHOSEN member list installed as this host's authority.
///
/// INC-I-174 (REQ-174-SEC-001) promoted this from `pub(crate)` to `pub` and re-exported
/// it at the crate root. `cf_undo` is now a second on-disk route by which bytes become
/// the trust root — the maintainer rewind in `bins/node/src/node/maintainer_rewind/`
/// restores a [`MaintainerSet`] from an undo record — and that route runs THIS function,
/// not a copy. `path` is only a label for the error message, so a caller whose source is
/// not a filesystem path may pass a descriptive pseudo-path such as
/// `cf_undo:maintainer_snapshot`.
pub fn validate_persisted_set(path: &Path, set: &MaintainerSet) -> Result<(), StorageError> {
    let malformed = |defect: String| StorageError::MalformedPersistedValue {
        file: path.display().to_string(),
        subject: "maintainer set",
        defect,
    };

    // The size bound is checked FIRST so it also bounds the cost of everything below it.
    // The duplicate scan is O(n²), the file is unauthenticated, and `bincode::deserialize`
    // puts no ceiling on a vector length, so a hand-written member list of 100_000 keys
    // would otherwise cost 10^10 comparisons at STARTUP. Rejecting on length first caps
    // the quadratic work at MAX_MAINTAINERS² = 25.
    if set.members.len() > MAX_MAINTAINERS {
        return Err(malformed(format!(
            "it holds {} members, above the maximum of {MAX_MAINTAINERS}. Every \
             derivation truncates to the initial count and add_maintainer refuses at the \
             maximum, so a longer list cannot have been derived from the chain",
            set.members.len()
        )));
    }

    for (i, member) in set.members.iter().enumerate() {
        if let Some(j) = set.members[..i].iter().position(|m| m == member) {
            return Err(malformed(format!(
                "member slots {j} and {i} hold the same key {}. A threshold counts \
                 DISTINCT signers by iterating member slots, so a duplicated key clears \
                 a {}-of-{} with ONE signature",
                member.to_hex(),
                set.threshold,
                set.members.len()
            )));
        }
    }

    // An empty set authorizes nothing at any threshold — see the carve-out above.
    if !set.members.is_empty() {
        let expected = MaintainerSet::calculate_threshold(set.members.len());
        if set.threshold != expected {
            return Err(malformed(format!(
                "it holds {} member(s) with a threshold of {}, but a {}-member set has a \
                 threshold of {expected}. An unreconciled threshold downgrades the quorum \
                 without touching the member list, so the set still looks correct",
                set.members.len(),
                set.threshold,
                set.members.len()
            )));
        }
    }

    Ok(())
}

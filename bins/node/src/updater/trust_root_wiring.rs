//! Composition-root resolution of the release-verification trust root (INC-I-172 F1).
//!
//! This is the ONLY place in the node that decides whether release verification runs
//! against the on-chain `MaintainerSet` or the compile-time bootstrap keys. Keeping
//! that decision in one function is the point of the change: the previous code
//! returned a bare `Vec<String>`, and `verification.rs` re-derived the decision from
//! "is the vector empty?", which conflated three different situations and resolved
//! all three by re-arming the compiled keys.
//!
//! The three situations, now distinguished:
//!
//! | on-chain members | `last_derived_height` | resolution                       |
//! |------------------|-----------------------|----------------------------------|
//! | non-empty, == the chain-derived five | any   | `TrustRoot::on_chain` (authoritative) |
//! | non-empty, != the chain-derived five | any   | `TrustRoot::on_chain(vec![])` → **fails closed** (M1 containment, AUDIT-P0-010) |
//! | empty            | 0                     | `TrustRoot::bootstrap` (never bootstrapped — REQ-172-005) |
//! | empty            | > 0                   | `TrustRoot::on_chain(vec![])` → **fails closed** (the attack case) |

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use doli_core::network::Network;
use tokio::sync::RwLock;
use tracing::error;
use updater::TrustRoot;

/// Load the persisted on-chain maintainer set at node startup.
///
/// INC-I-172 F5: a LOAD ERROR is FATAL. This file decides which keys may authorise a
/// binary update, so the node refuses to start rather than run with an empty trust
/// root — the previous `unwrap_or_default()` turned any unreadable file into a silent
/// empty root, which re-armed the compiled bootstrap keys fleet-wide and
/// simultaneously.
///
/// Two states are NOT errors and never reach this message:
/// - a MISSING file is `Ok(default())` — a fresh node is a legitimate state;
/// - a pre-INC-I-172 (unversioned) file is MIGRATED automatically, preserving the set
///   bit-for-bit (api-contract §9). No operator action is needed on upgrade day.
///
/// What remains fatal is a file that decodes as NEITHER layout: genuine damage, e.g. a
/// torn write. Guessing at the trust root is the one thing that must not happen.
pub fn load_maintainer_state(data_dir: &Path) -> Result<storage::MaintainerState> {
    storage::MaintainerState::load(data_dir).map_err(|e| {
        anyhow::anyhow!(
            "FATAL: cannot load the maintainer trust root: {e}\n  \
             This file decides which keys may authorise a binary update, so the node \
             refuses to start rather than run with an empty trust root.\n  \
             A file written by an older binary is migrated automatically, so this means \
             {}/maintainer_state.bin is damaged (truncated or partially written) rather \
             than merely old. Restore it from a backup, or remove it deliberately and let \
             the node re-derive the maintainer set from the chain.",
            data_dir.display()
        )
    })
}

/// Resolve the trust root from a loaded `MaintainerState`.
///
/// A thin adapter over [`TrustRoot::resolve`], which holds the actual decision — including
/// the AUDIT-P0-010 M1 containment and its `// M2:` lift condition. The decision lives in
/// the `updater` crate so that `doli upgrade` (`bins/cli`) reaches the SAME answer on the
/// same host; two separately-maintained copies of it is how the compiled keys stayed
/// authoritative on the CLI path after the node path was fixed (AUDIT-P1-012).
pub fn resolve_trust_root(state: &storage::MaintainerState, network: Network) -> TrustRoot {
    let keys: Vec<String> = state.set.members.iter().map(|m| m.to_hex()).collect();
    TrustRoot::resolve(
        keys,
        state.set.threshold,
        state.last_derived_height,
        network,
    )
}

/// Resolve the trust root for an OPERATOR-invoked `doli-node` command.
///
/// INC-I-172 F3. `doli-node upgrade`, `doli-node update verify` and
/// `doli-node update apply` all used to reach for `TrustRoot::bootstrap` through the
/// `verify_release_signatures` shim. That is wrong for this binary specifically: the
/// node runs ON the host that holds `maintainer_state.bin`, so the on-chain set is one
/// file read away, and consulting the compiled constants instead leaves the LEAKED keys
/// authoritative on every producer host through a single command — the exact claim M1
/// exists to retire. Operators reach for these commands precisely when the automatic
/// path starts refusing, so this is the path that matters most.
///
/// A load ERROR is propagated, not swallowed: the same fail-closed rule the node
/// applies at startup. A MISSING file resolves to Bootstrap via [`resolve_trust_root`],
/// which is correct for a genuinely unbootstrapped node and now warns loudly (F13).
///
/// The `doli` CLI in `bins/cli` deliberately does NOT use this: it is not the node
/// host, has no data directory of its own and no chain state, so `TrustRoot::bootstrap`
/// is the only root it can have. That is a stated limitation of the CLI path, not an
/// oversight — see the comment at its call site.
pub fn command_trust_root(data_dir: &Path, network: Network) -> Result<TrustRoot> {
    let state = load_maintainer_state(data_dir)?;
    let root = resolve_trust_root(&state, network);
    println!(
        "Trust root: {} ({} key(s), threshold {}, {})",
        root.provenance(),
        root.keys().len(),
        root.threshold(),
        network
    );
    Ok(root)
}

/// Build the closure the update service uses to resolve the CURRENT trust root.
///
/// It is resolved on every call, not cached, so a revocation that lands on-chain
/// reaches an update that is already in flight (F7(a)).
pub fn maintainer_trust_root_fn(
    maintainer_state: Arc<RwLock<storage::MaintainerState>>,
    network: Network,
) -> impl Fn() -> TrustRoot + Send + Sync + 'static {
    move || match maintainer_state.try_read() {
        Ok(state) => resolve_trust_root(&state, network),
        Err(_) => {
            // Could not read the trust root. "I could not check" is not "it is fine":
            // return an unusable OnChain root so verification refuses, rather than
            // silently degrading to the compiled bootstrap keys. The update check
            // runs every few hours and simply retries.
            error!(
                "Release trust root UNAVAILABLE: the maintainer state lock was held; refusing to \
                 verify rather than falling back to compiled bootstrap keys (INC-I-172 F1)"
            );
            TrustRoot::on_chain(Vec::new(), updater::REQUIRED_SIGNATURES)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use doli_core::maintainer::MaintainerSet;
    use updater::TrustRootProvenance;

    fn pubkey(seed: u8) -> crypto::PublicKey {
        crypto::PrivateKey::from_bytes([seed; 32]).public_key()
    }

    #[test]
    fn never_bootstrapped_node_uses_the_bootstrap_root() {
        let state = storage::MaintainerState::default();
        let root = resolve_trust_root(&state, Network::Mainnet);
        assert_eq!(root.provenance(), TrustRootProvenance::Bootstrap);
        assert!(root.is_usable());
    }

    /// The compiled bootstrap five, as `PublicKey`s.
    fn chain_derived_five(network: Network) -> Vec<crypto::PublicKey> {
        updater::bootstrap_maintainer_keys(network)
            .iter()
            .map(|k| crypto::PublicKey::from_hex(k).expect("compiled key must be valid hex"))
            .collect()
    }

    #[test]
    fn populated_set_is_authoritative_and_carries_its_own_threshold() {
        let members = chain_derived_five(Network::Mainnet);
        let state = storage::MaintainerState {
            version: storage::MAINTAINER_STATE_VERSION,
            set: MaintainerSet::with_members(members, 10),
            last_derived_height: 10,
        };
        let root = resolve_trust_root(&state, Network::Mainnet);
        assert_eq!(root.provenance(), TrustRootProvenance::OnChain);
        assert_eq!(root.keys().len(), 5);
        assert_eq!(root.threshold(), state.set.threshold);
        assert!(root.is_usable());
    }

    /// INC-I-196: a rotated set is the authority and carries its own keys. The M1
    /// containment emptied this root, which bricked auto-update fleet-wide.
    #[test]
    fn a_rotated_on_chain_set_is_authoritative_and_carries_its_own_keys() {
        let mut members = chain_derived_five(Network::Mainnet);
        members[0] = pubkey(200); // one maintainer swapped — one step of a rotation
        let state = storage::MaintainerState {
            version: storage::MAINTAINER_STATE_VERSION,
            set: MaintainerSet::with_members(members.clone(), 10),
            last_derived_height: 10,
        };
        let root = resolve_trust_root(&state, Network::Mainnet);
        assert_eq!(root.provenance(), TrustRootProvenance::OnChain);
        assert!(root.is_usable());
        let expected: Vec<String> = members.iter().map(|m| m.to_hex()).collect();
        assert_eq!(root.keys(), expected.as_slice());
    }

    #[test]
    fn emptied_set_fails_closed_and_never_becomes_bootstrap() {
        let mut set = MaintainerSet::with_members(vec![pubkey(1), pubkey(2), pubkey(3)], 10);
        set.members.clear();
        let state = storage::MaintainerState {
            version: storage::MAINTAINER_STATE_VERSION,
            set,
            last_derived_height: 4242,
        };
        let root = resolve_trust_root(&state, Network::Mainnet);
        assert_eq!(
            root.provenance(),
            TrustRootProvenance::OnChain,
            "an emptied on-chain set must stay OnChain — becoming Bootstrap is the F1 defect"
        );
        assert!(
            !root.is_usable(),
            "an emptied on-chain set must fail closed"
        );
    }
}

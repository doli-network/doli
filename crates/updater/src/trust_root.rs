//! Resolved release-verification trust root (INC-I-172 F1).
//!
//! Before this module, the release verifier took a bare `&[String]` of on-chain
//! maintainer keys and treated the EMPTY slice as "no on-chain state yet, use the
//! compile-time bootstrap keys". That single sentinel conflated three different
//! situations — "this node has never established an on-chain set", "the on-chain
//! set exists and is empty", and "the wire from the chain to the updater is not
//! connected" — and resolved all three by re-arming the compiled keys.
//!
//! `TrustRoot` makes the three distinguishable: the provenance is carried with the
//! keys, an empty on-chain root is representable, and only the composition root
//! (`bins/node/src/updater/trust_root_wiring.rs`) may choose `bootstrap`.

use std::collections::BTreeSet;
use std::fmt;

use doli_core::network::Network;
use tracing::{debug, error, warn};

use crate::constants::{bootstrap_maintainer_keys, REQUIRED_SIGNATURES};

/// Where a release-verification trust root came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustRootProvenance {
    /// Compile-time bootstrap keys. Used ONLY by a node that has never established
    /// an on-chain maintainer set (fresh install, un-upgraded node) and by the CLI.
    Bootstrap,
    /// The on-chain `MaintainerSet`. An EMPTY on-chain root is representable and
    /// FAILS CLOSED — it must NEVER silently become `Bootstrap`.
    OnChain,
}

impl fmt::Display for TrustRootProvenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrustRootProvenance::Bootstrap => write!(f, "Bootstrap"),
            TrustRootProvenance::OnChain => write!(f, "OnChain"),
        }
    }
}

/// A resolved trust root: keys + the threshold they must meet + where they came from.
///
/// Replaces the fail-open empty-`Vec<String>` sentinel (F1).
#[derive(Clone, Debug)]
pub struct TrustRoot {
    /// Lowercase hex Ed25519 public keys.
    keys: Vec<String>,
    /// Distinct signers required before this root authorises an install.
    threshold: usize,
    provenance: TrustRootProvenance,
}

impl TrustRoot {
    /// Compile-time bootstrap root for `network`; threshold = `REQUIRED_SIGNATURES`.
    pub fn bootstrap(network: Network) -> Self {
        Self {
            keys: bootstrap_maintainer_keys(network)
                .iter()
                .map(|k| (*k).to_string())
                .collect(),
            threshold: REQUIRED_SIGNATURES,
            provenance: TrustRootProvenance::Bootstrap,
        }
    }

    /// On-chain root. `keys` MAY be empty — that is the fail-closed case, not a
    /// fallback trigger.
    pub fn on_chain(keys: Vec<String>, threshold: usize) -> Self {
        Self {
            keys,
            threshold,
            provenance: TrustRootProvenance::OnChain,
        }
    }

    /// The keys that may authorise a release under this root.
    pub fn keys(&self) -> &[String] {
        &self.keys
    }

    /// Distinct signers required.
    pub fn threshold(&self) -> usize {
        self.threshold
    }

    /// Where the root came from.
    pub fn provenance(&self) -> TrustRootProvenance {
        self.provenance
    }

    /// A root can authorize an install only if it has at least `threshold` keys and
    /// `threshold >= 1`. An empty or sub-threshold root is NOT usable.
    ///
    /// `threshold >= 1` matters on its own: `MaintainerSet::calculate_threshold(0)`
    /// is 0, so a `valid >= threshold` test on a defaulted set would be vacuously
    /// satisfied by a release carrying zero valid signatures (FM-02).
    pub fn is_usable(&self) -> bool {
        self.threshold >= 1 && self.keys.len() >= self.threshold
    }

    /// Resolve a trust root from a host's persisted on-chain maintainer set.
    ///
    /// This is the ONE decision function. It lives in this crate rather than in the
    /// node's composition root because BOTH root-running binaries must reach the same
    /// answer on the same host: `doli-node upgrade` / `update verify` / `update apply`
    /// (`bins/node/src/updater/trust_root_wiring.rs`) and `doli upgrade`
    /// (`bins/cli/src/cmd_upgrade.rs`). A second, separately-maintained copy of this
    /// decision is how the compiled keys stayed authoritative on the `doli upgrade` path
    /// after the node path was fixed — AUDIT-P1-012.
    ///
    /// | on-chain members                     | `last_derived_height` | resolution |
    /// |--------------------------------------|-----------------------|------------|
    /// | non-empty, == the chain-derived five | any                   | `OnChain`, authoritative |
    /// | non-empty, != the chain-derived five | any                   | `OnChain`, EMPTY → fails closed (M1 containment) |
    /// | empty                                | 0                     | `Bootstrap` (never bootstrapped — REQ-172-005) |
    /// | empty                                | > 0                   | `OnChain`, EMPTY → fails closed (the attack case) |
    ///
    /// It never returns `Bootstrap` for a host that HAS an on-chain set. "I could not
    /// use it" must not become "use the leaked compiled keys".
    pub fn resolve(
        keys: Vec<String>,
        threshold: usize,
        last_derived_height: u64,
        network: Network,
    ) -> Self {
        if !keys.is_empty() {
            // ── M1 CONTAINMENT (AUDIT-P0-010) ───────────────────────────────────────
            // M1 promotes the on-chain `MaintainerSet` to the SOLE binary-install trust
            // root, with the compiled-constants fallback deleted. The governance
            // multisig guarding mutations of that set counts signature ENTRIES, not
            // DISTINCT signers (`doli_core::maintainer::MaintainerSet::verify_multisig`
            // / `verify_multisig_excluding`), so three byte-identical copies of ONE
            // valid Ed25519 signature satisfy a 3-of-5. Unguarded, one maintainer key
            // rewrites the install root of the whole fleet, permanently and unattended.
            //
            // The root fix is in that counter and is consensus-visible: a
            // user-submittable AddMaintainer/RemoveMaintainer tx reaches it and
            // `ProtocolActivation` acceptance depends on it (INV-12 Q1 YES / Q2 YES /
            // Q3 NO), so it requires an activation height — which M1 does not have.
            //
            // What IS node-local is the LINK from that counter to install authority,
            // and the link is exactly this function: nothing on a consensus path calls
            // it. So M1 severs the link instead of fixing the counter — an on-chain set
            // is install-authoritative only while it is still the chain-derived
            // bootstrap five. Any mutation, legitimate or forged, fails CLOSED.
            //
            // M2: DELETE this guard when the distinct-signer governance counter
            // activates at its activation height. Until then a legitimate rotation also
            // refuses, and that is the accepted cost: M1 cannot tell a legitimate
            // rotation from one forged with duplicate signatures, because the code that
            // would tell them apart is the code being deferred. (AUDIT-P1-013
            // independently shows an on-chain rotation cannot survive a single block
            // today, so this guard removes no capability that exists.)
            if !is_chain_derived_bootstrap_set(&keys, network) {
                error!(
                    "TRUST_ROOT_CONTAINED: the on-chain maintainer set ({} key(s), threshold {}, \
                     derived at height {}) is NOT the chain-derived bootstrap set for {:?}. \
                     Release verification will refuse every release on this host until the set \
                     matches again. It will NOT fall back to the compiled bootstrap keys. Until \
                     the distinct-signer governance multisig activates (INC-I-172 M2), a mutated \
                     maintainer set cannot be told apart from one forged with duplicate \
                     signatures from a single key (AUDIT-P0-010), so it is refused.",
                    keys.len(),
                    threshold,
                    last_derived_height,
                    network
                );
                return Self::on_chain(Vec::new(), threshold);
            }

            debug!(
                "Release trust root: on-chain maintainer set ({} keys, threshold {}, derived at \
                 height {})",
                keys.len(),
                threshold,
                last_derived_height
            );
            // M2: the threshold is taken from `MaintainerSet.threshold` verbatim.
            // Reconciling it with `REQUIRED_SIGNATURES` means changing
            // `MaintainerSet::calculate_threshold`, which feeds the GOVERNANCE multisig
            // and therefore changes which AddMaintainer / RemoveMaintainer /
            // ProtocolActivation transactions take effect (INV-12 Q1 YES / Q2 YES /
            // Q3 NO) — that needs an activation height, which M1 does not have.
            Self::on_chain(keys, threshold)
        } else if last_derived_height == 0 {
            // This host has never established an on-chain maintainer set: a fresh
            // install, or a node that has not yet reached the bootstrap height. It keeps
            // verifying exactly as it does today (REQ-172-005).
            //
            // F13: this is `warn!`, not `debug!`, because the branch is ALSO how a host
            // silently returns to the compiled constants. A missing `maintainer_state.bin`
            // is `Ok(default())`, and a fresh node is indistinguishable from a WIPED one —
            // and this project's runbooks wipe data dirs routinely (chain-reset,
            // cascade-recovery "full-wipe + snap"). Until the set is re-derived, this host
            // verifies binaries against the exposed compiled keys, so the transition must
            // be visible in a default `warn`-level log and greppable for monitoring — the
            // fixed token below is the grep anchor.
            warn!(
                "TRUST_ROOT_BOOTSTRAP: release verification is using the COMPILE-TIME bootstrap \
                 keys for {:?}. This host has no on-chain maintainer set on disk (no \
                 maintainer_state.bin, or one that has never been derived). Expected on a fresh \
                 install; if this node was previously synced, its data directory was wiped or \
                 maintainer_state.bin was deleted, and the leaked compiled constants are \
                 authoritative again until the set is re-derived from the chain.",
                network
            );
            Self::bootstrap(network)
        } else {
            // The set EXISTED (it was derived at a real height) and is now empty. That is
            // the attack case, not a fresh node: fail closed rather than handing authority
            // back to the compiled keys.
            error!(
                "Release trust root UNAVAILABLE: the on-chain maintainer set was derived at \
                 height {} but is now EMPTY. Release verification will refuse every release \
                 until the set is restored. This host will NOT fall back to the compiled \
                 bootstrap keys (INC-I-172 F1).",
                last_derived_height
            );
            Self::on_chain(Vec::new(), threshold)
        }
    }
}

/// Is this on-chain membership still exactly the chain-derived bootstrap five?
///
/// Compared as a SET of lowercase hex keys: order-insensitive and case-insensitive
/// (`PublicKey::to_hex` is lowercase, but nothing enforces that on a hand-written file).
///
/// Why order-insensitive, precisely (updated 2026-08-10, INC-I-172 M2): member order is
/// not a stable property, so it must not gate installs. **Below**
/// `maintainer_derivation_activation_height` the chain derivation stable-sorts a
/// `HashMap` iteration with no pubkey tiebreak (AUDIT-P3-014), which is outright
/// non-deterministic across nodes. **At and above** the gate,
/// `derive_canonical_maintainer_set` imposes the total order
/// `(registered_at, pubkey_bytes)` and IS deterministic — but a set-based comparison is
/// still the correct shape here: this function has no chain height available (it is
/// called from both `doli upgrade` and the node's updater service), so it cannot know
/// which side of the gate produced the set it is reading, and a persisted
/// `maintainer_state.bin` may have been seeded before the crossing.
///
/// The comparison target is [`bootstrap_maintainer_keys`], which on mainnet and testnet is
/// byte-identical to the first five genesis producers — exactly what
/// `Node::maybe_bootstrap_maintainer_set` derives from the chain. On a bootstrap-mode
/// chain (devnet) it is not, so a devnet host with a derived set resolves an unusable root
/// and does not auto-update: fail-closed, and devnet has no signed releases.
fn is_chain_derived_bootstrap_set(keys: &[String], network: Network) -> bool {
    let expected: BTreeSet<String> = bootstrap_maintainer_keys(network)
        .iter()
        .map(|k| k.to_ascii_lowercase())
        .collect();
    let found: BTreeSet<String> = keys.iter().map(|k| k.to_ascii_lowercase()).collect();
    found == expected
}

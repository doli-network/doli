// INC-I-172 M1 security audit, AUDIT-P1-012 — `doli upgrade` must resolve the ON-CHAIN
// trust root held on this host, not pin the compiled bootstrap keys.
// REQ-172-001 (Must), REQ-172-006 (Must).
//
// THE DEFECT this locks shut: `bins/cli/src/cmd_upgrade.rs` ran as root on producer
// hosts and used `updater::TrustRoot::bootstrap(network)` — the compiled array whose
// matching PRIVATE keys are committed in a public repository — while the host's own
// `maintainer_state.bin` sat one file read away. M1 made the on-chain set authoritative
// on the `doli-node` paths; leaving `doli` on the constants means every revocation is
// undone by typing the other binary's name. The audit calls this a one-command
// revocation bypass on the path the docs call the remediation path.
//
// ============================================================================
// OUTPUT CONTRACT
// ============================================================================
// TWO subjects, deliberately:
//
//   (A) BEHAVIOURAL — `updater::TrustRoot::resolve(keys, threshold, height, network)`,
//       the shared decision `cmd_upgrade` now calls. This is a pure function in a
//       library crate, so it is testable directly and is where the real assertions live.
//
//   (B) STRUCTURAL — the SOURCE of `cmd_upgrade`, via `include_str!`. `doli` is a binary
//       crate with no lib target, so `cmd_upgrade` itself is unreachable from an
//       integration test, and running it end to end needs GitHub plus a writable root
//       install target. The convention is already used by
//       bins/cli/tests/inc_i_172_upgrade_verify_blocks_test.rs. It asserts only the
//       WIRING: that the fn reaches (A) and no longer reaches `TrustRoot::bootstrap`
//       directly.
//
// ENUMERATION OF OBSERVABLE OUTPUTS.
//   (A) return value only — O1 provenance, O2 keys, O3 is_usable(). No mutable params,
//       no store, no side channel except `tracing` records (DECLARED UNASSERTED: every
//       fact logged is already in O1..O3).
//   (B) the source text itself — O4 wiring.
//
// CODE PATHS of (A):
//   P1: keys == the chain-derived bootstrap five   -> OnChain, usable
//   P2: keys != the chain-derived five             -> OnChain, EMPTY, unusable [AUDIT-P0-010]
//   P3: keys empty, last_derived_height == 0       -> Bootstrap, usable (REQ-172-005)
//   P4: keys empty, last_derived_height > 0        -> OnChain, EMPTY, unusable
//
// INPUT PARTITIONS: the four paths above, plus both networks that HAVE a compiled five
// (Mainnet, Testnet) on the branch that matters most (P2), so the guard cannot be keyed
// to one network by accident.
//
// Declared limitation: (B) is an assertion over source text. A restructuring that keeps
// the property but moves the landmark will fail it; the message says to re-anchor rather
// than delete. It also cannot prove the resolved root is the one PASSED to the verifier —
// that ordering property is already covered by inc_i_172_upgrade_verify_blocks_test.rs.
// ============================================================================

use doli_core::Network;
use updater::{TrustRoot, TrustRootProvenance};

const SRC: &str = include_str!("../src/cmd_upgrade.rs");

fn chain_derived_five(network: Network) -> Vec<String> {
    updater::bootstrap_maintainer_keys(network)
        .iter()
        .map(|k| (*k).to_string())
        .collect()
}

/// Body of `cmd_upgrade`, from its signature to the end of the file. Brace counting is
/// not usable — the body is full of `println!` format strings containing `{}`.
fn cmd_upgrade_body() -> &'static str {
    const SIG: &str = "pub(crate) async fn cmd_upgrade(";
    let start = SRC.find(SIG).unwrap_or_else(|| {
        panic!(
            "`cmd_upgrade` signature not found in bins/cli/src/cmd_upgrade.rs. If the fn \
             was renamed or its parameter list reformatted, re-anchor SIG here — do NOT \
             delete this test: AUDIT-P1-012 still requires `doli upgrade` to verify \
             against this host's on-chain trust root."
        )
    });
    &SRC[start + SIG.len()..]
}

/// REQ-172-001 (Must). GREEN-lock.
/// Acceptance: an intact, chain-derived maintainer set is install-authoritative for
/// `doli upgrade` too. The fix must not turn "has an on-chain set" into "cannot upgrade".
/// [P1 -> O1, O2, O3]
#[test]
fn the_chain_derived_five_resolves_to_a_usable_on_chain_root() {
    for network in [Network::Mainnet, Network::Testnet] {
        let keys = chain_derived_five(network);
        let root = TrustRoot::resolve(keys.clone(), 3, 4242, network);

        assert_eq!(root.provenance(), TrustRootProvenance::OnChain);
        assert!(
            root.is_usable(),
            "{network:?}: the honest on-chain set must still authorise `doli upgrade`"
        );
        assert_eq!(root.keys(), keys.as_slice());
    }
}

/// REQ-172-011 (Must), REQ-196-001 (Must).
/// Acceptance: a rotated maintainer set is authoritative on the CLI path and carries its
/// own keys — and still never degrades to the compiled keys.
///
/// This assertion INVERTED at INC-I-196. `doli upgrade` re-resolves on every invocation,
/// so the M1 containment made it the FIRST thing to break after the INC-I-175 rotation:
/// `Insufficient signatures: 0/3` against a zero-key root, on a correctly signed release.
/// [P2 -> O1, O2, O3]
#[test]
fn a_rotated_set_is_authoritative_and_never_becomes_bootstrap() {
    for network in [Network::Mainnet, Network::Testnet] {
        let mut keys = chain_derived_five(network);
        keys[0] = "11".repeat(32); // one maintainer swapped — one step of a rotation

        let root = TrustRoot::resolve(keys.clone(), 3, 4242, network);

        assert_eq!(
            root.provenance(),
            TrustRootProvenance::OnChain,
            "{network:?}: an on-chain root must stay OnChain — becoming Bootstrap hands \
             authority back to the publicly exposed compiled keys"
        );
        assert_eq!(
            root.keys(),
            keys.as_slice(),
            "{network:?}: the CLI must verify against the host's own on-chain members"
        );
        assert!(
            root.is_usable(),
            "{network:?}: reaching this membership already required `threshold` DISTINCT \
             on-chain maintainer signatures. Refusing it is what bricked `doli upgrade` \
             fleet-wide after the INC-I-175 rotation (INC-I-196)."
        );
    }
}

/// REQ-172-005 (Must). GREEN-lock.
/// Acceptance: a genuinely unbootstrapped host still gets the bootstrap root, so a fresh
/// install can be upgraded; a set that EXISTED and is now empty still fails closed.
/// [P3, P4 -> O1, O3]
#[test]
fn only_a_genuinely_unbootstrapped_host_reaches_the_compiled_keys() {
    let fresh = TrustRoot::resolve(Vec::new(), 0, 0, Network::Mainnet);
    assert_eq!(fresh.provenance(), TrustRootProvenance::Bootstrap);
    assert!(fresh.is_usable(), "a fresh host must still be upgradable");

    let emptied = TrustRoot::resolve(Vec::new(), 3, 4242, Network::Mainnet);
    assert_eq!(
        emptied.provenance(),
        TrustRootProvenance::OnChain,
        "a set derived at a real height and now empty is the attack case, not a fresh host"
    );
    assert!(!emptied.is_usable());
}

/// REQ-172-001 (Must). RED before the fix.
/// Acceptance: `cmd_upgrade` no longer pins the compiled bootstrap keys, and does reach
/// the shared resolver with a data directory. This is an ABSENCE assertion, so it is
/// weak on its own — it is paired with the behavioural tests above, which is where the
/// property actually lives.
/// [B -> O4]
#[test]
fn cmd_upgrade_resolves_the_host_trust_root_instead_of_pinning_bootstrap() {
    let body = cmd_upgrade_body();

    assert!(
        !body.contains("TrustRoot::bootstrap"),
        "`cmd_upgrade` still calls `TrustRoot::bootstrap` directly. `doli upgrade` runs \
         as root ON the node host, so the on-chain maintainer set is one file read away; \
         pinning the compiled array (whose private keys are committed in a public repo) \
         makes this command a one-command bypass of every revocation the `doli-node` path \
         honours — AUDIT-P1-012. Bootstrap must be reached only through the \
         genuinely-unbootstrapped branch inside `TrustRoot::resolve`.\n--- body ---\n{body}"
    );
    assert!(
        body.contains("resolve_upgrade_trust_root("),
        "`cmd_upgrade` must resolve the trust root from this host's data directory. If \
         the helper was renamed, re-anchor this assertion — do not delete it."
    );
    assert!(
        body.contains("data_dir"),
        "`cmd_upgrade` must consult a node data directory to find maintainer_state.bin"
    );
}

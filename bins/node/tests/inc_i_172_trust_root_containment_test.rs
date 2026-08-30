// INC-I-196 — the M1 CONTAINMENT IS DELETED. This file now pins its replacement.
// REQ-172-001 (Must), REQ-172-005 (Must), REQ-172-011 (Must), REQ-196-001 (Must).
//
// WHY THIS FILE CHANGED. INC-I-172 M1 made an on-chain maintainer set
// install-authoritative ONLY while it was still byte-equal to the compiled bootstrap
// five. That guard was always explicitly temporary — its own comment read "M2: DELETE
// this guard when the distinct-signer governance counter activates at its activation
// height."
//
// That counter is live. `MaintainerSet::count_distinct_signers`
// (`crates/core/src/maintainer/set.rs:130`) counts each member at most once, and
// `verify_multisig_at` routes to it whenever `height >= activation_height`. On mainnet
// that height is `maintainer_derivation_activation_height = 172_000`
// (`crates/core/src/network_params/defaults.rs:273`), crossed ~159k blocks ago. The
// premise the guard rested on — "a mutated set cannot be told apart from one forged with
// duplicate signatures from a single key" — has been FALSE since h=172,000.
//
// Leaving the guard in place cost an outage: the INC-I-175 mainnet key rotation
// (h=331,442 -> 331,457, verified BY that distinct-signer counter) made the on-chain set
// differ from the compiled array, so every host holding `maintainer_state.bin` resolved a
// ZERO-KEY trust root and refused every release regardless of how it was signed.
//
// So the contract inverts: a mutated on-chain set is now the AUTHORITY, because reaching
// that state already required `threshold` DISTINCT maintainer signatures on-chain. What
// does NOT change is the fail-closed half — see
// `both_empty_set_paths_still_fail_closed_or_bootstrap`.
//
// ============================================================================
// OUTPUT CONTRACT
// ============================================================================
// Function under test:
//   `doli_node::updater::resolve_trust_root(&storage::MaintainerState, Network)
//        -> updater::TrustRoot`
//
// ENUMERATION OF OBSERVABLE OUTPUTS.
//   - return value     : O1 provenance, O2 keys, O3 threshold, O4 is_usable().
//   - mutable params   : NONE (shared ref + Copy).
//   - persistent store : NONE — this function does no I/O. (`command_trust_root` reads
//                        the data dir; that is covered by
//                        inc_i_172_command_trust_root_test.rs.)
//   - side channel     : `tracing` records only. DECLARED UNASSERTED — every fact
//                        logged is already in O1..O4.
//
//   O1: provenance  — must NEVER become Bootstrap on a host that HAS an on-chain set.
//   O2: keys        — exactly the on-chain members; empty ONLY on the emptied-set path.
//   O3: threshold   — carried verbatim from the set (api-contract G5).
//   O4: is_usable() — the single bit that decides whether a release can install.
//
// CODE PATHS (of `resolve_trust_root`), AFTER the containment is deleted:
//   P1: members non-empty                        -> OnChain, USABLE, carries its members
//   P2: members empty, last_derived_height == 0   -> Bootstrap, usable (REQ-172-005)
//   P3: members empty, last_derived_height > 0    -> OnChain, EMPTY, NOT usable
//
// INPUT PARTITIONS on P1 — the same shapes the deleted guard refused, now accepted
// because each one requires `threshold` distinct on-chain signers to reach:
//   I1: one member SWAPPED        (Remove+Add — one step of a rotation)
//   I2: one member REMOVED        (set of 4; governance floor is MIN_MAINTAINERS=3)
//   I3: one member ADDED          (set of 6)
//   I4: ALL FIVE members replaced (the completed INC-I-175 rotation — the live mainnet
//                                  state as of h=331,457)
//   I5: the five, REORDERED       -> accepted (order was never a security property)
//   I6: the five, UPPERCASE hex   -> accepted (nothing enforces lowercase on disk)
//   Network partition: asserted on Mainnet and Testnet so the behaviour cannot be keyed
//   to one network by accident.
//
// DELIBERATELY NOT ASSERTED HERE — a below-floor set (1 or 2 members). It is unreachable
// today and is tracked separately; see `a_below_floor_set_is_structurally_unreachable`.
// ============================================================================

use doli_core::maintainer::MaintainerSet;
use doli_core::Network;
use doli_node::updater::resolve_trust_root;
use updater::TrustRootProvenance;

fn attacker_key(seed: u8) -> crypto::PublicKey {
    crypto::PrivateKey::from_bytes([seed; 32]).public_key()
}

/// The five keys the compiled bootstrap array carries for `network`. After the INC-I-196
/// cutover this is the ROTATED five on mainnet, not the genesis producers — the two are no
/// longer the same thing, which is the whole point of INC-I-175.
fn compiled_five(network: Network) -> Vec<crypto::PublicKey> {
    updater::bootstrap_maintainer_keys(network)
        .iter()
        .map(|k| crypto::PublicKey::from_hex(k).expect("a compiled bootstrap key must be hex"))
        .collect()
}

fn state_with(members: Vec<crypto::PublicKey>, height: u64) -> storage::MaintainerState {
    storage::MaintainerState {
        version: storage::MAINTAINER_STATE_VERSION,
        set: MaintainerSet::with_members(members, height),
        last_derived_height: height,
    }
}

fn assert_authoritative(root: &updater::TrustRoot, members: &[crypto::PublicKey], label: &str) {
    assert_eq!(
        root.provenance(),
        TrustRootProvenance::OnChain,
        "{label}: a host WITH an on-chain set must resolve OnChain — becoming Bootstrap \
         would hand authority back to the compiled keys (the F1 defect)"
    );
    assert!(
        root.is_usable(),
        "{label}: reaching this on-chain state already required `threshold` DISTINCT \
         maintainer signatures (count_distinct_signers, live since the derivation \
         activation height). Refusing it is an OUTAGE, not a containment — that is \
         exactly the INC-I-196 fleet-wide brick."
    );
    let expected: Vec<String> = members.iter().map(|m| m.to_hex()).collect();
    assert_eq!(
        root.keys(),
        expected.as_slice(),
        "{label}: the root must carry the on-chain members verbatim"
    );
}

/// REQ-172-001 (Must). GREEN-lock.
/// Acceptance: an on-chain set equal to the compiled five is install-authoritative.
/// [P1 -> O1, O2, O3, O4]
#[test]
fn a_set_matching_the_compiled_five_is_a_usable_on_chain_root() {
    for network in [Network::Mainnet, Network::Testnet] {
        let members = compiled_five(network);
        let state = state_with(members.clone(), 4242);

        let root = resolve_trust_root(&state, network);

        assert_authoritative(&root, &members, &format!("{network:?}"));
        assert_eq!(
            root.threshold(),
            state.set.threshold,
            "{network:?}: the root carries the set's own threshold (api-contract G5)"
        );
    }
}

/// REQ-196-001 (Must). **THE REPRODUCTION TEST — RED before the fix.**
/// Acceptance: an on-chain set that DIFFERS from the compiled array is the authority and
/// carries its own keys. Before the fix every shape below resolved to a zero-key root and
/// refused all releases; I4 is the live mainnet state that bricked the fleet.
/// [P1, I1/I2/I3/I4 -> O1, O2, O4]
#[test]
fn a_rotated_on_chain_set_is_authoritative_and_carries_its_own_keys() {
    let five = compiled_five(Network::Mainnet);

    let mut swapped = five.clone(); // I1
    swapped[0] = attacker_key(200);

    let mut removed = five.clone(); // I2
    removed.pop();

    let mut added = five.clone(); // I3
    added.push(attacker_key(201));

    let fully_rotated: Vec<_> = (210u8..215).map(attacker_key).collect(); // I4

    for (label, members) in [
        ("one member swapped", swapped),
        ("one member removed", removed),
        ("one member added", added),
        ("all five rotated (the INC-I-175 end state)", fully_rotated),
    ] {
        let state = state_with(members.clone(), 331_457);
        let root = resolve_trust_root(&state, Network::Mainnet);

        assert_authoritative(&root, &members, label);
    }
}

/// REQ-172-011 (Must).
/// Acceptance: resolution reads MEMBERSHIP, not encoding. Neither member order nor hex
/// case may change which keys a host trusts.
/// [P1, I5, I6 -> O2, O4]
#[test]
fn resolution_is_insensitive_to_member_order_and_hex_case() {
    let mut reordered = compiled_five(Network::Mainnet);
    reordered.reverse();
    let root = resolve_trust_root(&state_with(reordered.clone(), 7), Network::Mainnet);
    assert_authoritative(&root, &reordered, "reordered members");

    // The same five, hand-written in uppercase hex. Built through the string form because
    // `PublicKey::to_hex` always lowercases.
    let members: Vec<crypto::PublicKey> = updater::bootstrap_maintainer_keys(Network::Mainnet)
        .iter()
        .map(|k| {
            crypto::PublicKey::from_hex(&k.to_ascii_uppercase())
                .expect("uppercase hex must still decode")
        })
        .collect();
    let root = resolve_trust_root(&state_with(members.clone(), 7), Network::Mainnet);
    assert_authoritative(&root, &members, "uppercase hex members");
}

/// REQ-172-005 (Must), REQ-172-011 (Must). **REGRESSION LOCK — do not weaken.**
/// Acceptance: deleting the containment must not touch either empty-set path. A
/// never-bootstrapped node keeps the bootstrap root (or a fresh install could never be
/// upgraded); a set that EXISTED and is now empty still fails closed and must NEVER
/// degrade to the compiled keys.
/// [P2, P3 -> O1, O4]
#[test]
fn both_empty_set_paths_still_fail_closed_or_bootstrap() {
    let fresh = storage::MaintainerState::default();
    let root = resolve_trust_root(&fresh, Network::Mainnet);
    assert_eq!(root.provenance(), TrustRootProvenance::Bootstrap);
    assert!(root.is_usable(), "a fresh node must still be upgradable");

    let mut emptied = state_with(compiled_five(Network::Mainnet), 4242);
    emptied.set.members.clear();
    let root = resolve_trust_root(&emptied, Network::Mainnet);
    assert_eq!(
        root.provenance(),
        TrustRootProvenance::OnChain,
        "a set derived at a real height and now empty is the attack case, not a fresh node"
    );
    assert!(
        !root.is_usable(),
        "an emptied set must authorise nothing — and must NOT fall back to the compiled \
         bootstrap keys (INC-I-172 F1)"
    );
    assert!(root.keys().is_empty());
}

/// REQ-196-002 (Should). Documents what the deleted containment was INCIDENTALLY
/// protecting, and why nothing replaces it here.
///
/// `TrustRoot::is_usable()` is `threshold >= 1 && keys.len() >= threshold`, and
/// `MaintainerSet::calculate_threshold(1) == 1`. So a ONE-member on-chain set would
/// resolve to a usable 1-of-1 install root. The containment refused that shape as a side
/// effect of refusing every non-bootstrap shape.
///
/// It is not reachable:
///   * governance removal is floored — `can_remove()` is `members.len() > MIN_MAINTAINERS`
///     (`set.rs:65`, MIN_MAINTAINERS = 3), so `RemoveMaintainer` stops at 3 members;
///   * the ONLY sub-floor path is `ReplayAction::Slash` -> `force_remove_maintainer`
///     (`crates/core/src/maintainer/derivation.rs:269`), which fires on double-production
///     slashing and therefore requires a maintainer to ALSO be a bonded producer.
///
/// INC-I-175 severed exactly that dual role: M1-M5 are signing-only wallets, never
/// registered as producers and never bonded. So the slash path cannot target them.
///
/// This test pins the floor it depends on. If MIN_MAINTAINERS drops, or a maintainer is
/// ever also registered as a producer, the 1-of-1 shape becomes reachable and this
/// decision must be revisited.
#[test]
fn a_below_floor_set_is_structurally_unreachable() {
    assert_eq!(
        doli_core::maintainer::MIN_MAINTAINERS,
        3,
        "the governance removal floor is what keeps a 1-of-1 install root unreachable"
    );

    let mut three = compiled_five(Network::Mainnet);
    three.truncate(3);
    let set = MaintainerSet::with_members(three, 1);
    assert!(
        !set.can_remove(),
        "governance must not be able to remove below MIN_MAINTAINERS — if it can, a \
         below-floor trust root is reachable without any slashing"
    );
}

// INC-I-172 M1 security audit, AUDIT-P0-010 — the M1 CONTAINMENT.
// REQ-172-001 (Must), REQ-172-011 (Must).
//
// WHY THIS FILE EXISTS. M1 promotes the on-chain `MaintainerSet` to the SOLE
// binary-install trust root and deletes the compiled-constants fallback. The governance
// multisig that guards mutations of that set counts signature ENTRIES, not DISTINCT
// signers (`crates/core/src/maintainer.rs::verify_multisig`), so three byte-identical
// copies of ONE valid Ed25519 signature satisfy a 3-of-5 and ONE key rewrites the
// install root of the whole fleet, permanently and unattended.
//
// The root fix is in that counter and is consensus-visible (a user-submittable
// AddMaintainer tx reaches it; `ProtocolActivation` acceptance depends on it), so it
// needs an activation height and belongs to M2. What is node-local — and what this file
// pins — is the LINK from that counter to install authority. It lives entirely in
// `resolve_trust_root`, which has zero consensus consumers.
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
//   O2: keys        — empty when contained; exactly the five when accepted.
//   O3: threshold   — carried verbatim from the set (M2 reconciles it, api-contract G5).
//   O4: is_usable() — the single bit that decides whether a release can install.
//
// CODE PATHS (of `resolve_trust_root`):
//   P1: members == the chain-derived bootstrap five   -> OnChain, USABLE
//   P2: members != the chain-derived five (mutated)   -> OnChain, NOT usable [CONTAINMENT]
//   P3: members empty, last_derived_height == 0       -> Bootstrap, usable (REQ-172-005)
//   P4: members empty, last_derived_height > 0        -> OnChain, NOT usable
//
// INPUT PARTITIONS on P2 — every shape the defective ENTRY counter can produce:
//   I1: one member SWAPPED           (Remove+Add from a single key — the P0 sequence)
//   I2: one member REMOVED           (a lone RemoveMaintainer, set of 4)
//   I3: one member ADDED             (a lone AddMaintainer, set of 6)
//   I4: ALL members attacker-held    (the end state of the takeover)
//   I5: a SINGLE attacker member     (the AUDIT-P1-010 empty-set/zero-signature outcome)
//   I6: the five, REORDERED          -> must still be ACCEPTED (P1). The derivation
//                                       stable-sorts a HashMap iteration with no pubkey
//                                       tiebreak (AUDIT-P3-014), so member ORDER is not
//                                       a security property and must not gate installs.
//   I7: the five, UPPERCASE hex      -> must still be ACCEPTED (P1). Nothing enforces
//                                       lowercase on a hand-written file.
//   Network partition: Mainnet and Testnet both have a compiled five; asserted on both
//   so the guard cannot be keyed to one network by accident.
// ============================================================================

use doli_core::maintainer::MaintainerSet;
use doli_core::Network;
use doli_node::updater::resolve_trust_root;
use updater::TrustRootProvenance;

fn attacker_key(seed: u8) -> crypto::PublicKey {
    crypto::PrivateKey::from_bytes([seed; 32]).public_key()
}

/// The five keys `Node::maybe_bootstrap_maintainer_set` derives from the chain on
/// mainnet and testnet — byte-identical to the compiled bootstrap array by construction
/// (`crates/core/src/genesis.rs:90`).
fn chain_derived_five(network: Network) -> Vec<crypto::PublicKey> {
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

/// REQ-172-001 (Must). GREEN-lock.
/// Acceptance: the untouched, chain-derived maintainer set is still install-authoritative
/// on both networks. The containment must not brick the honest path.
/// [P1, I6-negative -> O1, O2, O3, O4]
#[test]
fn the_chain_derived_five_is_still_a_usable_on_chain_root() {
    for network in [Network::Mainnet, Network::Testnet] {
        let members = chain_derived_five(network);
        let state = state_with(members.clone(), 4242);

        let root = resolve_trust_root(&state, network);

        assert_eq!(
            root.provenance(),
            TrustRootProvenance::OnChain,
            "{network:?}: an intact on-chain set must stay the authority"
        );
        assert!(
            root.is_usable(),
            "{network:?}: the honest set must still authorise releases — a containment \
             that refuses everything is an outage, not a fix"
        );
        let expected: Vec<String> = members.iter().map(|m| m.to_hex()).collect();
        assert_eq!(root.keys(), expected.as_slice());
        assert_eq!(
            root.threshold(),
            state.set.threshold,
            "{network:?}: the root carries the set's own threshold (api-contract G5 \
             defers reconciliation to M2)"
        );
    }
}

/// REQ-172-011 (Must). RED before the containment.
/// Acceptance: a maintainer set that is no longer the chain-derived five authorises
/// NOTHING. This is the AUDIT-P0-010 outcome — the state one maintainer key can reach
/// today through the entry-counting multisig — and it must not become install authority.
/// [P2, I1/I2/I3/I4/I5 -> O1, O2, O4]
#[test]
fn a_set_that_is_not_the_chain_derived_five_authorises_nothing() {
    let five = chain_derived_five(Network::Mainnet);

    let mut swapped = five.clone(); // I1
    swapped[0] = attacker_key(200);

    let mut removed = five.clone(); // I2
    removed.pop();

    let mut added = five.clone(); // I3
    added.push(attacker_key(201));

    let all_attacker: Vec<_> = (210u8..215).map(attacker_key).collect(); // I4
    let lone_attacker = vec![attacker_key(220)]; // I5

    for (label, members) in [
        ("one member swapped", swapped),
        ("one member removed", removed),
        ("one member added", added),
        ("every member attacker-held", all_attacker),
        ("a single attacker member", lone_attacker),
    ] {
        let state = state_with(members, 9_000);
        let root = resolve_trust_root(&state, Network::Mainnet);

        assert_eq!(
            root.provenance(),
            TrustRootProvenance::OnChain,
            "{label}: the contained root must stay OnChain — becoming Bootstrap would \
             hand authority back to the leaked compiled keys (the F1 defect)"
        );
        assert!(
            !root.is_usable(),
            "{label}: M1 makes this set the SOLE install authority, and the multisig \
             guarding it counts signature ENTRIES not distinct signers — so one key can \
             produce exactly this state. It must authorise nothing until the M2 \
             distinct-signer counter activates (AUDIT-P0-010)."
        );
        assert!(
            root.keys().is_empty(),
            "{label}: a contained root must carry no keys at all"
        );
    }
}

/// REQ-172-011 (Must).
/// Acceptance: the containment compares MEMBERSHIP, not encoding. Member order is not a
/// security property (the derivation has no pubkey tiebreak — AUDIT-P3-014) and hex case
/// is not either, so neither may flip a healthy fleet to "refuse every release".
/// [P1, I6, I7 -> O4]
#[test]
fn the_containment_is_insensitive_to_member_order_and_hex_case() {
    let mut reordered = chain_derived_five(Network::Mainnet);
    reordered.reverse();
    let root = resolve_trust_root(&state_with(reordered, 7), Network::Mainnet);
    assert!(
        root.is_usable(),
        "member ORDER must not gate installs: the chain derivation stable-sorts a \
         HashMap iteration with no pubkey tiebreak, so the order is not stable and a \
         reordering would be an availability bug across the whole fleet"
    );

    // The same five, hand-written in uppercase hex. Built through the string form
    // because `PublicKey::to_hex` always lowercases.
    let upper: Vec<String> = updater::bootstrap_maintainer_keys(Network::Mainnet)
        .iter()
        .map(|k| k.to_ascii_uppercase())
        .collect();
    let members: Vec<crypto::PublicKey> = upper
        .iter()
        .map(|k| crypto::PublicKey::from_hex(k).expect("uppercase hex must still decode"))
        .collect();
    assert!(
        resolve_trust_root(&state_with(members, 7), Network::Mainnet).is_usable(),
        "hex CASE must not gate installs"
    );
}

/// REQ-172-005 (Must). GREEN-lock.
/// Acceptance: the containment does not touch the two empty-set paths. A never-bootstrapped
/// node keeps the bootstrap root (or a fresh install could never be upgraded); a set that
/// EXISTED and is now empty still fails closed.
/// [P3, P4 -> O1, O4]
#[test]
fn the_containment_leaves_both_empty_set_paths_unchanged() {
    let fresh = storage::MaintainerState::default();
    let root = resolve_trust_root(&fresh, Network::Mainnet);
    assert_eq!(root.provenance(), TrustRootProvenance::Bootstrap);
    assert!(root.is_usable(), "a fresh node must still be upgradable");

    let mut emptied = state_with(chain_derived_five(Network::Mainnet), 4242);
    emptied.set.members.clear();
    let root = resolve_trust_root(&emptied, Network::Mainnet);
    assert_eq!(
        root.provenance(),
        TrustRootProvenance::OnChain,
        "a set derived at a real height and now empty is the attack case, not a fresh node"
    );
    assert!(!root.is_usable());
}

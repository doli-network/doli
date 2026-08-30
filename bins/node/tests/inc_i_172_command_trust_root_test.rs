// INC-I-172 M1 review pass 1, [F3] MAJOR — the node's OWN operator commands must
// verify against the on-chain maintainer set held on their host, not the compiled
// bootstrap keys.
// REQ-172-001 (Must), REQ-172-006 (Must).
//
// WHY THIS FILE EXISTS. `doli-node upgrade`, `doli-node update verify` and
// `doli-node update apply` all reached the compiled constants through the
// `verify_release_signatures` shim. M1's headline claim is that the LEAKED compiled
// keys are no longer authoritative — and on every producer host they still were,
// through the one command an operator reaches for when the automatic path starts
// refusing. The three commands are network-bound end to end, so what is behavioural
// here is the decision they now share: `command_trust_root`.
//
// ============================================================================
// OUTPUT CONTRACT
// ============================================================================
// Function under test:
//   `doli_node::updater::command_trust_root(data_dir: &Path, network: Network)
//        -> anyhow::Result<TrustRoot>`
//
// ENUMERATION OF OBSERVABLE OUTPUTS.
//   - return value     : Result. O1 (discriminant), O2 (the TrustRoot: provenance,
//                        keys, threshold, is_usable).
//   - mutable params   : NONE (both shared refs).
//   - persistent store : the data dir is READ. A legacy file is migrated (re-saved) by
//                        `MaintainerState::load`, which is that function's contract and
//                        is asserted in crates/storage/tests; not re-asserted here.
//   - side channel     : one `println!` naming the provenance, and `tracing` records.
//                        DECLARED UNASSERTED — stdout capture would pin operator
//                        wording, and every fact printed is in O2.
//
//   O1: Result discriminant  — Ok / Err (an unreadable trust root is fatal, as at boot).
//   O2: TrustRoot            — provenance / keys / threshold. `provenance == Bootstrap`
//                              on a host that HAS an on-chain set is the F3 defect.
//
// CODE PATHS:
//   P1: data dir holds the chain-derived on-chain set  -> OnChain, that set's keys
//   P2: data dir holds no maintainer_state.bin         -> Bootstrap (REQ-172-005)
//   P3: data dir holds an EMPTIED set (derived once)   -> OnChain, unusable (fail closed)
//   P4: data dir holds an undecodable file             -> Err (fatal, never a default)
//   P5: data dir holds a ROTATED on-chain set          -> OnChain, that set's keys, USABLE
//       (INC-I-196; see inc_i_172_trust_root_containment_test.rs)
//
// INPUT PARTITIONS:
//   I1: an on-chain set that does NOT contain any compiled bootstrap key — the case
//       that separates "reads the file" from "reads the constants". A set that happened
//       to contain them would make both implementations look identical. On I1 a
//       `TrustRoot::bootstrap` implementation returns the five COMPILED keys, while the
//       correct code returns the file's OWN keys; both are usable, so the discriminator
//       is WHICH keys came back, not whether the root authorises.
// ============================================================================

use doli_core::maintainer::MaintainerSet;
use doli_core::Network;
use doli_node::updater::command_trust_root;
use updater::TrustRootProvenance;

fn pubkey(seed: u8) -> crypto::PublicKey {
    crypto::PrivateKey::from_bytes([seed; 32]).public_key()
}

/// The five keys the chain derivation produces on mainnet/testnet.
fn chain_derived_five(network: Network) -> Vec<crypto::PublicKey> {
    updater::bootstrap_maintainer_keys(network)
        .iter()
        .map(|k| crypto::PublicKey::from_hex(k).expect("a compiled bootstrap key must be hex"))
        .collect()
}

/// REQ-172-001 (Must). RED before this fix.
/// Acceptance: with an on-chain set on disk, the operator commands resolve THAT set from
/// the file — provenance OnChain, the file's own keys and the file's own threshold —
/// rather than reaching for the compiled constants.
/// [P1 -> O1, O2]
#[test]
fn f3_operator_commands_use_the_on_chain_set_on_this_host() {
    let dir = tempfile::tempdir().unwrap();
    let members = chain_derived_five(Network::Mainnet);
    let mut state = storage::MaintainerState::default();
    state
        .update(
            MaintainerSet::with_members(members.clone(), 2),
            4242,
            dir.path(),
        )
        .expect("writing the on-chain set must succeed");

    let root = command_trust_root(dir.path(), Network::Mainnet)
        .expect("a readable maintainer_state.bin must resolve");

    assert_eq!(
        root.provenance(),
        TrustRootProvenance::OnChain,
        "the node binary runs on the host that holds maintainer_state.bin. Resolving \
         Bootstrap here leaves the LEAKED compiled keys authoritative on every producer \
         host through one command — that is the F3 defect."
    );
    let expected: Vec<String> = members.iter().map(|m| m.to_hex()).collect();
    assert_eq!(root.keys(), expected.as_slice());
    assert!(
        root.is_usable(),
        "the intact chain-derived set must authorise"
    );
}

/// REQ-172-001 (Must), REQ-196-001 (Must).
/// Acceptance: on a host whose on-chain set differs from the compiled array, the operator
/// commands use THAT set — and still never reach the compiled constants. This is the input
/// on which "reads the file" and "reads the constants" differ most, so it is the one that
/// tells the two implementations apart.
///
/// The usability assertion INVERTED at INC-I-196: the M1 containment refused this shape,
/// which turned the INC-I-175 rotation into a fleet-wide refusal of every release. The
/// no-compiled-keys discriminator below is unchanged and must stay.
/// [P5, I1 -> O1, O2]
#[test]
fn f3_a_rotated_on_chain_set_is_used_and_never_degrades_to_the_compiled_keys() {
    let dir = tempfile::tempdir().unwrap();
    let members = vec![pubkey(101), pubkey(102), pubkey(103)];
    let mut state = storage::MaintainerState::default();
    state
        .update(
            MaintainerSet::with_members(members.clone(), 2),
            4242,
            dir.path(),
        )
        .expect("writing the on-chain set must succeed");

    let root = command_trust_root(dir.path(), Network::Mainnet)
        .expect("a readable maintainer_state.bin must resolve");

    assert_eq!(root.provenance(), TrustRootProvenance::OnChain);
    let expected: Vec<String> = members.iter().map(|m| m.to_hex()).collect();
    assert_eq!(
        root.keys(),
        expected.as_slice(),
        "the operator commands must use the on-chain members verbatim"
    );
    assert!(
        root.is_usable(),
        "reaching this on-chain membership already required `threshold` DISTINCT \
         maintainer signatures (count_distinct_signers, live at and above the derivation \
         activation height). Refusing it is the INC-I-196 brick, not a containment."
    );

    // The discriminator: no compiled bootstrap key may authorise anything here.
    for compiled in updater::bootstrap_maintainer_keys(Network::Mainnet) {
        assert!(
            !root.keys().iter().any(|k| k.eq_ignore_ascii_case(compiled)),
            "a compiled bootstrap key ({}...) is inside the resolved root. A release \
             signed by the compiled constants would be accepted on a host that has its \
             own on-chain set.",
            &compiled[..16.min(compiled.len())]
        );
    }
}

/// REQ-172-005 (Must). GREEN-lock.
/// Acceptance: a node that has never established an on-chain set still resolves the
/// bootstrap root, so a fresh install can still be upgraded. The fix must not turn
/// "no set yet" into "cannot upgrade".
/// [P2 -> O1, O2]
#[test]
fn f3_a_host_with_no_maintainer_state_still_gets_the_bootstrap_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = command_trust_root(dir.path(), Network::Mainnet)
        .expect("a missing file is a legitimate fresh node, not an error");
    assert_eq!(root.provenance(), TrustRootProvenance::Bootstrap);
    assert!(
        root.is_usable(),
        "a fresh node must still be able to upgrade"
    );
}

/// REQ-172-011 (Must). RED before this fix.
/// Acceptance: an EMPTIED on-chain set fails closed for the operator commands too —
/// it must not hand authority back to the compiled keys.
/// [P3 -> O2]
#[test]
fn f3_an_emptied_on_chain_set_fails_closed_for_operator_commands() {
    let dir = tempfile::tempdir().unwrap();
    let mut set = MaintainerSet::with_members(vec![pubkey(111), pubkey(112)], 8);
    set.members.clear();
    let mut state = storage::MaintainerState::default();
    state
        .update(set, 9_000, dir.path())
        .expect("writing the emptied set must succeed");

    let root = command_trust_root(dir.path(), Network::Mainnet).expect("the file is readable");
    assert_eq!(
        root.provenance(),
        TrustRootProvenance::OnChain,
        "an emptied set that was derived at a real height is the ATTACK case, not a \
         fresh node; becoming Bootstrap here is fail-open"
    );
    assert!(!root.is_usable(), "an emptied root must authorise nothing");
}

/// REQ-172-011 (Must).
/// Acceptance: an undecodable trust-root file is FATAL for the operator commands, the
/// same as at boot. Guessing at the trust root is the one thing that must not happen —
/// and "I could not read it" must never resolve to the compiled keys.
/// [P4 -> O1]
#[test]
fn f3_an_undecodable_trust_root_file_is_fatal_not_a_fallback() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("maintainer_state.bin"), [0xFFu8; 7]).unwrap();

    let err = command_trust_root(dir.path(), Network::Mainnet).expect_err(
        "an undecodable maintainer_state.bin must abort the command, not silently \
         resolve to the compiled bootstrap keys",
    );
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("maintainer"),
        "the error must name what could not be loaded; got: {msg:?}"
    );
}

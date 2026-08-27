//! INC-I-176 **M2** — shared node-level harness for the gate-wiring evidence.
//!
//! OUTPUT CONTRACT: N/A — fixture module. It asserts nothing about the system
//! under test; the only assertions here are FIXTURE PRECONDITIONS (e.g. "three
//! distinct signers were actually found"), which fail the setup rather than the
//! claim.
//! INPUT PARTITIONS: N/A — fixture module.
//!
//! WHY A SHARED FIXTURE. `inc_i_176_m2_gate_wiring.rs` and
//! `inc_i_176_m2_domain_separation.rs` make statements about the SAME production
//! call site under the SAME gate. If each built its own node harness and its own
//! gate literals, a drift between them would read as a disagreement between tests
//! rather than as a contract break. One harness, one place. The split itself
//! exists for the 800-line test-file budget (CLAUDE.md rule 19), not by accident.
//!
//! ---------------------------------------------------------------------------
//! THE HARNESS HAZARD, AND HOW IT IS SOLVED
//! ---------------------------------------------------------------------------
//! `Node::new_for_test` is hardwired to `Network::Devnet`, whose #22 is `20` and
//! whose #20 is `0`. A below-gate band therefore DOES exist on devnet — heights
//! `0..=19` — so the hazard is not "the below-gate arm is unreachable". It is
//! that the devnet boundary is the WRONG INSTRUMENT to measure against:
//!
//! * Devnet `#22 = 20` is a FENCE, not a test constant. It was chosen to keep the
//!   five INC-I-174 node suites (which run at block heights 0-7) below the gate so
//!   they pass unmodified, and it is exempted from the `#22 <= #21` ordering half
//!   on devnet ONLY. Nothing makes it immutable — devnet has a fresh genesis every
//!   run — so it may be re-pinned the day a fenced suite grows past height 20.
//!   Hard-coding `19` / `20` here would couple this experiment to that unrelated
//!   fence, and a re-pin would move this harness's boundary silently.
//! * The devnet pair would also be one block apart and adjacent to genesis, on a
//!   node whose bootstrap the devnet params shape for exactly that region.
//!
//! Testnet's `300_000` is the opposite on both counts: a pinned release value, and
//! `172_800` blocks clear of #20.
//!
//! SOLVED the same way `bins/node/tests/inc_i_172_m2_fail_close.rs` solves it for
//! gate #20: [`make_node`] builds the node with `new_for_test` and then switches
//! `node.config.network = Network::Testnet`, whose #22 is `300_000` and whose #20
//! is `127_200`. The production site reads its gate from
//! `self.config.network.params()`, so the switch moves the gate the code actually
//! consults. **The full `Node` harness therefore reaches BOTH arms** — no weaker
//! dispatcher-only substitute was needed, and none is used.
//!
//! Two consequences of the switch are recorded rather than hidden:
//!
//! 1. Both test heights ([`BELOW_GATE`] `299_999` and [`AT_GATE`] `300_000`) are
//!    ABOVE gate #20 `127_200`, so `verify_multisig{,_excluding}_at` takes the
//!    DISTINCT-SIGNER arm on both sides. The height difference between the two
//!    rows therefore changes the MESSAGE and nothing else — which is what makes
//!    the pair a controlled experiment instead of two unrelated observations.
//! 2. `node.params` stays the DEVNET `ConsensusParams` that `new_for_test` built,
//!    so `node.params.genesis_hash` is the devnet genesis. Every expected bound
//!    message is therefore computed by [`bound_message`] from
//!    `node.params.genesis_hash` READ BACK OFF THE NODE, never from a network
//!    constant. That is deliberate: it makes the tests fail if the production
//!    site binds to anything other than `self.params.genesis_hash` (e.g. a
//!    hardcoded `ChainSpec::mainnet()` hash).

#![allow(dead_code)]

use std::sync::Arc;

use crypto::{KeyPair, PublicKey};
use doli_core::maintainer::{
    signing_message, MaintainerChangeData, MaintainerSignature, MAINTAINER_AUTH_VALID_BEFORE_UNSET,
};
use doli_core::transaction::TxType;
use doli_core::{MaintainerSet, Network, Transaction};
use doli_node::node::Node;
use storage::MaintainerState;
use tempfile::TempDir;
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// The pinned testnet gates. Duplicated as literals ON PURPOSE, and bound to the
// shipped params by
// `inc_i_176_m2_gate_wiring::req_176_022_harness_gate_literals_match_the_shipped_params`
// — a harness that silently followed a moved gate would keep passing while
// testing the wrong side of it.
// ---------------------------------------------------------------------------

/// `maintainer_derivation_activation_height` (#20) for `Network::Testnet`.
pub const TESTNET_GATE_20: u64 = 127_200;

/// `inc_i_176_auth_binding_activation_height` (#22) for `Network::Testnet`.
pub const TESTNET_GATE_22: u64 = 300_000;

/// One block BELOW #22 — the legacy arm, and still comfortably above #20.
pub const BELOW_GATE: u64 = TESTNET_GATE_22 - 1;

/// EXACTLY #22 — the boundary. The comparison must be `>=` (`set.rs:269`), so
/// this height takes the BOUND arm.
pub const AT_GATE: u64 = TESTNET_GATE_22;

/// Well above #22, to show the bound arm is not an edge artefact.
pub const ABOVE_GATE: u64 = TESTNET_GATE_22 + 1_000;

/// Compile-time guards. If #22 or #20 is ever re-pinned outside these bounds the
/// suite stops BUILDING instead of quietly asserting the wrong arm.
const _: () = assert!(BELOW_GATE < TESTNET_GATE_22);
const _: () = assert!(AT_GATE >= TESTNET_GATE_22);
const _: () = assert!(ABOVE_GATE > TESTNET_GATE_22);
/// Both rows must sit ABOVE #20 so the DISTINCT-SIGNER counter applies on both
/// sides and the only varying input is the message.
const _: () = assert!(BELOW_GATE >= TESTNET_GATE_20);

pub const ACTIVATION_VERSION: u32 = 9;
pub const ACTIVATION_EPOCH: u64 = 4_000;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A TESTNET-shaped node whose on-chain maintainer root has exactly
/// `maintainer_count` members.
///
/// The maintainers are generated INDEPENDENTLY of the genesis producers. The
/// `AddMaintainer` / `RemoveMaintainer` arms authorize against `ms.set` alone —
/// never against producer keys — so using disjoint key material makes an
/// accidental producer-key acceptance impossible to mistake for success.
pub async fn make_node(maintainer_count: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..5).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers)
        .await
        .expect("Node::new_for_test failed");

    // THE HARNESS HAZARD FIX — see the module header. Devnet's #22 is 20, a FENCE
    // for the INC-I-174 suites rather than a stable test constant; testnet's
    // 300_000 is a pinned release value 172_800 blocks clear of #20.
    node.config.network = Network::Testnet;

    let maintainers: Vec<KeyPair> = (0..maintainer_count).map(|_| KeyPair::generate()).collect();
    let members: Vec<PublicKey> = maintainers.iter().map(|kp| *kp.public_key()).collect();
    let state = MaintainerState {
        set: MaintainerSet::with_members(members, 0),
        ..Default::default()
    };
    node.set_maintainer_state(Arc::new(RwLock::new(state)));

    (node, maintainers, temp)
}

pub fn sig(kp: &KeyPair, message: &[u8]) -> MaintainerSignature {
    MaintainerSignature::new(
        *kp.public_key(),
        crypto::signature::sign(message, kp.private_key()),
    )
}

/// A maintainer-change transaction whose signatures are over `message` —
/// WHICHEVER bytes the caller chose. That choice is the independent variable of
/// the whole suite.
pub fn change_tx(
    is_add: bool,
    target: &PublicKey,
    message: &[u8],
    signers: &[&KeyPair],
) -> Transaction {
    let data =
        MaintainerChangeData::new(*target, signers.iter().map(|kp| sig(kp, message)).collect());
    Transaction {
        version: 1,
        tx_type: if is_add {
            TxType::AddMaintainer
        } else {
            TxType::RemoveMaintainer
        },
        inputs: vec![],
        outputs: vec![],
        extra_data: data.to_bytes(),
    }
}

/// The bytes a signer must produce AT AND ABOVE #22, computed from the genesis
/// hash READ OFF THE NODE — see harness note 2 in the module header.
pub fn bound_message(node: &Node, is_add: bool, target: &PublicKey) -> Vec<u8> {
    signing_message(
        node.params.genesis_hash.as_bytes(),
        is_add,
        target,
        MAINTAINER_AUTH_VALID_BEFORE_UNSET,
    )
}

pub async fn members(node: &Node) -> Vec<PublicKey> {
    node.maintainer_state
        .as_ref()
        .expect("maintainer_state must be attached")
        .read()
        .await
        .set
        .members
        .clone()
}

pub async fn threshold(node: &Node) -> usize {
    node.maintainer_state
        .as_ref()
        .expect("maintainer_state must be attached")
        .read()
        .await
        .set
        .threshold
}

pub async fn submit(node: &Node, tx: &Transaction, height: u64) -> Option<(u32, u64)> {
    let producers = node.producer_set.read().await.clone();
    node.process_transaction_governance(tx, height, &producers)
        .await
}

/// Three DISTINCT maintainer signers taken from `maintainers`.
///
/// Distinctness matters: both test heights are above gate #20, where
/// `verify_multisig` counts SIGNERS, not signature ENTRIES (INC-I-172 M2 /
/// AUDIT-P0-010). The remove-arm callers pass `&maintainers[1..]` so the target
/// itself is never among the signers — `verify_multisig_excluding` would drop it.
pub fn quorum(maintainers: &[KeyPair]) -> Vec<&KeyPair> {
    let picked: Vec<&KeyPair> = maintainers.iter().take(3).collect();
    assert_eq!(
        picked.len(),
        3,
        "fixture: three DISTINCT maintainer signers are required"
    );
    picked
}

/// The release-signing preimage, reproduced verbatim from
/// `crates/updater/src/verification.rs:33` (`format!("{}:{}", version,
/// binary_sha256)`).
///
/// Rebuilt from the FORMAT STRING rather than borrowed from
/// `signing_message_legacy`, so the AUDIT-P0-011 collision is DEMONSTRATED rather
/// than assumed.
pub fn release_signing_message(version: &str, binary_sha256: &str) -> Vec<u8> {
    format!("{}:{}", version, binary_sha256).into_bytes()
}

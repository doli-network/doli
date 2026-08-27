//! INC-I-172 M2 review **F3** — regression: governance must stay alive on the
//! repo's own devnet.
//!
//! What M2 shipped, and what it broke
//! ----------------------------------
//! M2 gated the trust-root work behind
//! `NetworkParams::maintainer_derivation_activation_height`, which is **0** on
//! devnet (fresh genesis every run, no history to reinterpret). Above the gate
//! `ProtocolActivation` FAILS CLOSED when the on-chain root is not authorizable
//! (`bins/node/src/node/apply_block/governance.rs`), and `AddMaintainer` cannot
//! repair it either — `MaintainerSet::is_authorizable()` is false on an empty
//! set, so `verify_multisig_at` refuses before it counts anything.
//!
//! The seed precondition in `Node::maybe_bootstrap_maintainer_set`
//! (`bins/node/src/node/periodic.rs`) was the hardcoded constant
//! `INITIAL_MAINTAINER_COUNT` (5). `scripts/launch_testnet.sh` boots a **TWO**
//! producer devnet. Two is less than five, so the root never seeded, and because
//! the devnet gate is 0 the empty root was reached at block 0 and was
//! **ABSORBING**: governance and `ProtocolActivation` were dead for the life of
//! the chain, on the one network where the update path is testable at all.
//!
//! Pre-M2 the same devnet worked, via
//! `derive_ad_hoc_maintainer_set(2 producers)` -> `calculate_threshold(2) == 2`.
//! So this was a functional REGRESSION introduced by M2, not a pre-existing gap.
//!
//! The fix: `NetworkParams::maintainer_seed_min_producers` — 5 on mainnet and
//! testnet (byte-identical to M2 as reviewed), 2 on devnet.
//!
//! It is also a SCALE-SENSITIVITY defect by the protection-registry definition
//! (`.claude/protocols/system-impact.md`): a constant calibrated for production
//! that self-starves the small-N network. IP-D5/IP-D6 below are the "tested at
//! both ends" evidence that rule demands.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! Functions under test:
//!   G1: `Node::maybe_bootstrap_maintainer_set(&self, u64)`
//!   G2: `Node::process_transaction_governance(&self, &Transaction, u64, &ProducerSet)
//!        -> Option<(u32, u64)>`
//!   G3: `NetworkParams::load(Network) -> NetworkParams` (the params surface that
//!        carries the precondition)
//!
//! OUTPUTS
//!   O1 (receiver mutation) `maintainer_state.set.members`
//!   O2 (receiver mutation) `maintainer_state.set.threshold`
//!   O3 (pure predicate)    `maintainer_state.set.is_authorizable()`
//!   O4 (return of G2)      `Option<(u32, u64)>` — activation accepted or not
//!   O5 (return of G3)      `maintainer_seed_min_producers` per network
//!
//! PATHS
//!   PS-seeded    — the precondition cleared and the root was written
//!   PS-dead      — the precondition did not clear; the root is empty (absorbing)
//!   PA-accepted  — `ProtocolActivation` returned Some
//!   PA-closed    — `ProtocolActivation` returned None (fail-close)
//!
//! INPUT PARTITIONS
//!   IP-D1  devnet, 2 producers, seed at height 0   -> PS-seeded  (RED before F3)
//!   IP-D2  IP-D1 then a 2-distinct-signer ProtocolActivation at height 0
//!                                                  -> PA-accepted (RED before F3)
//!   IP-D3  devnet, 5 producers                     -> PS-seeded  (GREEN before F3;
//!          control proving the harness is not simply always-green)
//!   IP-D4  devnet, 1 producer                      -> PS-dead + PA-closed. The
//!          documented residual: the devnet precondition is 2, not 1.
//!   IP-D5  mainnet params                          -> O5 == INITIAL_MAINTAINER_COUNT
//!   IP-D6  testnet params                          -> O5 == INITIAL_MAINTAINER_COUNT
//!
//! MATRIX
//!   O1 x {IP-D1, IP-D3, IP-D4}  = 3 assertions
//!   O2 x {IP-D1}                = 1 assertion
//!   O3 x {IP-D1, IP-D4}         = 2 assertions
//!   O4 x {IP-D2, IP-D4}         = 2 assertions
//!   O5 x {IP-D1, IP-D5, IP-D6}  = 3 assertions
//!
//! ANTI-VACUITY
//!   IP-D1 <-> IP-D3 — byte-identical harness, only the producer COUNT differs
//!                     (2 vs 5). IP-D3 was green before F3, so a green IP-D1
//!                     cannot come from a harness that seeds unconditionally.
//!   IP-D1 <-> IP-D4 — 2 vs 1 producer. IP-D4 stays PS-dead, so IP-D1's PS-seeded
//!                     is caused by the precondition clearing, not by the
//!                     precondition having been removed altogether.
//!   IP-D5 / IP-D6   — fork-safety guard: if anyone lowers the mainnet or testnet
//!                     precondition, the seed path stops being byte-identical to
//!                     the reviewed M2 and these fail.

use std::sync::Arc;

use crypto::{KeyPair, PublicKey};
use doli_core::maintainer::{
    MaintainerSignature, ProtocolActivationData, INITIAL_MAINTAINER_COUNT,
};
use doli_core::network_params::NetworkParams;
use doli_core::transaction::TxType;
use doli_core::{Network, Transaction};
use doli_node::node::Node;
use storage::MaintainerState;
use tempfile::TempDir;
use tokio::sync::RwLock;

const ACTIVATION_VERSION: u32 = 9;
const ACTIVATION_EPOCH: u64 = 4_000;

/// `scripts/launch_testnet.sh` — "DOLI Testnet - Two Producer Genesis Launch".
const DEVNET_PRODUCERS: usize = 2;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A DEVNET node (`Node::new_for_test` is hardwired to `Network::Devnet`, whose
/// `maintainer_derivation_activation_height` is 0) with `n` genesis producers
/// and an EMPTY maintainer root.
async fn make_devnet_node(n: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    assert_eq!(
        node.config.network,
        Network::Devnet,
        "setup: this file is about DEVNET; new_for_test must stay devnet-shaped"
    );
    node.set_maintainer_state(Arc::new(RwLock::new(MaintainerState::default())));
    (node, producers, temp)
}

fn sig(kp: &KeyPair, message: &[u8]) -> MaintainerSignature {
    MaintainerSignature::new(
        *kp.public_key(),
        crypto::signature::sign(message, kp.private_key()),
    )
}

fn activation_message() -> Vec<u8> {
    ProtocolActivationData::new(ACTIVATION_VERSION, ACTIVATION_EPOCH, String::new(), vec![])
        .signing_message()
}

fn activation_tx(signatures: Vec<MaintainerSignature>) -> Transaction {
    let data = ProtocolActivationData::new(
        ACTIVATION_VERSION,
        ACTIVATION_EPOCH,
        "INC-I-172 M2 F3 devnet regression".to_string(),
        signatures,
    );
    Transaction {
        version: 1,
        tx_type: TxType::ProtocolActivation,
        inputs: vec![],
        outputs: vec![],
        extra_data: data.to_bytes(),
    }
}

async fn root_members(node: &Node) -> Vec<PublicKey> {
    node.maintainer_state
        .as_ref()
        .expect("maintainer_state must be attached")
        .read()
        .await
        .set
        .members
        .clone()
}

async fn submit(node: &Node, tx: &Transaction, height: u64) -> Option<(u32, u64)> {
    let producers = node.producer_set.read().await.clone();
    node.process_transaction_governance(tx, height, &producers)
        .await
}

// ---------------------------------------------------------------------------
// IP-D1 / IP-D2 — the regression itself
// ---------------------------------------------------------------------------

/// IP-D1. O1, O2, O3, O5 x PS-seeded. **F3 — RED before the fix.**
#[tokio::test]
async fn devnet_seeds_its_trust_root_with_only_two_producers() {
    let params = NetworkParams::load(Network::Devnet);

    // O5 — the precondition itself. This is the line that was hardcoded to 5.
    assert!(
        params.maintainer_seed_min_producers <= DEVNET_PRODUCERS,
        "F3: devnet's seed precondition ({}) must be satisfiable by the \
         {}-producer devnet that scripts/launch_testnet.sh actually boots. \
         Above the gate (devnet AH = 0) an unseeded root is ABSORBING: \
         ProtocolActivation fails closed and an empty set refuses the \
         AddMaintainer that would repair it.",
        params.maintainer_seed_min_producers,
        DEVNET_PRODUCERS
    );

    let (node, _producers, _t) = make_devnet_node(DEVNET_PRODUCERS).await;
    node.maybe_bootstrap_maintainer_set(0).await;

    // O1
    let members = root_members(&node).await;
    assert_eq!(
        members.len(),
        DEVNET_PRODUCERS,
        "F3: a {}-producer devnet must seat all {} producers in the trust root, \
         not leave it empty",
        DEVNET_PRODUCERS,
        DEVNET_PRODUCERS
    );

    let ms = node.maintainer_state.as_ref().unwrap().read().await;
    // O2 — calculate_threshold(2) == 2, the same value the pre-M2 devnet used.
    assert_eq!(
        ms.set.threshold, 2,
        "F3: a 2-member root must carry threshold 2, matching pre-M2 devnet \
         (derive_ad_hoc_maintainer_set(2) -> calculate_threshold(2))"
    );
    // O3 — the property that makes governance reachable at all.
    assert!(
        ms.set.is_authorizable(),
        "F3: the devnet root must be AUTHORIZABLE, or every verifier \
         short-circuits false before counting a single signature"
    );
}

/// IP-D2. O4 x PA-accepted. **F3 — RED before the fix.**
#[tokio::test]
async fn devnet_protocol_activation_is_accepted_by_its_two_maintainers() {
    let (node, producers, _t) = make_devnet_node(DEVNET_PRODUCERS).await;
    node.maybe_bootstrap_maintainer_set(0).await;

    let seeded = root_members(&node).await;
    assert_eq!(
        seeded.len(),
        DEVNET_PRODUCERS,
        "setup: the root must be seeded"
    );

    let msg = activation_message();
    let signers: Vec<&KeyPair> = producers
        .iter()
        .filter(|kp| seeded.contains(kp.public_key()))
        .collect();
    assert_eq!(
        signers.len(),
        DEVNET_PRODUCERS,
        "setup: both devnet producers must be in the seeded root"
    );

    let tx = activation_tx(signers.iter().map(|kp| sig(kp, &msg)).collect());

    // Devnet AH is 0, so height 0 is already AT the gate: this exercises the
    // post-activation fail-close branch, not the frozen producer fallback.
    assert_eq!(
        submit(&node, &tx, 0).await,
        Some((ACTIVATION_VERSION, ACTIVATION_EPOCH)),
        "F3: a genuine quorum of ALL devnet maintainers must be able to activate \
         a protocol version. M2 made this permanently impossible on the repo's \
         own 2-producer devnet, which is the network the auto-update path is \
         developed against."
    );
}

// ---------------------------------------------------------------------------
// IP-D3 / IP-D4 — anti-vacuity
// ---------------------------------------------------------------------------

/// IP-D3 (control, GREEN before F3). O1 x PS-seeded.
///
/// Same harness, five producers. This passed before the fix, so a green IP-D1
/// cannot be an artefact of a harness that seeds unconditionally.
#[tokio::test]
async fn control_devnet_with_five_producers_seeds_the_full_five() {
    let (node, _producers, _t) = make_devnet_node(INITIAL_MAINTAINER_COUNT).await;
    node.maybe_bootstrap_maintainer_set(0).await;

    assert_eq!(
        root_members(&node).await.len(),
        INITIAL_MAINTAINER_COUNT,
        "CONTROL: five producers seat five maintainers, as before F3"
    );
}

/// IP-D4 (control + documented residual). O1, O3, O4 x PS-dead / PA-closed.
///
/// The devnet precondition is 2, not 1. A ONE-producer devnet is still a
/// dead-end, and that is a decision rather than a surprise: a 1-member root has
/// `calculate_threshold(1) == 1`, i.e. a single key would hold unilateral
/// governance authority. This test pins the boundary so the next reader knows
/// where it is.
#[tokio::test]
async fn control_devnet_with_one_producer_stays_a_documented_dead_end() {
    let params = NetworkParams::load(Network::Devnet);
    assert_eq!(
        params.maintainer_seed_min_producers, 2,
        "the devnet precondition is deliberately 2 (the launch_testnet.sh shape), \
         not 1: a 1-member root gives one key unilateral governance authority"
    );

    let (node, producers, _t) = make_devnet_node(1).await;
    node.maybe_bootstrap_maintainer_set(0).await;

    // O1 / O3 — PS-dead.
    assert!(
        root_members(&node).await.is_empty(),
        "CONTROL: one producer is below the devnet precondition, so the root \
         stays empty. IP-D1's success therefore comes from the precondition \
         CLEARING, not from the precondition having been deleted."
    );

    // O4 — PA-closed. The absorbing state, demonstrated.
    let msg = activation_message();
    let tx = activation_tx(vec![sig(&producers[0], &msg)]);
    assert!(
        submit(&node, &tx, 0).await.is_none(),
        "CONTROL: with an unseeded root above the gate, ProtocolActivation fails \
         closed — this is exactly the state a 2-producer devnet was stuck in \
         before F3"
    );
}

// ---------------------------------------------------------------------------
// IP-D5 / IP-D6 — fork-safety guard on the live networks
// ---------------------------------------------------------------------------

/// IP-D5 + IP-D6. O5 x {mainnet, testnet}.
///
/// The F3 fix must not move mainnet or testnet by one bit. Both keep the
/// historical hardcoded precondition, so their seed path is byte-identical to
/// the M2 tree the fork-safety review was performed against.
#[test]
fn mainnet_and_testnet_keep_the_reviewed_seed_precondition() {
    assert_eq!(
        NetworkParams::load(Network::Mainnet).maintainer_seed_min_producers,
        INITIAL_MAINTAINER_COUNT,
        "FORK SAFETY: mainnet's seed precondition must remain \
         INITIAL_MAINTAINER_COUNT. Lowering it changes WHEN the trust root is \
         first written on a live chain, which decides which ProtocolActivations \
         a node accepts — history that is not re-derivable once written."
    );
    assert_eq!(
        NetworkParams::load(Network::Testnet).maintainer_seed_min_producers,
        INITIAL_MAINTAINER_COUNT,
        "FORK SAFETY: testnet's seed precondition must remain \
         INITIAL_MAINTAINER_COUNT — the 12-node local testnet clears it, and the \
         127_200 gate rehearsal must exercise the same path mainnet will."
    );
}

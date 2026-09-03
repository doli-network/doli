//! INC-I-172 M2 — CATEGORY B (runtime-red): F4 fail-close, plus the
//! ACTIVATION-GATING REGRESSION suite that proves pre-`maintainer_derivation_activation_height`
//! behavior is preserved EXACTLY.
//!
//! Unlike the other Category B files, this one COMPILES against the current
//! tree: the node-level entry points already take `height`, so M2 needs no
//! signature change here — only a height-gated branch that reads
//! `self.config.network.params().maintainer_derivation_activation_height`.
//! The post-activation tests are therefore RUNTIME-red today; the
//! pre-activation parity tests are GREEN today and must STAY green.
//!
//! Findings under test
//! -------------------
//! * **F4** — `Node::derive_ad_hoc_maintainer_set`
//!   (`bins/node/src/node/apply_block/governance.rs:112-124`). Whenever the
//!   on-chain root is not `is_fully_bootstrapped()`, `ProtocolActivation`
//!   verification silently reverts to PRODUCER-KEY authority
//!   (`governance.rs:83-93`). That is a silent downgrade back to the compromised
//!   key set, and it is reachable by any actor able to drive the root
//!   sub-threshold. Post-activation it must FAIL CLOSED.
//! * **AUDIT-P0-010** — the same `verify_multisig` entry-counting defect, reached
//!   through the node's `ProtocolActivation` path.
//! * **AUDIT-P1-013 / FM-01** — durability of a governance removal, gated.
//!
//! Requirements: REQ-172-002 (removes a hostile-quorum back-door),
//! REQ-172-005, REQ-172-012.
//! Spec: `specs/maintainer-trust-root-architecture.md` §F2, §F4.
//!
//! ---------------------------------------------------------------------------
//! WHY THE PRE-ACTIVATION TESTS ASSERT THE DEFECT
//! ---------------------------------------------------------------------------
//! Below the gate the node MUST still produce the old — insecure — answer.
//! `ProtocolActivation` acceptance is consensus-visible (INV-12: Q1 YES, Q2 YES,
//! Q3 NO), so a node that applies the new rule to a historical height disagrees
//! with the fleet about which activations took effect and forks. "Pre-height
//! parity must be PROVEN, not assumed" — these tests are that proof. They are
//! expected to read uncomfortably.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! Functions under test:
//!   G1 `Node::process_transaction_governance(&self, &Transaction, u64, &ProducerSet)
//!       -> Option<(u32, u64)>`
//!   G2 `Node::maybe_bootstrap_maintainer_set(&self, u64)`
//!      (M2 contract: the SAME name and signature must survive as a ONE-SHOT
//!       genesis seed; the tests below assert BEHAVIOR — "a seed attempt after a
//!       governance mutation does not change the root" — and deliberately do
//!       NOT prescribe the guard's mechanism, because the "has this root ever
//!       been mutated?" flag could otherwise be read as requiring a new
//!       persisted field, hence a MAINTAINER_STATE_VERSION bump, which needs
//!       explicit user approval.)
//!
//! OUTPUTS
//!   O1 (return of G1)             `Option<(u32, u64)>` — accepted activation
//!   O2 (receiver mutation)        `maintainer_state.set.members`
//!   O3 (receiver mutation)        `maintainer_state.set.threshold`
//!   O4 (persistent store write)   `<data_dir>/maintainer_state.bin`
//!   O5 (mutable params)           — NONE; both entry points take `&self`
//!
//! PATHS
//!   PF-open   — activation accepted through the producer-key fallback
//!   PF-closed — activation rejected because the on-chain root is unusable
//!   PD-durable / PD-reverted — the governance removal survives / is erased
//!
//! INPUT PARTITIONS (network = Testnet, gate = 127_200 for every row)
//!   IP-F1  empty root, 3 producer-key sigs, h = 200_000  -> PF-closed  (RED today)
//!   IP-F2  empty root, 3 producer-key sigs, h = 1        -> PF-open    (PARITY, green today)
//!   IP-F3  seeded root, 3 DISTINCT maintainer sigs, h = 200_000 -> PF-open (liveness control)
//!   IP-F4  seeded root, 3 entries from ONE maintainer, h = 200_000 -> PF-closed (RED today)
//!   IP-F5  seeded root, 3 entries from ONE maintainer, h = 1 -> PF-open (PARITY, green today)
//!   IP-F6  seeded root, valid removal, seed attempt at h = 200_001 -> PD-durable (RED today)
//!   IP-F7  seeded root, valid removal, seed attempt at h = 2 -> PD-reverted (PARITY, green today)
//!
//! MATRIX
//!   O1 x {IP-F1..IP-F5} = 5 assertions
//!   O2 x {IP-F6, IP-F7} = 2 assertions
//!   O3 x {IP-F6}        = 1 assertion
//!   O4 x {IP-F6}        = 1 assertion
//!   O5 — structurally absent.
//!
//! ANTI-VACUITY PAIRING (each row differs from its partner in ONE input)
//!   IP-F1 <-> IP-F2 (only the height)
//!   IP-F4 <-> IP-F5 (only the height)
//!   IP-F4 <-> IP-F3 (only signer distinctness)
//!   IP-F6 <-> IP-F7 (only the height)

use std::sync::Arc;

use crypto::{KeyPair, PublicKey};
use doli_core::maintainer::{MaintainerChangeData, MaintainerSignature, ProtocolActivationData};
use doli_core::transaction::TxType;
use doli_core::{Network, Transaction};
use doli_node::node::Node;
use storage::MaintainerState;
use tempfile::TempDir;
use tokio::sync::RwLock;

/// `maintainer_derivation_activation_height` for `Network::Testnet`.
/// Duplicated as a literal on purpose: this test binary must fail loudly if the
/// pinned testnet gate ever moves, and the constant is asserted against
/// `NetworkParams` in `crates/core/tests/inc_i_172_m2_activation_height.rs`.
const TESTNET_GATE: u64 = 127_200;

const BELOW_GATE: u64 = 1;
const ABOVE_GATE: u64 = 200_000;

const ACTIVATION_VERSION: u32 = 9;
const ACTIVATION_EPOCH: u64 = 4_000;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A Testnet-shaped node with five genesis producers and an EMPTY maintainer
/// root. `Node::new_for_test` is hardwired to Devnet, whose gate is 0 (always
/// active), so it cannot express a pre-activation height at all — the network is
/// switched to Testnet, whose gate is 127_200.
async fn make_node(n: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    node.config.network = Network::Testnet;
    node.set_maintainer_state(Arc::new(RwLock::new(MaintainerState::default())));
    (node, producers, temp)
}

fn sig(kp: &KeyPair, message: &[u8]) -> MaintainerSignature {
    MaintainerSignature::new(
        *kp.public_key(),
        crypto::signature::sign(message, kp.private_key()),
    )
}

/// A `ProtocolActivation` transaction carrying `signatures`.
fn activation_tx(signatures: Vec<MaintainerSignature>) -> Transaction {
    let data = ProtocolActivationData::new(
        ACTIVATION_VERSION,
        ACTIVATION_EPOCH,
        "INC-I-172 M2 gate test".to_string(),
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

fn activation_message() -> Vec<u8> {
    ProtocolActivationData::new(ACTIVATION_VERSION, ACTIVATION_EPOCH, String::new(), vec![])
        .signing_message()
}

/// INC-I-201: sign exactly what `apply_block/governance.rs` verifies at
/// `height`. Above `inc_i_176_auth_binding_activation_height` (INC-I-176 M4,
/// testnet 15_087 since the 2026-08-24 re-pin) the node checks the
/// genesis-bound, domain-tagged digest; below it, the legacy message.
/// `ABOVE_GATE` (200_000) crossed that line when the height was re-pinned
/// from 300_000, so a legacy-signed removal was silently rejected as
/// "insufficient signatures" and the F2 precondition (5 -> 4) never held.
fn remove_tx(node: &Node, target: &PublicKey, signers: &[&KeyPair], height: u64) -> Transaction {
    let mut data = MaintainerChangeData::new(*target, vec![]);
    let message = doli_core::maintainer::signing_message_at(
        node.params.genesis_hash.as_bytes(),
        false,
        target,
        doli_core::maintainer::MAINTAINER_AUTH_VALID_BEFORE_UNSET,
        height,
        node.config
            .network
            .params()
            .inc_i_176_auth_binding_activation_height,
    );
    data.signatures = signers.iter().map(|kp| sig(kp, &message)).collect();
    Transaction {
        version: 1,
        tx_type: TxType::RemoveMaintainer,
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

/// Compile-time guard so a moved testnet gate cannot silently turn a
/// post-activation partition into a pre-activation one. If `TESTNET_GATE` is
/// ever re-pinned outside `(BELOW_GATE, ABOVE_GATE]`, this file stops building
/// instead of quietly asserting the wrong side of the gate.
const _: () = assert!(BELOW_GATE < TESTNET_GATE);
const _: () = assert!(ABOVE_GATE >= TESTNET_GATE);

// ---------------------------------------------------------------------------
// F4 — fail close
// ---------------------------------------------------------------------------

/// IP-F1. O1 x PF-closed. **F4 / REQ-172-002 — MUST FAIL TODAY.**
#[tokio::test]
async fn protocol_activation_fails_closed_above_the_gate_when_the_root_is_unbootstrapped() {
    let (node, producers, _t) = make_node(5).await;

    // PRECONDITION — the on-chain root is empty, which is precisely the state
    // that triggers the producer-key fallback today.
    assert!(
        root_members(&node).await.is_empty(),
        "precondition: the on-chain maintainer root must be unbootstrapped"
    );

    let msg = activation_message();
    let tx = activation_tx(vec![
        sig(&producers[0], &msg),
        sig(&producers[1], &msg),
        sig(&producers[2], &msg),
    ]);

    let accepted = submit(&node, &tx, ABOVE_GATE).await;

    assert!(
        accepted.is_none(),
        "F4 / REQ-172-002: above maintainer_derivation_activation_height a \
         ProtocolActivation must FAIL CLOSED when the on-chain root is absent or \
         sub-threshold. Today governance.rs:83-93 silently derives authority from \
         PRODUCER keys instead (derive_ad_hoc_maintainer_set, governance.rs:112-124), \
         so any actor who can drive the root sub-threshold reclaims activation \
         authority through the very key set INC-I-172 is trying to retire."
    );
}

/// IP-F2 (PARITY — green today, MUST STAY GREEN). O1 x PF-open.
///
/// Below the gate the producer-key fallback is consensus history. Applying the
/// new rule retroactively would change which activations took effect and fork
/// the node off the chain.
#[tokio::test]
async fn parity_below_the_gate_protocol_activation_still_uses_the_producer_fallback() {
    let (node, producers, _t) = make_node(5).await;

    let msg = activation_message();
    let tx = activation_tx(vec![
        sig(&producers[0], &msg),
        sig(&producers[1], &msg),
        sig(&producers[2], &msg),
    ]);

    let accepted = submit(&node, &tx, BELOW_GATE).await;

    assert_eq!(
        accepted,
        Some((ACTIVATION_VERSION, ACTIVATION_EPOCH)),
        "ACTIVATION-GATING PARITY: below maintainer_derivation_activation_height \
         the ad-hoc producer-key fallback MUST be preserved byte-for-byte. \
         ProtocolActivation acceptance is consensus-visible (INV-12 Q1/Q2 YES, \
         Q3 NO); a node that fails closed at a historical height disagrees with \
         the fleet about which activations took effect."
    );
}

/// IP-F3 (liveness control). O1 x PF-open. Above the gate a GENUINE quorum on a
/// properly seeded root must still be accepted — otherwise F4 is a governance
/// lock-out, not a security fix.
#[tokio::test]
async fn control_above_the_gate_a_genuine_maintainer_quorum_is_accepted() {
    let (node, producers, _t) = make_node(5).await;

    node.maybe_bootstrap_maintainer_set(0).await;
    let seeded = root_members(&node).await;
    assert_eq!(seeded.len(), 5, "setup: the root must be seeded");

    let msg = activation_message();
    let signers: Vec<&KeyPair> = producers
        .iter()
        .filter(|kp| seeded.contains(kp.public_key()))
        .take(3)
        .collect();
    assert_eq!(signers.len(), 3, "setup: three distinct seeded maintainers");

    let tx = activation_tx(signers.iter().map(|kp| sig(kp, &msg)).collect());

    assert_eq!(
        submit(&node, &tx, ABOVE_GATE).await,
        Some((ACTIVATION_VERSION, ACTIVATION_EPOCH)),
        "CONTROL: a genuine 3-distinct-signer quorum on a seeded root MUST still \
         activate above the gate"
    );
}

/// IP-F4. O1 x PF-closed. **AUDIT-P0-010 at the node level — MUST FAIL TODAY.**
#[tokio::test]
async fn protocol_activation_from_one_key_signing_three_times_is_rejected_above_the_gate() {
    let (node, producers, _t) = make_node(5).await;

    node.maybe_bootstrap_maintainer_set(0).await;
    let seeded = root_members(&node).await;
    assert_eq!(seeded.len(), 5, "setup: the root must be seeded");

    let lone = producers
        .iter()
        .find(|kp| seeded.contains(kp.public_key()))
        .expect("setup: at least one seeded maintainer");

    let msg = activation_message();
    let tx = activation_tx(vec![sig(lone, &msg), sig(lone, &msg), sig(lone, &msg)]);

    assert!(
        submit(&node, &tx, ABOVE_GATE).await.is_none(),
        "AUDIT-P0-010 / REQ-172-012: above the gate, THREE signature entries from \
         ONE maintainer key are 1 distinct signer and must not clear the 3-of-5 \
         threshold that guards ProtocolActivation"
    );
}

/// IP-F5 (PARITY — green today, MUST STAY GREEN). O1 x PF-open.
#[tokio::test]
async fn parity_below_the_gate_one_key_signing_three_times_is_still_accepted() {
    let (node, producers, _t) = make_node(5).await;

    node.maybe_bootstrap_maintainer_set(0).await;
    let seeded = root_members(&node).await;
    let lone = producers
        .iter()
        .find(|kp| seeded.contains(kp.public_key()))
        .expect("setup: at least one seeded maintainer");

    let msg = activation_message();
    let tx = activation_tx(vec![sig(lone, &msg), sig(lone, &msg), sig(lone, &msg)]);

    assert_eq!(
        submit(&node, &tx, BELOW_GATE).await,
        Some((ACTIVATION_VERSION, ACTIVATION_EPOCH)),
        "ACTIVATION-GATING PARITY: the entry-counting counter is consensus \
         history below the gate. This assertion is deliberately asserting the \
         DEFECT: rewriting it retroactively forks the node."
    );
}

// ---------------------------------------------------------------------------
// F2 — the reset button, gated
// ---------------------------------------------------------------------------

/// Seed the root, then remove one maintainer with a genuine 3-distinct-signer
/// quorum. Returns the removed key.
async fn seed_and_remove(node: &Node, producers: &[KeyPair], height: u64) -> PublicKey {
    node.maybe_bootstrap_maintainer_set(0).await;
    let seeded = root_members(node).await;
    assert_eq!(seeded.len(), 5, "setup: the root must be seeded with five");

    let target = seeded[0];
    let signers: Vec<&KeyPair> = producers
        .iter()
        .filter(|kp| *kp.public_key() != target && seeded.contains(kp.public_key()))
        .take(3)
        .collect();
    assert_eq!(signers.len(), 3, "setup: three distinct non-target signers");

    let tx = remove_tx(node, &target, &signers, height);
    assert!(
        submit(node, &tx, height).await.is_none(),
        "setup: a RemoveMaintainer must not report a ProtocolActivation"
    );
    assert_eq!(
        root_members(node).await.len(),
        4,
        "setup: the removal must have applied (5 -> 4)"
    );
    target
}

/// IP-F6. O2, O3, O4 x PD-durable. **FM-01 gated — MUST FAIL TODAY.**
///
/// `maybe_bootstrap_maintainer_set` is invoked here exactly as
/// `apply_block/state_update.rs:214` invokes it: once, with the height of the
/// block just applied.
#[tokio::test]
async fn above_the_gate_a_governance_removal_survives_the_next_block() {
    let (node, producers, _t) = make_node(5).await;
    let data_dir = node.config.data_dir.clone();

    let removed = seed_and_remove(&node, &producers, ABOVE_GATE).await;

    // The very next applied block.
    node.maybe_bootstrap_maintainer_set(ABOVE_GATE + 1).await;

    let after = root_members(&node).await;
    assert_eq!(
        after.len(),
        4,
        "AUDIT-P1-013 / FM-01: above maintainer_derivation_activation_height the \
         genesis seed must be ONE-SHOT. Re-deriving on every block reverts a \
         governance removal in ~10 s and makes the on-chain trust root \
         unrotatable."
    );
    assert!(
        !after.contains(&removed),
        "FM-01: the removed maintainer must not return"
    );

    // O3 / O4 — the derived threshold and the file the updater trusts.
    let ms = node.maintainer_state.as_ref().unwrap().read().await;
    assert_eq!(ms.set.threshold, 3, "O3: a 4-member set has threshold 3");
    drop(ms);

    let persisted = MaintainerState::load(&data_dir).expect("O4: the state file must be readable");
    assert!(
        !persisted.set.is_maintainer(&removed),
        "O4: the persisted maintainer_state.bin is the updater's install trust \
         root (trust_root_wiring.rs:95); a reverted root on disk re-arms the \
         removed key for every binary install"
    );
}

/// IP-F7 (PARITY — green today, MUST STAY GREEN). O2 x PD-reverted.
///
/// This asserts the DEFECT below the gate, on purpose. The maintainer root is
/// node-local state, but it decides `ProtocolActivation` acceptance
/// (`governance.rs:83-93`), which is consensus-visible. Changing which root a
/// node holds at a historical height changes which activations it accepts.
#[tokio::test]
async fn parity_below_the_gate_a_governance_removal_is_still_reverted() {
    let (node, producers, _t) = make_node(5).await;

    let removed = seed_and_remove(&node, &producers, BELOW_GATE).await;

    node.maybe_bootstrap_maintainer_set(BELOW_GATE + 1).await;

    let after = root_members(&node).await;
    assert_eq!(
        after.len(),
        5,
        "ACTIVATION-GATING PARITY: below the gate the per-block re-derivation is \
         consensus history and MUST be preserved. Fixing it retroactively changes \
         which maintainer root a replaying node holds at historical heights, and \
         therefore which ProtocolActivation transactions it accepts."
    );
    assert!(
        after.contains(&removed),
        "ACTIVATION-GATING PARITY: below the gate the removed maintainer IS \
         restored — that is the historical behavior"
    );
}

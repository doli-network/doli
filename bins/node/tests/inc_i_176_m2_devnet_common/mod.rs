//! INC-I-176 **M2** — shared DEVNET harness for the bound-arm evidence
//! (**REQ-176-022**, gate #22 `inc_i_176_auth_binding_activation_height`; the
//! per-network value is **REQ-176-021** and the ordering constraint is
//! **REV-176-M1a-001**).
//!
//! OUTPUT CONTRACT: N/A — fixture module. It asserts nothing about the system
//! under test; the only assertions here are FIXTURE PRECONDITIONS ("the root
//! actually seeded", "the block actually applied"), which fail the SETUP rather
//! than the claim — except [`apply_governance_block`], which deliberately carries
//! outputs **O1** and **O7** because they must hold on EVERY row and duplicating
//! them per test is how one row quietly loses them.
//! INPUT PARTITIONS: N/A — fixture module.
//!
//! ---------------------------------------------------------------------------
//! WHY THIS IS NOT `inc_i_176_m2_common`
//! ---------------------------------------------------------------------------
//! `bins/node/tests/inc_i_176_m2_common/mod.rs` switches `node.config.network`
//! to **Testnet**, because devnet's `#22 = 20` is a FENCE for the INC-I-174
//! suites rather than a stable test constant and sits one block from its own
//! boundary (see that file's header). This harness is the opposite
//! choice on purpose: it LEAVES the node on **DEVNET**, which is the network the
//! `#22 = 20` decision is about, and drives real blocks through
//! `Node::apply_block` rather than calling `process_transaction_governance`
//! directly. With #22 = 20 devnet has BOTH arms natively — heights 0..19 legacy,
//! 20+ bound — which is the property the whole decision exists to buy.
//!
//! Block construction and seeding mirror
//! `bins/node/tests/inc_i_174_maintainer_undo.rs` so the two stay comparable: the
//! five INC-I-174 suites are exactly what devnet #22 = 20 exists to keep green,
//! so their harness is the right one to follow.
//!
//! ---------------------------------------------------------------------------
//! THE LEGACY ENCODER IS DELIBERATELY NOT THE CRATE'S
//! ---------------------------------------------------------------------------
//! [`legacy_message_independent`] rebuilds `format!("{}:{}", action, hex)` from
//! its FORMAT STRING here, exactly as the five INC-I-174 suites do. It is NOT
//! `doli_core::maintainer::signing_message_legacy`, and the INC-I-174 builders
//! are NOT rerouted through the crate constructor — a separate encoder is what
//! keeps "below the gate the legacy bytes are still accepted" a real claim about
//! the bytes a signer must produce, instead of a restatement of the
//! implementation. Considered and explicitly rejected for M2.

#![allow(dead_code)]

use std::sync::Arc;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Network, Transaction};
use doli_node::node::Node;
use storage::MaintainerState;
use tempfile::TempDir;
use tokio::sync::RwLock;
use vdf::{VdfOutput, VdfProof};

// ===========================================================================
// THE PINNED DEVNET GATES
//
// Duplicated as literals ON PURPOSE and bound to the shipped params by
// `inc_i_176_m2_devnet_bound_arm::fixture_devnet_gate_literals_match_the_shipped_params`.
// A harness that silently followed a moved gate would keep passing while testing
// the wrong arm.
// ===========================================================================

/// `inc_i_176_auth_binding_activation_height` (#22) for `Network::Devnet`.
///
/// **20, not 0.** The five INC-I-174 node suites drive governance at block
/// heights 0-7 and sign the LEGACY message with their own in-file encoder; at a
/// gate of 0 all 25 of those tests would need rewriting, and rewriting the
/// regression suite that proves INC-I-174 is not an option. At 20 they sit below
/// the gate and pass unmodified (measured: 0-line `git diff HEAD` for each of the
/// five files).
pub const DEVNET_GATE_22: u64 = 20;

/// `maintainer_derivation_activation_height` (#20) for `Network::Devnet`.
///
/// `0`, so EVERY height driven here — 5, 19, 20, 21 alike — is above #20 and
/// takes the DISTINCT-SIGNER counter (`verify_multisig_at`). The height therefore
/// changes the MESSAGE and nothing else, which is what makes the rows a
/// controlled experiment rather than a set of unrelated observations. It is also
/// why [`quorum`] must supply `threshold` DIFFERENT seated keys.
pub const DEVNET_GATE_20: u64 = 0;

/// Well below #22 — the LEGACY arm. Chosen ABOVE the INC-I-174 band (0-7) so a
/// failure here is unambiguously about this gate and not about that harness.
pub const BELOW_GATE: u64 = 5;

/// One block below #22 — still LEGACY. The `>=`-not-`>` boundary evidence.
pub const EDGE_BELOW: u64 = DEVNET_GATE_22 - 1;

/// EXACTLY #22 — the comparison is `>=` (`authmsg.rs::signing_message_at`), so
/// this height ALREADY takes the BOUND arm.
pub const AT_GATE: u64 = DEVNET_GATE_22;

/// Above #22 — the bound arm away from the edge, so acceptance cannot be read as
/// an off-by-one artefact.
pub const ABOVE_GATE: u64 = DEVNET_GATE_22 + 1;

// Compile-time guards. If #22 is re-pinned outside these bounds the suite stops
// BUILDING instead of quietly asserting the wrong arm.
const _: () = assert!(BELOW_GATE < DEVNET_GATE_22);
const _: () = assert!(EDGE_BELOW < DEVNET_GATE_22);
const _: () = assert!(AT_GATE >= DEVNET_GATE_22);
const _: () = assert!(ABOVE_GATE > DEVNET_GATE_22);
/// The gate must clear the INC-I-174 working band (block heights 0-7).
const _: () = assert!(DEVNET_GATE_22 > 7);
// The "every height sits at or above #20" guard is NOT a `const _`: devnet #20 is
// `0`, the minimum of `u64`, so a compile-time `>= 0` is a tautology clippy
// rejects (`absurd_extreme_comparisons`) and a reader would rightly distrust. It
// is asserted at RUNTIME against the SHIPPED params instead, in
// `fixture_devnet_gate_literals_match_the_shipped_params` — the stronger form
// anyway, because it fires if devnet #20 ever stops being 0.

// ===========================================================================
// HARNESS
// ===========================================================================

/// A DEVNET node with `n` genesis producers and a SEEDED maintainer root.
///
/// `Node::new_for_test` is hardwired to `Network::Devnet` and this harness
/// deliberately LEAVES IT THERE — see the module header.
///
/// The seed happens at height 0, so `set.last_updated == 0` before any rotation
/// and a rotation at height H is unambiguously distinguishable from it.
pub async fn seeded_devnet_node(n: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    assert_eq!(
        node.config.network,
        Network::Devnet,
        "harness: this suite is about DEVNET. If new_for_test stops being \
         devnet-shaped, every gate literal here is the wrong network's."
    );
    node.set_maintainer_state(Arc::new(RwLock::new(MaintainerState::default())));
    node.maybe_bootstrap_maintainer_set(0).await;
    let seeded = root_members(&node).await;
    assert_eq!(
        seeded.len(),
        n,
        "harness: all {n} genesis producers must be seated in the trust root, or \
         the quorum cannot reach threshold and every result is vacuous"
    );
    (node, producers, temp)
}

pub fn build_block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
    extra_txs: Vec<Transaction>,
) -> Block {
    let reward = params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let mut txs = vec![coinbase];
    txs.extend(extra_txs);
    let merkle_root = doli_core::block::compute_merkle_root(&txs);
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash,
        timestamp,
        slot,
        producer: *producer.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: crypto::Hash::ZERO,
        fork_id: crypto::Hash::ZERO,
    };

    Block::new(header, txs)
}

/// Apply coinbase-only blocks `1..height` so the next block can be `height`.
/// Returns the hash of the last applied block.
///
/// Real blocks, applied one at a time through `Node::apply_block` — the same path
/// production takes. This is also why devnet #22 has to stay a SMALL number: a
/// gate in the thousands would trade an unreachable arm for an unusably slow one.
pub async fn advance_to(
    node: &mut Node,
    producer: &KeyPair,
    params: &ConsensusParams,
    height: u64,
) -> Hash {
    let mut prev = Hash::ZERO;
    for h in 1..height {
        let b = build_block(h, h as u32, prev, producer, params, vec![]);
        prev = b.hash();
        node.apply_block(b, ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("harness: apply_block failed at h={h}: {e}"));
    }
    prev
}

/// Apply ONE block carrying `tx` at `height`, and assert **O1** and **O7**: the
/// block applies without error and the chain advances — on EVERY row, acceptance
/// and refusal alike.
///
/// This is the non-fatality proof, and it lives in the harness precisely so no
/// row can quietly omit it. #22 adds no reject path anywhere: the governance site
/// returns `Option`, never `Result`, and a failed authorization warns and skips
/// while the block still lands. A refusal that killed the block would be a
/// consensus change, not a verification change.
pub async fn apply_governance_block(
    node: &mut Node,
    producer: &KeyPair,
    params: &ConsensusParams,
    prev: Hash,
    height: u64,
    tx: Transaction,
) {
    let block = build_block(height, height as u32, prev, producer, params, vec![tx]);
    let outcome = node.apply_block(block, ValidationMode::Light).await;
    assert!(
        outcome.is_ok(),
        "O1: a governance transaction must NEVER make `apply_block` fail, at any \
         height and whichever bytes were signed. The #22 site \
         (apply_block/governance.rs) returns `Option`, not `Result` — it warns and \
         skips. If this errors, M2 has added a reject path and \
         crates/core/src/validation/tx_types.rs is no longer the only owner of \
         transaction rejection (binding user decision 1). Error: {:?}",
        outcome.err()
    );
    assert_eq!(
        node.chain_state.read().await.best_height,
        height,
        "O7: the block carrying the governance transaction must still be APPLIED. \
         Without this, every refusal row could be explained by 'the block never \
         landed' rather than by the authorization being skipped."
    );
}

/// **O2** — the in-memory member list, THE acceptance oracle.
pub async fn root_members(node: &Node) -> Vec<PublicKey> {
    node.maintainer_state
        .as_ref()
        .expect("harness: a maintainer root must be attached")
        .read()
        .await
        .set
        .members
        .clone()
}

/// **O3** — re-derived by `add_maintainer` / `remove_maintainer`.
pub async fn root_threshold(node: &Node) -> usize {
    node.maintainer_state
        .as_ref()
        .unwrap()
        .read()
        .await
        .set
        .threshold
}

/// **O4** — served as `getMaintainerSet.last_change_block`; the divergence
/// instrument. Membership can be right while this is wrong.
pub async fn root_last_updated(node: &Node) -> u64 {
    node.maintainer_state
        .as_ref()
        .unwrap()
        .read()
        .await
        .set
        .last_updated
}

/// **O5** — the seed arm; the apply path sets it on a SUCCESSFUL rotation only.
pub async fn last_derived_height(node: &Node) -> u64 {
    node.maintainer_state
        .as_ref()
        .unwrap()
        .read()
        .await
        .last_derived_height
}

/// **O6** — read the persisted trust root back INDEPENDENTLY of the in-memory
/// one. The updater reads this FILE to decide which keys may authorize a ROOT
/// BINARY INSTALL, so an in-memory-only result is undone by the next restart.
pub fn on_disk(dir: &TempDir) -> MaintainerState {
    MaintainerState::load(dir.path())
        .expect("O6: the persisted trust root must exist and still decode")
}

/// The LEGACY preimage, rebuilt from its FORMAT STRING here — character for
/// character the encoder
/// `bins/node/tests/inc_i_174_maintainer_undo.rs::maintainer_tx` uses. See the
/// module header for why it is not the crate's function.
pub fn legacy_message_independent(is_add: bool, target: &PublicKey) -> Vec<u8> {
    let action = if is_add { "add" } else { "remove" };
    format!("{}:{}", action, target.to_hex()).into_bytes()
}

/// `n` DISTINCT seated signers drawn from `candidates`.
///
/// Every height driven here is at or above #20, where `verify_multisig_at` counts
/// DISTINCT MEMBER SLOTS rather than signature ENTRIES (INC-I-172 M2 /
/// AUDIT-P0-010). Supplying the same key `n` times would be refused.
pub fn quorum<'a>(candidates: &'a [KeyPair], seated: &[PublicKey], n: usize) -> Vec<&'a KeyPair> {
    let picked: Vec<&KeyPair> = candidates
        .iter()
        .filter(|kp| seated.contains(kp.public_key()))
        .take(n)
        .collect();
    assert_eq!(
        picked.len(),
        n,
        "fixture: {n} DISTINCT seated signers are required — above #20 the \
         verifier counts member slots, not signature entries"
    );
    picked
}

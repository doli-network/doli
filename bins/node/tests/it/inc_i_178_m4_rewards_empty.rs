//! INC-I-178 M4 — REQ-BLS-004 / REQ-BLS-005: the empty-attendance detector in
//! `calculate_epoch_rewards` must classify the post-AH canonical-empty commitment as
//! COMPLETE attendance, and must keep classifying every other empty-bitfield block as
//! silent.
//!
//! OUTPUT CONTRACT
//!
//! F1: `Node::calculate_epoch_rewards(&self, epoch)
//!      -> Result<Vec<(u64, Hash)>, IncompleteEpochStoreError>`
//!   O1 return value — `Ok(outputs)` or `Err(IncompleteEpochStoreError)`
//!   O2 mutable params — NONE (`&self`, `epoch` by value)
//!   O3 receiver/self — read-only; asserted negatively via the epoch list length
//!   O4 persistent store writes — NONE (the function only reads block_store/utxo_set)
//!   O5 statics / O6 channels — NONE
//!   PATHS (per block in the epoch window):
//!     PA `presence_root.is_zero()`                     -> skip, no counter
//!     PB empty bitfield, root == canonical empty, h>=AH -> skip, no counter  (NEW)
//!     PC empty bitfield, root non-zero and not canonical -> silent, counter++, Err
//!     PD non-empty bitfield                             -> decode and credit
//!   INPUT PARTITIONS: the gate is read from the node field, never a literal; each
//!     case is run once with the gate ABOVE the whole epoch window (pre-AH) and once
//!     with it AT the window start (post-AH).
//!   MATRIX: O1 claimed for PA/PB/PC on both sides of the gate; O2-O6 structural,
//!     O3 asserted once per test.

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::reward_pool_pubkey_hash;
use doli_core::presence_commitment;
use doli_core::transaction::Output;
use doli_core::{Block, BlockHeader};
use doli_node::node::Node;
use storage::{Outpoint, UtxoEntry};
use vdf::{VdfOutput, VdfProof};

use crate::inc_i_178_m0_common::{make_node, N_SMALL};

/// A stored block: the epoch scan reads only `slot`, `presence_root` and the body
/// bitfield, so the rest of the header is fixture material.
fn stored_block(height: u64, producer: &PublicKey, root: Hash, bitfield: Vec<u8>) -> Block {
    let header = BlockHeader {
        version: 2,
        prev_hash: crypto::hash::hash(&height.to_le_bytes()),
        merkle_root: Hash::ZERO,
        presence_root: root,
        genesis_hash: Hash::ZERO,
        timestamp: 1_700_000_000 + height,
        slot: height as u32,
        producer: *producer,
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    let mut block = Block::new(header, Vec::new());
    block.attestation_bitfield = bitfield;
    block
}

fn full_bitfield(producer_count: usize) -> Vec<u8> {
    let indices: Vec<usize> = (0..producer_count).collect();
    doli_core::encode_attestation_bitfield_vec(&indices, producer_count)
}

async fn seed_reward_pool(node: &Node, amount: u64) {
    let entry = UtxoEntry {
        output: Output::normal(amount, reward_pool_pubkey_hash()),
        height: 0,
        is_coinbase: true,
        is_epoch_reward: false,
    };
    let mut utxo = node.utxo_set.write().await;
    utxo.insert(
        Outpoint::new(crypto::hash::hash(b"inc-i-178-m4-pool"), 0),
        entry,
    )
    .expect("insert pool UTXO");
}

/// The window under test: an epoch strictly after genesis, with every height filled.
struct Window {
    epoch: u64,
    start: u64,
    end: u64,
    last: u64,
}

fn window(node: &Node) -> Window {
    let bpe = node.config.network.blocks_per_reward_epoch();
    let epoch = (node.config.network.genesis_blocks() / bpe) + 2;
    let start = epoch * bpe;
    let end = (epoch + 1) * bpe;
    assert!(epoch > 0, "epoch 0 is exempt from the incompleteness check");
    assert!(
        end > start + 1,
        "the window needs a variant height AND a filler"
    );
    Window {
        epoch,
        start,
        end,
        last: end - 1,
    }
}

/// Fill the whole window with fully-attested blocks, then overwrite the LAST height
/// with the variant under test.
async fn fill(node: &Node, w: &Window, producers: &[KeyPair], variant: &Block) {
    let count = node.epoch_state.producer_list.len();
    for h in w.start..w.last {
        let b = stored_block(
            h,
            producers[0].public_key(),
            Hash::ZERO,
            full_bitfield(count),
        );
        let mut b = b;
        b.header.presence_root = crypto::hash::hash(&b.attestation_bitfield);
        node.block_store
            .put_block_canonical(&b, h)
            .expect("put_block_canonical");
    }
    node.block_store
        .put_block_canonical(variant, w.last)
        .expect("put_block_canonical (variant)");
}

async fn setup() -> (Node, Vec<KeyPair>, tempfile::TempDir, Window) {
    let (node, producers, tmp) = make_node(N_SMALL).await;
    seed_reward_pool(&node, 10_000_000_000).await;
    let w = window(&node);
    (node, producers, tmp, w)
}

// ===========================================================================
// PC x pre-AH — the recorded behaviour, byte-identical.
// ===========================================================================

/// REQ-BLS-005 (Must) — Decision: a failure means M4 relaxed the M-RC9 store-integrity
/// detector for blocks BELOW the activation height, so a node with a snap-sync gap would
/// silently distribute a reward set its peers do not compute — the divergence M-RC9
/// exists to refuse.
#[tokio::test]
async fn req_bls_005_m4_pre_ah_an_empty_bitfield_with_a_non_zero_root_still_aborts() {
    let (mut node, producers, _tmp, w) = setup().await;
    // Gate ABOVE the whole window: every height in it is strictly pre-AH.
    node.inc_i_178_attestation_bls_activation_height = w.end + 1;

    let silent = stored_block(
        w.last,
        producers[0].public_key(),
        presence_commitment(&[], &[]),
        Vec::new(),
    );
    fill(&node, &w, &producers, &silent).await;

    let verdict = node.calculate_epoch_rewards(w.epoch).await;
    assert!(
        verdict.is_err(),
        "pre-AH an empty bitfield with a non-zero root is silent and must abort the \
         epoch; got {:?}",
        verdict.map(|v| v.len())
    );
    assert_eq!(
        node.epoch_state.producer_list.len(),
        N_SMALL,
        "O3: the reward scan must not mutate node state"
    );
}

// ===========================================================================
// PB x post-AH — the new arm.
// ===========================================================================

/// REQ-BLS-004 (Must) — Decision: a failure means the first post-AH block whose producer
/// holds no pooled BLS signatures aborts the entire epoch's reward distribution for every
/// node that sees it. That is a fleet-wide reward halt triggered by an honest block, and
/// it is reachable on the very first slot after the gate.
#[tokio::test]
async fn req_bls_004_m4_post_ah_the_canonical_empty_is_complete_attendance() {
    let (mut node, producers, _tmp, w) = setup().await;
    // Gate AT the window start: every height in it is post-AH.
    node.inc_i_178_attestation_bls_activation_height = w.start;

    let zero_pooled = stored_block(
        w.last,
        producers[0].public_key(),
        presence_commitment(&[], &[]),
        Vec::new(),
    );
    assert!(
        !zero_pooled.header.presence_root.is_zero(),
        "the canonical empty is a real hash — if it were ZERO this test would only be \
         re-testing the legacy fast path"
    );
    fill(&node, &w, &producers, &zero_pooled).await;

    let verdict = node.calculate_epoch_rewards(w.epoch).await;
    assert!(
        verdict.is_ok(),
        "post-AH a zero-pooled block is complete attendance and must not abort the \
         epoch; got {:?}",
        verdict.err()
    );
    assert_eq!(
        node.epoch_state.producer_list.len(),
        N_SMALL,
        "O3: the reward scan must not mutate node state"
    );
}

/// REQ-BLS-004 (Must) — Decision: a failure means the new arm is not "exactly what the
/// `is_zero` fast path does today" — it credits or withholds something the sentinel path
/// does not, so the reward set depends on which of two encodings of "nobody attested"
/// the producer happened to emit.
#[tokio::test]
async fn req_bls_004_m4_post_ah_the_canonical_empty_matches_the_zero_root_fast_path() {
    let (mut node, producers, _tmp, w) = setup().await;
    node.inc_i_178_attestation_bls_activation_height = w.start;

    let sentinel = stored_block(w.last, producers[0].public_key(), Hash::ZERO, Vec::new());
    fill(&node, &w, &producers, &sentinel).await;
    let baseline = node
        .calculate_epoch_rewards(w.epoch)
        .await
        .expect("the Hash::ZERO fast path never aborts");
    assert!(
        !baseline.is_empty(),
        "the baseline must be a real distribution, or the comparison below is vacuous"
    );

    let canonical = stored_block(
        w.last,
        producers[0].public_key(),
        presence_commitment(&[], &[]),
        Vec::new(),
    );
    assert_ne!(
        canonical.hash(),
        sentinel.hash(),
        "the two encodings must be distinguishable, or the comparison is vacuous"
    );
    node.block_store
        .put_block_canonical(&canonical, w.last)
        .expect("overwrite the variant height");

    let got = node
        .calculate_epoch_rewards(w.epoch)
        .await
        .expect("post-AH the canonical empty must not abort the epoch");

    assert_eq!(
        got, baseline,
        "post-AH the canonical empty must produce the SAME distribution as the \
         Hash::ZERO fast path"
    );
}

// ===========================================================================
// PC x post-AH — the detector stays armed.
// ===========================================================================

/// REQ-BLS-004 (Must) — Decision: a failure means the store-integrity detector was
/// disarmed above the activation height. A header-only store or a snap-sync gap would
/// then be indistinguishable from complete attendance, and the node would compute a
/// reward set from partial data — INC-I-034's divergence, re-opened by the gate.
#[tokio::test]
async fn req_bls_004_m4_post_ah_a_non_canonical_empty_bitfield_is_still_silent() {
    let (mut node, producers, _tmp, w) = setup().await;
    node.inc_i_178_attestation_bls_activation_height = w.start;

    let bogus_root = crypto::hash::hash(b"neither zero nor the canonical empty");
    assert!(!bogus_root.is_zero(), "fixture: the root must be non-zero");
    assert_ne!(
        bogus_root,
        presence_commitment(&[], &[]),
        "fixture: the root must NOT be the canonical empty"
    );

    let silent = stored_block(w.last, producers[0].public_key(), bogus_root, Vec::new());
    fill(&node, &w, &producers, &silent).await;

    let verdict = node.calculate_epoch_rewards(w.epoch).await;
    assert!(
        verdict.is_err(),
        "post-AH an empty bitfield whose root is not the canonical empty is still a \
         store-integrity failure; got {:?}",
        verdict.map(|v| v.len())
    );
}

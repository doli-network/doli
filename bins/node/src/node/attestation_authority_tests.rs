//! Attestation authority ingress tests (INC-I-191 / INC-I-192, Seam A / [F1]).
//!
//! Verifies that BOTH attestation ingress sites derive the attester's authority
//! from the LOCAL ProducerSet instead of trusting the wire's self-declared
//! `attester_weight` / arbitrary attester pubkey.
//!
//! OUTPUT CONTRACT (per .claude/protocols/output-contract.md):
//!   Output 1: finality weight accumulator (SyncManager finality tracker,
//!             observed via `last_finalized_height()`).
//!     Path: on_new_attestation -> derive_attester_weight -> add_attestation_weight
//!   Output 2: minute attendance tracker (Node.minute_tracker,
//!             observed via `total_entries()`).
//!     Path: record_direct_attestation -> derive_attester_weight().is_some()
//!
//! INPUT PARTITIONS:
//!   For Output 1 (on_new_attestation):
//!     - forged non-member key, inflated attester_weight => MUST NOT finalize
//!     - (control) empty tracker before injection         => not finalized
//!   For Output 2 (record_direct_attestation):
//!     - non-member key        => MUST NOT be recorded (total_entries stays 0)
//!     - member (producer) key => MUST be recorded (INV-ATTEST-001: attend
//!                                regardless of selection_weight)

use super::*;
use tempfile::TempDir;

// --- Local test helpers (kept independent of fork_recovery_tests) ---

async fn make_test_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    (node, producers, temp)
}

fn build_test_block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
) -> Block {
    let reward = params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase));

    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash: params.genesis_hash,
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

    Block::new(header, vec![coinbase])
}

fn build_chain(
    start_height: u64,
    start_slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    count: usize,
    params: &ConsensusParams,
) -> Vec<Block> {
    let mut blocks = Vec::with_capacity(count);
    let mut prev = prev_hash;
    for i in 0..count {
        let h = start_height + i as u64;
        let s = start_slot + i as u32;
        let block = build_test_block(h, s, prev, producer, params);
        prev = block.hash();
        blocks.push(block);
    }
    blocks
}

// ============================================================
// A1 / INC-I-191: forged non-member weight must not reach finality
// ============================================================
#[tokio::test]
async fn test_forged_nonmember_attestation_does_not_gain_finality_weight() {
    let (mut node, producers, _tmp) = make_test_node(3).await;
    let params = node.params.clone();
    let genesis_hash = node.chain_state.read().await.best_hash;

    // Apply a short chain so the tip block is tracked for finality with a
    // real (positive) total network weight derived from the ProducerSet.
    let chain = build_chain(1, 1, genesis_hash, &producers[0], 5, &params);
    for block in &chain {
        node.apply_block(block.clone(), ValidationMode::Light)
            .await
            .expect("apply_block failed");
    }
    let tip = chain.last().unwrap();
    let tip_height = 5u64;

    // Sanity: nothing is finalized yet (no attestations accumulated).
    assert_eq!(
        node.sync_manager.read().await.last_finalized_height(),
        None,
        "precondition: no block finalized before any attestation"
    );

    // Attacker: a fresh keypair that is NOT in the ProducerSet, claiming a huge
    // self-declared weight. The signature over (block_hash, slot) is valid, so
    // Attestation::verify() passes — the authority is the ONLY missing check.
    let attacker = KeyPair::generate();
    let forged = doli_core::Attestation::new(
        tip.hash(),
        tip.header.slot,
        tip_height,
        1_000_000, // inflated self-declared weight
        attacker.private_key(),
        *attacker.public_key(),
    );
    assert!(
        forged.verify().is_ok(),
        "forged attestation is validly self-signed"
    );

    node.on_new_attestation(forged.to_bytes(), network::PeerId::random())
        .await;

    // The forged weight must NOT have been counted: the block stays un-finalized.
    assert_eq!(
        node.sync_manager.read().await.last_finalized_height(),
        None,
        "forged non-member weight must not finalize the block (INC-I-191)"
    );
}

// ============================================================
// A2 / INC-I-192: non-member DirectAttestation must not grow the tracker
// ============================================================
#[tokio::test]
async fn test_nonmember_direct_attestation_does_not_grow_minute_tracker() {
    let (mut node, _producers, _tmp) = make_test_node(3).await;

    assert_eq!(
        node.minute_tracker.total_entries(),
        0,
        "tracker starts empty"
    );

    // Attacker key not present in the ProducerSet.
    let attacker = KeyPair::generate();
    let att = doli_core::Attestation::new(
        Hash::ZERO, // ZERO => maybe_fetch short-circuits, no network needed
        7,
        0,
        1_000_000,
        attacker.private_key(),
        *attacker.public_key(),
    );

    node.record_direct_attestation(att, network::PeerId::random())
        .await;

    assert_eq!(
        node.minute_tracker.total_entries(),
        0,
        "non-member DirectAttestation must be dropped (INC-I-192 DoS)"
    );
}

// ============================================================
// A2 control: a genuine member IS still recorded (INV-ATTEST-001)
// ============================================================
#[tokio::test]
async fn test_member_direct_attestation_is_recorded() {
    let (mut node, producers, _tmp) = make_test_node(3).await;

    assert_eq!(
        node.minute_tracker.total_entries(),
        0,
        "tracker starts empty"
    );

    // producers[0] is a genuine ProducerSet member.
    let member = &producers[0];
    let att = doli_core::Attestation::new(
        Hash::ZERO,
        7,
        0,
        1, // honest weight is irrelevant; membership is what admits the record
        member.private_key(),
        *member.public_key(),
    );

    node.record_direct_attestation(att, network::PeerId::random())
        .await;

    assert_eq!(
        node.minute_tracker.total_entries(),
        1,
        "member DirectAttestation must be recorded regardless of weight"
    );
}

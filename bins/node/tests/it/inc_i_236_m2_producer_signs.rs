//! INC-I-236 M2 — the node signs every block it produces, and the node's own
//! validation ctx enforces the signature at devnet AH = 0.
//!
// covers: bins/node/src/node/production/mod.rs bins/node/src/node/production/assembly.rs bins/node/src/node/validation_checks/mod.rs bins/node/src/node/apply_block/mod.rs bins/node/src/node/block_handling.rs
//
// OUTPUT CONTRACT: Node::try_produce_block(&mut self)
//   O1 block written to block_store (header.vdf_proof.pi, producer, hash)
//   O2 chain_state.best_height advances; O3 return Ok(())
// OUTPUT CONTRACT: Node::validate_block_for_apply(&Block, height, mode)
//   O1 return Result<(), ValidationError>; O2 none (read-only)
// OUTPUT CONTRACT: Node::handle_new_block(Block, PeerId)
//   O1 chain_state.best_height / best_hash; O2 return
// INPUT PARTITIONS
//   IP-NET testnet (AH = u64::MAX, full production path) | devnet (AH = 0)
//   IP-PI  signed by producer | empty | corrupted 64 bytes
// MATRIX
//   O1,O2(produce) x testnet          -> req_auth_002_produced_block_is_signed_below_ah
//   O1(validate_for_apply) x devnet x {signed, empty} -> req_auth_001_node_validation_ctx_enforces_devnet_ah
//   O1(handle_new_block) x devnet x {empty, corrupted} then signed -> req_auth_sec_002_*

use crypto::KeyPair;
use doli_core::block::Block;
use doli_core::validation::{validate_block_with_mode, ValidationMode};
use network::PeerId;

use super::inc_i_236_m2_common::{
    build_devnet_block, hash_with_pi_cleared, is_sig_error, node_ctx, produce_one, producer_node,
    sign_into_pi, strict_verifies, testnet_producer_node, tip,
};

// REQ-AUTH-002 — Decision: a failure means producers emit unsigned blocks before the AH, so the rule cannot arm without halting the chain.
#[tokio::test]
async fn req_auth_002_produced_block_is_signed_below_ah() {
    let kp = KeyPair::generate();
    let (mut node, _tmp) = testnet_producer_node(&kp).await;
    assert_eq!(
        node.config
            .network
            .params()
            .inc_i_236_block_signature_activation_height,
        u64::MAX,
        "PRECONDITION: testnet AH is unpinned, so this block is below the AH"
    );
    let prev = tip(&node).await;
    let block = produce_one(&mut node).await;
    let height = node.chain_state.read().await.best_height;
    let off = node_ctx(&node, &block, prev, height, u64::MAX);
    let pre = validate_block_with_mode(&block, &off, ValidationMode::Full);
    assert!(
        pre.is_ok(),
        "NON-VACUITY: ctx must accept the block with the rule off; got {pre:?}"
    );

    assert_eq!(
        block.header.producer,
        *kp.public_key(),
        "producer must be the node key"
    );
    assert_eq!(
        block.header.vdf_proof.pi.len(),
        64,
        "REQ-AUTH-002: produced block must carry a 64-byte signature in vdf_proof.pi"
    );
    assert!(
        strict_verifies(&block),
        "REQ-AUTH-002: pi must strictly verify against header.producer over signing_message()"
    );
    assert_eq!(
        hash_with_pi_cleared(&block),
        block.hash(),
        "REQ-AUTH-002: the signature must not change header.hash()"
    );

    let on = node_ctx(&node, &block, prev, height, 0);
    let r = validate_block_with_mode(&block, &on, ValidationMode::Full);
    assert!(
        r.is_ok(),
        "REQ-AUTH-002: Full validation at AH = 0 must accept the produced block; got {r:?}"
    );
}

// REQ-AUTH-001 — Decision: a failure means validate_block_for_apply's ctx (validation_checks :299) keeps AH = u64::MAX, so the node never enforces the rule.
#[tokio::test]
async fn req_auth_001_node_validation_ctx_enforces_devnet_ah() {
    let kp = KeyPair::generate();
    let (mut src, _t1) = producer_node(&kp).await;
    let mut block = build_devnet_block(&mut src).await;
    let (node, _t2) = producer_node(&kp).await;
    let height = node.chain_state.read().await.best_height + 1;

    sign_into_pi(&mut block, &kp);
    let ok = node
        .validate_block_for_apply(&block, height, ValidationMode::Light)
        .await;
    assert!(
        ok.is_ok(),
        "NON-VACUITY: signed block must validate; got {ok:?}"
    );

    block.header.vdf_proof.pi.clear();
    let r = node
        .validate_block_for_apply(&block, height, ValidationMode::Light)
        .await;
    assert!(
        is_sig_error(&r),
        "REQ-AUTH-001: devnet (AH = 0) node validation must reject an unsigned block; got {r:?}"
    );
}

// REQ-AUTH-SEC-002 — Decision: a failure means an unsigned copy is applied (rule not enforced) or poisons the hash so the signed copy is refused.
#[tokio::test]
async fn req_auth_sec_002_unsigned_copy_first_does_not_block_signed_copy() {
    sec_002_case(|b| b.header.vdf_proof.pi.clear()).await;
}

// REQ-AUTH-SEC-002 — Decision: as above, for a corrupted 64-byte signature with the same hash().
#[tokio::test]
async fn req_auth_sec_002_corrupted_copy_first_does_not_block_signed_copy() {
    sec_002_case(|b| b.header.vdf_proof.pi = vec![0xA5; 64]).await;
}

async fn sec_002_case(corrupt: impl Fn(&mut Block)) {
    let kp = KeyPair::generate();
    let (mut src, _t1) = producer_node(&kp).await;
    let mut signed = build_devnet_block(&mut src).await;
    sign_into_pi(&mut signed, &kp);
    let mut bad = signed.clone();
    corrupt(&mut bad);
    assert_eq!(
        bad.hash(),
        signed.hash(),
        "PRECONDITION: both copies share hash()"
    );

    let (mut dst, _t2) = producer_node(&kp).await;
    dst.producer_key = None;
    let peer = PeerId::random();
    let h0 = dst.chain_state.read().await.best_height;

    let _ = dst.handle_new_block(bad, peer).await;
    assert_eq!(
        dst.chain_state.read().await.best_height,
        h0,
        "REQ-AUTH-SEC-002: a copy with a bad signature must not be applied"
    );

    dst.handle_new_block(signed.clone(), peer)
        .await
        .expect("handle_new_block must not error on the signed copy");
    let st = dst.chain_state.read().await;
    assert_eq!(
        st.best_height,
        h0 + 1,
        "REQ-AUTH-SEC-002: the signed copy must be applied"
    );
    assert_eq!(
        st.best_hash,
        signed.hash(),
        "REQ-AUTH-SEC-002: tip must be the signed block"
    );
}

//! INC-I-208 M1 — the builder must keep a copy of the attestation it broadcasts.
//!
//! Post AH 112,619 the presence bitfield is built only from `parent_sig_pool`
//! (`attestation/commit.rs` `pooled_commitment`), and the only pool insert lives in
//! the gossip `ingress`. The `startup` egress signs and broadcasts but never pools,
//! and gossipsub does not loop a node's own message back, so every block omits its
//! own builder's bit (`[ATTEST_MISS]`). The `assembly` caller `attest_own_block`
//! reaches the same egress, so both production call sites inherit the gap.
//!
//! covers: bins/node/src/node/startup.rs (egress), bins/node/src/node/attestation/ingress.rs (bls_verdict seam), bins/node/src/node/production/assembly.rs (attest_own_block caller)

// OUTPUT CONTRACT: fn create_and_broadcast_attestation(block_hash, slot, height)
// O1: Option<Attestation> — the returned attestation; Ed25519 half always, BLS half when bls_key is Some
// O2: self.parent_sig_pool — (block_hash, own_pubkey) -> 96-byte BLS half; the output the bug drops
// PATHS:
//   P1: on-chain bls_pubkey EQUALS the local bls_key public key (Valid) -> Some + pooled
//   P2: on-chain bls_pubkey is a DIFFERENT valid BLS key (Invalid)      -> Some + not pooled
//   P3: on-chain bls_pubkey EMPTY, the new_for_test default (NoKey)     -> Some + not pooled
// INPUT PARTITIONS:
//   P1: {onchain_bls = local bls_key public bytes}
//   P2: {onchain_bls = an unrelated BlsKeyPair::generate() public key}
//   P3: {onchain_bls = Vec::new()}
// MATRIX:
//   O1xP1: Some(att), att.attester == producers[0], att.bls_signature is 96 bytes
//   O1xP2: Some(att) — a key mismatch costs one attendance bit, never the block
//   O1xP3: Some(att) — an unregistered key costs one attendance bit, never the block
//   O2xP1: get(&block_hash, own_pk) == Some(att.bls_signature); total_signatures() == 1
//   O2xP2: get(&block_hash, own_pk) == None; total_signatures() == 0
//   O2xP3: get(&block_hash, own_pk) == None; total_signatures() == 0

use crypto::{BlsKeyPair, Hash, KeyPair};
use doli_node::node::Node;
use tempfile::TempDir;

async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    (node, producers, temp)
}

/// Write `bls_pubkey` for `producers[0]` into the test ProducerSet.
///
/// `register_genesis_producer` leaves it `Vec::new()`, which lands every test node
/// on the `NoKey` arm — the reproduction would go green for the wrong reason.
async fn set_onchain_bls_key(node: &Node, owner: &KeyPair, key_bytes: Vec<u8>) {
    let mut ps = node.producer_set.write().await;
    let info = ps
        .get_by_pubkey_mut(owner.public_key())
        .expect("producer exists");
    info.bls_pubkey = key_bytes;
}

fn local_bls_pubkey_bytes(node: &Node) -> Vec<u8> {
    node.bls_key
        .as_ref()
        .expect("new_for_test configures a BLS key")
        .public_key()
        .as_bytes()
        .to_vec()
}

/// P1 — REQ-208-001 / INC-I-208 — Decision: a failure means every post-AH block this
/// node builds still drops its own builder's attendance bit, because `pooled_commitment`
/// can only set a bit the pool holds.
#[tokio::test]
async fn egress_pools_own_bls_half_when_onchain_key_matches() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let block_hash = Hash::from_bytes([208u8; 32]);
    let own_pk = *producers[0].public_key();

    set_onchain_bls_key(&node, &producers[0], local_bls_pubkey_bytes(&node)).await;

    let att = node
        .create_and_broadcast_attestation(block_hash, 1, 1)
        .await
        .expect("active producer with weight > 0 must attest");

    assert_eq!(att.attester, own_pk, "attestation must carry our own key");
    assert_eq!(
        att.bls_signature.len(),
        96,
        "dual-signing must produce a 96-byte BLS half before pooling can be judged"
    );

    let pooled = node.parent_sig_pool.get(&block_hash, &own_pk);
    assert!(
        pooled.is_some(),
        "INC-I-208: the egress broadcast its own BLS half but never pooled it — \
         pooled_commitment sources bits ONLY from parent_sig_pool, so this block \
         omits its own builder's attendance bit"
    );
    assert_eq!(
        pooled.unwrap().as_slice(),
        att.bls_signature.as_slice(),
        "the pooled bytes must be the same half that was broadcast"
    );
    assert_eq!(
        node.parent_sig_pool.total_signatures(),
        1,
        "exactly one own signature, no extra parents"
    );
}

/// P2 — REQ-208-002 / INC-I-208 — Decision: a failure means a producer whose local BLS
/// key drifts from its on-chain key would emit an unverifiable aggregate and have every
/// block it produces rejected fleet-wide, turning a one-bit loss into total slot loss.
#[tokio::test]
async fn egress_does_not_pool_when_onchain_key_differs() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let block_hash = Hash::from_bytes([209u8; 32]);
    let own_pk = *producers[0].public_key();

    let other = BlsKeyPair::generate();
    set_onchain_bls_key(&node, &producers[0], other.public_key().as_bytes().to_vec()).await;
    assert_ne!(
        other.public_key().as_bytes().to_vec(),
        local_bls_pubkey_bytes(&node),
        "precondition: the on-chain key must differ from the local key"
    );

    let att = node
        .create_and_broadcast_attestation(block_hash, 1, 1)
        .await
        .expect("a key mismatch must not suppress the attestation");

    assert_eq!(att.attester, own_pk);
    assert!(
        node.parent_sig_pool.get(&block_hash, &own_pk).is_none(),
        "an unverifiable own half must never be pooled — the aggregate would fail \
         verification at every peer"
    );
    assert_eq!(node.parent_sig_pool.total_signatures(), 0);
}

/// P3 — REQ-208-003 / INV-ATTEST-001 — Decision: a failure means an unregistered BLS key
/// (`bls_pubkey` empty, the on-chain default) either panics the egress or pools an
/// unverifiable half, so a producer that never published a key loses its whole block.
#[tokio::test]
async fn egress_does_not_pool_when_onchain_key_is_unregistered() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let block_hash = Hash::from_bytes([210u8; 32]);
    let own_pk = *producers[0].public_key();

    {
        let ps = node.producer_set.read().await;
        assert!(
            ps.get_by_pubkey(&own_pk)
                .expect("producer exists")
                .bls_pubkey
                .is_empty(),
            "precondition: register_genesis_producer leaves bls_pubkey empty"
        );
    }

    let att = node
        .create_and_broadcast_attestation(block_hash, 1, 1)
        .await
        .expect("an unregistered BLS key must not suppress the attestation");

    assert_eq!(att.attester, own_pk);
    assert!(
        node.parent_sig_pool.get(&block_hash, &own_pk).is_none(),
        "no on-chain key means no verdict and no pooling"
    );
    assert_eq!(node.parent_sig_pool.total_signatures(), 0);
}

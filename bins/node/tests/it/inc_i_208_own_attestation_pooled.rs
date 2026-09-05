//! INC-I-208 — the builder keeps a copy of the attestation it broadcasts, and it does so
//! ONLY at or above an activation height.
//!
//! M1: post AH 112,619 the presence bitfield is built only from `parent_sig_pool`
//! (`attestation/commit.rs` `pooled_commitment`), and the only pool insert lived in the
//! gossip `ingress`. The `startup` egress signed and broadcast but never pooled, and
//! gossipsub does not loop a node's own message back, so every block omitted its own
//! builder's bit (`[ATTEST_MISS]`). The `assembly` caller `attest_own_block` reaches the
//! same egress, so both production call sites inherited the gap.
//!
//! M2: pooling its own half changes what an upgraded producer EMITS — its own bit in the
//! attestation bitfield, its own component in the aggregate, therefore `presence_root`,
//! which is inside `BlockHeader::hash()`. That is block CONTENT, so INV-DEPLOY-001 and
//! CLAUDE.md require an activation height: `inc_i_208_own_attestation_activation_height`
//! on `NetworkParams`, frozen at `u64::MAX` on Mainnet, Testnet and Devnet.
//!
//! covers: bins/node/src/node/startup.rs (the gated egress),
//! bins/node/src/node/production/assembly.rs (the `attest_own_block` caller),
//! crates/core/src/network_params/mod.rs (the field),
//! crates/core/src/network_params/defaults.rs (the three per-network literals),
//! crates/core/src/network_params/env_loader.rs (the exhaustive rebuild that must name
//! the field or the crate does not compile).
//!
//! HEIGHT PARTITIONING WITHOUT CONFIG PLUMBING. The shipped default is `u64::MAX` on
//! every network and `new_for_test` builds a Devnet node, so any ordinary height is
//! BELOW the gate and `u64::MAX` is exactly AT it. `create_and_broadcast_attestation`
//! already takes `height`, so the two partitions are two arguments and nothing here
//! overrides config. `height = u64::MAX` is safe: it reaches only
//! `derive_attester_weight` -> `ProducerInfo::selection_weight_at`
//! (crates/storage/src/producer/info.rs:389-406), which branches on `height >=
//! audit_activation` and does no arithmetic on `height`, so there is no debug overflow
//! and the weight stays non-zero.

// OUTPUT CONTRACT: fn create_and_broadcast_attestation(block_hash, slot, height)
// OUTPUTS:
//   O1 (return): Option<Attestation> — Ed25519 half always, BLS half when bls_key is Some
//   O2 (receiver): self.parent_sig_pool — (block_hash, own_pubkey) -> 96-byte BLS half.
//       The block-content output, and the ONLY output M2 gates.
//   O3 (receiver, via lock): self.sync_manager finality weight (add_attestation_weight).
//       DECLARED, NOT OBSERVABLE here — `SyncManager::finality_tracker` is private and
//       exposes no per-block weight getter. O1 == Some is the only available proxy: the
//       weight add and the return sit on one straight line with no branch between them,
//       so a gate that returns None would be caught, while a gate that early-returns
//       Some before the weight add would NOT be. Named so the gap is not silent.
//   O4 (network): gossip broadcast + DirectAttestation send — N/A in this harness;
//       `new_for_test` sets `network: None`, so neither call is reachable.
//   (no mutable params, no persistent-store writes, no global state.)
// AXES — the matrix is KEY-state x GATE-state:
//   KEY:  K-Valid   on-chain bls_pubkey EQUALS the local bls_key public key
//         K-Invalid on-chain bls_pubkey is a DIFFERENT valid BLS key
//         K-NoKey   on-chain bls_pubkey is EMPTY (the register_genesis_producer default)
//   GATE: G-Below   height < the frozen gate (any ordinary height)
//         G-At      height == the frozen gate, u64::MAX (pins the predicate inclusive)
// MATRIX — 6 cells, 4 CLAIMED:
//   K-Valid   x G-At    -> O1 Some, att.attester == own, 96-byte half; O2 pooled, total 1
//   K-Invalid x G-At    -> O1 Some; O2 empty, total 0        (a key mismatch costs a bit)
//   K-NoKey   x G-At    -> O1 Some; O2 empty, total 0        (unregistered costs a bit)
//   K-Valid   x G-Below -> O1 Some + 96-byte half; O2 empty, total 0        (THE RED)
// MATRIX — 2 cells DECLARED UNCLAIMED:
//   K-Invalid x G-Below, K-NoKey x G-Below. Below the gate O2 is empty for EVERY key
//   state, so both cells are dominated by K-Valid x G-Below: a test there would pass for
//   two independent reasons at once and could not distinguish which one held.
// WHY THE NEGATIVE CASES RUN AT THE GATE: below it they would pass for the gate's reason,
// not the key's, and would stop proving anything about the KEY axis.

use crypto::{BlsKeyPair, Hash, KeyPair};
use doli_node::node::Node;
use tempfile::TempDir;

/// An ordinary height, BELOW the frozen gate on every network.
const BELOW_THE_GATE: u64 = 1;

/// Exactly the frozen gate. Also pins the gate predicate as inclusive (`>=`, not `>`).
const AT_THE_GATE: u64 = u64::MAX;

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

/// K-Valid x G-Below — REQ-208-005 / INV-DEPLOY-001 — Decision: a failure means the M1
/// egress pools unconditionally, so an upgraded producer emits a different bit, a
/// different aggregate and therefore a different `presence_root` from a peer still on the
/// old binary at the SAME height; during any mixed-version window the two honest
/// producers build different bytes for the same block and the chain splits.
#[tokio::test]
async fn req_208_005_below_the_gate_the_egress_does_not_pool_its_own_bls_half() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let block_hash = Hash::from_bytes([211u8; 32]);
    let own_pk = *producers[0].public_key();

    set_onchain_bls_key(&node, &producers[0], local_bls_pubkey_bytes(&node)).await;

    let att = node
        .create_and_broadcast_attestation(block_hash, 1, BELOW_THE_GATE)
        .await
        .expect("O1: the gate is about POOLING; the attestation must still be produced");

    assert_eq!(
        att.attester, own_pk,
        "O1: the broadcast half is unchanged below the gate"
    );
    // Anti-vacuity: without a 96-byte half there would be nothing to pool, and the
    // empty-pool assertions below would hold for a reason that has nothing to do with
    // the gate.
    assert_eq!(
        att.bls_signature.len(),
        96,
        "O1: a BLS half MUST exist below the gate — the gate withholds it from the pool, \
         it does not stop dual-signing"
    );

    assert!(
        node.parent_sig_pool.get(&block_hash, &own_pk).is_none(),
        "O2 x K-Valid x G-Below: the egress pooled its own BLS half below the activation \
         height. That sets this producer's own bit, changing the aggregate and the \
         presence_root inside BlockHeader::hash() — block CONTENT — while every peer on \
         the previous binary builds the block without it (INV-DEPLOY-001)."
    );
    assert_eq!(
        node.parent_sig_pool.total_signatures(),
        0,
        "O2 x K-Valid x G-Below: nothing at all may enter the pool from the egress below \
         the gate, not our own half and not a stray parent"
    );
}

/// K-Valid x G-At — REQ-208-005 / REQ-208-001 — Decision: a failure means the gate is
/// exclusive (`>`) or is never reached, so once a height is pinned every post-AH block
/// this node builds still drops its own builder's attendance bit — `pooled_commitment`
/// can only set a bit the pool holds, and INC-I-208 is not actually fixed.
#[tokio::test]
async fn req_208_005_at_the_gate_the_egress_pools_its_own_bls_half() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let block_hash = Hash::from_bytes([208u8; 32]);
    let own_pk = *producers[0].public_key();

    set_onchain_bls_key(&node, &producers[0], local_bls_pubkey_bytes(&node)).await;

    let att = node
        .create_and_broadcast_attestation(block_hash, 1, AT_THE_GATE)
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
        "O2 x K-Valid x G-At: at the activation height the egress must pool the half it \
         broadcast. pooled_commitment sources bits ONLY from parent_sig_pool, so without \
         this insert the block omits its own builder's attendance bit. The height is \
         u64::MAX, exactly the gate, so this also fails if the predicate is `>` not `>=`."
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

/// K-Invalid x G-At — REQ-208-002 / INC-I-208 — Decision: a failure means a producer whose
/// local BLS key drifts from its on-chain key would emit an unverifiable aggregate and
/// have every block it produces rejected fleet-wide, turning a one-bit loss into total
/// slot loss. Run AT the gate on purpose: below it the empty pool would prove the gate,
/// not the key.
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
        .create_and_broadcast_attestation(block_hash, 1, AT_THE_GATE)
        .await
        .expect("a key mismatch must not suppress the attestation");

    assert_eq!(att.attester, own_pk);
    assert!(
        node.parent_sig_pool.get(&block_hash, &own_pk).is_none(),
        "an unverifiable own half must never be pooled — the aggregate would fail \
         verification at every peer, even at the activation height"
    );
    assert_eq!(node.parent_sig_pool.total_signatures(), 0);
}

/// K-NoKey x G-At — REQ-208-003 / INV-ATTEST-001 — Decision: a failure means an
/// unregistered BLS key (`bls_pubkey` empty, the on-chain default) either panics the
/// egress or pools an unverifiable half, so a producer that never published a key loses
/// its whole block. Run AT the gate for the same reason as the case above.
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
        .create_and_broadcast_attestation(block_hash, 1, AT_THE_GATE)
        .await
        .expect("an unregistered BLS key must not suppress the attestation");

    assert_eq!(att.attester, own_pk);
    assert!(
        node.parent_sig_pool.get(&block_hash, &own_pk).is_none(),
        "no on-chain key means no verdict and no pooling, at the gate or below it"
    );
    assert_eq!(node.parent_sig_pool.total_signatures(), 0);
}

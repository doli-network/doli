//! INC-I-237 M2 (D5) — the node's tx relay verdict: Accept iff EVERY tx in the
//! gossip message was NEWLY admitted to the mempool by THIS message.
//!
//! OUTPUT CONTRACT: async fn Node::handle_gossip_txs(&self, Vec<Transaction>) -> network::Verdict
//!   O1: none (txs moved in)
//!   O2: self.mempool membership + the stored bytes of each admitted tx
//!   O3: Verdict::{Accept, Ignore} (the type has no Reject: SEC-004 / F-4)
//!   O4: none   O5: none
//!   O6: network commands — none here (network = None); the relay file proves
//!       zero re-publish and the verdict send on a live swarm
//! PATHS: P1 all new -> Accept; P2 any AlreadyExists -> Ignore;
//!   P3 any refused (bad signature, unfunded, unmineable type) -> Ignore;
//!   P4 empty message -> Ignore; P5 same tx twice in one message -> Ignore
//! INPUT PARTITIONS: size {0, 1, 2}; residency {fresh, byte-identical resident,
//!   signature-malleated variant of a resident (same tx.hash())}; refusal
//!   {invalid signature, missing UTXO, SlashProducer (W01)}; order {new first,
//!   resident first}
//! MATRIX: O3 x every partition, O2 x every partition (admission is per tx and
//!   never undone by the verdict) — asserted below.
//! M8 (AUDIT-P1-505, auth AH 0): P6 forged zero-flow tx -> refused + Ignore;
//!   genuine DelegateBond -> admitted + Accept; partitions {4 zero-flow types,
//!   forged/genuine, 1/100 per message, with a fee-paying sibling}.

use crypto::{Hash, KeyPair};
use doli_core::maintainer::MaintainerSignature;
use doli_core::transaction::RevokeDelegationData;
use doli_core::{BlockHeader, Transaction};
use doli_node::node::Node;
use network::{EquivocationProof, Verdict};

use super::inc_i_237_m2_node_common as c;
use super::inc_i_237_m8_common as m8;

async fn has(node: &Node, tx: &Transaction) -> bool {
    node.mempool.read().await.contains(&tx.hash())
}

async fn stored(node: &Node, tx: &Transaction) -> Option<Vec<u8>> {
    node.mempool
        .read()
        .await
        .get(&tx.hash())
        .map(|e| e.tx.serialize())
}

/// Two admissible transfers on a node funded for both.
async fn funded() -> (Node, Transaction, Transaction, tempfile::TempDir) {
    let (node, kp, t) = c::make_node().await;
    c::seed(&node, &kp, c::prev(1)).await;
    c::seed(&node, &kp, c::prev(2)).await;
    let (x, y) = (c::transfer(&kp, c::prev(1)), c::transfer(&kp, c::prev(2)));
    (node, x, y, t)
}

fn slash_tx() -> Transaction {
    let header = |tag: u8| BlockHeader {
        version: 2,
        prev_hash: Hash::from_bytes([1; 32]),
        merkle_root: Hash::from_bytes([tag; 32]),
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 1,
        slot: 77,
        producer: *KeyPair::from_seed([9; 32]).public_key(),
        vdf_output: vdf::VdfOutput { value: vec![0; 32] },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    EquivocationProof {
        producer: *KeyPair::from_seed([9; 32]).public_key(),
        block_header_1: header(0xA1),
        block_header_2: header(0xB2),
        slot: 77,
    }
    .to_slash_transaction(&KeyPair::generate())
}

// REQ-GRV-001 / D5 — Decision: a failure means a message of brand-new valid txs (single or batch) is not Accepted, so the node never relays valid content and the one-hop gap stays.
#[tokio::test]
async fn d5_all_newly_admitted_message_accepts() {
    let (node, x, _y, _t) = funded().await;
    assert_eq!(
        node.handle_gossip_txs(vec![x.clone()]).await,
        Verdict::Accept,
        "1 new tx"
    );
    assert!(has(&node, &x).await);
    let (node, x, y, _t) = funded().await;
    let v = node.handle_gossip_txs(vec![x.clone(), y.clone()]).await;
    assert_eq!(v, Verdict::Accept, "2 new txs");
    assert!(has(&node, &x).await && has(&node, &y).await);
}

// F-5 / REQ-GRV-005 — Decision: a failure means mempool residency (AlreadyExists) authorizes a forward, so every re-delivery or recomposition of paid content is relayed again.
#[tokio::test]
async fn f_5_already_resident_tx_never_authorizes_a_forward() {
    let (node, x, _y, _t) = funded().await;
    assert_eq!(
        node.handle_gossip_txs(vec![x.clone()]).await,
        Verdict::Accept
    );
    let before = node.mempool.read().await.len();
    assert_eq!(
        node.handle_gossip_txs(vec![x.clone()]).await,
        Verdict::Ignore,
        "byte-identical resident"
    );
    assert_eq!(
        node.mempool.read().await.len(),
        before,
        "nothing new admitted"
    );
}

// F-5 / REQ-GRV-SEC-002 — Decision: a failure means a garbage-signature variant sharing a resident tx's hash (signatures are outside tx.hash()) is relayed, or replaces the genuine bytes in the mempool.
#[tokio::test]
async fn f_5_signature_malleated_variant_of_a_resident_tx_is_ignored() {
    let (node, x, _y, _t) = funded().await;
    assert_eq!(
        node.handle_gossip_txs(vec![x.clone()]).await,
        Verdict::Accept
    );
    let forged = c::malleate(&x);
    assert_eq!(forged.hash(), x.hash(), "fixture: same tx.hash()");
    assert_eq!(node.handle_gossip_txs(vec![forged]).await, Verdict::Ignore);
    assert_eq!(
        stored(&node, &x).await,
        Some(x.serialize()),
        "genuine bytes kept"
    );
}

// REQ-GRV-SEC-002 — Decision: a failure means a forged variant seen first suppresses (or poisons the residency of) the genuine tx, so an attacker can censor a victim's tx at every honest hop.
#[tokio::test]
async fn req_grv_sec_002_forged_variant_first_does_not_suppress_the_genuine() {
    let (node, x, _y, _t) = funded().await;
    let forged = c::malleate(&x);
    assert_eq!(
        node.handle_gossip_txs(vec![forged]).await,
        Verdict::Ignore,
        "forged first"
    );
    assert!(
        !has(&node, &x).await,
        "a forged signature is never admitted"
    );
    assert_eq!(
        node.handle_gossip_txs(vec![x.clone()]).await,
        Verdict::Accept,
        "genuine after"
    );
    assert_eq!(stored(&node, &x).await, Some(x.serialize()));
}

// REQ-GRV-SEC-001 / F-4 — Decision: a failure means a tx no node can admit (bad signature, unfunded) is relayed on its sender's word, or the verdict path grows a Reject that penalises honest mixed-version peers.
#[tokio::test]
async fn req_grv_sec_001_unadmittable_txs_are_ignored_and_never_admitted() {
    let (node, kp, _t) = c::make_node().await;
    c::seed(&node, &kp, c::prev(3)).await;
    let bad_sig = c::malleate(&c::transfer(&kp, c::prev(3)));
    let unfunded = c::transfer(&kp, c::prev(4));
    for (row, tx) in [("bad signature", bad_sig), ("missing UTXO", unfunded)] {
        assert_eq!(
            node.handle_gossip_txs(vec![tx.clone()]).await,
            Verdict::Ignore,
            "{row}"
        );
        assert!(!has(&node, &tx).await, "{row} admitted");
    }
}

// D5 / C2 — Decision: a failure means one fresh tx carries resident or refused siblings across the hop (up to 1 MiB per forward), or the verdict starts undoing per-tx admission the mempool already did.
#[tokio::test]
async fn d5_mixed_messages_are_ignored_but_admission_stays_per_tx() {
    let (node, x, y, _t) = funded().await;
    assert_eq!(
        node.handle_gossip_txs(vec![x.clone()]).await,
        Verdict::Accept
    );
    let v = node.handle_gossip_txs(vec![x.clone(), y.clone()]).await;
    assert_eq!(v, Verdict::Ignore, "resident first, new second");
    assert!(has(&node, &y).await, "the new sibling is still admitted");
    let (node, x, y, _t) = funded().await;
    let refused = c::malleate(&y);
    let v = node.handle_gossip_txs(vec![x.clone(), refused]).await;
    assert_eq!(v, Verdict::Ignore, "new first, refused second");
    assert!(has(&node, &x).await && !has(&node, &y).await);
}

// D5 — Decision: a failure means an empty or self-duplicating message passes a vacuous "every tx is new" check and is relayed as junk.
#[tokio::test]
async fn d5_empty_and_self_duplicating_messages_are_ignored() {
    let (node, x, _y, _t) = funded().await;
    assert_eq!(
        node.handle_gossip_txs(Vec::new()).await,
        Verdict::Ignore,
        "empty"
    );
    let v = node.handle_gossip_txs(vec![x.clone(), x.clone()]).await;
    assert_eq!(v, Verdict::Ignore, "same tx twice in one message");
    assert!(has(&node, &x).await, "admitted once");
}

// W01 — Decision: a failure means a gossiped SlashProducer (unmineable at every height) is admitted and so relayed: admission became relay authority in M2, so this re-opens the INC-I-237 flood.
#[tokio::test]
async fn w01_gossiped_slash_producer_is_ignored_and_not_admitted() {
    let (node, x, _y, _t) = funded().await;
    let slash = slash_tx();
    assert_eq!(
        node.handle_gossip_txs(vec![slash.clone()]).await,
        Verdict::Ignore
    );
    assert!(
        !has(&node, &slash).await,
        "SlashProducer admitted from gossip"
    );
    let v = node.handle_gossip_txs(vec![x.clone(), slash.clone()]).await;
    assert_eq!(v, Verdict::Ignore, "a valid sibling does not carry it");
    assert!(!has(&node, &slash).await);
}

// REQ-GRV-008 / F-6 — Decision: a failure means node code builds the verdict command itself (and can .await it on a full channel, the C1 cycle) instead of the one try_send helper, or no node path sends a verdict at all.
#[test]
fn req_grv_008_node_sends_verdicts_only_through_the_try_send_helper() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let (mut literal, mut helper) = (Vec::new(), 0);
    let mut stack = vec![src];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).expect("read src").flatten() {
            let p = e.path();
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            if p.is_dir() {
                stack.push(p);
            } else if name.ends_with(".rs") && !name.ends_with("_tests.rs") && name != "tests.rs" {
                let s = std::fs::read_to_string(&p).unwrap();
                if s.contains("NetworkCommand::GossipVerdict") {
                    literal.push(p.display().to_string());
                }
                helper += s.matches("send_gossip_verdict(").count();
            }
        }
    }
    assert!(
        literal.is_empty(),
        "verdict built outside the try_send helper: {literal:?}"
    );
    assert!(helper > 0, "no node path sends a gossip verdict");
}

/// The other admissible zero-flow types, each with garbage signatures.
fn forged_other_zero_flow() -> Vec<(&'static str, Transaction)> {
    let target = *KeyPair::generate().public_key();
    let sigs: Vec<MaintainerSignature> = (1..=3u8)
        .map(|i| MaintainerSignature {
            pubkey: *KeyPair::from_seed([i; 32]).public_key(),
            signature: crypto::Signature::from_bytes([0xE0 ^ i; 64]),
        })
        .collect();
    let mut revoke = RevokeDelegationData::new(*KeyPair::generate().public_key(), target);
    revoke.signature = crypto::Signature::from_bytes([0xD1; 64]);
    vec![
        (
            "RevokeDelegation",
            Transaction::new_revoke_delegation(revoke),
        ),
        (
            "AddMaintainer",
            Transaction::new_add_maintainer(target, sigs.clone()),
        ),
        (
            "RemoveMaintainer",
            Transaction::new_remove_maintainer(target, sigs, None),
        ),
    ]
}

// REQ-GRV-SEC-008 / REQ-GRV-029 (AUDIT-P1-505) — Decision: a failure means a forged zero-flow tx is admitted or authorizes a forward, or a genuine one (signature verified at admission) still stops at hop 1.
#[tokio::test]
async fn m5_audit_p1_505_forged_zero_flow_tx_never_authorizes_a_forward() {
    let (node, kp, _m, _t) = m8::auth_node().await;
    c::seed(&node, &kp, c::prev(1)).await;
    c::seed(&node, &kp, c::prev(2)).await;
    let (x, y) = (c::transfer(&kp, c::prev(1)), c::transfer(&kp, c::prev(2)));
    let before = node.mempool.read().await.len();
    let forged = c::forged_delegation(1);
    let v = node.handle_gossip_txs(vec![forged.clone()]).await;
    assert_eq!(v, Verdict::Ignore, "AUDIT-P1-505: forged DelegateBond");
    assert!(
        !has(&node, &forged).await,
        "REQ-GRV-SEC-008: forged DelegateBond admitted"
    );
    let batch: Vec<Transaction> = (100..200).map(c::forged_delegation).collect();
    let v = node.handle_gossip_txs(batch.clone()).await;
    assert_eq!(v, Verdict::Ignore, "AUDIT-P1-505: 100 forged DelegateBonds");
    for (row, tx) in forged_other_zero_flow() {
        let v = node.handle_gossip_txs(vec![tx.clone()]).await;
        assert_eq!(v, Verdict::Ignore, "AUDIT-P1-505: forged {row}");
        assert!(
            !has(&node, &tx).await,
            "REQ-GRV-SEC-008: forged {row} admitted"
        );
    }
    assert_eq!(
        node.mempool.read().await.len(),
        before,
        "REQ-GRV-SEC-008: no forged zero-flow tx admitted"
    );
    let genuine = m8::genuine_delegate_bond(&[&node]).await;
    let v = node.handle_gossip_txs(vec![genuine.clone()]).await;
    assert_eq!(
        v,
        Verdict::Accept,
        "REQ-GRV-029: a genuine DelegateBond authorizes a forward"
    );
    assert!(has(&node, &genuine).await, "genuine DelegateBond admitted");
    let v = node
        .handle_gossip_txs(vec![x.clone(), c::forged_delegation(2)])
        .await;
    assert_eq!(
        v,
        Verdict::Ignore,
        "a fee-paying sibling does not carry a forged zero-flow tx"
    );
    assert!(
        has(&node, &x).await,
        "the fee-paying sibling is still admitted"
    );
    let v = node.handle_gossip_txs(vec![y.clone()]).await;
    assert_eq!(
        v,
        Verdict::Accept,
        "control: a new fee-paying tx alone still relays"
    );
}

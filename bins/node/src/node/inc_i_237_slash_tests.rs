//! INC-I-237 — SlashProducer flood. Devnet node: the INC-I-255 slash gate is 0,
//! so one equivocation yields exactly one canonical slash tx, never more.
//!
//! OUTPUT CONTRACT
//!   fn Node::handle_equivocation(&mut self, EquivocationProof)
//!     O1 receiver: mempool SlashProducer count (broadcast is gated on it)
//!   fn Node::handle_new_block(&mut self, Block, PeerId) — equivocation arm
//!     O1 receiver: mempool SlashProducer count
//!     O2 detector still records the proof
//!   fn Node::handle_gossip_txs(&self, Vec<Transaction>) -> Verdict (M2)
//!     O1 receiver: mempool membership   O3 verdict (relay is gated on it)
//!
//! INPUT PARTITIONS
//!   P1 VDF-committed pair (merkle_root differs)   -> exactly one slash tx
//!   P2 presence_root-only pair (same VDF input)   -> exactly one slash tx
//!   P3 reporter-signed slash tx arriving by gossip -> not admitted

use super::*;
use doli_core::tpop::heartbeat::verify_hash_chain_vdf;
use tempfile::TempDir;

const SLOT: u32 = 77;

async fn node(producers: &[KeyPair]) -> (Node, TempDir) {
    let temp = TempDir::new().unwrap();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers.to_vec())
        .await
        .expect("Node::new_for_test failed");
    (node, temp)
}

/// A header whose VDF output is REAL for the node's network (Devnet).
fn header(by: &KeyPair, prev: Hash, merkle: Hash, presence: Hash) -> BlockHeader {
    let mut h = BlockHeader {
        version: 2,
        prev_hash: prev,
        merkle_root: merkle,
        presence_root: presence,
        genesis_hash: doli_core::chainspec::ChainSpec::devnet().genesis_hash(),
        timestamp: SLOT as u64 * 10,
        slot: SLOT,
        producer: *by.public_key(),
        vdf_output: VdfOutput { value: vec![] },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    let out = hash_chain_vdf(&h.vdf_input(), Network::Devnet.heartbeat_vdf_iterations());
    h.vdf_output = VdfOutput {
        value: out.to_vec(),
    };
    crate::node::test_block_sig::sign_header(&mut h, by);
    h
}

fn vdf_ok(h: &BlockHeader) -> bool {
    let out: [u8; 32] = h.vdf_output.value.as_slice().try_into().unwrap();
    verify_hash_chain_vdf(
        &h.vdf_input(),
        &out,
        Network::Devnet.heartbeat_vdf_iterations(),
    )
}

async fn slash_count(node: &Node) -> usize {
    node.mempool
        .read()
        .await
        .iter()
        .filter(|(_, e)| e.tx.tx_type == TxType::SlashProducer)
        .count()
}

// T1 — handle_equivocation builds exactly one canonical slash tx.
#[tokio::test]
async fn test_handle_equivocation_creates_no_unmineable_slash_tx() {
    let producers: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
    let h1 = header(
        &producers[0],
        crypto_hash(b"p"),
        crypto_hash(b"m-A"),
        Hash::ZERO,
    );
    let h2 = header(
        &producers[0],
        crypto_hash(b"p"),
        crypto_hash(b"m-B"),
        Hash::ZERO,
    );
    assert!(vdf_ok(&h1) && vdf_ok(&h2), "setup: both VDFs valid");
    let proof = EquivocationProof {
        producer: *producers[0].public_key(),
        block_header_1: h1,
        block_header_2: h2,
        slot: SLOT,
    };
    let (mut n, _t) = node(&producers).await;
    // Non-vacuity: the producer is registered, so the old code DID submit.
    assert!(n
        .producer_set
        .read()
        .await
        .get_by_pubkey(producers[0].public_key())
        .is_some());
    n.handle_equivocation(proof).await;
    let got = slash_count(&n).await;
    assert_eq!(
        got, 1,
        "INC-I-237: handle_equivocation put {got} SlashProducer tx(s) in the mempool; \
         exactly one canonical tx is allowed"
    );
}

// T2 — a slash tx that arrives by gossip is not admitted, so it is not relayed.
#[tokio::test]
async fn test_handle_new_transaction_does_not_admit_gossiped_slash() {
    let producers: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
    let h1 = header(
        &producers[0],
        crypto_hash(b"p"),
        crypto_hash(b"m-A"),
        Hash::ZERO,
    );
    let h2 = header(
        &producers[0],
        crypto_hash(b"p"),
        crypto_hash(b"m-B"),
        Hash::ZERO,
    );
    let tx = EquivocationProof {
        producer: *producers[0].public_key(),
        block_header_1: h1,
        block_header_2: h2,
        slot: SLOT,
    }
    .to_slash_transaction(&KeyPair::generate());
    let (peer, _t) = node(&producers).await;
    assert_eq!(
        peer.handle_gossip_txs(vec![tx.clone()]).await,
        network::Verdict::Ignore,
        "M2 W01: a gossiped SlashProducer is never relayed"
    );
    assert!(
        !peer.mempool.read().await.contains(&tx.hash()),
        "gossiped SlashProducer must not enter the mempool"
    );
}

// T3 — through the real block entry point, each pair shape yields exactly one
// slash tx; the detector records the proof.
#[tokio::test]
async fn test_handle_new_block_equivocation_yields_no_slash_tx() {
    let producers: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
    let peer = PeerId::random();
    let p = crypto_hash(b"p");
    let m = crypto_hash(b"m");
    let pairs = [
        (
            "merkle_root pair",
            header(&producers[0], p, crypto_hash(b"m-A"), Hash::ZERO),
            header(&producers[0], p, crypto_hash(b"m-B"), Hash::ZERO),
        ),
        (
            "presence_root-only pair",
            header(&producers[0], p, m, crypto_hash(b"attest-set-A")),
            header(&producers[0], p, m, crypto_hash(b"attest-set-B")),
        ),
    ];
    for (name, h1, h2) in pairs {
        assert!(vdf_ok(&h1) && vdf_ok(&h2), "{name}: setup VDFs valid");
        let (mut n, _t) = node(&producers).await;
        let _ = n.handle_new_block(Block::new(h1, vec![]), peer).await;
        let _ = n.handle_new_block(Block::new(h2, vec![]), peer).await;
        let detected = n
            .equivocation_detector
            .write()
            .await
            .take_pending_proofs()
            .len();
        assert_eq!(detected, 1, "{name}: setup — equivocation must be detected");
        let got = slash_count(&n).await;
        assert_eq!(
            got, 1,
            "INC-I-237 {name}: produced {got} SlashProducer tx(s)"
        );
    }
}

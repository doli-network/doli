// INC-I-281 R-4 M3a.1 (Must): a snap-synced node stores its anchor as the real header with
// an empty body; the three block-serving sync requests answer that pair as stored, and the
// requester's merkle check refuses it.
//
// OUTPUT CONTRACT: Node::serve_block_by_height / serve_block_by_hash / serve_bodies (the
// channel-free seam the GetBlockByHeight / GetBlockByHash / GetBodies arms of
// handle_sync_request delegate to); requester side Block::verify_merkle_root,
// Node::validate_block_for_apply (Light, Full), Node::apply_block.
// OUTPUTS:
//   O1 SyncResponse variant and payload per request kind (Block(None|Some), Bodies(..))
//   O2 a served block equals the stored block (header and body unchanged)
//   O3 requester verdict: InvalidMerkleRoot { header: header root, computed: root([]) }
//   O4 requester chain tip and store after the refused apply
// PATHS: by height | by hash | bodies (x stored block kind)
// INPUT PARTITIONS (stored block):
//   S stub: real header (merkle_root != root([])), empty body -> not served (None / omitted)
//   E consistent empty: merkle_root == root([]), empty body   -> served unchanged
//   F full block                                              -> served unchanged
//   N absent                                                  -> None / omitted
// MATRIX: S x {height, hash, bodies} x O1; E, F x {height, hash, bodies} x {O1, O2};
//   N x {height, hash, bodies} x O1; S x requester x {O3, O4}.
// COINBASE: every applied block carries a coinbase (validate_block_economics refuses an
//   empty body, ECON_COINBASE_MISSING), so the filter keys on header/body consistency.
// covers: block_serve.rs, validation_checks/mod.rs, node/mod.rs, fork_recovery.rs

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;

use std::collections::HashMap;

use crypto::{Hash, KeyPair};
use doli_core::block::compute_merkle_root;
use doli_core::consensus::ConsensusParams;
use doli_core::validation::{ValidationError, ValidationMode};
use doli_core::Block;
use doli_node::node::Node;
use inc_i_236_m2_reorg_support::{block, Sig};
use network::protocols::SyncResponse;
use tempfile::TempDir;

/// Base h=1..=BASE_TOP; the anchor sits at ANCHOR_H.
const BASE_TOP: u64 = 5;
const ANCHOR_H: u64 = BASE_TOP + 1;

struct Fx {
    node: Node,
    keys: Vec<KeyPair>,
    params: ConsensusParams,
    base: Vec<Block>,
    _tmp: TempDir,
}

fn slot_of(h: u64) -> u32 {
    100 + h as u32
}

async fn node_with_base(keys: &[KeyPair], base: &[Block]) -> (Node, TempDir) {
    let tmp = TempDir::new().unwrap();
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), keys.to_vec())
        .await
        .expect("Node::new_for_test");
    for b in base {
        node.apply_block(b.clone(), ValidationMode::Light)
            .await
            .expect("fixture: base block");
    }
    (node, tmp)
}

async fn fixture() -> Fx {
    let keys: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
    let (mut node, tmp) = node_with_base(&keys, &[]).await;
    let params = node.params.clone();
    let mut base = Vec::new();
    let mut prev = Hash::ZERO;
    for h in 1..=BASE_TOP {
        let b = block(h, slot_of(h), prev, &keys[0], Sig::Valid, &params);
        prev = b.hash();
        node.apply_block(b.clone(), ValidationMode::Light)
            .await
            .expect("fixture: base block");
        base.push(b);
    }
    Fx {
        node,
        keys,
        params,
        base,
        _tmp: tmp,
    }
}

impl Fx {
    /// The real block at ANCHOR_H on the base tip (coinbase body).
    fn anchor(&self) -> Block {
        let prev = self.base.last().unwrap().hash();
        block(
            ANCHOR_H,
            slot_of(ANCHOR_H),
            prev,
            &self.keys[0],
            Sig::Valid,
            &self.params,
        )
    }

    /// A signed header at ANCHOR_H whose merkle_root is root([]), with an empty body.
    fn consistent_empty(&self) -> Block {
        let mut header = self.anchor().header;
        header.merkle_root = compute_merkle_root(&[]);
        block_sig::sign_header(&mut header, &self.keys[0]);
        bare(header)
    }

    /// The snap-anchor write: canonical index, then put_block of `b` at ANCHOR_H.
    fn store_as_anchor(&self, b: &Block) {
        let store = self.node.block_store();
        store
            .seed_canonical_index(b.hash(), ANCHOR_H)
            .expect("seed_canonical_index");
        store.put_block(b, ANCHOR_H).expect("put_block");
        let stored = store
            .get_block_by_height(ANCHOR_H)
            .expect("store")
            .expect("precondition: block indexed at ANCHOR_H");
        assert_eq!(&stored, b, "precondition: stored as written");
    }
}

/// The block the snap path persists for an anchor header (fork_recovery.rs, Option C).
fn bare(header: doli_core::BlockHeader) -> Block {
    Block {
        header,
        transactions: vec![],
        aggregate_bls_signature: vec![],
        attestation_bitfield: vec![],
    }
}

fn summary(resp: &SyncResponse) -> String {
    let one = |b: &Block| format!("{:.16}/txs={}", b.hash(), b.transactions.len());
    match resp {
        SyncResponse::Block(None) => "Block(None)".into(),
        SyncResponse::Block(Some(b)) => format!("Block(Some({}))", one(b)),
        SyncResponse::Bodies(v) => format!("Bodies({:?})", v.iter().map(one).collect::<Vec<_>>()),
        _ => "other variant".into(),
    }
}

/// True when `resp` carries a block with `hash` and an empty body.
fn serves_stub(resp: &SyncResponse, hash: &Hash) -> bool {
    let is_stub = |b: &Block| b.hash() == *hash && b.transactions.is_empty();
    match resp {
        SyncResponse::Block(Some(b)) => is_stub(b),
        SyncResponse::Bodies(v) => v.iter().any(is_stub),
        _ => false,
    }
}

fn probe(kind: &str, served: bool) {
    println!("INC281_PROBE stubserve={kind} served={}", served as u8);
}

fn check_none(resp: &SyncResponse, label: &str, wrong: &mut Vec<String>) {
    if !matches!(resp, SyncResponse::Block(None)) {
        wrong.push(format!("{label}: {}", summary(resp)));
    }
}

/// `resp` is Block(Some(b)) with b == `want`.
fn check_block(resp: &SyncResponse, want: &Block, label: &str, wrong: &mut Vec<String>) {
    match resp {
        SyncResponse::Block(Some(b)) if b == want => {}
        _ => wrong.push(format!(
            "{label}: {} (want {:.16})",
            summary(resp),
            want.hash()
        )),
    }
}

/// `resp` is Bodies holding exactly `want` (by hash, each unchanged).
fn check_bodies(resp: &SyncResponse, want: &[Block], label: &str, wrong: &mut Vec<String>) {
    let SyncResponse::Bodies(got) = resp else {
        wrong.push(format!("{label}: {}", summary(resp)));
        return;
    };
    let by_hash: HashMap<Hash, &Block> = got.iter().map(|b| (b.hash(), b)).collect();
    let exact = got.len() == want.len()
        && want
            .iter()
            .all(|w| by_hash.get(&w.hash()).is_some_and(|b| *b == w));
    if !exact {
        wrong.push(format!("{label}: {}", summary(resp)));
    }
}

// INC-I-281 R-4 M3a.1 (Must) — Decision: a failure means a node still serves its snap anchor as the real header with an empty body, a pair every requester refuses on the merkle check, so a peer that asks this node for that height never gets a usable block.
#[tokio::test]
async fn stub_anchor_is_not_served() {
    let fx = fixture().await;
    let anchor = fx.anchor();
    let stub = bare(anchor.header.clone());
    assert_eq!(stub.hash(), anchor.hash(), "precondition: same header hash");
    assert!(
        !stub.verify_merkle_root(),
        "precondition: the stub's header root is not root([])"
    );
    fx.store_as_anchor(&stub);
    let good = [fx.base[3].clone(), fx.base[4].clone()];
    let hash = stub.hash();

    let by_height: SyncResponse = fx.node.serve_block_by_height(ANCHOR_H);
    let by_hash: SyncResponse = fx.node.serve_block_by_hash(&hash);
    let bodies: SyncResponse = fx
        .node
        .serve_bodies(vec![good[0].hash(), hash, good[1].hash()]);
    probe("height", serves_stub(&by_height, &hash));
    probe("hash", serves_stub(&by_hash, &hash));
    probe("bodies", serves_stub(&bodies, &hash));

    let mut wrong = Vec::new();
    check_none(&by_height, "height", &mut wrong);
    check_none(&by_hash, "hash", &mut wrong);
    check_bodies(&bodies, &good, "bodies", &mut wrong);
    if let SyncResponse::Bodies(v) = &bodies {
        if !v.iter().all(Block::verify_merkle_root) {
            wrong.push("bodies: a served body fails its header root".into());
        }
    }
    assert!(
        wrong.is_empty(),
        "stub anchor {hash:.16} (header root != root([]), empty body) was served: {wrong:?}"
    );
}

// INC-I-281 R-4 M3a.1 (Must) — Decision: a failure means the serve filter keys on an empty body rather than on header/body consistency, so it withholds a block whose header commits to the empty list.
#[tokio::test]
async fn consistent_empty_body_block_is_served() {
    let fx = fixture().await;
    let empty = fx.consistent_empty();
    assert!(
        empty.transactions.is_empty() && empty.verify_merkle_root(),
        "precondition: empty body, header root == root([])"
    );
    fx.store_as_anchor(&empty);
    let hash = empty.hash();

    let by_height: SyncResponse = fx.node.serve_block_by_height(ANCHOR_H);
    let by_hash: SyncResponse = fx.node.serve_block_by_hash(&hash);
    let bodies: SyncResponse = fx.node.serve_bodies(vec![fx.base[4].hash(), hash]);

    let mut wrong = Vec::new();
    check_block(&by_height, &empty, "height", &mut wrong);
    check_block(&by_hash, &empty, "hash", &mut wrong);
    check_bodies(
        &bodies,
        &[fx.base[4].clone(), empty.clone()],
        "bodies",
        &mut wrong,
    );
    assert!(
        wrong.is_empty(),
        "consistent empty-body block {hash:.16} not served unchanged: {wrong:?}"
    );
}

// INC-I-281 R-4 M3a.1 (Must) — Decision: a failure means the serve seam changed what a node answers for an ordinary full block or for an absent one, breaking every sync request outside the anchor.
#[tokio::test]
async fn full_block_is_served_unchanged() {
    let fx = fixture().await;
    let full = fx.base[4].clone();
    assert!(
        full.verify_merkle_root() && !full.transactions.is_empty(),
        "precondition: full block"
    );
    let absent = fx.anchor().hash();

    let by_height: SyncResponse = fx.node.serve_block_by_height(BASE_TOP);
    let by_hash: SyncResponse = fx.node.serve_block_by_hash(&full.hash());
    let bodies: SyncResponse = fx
        .node
        .serve_bodies(vec![fx.base[3].hash(), absent, full.hash()]);
    let none_height: SyncResponse = fx.node.serve_block_by_height(ANCHOR_H + 10);
    let none_hash: SyncResponse = fx.node.serve_block_by_hash(&absent);

    let mut wrong = Vec::new();
    check_block(&by_height, &full, "height", &mut wrong);
    check_block(&by_hash, &full, "hash", &mut wrong);
    check_bodies(
        &bodies,
        &[fx.base[3].clone(), full.clone()],
        "bodies",
        &mut wrong,
    );
    check_none(&none_height, "absent height", &mut wrong);
    check_none(&none_hash, "absent hash", &mut wrong);
    assert!(
        wrong.is_empty(),
        "full / absent block answers changed: {wrong:?}"
    );
}

// INC-I-281 R-4 M3a.1 (Must) — Decision: a failure means the requester no longer refuses a real header paired with an empty body, so the serve-side filter would be the only guard against applying a block without its transactions.
#[tokio::test]
async fn requester_rejects_header_with_stub_body() {
    let fx = fixture().await;
    let anchor = fx.anchor();
    let stub = bare(anchor.header.clone());
    let empty_root = crypto::hash::hash(b"");
    assert_eq!(
        compute_merkle_root(&[]),
        empty_root,
        "root([]) = hash(\"\")"
    );
    assert_eq!(
        stub.hash(),
        anchor.hash(),
        "body intake matches by this hash"
    );
    assert_ne!(
        anchor.header.merkle_root, empty_root,
        "precondition: real root"
    );
    assert!(!stub.verify_merkle_root(), "stub fails verify_merkle_root");
    assert_eq!(stub.compute_merkle_root(), empty_root);

    let (mut req, _tmp) = node_with_base(&fx.keys, &fx.base).await;
    let parent = fx.base.last().unwrap().hash();
    for mode in [ValidationMode::Light, ValidationMode::Full] {
        let verdict = req.validate_block_for_apply(&stub, ANCHOR_H, mode).await;
        assert!(
            matches!(
                &verdict,
                Err(ValidationError::InvalidMerkleRoot { header, computed })
                    if *header == anchor.header.merkle_root && *computed == empty_root
            ),
            "{mode:?}: requester verdict on the stub is not InvalidMerkleRoot(root([])): {verdict:?}"
        );
    }

    let applied = req.apply_block(stub.clone(), ValidationMode::Light).await;
    assert!(applied.is_err(), "requester applied the stub");
    assert_eq!(
        req.chain_state.read().await.best_hash,
        parent,
        "tip unchanged after the refusal"
    );
    assert!(
        req.block_store()
            .get_block(&stub.hash())
            .expect("store")
            .is_none(),
        "the refused stub is not stored"
    );

    req.apply_block(anchor.clone(), ValidationMode::Light)
        .await
        .expect("control: the same header with its body applies");
    assert_eq!(req.chain_state.read().await.best_hash, anchor.hash());
}

//! INC-I-289 K-1: at/above the INC-I-288 AH a branch weighs the bond of its
//! DISTINCT supporters from the common ancestor (its blocks' producers + the
//! producers whose attestations for its blocks the node holds: bits in the
//! branch's own blocks, verified gossip attestations), each producer once — NOT
//! its block count. Equal support falls to the existing tie rule. Below the AH:
//! one weight per block (unchanged). Every rig producer has bond weight 1.
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(block, peer) (HeightOccupied / Orphan
//   reroute -> gossip door / wedge door -> execute_reorg), then fn Node::run_periodic_tasks()
//   O1: chain_state best_hash / best_height, canonical entry, persisted tip, sync tip
//   O2: block_store holds (or never held) the losing branch's tip
//   O3: return values Ok(()) (handle_new_block / run_periodic_tasks, via helpers)
//   PATHS: P1 support-heavier branch arrives while on the count-heavier branch;
//     P2 count-heavier branch arrives while on the support-heavier branch;
//     P3 equal support, no attestations (tie rule); P4 bits in a block on our
//     sibling's competitor carry support across the fork; P5 below AH (child re-exec)
//   INPUT PARTITIONS: K1A on A (6 blocks, 2 of 7 producers), B (2 blocks, bits
//     5 of 7) arrives -> B; K1A2 on B, A arrives -> B kept, A6 never applied;
//     K1B below AH, on B, A arrives -> A6 (count rule); K1C one-block siblings
//     1 vs 1 (fixed keys), higher-hash and lower-hash -> today's tie outcome; K1D on S1,S2
//     (2 of 3), E1 on D with bits D by 3 of 3 (E1 hashes above S2) -> E1
//   MATRIX: K1A O1,O3 · K1A2 O1-O3 · K1B O1,O3 · K1C O1,O3 · K1D O1,O3
//   covers: doors fork_choice wedge_escape fork_recovery fork_cache fork_walk block_handling block_lifecycle

use crypto::{bls_aggregate, bls_sign, BlsKeyPair, Hash, KeyPair, PublicKey};
use doli_core::attestation::bls_attest_msg;
use doli_core::validation::ValidationMode;
use doli_core::{Block, Network, NetworkParams};
use doli_node::node::Node;
use network::PeerId;
use tempfile::TempDir;

use super::inc_i_289_common::{self as c, block, leader, F};

const N: usize = 7;

pub(super) struct R7 {
    pub(super) node: Node,
    pub(super) f_hash: Hash,
    pub(super) keys: Vec<KeyPair>,
    /// `bls[i]` is `keys[i]`'s on-chain BLS key.
    pub(super) bls: Vec<BlsKeyPair>,
    _tmp: TempDir,
}

/// `leader(keys, i)` for i < keys.len() is the i-th key in pubkey order.
pub(super) fn nth(keys: &[KeyPair], i: usize) -> &KeyPair {
    leader(keys, i as u32)
}

/// INC-I-289 ruling 2: bits count only when their aggregate verifies, so K1's
/// bits are real: BLS AH 0 and a BLS key on-chain for every producer.
async fn rig7() -> R7 {
    rig7_with(0).await
}

/// Seven genesis producers of equal bond; base h=1..=F by keys[0]; the node's
/// INC-I-178 BLS AH is `ah178`; every producer has an on-chain BLS key.
pub(super) async fn rig7_with(ah178: u64) -> R7 {
    let tmp = TempDir::new().unwrap();
    let keys: Vec<KeyPair> = (0..N).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), keys.clone())
        .await
        .expect("Node::new_for_test with 7 producers");
    let p = node.params.clone();
    let mut prev = Hash::ZERO;
    for h in 1..=F {
        let b = block(h, h as u32, prev, &keys[0], &p);
        prev = b.hash();
        node.apply_block(b, ValidationMode::Light)
            .await
            .expect("fixture: base block");
    }
    let bls = register_bls_all(&mut node, &keys).await;
    node.inc_i_178_attestation_bls_activation_height = ah178;
    let audit = NetworkParams::load(Network::Devnet).security_audit_activation_height;
    {
        let ps = node.producer_set.read().await;
        for h in 1..=(F + 8) {
            for k in &keys {
                let pi = ps.get_by_pubkey(k.public_key()).expect("fixture: member");
                assert_eq!(pi.effective_weight(h), 1, "fixture: weight 1 at h={h}");
                assert_eq!(
                    pi.selection_weight_at(h, audit),
                    1,
                    "fixture: bond 1 at h={h}"
                );
            }
        }
    }
    R7 {
        node,
        f_hash: prev,
        keys,
        bls,
        _tmp: tmp,
    }
}

/// The BLS key `bls[i]` paired with `k` (`keys[i]`).
pub(super) fn bls_of<'a>(keys: &[KeyPair], bls: &'a [BlsKeyPair], k: &KeyPair) -> &'a BlsKeyPair {
    let i = keys
        .iter()
        .position(|x| x.public_key() == k.public_key())
        .expect("fixture: key in the rig");
    &bls[i]
}

/// The aggregate of `signers` over `bls_attest_msg(parent)` (bits attest the PARENT).
pub(super) fn valid_agg(parent: &Hash, signers: &[&BlsKeyPair]) -> Vec<u8> {
    let msg = bls_attest_msg(parent);
    let sigs: Vec<_> = signers
        .iter()
        .map(|k| bls_sign(&msg, k.secret_key()).expect("BLS sign"))
        .collect();
    bls_aggregate(&sigs)
        .expect("BLS aggregate")
        .as_bytes()
        .to_vec()
}

/// INC-I-178 BLS AH 0 on `node` and an on-chain BLS key per `keys[i]` (returned aligned).
pub(super) async fn register_bls_all(node: &mut Node, keys: &[KeyPair]) -> Vec<BlsKeyPair> {
    node.inc_i_178_attestation_bls_activation_height = 0;
    let mut ps = node.producer_set.write().await;
    let bls: Vec<BlsKeyPair> = keys
        .iter()
        .map(|k| {
            let kb = BlsKeyPair::generate();
            ps.get_by_pubkey_mut(k.public_key())
                .expect("fixture: producer in the set")
                .bls_pubkey = kb.public_key().as_bytes().to_vec();
            kb
        })
        .collect();
    // On-chain from genesis: every kept epoch table (INC-I-289 L-1) carries them too.
    for start in node.state_db.epoch_table_starts() {
        let mut t = node
            .state_db
            .get_epoch_table(start)
            .expect("fixture: table");
        let mut set = t.producer_set().expect("fixture: table set");
        for (k, kb) in keys.iter().zip(&bls) {
            let p = set
                .get_by_pubkey_mut(k.public_key())
                .expect("fixture: member");
            p.bls_pubkey = kb.public_key().as_bytes().to_vec();
        }
        t.producers = bincode::serialize(&set).expect("fixture: set bytes");
        node.state_db.put_epoch_table(&t);
    }
    bls
}

/// The node's attestation universe at `height` (`[producer_list | active \ list sorted]`).
pub(super) async fn universe(node: &Node, height: u64) -> Vec<PublicKey> {
    let active: Vec<PublicKey> = (node.producer_set.read().await)
        .active_producers_at_height(height)
        .iter()
        .map(|p| p.public_key)
        .collect();
    doli_core::attestation_universe(&node.epoch_state.producer_list, &active)
}

/// `block()` whose body bits say `attesters` attested `prev`, carrying `agg`;
/// presence_root = `presence_root_at(ah178, height, bits, agg)`.
#[allow(clippy::too_many_arguments)]
pub(super) fn attested_block(
    height: u64,
    slot: u32,
    prev: Hash,
    by: &KeyPair,
    p: &doli_core::consensus::ConsensusParams,
    uni: &[PublicKey],
    attesters: &[&KeyPair],
    agg: &[u8],
    ah178: u64,
) -> Block {
    let idx: Vec<usize> = attesters
        .iter()
        .map(|k| {
            uni.iter()
                .position(|u| u == k.public_key())
                .expect("fixture: attester in the universe")
        })
        .collect();
    let bf = doli_core::encode_attestation_bitfield_vec(&idx, uni.len());
    let plain = block(height, slot, prev, by, p);
    let mut header = plain.header.clone();
    header.presence_root = doli_core::presence_root_at(ah178, height, &bf, agg);
    crate::block_sig::sign_header(&mut header, by);
    let mut b = Block::new(header, plain.transactions.clone());
    b.attestation_bitfield = bf;
    b.aggregate_bls_signature = agg.to_vec();
    b
}

/// `block()` with the presence root an empty body needs at BLS AH `ah178`
/// (at/above it the root commits the empty bitfield and aggregate).
pub(super) fn plain_at(
    ah178: u64,
    height: u64,
    slot: u32,
    prev: Hash,
    by: &KeyPair,
    p: &doli_core::consensus::ConsensusParams,
) -> Block {
    let b = block(height, slot, prev, by, p);
    if height < ah178 {
        return b;
    }
    let mut header = b.header.clone();
    header.presence_root = doli_core::presence_root_at(ah178, height, &[], &[]);
    crate::block_sig::sign_header(&mut header, by);
    Block::new(header, b.transactions)
}

/// `c::chain_on` with `plain_at(ah178, ..)` blocks.
pub(super) fn chain_on_at(
    ah178: u64,
    r: &c::Rig,
    prev: Hash,
    height: u64,
    slots: &[u32],
) -> Vec<Block> {
    let mut prev = prev;
    slots
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let by = leader(&r.keys, *s);
            let b = plain_at(ah178, height + i as u64, *s, prev, by, &r.node.params);
            prev = b.hash();
            b
        })
        .collect()
}

/// `attested_block` whose aggregate is the valid one of `attesters` over `prev`.
#[allow(clippy::too_many_arguments)]
pub(super) fn verified_block(
    r_keys: &[KeyPair],
    r_bls: &[BlsKeyPair],
    height: u64,
    slot: u32,
    prev: Hash,
    by: &KeyPair,
    p: &doli_core::consensus::ConsensusParams,
    uni: &[PublicKey],
    attesters: &[&KeyPair],
) -> Block {
    let signers: Vec<&BlsKeyPair> = attesters.iter().map(|k| bls_of(r_keys, r_bls, k)).collect();
    let agg = valid_agg(&prev, &signers);
    attested_block(height, slot, prev, by, p, uni, attesters, &agg, 0)
}

pub(super) async fn give(node: &mut Node, b: &Block) {
    node.handle_new_block(b.clone(), PeerId::random())
        .await
        .expect("handle_new_block returns Ok");
}

pub(super) async fn tip(node: &Node) -> Hash {
    node.chain_state.read().await.best_hash
}

/// Ticks until the tip is `want` or `max` ticks ran.
pub(super) async fn settle(node: &mut Node, want: &Hash, max: usize) -> usize {
    for i in 0..=max {
        if tip(node).await == *want {
            return i;
        }
        if i < max {
            node.run_periodic_tasks()
                .await
                .expect("run_periodic_tasks Ok");
        }
    }
    max
}

/// Tip hash, height, canonical entry, persisted tip and sync tip all name `b`.
pub(super) async fn assert_tip(node: &Node, b: &Block, height: u64, label: &str) {
    let (best_hash, best_height) = {
        let cs = node.chain_state.read().await;
        (cs.best_hash, cs.best_height)
    };
    assert_eq!(best_hash, b.hash(), "{label}: tip hash (h={best_height})");
    assert_eq!(best_height, height, "{label}: tip height");
    assert_eq!(
        node.block_store.get_hash_by_height(height).unwrap(),
        Some(b.hash()),
        "{label}: canonical entry at h={height}"
    );
    let persisted = node.state_db.get_chain_state().map(|c| c.best_hash);
    assert_eq!(persisted, Some(b.hash()), "{label}: persisted tip");
    let sync_h = node.sync_manager.read().await.local_tip().0;
    assert_eq!(sync_h, height, "{label}: sync manager tip");
}

/// A: 6 blocks on F by sorted[0], sorted[1] alternating (slots 7,8,14,15,21,22).
/// B: B1 (slot 23, sorted[2]) on F, B2 (slot 24, sorted[3]) on B1 whose bits say
/// sorted[2..7] (5 of 7) attested B1.
async fn two_branches(r: &R7) -> (Vec<Block>, Vec<Block>) {
    let p = r.node.params.clone();
    let mut a = Vec::new();
    let mut prev = r.f_hash;
    for (i, s) in [7u32, 8, 14, 15, 21, 22].iter().enumerate() {
        let by = leader(&r.keys, *s);
        assert!(
            by.public_key() == nth(&r.keys, 0).public_key()
                || by.public_key() == nth(&r.keys, 1).public_key(),
            "fixture: A is built by two producers"
        );
        let b = plain_at(0, F + 1 + i as u64, *s, prev, by, &p);
        prev = b.hash();
        a.push(b);
    }
    let b1 = plain_at(0, F + 1, 23, r.f_hash, leader(&r.keys, 23), &p);
    let uni = universe(&r.node, F + 2).await;
    assert_eq!(uni.len(), N, "fixture: universe is the 7 producers");
    let attesters: Vec<&KeyPair> = (2..N).map(|i| nth(&r.keys, i)).collect();
    // INC-I-289 ruling 2: B2's bits carry a VALID aggregate of the 5 attesters.
    let b2 = verified_block(
        &r.keys,
        &r.bls,
        F + 2,
        24,
        b1.hash(),
        leader(&r.keys, 24),
        &p,
        &uni,
        &attesters,
    );
    assert_eq!(
        leader(&r.keys, 23).public_key(),
        nth(&r.keys, 2).public_key()
    );
    assert_eq!(
        leader(&r.keys, 24).public_key(),
        nth(&r.keys, 3).public_key()
    );
    (a, vec![b1, b2])
}

// K1A INC-I-289 — Decision: a failure means fork choice still weighs blocks, so two producers' six-block branch keeps the node off the branch 5 of 7 producers support (the h=835,607 lock).
#[tokio::test]
async fn k1a_inc_i_289_bond_majority_branch_wins_over_block_count_majority() {
    let mut r = rig7().await;
    let (a, b) = two_branches(&r).await;
    for blk in &a {
        r.node
            .apply_block(blk.clone(), ValidationMode::Light)
            .await
            .expect("fixture: A applied");
    }
    assert_tip(&r.node, &a[5], F + 6, "K1A precondition: on A6").await;

    give(&mut r.node, &b[0]).await;
    give(&mut r.node, &b[1]).await;
    let ticks = settle(&mut r.node, &b[1].hash(), 5).await;
    eprintln!("[K1A] ticks: {ticks}");

    assert_tip(
        &r.node,
        &b[1],
        F + 2,
        "K1A: B (5 of 7) must beat A (2 of 7)",
    )
    .await;
}

// K1A2 INC-I-289 — Decision: a failure means a count-heavier branch of a bond minority still reorgs a node off the bond-majority branch it already follows.
#[tokio::test]
async fn k1a2_inc_i_289_node_on_bond_majority_ignores_block_count_majority() {
    let mut r = rig7().await;
    let (a, b) = two_branches(&r).await;
    give(&mut r.node, &b[0]).await;
    give(&mut r.node, &b[1]).await;
    assert_tip(
        &r.node,
        &b[1],
        F + 2,
        "K1A2 precondition: B2 (bits valid) applied",
    )
    .await;

    for blk in &a {
        give(&mut r.node, blk).await;
    }
    for _ in 0..3 {
        r.node
            .run_periodic_tasks()
            .await
            .expect("run_periodic_tasks Ok");
    }

    assert_tip(&r.node, &b[1], F + 2, "K1A2: B kept against A's 6 blocks").await;
    assert!(
        !r.node.block_store.has_block(&a[5].hash()).unwrap(),
        "K1A2: A6 was applied"
    );
}

// K1B INC-I-289 — Decision: a failure means the support rule leaked below its activation height (block count no longer decides there).
#[tokio::test]
async fn k1b_inc_i_289_below_ah_block_count_still_wins() {
    if !c::in_below_ah_child(
        "inc_i_289_k1_bond_weight::k1b_inc_i_289_below_ah_block_count_still_wins",
    ) {
        return;
    }
    // K1A's exact input; only the AH differs.
    let mut r = rig7().await;
    let (a, b) = two_branches(&r).await;
    for blk in &a {
        r.node
            .apply_block(blk.clone(), ValidationMode::Light)
            .await
            .expect("fixture: A applied");
    }
    assert_tip(&r.node, &a[5], F + 6, "K1B precondition: on A6").await;

    give(&mut r.node, &b[0]).await;
    give(&mut r.node, &b[1]).await;
    for _ in 0..5 {
        r.node
            .run_periodic_tasks()
            .await
            .expect("run_periodic_tasks Ok");
    }

    assert_tip(&r.node, &a[5], F + 6, "K1B below AH: A (6 blocks) kept").await;
    assert!(
        !r.node.block_store.has_block(&b[1].hash()).unwrap(),
        "K1B below AH: B2 was applied"
    );
}

/// Fixed keys for K1C. With random keys D hashes below (or above) every
/// candidate sibling about 1 run in 67, and the search below finds none.
fn k1c_keys() -> Vec<KeyPair> {
    (1..=3u8).map(|i| KeyPair::from_seed([i; 32])).collect()
}

// K1C INC-I-289 — Decision: a failure means equal support (one producer each, no attestations) no longer falls to the existing tie rule.
#[tokio::test]
async fn k1c_inc_i_289_equal_support_uses_the_tie_rule() {
    for lower in [false, true] {
        let mut r = c::rig_with_keys(k1c_keys(), &[210]).await;
        let d = r.ours[0].clone();
        let p = r.node.params.clone();
        let sib = (211..411u32)
            .map(|s| block(F + 1, s, r.f_hash, leader(&r.keys, s), &p))
            .find(|s| {
                s.header.producer != d.header.producer
                    && ((s.hash().as_bytes() < d.hash().as_bytes()) == lower)
            })
            .expect("fixture: a sibling by another producer (fixed keys: pick another seed)");
        eprintln!("[K1C] lower={lower}: sibling at slot {}", sib.header.slot);
        give(&mut r.node, &sib).await;
        for _ in 0..3 {
            r.node
                .run_periodic_tasks()
                .await
                .expect("run_periodic_tasks Ok");
        }
        assert_tip(
            &r.node,
            &d,
            F + 1,
            &format!("K1C lower={lower}: tie keeps D"),
        )
        .await;
    }
}

// K1D INC-I-289 — Decision: a failure means bits a block carries for a branch the node is not on are not counted, so a 3-of-3 attested branch loses a 2-vs-2 block tie to a 2-of-3 branch.
#[tokio::test]
async fn k1d_inc_i_289_attestations_for_a_block_off_our_chain_count() {
    let mut r = c::rig(&[210]).await;
    // INC-I-289 ruling 2: E1's bits must verify — keys on-chain, BLS AH F + 2
    // (D, applied with a pre-AH root, re-applies on the reorg; E1 is at the AH).
    let bls = register_bls_all(&mut r.node, &r.keys).await;
    r.node.inc_i_178_attestation_bls_activation_height = F + 2;
    let d = r.ours[0].clone();
    let sib = chain_on_at(F + 2, &r, r.f_hash, F + 1, &[212, 213]);
    give(&mut r.node, &sib[0]).await;
    give(&mut r.node, &sib[1]).await;
    settle(&mut r.node, &sib[1].hash(), 5).await;
    assert_tip(&r.node, &sib[1], F + 2, "K1D precondition: on S2").await;

    let p = r.node.params.clone();
    let uni = universe(&r.node, F + 2).await;
    assert_eq!(uni.len(), 3, "fixture: universe is the 3 producers");
    let all: Vec<&KeyPair> = r.keys.iter().collect();
    let e1 = (214..614u32)
        .map(|s| {
            let by = leader(&r.keys, s);
            verified_block(&r.keys, &bls, F + 2, s, d.hash(), by, &p, &uni, &all)
        })
        .find(|e| e.hash().as_bytes() > sib[1].hash().as_bytes())
        .expect("fixture: E1 hashing above S2 (a count tie keeps S2)");
    give(&mut r.node, &e1).await;
    let ticks = settle(&mut r.node, &e1.hash(), 5).await;
    eprintln!("[K1D] ticks: {ticks}");

    assert_tip(
        &r.node,
        &e1,
        F + 2,
        "K1D: D,E1 (3 of 3) must beat S1,S2 (2 of 3)",
    )
    .await;
}

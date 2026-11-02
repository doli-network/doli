//! INC-I-289 L-1 (AUDIT-P2-639-004, operator ruling 1): above the INC-I-288 gate
//! the doors weigh BOTH branches with the epoch table of their common ancestor, so
//! two nodes on opposite branches name the same winner even when one branch crossed
//! a boundary that changed a bond.
//!
// OUTPUT CONTRACT: Node::handle_completed_fork_recovery (plan door) on two nodes.
//   O1 node 1's tip after it plans toward Y; O2 node 2's tip after it plans toward X.
// PATHS: P1 the boundary on X raises a bond of a supporter of X (not X's producer);
//   P2 the boundary on X raises the bond of X's own producer (attack shape: its own
//   AddBond, not broadcast, just before the boundary).
// INPUT PARTITIONS: 5 producers of bond 1; ancestor F=5 (epoch start 4); X = 3
//   blocks by s0 crossing B=8 + s1's attestation on X8; Y = 2 blocks by s2, s3 +
//   s4's attestation on Y7; +9 bonds queued on node 1 only, applied at B=8.
//   P3 node 1's list after B differs by one producer (a key inserted at index 0)
//   and X9's bits (s1,s2,s3 attest X8) are encoded against it; P4 = P3 with the
//   list unchanged (control: the stored table credits verified bits).
//   P3/P4 partitions: BLS AH 0, the same on-chain BLS keys on both nodes before
//   block 4; Y = Y6 by s4 + s1's attestation (2).
// MATRIX: P1 x O1 O2 -> l1a · P2 x O1 O2 -> l1c · P3 x O1 O2 -> l1b · P4 x O1 O2 -> l1b_control
//! covers: bins/node/src/node/branch_support.rs epoch_tables.rs branch_rank.rs

#[path = "inc_i_156_m1_harness/mod.rs"]
mod inc_i_156_m1_harness;
use inc_i_156_m1_harness::block_sig;
#[path = "it/inc_i_289_common.rs"]
mod inc_i_289_common;

use crypto::{bls_aggregate, bls_sign, BlsKeyPair, Hash, KeyPair, PublicKey};
use doli_core::attestation::bls_attest_msg;
use doli_core::validation::ValidationMode;
use doli_core::{Attestation, Block};
use doli_node::node::Node;
use inc_i_289_common::{block, leader, F};
use network::sync::CompletedRecovery;
use network::PeerId;
use storage::PendingProducerUpdate;
use tempfile::TempDir;

type Params = doli_core::consensus::ConsensusParams;

const N: usize = 5;
/// Bonds the boundary B=8 adds on X (node 1 only).
const ADDED: usize = 9;

/// Keys in pubkey order, so `leader(keys, slot)` = `keys[slot % N]` and the node's
/// own key (`keys[0]`) is X's producer (its gossip is never counted).
fn sorted_keys() -> Vec<KeyPair> {
    let mut k: Vec<KeyPair> = (0..N).map(|_| KeyPair::generate()).collect();
    k.sort_by(|a, b| a.public_key().as_bytes().cmp(b.public_key().as_bytes()));
    k
}

fn chain(
    keys: &[KeyPair],
    p: &doli_core::consensus::ConsensusParams,
    prev: Hash,
    h: u64,
    slots: &[u32],
) -> Vec<Block> {
    let mut prev = prev;
    (slots.iter().enumerate())
        .map(|(i, s)| {
            let b = block(h + i as u64, *s, prev, leader(keys, *s), p);
            prev = b.hash();
            b
        })
        .collect()
}

struct Fork {
    keys: Vec<KeyPair>,
    base: Vec<Block>,
    x: Vec<Block>,
    y: Vec<Block>,
}

fn fork(keys: Vec<KeyPair>, p: &doli_core::consensus::ConsensusParams) -> Fork {
    let mut base = Vec::new();
    let mut prev = Hash::ZERO;
    for h in 1..=F {
        let b = block(h, h as u32, prev, &keys[0], p);
        prev = b.hash();
        base.push(b);
    }
    let x = chain(&keys, p, prev, F + 1, &[10, 15, 20]);
    let y = chain(&keys, p, prev, F + 1, &[7, 8]);
    Fork { keys, base, x, y }
}

/// A node on base + `own`; `bump` = (producer, extra bonds) queued before the block
/// at the epoch boundary, so it applies there (this branch only).
async fn node_on(f: &Fork, tmp: &TempDir, own: &[Block], bump: Option<PublicKey>) -> Node {
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), f.keys.clone())
        .await
        .expect("Node::new_for_test");
    let gate = (node.config.network.params()).inc_i_288_fork_choice_activation_height;
    assert!(gate <= F, "fixture: the node runs above the gate ({gate})");
    let bpe = node.config.network.blocks_per_reward_epoch();
    assert_eq!(
        F - F % bpe,
        4,
        "fixture: the ancestor F is in the epoch from 4"
    );
    for (i, b) in f.base.iter().chain(own).enumerate() {
        let h = i as u64 + 1;
        if let (Some(pk), true) = (bump, h.is_multiple_of(bpe) && h > F) {
            let outpoints = (0..ADDED as u8)
                .map(|i| (Hash::from_bytes([i + 1; 32]), 0))
                .collect();
            (node.producer_set.write().await).queue_update(PendingProducerUpdate::AddBond {
                pubkey: pk,
                outpoints,
                bond_unit: 1,
                creation_slot: 0,
            });
        }
        node.apply_block(b.clone(), ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("fixture: block at slot {} must apply: {e}", b.header.slot));
    }
    node
}

async fn attest(node: &mut Node, by: &KeyPair, b: &Block, height: u64) {
    let a = Attestation::new(
        b.hash(),
        b.header.slot,
        height,
        1,
        by.private_key(),
        *by.public_key(),
    );
    node.on_new_attestation(a.to_bytes(), PeerId::random())
        .await;
}

async fn tip(node: &Node) -> Hash {
    node.chain_state.read().await.best_hash
}

/// Node `node` (on its own branch) caches `other`, holds `att` on its head, and
/// walks it into the plan door.
async fn plan_toward(node: &mut Node, f_hash: Hash, other: &[Block], att: (&KeyPair, u64)) {
    let head = other.last().unwrap();
    for b in other {
        (node.fork_block_cache.write().await).insert(b.hash(), b.clone());
    }
    attest(node, att.0, head, att.1).await;
    let rec = CompletedRecovery {
        blocks: other.to_vec(),
        connection_point: f_hash,
        suppliers: Vec::new(),
    };
    let _ = node.handle_completed_fork_recovery(rec).await;
}

/// Both nodes plan toward the other's branch; returns (node 1 tip, node 2 tip).
async fn both_plan(bumped: usize) -> (Fork, Hash, Hash) {
    let (t1, t2) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let mut p = doli_core::consensus::ConsensusParams::devnet();
    p.genesis_time = 0;
    let f = fork(sorted_keys(), &p);
    let k = &f.keys;
    let mut n1 = node_on(&f, &t1, &f.x, Some(*k[bumped].public_key())).await;
    let mut n2 = node_on(&f, &t2, &f.y, None).await;
    let f_hash = f.base.last().unwrap().hash();
    let x8 = f.x.last().unwrap();
    let y7 = f.y.last().unwrap();
    attest(&mut n1, &k[1], x8, F + 3).await;
    attest(&mut n2, &k[4], y7, F + 2).await;
    plan_toward(&mut n1, f_hash, &f.y, (&k[4], F + 2)).await;
    plan_toward(&mut n2, f_hash, &f.x, (&k[1], F + 3)).await;
    let (a, b) = (tip(&n1).await, tip(&n2).await);
    (f, a, b)
}

// L1A — Decision: a failure means two honest nodes split on one fork because one
// of them weighs with a bond its branch's boundary added.
#[tokio::test]
async fn l1a_both_nodes_name_the_same_winner_across_a_bond_boundary() {
    let (f, t1, t2) = both_plan(1).await;
    assert_eq!(
        t1, t2,
        "L-1: node 1 (on X, bond raised at B) and node 2 (on Y) name different winners"
    );
    assert_eq!(
        t1,
        f.y.last().unwrap().hash(),
        "L-1: ancestor table X=2 < Y=3 -> Y"
    );
}

// L1C — Decision: a failure means a producer can win a fork on one side only by
// adding bond to itself on its own branch just before the boundary.
#[tokio::test]
async fn l1c_own_addbond_before_the_boundary_does_not_split_the_nodes() {
    let (f, t1, t2) = both_plan(0).await;
    assert_eq!(t1, t2, "L-1 attack: the two sides name different winners");
    assert_eq!(
        t1,
        f.y.last().unwrap().hash(),
        "L-1 attack: ancestor table -> Y"
    );
}

/// Same on-chain BLS keys on `node` (set before block 4, so the epoch-4 table has them).
async fn set_bls(node: &Node, keys: &[KeyPair], bls: &[BlsKeyPair]) {
    let mut ps = node.producer_set.write().await;
    for (k, b) in keys.iter().zip(bls) {
        let p = ps
            .get_by_pubkey_mut(k.public_key())
            .expect("fixture: producer");
        p.bls_pubkey = b.public_key().as_bytes().to_vec();
    }
}

/// X9 by s0 on X8: bits name `att` (attesting X8) in `uni`, with their real aggregate.
fn x9(
    keys: &[KeyPair],
    bls: &[BlsKeyPair],
    p: &doli_core::consensus::ConsensusParams,
    x8: &Block,
    uni: &[PublicKey],
) -> Block {
    let att = [1usize, 2, 3];
    let idx: Vec<usize> = (att.iter())
        .map(|i| {
            uni.iter()
                .position(|u| u == keys[*i].public_key())
                .expect("attester in universe")
        })
        .collect();
    let bf = doli_core::encode_attestation_bitfield_vec(&idx, uni.len());
    let msg = bls_attest_msg(&x8.hash());
    let sigs: Vec<_> = (att.iter())
        .map(|i| bls_sign(&msg, bls[*i].secret_key()).unwrap())
        .collect();
    let agg = bls_aggregate(&sigs).unwrap().as_bytes().to_vec();
    let plain = block(F + 4, 25, x8.hash(), &keys[0], p);
    let mut header = plain.header.clone();
    header.presence_root = doli_core::presence_root_at(0, F + 4, &bf, &agg);
    crate::block_sig::sign_header(&mut header, &keys[0]);
    let mut b = Block::new(header, plain.transactions.clone());
    b.attestation_bitfield = bf;
    b.aggregate_bls_signature = agg;
    b
}

/// `chain` with the empty-body presence root BLS AH 0 requires.
fn chain0(keys: &[KeyPair], p: &Params, prev: Hash, h: u64, slots: &[u32]) -> Vec<Block> {
    let mut prev = prev;
    (slots.iter().enumerate())
        .map(|(i, s)| {
            let (height, by) = (h + i as u64, leader(keys, *s));
            let mut header = block(height, *s, prev, by, p).header;
            header.presence_root = doli_core::presence_root_at(0, height, &[], &[]);
            crate::block_sig::sign_header(&mut header, by);
            let b = Block::new(header, block(height, *s, prev, by, p).transactions);
            prev = b.hash();
            b
        })
        .collect()
}

/// On-chain BLS keys set before block 1; base under BLS AH off, then AH 0 and `own`.
async fn bls_node(f: &Fork, bls: &[BlsKeyPair], tmp: &TempDir, own: &[Block]) -> Node {
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), f.keys.clone())
        .await
        .expect("Node::new_for_test");
    set_bls(&node, &f.keys, bls).await;
    for (i, b) in f.base.iter().chain(own).enumerate() {
        if i == f.base.len() {
            node.inc_i_178_attestation_bls_activation_height = 0;
        }
        node.apply_block(b.clone(), ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("fixture: block at slot {} must apply: {e}", b.header.slot));
    }
    node
}

/// Node 1 on X6..X9, node 2 on Y6. Node 1's list after B is the ancestor's (the
/// epoch-4 table's) or it with one producer inserted at index 0 (`changed`); X9's
/// bits are encoded against node 1's list. Each node plans toward the other.
/// Returns (node 1 tip, node 2 tip, X9).
async fn bits_plan(changed: bool) -> (Hash, Hash, Hash) {
    let mut p = Params::devnet();
    p.genesis_time = 0;
    let f = fork(sorted_keys(), &p);
    let k = &f.keys;
    let bls: Vec<BlsKeyPair> = (0..N).map(|_| BlsKeyPair::generate()).collect();
    let f_hash = f.base.last().unwrap().hash();
    let y = chain0(k, &p, f_hash, F + 1, &[9]);
    let x = chain0(k, &p, f_hash, F + 1, &[10, 15, 20]);
    let (t1, t2) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let mut n1 = bls_node(&f, &bls, &t1, &x).await;
    let table4 = n1
        .state_db
        .get_epoch_table(4)
        .expect("fixture: epoch-4 table");
    let mut list = table4.producer_list;
    if changed {
        list.insert(0, *KeyPair::generate().public_key());
    }
    n1.epoch_state.producer_list = list;
    let active: Vec<PublicKey> = (n1.producer_set.read().await)
        .active_producers_at_height(F + 4)
        .iter()
        .map(|p| p.public_key)
        .collect();
    let uni = doli_core::attestation_universe(&n1.epoch_state.producer_list, &active);
    let x9 = x9(k, &bls, &p, x.last().unwrap(), &uni);
    (n1.apply_block(x9.clone(), ValidationMode::Light).await).expect("fixture: X9 applies");
    let mut n2 = bls_node(&f, &bls, &t2, &y).await;
    attest(&mut n2, &k[1], &y[0], F + 1).await;
    plan_toward(&mut n1, f_hash, &y, (&k[1], F + 1)).await;
    let mut xs = x.clone();
    xs.push(x9.clone());
    for b in &xs {
        (n2.fork_block_cache.write().await).insert(b.hash(), b.clone());
    }
    let rec = CompletedRecovery {
        blocks: xs,
        connection_point: f_hash,
        suppliers: Vec::new(),
    };
    let _ = n2.handle_completed_fork_recovery(rec).await;
    (tip(&n1).await, tip(&n2).await, x9.hash())
}

// L1B — Decision: a failure means a block's bits weigh on one node and not on the
// other because each decodes them against its own tip list.
#[tokio::test]
async fn l1b_bits_against_a_changed_list_weigh_the_same_on_both_nodes() {
    let (t1, t2, _) = bits_plan(true).await;
    assert_eq!(
        t1, t2,
        "L-1 bits: the two nodes weigh X9's bits differently"
    );
}

// L1B-CTL — Decision: a failure means the stored table drops verified bits (keys or
// list missing), so l1b passes only because no bit is ever credited.
#[tokio::test]
async fn l1b_control_bits_against_the_unchanged_list_are_credited_on_both() {
    let (t1, t2, x9) = bits_plan(false).await;
    assert_eq!(
        (t1, t2),
        (x9, x9),
        "control: X = s0 + 3 bits (4) beats Y (2) on both"
    );
}

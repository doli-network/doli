//! INC-I-237 M8 / REQ-GRV-030 — M8 is admission-only: a block an OLD producer
//! built (it admitted forged zero-flow txs on shape) still validates and applies
//! on the new binary, and the forged txs have no state effect.
//!
//! OUTPUT CONTRACT: Node::apply_block(&mut self, Block, ValidationMode::Light) -> Result<()>
//!   O1: none   O2: chain_state.best_height, maintainer set members
//!   O3: Result (Ok required)   O4: state_db (same writes as before M8)
//!   O5: none   O6: none
//! PATHS: P1 [coinbase, forged DelegateBond, forged RevokeDelegation] on auth
//!   params (AH 0) -> Ok, no apply effect; P2 [coinbase, forged AddMaintainer,
//!   forged RemoveMaintainer, genuine AddMaintainer] on devnet -> Ok, members =
//!   before + the genuine target only
//! INPUT PARTITIONS: forged type {DelegateBond, RevokeDelegation, AddMaintainer,
//!   RemoveMaintainer}; params {testnet at height 1 (maintainer txs are not
//!   state-only there before INC-I-173), devnet (all maintainer gates 0)}.
// covers: zero_flow_auth pool lib gossip_txs validation_checks mod transaction governance context verdict relay_decision init startup gossip-relay-verdict-status gossip-relay-verdict-architecture

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Transaction};
use doli_node::node::Node;
use vdf::{VdfOutput, VdfProof};

use super::inc_i_237_m2_node_common as c;
use super::inc_i_237_m8_common::{self as m8, Parties, Sig};

const H: u64 = 1;
const SLOT: u32 = 1;

fn old_producer_block(node: &Node, producer: &KeyPair, txs: Vec<Transaction>) -> Block {
    let pool = doli_core::consensus::reward_pool_pubkey_hash();
    let mut all = vec![Transaction::new_coinbase(
        node.params.block_reward(H),
        pool,
        H,
        SLOT,
    )];
    all.extend(txs);
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root: doli_core::block::compute_merkle_root(&all),
        presence_root: Hash::ZERO,
        genesis_hash: ConsensusParams::for_network(node.config.network).genesis_hash,
        timestamp: node.params.genesis_time + (SLOT as u64 * node.params.slot_duration),
        slot: SLOT,
        producer: *producer.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, all)
}

async fn apply(node: &mut Node, kp: &KeyPair, txs: Vec<Transaction>) {
    let ids: Vec<String> = txs.iter().map(|t| format!("{:?}", t.tx_type)).collect();
    let block = old_producer_block(node, kp, txs);
    let res = node.apply_block(block, ValidationMode::Light).await;
    assert!(
        res.is_ok(),
        "REQ-GRV-030: block refused: {res:?} txs={ids:?}"
    );
    assert_eq!(node.chain_state.read().await.best_height, H, "applied");
}

async fn members(node: &Node) -> Vec<String> {
    let ms = node.maintainer_state.as_ref().expect("set attached");
    let mut v: Vec<String> = ms
        .read()
        .await
        .set
        .members
        .iter()
        .map(|k| k.to_hex())
        .collect();
    v.sort();
    v
}

// REQ-GRV-030 — Decision: a failure means the delegation signature check leaked into block validation (a consensus rule needing an activation height), or a forged delegation in an old producer's block now has an effect.
#[tokio::test]
async fn req_grv_030_old_block_with_forged_delegations_applies_without_effect() {
    let (mut node, kp, _t) = c::make_node().await;
    m8::on_testnet(&mut node);
    m8::attach(&mut node, &m8::maintainer_keys());
    let throwaway = Parties::throwaway();
    let forged = vec![
        m8::delegate_bond(&throwaway, Sig::Wrong),
        m8::revoke(&throwaway, Sig::Absent),
    ];
    for tx in &forged {
        let fx = m8::apply_effect(&node, tx, H).await;
        assert!(!fx, "forged {:?} has an apply effect", tx.tx_type);
    }
    apply(&mut node, &kp, forged).await;
    let genuine = m8::genuine_delegate_bond(&[&node]).await;
    assert!(
        m8::apply_effect(&node, &genuine, H).await,
        "non-vacuity: auth gate live at H"
    );
}

// REQ-GRV-030 — Decision: a failure means a maintainer-signature rule leaked into block validation, or a forged maintainer change in an old producer's block now changes the set.
#[tokio::test]
async fn req_grv_030_old_block_with_forged_maintainer_changes_applies_without_effect() {
    let (mut node, kp, m, _t) = m8::devnet_node().await;
    let forged_add = m8::forged_changes(&node, &m, true).await.remove(0).1;
    let forged_remove = m8::forged_changes(&node, &m, false).await.remove(1).1;
    let genuine = m8::genuine_add(&node, &m).await;
    let target = doli_core::maintainer::MaintainerChangeData::from_bytes(&genuine.extra_data)
        .expect("genuine add data")
        .target;
    let mut want = members(&node).await;
    want.push(target.to_hex());
    want.sort();
    apply(&mut node, &kp, vec![forged_add, forged_remove, genuine]).await;
    assert_eq!(
        members(&node).await,
        want,
        "only the genuine change took effect (governance ran on the block)"
    );
}

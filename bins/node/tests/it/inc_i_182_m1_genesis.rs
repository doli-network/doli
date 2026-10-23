//! INC-I-182 M1 — with the producer-auth gate at 0, a fresh devnet whose
//! producers take turns building the genesis window applies every block and
//! derives every genesis producer at genesis completion (the node builder's own
//! genesis Registration must pass the rule).
//!
// covers: producer_auth transaction types unbonding defaults env_loader network_params pool assembly tx_processing validation_checks withdrawal_holdings genesis register bonds withdrawal exit unbonding_tx core data block rpc_client
//
// OUTPUT CONTRACT: fn Node::build_block_content(..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O1 the genesis-window block carries the builder's Registration for its own key
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode::Full) -> Result<()>
//   O2 return per block (genesis window and the completion block)
// OUTPUT CONTRACT: fn Node::derive_genesis_producers_from_chain(&self) -> Vec<PublicKey>
//   O3 return; O4 producer set after completion (each derived producer present with bonds)
// PATHS: child process, producer-auth gate env 0, genesis window env 4, three producers in turn
// MATRIX
//   O1-O4 x the fresh devnet -> inc_i_182_m1_req_pauth_006_fresh_devnet_completes_genesis_at_gate_zero
// INPUT PARTITIONS:
//   builder key: producer 0 | 1 | 2 (rotated per block); height: 1..=4 (genesis) | 5 (completion)

use crypto::{BlsKeyPair, KeyPair, PublicKey};
use doli_core::transaction::TxType;
use doli_core::validation::ValidationMode;
use doli_node::node::Node;
use tempfile::TempDir;

use super::inc_i_182_m1_node_fx::{in_child, run_child, GENESIS_ENV, PAUTH_ENV};
use super::inc_i_255_m1_common::full_block_with;
use super::inc_i_255_m4_common::err_text;
use crate::inc_i_156_m1_harness::install_production_utxo_backend;

const TEST: &str =
    "inc_i_182_m1_genesis::inc_i_182_m1_req_pauth_006_fresh_devnet_completes_genesis_at_gate_zero";
const WINDOW: u64 = 4;
const PRODUCERS: u8 = 3;

fn registers(block: &doli_core::Block, pk: &PublicKey) -> bool {
    block.transactions.iter().any(|t| {
        t.tx_type == TxType::Registration
            && t.registration_data().is_some_and(|d| d.public_key == *pk)
    })
}

/// Past the window the slot schedule decides who may build (wall clock), so a
/// slot that schedules another producer is retried on a later slot.
async fn apply(node: &mut Node, pk: &PublicKey, height: u64) {
    let (mut accepted, mut last) = (false, String::new());
    for _ in 0..12 {
        let (block, _) = full_block_with(node, &[]).await;
        if height <= WINDOW {
            assert!(
                registers(&block, pk),
                "PRECONDITION: the builder's block at {height} carries its own genesis Registration"
            );
        }
        let r = node.apply_block(block, ValidationMode::Full).await;
        (accepted, last) = (r.is_ok(), err_text(&r));
        if accepted || height <= WINDOW || !last.contains("invalid producer for slot") {
            break;
        }
    }
    assert!(accepted, "node-built block at {height} refused: {last}");
}

// REQ-PAUTH-006 — Decision: a failure means a fresh devnet cannot finish genesis with the rule armed at 0 (the builder's own Registration is refused or not derived), so devnet and the gauntlet break.
#[tokio::test]
async fn inc_i_182_m1_req_pauth_006_fresh_devnet_completes_genesis_at_gate_zero() {
    if !in_child() {
        return run_child(
            TEST,
            &[
                (PAUTH_ENV, "0".to_string()),
                (GENESIS_ENV, WINDOW.to_string()),
            ],
        );
    }
    let tmp = TempDir::new().unwrap();
    let keys: Vec<KeyPair> = (0..PRODUCERS)
        .map(|i| KeyPair::from_seed([0x60 + i; 32]))
        .collect();
    let bls: Vec<BlsKeyPair> = (0..PRODUCERS)
        .map(|i| BlsKeyPair::from_seed(&[0x70 + i; 32]).expect("FIXTURE: BLS seed"))
        .collect();
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), keys.clone())
        .await
        .expect("Node::new_for_test failed");
    install_production_utxo_backend(&node).await;
    assert_eq!(
        node.config.network.genesis_blocks(),
        WINDOW,
        "PRECONDITION: {GENESIS_ENV} sets the genesis window"
    );
    node.genesis_vdf_output = Some([0x11; 32]);
    for height in 1..=WINDOW + 1 {
        let i = ((height - 1) % PRODUCERS as u64) as usize;
        node.producer_key = Some(keys[i].clone());
        node.bls_key = Some(bls[i].clone());
        apply(&mut node, keys[i].public_key(), height).await;
    }
    let want: Vec<PublicKey> = keys.iter().map(|k| *k.public_key()).collect();
    assert_eq!(
        node.derive_genesis_producers_from_chain(),
        want,
        "every genesis producer is derived, in order of first registration"
    );
    let ps = node.producer_set.read().await;
    for (i, pk) in want.iter().enumerate() {
        let bonds = ps.get_by_pubkey(pk).map(|p| p.bond_count);
        assert!(
            bonds.is_some_and(|b| b >= 1),
            "genesis producer {i} not bonded after completion: {bonds:?}"
        );
    }
}

//! INC-I-236 Run A — replay from genesis of unsigned history below the
//! activation height. `NetworkParams` is cached per process, so each body runs
//! in a re-exec'd child with the devnet AH env var set.
//!
//! OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode::Replay) -> Result<()>
//!   O1 return per block
//!   O2 chain_state best_height / best_hash
//!   O4 state root over chain state, state_db UTXO set and producer set
//! PATHS: P1 every replayed block below the AH | P2 first replayed block at or above the AH
//! INPUT PARTITIONS:
//!   P1a unsigned history (pi empty), AH far above the tip -> every block Ok; tip and state root equal the live node
//!   P2a the same unsigned history, AH = 1 -> block 1 rejected with InvalidBlockSignature; state stays at genesis
//!   P2b the same history signed by its producer, AH = 1 -> every block Ok (non-vacuity control for P2a)

use crypto::{Hash, KeyPair};
use doli_core::validation::{ValidationError, ValidationMode};
use doli_core::{Block, Network};
use doli_node::node::Node;
use tempfile::TempDir;

use crate::inc_i_156_m1_harness::{
    build_block_with_txs, coinbase_for, install_production_utxo_backend,
};

const AH_ENV: &str = "DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_236_RUN_A_REPLAY_CHILD";
const ABOVE_TIP: u64 = 1_000_000_000;
const CHAIN_LEN: u64 = 5;
const MOD: &str = "inc_i_236_run_a_replay_unsigned::";

/// Re-exec `name` in a child with the devnet AH set to `ah`; true in the child.
fn in_child(name: &str, ah: u64) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        return true;
    }
    let out = std::process::Command::new(std::env::current_exe().expect("current exe"))
        .args([
            &format!("{MOD}{name}"),
            "--exact",
            "--test-threads=1",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .env(AH_ENV, ah.to_string())
        .output()
        .expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "child {name} failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    false
}

async fn fresh_node(kp: &KeyPair) -> (Node, TempDir) {
    let temp = TempDir::new().unwrap();
    let node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test failed");
    install_production_utxo_backend(&node).await;
    (node, temp)
}

/// Linked devnet chain from genesis by `kp`, every block with an empty `pi`.
fn unsigned_chain(node: &Node, genesis: Hash, kp: &KeyPair) -> Vec<Block> {
    let mut prev = genesis;
    (1..=CHAIN_LEN)
        .map(|h| {
            let txs = vec![coinbase_for(h, h as u32, &node.params)];
            let mut b = build_block_with_txs(h as u32, prev, kp, &node.params, txs);
            b.header.vdf_proof.pi.clear();
            prev = b.hash();
            b
        })
        .collect()
}

async fn tip_and_root(node: &Node) -> (u64, Hash, Hash) {
    let cs = node.chain_state.read().await;
    let utxo = node.utxo_set.read().await;
    let ps = node.producer_set.read().await;
    let root = storage::compute_state_root(&cs, &utxo, &ps).expect("state root");
    (cs.best_height, cs.best_hash, root)
}

fn assert_devnet_ah(expected: u64) {
    assert_eq!(
        Network::Devnet
            .params()
            .inc_i_236_block_signature_activation_height,
        expected,
        "PRECONDITION: child must read the devnet AH override"
    );
}

// REQ-AUTH-013 — Decision: a failure means the operator replay/rebuild path rejects or diverges on today's unsigned history, so recovery breaks before the height.
#[tokio::test]
async fn req_auth_013_replay_unsigned_history_below_ah_matches_live_state() {
    if !in_child(
        "req_auth_013_replay_unsigned_history_below_ah_matches_live_state",
        ABOVE_TIP,
    ) {
        return;
    }
    assert_devnet_ah(ABOVE_TIP);
    let kp = KeyPair::generate();

    let (mut live, _t1) = fresh_node(&kp).await;
    let genesis = live.chain_state.read().await.best_hash;
    for b in unsigned_chain(&live, genesis, &kp) {
        live.apply_block(b, ValidationMode::Light)
            .await
            .expect("live node must apply unsigned history below the AH");
    }
    let expected = tip_and_root(&live).await;
    assert_eq!(expected.0, CHAIN_LEN, "NON-VACUITY: live chain applied");

    let (mut replay, _t2) = fresh_node(&kp).await;
    assert_eq!(
        replay.chain_state.read().await.best_hash,
        genesis,
        "PRECONDITION: same genesis"
    );
    for h in 1..=CHAIN_LEN {
        let stored = live
            .block_store
            .get_block_by_height(h)
            .expect("block_store read")
            .expect("stored block");
        assert!(
            stored.header.vdf_proof.pi.is_empty(),
            "stored history is unsigned at h={h}"
        );
        replay
            .apply_block(stored, ValidationMode::Replay)
            .await
            .unwrap_or_else(|e| {
                panic!("replay of unsigned block h={h} below the AH must pass: {e}")
            });
    }
    let actual = tip_and_root(&replay).await;
    assert_eq!(actual.0, expected.0, "replay height");
    assert_eq!(actual.1, expected.1, "replay tip hash");
    assert_eq!(actual.2, expected.2, "replay state root");
}

// REQ-AUTH-013 — Decision: a failure means the replay path never reads the signature, so the P1a pass above proves nothing about the gate.
#[tokio::test]
async fn req_auth_013_contrast_replay_unsigned_history_at_ah_rejected() {
    if !in_child(
        "req_auth_013_contrast_replay_unsigned_history_at_ah_rejected",
        1,
    ) {
        return;
    }
    assert_devnet_ah(1);
    let kp = KeyPair::generate();

    let (mut replay, _t) = fresh_node(&kp).await;
    let genesis = replay.chain_state.read().await.best_hash;
    let chain = unsigned_chain(&replay, genesis, &kp);
    let err = replay
        .apply_block(chain[0].clone(), ValidationMode::Replay)
        .await
        .expect_err("unsigned block 1 at h >= AH must be rejected by replay");
    assert!(
        matches!(
            err.downcast_ref::<ValidationError>(),
            Some(ValidationError::InvalidBlockSignature { .. })
        ),
        "rejection must come from the signature rule, got {err:#}"
    );
    let st = replay.chain_state.read().await;
    assert_eq!(
        (st.best_height, st.best_hash),
        (0, genesis),
        "state must stay at genesis"
    );
    drop(st);
    assert!(
        replay
            .block_store
            .get_block_by_height(1)
            .expect("block_store read")
            .is_none(),
        "rejected block must not be stored"
    );

    let (mut control, _t2) = fresh_node(&kp).await;
    for mut b in chain {
        crate::block_sig::sign_block(&mut b, &kp);
        control
            .apply_block(b, ValidationMode::Replay)
            .await
            .expect("NON-VACUITY: the same history signed by its producer must replay");
    }
    assert_eq!(control.chain_state.read().await.best_height, CHAIN_LEN);
}

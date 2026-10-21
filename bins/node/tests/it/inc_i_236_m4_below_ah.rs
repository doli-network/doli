//! INC-I-236 M4 below-AH lock: with the signature AH above the tip, header
//! side effects behave exactly as before the fix. `NetworkParams` is cached per
//! process, so the body runs in a re-exec'd child with the AH env var set.
//!
//! OUTPUT CONTRACT: fn Node::handle_new_block(&mut self, block, peer) -> Result<()>
//!   O1 equivocation_detector proof naming the victim
//!   O2 mempool SlashProducer tx naming the victim
//!   O3 sync_manager stuck_fork_signal
//! PATHS: P1 check_block; P2 HeightOccupied{is_better:true} arm — both below the AH.
//! INPUT PARTITIONS:
//!   P1a forged unsigned + real signed, same (producer, slot) -> O1 = 1, O2 = 0 (log-only)
//!   P2a forged unsigned lower-slot sibling at an occupied height -> O3 raised

use doli_core::{Network, NetworkParams};

use super::inc_i_236_m4_common::{
    canonical_two, feed, header_block, proofs_against, reporter_node, slash_txs_against,
    stuck_fork_raised,
};

const AH_ENV: &str = "DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_236_M4_BELOW_AH_CHILD";
const BELOW_AH: u64 = 1_000_000_000;
const SELF_NAME: &str = "inc_i_236_m4_below_ah::req_auth_007_012_below_ah_behavior_unchanged";

// REQ-AUTH-007 — Decision: a failure means the fix changed pre-activation behavior (a consensus-history break).
// REQ-AUTH-012 — Decision: a failure means pre-activation stuck-fork signalling changed without an AH gate.
#[tokio::test]
async fn req_auth_007_012_below_ah_behavior_unchanged() {
    if std::env::var_os(CHILD_ENV).is_none() {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([SELF_NAME, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD_ENV, "1")
            .env(AH_ENV, BELOW_AH.to_string())
            .output()
            .expect("spawn child test process");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "below-AH child failed or did not run:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }

    assert_eq!(
        NetworkParams::load(Network::Devnet).inc_i_236_block_signature_activation_height,
        BELOW_AH,
        "child must pick up the AH env override"
    );

    // P1a
    let (mut node, _reporter, victim, _tmp) = reporter_node().await;
    let genesis = node.chain_state.read().await.best_hash;
    let pk = *victim.public_key();
    let real = header_block(&mut node, 1, 7, genesis, &pk, Some(&victim), 0).await;
    let forged = header_block(&mut node, 1, 7, genesis, &pk, None, 1).await;
    feed(&mut node, real).await;
    feed(&mut node, forged).await;
    assert_eq!(
        proofs_against(&node, &pk).await,
        1,
        "below AH: proof as before"
    );
    assert_eq!(
        slash_txs_against(&node, &pk).await,
        0,
        "below AH: no slash tx, as before (handle_equivocation is log-only)"
    );

    // P2a
    let (mut node, _reporter, victim, _tmp) = reporter_node().await;
    let (b1, _b2) = canonical_two(&mut node, &victim, 10).await;
    stuck_fork_raised(&node).await;
    let forged = header_block(&mut node, 2, 5, b1.hash(), victim.public_key(), None, 1).await;
    feed(&mut node, forged).await;
    assert!(
        stuck_fork_raised(&node).await,
        "below AH: stuck fork as before"
    );
}

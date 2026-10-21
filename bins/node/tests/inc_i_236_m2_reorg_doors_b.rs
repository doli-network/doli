// OUTPUT CONTRACT: execute_reorg / handle_completed_fork_recovery BELOW the signature AH
// OUTPUTS: O1 return Ok; O2 chain_state.best_hash / best_height move to the fork tip
// PATHS: P7 execute_reorg direct; P8 fork walk (eligibility gate, then execute_reorg)
// INPUT PARTITIONS: unsigned fork (empty pi) at h=6..8 with AH=1000 (pre-activation)
// MATRIX: O1-O2 x {P7, P8} x {unsigned below AH}; at/above-AH cells in inc_i_236_m2_reorg_doors.rs.
// covers: bins/node/src/node/validation_checks/mod.rs
// covers: bins/node/src/node/block_handling.rs
// covers: bins/node/src/node/fork_recovery.rs
// covers: bins/node/src/node/fork_walk.rs

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;

use inc_i_236_m2_reorg_support::*;
use std::sync::Once;

const AH: u64 = 1_000;
static INIT_ENV: Once = Once::new();

fn init_env() {
    INIT_ENV.call_once(|| {
        std::env::set_var(
            "DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT",
            AH.to_string(),
        );
    });
}

// REQ-AUTH-020 — Decision: a failure means the new reorg-door gates fire below AH and fork pre-activation history.
#[tokio::test]
async fn req_auth_020_below_ah_unsigned_fork_reorgs_unchanged() {
    init_env();
    let unsigned = [Sig::Unsigned; 3];

    let mut fx = fixture().await;
    assert_eq!(
        fx.node
            .config
            .network
            .params()
            .inc_i_236_block_signature_activation_height,
        AH,
        "precondition: AH override took effect"
    );
    let fork = branch(&fx, unsigned);
    run_execute_reorg(&mut fx, &fork)
        .await
        .expect("P7 below AH: execute_reorg");
    let cs = fx.node.chain_state.read().await.clone();
    assert_eq!(
        (cs.best_hash, cs.best_height),
        (fork[2].hash(), 8),
        "P7 below AH"
    );

    let mut fx = fixture().await;
    let fork = branch(&fx, unsigned);
    run_walk(&mut fx, &fork).await.expect("P8 below AH: walk");
    let cs = fx.node.chain_state.read().await.clone();
    assert_eq!(
        (cs.best_hash, cs.best_height),
        (fork[2].hash(), 8),
        "P8 below AH"
    );
}

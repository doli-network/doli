//! INC-I-255 M1 — the slash gate is a forward-only activation: below it every
//! path keeps today's verdict, and admission judges the gate at the height the
//! tx would be mined at. `NetworkParams` is cached per process, so each body
//! runs in a re-exec'd child with the devnet slash gate env-set.
//!
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64, u32) -> Result<AddTransactionResult, MempoolError>
//   O1 return
// OUTPUT CONTRACT: fn Node::handle_new_transaction(&self, Transaction) -> bool
//   O1 return
// OUTPUT CONTRACT: fn Node::build_block_content(&mut self, ..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O1 transactions the builder selected
// OUTPUT CONTRACT: fn Node::validate_block_for_apply(&self, &Block, u64, ValidationMode) -> Result<(), ValidationError>
//   O1 return per mode
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return per mode; O2 chain tip
// OUTPUT CONTRACT: fn Node::handle_new_block(&mut self, Block, PeerId) -> Result<()> (reaches handle_equivocation)
//   O1 mempool slash entries naming the accused
// PATHS: P-OFF gate far above the tip | P-EDGE gate G with best = G-2 then G-1 | control at the devnet default
// MATRIX
//   P-OFF: add_transaction x {canonical, reporter-signed, unknown producer} -> FeeTooLow(0, 1)
//          builder x resident canonical -> not selected
//          validate_block_for_apply x {Full, Light, Replay} -> Ok
//          apply_block x {Full, Light} -> insufficient fee, tip unchanged; x Replay -> Ok, tip advances
//          handle_new_block x signed siblings -> no slash tx
//          control (parent, default gate) -> canonical admitted
//   P-EDGE: best + 1 < G -> FeeTooLow(0, 1); best + 1 = G -> handle_new_transaction true
// INPUT PARTITIONS:
//   gate: far above the tip | G = 3 | devnet default
//   evidence: canonical | reporter-signed | canonical against an unregistered key
//   mode: Full | Light | Replay

use crypto::KeyPair;
use doli_core::validation::ValidationMode;
use mempool::MempoolError;

use super::inc_i_255_m1_common::*;
use crate::inc_i_156_m1_harness::apply_plain_up_to;

const BELOW: u64 = 1_000_000_000;
const EDGE_GATE: u64 = 3;
const MOD: &str = "inc_i_255_m1_below_gate::";

async fn below_gate_body() {
    let mut w = world().await;
    let genesis_tip = tip(&w.node).await;
    let tx = valid_slash(&w);
    let (a, b) = pair(&w.accused, chain_genesis());
    let reporter_signed = proof_of(&w, a, b).to_slash_transaction(&w.reporter);
    let stranger = KeyPair::generate();
    let (sa, sb) = pair(&stranger, chain_genesis());
    let unknown = canonical(*stranger.public_key(), sa, sb);
    for (what, t) in [
        ("canonical", &tx),
        ("reporter-signed", &reporter_signed),
        ("unknown", &unknown),
    ] {
        let r = admit(&w.node, t).await;
        assert!(
            matches!(r, Err(MempoolError::FeeTooLow(0, 1))),
            "below the gate admission of the {what} slash keeps FeeTooLow(0, 1); got {r:?}"
        );
    }

    assert!(
        inject(&w.node, &tx).await,
        "PRECONDITION: the fee-free lane holds the slash as today"
    );
    let (block, selected) = full_block_with(&mut w.node, std::slice::from_ref(&tx)).await;
    assert!(
        !selected.contains(&tx.hash()),
        "below the gate the builder must keep skipping the slash"
    );
    for mode in MODES {
        let r = w.node.validate_block_for_apply(&block, 1, mode).await;
        assert!(
            r.is_ok(),
            "below the gate block validation {mode:?} stays Ok; got {r:?}"
        );
    }
    for mode in [ValidationMode::Full, ValidationMode::Light] {
        let r = w.node.apply_block(block.clone(), mode).await;
        let e = r
            .as_ref()
            .err()
            .map(|e| format!("{e:#}"))
            .unwrap_or_default();
        assert!(
            e.contains("insufficient fee: got 0, minimum 1"),
            "below the gate {mode:?} apply keeps the fee-floor error; got {e:?}"
        );
        assert_eq!(
            tip(&w.node).await,
            genesis_tip,
            "{mode:?}: tip must not move"
        );
    }
    w.node
        .apply_block(block.clone(), ValidationMode::Replay)
        .await
        .expect("below the gate Replay keeps tolerating the fee failure");
    assert_eq!(tip(&w.node).await, (1, block.hash()), "Replay: tip");

    let (resident, _) = handler_slashes(&mut world().await).await;
    assert!(
        resident.is_empty(),
        "below the gate the handler must stay log-only; mempool holds {resident:?}"
    );
}

// REQ-SLASH-002 — Decision: a failure means the fix changed a pre-activation verdict on some path, a consensus-history break that forks old blocks on replay.
#[tokio::test]
async fn inc_i_255_below_gate_every_path_keeps_head_verdict() {
    if in_child() {
        below_gate_body().await;
        return;
    }
    run_in_child(
        &format!("{MOD}inc_i_255_below_gate_every_path_keeps_head_verdict"),
        BELOW,
    );

    let w = world().await;
    let tx = valid_slash(&w);
    let r = admit(&w.node, &tx).await;
    assert!(
        r.is_ok(),
        "NON-VACUITY: at the devnet default gate the same slash must be admitted, else the child proves nothing; got {r:?}"
    );
}

// REQ-SLASH-001 — Decision: a failure means admission judges the gate at the tip instead of the mining height, so a slash detected at gate-1 is lost (N4).
#[tokio::test]
async fn inc_i_255_n4_admission_evaluates_gate_at_next_height() {
    if !in_child() {
        run_in_child(
            &format!("{MOD}inc_i_255_n4_admission_evaluates_gate_at_next_height"),
            EDGE_GATE,
        );
        return;
    }
    let mut w = world().await;
    let keys = [w.reporter.clone(), w.accused.clone()];
    let params = w.node.params.clone();
    apply_plain_up_to(&mut w.node, &keys, EDGE_GATE - 2, &params).await;
    let tx = valid_slash(&w);
    let r = admit(&w.node, &tx).await;
    assert!(
        matches!(r, Err(MempoolError::FeeTooLow(0, 1))),
        "best + 1 below the gate keeps the fee floor; got {r:?}"
    );

    apply_plain_up_to(&mut w.node, &keys, EDGE_GATE - 1, &params).await;
    assert_eq!(
        tip(&w.node).await.0,
        EDGE_GATE - 1,
        "PRECONDITION: best = gate - 1"
    );
    let admitted = w.node.handle_new_transaction(tx.clone()).await;
    let why = if admitted {
        None
    } else {
        Some(admit(&w.node, &tx).await)
    };
    assert!(
        admitted,
        "N4: at best = gate - 1 the slash is mined at the gate, so admission must accept; verdict {why:?}"
    );
}

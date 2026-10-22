//! INC-I-255 M6 — with Slash(p) and a Transfer T(p) spending p's expired Bond
//! UTXO both resident, the builder packs the slash and not T(p), and the
//! producer's own apply accepts the block.
//!
// covers: slash_state, utxo, block, assembly, tx_processing, slash_burn, slash_admission, pool, slash_withdrawal, withdrawal_economics, slash_bond_spend, mod.rs, lib.rs, validation_checks, protocol, security_model
//
// OUTPUT CONTRACT: fn Node::build_block_content(&mut self, ..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O1 transactions the builder selected
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return on the producer's own block; O2 slash recorded
// PATHS: child process, slash gate env-set to 3; tip advanced to 2, so the built block (height 3) is at the gate.
//        T(p) is admitted as judged below the gate (height 1, lock_until 1), the slash at the gate.
// MATRIX
//   build O1 x control (T(p) alone) -> T(p) selected; apply O1 -> Ok (non-vacuity)
//   build O1 x tie (fee 1, rate 0 = slash rate) -> slash only; apply O1 Ok, O2 recorded
//   build O1 x T(p) outbids (rate > 0) -> slash only; apply O1 Ok, O2 recorded
// INPUT PARTITIONS:
//   fee rate: tie with the fee-0 slash | T(p) strictly higher
//   resident set: T(p) alone | Slash(p) + T(p)

use doli_core::transaction::Transaction;
use doli_core::validation::ValidationMode;

use super::inc_i_255_m1_common::{full_block_with, in_child, run_in_child, slash_recorded, tip};
use super::inc_i_255_m4_common::{admit_at, err_text, resident};
use super::inc_i_255_m6_common::*;

const GATE: u64 = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Case {
    Control,
    Tie,
    Outbids,
}

#[derive(Debug)]
struct Seen {
    case: Case,
    slash_resident: bool,
    t_resident: bool,
    slash_selected: bool,
    t_selected: bool,
    applied: String,
    recorded: bool,
}

async fn run(case: Case) -> Seen {
    let fee = if case == Case::Outbids {
        FEE_OUTBID
    } else {
        FEE_TIE
    };
    let mut r = m6(fee).await;
    for _ in 0..2 {
        let (b, _) = full_block_with(&mut r.w.node, &[]).await;
        let adv = r.w.node.apply_block(b, ValidationMode::Full).await;
        assert!(adv.is_ok(), "FIXTURE: advance; got {}", err_text(&adv));
    }
    let h = tip(&r.w.node).await.0;
    assert_eq!(h + 1, GATE, "FIXTURE: the next block is at the gate");
    let (slash, t) = (r.slash.clone(), r.t.clone());
    let n = &r.w.node;
    let order: Vec<(&Transaction, u64)> = match case {
        Case::Control => vec![(&t, h - 1)],
        Case::Tie | Case::Outbids => vec![(&slash, h), (&t, h - 1)],
    };
    for (tx, at) in order {
        let res = admit_at(n, tx, at).await;
        assert!(res.is_ok(), "{case:?}: FIXTURE: admission; got {res:?}");
    }
    let (slash_resident, t_resident) = (resident(n, &slash).await, resident(n, &t).await);
    let (block, selected) = full_block_with(&mut r.w.node, &[]).await;
    let applied = r.w.node.apply_block(block, ValidationMode::Full).await;
    Seen {
        case,
        slash_resident,
        t_resident,
        slash_selected: selected.contains(&slash.hash()),
        t_selected: selected.contains(&t.hash()),
        applied: err_text(&applied),
        recorded: slash_recorded(&r.w.node, &r.w.accused_pk()).await,
    }
}

// REQ-SLASH-002 — Decision: a failure means the builder packs Slash(p) with T(p), so its own apply rejects the block (lost slot, slash purged).
#[tokio::test]
async fn inc_i_255_m6_probe_4_builder_packs_slash_only() {
    if !in_child() {
        run_in_child(
            "inc_i_255_m6_builder::inc_i_255_m6_probe_4_builder_packs_slash_only",
            GATE,
        );
        return;
    }
    let mut seen = Vec::new();
    for case in [Case::Control, Case::Tie, Case::Outbids] {
        seen.push(run(case).await);
    }
    eprintln!("OBSERVED builder: {seen:#?}");
    let c = &seen[0];
    assert!(
        c.t_resident && c.t_selected && c.applied == "Ok",
        "NON-VACUITY: T(p) alone is selected and applies; got {c:?}"
    );
    for s in &seen[1..] {
        assert!(
            s.slash_resident && s.t_resident,
            "{:?}: PRECONDITION: Slash(p) and T(p) both resident; {s:?}",
            s.case
        );
    }
    for s in &seen[1..] {
        assert!(
            s.slash_selected && !s.t_selected,
            "{:?}: the builder must pack Slash(p) and not T(p); got {s:?}",
            s.case
        );
        assert!(
            s.applied == "Ok" && s.recorded,
            "{:?}: the producer's own apply must accept its block and record the slash; got {s:?}",
            s.case
        );
    }
}

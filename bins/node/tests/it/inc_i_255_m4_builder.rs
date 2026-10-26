//! INC-I-255 M4 — with a slash for p and p's bond withdrawal both resident,
//! the builder packs the slash and not the withdrawal, whatever the mempool
//! iteration order, and the producer's own apply accepts the block.
//!
// covers: slash_state, block, assembly, slash_admission, pool, slash_withdrawal, slash_evidence, entry, mod, lib, protocol, security_model
//
// OUTPUT CONTRACT: fn Node::build_block_content(&mut self, ..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O1 transactions the builder selected
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return on the producer's own block; O2 slash recorded
// PATHS: child process, slash gate env-set to 2; tip 1, so the built block (height 2) is at the gate.
//        RW(p) is admitted as judged below the gate (the only way it can sit beside Slash(p) once admission refuses the pair).
// MATRIX
//   build O1 x control (RW(p) alone) -> RW(p) selected; apply O1 -> Ok (non-vacuity)
//   build O1 x tie (fee 1, rate 0 = slash rate) x slash admitted first -> slash only; apply O1 Ok, O2 recorded
//   build O1 x tie x RW(p) admitted first -> slash only; apply O1 Ok, O2 recorded
//   build O1 x RW(p) outbids (rate > 0, iterated before the slash) -> slash only; apply O1 Ok, O2 recorded
// INPUT PARTITIONS:
//   fee rate: tie with the fee-0 slash | withdrawal strictly higher
//   insertion order: slash first | withdrawal first
//   resident set: RW(p) alone | Slash(p) + RW(p)

use doli_core::transaction::Transaction;
use doli_core::validation::ValidationMode;

use super::inc_i_255_m1_common::{full_block_with, in_child, run_in_child, slash_recorded, tip};
use super::inc_i_255_m4_common::*;
use crate::inc_i_156_m1_harness::apply_plain_up_to;

const GATE: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Case {
    Control,
    TieSlashFirst,
    TieWdFirst,
    WdOutbids,
}

#[derive(Debug)]
struct Seen {
    case: Case,
    slash_resident: bool,
    wd_resident: bool,
    slash_selected: bool,
    wd_selected: bool,
    applied: String,
    recorded: bool,
}

async fn run(case: Case) -> Seen {
    let fee = if case == Case::WdOutbids {
        FEE_OUTBID
    } else {
        FEE_TIE
    };
    let mut r = m4(fee).await;
    // Dense slots up to the gate: the next block has one block of tenure and
    // the evidence (slot SLOT) is not older than the parent slot.
    let keys = [r.w.reporter.clone(), r.w.accused.clone()];
    let params = r.w.node.params.clone();
    apply_plain_up_to(&mut r.w.node, &keys, GATE, &params).await;
    let h = tip(&r.w.node).await.0;
    assert_eq!(h, GATE, "FIXTURE: the next block is one above the gate");
    let (slash, wd) = (r.slash.clone(), r.wd.clone());
    let n = &r.w.node;
    let order: Vec<(&Transaction, u64)> = match case {
        Case::Control => vec![(&wd, h - 2)],
        Case::TieSlashFirst | Case::WdOutbids => vec![(&slash, h), (&wd, h - 2)],
        Case::TieWdFirst => vec![(&wd, h - 2), (&slash, h)],
    };
    for (tx, at) in order {
        let res = admit_at(n, tx, at).await;
        assert!(res.is_ok(), "{case:?}: FIXTURE: admission; got {res:?}");
    }
    let (slash_resident, wd_resident) = (resident(n, &slash).await, resident(n, &wd).await);
    let (block, selected) = full_block_with(&mut r.w.node, &[]).await;
    let applied = r.w.node.apply_block(block, ValidationMode::Full).await;
    Seen {
        case,
        slash_resident,
        wd_resident,
        slash_selected: selected.contains(&slash.hash()),
        wd_selected: selected.contains(&wd.hash()),
        applied: err_text(&applied),
        recorded: slash_recorded(&r.w.node, &r.w.accused_pk()).await,
    }
}

// REQ-SLASH-002 — Decision: a failure means the builder packs Slash(p) with RW(p), so the producer's own apply rejects its block (lost slot, slash purged) or p withdraws before the burn.
#[tokio::test]
async fn inc_i_255_m4_probe_4_builder_packs_slash_only() {
    if !in_child() {
        run_in_child(
            "inc_i_255_m4_builder::inc_i_255_m4_probe_4_builder_packs_slash_only",
            GATE,
        );
        return;
    }
    let mut seen = Vec::new();
    for case in [
        Case::Control,
        Case::TieSlashFirst,
        Case::TieWdFirst,
        Case::WdOutbids,
    ] {
        seen.push(run(case).await);
    }
    eprintln!("OBSERVED builder: {seen:#?}");
    let c = &seen[0];
    assert!(
        c.wd_resident && c.wd_selected && c.applied == "Ok",
        "NON-VACUITY: RW(p) alone is selected and applies; got {c:?}"
    );
    for s in &seen[1..] {
        assert!(
            s.slash_resident,
            "{:?}: PRECONDITION: slash resident; {s:?}",
            s.case
        );
        if s.case != Case::TieWdFirst {
            assert!(
                s.wd_resident,
                "{:?}: PRECONDITION: RW(p) admitted below the gate sits beside the slash; {s:?}",
                s.case
            );
        }
    }
    for s in &seen[1..] {
        assert!(
            s.slash_selected && !s.wd_selected,
            "{:?}: the builder must pack Slash(p) and not RW(p); got {s:?}",
            s.case
        );
        assert!(
            s.applied == "Ok" && s.recorded,
            "{:?}: the producer's own apply must accept its block and record the slash; got {s:?}",
            s.case
        );
    }
}

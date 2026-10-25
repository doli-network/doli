//! INC-I-262 M1 — on a devnet node at the producer-auth gate, the desktop
//! wallet's Registration, AddBond and RequestWithdrawal as today's code builds
//! them are refused on every path, each for its recorded reason.
//!
// covers: producer builder
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(..) -> Result<..>   O1 return at tip 2
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode::Light) -> Result<()>   O2 return at 2 (tx carried or dropped)
// PATHS: child process, producer-auth gate env 0, genesis window env 0
// MATRIX
//   O1, O2 x {Registration, AddBond, RequestWithdrawal} x current form -> refused: [ERRTX-PAUTH001] | [ERRTX007] | [ERRTX003]
//     (inc_i_262_m1_req_def_001_current_forms_refused_at_gate)
// INPUT PARTITIONS:
//   form: GUI Registration without the auth tail | wallet AddBond without AddBondData | wallet RequestWithdrawal 4-byte payload, dummy input, zero output

use super::inc_i_182_m1_node_fx::{env_at, in_child, run_child};
use super::inc_i_262_m1_node_fx::{on_apply, on_mempool, CURRENT};

const AT: u64 = 2;
const MOD: &str = "inc_i_262_m1_current_forms::";
const WHY: [(&str, &str); 3] = [
    ("Registration", "[ERRTX-PAUTH001]"),
    ("AddBond", "[ERRTX007] bond output 0 has 0 bytes extra_data"),
    ("RequestWithdrawal", "[ERRTX003] output 0 has zero amount"),
];

// REQ-DEF-001 — Decision: a failure means a desktop-wallet producer tx that today's code builds is accepted at the gate, or refused for another reason than the one the fix removes.
#[tokio::test]
async fn inc_i_262_m1_req_def_001_current_forms_refused_at_gate() {
    if !in_child() {
        return run_child(
            &format!("{MOD}inc_i_262_m1_req_def_001_current_forms_refused_at_gate"),
            &env_at(0),
        );
    }
    let mut admitted = 0;
    let mut bad = Vec::new();
    for ((kind, build), (_, why)) in CURRENT.iter().zip(WHY.iter()) {
        let (pool, _) = on_mempool(*build, AT).await;
        let (apply, _) = on_apply(*build, AT).await;
        println!("GOLDEN_OBSERVED {kind}: {} | {}", pool.text, apply.text);
        admitted += usize::from(pool.accepted);
        if pool.accepted || !pool.text.contains(why) {
            bad.push(format!("{kind}: want refused with {why:?}: {}", pool.text));
        }
        if apply.accepted || !apply.text.contains(why) {
            bad.push(format!(
                "{kind}: want the block refused with {why:?}: {}",
                apply.text
            ));
        }
    }
    println!("GOLDEN_OBSERVED INC_I_262_CURRENT_ADMITTED={admitted}");
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    assert_eq!(admitted, 0, "no current form is admitted at the gate");
}

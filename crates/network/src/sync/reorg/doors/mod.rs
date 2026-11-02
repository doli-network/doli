//! INC-I-204 M5 — the two reorg doors: the gossip door and the planning door.
//!
//! Split out of `reorg/mod.rs` for the module-size budget. Both doors reach their
//! weight verdict through `fork_choice::resolve_weight` and their finality verdict
//! through `ReorgHandler::fork_choice_admits`, gated on
//! `inc_i_204_fork_choice_activation_height`. Below the gate each keeps its own
//! pre-M5 code path verbatim.

mod gossip;
mod plan;

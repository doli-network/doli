//! Pins `Input`'s home after the M3 module split.
//!
//! covers: REQ-ROT-001
//!
//! `types.rs` sits at the 500-line module budget, so M3 moves `Input` into
//! `transaction::input`. This file binds the NEW module path and the existing
//! re-exports to the SAME type.

use crypto::Hash;

use super::input::Input as InputFromModule;

// REQ-ROT-001 — Decision: whether Input actually moved into its own module or was only re-exported, leaving types.rs still at the budget ceiling.
#[test]
fn input_lives_in_the_transaction_input_module() {
    let via_module = InputFromModule::new(Hash::from_bytes([0x11u8; 32]), 4);
    assert_eq!(via_module.output_index, 4);
    assert_eq!(via_module.prev_tx_hash, Hash::from_bytes([0x11u8; 32]));
}

// REQ-ROT-001 — Decision: whether the move broke `doli_core::Input` / `transaction::Input` for the wallet, CLI, node and every other caller outside the module.
#[test]
fn input_is_still_reachable_at_both_legacy_paths() {
    let via_module = InputFromModule::new(Hash::from_bytes([0x22u8; 32]), 9);

    // Each binding compiles only if it is the SAME type, not a look-alike.
    let via_transaction: super::Input = via_module.clone();
    let via_crate_root: crate::Input = via_transaction.clone();

    assert_eq!(via_crate_root.output_index, 9);
    assert_eq!(via_module.prev_tx_hash, via_crate_root.prev_tx_hash);
}

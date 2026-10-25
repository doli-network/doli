//! INC-I-262 M2 / REQ-DEF-003 (D3-pin) — on every network's defaults the
//! effective slashing gate is at or above the unbonding gate.
//!
// covers: slash_window slash_state
//
// OUTPUT CONTRACT: fn NetworkParams::effective_slashing_activation_height(&self) -> u64
//   O1 return, compared with NetworkParams::defaults(network).inc_i_257_unbonding_activation_height
//   O2 receiver: none (&self); no store writes; defaults read no env
// PATHS: P-MAIN mainnet | P-TEST testnet | P-DEV devnet
// INPUT PARTITIONS: 806_859 >= 806_859 (mainnet), 330_849 >= 330_849 (testnet), 0 >= 0 (devnet); equality is the edge
// MATRIX: O1 x {P-MAIN, P-TEST, P-DEV} -> inc_i_262_m2_pin_order_slashing_gate_not_below_unbonding_gate

use doli_core::{Network, NetworkParams};

// REQ-DEF-003 — Decision: a failure means a network can arm slashing before the unbonding lock, where a RequestWithdrawal pays out instantly, so no locked collateral exists and the floor waiver cannot stop a drained floor from passing the offence.
#[test]
fn inc_i_262_m2_pin_order_slashing_gate_not_below_unbonding_gate() {
    for network in [Network::Mainnet, Network::Testnet, Network::Devnet] {
        let p = NetworkParams::defaults(network);
        assert!(
            p.effective_slashing_activation_height() >= p.inc_i_257_unbonding_activation_height,
            "{network:?}: effective slashing gate {} below the unbonding gate {}",
            p.effective_slashing_activation_height(),
            p.inc_i_257_unbonding_activation_height
        );
    }
}

//! Testnet pin of the four defense gates at ONE height (user decision 2026-10-25).
//!
//! The pin covers INC-I-236 (signed blocks), INC-I-255 (slashing), INC-I-257
//! (unbonding lock) and INC-I-182 (producer auth). The chain tip was 330_779 when
//! the six testnet producers were stopped, so no stored block is at or above
//! 330_849; the gates engage forward-only (CLAUDE.md #0 rule, no genesis reset).
//! Mainnet stays FROZEN at `u64::MAX`; devnet keeps its own value and is not
//! asserted here.
//!
// OUTPUT CONTRACT: fn NetworkParams::defaults(network: Network) -> NetworkParams
//   O1 testnet .inc_i_{236,255,257,182}_*_activation_height == 330_849 (all four)
//   O2 mainnet .inc_i_{236,255,257,182}_*_activation_height == u64::MAX (all four)
//   O3 testnet effective_slashing_activation_height() == 330_849 (236 == 255, 181 below)
//   O4 mutable params / receiver / store: NONE (pure constructor)
//   PATHS: Mainnet arm | Testnet arm (Devnet arm NOT covered: unchanged by this pin)
//   INPUT PARTITIONS: IP-M mainnet | IP-T testnet
//   MATRIX: {IP-M, IP-T} x {4 fields} -> 8 assertions below, plus O3

use crate::Network;

use super::NetworkParams;

/// The one height the four testnet gates share. Pinned 2026-10-25 with the
/// producers stopped at tip 330_779 (70 blocks of lead).
const TESTNET_DEFENSE_PIN: u64 = 330_849;

fn four_gates(params: &NetworkParams) -> [(&'static str, u64); 4] {
    [
        (
            "inc_i_236_block_signature_activation_height",
            params.inc_i_236_block_signature_activation_height,
        ),
        (
            "inc_i_255_slashing_activation_height",
            params.inc_i_255_slashing_activation_height,
        ),
        (
            "inc_i_257_unbonding_activation_height",
            params.inc_i_257_unbonding_activation_height,
        ),
        (
            "inc_i_182_producer_auth_activation_height",
            params.inc_i_182_producer_auth_activation_height,
        ),
    ]
}

// Decision: a failure means a testnet node would run the old rules at and above
// 330_849, or one gate would engage at a different block than the other three.
#[test]
fn testnet_defense_gates_pinned_at_330849() {
    let testnet = NetworkParams::defaults(Network::Testnet);
    for (name, height) in four_gates(&testnet) {
        assert_eq!(
            height, TESTNET_DEFENSE_PIN,
            "testnet {name} must be pinned at {TESTNET_DEFENSE_PIN}"
        );
    }
    assert_eq!(
        testnet.effective_slashing_activation_height(),
        TESTNET_DEFENSE_PIN,
        "testnet effective slashing height must equal the pin (236 == 255, 181 is below)"
    );
    assert!(
        testnet.inc_i_255_slashing_activation_height
            >= testnet.inc_i_257_unbonding_activation_height,
        "pin order: slashing (255) must not engage below unbonding (257)"
    );
}

// Decision: a failure means this testnet pin leaked into mainnet, which must stay
// FROZEN until its own decision-session (INC-I-054 immutability applies after).
#[test]
fn mainnet_defense_gates_stay_frozen() {
    let mainnet = NetworkParams::defaults(Network::Mainnet);
    for (name, height) in four_gates(&mainnet) {
        assert_eq!(
            height,
            u64::MAX,
            "mainnet {name} must stay u64::MAX (frozen)"
        );
    }
}

//! Pins of the four defense gates at ONE height per network (user decisions).
//!
//! The pin covers INC-I-236 (signed blocks), INC-I-255 (slashing), INC-I-257
//! (unbonding lock) and INC-I-182 (producer auth). On each network the chain tip
//! was below the pin when it was decided, so no stored block is at or above it;
//! the gates engage forward-only (CLAUDE.md #0 rule, no genesis reset). Once a
//! pin is crossed it is IMMUTABLE (INC-I-054). Devnet keeps its own value and
//! is not asserted here.
//!
// OUTPUT CONTRACT: fn NetworkParams::defaults(network: Network) -> NetworkParams
//   O1 testnet .inc_i_{236,255,257,182}_*_activation_height == 330_849 (all four)
//   O2 mainnet .inc_i_{236,255,257,182}_*_activation_height == 806_859 (all four)
//   O3 testnet effective_slashing_activation_height() == 330_849 (236 == 255, 181 below)
//   O4 mainnet effective_slashing_activation_height() == 806_859 (236 == 255, 181 below)
//   O5 mutable params / receiver / store: NONE (pure constructor)
//   PATHS: Mainnet arm | Testnet arm (Devnet arm NOT covered: unchanged by the pins)
//   INPUT PARTITIONS: IP-M mainnet | IP-T testnet
//   MATRIX: {IP-M, IP-T} x {4 fields} -> 8 assertions below, plus O3, O4 and the pin order

use crate::Network;

use super::NetworkParams;

/// The one height the four testnet gates share.
const TESTNET_DEFENSE_PIN: u64 = 330_849;

/// The one height the four mainnet gates share.
const MAINNET_DEFENSE_PIN: u64 = 806_859;

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

fn assert_pinned(network: Network, pin: u64) {
    let params = NetworkParams::defaults(network);
    for (name, height) in four_gates(&params) {
        assert_eq!(height, pin, "{network:?} {name} must be pinned at {pin}");
    }
    assert_eq!(
        params.effective_slashing_activation_height(),
        pin,
        "{network:?} effective slashing height must equal the pin (236 == 255, 181 is below)"
    );
    assert!(
        params.inc_i_255_slashing_activation_height >= params.inc_i_257_unbonding_activation_height,
        "{network:?} pin order: slashing (255) must not engage below unbonding (257)"
    );
}

// Decision: a failure means a testnet node would run the old rules at and above
// 330_849, or one gate would engage at a different block than the other three.
#[test]
fn testnet_defense_gates_pinned_at_330849() {
    assert_pinned(Network::Testnet, TESTNET_DEFENSE_PIN);
}

// Decision: a failure means a mainnet node would run the old rules at and above
// 806_859, one gate would engage at a different block than the other three, or
// the testnet pin leaked into mainnet.
#[test]
fn mainnet_defense_gates_pinned_at_806859() {
    assert_pinned(Network::Mainnet, MAINNET_DEFENSE_PIN);
}

// Decision: a failure means the two networks share one pin, which would hide a
// copy-paste of one network's height into the other.
#[test]
fn mainnet_and_testnet_pins_are_distinct() {
    assert_ne!(MAINNET_DEFENSE_PIN, TESTNET_DEFENSE_PIN);
    assert_ne!(
        NetworkParams::defaults(Network::Mainnet).inc_i_236_block_signature_activation_height,
        NetworkParams::defaults(Network::Testnet).inc_i_236_block_signature_activation_height
    );
}

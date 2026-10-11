//! INC-I-237 M2 / REQ-GRV-011 — the relay change is node-local: nothing the
//! peer handshake, the fork id or a peer's decoder depends on moves.
//!
//! OUTPUT CONTRACT: consts TRANSACTIONS_TOPIC .. ATTESTATION_TOPIC,
//!   CURRENT_PROTOCOL_VERSION, MIN_PEER_PROTOCOL_VERSION, EPOCH_STATE_FORMAT_VERSION;
//!   fn HardForkSchedule::for_network(n).all()
//!   O1-O2: none   O3: the pinned values   O4-O6: none
//! PATHS: P1 one read per constant / network
//! INPUT PARTITIONS: network {Mainnet, Testnet, Devnet}; topic {5 subscribed}
//! MATRIX: O3 x every constant and network. Falsifier: any M2 edit that moves
//!   one of them (a version bump deletes epoch state on restart, INC-I-054; a
//!   topic or schedule change partitions mixed versions) turns this red. The
//!   message-id function and tx wire bytes are pinned on the wire in
//!   crates/network/tests/it/inc_i_237_m2_origin.rs and the M0 F-10 corpus.

use doli_core::Network;
use network::gossip::{
    ATTESTATION_TOPIC, BLOCKS_TOPIC, PRODUCERS_TOPIC, TRANSACTIONS_TOPIC, VOTES_TOPIC,
};
use network::protocols::status::{
    CURRENT_PROTOCOL_VERSION, EPOCH_STATE_FORMAT_VERSION, MIN_PEER_PROTOCOL_VERSION,
};
use updater::hardfork::HardForkSchedule;

// REQ-GRV-011 — Decision: a failure means M2 moved a consensus- or handshake-visible value (topic, protocol or epoch-state version, hard-fork schedule), so the rolling-deploy / no-activation-height answer no longer holds.
#[test]
fn req_grv_011_topics_versions_and_fork_schedule_are_unchanged() {
    assert_eq!(
        [
            TRANSACTIONS_TOPIC,
            BLOCKS_TOPIC,
            ATTESTATION_TOPIC,
            VOTES_TOPIC,
            PRODUCERS_TOPIC
        ],
        [
            "/doli/txs/1",
            "/doli/blocks/1",
            "/doli/attestations/1",
            "/doli/votes/1",
            "/doli/producers/1"
        ]
    );
    assert_eq!(
        (
            CURRENT_PROTOCOL_VERSION,
            MIN_PEER_PROTOCOL_VERSION,
            EPOCH_STATE_FORMAT_VERSION
        ),
        (8, 1, 1)
    );
    for (net, heights) in [
        (Network::Mainnet, vec![]),
        (Network::Testnet, vec![3_100u64, 4_836]),
        (Network::Devnet, vec![]),
    ] {
        let got: Vec<u64> = HardForkSchedule::for_network(net)
            .all()
            .iter()
            .map(|f| f.activation_height)
            .collect();
        assert_eq!(got, heights, "{net:?} HardForkSchedule");
    }
}

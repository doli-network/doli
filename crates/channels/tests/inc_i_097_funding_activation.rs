//! INC-I-097 — Payment channel state never transitions FundingBroadcast -> Active.
//!
//! Root cause (confirmed): no caller observes funding-tx confirmation and applies
//! the (already-valid) FundingBroadcast -> Active transition, so `channel pay`
//! permanently fails with "Channel is not active (state: FundingBroadcast)".
//!
//! Fix under test: `ChannelRecord::try_activate(confirmations, required) -> bool`,
//! a pure, I/O-free method that records the observed confirmation count and
//! advances a FundingBroadcast channel to Active once it reaches the required
//! confirmation depth. The CLI Pay path calls this after querying the node.
//!
// OUTPUT CONTRACT: fn ChannelRecord::try_activate(&mut self, confirmations: u32, required: u32) -> bool
//   Observable outputs:
//     O1: return bool (true iff state changed to Active)
//     O2: self.state          (mutated: FundingBroadcast -> Active on success)
//     O3: self.funding_confirmations (mutated: records observed confs when in FundingBroadcast)
//   PATHS:
//     P1: state != FundingBroadcast               -> return false, NO mutation
//     P2: FundingBroadcast & confs <  required(>=1)-> record confs, NO transition, return false
//     P3: FundingBroadcast & confs >= required(>=1)-> record confs, transition Active, return true
//   MATRIX: {O1,O2,O3} x {P1,P2,P3} asserted below.
//
// INPUT PARTITIONS:
//   P1 (non-FundingBroadcast state): {Active (already activated), Closed (terminal)}
//        - distinct logical class: guard must reject any non-FundingBroadcast state.
//   P2 (insufficient confirmations): {confs=0/required=1 (zero), confs=2/required=3 (below threshold)}
//        - distinct class: observed < required, including the degenerate zero-confirmation case.
//   P3 (sufficient confirmations):   {confs=1/required=1 (boundary equality), confs=5/required=3 (strictly above)}
//        - distinct class: observed >= required, exercising both the == boundary and the > case.
//   Defensive partition (required=0): {confs=0/required=0} must NOT activate (zero-conf guard via required.max(1)).

use channels::channel::ChannelRecord;
use channels::types::{ChannelBalance, ChannelId, ChannelState, FundingOutpoint};

fn funding_broadcast_channel() -> ChannelRecord {
    ChannelRecord {
        channel_id: ChannelId([7u8; 32]),
        state: ChannelState::FundingBroadcast,
        local_pubkey_hash: [1u8; 32],
        remote_pubkey_hash: [2u8; 32],
        funding_outpoint: FundingOutpoint {
            tx_hash: [7u8; 32],
            output_index: 0,
        },
        capacity: 1_000_000,
        balance: ChannelBalance::new(1_000_000, 0),
        commitment_number: 0,
        channel_seed: [0u8; 32],
        revocation_store: Default::default(),
        dispute_window: 20,
        htlcs: Vec::new(),
        funding_confirmations: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        close_tx_hash: None,
        penalty_tx_hash: None,
    }
}

// ── P3: sufficient confirmations → activate ───────────────────────────────

#[test]
fn p3_boundary_equal_confirmations_activates() {
    // confs == required (testnet depth = 1): the channel must go Active.
    let mut ch = funding_broadcast_channel();
    let changed = ch.try_activate(1, 1);
    assert!(
        changed,
        "O1: try_activate must return true at the boundary confs==required"
    );
    assert_eq!(
        ch.state,
        ChannelState::Active,
        "O2: state must advance to Active"
    );
    assert_eq!(
        ch.funding_confirmations, 1,
        "O3: observed confirmations recorded"
    );
    assert!(ch.state.is_active(), "pay guard (is_active) must now pass");
}

#[test]
fn p3_above_threshold_activates() {
    // confs strictly above required (mainnet depth = 3, observed = 5).
    let mut ch = funding_broadcast_channel();
    let changed = ch.try_activate(5, 3);
    assert!(changed, "O1: true when confs > required");
    assert_eq!(ch.state, ChannelState::Active, "O2: Active");
    assert_eq!(ch.funding_confirmations, 5, "O3: records observed confs");
}

// ── P2: insufficient confirmations → stay FundingBroadcast ─────────────────

#[test]
fn p2_zero_confirmations_does_not_activate() {
    let mut ch = funding_broadcast_channel();
    let changed = ch.try_activate(0, 1);
    assert!(!changed, "O1: false when unconfirmed");
    assert_eq!(
        ch.state,
        ChannelState::FundingBroadcast,
        "O2: state unchanged — pay guard must still block"
    );
    assert_eq!(
        ch.funding_confirmations, 0,
        "O3: zero observed confs recorded"
    );
}

#[test]
fn p2_below_threshold_does_not_activate_but_records_confs() {
    // mainnet depth = 3, observed = 2: not enough yet, but record progress.
    let mut ch = funding_broadcast_channel();
    let changed = ch.try_activate(2, 3);
    assert!(!changed, "O1: false below threshold");
    assert_eq!(
        ch.state,
        ChannelState::FundingBroadcast,
        "O2: still FundingBroadcast"
    );
    assert_eq!(ch.funding_confirmations, 2, "O3: partial progress recorded");
}

// ── P1: non-FundingBroadcast state → no-op ─────────────────────────────────

#[test]
fn p1_already_active_is_noop() {
    let mut ch = funding_broadcast_channel();
    ch.state = ChannelState::Active;
    let changed = ch.try_activate(10, 1);
    assert!(
        !changed,
        "O1: idempotent — already-Active channel is not re-activated"
    );
    assert_eq!(ch.state, ChannelState::Active, "O2: stays Active");
}

#[test]
fn p1_terminal_state_is_noop() {
    let mut ch = funding_broadcast_channel();
    ch.state = ChannelState::Closed;
    let changed = ch.try_activate(99, 1);
    assert!(!changed, "O1: closed channel cannot be activated");
    assert_eq!(ch.state, ChannelState::Closed, "O2: stays Closed");
}

// ── Defensive: required=0 must not activate on zero confirmations ───────────

#[test]
fn defensive_zero_required_zero_confs_does_not_activate() {
    // Guard against a misconfigured required=0 silently activating an
    // unconfirmed channel. require at least 1 confirmation.
    let mut ch = funding_broadcast_channel();
    let changed = ch.try_activate(0, 0);
    assert!(
        !changed,
        "O1: never activate with zero confirmations, even if required==0"
    );
    assert_eq!(ch.state, ChannelState::FundingBroadcast, "O2: unchanged");
}

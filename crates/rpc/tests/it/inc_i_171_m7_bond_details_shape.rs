//! INC-I-171 M7 — the vesting ladder and the wire shape that `getBondDetails` serves.
//!
//! OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//!   F1: withdrawal_penalty_rate_with_quarter(age, quarter) -> u8, and
//!       penalized_bond_net(amount, creation_slot, block_slot, quarter) -> u64.
//!       Pure free fns. Mutable params: NONE. Receiver mutation: NONE. Store writes: NONE.
//!       O1: the return value — the only observable output.
//!       PATHS: quarters 0 / 1 / 2 / >=3. MATRIX: O1 x every path, at both edges of every tier.
//!   F2: <BondDetailsResponse as Serialize>::serialize -> serde_json::Value.
//!       O1: the JSON object — key set and values. PATHS: one (derive). Store writes: NONE.
//!   F3: source-content tripwire over crates/rpc/src/methods/producer.rs. O1: assertion verdict.
//!   INPUT PARTITIONS: amount 1_000_007 (truncation-visible at 75/50/25) at the testnet quarter
//!     2160; a bonds vector with one entry (the empty-vector case is the serde `default`).
//!   NOT CLAIMED HERE: the end-to-end RPC handler (needs a live RpcContext); the payout bound.

use doli_core::consensus::withdrawal_penalty_rate_with_quarter;
use doli_core::validation::vesting::penalized_bond_net;
use rpc::types::{BondDetailsResponse, BondEntryResponse, BondsSummaryResponse};

const Q: u32 = 2_160;
const AMOUNT: u64 = 1_000_007;

// REQ-VEST-002 (Must) / INV-VEST-004 — Decision: reveals that the M7 rewrite of the bond-tier
// computation moved a testnet bond into a different vesting quarter than the node charges it in.
#[test]
fn req_vest_002_bond_details_ladder_boundaries_at_the_testnet_quarter() {
    let table: [(u32, u8, u64); 8] = [
        (0, 75, 250_002),
        (Q - 1, 75, 250_002),
        (Q, 50, 500_004),
        (2 * Q - 1, 50, 500_004),
        (2 * Q, 25, 750_006),
        (3 * Q - 1, 25, 750_006),
        (3 * Q, 0, AMOUNT),
        (4 * Q, 0, AMOUNT),
    ];
    for (age, expected_pct, expected_net) in table {
        assert_eq!(
            withdrawal_penalty_rate_with_quarter(age, Q),
            expected_pct,
            "REQ-VEST-002: age {age} at quarter {Q} must report tier {expected_pct}%"
        );
        assert_eq!(
            penalized_bond_net(AMOUNT, 0, age, Q),
            expected_net,
            "REQ-VEST-002: age {age} must net {expected_net} (penalty truncated, then subtracted)"
        );
    }
}

fn sample_response() -> BondDetailsResponse {
    BondDetailsResponse {
        public_key: "aabb".to_string(),
        bond_count: 2,
        total_staked: 3_000,
        registration_slot: 1_000,
        age_slots: 9_000,
        penalty_pct: 0,
        vested: true,
        maturation_slot: 9_640,
        vesting_quarter_slots: Q as u64,
        vesting_period_slots: 4 * Q as u64,
        summary: BondsSummaryResponse {
            q1: 1,
            q2: 0,
            q3: 0,
            vested: 1,
        },
        bonds: vec![BondEntryResponse {
            creation_slot: 8_000,
            amount: 101,
            age_slots: 2_000,
            penalty_pct: 75,
            vested: false,
            maturation_slot: 16_640,
        }],
        withdrawal_pending_count: 0,
    }
}

fn sorted_keys(v: &serde_json::Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

// REQ-VEST-002 (Must) — Decision: reveals that the M7 edit renamed, dropped or added a
// getBondDetails field, silently breaking every wallet and explorer that reads the tier.
#[test]
fn req_vest_002_bond_details_response_serializes_the_shipped_camel_case_shape() {
    let v = serde_json::to_value(sample_response()).unwrap();
    assert_eq!(
        sorted_keys(&v),
        vec![
            "ageSlots",
            "bondCount",
            "bonds",
            "maturationSlot",
            "penaltyPct",
            "publicKey",
            "registrationSlot",
            "summary",
            "totalStaked",
            "vested",
            "vestingPeriodSlots",
            "vestingQuarterSlots",
            "withdrawalPendingCount",
        ],
        "REQ-VEST-002: getBondDetails ships exactly these top-level keys"
    );
    assert_eq!(v["publicKey"], "aabb");
    assert_eq!(v["bondCount"], 2);
    assert_eq!(v["totalStaked"], 3_000);
    assert_eq!(v["registrationSlot"], 1_000);
    assert_eq!(v["ageSlots"], 9_000);
    assert_eq!(v["penaltyPct"], 0);
    assert_eq!(v["vested"], true);
    assert_eq!(v["maturationSlot"], 9_640);
    assert_eq!(v["vestingQuarterSlots"], 2_160);
    assert_eq!(v["vestingPeriodSlots"], 8_640);
    assert_eq!(v["withdrawalPendingCount"], 0);
    assert_eq!(sorted_keys(&v["summary"]), vec!["q1", "q2", "q3", "vested"]);
    assert_eq!(v["summary"]["q1"], 1);
    assert_eq!(v["summary"]["vested"], 1);
}

// REQ-VEST-002 (Must) — Decision: reveals a per-bond wire change, including a net-amount field
// invented during M7 that would put a second penalty implementation on the wire.
#[test]
fn req_vest_002_bond_entry_serializes_six_keys_and_no_net_amount() {
    let v = serde_json::to_value(sample_response()).unwrap();
    let entry = &v["bonds"][0];
    assert_eq!(
        sorted_keys(entry),
        vec![
            "ageSlots",
            "amount",
            "creationSlot",
            "maturationSlot",
            "penaltyPct",
            "vested",
        ],
        "REQ-VEST-002: a bond entry ships exactly these keys — no net field exists today"
    );
    assert_eq!(entry["creationSlot"], 8_000);
    assert_eq!(entry["amount"], 101);
    assert_eq!(entry["ageSlots"], 2_000);
    assert_eq!(entry["penaltyPct"], 75);
    assert_eq!(entry["vested"], false);
    assert_eq!(entry["maturationSlot"], 16_640);
}

const PRODUCER_SRC: &str = include_str!("../../src/methods/producer.rs");

// REQ-VEST-002 (Must) — Decision: reveals that getBondDetails still narrows a u64 slot with a
// silent `as u32`, which reports the wrong vesting tier instead of failing.
#[test]
fn inc_i_171_m7_rpc_keeps_no_lossy_slot_narrowing() {
    assert!(
        !PRODUCER_SRC.contains(concat!("age as ", "u32")),
        "REQ-VEST-002: crates/rpc/src/methods/producer.rs must not narrow a u64 bond age with \
         `as` (:316, :352) — a truncating cast reports a tier the node never charges"
    );
    assert!(
        !PRODUCER_SRC.contains(concat!("quarter as ", "u32")),
        "REQ-VEST-002: crates/rpc/src/methods/producer.rs must not narrow the u64 \
         vesting_quarter_slots with `as` (:316, :352) — a truncated quarter changes every tier"
    );
}

// REQ-VEST-002 (Must) — Decision: reveals that the M7 edit inlined the ladder into the RPC
// instead of calling the shared one.
#[test]
fn inc_i_171_m7_rpc_still_calls_the_shared_ladder() {
    assert!(
        PRODUCER_SRC.contains("withdrawal_penalty_rate_with_quarter"),
        "REQ-VEST-002: getBondDetails must keep calling the core ladder, not a local copy"
    );
}

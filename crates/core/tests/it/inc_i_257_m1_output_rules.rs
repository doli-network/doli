//! INC-I-257 M1 — output rules: at/above the gate a RequestWithdrawal pays
//! exactly one Unbonding output to the producer's own address, empty
//! extra_data, locked for at least the unbonding period; an Unbonding output
//! anywhere else, or below the gate, is refused.
//!
// covers: validation transaction tx_types output_authority
//
// OUTPUT CONTRACT: fn validate_transaction(&Transaction, &ValidationContext) -> Result<(), ValidationError>
//   O1 return: Ok | Err (new refusals carry "ERRTX-UNBOND" in the message); pure, no store writes
// PATHS: P-OK gated payout | P-NORMAL Normal payout at/above | P-OWNER | P-LOCK | P-EXTRA | P-COUNT
//        | P-TYPE Unbonding output in another tx type | P-BELOW any Unbonding output below the gate | P-SAT
// INPUT PARTITIONS:
//   height: gate - 1 | gate | gate + 500; INC-I-234 placement gate: active (0) | inactive (MAX)
//   lock: H + P - 1 | H + P | H + P + 1 | MAX | 0; owner: bond_owner(producer) | other (destination also other)
//   extra_data: empty | 1 byte; outputs: 1 | 2; tx type: RequestWithdrawal | Transfer | AddBond
//   P-SAT: H = u64::MAX - 1, P = 60 -> floor saturates to u64::MAX
//   P-DATA: gated payout whose withdrawal data does not decode (owner not derivable)
// MATRIX:
//   O1 x P-DATA -> inc_i_257_m1_req_unbond_sec_001_undecodable_withdrawal_data_refused
//   O1 x P-OK -> inc_i_257_m1_req_unbond_004_gated_payout_accepted_at_and_above_gate
//   O1 x P-NORMAL -> inc_i_257_m1_req_unbond_004_normal_payout_refused_at_or_above_gate
//   O1 x P-OWNER -> inc_i_257_m1_req_unbond_004_wrong_owner_refused
//   O1 x P-LOCK -> inc_i_257_m1_req_unbond_004_short_lock_refused
//   O1 x P-EXTRA -> inc_i_257_m1_req_unbond_004_non_empty_extra_data_refused
//   O1 x P-COUNT -> inc_i_257_m1_req_unbond_004_two_outputs_refused
//   O1 x P-TYPE -> inc_i_257_m1_req_unbond_004_unbonding_output_refused_in_other_tx_types
//   O1 x P-BELOW -> inc_i_257_m1_req_unbond_004_unbonding_output_refused_below_gate
//   O1 x P-SAT -> inc_i_257_m1_req_unbond_sec_001_lock_floor_saturates

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{Input, Output, OutputType, Transaction};
use doli_core::validation::{bond_owner, validate_transaction, ValidationContext, ValidationError};
use doli_core::Network;

const GATE: u64 = 1_000;
const P: u64 = 60;
const AUTH: [u64; 2] = [0, u64::MAX];

fn producer() -> PublicKey {
    *KeyPair::from_seed([0x57; 32]).public_key()
}

fn other() -> Hash {
    Hash::from_bytes([0x99; 32])
}

fn ctx(height: u64, auth234: u64) -> ValidationContext {
    ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, height)
        .with_inc_i_234_activation_height(auth234)
        .with_inc_i_257_unbonding(GATE, P)
}

fn unbonding(owner: Hash, lock_until: u64, extra_data: Vec<u8>) -> Output {
    Output {
        output_type: OutputType::Unbonding,
        amount: 900,
        pubkey_hash: owner,
        lock_until,
        extra_data,
    }
}

fn input() -> Vec<Input> {
    let mut i = Input::new(Hash::from_bytes([0x42; 32]), 0);
    i.public_key = Some(producer());
    vec![i]
}

fn withdrawal(destination: Hash, outputs: Vec<Output>) -> Transaction {
    let mut tx = Transaction::new_request_withdrawal(input(), producer(), 1, destination, 900);
    tx.outputs = outputs;
    tx
}

fn gated(lock_until: u64) -> Transaction {
    let own = bond_owner(&producer());
    withdrawal(own, vec![unbonding(own, lock_until, Vec::new())])
}

fn is_unbond(r: &Result<(), ValidationError>) -> bool {
    matches!(r, Err(e) if e.to_string().contains("ERRTX-UNBOND"))
}

// REQ-UNBOND-004 — Decision: a failure means the honest gated withdrawal (the only legal shape) is refused, so no producer can withdraw after the gate.
#[test]
fn inc_i_257_m1_req_unbond_004_gated_payout_accepted_at_and_above_gate() {
    for auth in AUTH {
        for h in [GATE, GATE + 500] {
            for lock in [h + P, h + P + 1, u64::MAX] {
                let r = validate_transaction(&gated(lock), &ctx(h, auth));
                assert!(r.is_ok(), "auth234={auth} H={h} lock={lock}: {r:?}");
            }
        }
    }
}

// REQ-UNBOND-004 / REQ-UNBOND-001 — Decision: a failure means a withdrawal can still pay an instantly spendable Normal output at/above the gate, or the old shape is refused below it.
#[test]
fn inc_i_257_m1_req_unbond_004_normal_payout_refused_at_or_above_gate() {
    let own = bond_owner(&producer());
    let tx = withdrawal(own, vec![Output::normal(900, own)]);
    for auth in AUTH {
        let below = validate_transaction(&tx, &ctx(GATE - 1, auth));
        assert!(
            below.is_ok(),
            "auth234={auth}: below the gate the Normal payout stays valid: {below:?}"
        );
        for h in [GATE, GATE + 500] {
            let r = validate_transaction(&tx, &ctx(h, auth));
            assert!(
                is_unbond(&r),
                "auth234={auth} H={h}: Normal payout must be refused: {r:?}"
            );
        }
    }
}

// REQ-UNBOND-004 — Decision: a failure means the withdrawn value can be locked under a key the slash burn does not scan, so it escapes the burn.
#[test]
fn inc_i_257_m1_req_unbond_004_wrong_owner_refused() {
    let lock = GATE + P;
    let foreign = withdrawal(other(), vec![unbonding(other(), lock, Vec::new())]);
    let own_dest = withdrawal(
        bond_owner(&producer()),
        vec![unbonding(other(), lock, Vec::new())],
    );
    for auth in AUTH {
        let r = validate_transaction(&foreign, &ctx(GATE, auth));
        assert!(
            is_unbond(&r),
            "auth234={auth}: payout to a foreign owner: {r:?}"
        );
        let r = validate_transaction(&own_dest, &ctx(GATE, auth));
        assert!(
            r.is_err(),
            "auth234={auth}: output owner differs from destination: {r:?}"
        );
    }
}

// REQ-UNBOND-004 — Decision: a failure means a withdrawal can unlock before the unbonding period ends (off by one or no floor at all).
#[test]
fn inc_i_257_m1_req_unbond_004_short_lock_refused() {
    for auth in AUTH {
        for h in [GATE, GATE + 500] {
            for lock in [h + P - 1, h, 1] {
                let r = validate_transaction(&gated(lock), &ctx(h, auth));
                assert!(is_unbond(&r), "auth234={auth} H={h} lock={lock}: {r:?}");
            }
            assert!(
                validate_transaction(&gated(0), &ctx(h, auth)).is_err(),
                "lock 0"
            );
            assert!(
                validate_transaction(&gated(h + P), &ctx(h, auth)).is_ok(),
                "boundary H + P"
            );
        }
    }
}

// REQ-UNBOND-004 — Decision: a failure means an Unbonding output can carry arbitrary payload bytes (state bloat, or a future reinterpretation of the field).
#[test]
fn inc_i_257_m1_req_unbond_004_non_empty_extra_data_refused() {
    let own = bond_owner(&producer());
    for extra in [vec![0u8], vec![0xFF; 4]] {
        let tx = withdrawal(own, vec![unbonding(own, GATE + P, extra.clone())]);
        for auth in AUTH {
            let r = validate_transaction(&tx, &ctx(GATE, auth));
            assert!(r.is_err(), "auth234={auth} extra={extra:?}: {r:?}");
        }
    }
}

// REQ-UNBOND-004 — Decision: a failure means a withdrawal can split its payout so part of it is not the single locked output.
#[test]
fn inc_i_257_m1_req_unbond_004_two_outputs_refused() {
    let own = bond_owner(&producer());
    let lock = GATE + P;
    for outputs in [
        vec![
            unbonding(own, lock, Vec::new()),
            unbonding(own, lock, Vec::new()),
        ],
        vec![unbonding(own, lock, Vec::new()), Output::normal(1, own)],
    ] {
        for auth in AUTH {
            let r = validate_transaction(&withdrawal(own, outputs.clone()), &ctx(GATE, auth));
            assert!(
                r.is_err(),
                "auth234={auth} {} outputs: {r:?}",
                outputs.len()
            );
        }
    }
}

// REQ-UNBOND-004 — Decision: a failure means any user can mint an output of the slashable-collateral type outside a withdrawal (a Transfer creating one).
#[test]
fn inc_i_257_m1_req_unbond_004_unbonding_output_refused_in_other_tx_types() {
    let own = bond_owner(&producer());
    let normal = Transaction::new_transfer(input(), vec![Output::normal(900, own)]);
    let locked = Transaction::new_transfer(input(), vec![unbonding(own, GATE + P, Vec::new())]);
    let mut add_bond = Transaction::new_add_bond(input(), producer(), 1, 900, u64::MAX);
    add_bond.outputs.push(unbonding(own, GATE + P, Vec::new()));
    for auth in AUTH {
        let c = ctx(GATE, auth);
        assert!(
            validate_transaction(&normal, &c).is_ok(),
            "NON-VACUITY: plain Transfer is valid"
        );
        let r = validate_transaction(&locked, &c);
        assert!(
            is_unbond(&r),
            "auth234={auth}: Transfer with an Unbonding output: {r:?}"
        );
        let r = validate_transaction(&add_bond, &c);
        assert!(
            r.is_err(),
            "auth234={auth}: AddBond with an Unbonding output: {r:?}"
        );
    }
}

// REQ-UNBOND-004 / REQ-UNBOND-017 — Decision: a failure means an Unbonding output is accepted at a height where old binaries cannot decode it (a fork before activation).
#[test]
fn inc_i_257_m1_req_unbond_004_unbonding_output_refused_below_gate() {
    let own = bond_owner(&producer());
    for auth in AUTH {
        for h in [0, GATE - 1] {
            let r = validate_transaction(&gated(h + P + 5), &ctx(h, auth));
            assert!(
                is_unbond(&r),
                "auth234={auth} H={h}: gated withdrawal below the gate: {r:?}"
            );
            let t = Transaction::new_transfer(input(), vec![unbonding(own, h + P + 5, Vec::new())]);
            let r = validate_transaction(&t, &ctx(h, auth));
            assert!(
                is_unbond(&r),
                "auth234={auth} H={h}: Transfer with an Unbonding output: {r:?}"
            );
        }
        let frozen =
            ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, 5_000_000)
                .with_inc_i_234_activation_height(auth)
                .with_inc_i_257_unbonding(u64::MAX, 60_480);
        let r = validate_transaction(&gated(5_000_000 + 60_480), &frozen);
        assert!(
            is_unbond(&r),
            "auth234={auth}: frozen gate (mainnet value): {r:?}"
        );
    }
}

// REQ-UNBOND-SEC-001 — Decision: a failure means `H + period` overflows (a panic, or a wrapped floor that accepts any lock) at the top of the height range.
#[test]
fn inc_i_257_m1_req_unbond_sec_001_lock_floor_saturates() {
    let h = u64::MAX - 1;
    for auth in AUTH {
        let c = ctx(h, auth);
        let r = validate_transaction(&gated(u64::MAX), &c);
        assert!(
            r.is_ok(),
            "auth234={auth}: lock u64::MAX meets the saturated floor: {r:?}"
        );
        let r = validate_transaction(&gated(u64::MAX - 1), &c);
        assert!(
            is_unbond(&r),
            "auth234={auth}: lock below the saturated floor: {r:?}"
        );
        let r = validate_transaction(&gated(5), &c);
        assert!(
            is_unbond(&r),
            "auth234={auth}: a wrapped floor must not accept a small lock: {r:?}"
        );
    }
}

// REQ-UNBOND-SEC-001 — Decision: a failure means a gated payout whose withdrawal data does not decode passes the owner rule, so its owner is never checked against the producer.
#[test]
fn inc_i_257_m1_req_unbond_sec_001_undecodable_withdrawal_data_refused() {
    for auth in AUTH {
        let c = ctx(GATE, auth);
        assert!(
            validate_transaction(&gated(GATE + P), &c).is_ok(),
            "NON-VACUITY: auth234={auth}: the intact payout is valid"
        );
        let mut tx = gated(GATE + P);
        tx.extra_data.truncate(1);
        let r = validate_transaction(&tx, &c);
        assert!(
            r.is_err(),
            "auth234={auth}: undecodable withdrawal data: {r:?}"
        );
    }
}

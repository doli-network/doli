//! INC-I-257 M1 — the CLI builds the gated withdrawal (one Unbonding output to
//! the producer's own address, locked past the consensus floor by a margin, fee
//! taken from the withdrawn amount) at/above the gate and today's shape below
//! it; wallet send picks Unbonding outputs only once spendable.
//!
// covers: cmd_producer withdrawal cmd_wallet unbonding_tx
//
// OUTPUT CONTRACT: fn doli_cli::unbonding_tx::build_withdrawal_tx(&NetworkParams, best_height: u64, &WithdrawalSpec) -> Result<Transaction, String>
//   O1 return: unsigned tx (type, inputs, outputs, withdrawal data) | Err(reason); pure, no I/O
// OUTPUT CONTRACT: fn doli_cli::unbonding_tx::send_selectable(output_type: &str, spendable: bool) -> bool
//   O2 return: whether wallet send may use the UTXO
// PATHS: P-GATED best + 1 >= gate | P-BELOW best + 1 < gate | P-ERR foreign destination, dust, no fee UTXO below the gate
// INPUT PARTITIONS:
//   best height: gate - 1 (next block at the gate) | far above | gate - 2 (below) | mainnet below its gate
//   fee UTXOs offered: none | one | two (below the gate only the first is needed)
//   validity window: heights best + 1 and best + MARGIN valid, best + MARGIN + 1 refused
//   send: (unbonding|normal|bond|vesting) x (spendable|locked)
// MATRIX:
//   O1 x P-GATED -> inc_i_257_m1_req_unbond_009_gated_shape_at_and_above_gate,
//                   inc_i_257_m1_req_unbond_009_gated_tx_valid_for_the_margin_window,
//                   inc_i_257_m1_req_unbond_009_fee_from_withdrawn_amount_no_wallet_utxo_locked
//   O1 x P-BELOW -> inc_i_257_m1_req_unbond_009_below_gate_keeps_today_shape
//   O1 x P-ERR -> inc_i_257_m1_req_unbond_009_gated_refuses_foreign_destination_and_dust,
//                 inc_i_257_m1_req_unbond_009_below_gate_needs_a_fee_utxo
//   O2 -> inc_i_257_m1_req_unbond_009_send_selects_unbonding_once_spendable

use crypto::{Hash, KeyPair, PublicKey};
use doli_cli::unbonding_tx::{
    build_withdrawal_tx, send_selectable, FeeUtxo, WithdrawalSpec, UNBONDING_LOCK_MARGIN,
};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{OutputType, Transaction, TxType};
use doli_core::validation::{bond_owner, validate_transaction, ValidationContext};
use doli_core::{Network, NetworkParams};

const GATE: u64 = 1_000;
const NET: u64 = 1_998_000;
const FEE: u64 = 1;

fn producer() -> PublicKey {
    *KeyPair::from_seed([0x57; 32]).public_key()
}

fn params(gate: u64) -> NetworkParams {
    let mut p = NetworkParams::defaults(Network::Devnet);
    p.inc_i_257_unbonding_activation_height = gate;
    p
}

fn bonds() -> Vec<(Hash, u32)> {
    vec![
        (Hash::from_bytes([0xB1; 32]), 0),
        (Hash::from_bytes([0xB2; 32]), 3),
    ]
}

fn fee_utxos() -> Vec<FeeUtxo> {
    vec![
        FeeUtxo {
            tx_hash: Hash::from_bytes([0xF1; 32]),
            output_index: 1,
            amount: 5_000,
        },
        FeeUtxo {
            tx_hash: Hash::from_bytes([0xF2; 32]),
            output_index: 0,
            amount: 7_000,
        },
    ]
}

fn spec(destination: Hash, total_net: u64, fee_utxos: Vec<FeeUtxo>) -> WithdrawalSpec {
    WithdrawalSpec {
        producer: producer(),
        bond_count: 2,
        bond_inputs: bonds(),
        total_net,
        destination,
        fee_utxos,
        fee: FEE,
    }
}

fn own() -> Hash {
    bond_owner(&producer())
}

fn outpoints(tx: &Transaction) -> Vec<(Hash, u32)> {
    tx.inputs
        .iter()
        .map(|i| (i.prev_tx_hash, i.output_index))
        .collect()
}

fn ctx(p: &NetworkParams, height: u64) -> ValidationContext {
    ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, height)
        .with_inc_i_234_activation_height(0)
        .with_inc_i_257_unbonding(p.inc_i_257_unbonding_activation_height, p.unbonding_period)
}

// REQ-UNBOND-009 — Decision: a failure means the CLI emits a withdrawal that consensus refuses at/above the gate (wrong type, owner, lock or data), so producers cannot withdraw with the shipped client.
#[test]
fn inc_i_257_m1_req_unbond_009_gated_shape_at_and_above_gate() {
    let p = params(GATE);
    for best in [GATE - 1, GATE + 4_000] {
        let tx =
            build_withdrawal_tx(&p, best, &spec(own(), NET, fee_utxos())).expect("gated build");
        assert_eq!(tx.tx_type, TxType::RequestWithdrawal);
        assert_eq!(outpoints(&tx), bonds(), "best={best}: only the Bond inputs");
        assert_eq!(tx.outputs.len(), 1, "best={best}: exactly one output");
        let out = &tx.outputs[0];
        assert_eq!(out.output_type, OutputType::Unbonding, "best={best}");
        assert_eq!(
            out.pubkey_hash,
            own(),
            "best={best}: payout to the producer's own address"
        );
        assert!(out.extra_data.is_empty(), "best={best}: empty extra_data");
        assert_eq!(
            out.amount,
            NET - FEE,
            "best={best}: fee taken from the withdrawn amount"
        );
        assert_eq!(
            out.lock_until,
            best + p.unbonding_period + UNBONDING_LOCK_MARGIN,
            "best={best}"
        );
        assert!(
            out.lock_until > best + 1 + p.unbonding_period,
            "best={best}: lock must sit strictly above the floor of the next block"
        );
        let data = tx.withdrawal_request_data().expect("withdrawal data");
        assert_eq!(
            (data.producer_pubkey, data.bond_count, data.destination),
            (producer(), 2, own()),
            "best={best}: withdrawal data"
        );
    }
}

// REQ-UNBOND-009 — Decision: a failure means the CLI lock margin is zero or negative (the request is refused at once) or the tx stays valid outside the documented window.
#[test]
fn inc_i_257_m1_req_unbond_009_gated_tx_valid_for_the_margin_window() {
    let p = params(GATE);
    let best = GATE + 10;
    let tx = build_withdrawal_tx(&p, best, &spec(own(), NET, fee_utxos())).expect("gated build");
    for h in [best + 1, best + UNBONDING_LOCK_MARGIN] {
        let r = validate_transaction(&tx, &ctx(&p, h));
        assert!(r.is_ok(), "mined at {h} (best {best}): {r:?}");
    }
    let late = best + UNBONDING_LOCK_MARGIN + 1;
    assert!(
        validate_transaction(&tx, &ctx(&p, late)).is_err(),
        "mined at {late}: the lock is below the floor, so consensus refuses it"
    );
}

// REQ-UNBOND-009 — Decision: a failure means the CLI pays the fee from an unrelated wallet UTXO whose change would then be locked with the bond for the whole unbonding period.
#[test]
fn inc_i_257_m1_req_unbond_009_fee_from_withdrawn_amount_no_wallet_utxo_locked() {
    let p = params(0);
    for offered in [Vec::new(), fee_utxos()] {
        let n = offered.len();
        let tx = build_withdrawal_tx(&p, 50, &spec(own(), NET, offered)).expect("gated build");
        assert_eq!(
            outpoints(&tx),
            bonds(),
            "{n} fee UTXOs offered: no wallet input is used"
        );
        let paid: u64 = tx.outputs.iter().map(|o| o.amount).sum();
        assert_eq!(
            paid + FEE,
            NET,
            "{n} fee UTXOs offered: outputs + fee == withdrawn amount"
        );
    }
}

// REQ-UNBOND-009 / REQ-UNBOND-017 — Decision: a failure means the CLI switches shape before the gate (a refused tx on a network that has not activated) or loses today's shape.
#[test]
fn inc_i_257_m1_req_unbond_009_below_gate_keeps_today_shape() {
    let dest = Hash::from_bytes([0x99; 32]);
    let p = params(GATE);
    let best = GATE - 2;
    let tx =
        build_withdrawal_tx(&p, best, &spec(dest, NET, fee_utxos())).expect("below-gate build");
    let mut want_inputs = bonds();
    want_inputs.push((Hash::from_bytes([0xF1; 32]), 1));
    assert_eq!(
        outpoints(&tx),
        want_inputs,
        "Bond inputs plus the first fee UTXO"
    );
    assert_eq!(tx.outputs.len(), 1);
    let out = &tx.outputs[0];
    assert_eq!(
        (out.output_type, out.pubkey_hash, out.lock_until, out.amount),
        (OutputType::Normal, dest, 0, NET + 5_000 - FEE),
        "today's payout: Normal, free destination, unlocked, fee change rolled in"
    );
    let r = validate_transaction(&tx, &ctx(&p, best + 1));
    assert!(
        r.is_ok(),
        "below-gate shape stays valid at {}: {r:?}",
        best + 1
    );

    let mainnet = NetworkParams::defaults(Network::Mainnet);
    let tx = build_withdrawal_tx(&mainnet, 400_000, &spec(dest, NET, fee_utxos()))
        .expect("mainnet build");
    assert_eq!(
        tx.outputs[0].output_type,
        OutputType::Normal,
        "mainnet below its gate: today's shape"
    );
}

// REQ-UNBOND-009 — Decision: a failure means the CLI builds a gated withdrawal consensus refuses (foreign payout owner) or underflows the payout when the fee exceeds it.
#[test]
fn inc_i_257_m1_req_unbond_009_gated_refuses_foreign_destination_and_dust() {
    let p = params(0);
    let foreign = Hash::from_bytes([0x99; 32]);
    assert!(
        build_withdrawal_tx(&p, 50, &spec(foreign, NET, fee_utxos())).is_err(),
        "a foreign destination cannot hold the locked payout"
    );
    for dust in [0, FEE] {
        assert!(
            build_withdrawal_tx(&p, 50, &spec(own(), dust, fee_utxos())).is_err(),
            "withdrawn amount {dust} does not cover the fee"
        );
    }
}

// REQ-UNBOND-009 — Decision: a failure means the below-gate path signs a withdrawal with no fee input (today's CLI refuses it).
#[test]
fn inc_i_257_m1_req_unbond_009_below_gate_needs_a_fee_utxo() {
    let p = params(GATE);
    assert!(
        build_withdrawal_tx(&p, GATE - 2, &spec(own(), NET, Vec::new())).is_err(),
        "below the gate the fee comes from a wallet UTXO, so none offered is an error"
    );
}

// REQ-UNBOND-009 — Decision: a failure means wallet send spends a locked Unbonding output (a refused tx) or never spends an unlocked one (stranded funds).
#[test]
fn inc_i_257_m1_req_unbond_009_send_selects_unbonding_once_spendable() {
    for (ty, spendable, want) in [
        ("unbonding", true, true),
        ("unbonding", false, false),
        ("normal", true, true),
        ("normal", false, false),
        ("bond", true, false),
        ("vesting", true, false),
    ] {
        assert_eq!(
            send_selectable(ty, spendable),
            want,
            "{ty} spendable={spendable}"
        );
    }
}

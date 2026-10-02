//! INC-I-233 (F4) — per-tx coinbase credit shared by the block validator and builder.
//
// OUTPUT CONTRACT: fn tx_coinbase_credit(tx: &Transaction, exempt_gate_active: bool) -> u64
//   (doli_core::validation::coinbase_credit; callers: validator
//    bins/node/src/node/validation_checks/mod.rs validate_block_economics, builder
//    bins/node/src/node/production/assembly.rs; exempt list shared with
//    crates/core/src/validation/utxo.rs via is_native_fee_exempt; gate from
//    NetworkParams::inc_i_233_activation_height in network_params/mod.rs,
//    network_params/defaults.rs, network_params/env_loader.rs)
//   O1 return value  per-tx credit in sats
//   O2 return value  is_native_fee_exempt(tx_type) — the ONE exempt list (L1 fee skip)
// PATHS:
//   P-LEGACY     gate inactive (height < AH): Σ floor(len*FEE_PER_BYTE/FEE_DIVISOR) per output, any type
//   P-EXEMPT     gate active, tx type in the six fee-exempt types -> 0
//   P-FEE-PAYING gate active, any other type -> legacy formula (backed by the L1 fee floor)
// INPUT PARTITIONS: no outputs | outputs without extra_data | padded outputs (0, 99, 100, 250, 524_288 B)
// MATRIX:
//   O1 x P-LEGACY     x padded, all 6 exempt types + Transfer -> legacy_formula_below_gate
//   O1 x P-EXEMPT     x padded, all 6 exempt types           -> exempt_types_credit_zero_at_gate
//   O1 x P-FEE-PAYING x padded / unpadded / empty            -> fee_paying_unchanged_at_gate
//   O2 x all TxTypes of interest                             -> exempt_list_is_exactly_six

use crypto::Hash;
use doli_core::transaction::{Output, Transaction, TxType};
use doli_core::validation::coinbase_credit::{is_native_fee_exempt, tx_coinbase_credit};

const EXEMPT: [TxType; 6] = [
    TxType::CreatePool,
    TxType::Swap,
    TxType::AddLiquidity,
    TxType::RemoveLiquidity,
    TxType::MintAsset,
    TxType::BurnAsset,
];

fn out(len: usize) -> Output {
    let mut o = Output::normal(1, Hash::from_bytes([7; 32]));
    o.extra_data = vec![0xAB; len];
    o
}

fn tx(tx_type: TxType, lens: &[usize]) -> Transaction {
    Transaction {
        version: 1,
        tx_type,
        inputs: vec![],
        outputs: lens.iter().map(|&l| out(l)).collect(),
        extra_data: vec![],
    }
}

/// The pre-fix formula, verbatim from both call sites.
fn legacy(t: &Transaction) -> u64 {
    t.outputs
        .iter()
        .map(|o| {
            o.extra_data.len() as u64 * doli_core::consensus::FEE_PER_BYTE
                / doli_core::consensus::FEE_DIVISOR
        })
        .sum()
}

const LENS: [usize; 5] = [0, 99, 100, 250, 524_288];

#[test]
fn legacy_formula_below_gate() {
    for ty in EXEMPT.iter().copied().chain([TxType::Transfer]) {
        let t = tx(ty, &LENS);
        assert_eq!(legacy(&t), 1 + 2 + 5242, "fixture sanity");
        assert_eq!(
            tx_coinbase_credit(&t, false),
            legacy(&t),
            "{ty:?}: below the gate the credit must be byte-identical to the old formula"
        );
    }
}

#[test]
fn exempt_types_credit_zero_at_gate() {
    for ty in EXEMPT {
        let t = tx(ty, &LENS);
        assert_eq!(
            tx_coinbase_credit(&t, true),
            0,
            "{ty:?} destroys no native fee, so it must not credit the coinbase"
        );
    }
}

#[test]
fn fee_paying_unchanged_at_gate() {
    for ty in [
        TxType::Transfer,
        TxType::Registration,
        TxType::AddBond,
        TxType::RequestWithdrawal,
    ] {
        for lens in [&[][..], &[0, 0][..], &LENS[..]] {
            let t = tx(ty, lens);
            assert_eq!(tx_coinbase_credit(&t, true), legacy(&t), "{ty:?} {lens:?}");
        }
    }
}

#[test]
fn exempt_list_is_exactly_six() {
    for ty in EXEMPT {
        assert!(is_native_fee_exempt(ty), "{ty:?}");
    }
    for ty in [
        TxType::Transfer,
        TxType::Registration,
        TxType::AddBond,
        TxType::RequestWithdrawal,
        TxType::Exit,
        TxType::EpochReward,
        TxType::RotateBlsKey,
    ] {
        assert!(!is_native_fee_exempt(ty), "{ty:?}");
    }
}

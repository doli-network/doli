//! INC-I-233 side finding — getTransaction reported a confirmed Swap's fee as
//! the DOLI sent into the pool reserve (live testnet: 9,999,179 vs 2 pending).
//
// OUTPUT CONTRACT: fn doli_surplus(consumed: &[Output], outputs: &[Output]) -> Option<u64>
//   (doli_core::validation::amm, next to the consensus doli_value rule;
//    caller crates/rpc/src/methods/transaction.rs get_transaction, confirmed path)
//   O1 return value  Some(native DOLI destroyed) | None when outputs exceed inputs
// PATHS:
//   P-NATIVE   Transfer, native in/out                     -> in - out
//   P-SWAP     Swap DOLI->token: pool reserve_a grows      -> only the fee, not the reserve delta
//   P-CREATE   CreatePool: native moves into reserve_a     -> 0 fee
//   P-TOKEN    token-only Transfer                         -> 0 (token units are not DOLI)
//   P-DEFICIT  outputs > inputs (e.g. an unresolved input) -> None
// INPUT PARTITIONS:
//   IP1 only native outputs on both sides            (P-NATIVE)
//   IP2 Pool on both sides, reserve_a delta > 0       (P-SWAP)
//   IP3 native in, Pool out (reserve_a = native in)   (P-CREATE)
//   IP4 only FungibleAsset outputs on both sides      (P-TOKEN)
//   IP5 Σ doli_value(out) > Σ doli_value(in)          (P-DEFICIT)
// MATRIX: IP1..IP5 -> one test each below.

use crypto::Hash;
use doli_core::conditions::Condition;
use doli_core::transaction::Output;
use doli_core::validation::amm::doli_surplus;

fn pkh() -> Hash {
    Hash::from_bytes([7; 32])
}

fn pool(reserve_a: u64, reserve_b: u64) -> Output {
    Output::pool(
        Hash::from_bytes([1; 32]),
        Hash::from_bytes([2; 32]),
        reserve_a,
        reserve_b,
        1_000,
        0,
        0,
        30,
        0,
    )
}

fn token(amount: u64) -> Output {
    Output::fungible_asset(
        amount,
        pkh(),
        Hash::from_bytes([2; 32]),
        1_000_000,
        "TKN",
        &Condition::Signature(pkh()),
    )
    .expect("fa")
}

#[test]
fn native_transfer_fee_is_in_minus_out() {
    let consumed = [Output::normal(1_000, pkh())];
    let outputs = [Output::normal(990, pkh())];
    assert_eq!(doli_surplus(&consumed, &outputs), Some(10));
}

#[test]
fn swap_fee_excludes_the_reserve_delta() {
    // 10,000,000 DOLI swapped in; pool reserve_a grows by exactly that; 2 sats fee.
    let consumed = [pool(50_000_000, 500_000), Output::normal(20_000_002, pkh())];
    let outputs = [
        pool(60_000_000, 420_000),
        token(80_000),
        Output::normal(10_000_000, pkh()),
    ];
    assert_eq!(doli_surplus(&consumed, &outputs), Some(2));
}

#[test]
fn create_pool_moves_doli_into_reserve_without_fee() {
    let consumed = [Output::normal(5_000_000, pkh()), token(500_000)];
    let outputs = [pool(5_000_000, 500_000)];
    assert_eq!(doli_surplus(&consumed, &outputs), Some(0));
}

#[test]
fn token_only_transfer_has_zero_doli_fee() {
    let consumed = [token(823)];
    let outputs = [token(823)];
    assert_eq!(doli_surplus(&consumed, &outputs), Some(0));
}

#[test]
fn deficit_is_none() {
    let consumed = [Output::normal(10, pkh())];
    let outputs = [Output::normal(11, pkh())];
    assert_eq!(doli_surplus(&consumed, &outputs), None);
}

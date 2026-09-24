//! INC-I-234 M2 — pure output-creation authority predicate.
// covers: crates/core/src/validation/output_authority.rs
//! TDD RED: does not compile until `check_output_placement` / `check_value_authority` exist.

// OUTPUT CONTRACT: fn check_output_placement(tx: &Transaction, ctx: &ValidationContext) -> Result<(), ValidationError>
//   O1 return Ok(()) | Err carrying [ERRTX-AUTH001] (pool placement) or [ERRTX-AUTH005] (coinbase non-Normal)
//   O2 mutable params / receiver: NONE (&tx, &ctx)
//   O3 persistent store writes: NONE (pure)
//   PATHS: P-BelowAH (early Ok) | P-Coinbase | P-PoolOutputs (count/index/tx_type) | P-NoPool (Ok)
// OUTPUT CONTRACT: fn check_value_authority(tx: &Transaction, consumed_outputs: &[Output], ctx: &ValidationContext) -> Result<(), ValidationError>
//   O1 return Ok(()) | Err carrying [ERRTX-AUTH002] (pool input) / [ERRTX-AUTH003] (conservation) / [ERRTX-AUTH004] (issuance)
//   O2 mutable params / receiver: NONE
//   O3 persistent store writes: NONE (pure)
//   PATHS: P-BelowAH | P-PoolInputs | P-KeyConserved | P-KeyExemptAMM | P-KeyIssuance
// MATRIX (outputs x paths):
//   placement O1: BelowAH->Ok (parity_*) | Coinbase->AUTH005/Ok (w7_*, honest_coinbase) | PoolOutputs->AUTH001/Ok (w1,w6,registration,legit_*) | NoPool->Ok
//   authority O1: BelowAH->Ok (parity_*) | PoolInputs->AUTH002 (w8,transfer_consumes_pool,swap_without_pool_input)
//                 KeyConserved->AUTH003/Ok (w3,w4,cross_key,u128,metadata_key) | Exempt->Ok (legit_*) | Issuance->AUTH004/Ok (issuance_*)
//   O2/O3: NONE for both (signatures take shared refs only)
// INPUT PARTITIONS: height {AH-1, AH, default u64::MAX gate}; tx_type {Transfer, coinbase, Registration, BurnAsset,
//   CreatePool, Swap, AddLiquidity, RemoveLiquidity}; FA/LP amounts {small, u64::MAX}; issuance {0 inputs, 1, 2}

use crypto::Hash;
use doli_core::conditions::Condition;
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{Input, Output, Transaction, TxType};
use doli_core::validation::{check_output_placement, check_value_authority, ValidationContext};
use doli_core::Network;

const AH: u64 = 100;
const FEE_BPS: u16 = 30;

fn h(n: u8) -> Hash {
    Hash::from_bytes([n; 32])
}
fn ctx_at(height: u64) -> ValidationContext {
    ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, height)
        .with_inc_i_234_activation_height(AH)
}
fn active() -> ValidationContext {
    ctx_at(AH)
}
fn inp(n: u8) -> Input {
    Input::new(h(n), 0)
}
fn owner() -> Hash {
    h(0xAA)
}
fn asset_x() -> Hash {
    h(0x11)
}
fn asset_y() -> Hash {
    h(0x22)
}
fn pool_p() -> Hash {
    Output::compute_pool_id(&Hash::ZERO, &asset_x(), FEE_BPS)
}
fn pool_q() -> Hash {
    Output::compute_pool_id(&Hash::ZERO, &asset_y(), FEE_BPS)
}
fn pool_out(pool_id: Hash, asset_b: Hash, ra: u64, rb: u64, lp: u64) -> Output {
    Output::pool(pool_id, asset_b, ra, rb, lp, 0, 0, FEE_BPS, 0)
}
fn pool_p_out(ra: u64, rb: u64, lp: u64) -> Output {
    pool_out(pool_p(), asset_x(), ra, rb, lp)
}
fn fa(amount: u64, id: Hash, supply: u64) -> Output {
    fa_t(amount, id, supply, "TOK")
}
fn fa_t(amount: u64, id: Hash, supply: u64, ticker: &str) -> Output {
    Output::fungible_asset(
        amount,
        owner(),
        id,
        supply,
        ticker,
        &Condition::Signature(owner()),
    )
    .unwrap()
}
fn lp(amount: u64, pool_id: Hash) -> Output {
    Output::lp_share(amount, pool_id, owner())
}
fn native(amount: u64) -> Output {
    Output::normal(amount, owner())
}
fn tx(tx_type: TxType, inputs: Vec<Input>, outputs: Vec<Output>) -> Transaction {
    Transaction {
        version: 1,
        tx_type,
        inputs,
        outputs,
        extra_data: Vec::new(),
    }
}
fn inputs(n: usize) -> Vec<Input> {
    (0..n).map(|i| inp(i as u8 + 1)).collect()
}

fn assert_code<T: std::fmt::Debug, E: std::fmt::Display>(r: Result<T, E>, codes: &[&str]) {
    match r {
        Ok(v) => panic!("expected Err with one of {codes:?}, got Ok({v:?})"),
        Err(e) => {
            let s = e.to_string();
            assert!(
                codes.iter().any(|c| s.contains(c)),
                "error {s:?} lacks one of {codes:?}"
            );
        }
    }
}
const AUTH001: &str = "[ERRTX-AUTH001]";
const AUTH002: &str = "[ERRTX-AUTH002]";
const AUTH003: &str = "[ERRTX-AUTH003]";
const AUTH004: &str = "[ERRTX-AUTH004]";
const AUTH005: &str = "[ERRTX-AUTH005]";

fn both_ok(t: &Transaction, consumed: &[Output], c: &ValidationContext) {
    check_output_placement(t, c).expect("placement must be Ok");
    check_value_authority(t, consumed, c).expect("authority must be Ok");
}

// ---- attack fixtures (W1-W8) ------------------------------------------------------------------

fn w1_transfer_pool_out() -> (Transaction, Vec<Output>) {
    let t = tx(
        TxType::Transfer,
        inputs(1),
        vec![pool_p_out(1_000_000_000_000, 1, 1_000), native(1)],
    );
    (t, vec![native(10)])
}
fn w2_transfer_mints_existing_fa() -> (Transaction, Vec<Output>) {
    let t = tx(
        TxType::Transfer,
        inputs(1),
        vec![
            fa(1_000_000_000_000, asset_x(), 1_000_000_000_000),
            native(1),
        ],
    );
    (t, vec![native(10)])
}
fn w3_transfer_mints_lp() -> (Transaction, Vec<Output>) {
    (
        tx(
            TxType::Transfer,
            inputs(1),
            vec![lp(1_000_000_000, pool_p()), native(1)],
        ),
        vec![native(10)],
    )
}
fn w4_burn_inflates() -> (Transaction, Vec<Output>) {
    (
        tx(
            TxType::BurnAsset,
            inputs(1),
            vec![fa(1_000_000, asset_x(), 1_000)],
        ),
        vec![fa(1, asset_x(), 1_000)],
    )
}
fn w5_create_pool_foreign_fa_change() -> (Transaction, Vec<Output>) {
    let outs = vec![
        pool_p_out(1_000, 1_000, 1_000),
        lp(1_000, pool_p()),
        native(10),
        fa(1_000_000_000, asset_y(), 1_000_000_000),
    ];
    (
        tx(TxType::CreatePool, inputs(2), outs),
        vec![native(1_020), fa(1_000, asset_x(), 10_000)],
    )
}
fn w6_swap_second_pool_out() -> (Transaction, Vec<Output>) {
    let outs = vec![
        pool_p_out(1_100, 910, 1_000),
        pool_out(pool_q(), asset_y(), 1, 1_000_000, 1),
    ];
    (
        tx(TxType::Swap, inputs(2), outs),
        vec![pool_p_out(1_000, 1_000, 1_000), native(100)],
    )
}
fn w7_coinbase(out: Output) -> Transaction {
    let mut t = Transaction::new_coinbase(1_000, owner(), AH, 7);
    t.outputs = vec![out];
    t
}
fn w8_swap_pool_at_input1() -> (Transaction, Vec<Output>) {
    let outs = vec![pool_p_out(1_100, 910, 1_000), fa(90, asset_x(), 0)];
    let phantom = pool_out(pool_q(), asset_x(), 1_000_000_000, 1, 1);
    (
        tx(TxType::Swap, inputs(2), outs),
        vec![pool_p_out(1_000, 1_000, 1_000), phantom],
    )
}

// ---- REQ-234-SEC-004 pool placement ----------------------------------------------------------

// REQ-234-SEC-004 — Decision: a Transfer can still mint a Pool (INC-I-233 W1 drain stays open)
#[test]
fn w1_transfer_with_pool_output_is_rejected() {
    let (t, _) = w1_transfer_pool_out();
    assert_code(check_output_placement(&t, &active()), &[AUTH001]);
}

// REQ-234-SEC-004 — Decision: an AMM tx can smuggle a second Pool whose reserve_b is unbacked (W6)
#[test]
fn w6_swap_with_second_pool_at_output1_is_rejected() {
    let (t, _) = w6_swap_second_pool_out();
    assert_code(check_output_placement(&t, &active()), &[AUTH001]);
}

// REQ-234-SEC-004 — Decision: Registration still mints Pools because its validator checks only the Bond
#[test]
fn registration_with_pool_output_is_rejected() {
    let t = tx(
        TxType::Registration,
        inputs(1),
        vec![native(100), pool_p_out(1_000, 1, 1)],
    );
    assert_code(check_output_placement(&t, &active()), &[AUTH001]);
}

// REQ-234-SEC-004 — Decision: the Pool index is not enforced, so AMM math reads the wrong output
#[test]
fn swap_with_pool_not_at_output0_is_rejected() {
    let t = tx(
        TxType::Swap,
        inputs(2),
        vec![fa(90, asset_x(), 0), pool_p_out(1_100, 910, 1_000)],
    );
    assert_code(check_output_placement(&t, &active()), &[AUTH001]);
}

// REQ-234-SEC-004 — Decision: the "exactly one Pool" rule is missing for CreatePool
#[test]
fn create_pool_with_two_pool_outputs_is_rejected() {
    let outs = vec![
        pool_p_out(1_000, 1_000, 1_000),
        pool_p_out(1_000, 1_000, 1_000),
        lp(1_000, pool_p()),
    ];
    let t = tx(TxType::CreatePool, inputs(2), outs);
    assert_code(check_output_placement(&t, &active()), &[AUTH001]);
}

// ---- REQ-234-SEC-006 coinbase ----------------------------------------------------------------

// REQ-234-SEC-006 — Decision: a producer can still emit a Pool from the coinbase (W7)
#[test]
fn w7_coinbase_pool_output_is_rejected() {
    let t = w7_coinbase(pool_p_out(1_000_000_000, 1, 1));
    assert_code(check_output_placement(&t, &active()), &[AUTH005, AUTH001]);
}

// REQ-234-SEC-006 — Decision: a producer can still print FA from the coinbase (W7)
#[test]
fn w7_coinbase_fa_output_is_rejected() {
    let t = w7_coinbase(fa(1_000_000, asset_x(), 1_000_000));
    assert_code(check_output_placement(&t, &active()), &[AUTH005]);
}

// REQ-234-SEC-006 — Decision: a producer can still print LP shares from the coinbase (W7)
#[test]
fn w7_coinbase_lp_output_is_rejected() {
    let t = w7_coinbase(lp(1_000_000, pool_p()));
    assert_code(check_output_placement(&t, &active()), &[AUTH005]);
}

// REQ-234-SEC-006 — Decision: the coinbase rule rejects honest blocks and halts production
#[test]
fn honest_coinbase_is_ok() {
    let t = Transaction::new_coinbase(1_000, owner(), AH, 7);
    assert!(t.is_coinbase());
    both_ok(&t, &[], &active());
}

// ---- REQ-234-SEC-005 pool consumption --------------------------------------------------------

// REQ-234-SEC-005 — Decision: a phantom Pool at input[1] still feeds reserve_a into a Swap (W8)
#[test]
fn w8_swap_with_pool_at_input1_is_rejected() {
    let (t, c) = w8_swap_pool_at_input1();
    assert_code(check_value_authority(&t, &c, &active()), &[AUTH002]);
}

// REQ-234-SEC-005 — Decision: a non-AMM tx can still consume (and dissolve) a Pool UTXO
#[test]
fn transfer_consuming_pool_is_rejected() {
    let t = tx(TxType::Transfer, inputs(1), vec![native(1)]);
    assert_code(
        check_value_authority(&t, &[pool_p_out(1_000, 1_000, 1_000)], &active()),
        &[AUTH002],
    );
}

// REQ-234-SEC-005 — Decision: a Swap with no Pool at input[0] is accepted and binds nothing
#[test]
fn swap_without_pool_input_is_rejected() {
    let t = tx(
        TxType::Swap,
        inputs(1),
        vec![pool_p_out(1_100, 910, 1_000), fa(90, asset_x(), 0)],
    );
    assert_code(
        check_value_authority(&t, &[native(100)], &active()),
        &[AUTH002],
    );
}

// ---- REQ-234-SEC-007 per-key conservation ----------------------------------------------------

// REQ-234-SEC-007 — Decision: a Transfer still mints FA of a live pool's asset_b with no FA input (W2)
#[test]
fn w2_transfer_minting_existing_fa_without_input_is_rejected() {
    let (t, c) = w2_transfer_mints_existing_fa();
    assert_code(
        check_value_authority(&t, &c, &active()),
        &[AUTH003, AUTH004],
    );
}

// REQ-234-SEC-007 — Decision: counterfeit LP(P) via Transfer can still drain pool P through RemoveLiquidity (W3)
#[test]
fn w3_transfer_minting_lp_is_rejected() {
    let (t, c) = w3_transfer_mints_lp();
    assert_code(check_value_authority(&t, &c, &active()), &[AUTH003]);
}

// REQ-234-SEC-007 — Decision: AMM txs can still emit LP of a foreign pool (E3 extractor sees 0)
#[test]
fn w3_amm_txs_emitting_foreign_pool_lp_are_rejected() {
    let cases = [
        (
            TxType::Swap,
            vec![
                pool_p_out(1_100, 910, 1_000),
                fa(90, asset_x(), 0),
                lp(1_000_000, pool_q()),
            ],
            vec![pool_p_out(1_000, 1_000, 1_000), native(100)],
        ),
        (
            TxType::AddLiquidity,
            vec![
                pool_p_out(1_100, 1_100, 1_100),
                lp(100, pool_p()),
                lp(1_000_000, pool_q()),
            ],
            vec![
                pool_p_out(1_000, 1_000, 1_000),
                native(100),
                fa(100, asset_x(), 10_000),
            ],
        ),
        (
            TxType::RemoveLiquidity,
            vec![
                pool_p_out(900, 900, 900),
                native(100),
                fa(100, asset_x(), 0),
                lp(1_000_000, pool_q()),
            ],
            vec![pool_p_out(1_000, 1_000, 1_000), lp(100, pool_p())],
        ),
    ];
    for (ty, outs, consumed) in cases {
        let t = tx(ty, inputs(consumed.len()), outs);
        assert_code(check_value_authority(&t, &consumed, &active()), &[AUTH003]);
    }
}

// REQ-234-SEC-007 — Decision: BurnAsset still inflates any asset the sender holds 1 unit of (W4)
#[test]
fn w4_burn_asset_output_exceeding_input_is_rejected() {
    let (t, c) = w4_burn_inflates();
    assert_code(check_value_authority(&t, &c, &active()), &[AUTH003]);
}

// REQ-234-SEC-007 — Decision: CreatePool change can still counterfeit a non-asset_b FA (W5)
#[test]
fn w5_create_pool_foreign_fa_change_is_rejected() {
    let (t, c) = w5_create_pool_foreign_fa_change();
    assert_code(
        check_value_authority(&t, &c, &active()),
        &[AUTH003, AUTH004],
    );
}

// REQ-234-SEC-007 — Decision: FA of asset X can still be relabelled as asset Y (key mix-up)
#[test]
fn fa_input_of_one_asset_cannot_fund_another() {
    let t = tx(TxType::Transfer, inputs(1), vec![fa(100, asset_y(), 1_000)]);
    assert_code(
        check_value_authority(&t, &[fa(100, asset_x(), 1_000)], &active()),
        &[AUTH003, AUTH004],
    );
}

// REQ-234-SEC-007 — Decision: a one-unit FA overspend slips through (off-by-one in the bound)
#[test]
fn fa_transfer_one_over_input_is_rejected() {
    let t = tx(
        TxType::Transfer,
        inputs(1),
        vec![fa(60, asset_x(), 1_000), fa(41, asset_x(), 1_000)],
    );
    assert_code(
        check_value_authority(&t, &[fa(100, asset_x(), 1_000)], &active()),
        &[AUTH003],
    );
}

// REQ-234-SEC-007 — Decision: sums are u64 and overflow (panic or wraparound) at u64::MAX amounts
#[test]
fn conservation_sums_in_u128_without_overflow() {
    let m = u64::MAX;
    let t = tx(
        TxType::Transfer,
        inputs(2),
        vec![fa(m, asset_x(), m), fa(m, asset_x(), m)],
    );
    let consumed = [fa(m, asset_x(), m), fa(m, asset_x(), m)];
    check_value_authority(&t, &consumed, &active()).expect("equal u64::MAX sums must conserve");
    let t3 = tx(
        TxType::Transfer,
        inputs(2),
        vec![
            fa(m, asset_x(), m),
            fa(m, asset_x(), m),
            fa(1, asset_x(), m),
        ],
    );
    assert_code(check_value_authority(&t3, &consumed, &active()), &[AUTH003]);
}

// REQ-234-SEC-007 — Decision: the key includes FA metadata, so CLI swap outputs (ts=0, "XCHG") break
#[test]
fn fa_key_is_asset_id_only_ignoring_metadata() {
    let t = tx(
        TxType::Transfer,
        inputs(1),
        vec![fa_t(100, asset_x(), 0, "XCHG")],
    );
    both_ok(&t, &[fa_t(100, asset_x(), 1_000, "ABC")], &active());
}

// ---- REQ-234-SEC-008 issuance ----------------------------------------------------------------

fn issued_id() -> Hash {
    Output::compute_asset_id(&h(1), 0)
}

// REQ-234-SEC-008 — Decision: the new issuance path rejects honest input-anchored token issues
#[test]
fn issuance_with_correct_derivation_is_ok() {
    let t = tx(
        TxType::Transfer,
        inputs(1),
        vec![fa(1_000, issued_id(), 1_000), native(5)],
    );
    both_ok(&t, &[native(10)], &active());
    let split = tx(
        TxType::Transfer,
        inputs(1),
        vec![fa(600, issued_id(), 1_000), fa(400, issued_id(), 1_000)],
    );
    both_ok(&split, &[native(10)], &active());
}

// REQ-234-SEC-008 — Decision: the legacy placeholder asset_id still issues (unauthenticated mint)
#[test]
fn issuance_with_placeholder_id_is_rejected() {
    let t = tx(TxType::Transfer, inputs(1), vec![fa(1_000, h(0x77), 1_000)]);
    assert_code(
        check_value_authority(&t, &[native(10)], &active()),
        &[AUTH004],
    );
}

// REQ-234-SEC-008 — Decision: issuance can exceed its declared total_supply
#[test]
fn issuance_exceeding_total_supply_is_rejected() {
    let t = tx(
        TxType::Transfer,
        inputs(1),
        vec![fa(600, issued_id(), 1_000), fa(401, issued_id(), 1_000)],
    );
    assert_code(
        check_value_authority(&t, &[native(10)], &active()),
        &[AUTH004],
    );
}

// REQ-234-SEC-008 — Decision: one tx can issue two assets (one anchored on a non-first input)
#[test]
fn issuance_of_two_fresh_ids_is_rejected() {
    let second = Output::compute_asset_id(&h(2), 0);
    let t = tx(
        TxType::Transfer,
        inputs(2),
        vec![fa(10, issued_id(), 10), fa(10, second, 10)],
    );
    assert_code(
        check_value_authority(&t, &[native(10), native(10)], &active()),
        &[AUTH004],
    );
}

// REQ-234-SEC-008 — Decision: mixed total_supply lets the issuer pick the loosest bound
#[test]
fn issuance_with_non_uniform_total_supply_is_rejected() {
    let t = tx(
        TxType::Transfer,
        inputs(1),
        vec![fa(500, issued_id(), 1_000), fa(500, issued_id(), 2_000)],
    );
    assert_code(
        check_value_authority(&t, &[native(10)], &active()),
        &[AUTH004],
    );
}

// REQ-234-SEC-008 — Decision: a non-Transfer type can still issue a fresh asset
#[test]
fn fresh_id_in_non_transfer_is_rejected() {
    let t = tx(
        TxType::BurnAsset,
        inputs(1),
        vec![fa(1_000, issued_id(), 1_000)],
    );
    assert_code(
        check_value_authority(&t, &[native(10)], &active()),
        &[AUTH003, AUTH004],
    );
}

// REQ-234-SEC-008 — Decision: an input-less issuance indexes inputs[0] and panics the validator
#[test]
fn issuance_with_no_inputs_errs_without_panic() {
    let t = tx(
        TxType::Transfer,
        vec![],
        vec![fa(10, issued_id(), 10), fa(10, issued_id(), 10)],
    );
    assert_code(check_value_authority(&t, &[], &active()), &[AUTH004]);
}

// ---- REQ-234-011 legitimate flows ------------------------------------------------------------

// REQ-234-011 — Decision: CreatePool (Pool[0], LP[1], asset_b change) is broken by the new rule
#[test]
fn legit_create_pool_is_ok() {
    let outs = vec![
        pool_p_out(1_000, 800, 894),
        lp(894, pool_p()),
        native(10),
        fa(200, asset_x(), 10_000),
    ];
    let t = tx(TxType::CreatePool, inputs(2), outs);
    both_ok(
        &t,
        &[native(1_020), fa(1_000, asset_x(), 10_000)],
        &active(),
    );
}

// REQ-234-011 — Decision: Swap A->B (asset_b out, no FA input) is broken by the new rule
#[test]
fn legit_swap_a_to_b_is_ok() {
    let t = tx(
        TxType::Swap,
        inputs(2),
        vec![
            pool_p_out(1_100, 910, 1_000),
            fa_t(90, asset_x(), 0, "XCHG"),
            native(5),
        ],
    );
    both_ok(
        &t,
        &[pool_p_out(1_000, 1_000, 1_000), native(110)],
        &active(),
    );
}

// REQ-234-011 — Decision: Swap B->A (FA in, native out) is broken by the new rule
#[test]
fn legit_swap_b_to_a_is_ok() {
    let t = tx(
        TxType::Swap,
        inputs(2),
        vec![pool_p_out(910, 1_100, 1_000), native(89)],
    );
    both_ok(
        &t,
        &[pool_p_out(1_000, 1_000, 1_000), fa(100, asset_x(), 10_000)],
        &active(),
    );
}

// REQ-234-011 — Decision: AddLiquidity (fresh own-pool LP) is broken by the new rule
#[test]
fn legit_add_liquidity_is_ok() {
    let t = tx(
        TxType::AddLiquidity,
        inputs(3),
        vec![
            pool_p_out(1_100, 1_100, 1_100),
            lp(100, pool_p()),
            native(5),
        ],
    );
    both_ok(
        &t,
        &[
            pool_p_out(1_000, 1_000, 1_000),
            native(105),
            fa(100, asset_x(), 10_000),
        ],
        &active(),
    );
}

// REQ-234-011 — Decision: RemoveLiquidity with own-pool LP change is broken by the new rule
#[test]
fn legit_remove_liquidity_with_lp_change_is_ok() {
    let outs = vec![
        pool_p_out(900, 900, 900),
        native(100),
        fa(100, asset_x(), 0),
        lp(40, pool_p()),
    ];
    let t = tx(TxType::RemoveLiquidity, inputs(2), outs);
    both_ok(
        &t,
        &[pool_p_out(1_000, 1_000, 1_000), lp(140, pool_p())],
        &active(),
    );
}

// REQ-234-011 — Decision: plain FA transfer with change is broken by the new rule
#[test]
fn legit_fa_transfer_with_change_is_ok() {
    let t = tx(
        TxType::Transfer,
        inputs(2),
        vec![
            fa(60, asset_x(), 1_000),
            fa(40, asset_x(), 1_000),
            native(5),
        ],
    );
    both_ok(&t, &[fa(100, asset_x(), 1_000), native(10)], &active());
}

// REQ-234-011 — Decision: LP transfer between owners is broken by the new rule
#[test]
fn legit_lp_transfer_is_ok() {
    let to_other = Output::lp_share(70, pool_p(), h(0xBB));
    let t = tx(
        TxType::Transfer,
        inputs(2),
        vec![to_other, lp(30, pool_p())],
    );
    both_ok(&t, &[lp(100, pool_p()), native(10)], &active());
}

// REQ-234-011 — Decision: an honest BurnAsset within balance is broken by the new rule
#[test]
fn legit_burn_asset_within_balance_is_ok() {
    let t = tx(TxType::BurnAsset, inputs(1), vec![fa(40, asset_x(), 1_000)]);
    both_ok(&t, &[fa(100, asset_x(), 1_000)], &active());
}

// REQ-234-011 — Decision: native-only transfers change behaviour under the new rule
#[test]
fn legit_plain_native_transfer_is_ok() {
    let t = Transaction::new_transfer(inputs(2), vec![native(10), native(5)]);
    both_ok(&t, &[native(10), native(10)], &active());
}

// ---- REQ-234-010 below-AH parity -------------------------------------------------------------

fn all_w_fixtures() -> Vec<(&'static str, Transaction, Vec<Output>)> {
    let (w1, c1) = w1_transfer_pool_out();
    let (w2, c2) = w2_transfer_mints_existing_fa();
    let (w3, c3) = w3_transfer_mints_lp();
    let (w4, c4) = w4_burn_inflates();
    let (w5, c5) = w5_create_pool_foreign_fa_change();
    let (w6, c6) = w6_swap_second_pool_out();
    let (w8, c8) = w8_swap_pool_at_input1();
    vec![
        ("W1", w1, c1),
        ("W2", w2, c2),
        ("W3", w3, c3),
        ("W4", w4, c4),
        ("W5", w5, c5),
        ("W6", w6, c6),
        ("W7-pool", w7_coinbase(pool_p_out(1_000_000, 1, 1)), vec![]),
        ("W7-fa", w7_coinbase(fa(1_000, asset_x(), 1_000)), vec![]),
        ("W7-lp", w7_coinbase(lp(1_000, pool_p())), vec![]),
        ("W8", w8, c8),
    ]
}

// REQ-234-010 — Decision: the predicate fires below the AH and forks replay of historical blocks
#[test]
fn parity_every_w_fixture_is_ok_below_ah() {
    let below = ctx_at(AH - 1);
    for (name, t, c) in all_w_fixtures() {
        assert!(
            check_output_placement(&t, &below).is_ok(),
            "{name}: placement must be Ok at AH-1"
        );
        assert!(
            check_value_authority(&t, &c, &below).is_ok(),
            "{name}: authority must be Ok at AH-1"
        );
    }
}

// REQ-234-010 — Decision: the fixtures are vacuous (they would pass at the AH too), so parity proves nothing
#[test]
fn parity_every_w_fixture_errs_at_ah() {
    let at = active();
    for (name, t, c) in all_w_fixtures() {
        let rejected =
            check_output_placement(&t, &at).is_err() || check_value_authority(&t, &c, &at).is_err();
        assert!(rejected, "{name}: must be rejected at h=AH");
    }
}

// REQ-234-010 — Decision: the default ctx (gate u64::MAX) activates the rule on a network with no pin
#[test]
fn default_gate_is_inactive() {
    let c = ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, 10_000_000);
    let (t, cons) = w1_transfer_pool_out();
    assert!(check_output_placement(&t, &c).is_ok());
    assert!(check_value_authority(&t, &cons, &c).is_ok());
}

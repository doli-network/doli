//! INC-I-234 — activation-boundary parity between tip-height admission, builder check and block validation.
// covers: crates/core/src/validation/output_authority.rs
// OUTPUT CONTRACT: validate_transaction(tx, ctx) / validate_transaction_with_utxos(tx, ctx, utxos)
//   O1 return Ok(()) | Err (the only observable output; &tx, &ctx, &utxos are shared refs, no store writes)
// PATHS × HEIGHTS:
//   P-ADMIT  (mempool, ctx at tip = AH-1): stateless + stateful accept (pre-fix verdict)
//   P-BLOCK  (block validation at h = AH): stateless OR stateful rejects
//   P-BUILD  (builder at h = AH: validate_transaction_with_utxos ONLY): must reject too
// INPUT PARTITIONS: W1 Transfer+Pool, W2 Transfer minting non-anchored FA, W3 Transfer minting LP (reject at AH);
//   native Transfer, anchored issuance (accept at both heights).

use std::collections::HashMap;

use crypto::{Hash, KeyPair, Signature};
use doli_core::conditions::Condition;
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{Input, Output, SighashType, Transaction, TxType};
use doli_core::validation::{
    validate_transaction, validate_transaction_with_utxos, UtxoInfo, UtxoProvider,
    ValidationContext,
};
use doli_core::Network;

const AH: u64 = 10_000;
const FUND: u64 = 10_000_000_000;
const FEE_BPS: u16 = 30;

struct Utxos(HashMap<(Hash, u32), UtxoInfo>);

impl UtxoProvider for Utxos {
    fn get_utxo(&self, tx_hash: &Hash, index: u32) -> Option<UtxoInfo> {
        self.0.get(&(*tx_hash, index)).cloned()
    }
}

fn ctx_at(height: u64) -> ValidationContext {
    ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, height)
        .with_defi_activation_height(0)
        .with_amm_activation_height(0)
        .with_inc_i_234_activation_height(AH)
}

fn pkh(kp: &KeyPair) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes())
}

fn fund_prev() -> Hash {
    Hash::from_bytes([0x5A; 32])
}

fn fa(amount: u64, owner: Hash, id: Hash, supply: u64) -> Output {
    Output::fungible_asset(
        amount,
        owner,
        id,
        supply,
        "TOK",
        &Condition::Signature(owner),
    )
    .unwrap()
}

/// One native funding input; `extra` outputs first, native change last absorbing the fee.
fn build(kp: &KeyPair, extra: Vec<Output>) -> (Transaction, Utxos) {
    let owner = pkh(kp);
    let mut outputs = extra;
    outputs.push(Output::normal(0, owner));
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input {
            prev_tx_hash: fund_prev(),
            output_index: 0,
            signature: Signature::from_bytes([0u8; 64]),
            sighash_type: SighashType::All,
            committed_output_count: 0,
            public_key: Some(*kp.public_key()),
        }],
        outputs,
        extra_data: vec![],
    };
    let spent: u64 = tx.outputs.iter().map(|o| o.amount).sum();
    let last = tx.outputs.len() - 1;
    tx.outputs[last].amount = FUND - spent - tx.minimum_fee() - 10_000;
    let msg = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    let mut m = HashMap::new();
    m.insert(
        (fund_prev(), 0),
        UtxoInfo {
            output: Output::normal(FUND, owner),
            pubkey: None,
            spent: false,
        },
    );
    (tx, Utxos(m))
}

fn pool_output() -> Output {
    let asset = Hash::from_bytes([0x11; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset, FEE_BPS);
    Output::pool(pool_id, asset, 1_000_000, 1, 1_000, 0, 0, FEE_BPS, 0)
}

fn w1(kp: &KeyPair) -> (Transaction, Utxos) {
    build(kp, vec![pool_output()])
}
fn w2(kp: &KeyPair) -> (Transaction, Utxos) {
    let id = Hash::from_bytes([0x22; 32]);
    build(kp, vec![fa(1_000_000, pkh(kp), id, 1_000_000)])
}
fn w3(kp: &KeyPair) -> (Transaction, Utxos) {
    let asset = Hash::from_bytes([0x11; 32]);
    let pid = Output::compute_pool_id(&Hash::ZERO, &asset, FEE_BPS);
    build(kp, vec![Output::lp_share(1_000_000, pid, pkh(kp))])
}
fn legit_native(kp: &KeyPair) -> (Transaction, Utxos) {
    build(kp, vec![Output::normal(5_000, pkh(kp))])
}
fn legit_anchored(kp: &KeyPair) -> (Transaction, Utxos) {
    let id = Output::compute_asset_id(&fund_prev(), 0);
    build(kp, vec![fa(1_000, pkh(kp), id, 1_000)])
}

fn assert_boundary(name: &str, (tx, u): (Transaction, Utxos)) {
    let (below, at) = (ctx_at(AH - 1), ctx_at(AH));
    let s_below = validate_transaction(&tx, &below);
    let u_below = validate_transaction_with_utxos(&tx, &below, &u);
    assert!(
        s_below.is_ok() && u_below.is_ok(),
        "{name}: at tip AH-1 (mempool admission) must keep the pre-fix accept verdict: {s_below:?} / {u_below:?}"
    );
    let block_verdict =
        validate_transaction(&tx, &at).and(validate_transaction_with_utxos(&tx, &at, &u));
    assert!(
        block_verdict.is_err(),
        "{name}: block validation at h=AH accepted the violation"
    );
    let builder_verdict = validate_transaction_with_utxos(&tx, &at, &u);
    assert!(
        matches!(&builder_verdict, Err(e) if e.to_string().contains("[ERRTX-AUTH")),
        "{name}: builder check (with_utxos only) at h=AH ACCEPTS a tx block validation at h=AH rejects: {:?}",
        block_verdict
    );
}

fn assert_legit(name: &str, (tx, u): (Transaction, Utxos)) {
    for h in [AH - 1, AH] {
        let c = ctx_at(h);
        let s = validate_transaction(&tx, &c);
        let v = validate_transaction_with_utxos(&tx, &c, &u);
        assert!(
            s.is_ok() && v.is_ok(),
            "{name} rejected at h={h}: {s:?} / {v:?}"
        );
    }
}

// REQ-234-SEC-012 — Decision: a failure means a Transfer+Pool admitted at tip AH-1 is packed into block AH and the producer forks off its own block.
#[test]
fn boundary_w1_transfer_pool_output_builder_rejects_at_ah() {
    assert_boundary("W1", w1(&KeyPair::generate()));
}

// REQ-234-SEC-012 — Decision: a failure means a non-anchored FA mint slips from AH-1 admission into block AH.
#[test]
fn boundary_w2_transfer_mints_foreign_fa_builder_rejects_at_ah() {
    assert_boundary("W2", w2(&KeyPair::generate()));
}

// REQ-234-SEC-012 — Decision: a failure means an LP counterfeit slips from AH-1 admission into block AH.
#[test]
fn boundary_w3_transfer_mints_lp_builder_rejects_at_ah() {
    assert_boundary("W3", w3(&KeyPair::generate()));
}

// REQ-234-010 — Decision: a failure means the boundary checks reject honest native transfers (liveness loss).
#[test]
fn boundary_legit_native_transfer_accepted_both_sides() {
    assert_legit("native", legit_native(&KeyPair::generate()));
}

// REQ-234-011 — Decision: a failure means the boundary checks reject a legitimate anchored issuance.
#[test]
fn boundary_legit_anchored_issuance_accepted_both_sides() {
    assert_legit("anchored", legit_anchored(&KeyPair::generate()));
}

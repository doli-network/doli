//! INC-I-257 M2.5 — the mempool judges the INC-I-257 output rules at the
//! mined-at height (tip + 1) on admission and on revalidate.
//!
// covers: encoding parsers pool unbonding revalidate protocol security_model cli types mod
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64 /*tip*/, u32) -> Result<AddTransactionResult, MempoolError>
//                  fn Mempool::revalidate(&mut self, &UtxoSet, u64 /*tip*/, u32)
//   O1 add_transaction return (Ok | Err with code); O2 residency (receiver: contains)
//   O3 revalidate: eviction of residents (receiver: contains); no store writes, UtxoSet read-only
// PATHS: P-FLOOR payout lock floor at admission | P-GATE gate crossing at admission
//        | P-REVAL resident re-judged at the new tip | P-PARITY admission vs block validator at tip + 1
// INPUT PARTITIONS:
//   lock_until: T+59 | T+60 (floor holds at T only) | T+61 | T+62; tip: T | T+1
//   gate G (child process env): tip G-2 | G-1; payout: Normal (pre-gate shape) | Unbonding (gated shape)
// MATRIX:
//   O1,O2 x P-FLOOR -> inc_i_257_m25_req_unbond_005_admission_refuses_lock_at_tip_floor
//   O1,O2 x P-FLOOR -> inc_i_257_m25_req_unbond_005_admission_accepts_lock_at_tip_plus_one_floor
//   O1,O2 x P-GATE -> inc_i_257_m25_req_unbond_017_admission_gate_judged_at_tip_plus_one
//   O2,O3 x P-REVAL -> inc_i_257_m25_req_unbond_005_revalidate_evicts_unmineable_withdrawal
//   O1 x P-PARITY -> inc_i_257_m25_req_unbond_005_admission_parity_with_block_validator
//   O1,O3 (all) -> inc_i_257_m25_outcome_probe

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::conditions::Condition;
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::transaction::{Input, Output, OutputType, Transaction};
use doli_core::validation::{validate_transaction_with_utxos, ValidationContext};
use mempool::{Mempool, MempoolError, MempoolPolicy};
use storage::{Outpoint, UtxoEntry, UtxoSet};

const PERIOD: u64 = 60;
const FEE: u64 = 1_000;
const T: u64 = 5;
const G: u64 = 100;
const GATE_ENV: &str = "DOLI_INC_I_257_UNBONDING_ACTIVATION_HEIGHT";
const SLASH_ENV: &str = "DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_257_M25_MEMPOOL_CHILD";

fn kp() -> KeyPair {
    KeyPair::from_seed([0x57; 32])
}

fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

fn bond_op() -> Outpoint {
    Outpoint::new(Hash::from_bytes([0x25; 32]), 0)
}

fn net() -> u64 {
    Network::Devnet.bond_unit() - FEE
}

fn bond_utxo() -> UtxoSet {
    let pk = *kp().public_key();
    let mut utxo = UtxoSet::new();
    let entry = UtxoEntry {
        output: Output::bond(Network::Devnet.bond_unit(), addr(&pk), u64::MAX, 0),
        height: 0,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    utxo.insert(bond_op(), entry).expect("FIXTURE: seed Bond");
    utxo
}

fn signed(mut tx: Transaction) -> Transaction {
    let k = kp();
    for i in 0..tx.inputs.len() {
        tx.inputs[i].public_key = Some(*k.public_key());
        let msg = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&msg, k.private_key());
    }
    // INC-I-182: the devnet producer-auth gate is 0, so the producer signs the payload.
    let genesis = ConsensusParams::devnet().genesis_hash;
    doli_core::transaction::sign_producer_auth(&mut tx, &genesis, &k);
    tx
}

fn normal_withdrawal() -> Transaction {
    let pk = *kp().public_key();
    let ins = vec![Input::new(bond_op().tx_hash, bond_op().index)];
    signed(Transaction::new_request_withdrawal(
        ins,
        pk,
        1,
        addr(&pk),
        net(),
    ))
}

fn gated_withdrawal(lock_until: u64) -> Transaction {
    let pk = *kp().public_key();
    let ins = vec![Input::new(bond_op().tx_hash, bond_op().index)];
    let mut tx = Transaction::new_request_withdrawal(ins, pk, 1, addr(&pk), net());
    tx.outputs = vec![Output {
        output_type: OutputType::Unbonding,
        amount: net(),
        pubkey_hash: addr(&pk),
        lock_until,
        extra_data: Vec::new(),
    }];
    signed(tx)
}

fn pool() -> Mempool {
    Mempool::new(
        MempoolPolicy::testnet(),
        ConsensusParams::devnet(),
        Network::Devnet,
    )
}

fn offer(p: &mut Mempool, tx: &Transaction, utxo: &UtxoSet, tip: u64) -> Result<(), MempoolError> {
    p.add_transaction(tx.clone(), utxo, tip, 1).map(|_| ())
}

fn block_ctx(height: u64) -> ValidationContext {
    let n = Network::Devnet.params();
    ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, height)
        .with_sig_verification_height(n.sig_verification_height)
        .with_encrypted_content_activation_height(n.encrypted_content_activation_height)
        .with_encrypted_content_v2_activation_height(n.encrypted_content_v2_activation_height)
        .with_security_audit_activation_height(n.security_audit_activation_height)
        .with_defi_activation_height(n.defi_activation_height)
        .with_amm_activation_height(n.amm_activation_height)
        .with_inc_i_092_activation_height(n.inc_i_092_activation_height)
        .with_inc_i_096_activation_height(n.inc_i_096_activation_height)
        .with_inc_i_173_activation_height(n.inc_i_173_activation_height)
        .with_bls_key_rotation_activation_height(n.bls_key_rotation_activation_height)
        .with_inc_i_234_activation_height(n.inc_i_234_activation_height)
        .with_inc_i_236_block_signature_activation_height(
            n.inc_i_236_block_signature_activation_height,
        )
        .with_inc_i_255_slashing_activation_height(n.effective_slashing_activation_height())
        .with_inc_i_257_unbonding(n.inc_i_257_unbonding_activation_height, n.unbonding_period)
}

fn has_code(r: &Result<(), MempoolError>, code: &str) -> bool {
    matches!(r, Err(e) if e.to_string().contains(code))
}

fn guard16_bytes() -> Vec<u8> {
    let mut b = Condition::OutputTypeGuard {
        expected_type: OutputType::Normal,
        output_index: 0,
    }
    .encode()
    .expect("FIXTURE: encode guard");
    b[2] = 16;
    b
}

/// Parent: re-exec `test` alone with `env`, return true. Child: return false.
fn spawned_child(test: &str, env: &[(&str, u64)]) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        return false;
    }
    let mut cmd = std::process::Command::new(std::env::current_exe().expect("current exe"));
    cmd.arg(format!("inc_i_257_m25_mempool_height::{test}"))
        .args(["--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env_remove(GATE_ENV)
        .env_remove(SLASH_ENV);
    for (k, v) in env {
        cmd.env(k, v.to_string());
    }
    let out = cmd.output().expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "{test}: child failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    true
}

fn assert_devnet_preconditions() {
    let n = Network::Devnet.params();
    assert_eq!(
        n.inc_i_257_unbonding_activation_height, 0,
        "PRECONDITION: devnet gate 0"
    );
    assert_eq!(n.unbonding_period, PERIOD, "PRECONDITION: devnet period 60");
}

// REQ-UNBOND-005 — Decision: a failure means admission judges the payout floor at the tip, so a withdrawal no block can carry is admitted, propagated and holds the producer's Bonds.
#[test]
fn inc_i_257_m25_req_unbond_005_admission_refuses_lock_at_tip_floor() {
    assert_devnet_preconditions();
    let utxo = bond_utxo();
    let tx = gated_withdrawal(T + PERIOD);
    let mut p = pool();
    let r = offer(&mut p, &tx, &utxo, T);
    assert!(
        has_code(&r, "ERRTX-UNBOND006"),
        "tip {T}, lock_until {}: block height is >= {} so the floor fails; expected ERRTX-UNBOND006, got {r:?}",
        T + PERIOD,
        T + 1
    );
    assert!(
        !p.contains(&tx.hash()),
        "a refused withdrawal must not be resident"
    );
}

// REQ-UNBOND-005 — Decision: a failure means the tip + 1 floor over-refuses the earliest lock the next block accepts.
#[test]
fn inc_i_257_m25_req_unbond_005_admission_accepts_lock_at_tip_plus_one_floor() {
    assert_devnet_preconditions();
    let utxo = bond_utxo();
    let tx = gated_withdrawal(T + 1 + PERIOD);
    let mut p = pool();
    let r = offer(&mut p, &tx, &utxo, T);
    assert!(
        r.is_ok(),
        "tip {T}, lock_until {}: must admit, got {r:?}",
        T + 1 + PERIOD
    );
    assert!(p.contains(&tx.hash()), "an admitted withdrawal is resident");
}

// REQ-UNBOND-005 / REQ-UNBOND-017 — Decision: a failure means admission applies the gate at the tip, so at tip G-1 the honest gated shape is refused and the pre-gate shape the gate block refuses is admitted.
#[test]
fn inc_i_257_m25_req_unbond_017_admission_gate_judged_at_tip_plus_one() {
    if spawned_child(
        "inc_i_257_m25_req_unbond_017_admission_gate_judged_at_tip_plus_one",
        &[(GATE_ENV, G)],
    ) {
        return;
    }
    assert_eq!(
        Network::Devnet
            .params()
            .inc_i_257_unbonding_activation_height,
        G,
        "PRECONDITION: child sees gate {G}"
    );
    let utxo = bond_utxo();
    let gated = gated_withdrawal(G + PERIOD + 5);
    let normal = normal_withdrawal();
    for (tip, normal_ok, gated_ok) in [(G - 2, true, false), (G - 1, false, true)] {
        let mut p = pool();
        let rn = offer(&mut p, &normal, &utxo, tip);
        let mut q = pool();
        let rg = offer(&mut q, &gated, &utxo, tip);
        assert_eq!(
            (rn.is_ok(), rg.is_ok()),
            (normal_ok, gated_ok),
            "gate {G}, tip {tip} (block {}): expected Normal admitted={normal_ok}, gated admitted={gated_ok}; got Normal {rn:?}, gated {rg:?}",
            tip + 1
        );
        assert_eq!(
            p.contains(&normal.hash()),
            normal_ok,
            "Normal residency at tip {tip}"
        );
        assert_eq!(
            q.contains(&gated.hash()),
            gated_ok,
            "gated residency at tip {tip}"
        );
    }
}

// REQ-UNBOND-005 — Decision: a failure means a withdrawal that aged past its floor stays resident and holds the producer's Bond outpoints, so the retry is refused as a double spend.
#[test]
fn inc_i_257_m25_req_unbond_005_revalidate_evicts_unmineable_withdrawal() {
    assert_devnet_preconditions();
    let utxo = bond_utxo();
    let stale = gated_withdrawal(T + 1 + PERIOD);
    let mut p = pool();
    offer(&mut p, &stale, &utxo, T).expect("PRECONDITION: admitted at tip T");
    p.revalidate(&utxo, T, 1);
    assert!(
        p.contains(&stale.hash()),
        "NON-VACUITY: at tip T the withdrawal is still mineable and stays resident"
    );
    p.revalidate(&utxo, T + 1, 2);
    assert!(
        !p.contains(&stale.hash()),
        "tip {}: lock_until {} is below the floor {} of the next block; revalidate must evict it",
        T + 1,
        T + 1 + PERIOD,
        T + 2 + PERIOD
    );
    let retry = gated_withdrawal(T + 2 + PERIOD);
    let r = offer(&mut p, &retry, &utxo, T + 1);
    assert!(
        r.is_ok(),
        "a fresh withdrawal for the same Bonds must be admitted: {r:?}"
    );
    assert!(p.contains(&retry.hash()), "the replacement is resident");
}

// REQ-UNBOND-005 — Decision: a failure means the mempool and the block validator disagree on a withdrawal's lock floor, so admission either censors or admits unmineable transactions.
#[test]
fn inc_i_257_m25_req_unbond_005_admission_parity_with_block_validator() {
    assert_devnet_preconditions();
    let utxo = bond_utxo();
    let oracle = block_ctx(T + 1);
    let expect = [
        (T + 59, false),
        (T + 60, false),
        (T + 61, true),
        (T + 62, true),
    ];
    for (lock, ok) in expect {
        let v = validate_transaction_with_utxos(&gated_withdrawal(lock), &oracle, &utxo);
        assert_eq!(
            v.is_ok(),
            ok,
            "NON-VACUITY: block validator at H={} for lock_until {lock}: {v:?}",
            T + 1
        );
    }
    let mut rows = Vec::new();
    for (lock, _) in expect {
        let tx = gated_withdrawal(lock);
        let block = validate_transaction_with_utxos(&tx, &oracle, &utxo).is_ok();
        let mut p = pool();
        let pool_ok = offer(&mut p, &tx, &utxo, T).is_ok();
        rows.push((lock, pool_ok, block));
    }
    let diverging: Vec<_> = rows.iter().filter(|(_, a, b)| a != b).collect();
    assert!(
        diverging.is_empty(),
        "mempool at tip {T} vs block validator at H={}: (lock_until, mempool_ok, block_ok) diverging rows {diverging:?}",
        T + 1
    );
}

// REQ-UNBOND-005 / REQ-UNBOND-017 — Decision: the M2.5 outcome metric; total > 0 means at least one of the three defects is still live.
#[test]
fn inc_i_257_m25_outcome_probe() {
    let guard16 = u8::from(Condition::decode(&guard16_bytes()).is_ok());
    let utxo = bond_utxo();
    let mut p = pool();
    let unmineable = u8::from(offer(&mut p, &gated_withdrawal(T + PERIOD), &utxo, T).is_ok());
    let stale_tx = gated_withdrawal(T + 1 + PERIOD);
    let mut q = pool();
    let admitted = offer(&mut q, &stale_tx, &utxo, T).is_ok();
    q.revalidate(&utxo, T + 1, 2);
    let stale = u8::from(admitted && q.contains(&stale_tx.hash()));
    let total = guard16 + unmineable + stale;
    println!(
        "m25_defects guard16_decodes={guard16} unmineable_admitted={unmineable} stale_resident_after_revalidate={stale} total={total}"
    );
    assert!(
        admitted,
        "PRECONDITION: the T+61 withdrawal is admitted at tip T"
    );
    assert_eq!(total, 0, "M2.5 defects still live");
}

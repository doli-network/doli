//! INC-I-182 M1 — the mempool judges the producer-auth rule at the mined-at
//! height (tip + 1) on admission and on revalidate, so what it holds is what
//! the next block may carry.
//!
// covers: producer_auth transaction types unbonding defaults env_loader network_params pool assembly tx_processing validation_checks withdrawal_holdings genesis register bonds withdrawal exit unbonding_tx core data block rpc_client
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64 /*tip*/, u32) -> Result<AddTransactionResult, MempoolError>
// OUTPUT CONTRACT: fn Mempool::revalidate(&mut self, &UtxoSet, u64 /*tip*/, u32)
//   O1 add_transaction return (Ok | Err with ERRTX-PAUTH001); O2 residency (contains)
//   O3 revalidate eviction (contains); UtxoSet read-only
// OUTPUT CONTRACT: fn validate_transaction_with_utxos(..) at height G   O4 block verdict for the same tx
// PATHS: child process, producer-auth gate env G = 100 | P-ADMIT | P-REVAL | P-BLOCK
// MATRIX
//   O1, O2 x unsigned x tip {G-2, G-1} -> admitted / refused with the code
//   O1, O4 x signed x tip G-1 -> admitted and valid in the block at G; unsigned at G -> block refusal with the code
//     (inc_i_182_m1_req_pauth_002_admission_judged_at_next_height)
//   O2, O3 x {unsigned, signed} admitted at G-2, revalidated at tip G-1 -> evicted / kept
//     (inc_i_182_m1_req_pauth_002_revalidate_evicts_unsigned_at_gate)
// INPUT PARTITIONS:
//   form: unsigned | producer-signed with the mempool's genesis; payout: Unbonding, lock far above the floor

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::transaction::{Input, OutputType, Transaction};
use doli_core::validation::{validate_transaction_with_utxos, ValidationContext};
use mempool::{Mempool, MempoolError, MempoolPolicy};
use storage::{Outpoint, UtxoEntry, UtxoSet};

const CODE: &str = "ERRTX-PAUTH001";
const G: u64 = 100;
const FEE: u64 = 1_000;
const GATE_ENV: &str = "DOLI_INC_I_182_PRODUCER_AUTH_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_182_M1_MEMPOOL_CHILD";

fn kp() -> KeyPair {
    KeyPair::from_seed([0x82; 32])
}

fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

fn bond_op(tag: u8) -> Outpoint {
    Outpoint::new(Hash::from_bytes([tag; 32]), 0)
}

fn bonds() -> UtxoSet {
    let pk = *kp().public_key();
    let mut utxo = UtxoSet::new();
    for tag in [0x25, 0x26] {
        let entry = UtxoEntry {
            output: doli_core::transaction::Output::bond(
                Network::Devnet.bond_unit(),
                addr(&pk),
                u64::MAX,
                0,
            ),
            height: 0,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        utxo.insert(bond_op(tag), entry)
            .expect("FIXTURE: seed Bond");
    }
    utxo
}

fn reference_sign(tx: &mut Transaction) {
    let mut h = crypto::Hasher::new();
    h.update(b"DOLI-PRODUCER-AUTH-V1");
    h.update(ConsensusParams::devnet().genesis_hash.as_bytes());
    h.update(tx.signing_message().as_bytes());
    h.update(&tx.extra_data);
    let sig = crypto::signature::sign_hash(&h.finalize(), kp().private_key());
    tx.extra_data.extend_from_slice(sig.as_bytes());
}

fn withdrawal(tag: u8, signed: bool) -> Transaction {
    let k = kp();
    let pk = *k.public_key();
    let op = bond_op(tag);
    let mut input = Input::new(op.tx_hash, op.index);
    input.public_key = Some(pk);
    let net = Network::Devnet.bond_unit() - FEE;
    let mut tx = Transaction::new_request_withdrawal(vec![input], pk, 1, addr(&pk), net);
    tx.outputs[0].output_type = OutputType::Unbonding;
    tx.outputs[0].lock_until = G + 1_000;
    let msg = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&msg, k.private_key());
    if signed {
        reference_sign(&mut tx);
    }
    tx
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
        .with_inc_i_173_activation_height(n.inc_i_173_activation_height)
        .with_inc_i_234_activation_height(n.inc_i_234_activation_height)
        .with_inc_i_255_slashing_activation_height(n.effective_slashing_activation_height())
        .with_inc_i_257_unbonding(n.inc_i_257_unbonding_activation_height, n.unbonding_period)
        .with_inc_i_182_producer_auth(n.inc_i_182_producer_auth_activation_height)
}

fn coded(r: &Result<(), MempoolError>) -> bool {
    matches!(r, Err(e) if format!("{e} {e:?}").contains(CODE))
}

/// Parent: re-exec `test` alone with the gate at G, return true. Child: return false.
fn spawned_child(test: &str) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        assert_eq!(
            Network::Devnet
                .params()
                .inc_i_182_producer_auth_activation_height,
            G,
            "PRECONDITION: {GATE_ENV} moves the devnet gate"
        );
        return false;
    }
    let out = std::process::Command::new(std::env::current_exe().expect("current exe"))
        .arg(format!("inc_i_182_m1_mempool_height::{test}"))
        .args(["--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env(GATE_ENV, G.to_string())
        .output()
        .expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "{test}: child failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    true
}

// REQ-PAUTH-002 — Decision: a failure means admission judges the rule at the tip, so the mempool holds and relays an unsigned producer tx that no block at the gate can carry, or refuses a signed one the next block accepts.
#[test]
fn inc_i_182_m1_req_pauth_002_admission_judged_at_next_height() {
    if spawned_child("inc_i_182_m1_req_pauth_002_admission_judged_at_next_height") {
        return;
    }
    let utxo = bonds();
    let unsigned = withdrawal(0x25, false);
    let below = offer(&mut pool(), &unsigned, &utxo, G - 2);
    assert!(
        below.is_ok(),
        "unsigned at tip {} (mined at {}): {below:?}",
        G - 2,
        G - 1
    );
    let at = offer(&mut pool(), &unsigned, &utxo, G - 1);
    assert!(
        coded(&at),
        "unsigned at tip {} (mined at {G}): {at:?}",
        G - 1
    );
    let block = validate_transaction_with_utxos(&unsigned, &block_ctx(G), &utxo);
    assert!(
        matches!(&block, Err(e) if format!("{e} {e:?}").contains(CODE)),
        "the block at {G} must refuse the unsigned tx with the same code: {block:?}"
    );
    let signed = withdrawal(0x25, true);
    let ok = offer(&mut pool(), &signed, &utxo, G - 1);
    assert!(ok.is_ok(), "signed at tip {}: {ok:?}", G - 1);
    let block = validate_transaction_with_utxos(&signed, &block_ctx(G), &utxo);
    assert!(
        block.is_ok(),
        "signed tx admitted at {} must be valid in the block at {G}: {block:?}",
        G - 1
    );
}

// REQ-PAUTH-002 — Decision: a failure means an unsigned producer tx admitted just below the gate stays resident after the tip reaches it, so the builder retries it and peers keep relaying it (or a signed one is evicted).
#[test]
fn inc_i_182_m1_req_pauth_002_revalidate_evicts_unsigned_at_gate() {
    if spawned_child("inc_i_182_m1_req_pauth_002_revalidate_evicts_unsigned_at_gate") {
        return;
    }
    let utxo = bonds();
    let (unsigned, signed) = (withdrawal(0x25, false), withdrawal(0x26, true));
    let mut p = pool();
    for tx in [&unsigned, &signed] {
        let r = offer(&mut p, tx, &utxo, G - 2);
        assert!(r.is_ok(), "PRECONDITION: admitted at tip {}: {r:?}", G - 2);
    }
    p.revalidate(&utxo, G - 1, 1);
    assert!(
        !p.contains(&unsigned.hash()),
        "revalidate at tip {} must evict the unsigned tx (next block is at the gate)",
        G - 1
    );
    assert!(
        p.contains(&signed.hash()),
        "revalidate must keep the signed tx"
    );
}

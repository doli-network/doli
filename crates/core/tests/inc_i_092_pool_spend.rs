// INC-I-092 RC-A reproduction: AMM pool UTXO is permanently unspendable.
//
// Root cause: Output::pool() creates a NON-conditioned output with
// pubkey_hash = pool_id. The spend-authorization path
// (validate_transaction_with_utxos -> verify_input_conditions ->
// verify_input_signature) demands a pubkey whose ADDRESS_DOMAIN hash equals
// pool_id. But pool_id = BLAKE3(POOL_ID_DOMAIN || fee_bps || sort(a,b)) is NOT
// a pubkey hash, so NO key can satisfy it -> PubkeyHashMismatch ([MPTX002] in
// the mempool). Every Swap/AddLiquidity/RemoveLiquidity is rejected.
//
// Intended model: the pool input is authorized by the AMM invariant
// (new_k >= old_k + conservation, already enforced in validation/utxo.rs),
// exactly like ZKRollup is authorized by its proof. The pool-input signature
// EXEMPTION was never implemented. This fix adds it, GATED behind
// inc_i_092_activation_height for rolling-deploy safety (testnet amm is LIVE
// at h=20099).
//
// OUTPUT CONTRACT: fn validate_transaction_with_utxos(tx, ctx, utxo_provider)
//                  for tx.tx_type == Swap whose input[0] is a Pool UTXO.
//   Outputs:
//     O1: returned Result<(), ValidationError>.
//   PATHS:
//     P1: ctx.current_height >= ctx.inc_i_092_activation_height
//         -> the Pool input[0] of an AMM tx is EXEMPT from signature
//            verification; authorization is the swap invariant. An
//            invariant-valid A->B swap returns Ok.
//     P2: ctx.current_height <  ctx.inc_i_092_activation_height
//         -> legacy (pre-activation) behavior preserved: Pool input takes the
//            signature path -> Err(PubkeyHashMismatch). This is the CURRENT
//            buggy-but-consensus-consistent behavior; the gate must NOT change
//            it before activation (no fork on mixed fleet).
//   INPUT PARTITIONS:
//     IP1: invariant-valid A->B swap (DOLI in, tokens out), pool input carries
//          the swapper's pubkey (hash != pool_id), gate ACTIVE (inc_i_092=0).
//     IP2: same swap, gate INACTIVE (inc_i_092 = u64::MAX).
//     IP3: same swap, boundary current_height == inc_i_092 (strict `<` gate =>
//          active at equality).
//   MATRIX:
//     O1 x P1 x IP1 -> swap_with_pool_input_accepted_when_gate_active
//     O1 x P2 x IP2 -> swap_with_pool_input_rejected_when_gate_inactive
//     O1 x P1 x IP3 -> swap_pool_input_exempt_at_activation_boundary

use crypto::Hash;
use doli_core::conditions::Condition;
use doli_core::consensus::{ConsensusParams, GENESIS_TIME};
use doli_core::network::Network;
use doli_core::transaction::{Input, Output, SighashType, Transaction, TxType};
use doli_core::validation::{self, UtxoInfo, UtxoProvider, ValidationContext, ValidationError};
use std::collections::HashMap;

const POOL_PREV: Hash = Hash::from_bytes([0xA0; 32]);
const FUND_PREV: Hash = Hash::from_bytes([0xB0; 32]);

struct MockUtxos {
    utxos: HashMap<(Hash, u32), UtxoInfo>,
}
impl UtxoProvider for MockUtxos {
    fn get_utxo(&self, tx_hash: &Hash, index: u32) -> Option<UtxoInfo> {
        self.utxos.get(&(*tx_hash, index)).cloned()
    }
}

/// Build an invariant-valid A->B swap (DOLI in, tokens out) plus the UTXO set.
///
/// Old pool: reserve_a=1000 DOLI, reserve_b=1000 tokens, k=1_000_000.
/// New pool: reserve_a=1100, reserve_b=910 -> new_k=1_001_000 >= old_k.
/// tokens_out = 1000 - 910 = 90 -> FungibleAsset output to swapper.
/// Funding: 100 DOLI Normal input (native). total_input_native=100 >= 0.
fn build_swap_and_utxos() -> (Transaction, MockUtxos, crypto::KeyPair) {
    let kp = crypto::KeyPair::from_seed([0x42; 32]);
    let user_pkh =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes());

    let asset_b = Hash::from_bytes([0xBB; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);

    let old_pool = Output::pool(pool_id, asset_b, 1000, 1000, 707, 0, 100, 30, 100);
    let new_pool = Output::pool(pool_id, asset_b, 1100, 910, 707, 0, 101, 30, 100);
    let tokens_out = Output::fungible_asset(
        90,
        user_pkh,
        asset_b,
        1_000_000,
        "TKN",
        &Condition::Signature(user_pkh),
    )
    .expect("fungible asset output must encode");

    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Swap,
        inputs: vec![
            // input[0]: the Pool UTXO (non-conditioned, pubkey_hash = pool_id).
            // We attach the swapper's pubkey to reproduce the exact [MPTX002]
            // PubkeyHashMismatch symptom (user_pkh != pool_id).
            Input {
                prev_tx_hash: POOL_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
            // input[1]: DOLI funding (Normal, native).
            Input {
                prev_tx_hash: FUND_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
        ],
        outputs: vec![new_pool, tokens_out],
        extra_data: vec![],
    };

    // Sign both inputs (input[0] signature is irrelevant once exempt, but the
    // mismatch is on the pubkey HASH, not the signature, so we still sign).
    let sh0 = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&sh0, kp.private_key());
    let sh1 = tx.signing_message_for_input(1);
    tx.inputs[1].signature = crypto::signature::sign_hash(&sh1, kp.private_key());

    let mut utxos = MockUtxos {
        utxos: HashMap::new(),
    };
    utxos.utxos.insert(
        (POOL_PREV, 0),
        UtxoInfo {
            output: old_pool,
            pubkey: None,
            spent: false,
        },
    );
    utxos.utxos.insert(
        (FUND_PREV, 0),
        UtxoInfo {
            output: Output::normal(100, user_pkh),
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );

    (tx, utxos, kp)
}

fn ctx_at(height: u64, inc_i_092: u64) -> ValidationContext {
    ValidationContext::new(
        ConsensusParams::devnet(),
        Network::Devnet,
        GENESIS_TIME + 10 * height,
        height,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_sig_verification_height(0)
    .with_amm_activation_height(0)
    .with_inc_i_092_activation_height(inc_i_092)
}

// O1 x P1 x IP1 — gate active: pool input exempt, invariant-valid swap accepted.
#[test]
fn swap_with_pool_input_accepted_when_gate_active() {
    let (tx, utxos, _kp) = build_swap_and_utxos();
    let ctx = ctx_at(10, 0);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    assert!(
        res.is_ok(),
        "with inc_i_092 gate ACTIVE, an invariant-valid swap spending the Pool \
         UTXO must be accepted (pool input authorized by the AMM invariant, not \
         a signature). Got: {:?}",
        res
    );
}

// O1 x P2 x IP2 — gate inactive: legacy behavior preserved (rejected).
#[test]
fn swap_with_pool_input_rejected_when_gate_inactive() {
    let (tx, utxos, _kp) = build_swap_and_utxos();
    let ctx = ctx_at(10, u64::MAX);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    match res {
        Err(ValidationError::PubkeyHashMismatch { .. }) => {}
        other => panic!(
            "with inc_i_092 gate INACTIVE, the Pool input must still hit the \
             signature path and fail PubkeyHashMismatch (pre-activation \
             consensus must not change). Got: {:?}",
            other
        ),
    }
}

// O1 x P1 x IP3 — boundary: strict `<` gate => active at current_height == gate.
#[test]
fn swap_pool_input_exempt_at_activation_boundary() {
    let (tx, utxos, _kp) = build_swap_and_utxos();
    let ctx = ctx_at(10, 10);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    assert!(
        res.is_ok(),
        "gate uses `current_height >= inc_i_092_activation_height`, so at \
         equality the pool input must already be exempt. Got: {:?}",
        res
    );
}

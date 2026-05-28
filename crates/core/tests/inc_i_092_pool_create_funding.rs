// INC-I-092 RC-B reproduction: CreatePool accepts UNFUNDED reserves -> inflation.
//
// Root cause: Output::pool() sets amount=0 and stores reserve_a (DOLI) in
// extra_data. The DOLI-conservation check `total_input >= total_output`
// (validation/utxo.rs) compares native input against native output, and a Pool
// output's native amount is 0 -> the declared reserve_a is NEVER checked
// against the DOLI the caller actually funded. A CreatePool can declare
// reserve_a = u64::MAX while consuming ~nothing, minting ~184B phantom DOLI
// into pool reserves. The token side (reserve_b) has the same hole.
//
// Fix: in CreatePool UTXO-context validation, require the declared reserves be
// backed by NET inputs:
//   net DOLI in  = native_input  - native_change_out          >= reserve_a
//   net asset_b in = sum(asset_b FungibleAsset in) - asset_b change >= reserve_b
// Gated by inc_i_092_activation_height (below it, the legacy accept behavior is
// preserved so a mixed fleet does not fork).
//
// OUTPUT CONTRACT: fn validate_transaction_with_utxos(tx, ctx, utxo_provider)
//                  for tx.tx_type == CreatePool.
//   Outputs:
//     O1: returned Result<(), ValidationError>.
//   PATHS:
//     P1: gate ACTIVE  & declared reserve_a > net DOLI funded -> Err (rejected).
//     P2: gate ACTIVE  & reserves fully funded               -> Ok (accepted).
//     P3: gate INACTIVE & declared reserve_a > net DOLI funded -> Ok (legacy
//         behavior preserved; pre-activation must not change consensus).
//   INPUT PARTITIONS:
//     IP1: reserve_a = u64::MAX, DOLI input = 1000 (the reported vector),
//          reserve_b backed, gate active (inc_i_092 = 0).
//     IP2: reserve_a = 1000 backed by 1000 DOLI input, reserve_b = 2000 backed
//          by a 2000-token FungibleAsset input, gate active.
//     IP3: same as IP1 but gate inactive (inc_i_092 = u64::MAX).
//     IP4: reserve_a fully funded (1000 DOLI) but reserve_b underfunded
//          (token input 500 < reserve_b 2000), gate active — exercises the
//          reserve_b guard firing on its own.
//   MATRIX:
//     O1 x P1 x IP1 -> create_pool_rejects_unfunded_reserve_a_when_gate_active
//     O1 x P2 x IP2 -> create_pool_accepts_fully_funded_reserves
//     O1 x P3 x IP3 -> create_pool_unfunded_reserve_allowed_when_gate_inactive
//     O1 x P1 x IP4 -> create_pool_rejects_unfunded_reserve_b_when_gate_active

use crypto::Hash;
use doli_core::conditions::{Condition, Witness, WitnessSignature};
use doli_core::consensus::{ConsensusParams, GENESIS_TIME, MINIMUM_LIQUIDITY};
use doli_core::network::Network;
use doli_core::transaction::{Input, Output, SighashType, Transaction, TxType};
use doli_core::validation::{self, UtxoInfo, UtxoProvider, ValidationContext};
use std::collections::HashMap;

const DOLI_PREV: Hash = Hash::from_bytes([0xC0; 32]);
const TOKEN_PREV: Hash = Hash::from_bytes([0xD0; 32]);

struct MockUtxos {
    utxos: HashMap<(Hash, u32), UtxoInfo>,
}
impl UtxoProvider for MockUtxos {
    fn get_utxo(&self, tx_hash: &Hash, index: u32) -> Option<UtxoInfo> {
        self.utxos.get(&(*tx_hash, index)).cloned()
    }
}

/// Build a CreatePool tx + UTXO set.
///
/// reserve_a is the declared DOLI reserve. `doli_input` is the actual DOLI
/// funded. `token_input` is the actual asset_b funded; reserve_b is fixed at
/// 2000. The creator LPShare is (total_lp - MINIMUM_LIQUIDITY) per D1.
fn build_create_pool(
    reserve_a: u64,
    doli_input: u64,
    token_input: u64,
) -> (Transaction, MockUtxos) {
    let kp = crypto::KeyPair::from_seed([0x77; 32]);
    let user_pkh =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes());

    let asset_b = Hash::from_bytes([0xBB; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);
    let reserve_b: u64 = 2000;

    let total_lp = 707 + MINIMUM_LIQUIDITY; // D1: creator 707 + locked 1000
    let pool_out = Output::pool(
        pool_id, asset_b, reserve_a, reserve_b, total_lp, 0, 100, 30, 100,
    );
    let lp_out = Output::lp_share(707, pool_id, user_pkh);

    // Token input is a conditioned FungibleAsset (Signature(owner)).
    let token_in = Output::fungible_asset(
        token_input,
        user_pkh,
        asset_b,
        1_000_000,
        "TKN",
        &Condition::Signature(user_pkh),
    )
    .expect("token output encodes");

    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::CreatePool,
        inputs: vec![
            Input {
                prev_tx_hash: DOLI_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
            Input {
                prev_tx_hash: TOKEN_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
        ],
        outputs: vec![pool_out, lp_out],
        extra_data: vec![],
    };

    // Sign input[0] (Normal DOLI) into its signature field.
    let sh0 = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&sh0, kp.private_key());
    // Sign input[1] (FungibleAsset) and attach a Signature covenant witness.
    let sh1 = tx.signing_message_for_input(1);
    let sig1 = crypto::signature::sign_hash(&sh1, kp.private_key());
    tx.inputs[1].signature = sig1;
    let witness1 = Witness {
        signatures: vec![WitnessSignature {
            pubkey: *kp.public_key(),
            signature: sig1,
        }],
        preimage: None,
        or_branches: vec![],
    };
    tx.set_covenant_witnesses(&[vec![], witness1.encode()]);

    let mut utxos = MockUtxos {
        utxos: HashMap::new(),
    };
    utxos.utxos.insert(
        (DOLI_PREV, 0),
        UtxoInfo {
            output: Output::normal(doli_input, user_pkh),
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );
    utxos.utxos.insert(
        (TOKEN_PREV, 0),
        UtxoInfo {
            output: token_in,
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );

    (tx, utxos)
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

// O1 x P1 x IP1 — gate active: u64::MAX reserve_a with 1000 DOLI input rejected.
#[test]
fn create_pool_rejects_unfunded_reserve_a_when_gate_active() {
    let (tx, utxos) = build_create_pool(u64::MAX, 1000, 2000);
    let ctx = ctx_at(10, 0);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    assert!(
        res.is_err(),
        "declaring reserve_a=u64::MAX while funding only 1000 DOLI must be \
         REJECTED when the inc_i_092 gate is active (no phantom DOLI). Got: {:?}",
        res
    );
}

// O1 x P2 x IP2 — gate active: fully funded reserves accepted.
#[test]
fn create_pool_accepts_fully_funded_reserves() {
    let (tx, utxos) = build_create_pool(1000, 1000, 2000);
    let ctx = ctx_at(10, 0);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    assert!(
        res.is_ok(),
        "a CreatePool whose declared reserves are fully backed by inputs \
         (1000 DOLI, 2000 tokens) must be accepted. Got: {:?}",
        res
    );
}

// O1 x P1 x IP4 — gate active: reserve_b underfunded (token side) rejected.
#[test]
fn create_pool_rejects_unfunded_reserve_b_when_gate_active() {
    // reserve_a funded (1000 DOLI) but reserve_b=2000 backed by only 500 tokens.
    let (tx, utxos) = build_create_pool(1000, 1000, 500);
    let ctx = ctx_at(10, 0);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    assert!(
        res.is_err(),
        "declaring reserve_b=2000 while funding only 500 tokens must be REJECTED \
         when the gate is active (no phantom asset_b). Got: {:?}",
        res
    );
}

// O1 x P3 x IP3 — gate inactive: legacy (accepting) behavior preserved.
#[test]
fn create_pool_unfunded_reserve_allowed_when_gate_inactive() {
    let (tx, utxos) = build_create_pool(u64::MAX, 1000, 2000);
    let ctx = ctx_at(10, u64::MAX);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    assert!(
        res.is_ok(),
        "pre-activation (gate inactive) the legacy behavior must be preserved \
         so a mixed fleet does not fork. Got: {:?}",
        res
    );
}

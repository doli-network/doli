// INC-I-096 reproduction: AMM value conservation rejects valid RemoveLiquidity.
//
// Root cause: native value-conservation `total_input < total_output` at consensus
// (validation/utxo.rs:210-217) is blind to Pool reserve release. Pool UTXO has
// amount=0 (reserves in extra_data); is_native_amount() returns false for Pool/
// LPShare/FungibleAsset. Released DOLI (a Normal output) is counted in total_output
// but the pool reserve that funds it is invisible to total_input -> valid
// RemoveLiquidity and B->A Swap falsely rejected with InsufficientFunds.
//
// Fix: pool-aware conservation gated by inc_i_096_activation_height + proportional
// binding on reserve deltas + mempool parity fix. All gated; below the gate,
// behavior is bit-identical to the current (buggy) check.
//
// OUTPUT CONTRACT: fn validate_transaction_with_utxos(tx, ctx, utxo_provider)
//   for AMM types (RemoveLiquidity, Swap, AddLiquidity) with Pool UTXO inputs.
//
//   Outputs:
//     O1: returned Result<(), ValidationError>.
//   PATHS:
//     P1: gate ACTIVE (height >= inc_i_096) + valid remove -> Ok (pool-aware conservation)
//     P2: gate ACTIVE + malicious remove (1-share drain) -> Err (proportional binding)
//     P3: gate ACTIVE + malicious token drain -> Err (proportional binding on reserve_b)
//     P4: gate INACTIVE (height < inc_i_096) + valid remove -> Err(InsufficientFunds)
//         (legacy behavior preserved for mixed-fleet safety)
//     P5: gate ACTIVE + valid remove with rounding remainder -> Ok (<= not ==)
//     P6: gate ACTIVE + valid swap A->B -> Ok (no regression)
//     P7: gate ACTIVE + valid AddLiquidity -> Ok (no regression)
//   INPUT PARTITIONS:
//     IP1: valid RemoveLiquidity releasing 909 DOLI from reserve_a, burning 500 LP
//          shares out of 1000, gate active.
//     IP2: same tx, gate inactive (inc_i_096 = u64::MAX).
//     IP3: malicious remove: burns 1 LP share, drains reserve_a by 909.
//     IP4: malicious remove: burns proportional shares but drains excess token reserve.
//     IP5: valid remove with inexact division (shares=333, total=1000, reserve=1000).
//     IP6: valid A->B swap (DOLI in, tokens out).
//     IP7: valid AddLiquidity.
//   MATRIX:
//     T1: O1 x P1 x IP1 -> remove_liquidity_accepted_when_gate_active
//     T2: (mempool, see separate test) — valid remove passes mempool when active
//     T3: O1 x P2 x IP3 -> malicious_1share_drain_rejected_when_gate_active
//     T4: O1 x P3 x IP4 -> malicious_token_drain_rejected_when_gate_active
//     T5: O1 x P4 x IP2 -> remove_liquidity_rejected_when_gate_inactive
//     T6: O1 x P5 x IP5 -> rounding_remainder_remove_accepted_when_gate_active
//     T7: O1 x P6 x IP6 + O1 x P7 x IP7 -> no_regression_swap_and_add_liquidity

use crypto::Hash;
use doli_core::conditions::{Condition, Witness, WitnessSignature};
use doli_core::consensus::{ConsensusParams, GENESIS_TIME};
use doli_core::network::Network;
use doli_core::transaction::{Input, Output, SighashType, Transaction, TxType};
use doli_core::validation::{self, UtxoInfo, UtxoProvider, ValidationContext, ValidationError};
use std::collections::HashMap;

const POOL_PREV: Hash = Hash::from_bytes([0xA1; 32]);
const LP_PREV: Hash = Hash::from_bytes([0xA2; 32]);
const FUND_PREV: Hash = Hash::from_bytes([0xA3; 32]);
const TOKEN_PREV: Hash = Hash::from_bytes([0xA4; 32]);

struct MockUtxos {
    utxos: HashMap<(Hash, u32), UtxoInfo>,
}
impl UtxoProvider for MockUtxos {
    fn get_utxo(&self, tx_hash: &Hash, index: u32) -> Option<UtxoInfo> {
        self.utxos.get(&(*tx_hash, index)).cloned()
    }
}

fn ctx_at(height: u64, inc_i_092: u64, inc_i_096: u64) -> ValidationContext {
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
    .with_inc_i_096_activation_height(inc_i_096)
}

/// Sign a RemoveLiquidity tx with proper covenant witnesses.
/// input[0] = Pool (non-conditioned, exempt via INC-I-092)
/// input[1] = LPShare (conditioned with Signature(owner))
/// input[2] = Normal DOLI (non-conditioned, standard sig)
fn sign_remove_liquidity(tx: &mut Transaction, kp: &crypto::KeyPair) {
    // First, sign all inputs with standard signatures
    for i in 0..tx.inputs.len() {
        let sh = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&sh, kp.private_key());
    }

    // Build covenant witnesses: input[1] (LPShare) needs a Witness with the signature.
    // Inputs 0 and 2 are non-conditioned (empty witness).
    let sh1 = tx.signing_message_for_input(1);
    let sig1 = crypto::signature::sign_hash(&sh1, kp.private_key());
    let lp_witness = Witness {
        signatures: vec![WitnessSignature {
            pubkey: *kp.public_key(),
            signature: sig1,
        }],
        preimage: None,
        or_branches: vec![],
    };

    let witnesses: Vec<Vec<u8>> = vec![
        vec![],              // input[0]: Pool (non-conditioned)
        lp_witness.encode(), // input[1]: LPShare (conditioned)
        vec![],              // input[2]: Normal (non-conditioned)
    ];
    tx.set_covenant_witnesses(&witnesses);

    // Re-sign after setting witnesses (signing_message changes with extra_data)
    for i in 0..tx.inputs.len() {
        let sh = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&sh, kp.private_key());
    }

    // Re-build witnesses with fresh signatures
    let sh1 = tx.signing_message_for_input(1);
    let sig1 = crypto::signature::sign_hash(&sh1, kp.private_key());
    let lp_witness = Witness {
        signatures: vec![WitnessSignature {
            pubkey: *kp.public_key(),
            signature: sig1,
        }],
        preimage: None,
        or_branches: vec![],
    };
    let witnesses: Vec<Vec<u8>> = vec![vec![], lp_witness.encode(), vec![]];
    tx.set_covenant_witnesses(&witnesses);
}

/// Build a valid RemoveLiquidity tx + UTXO set.
///
/// Pool state: reserve_a=1000 DOLI, reserve_b=2000 tokens, total_lp=1000.
/// Burn 500 LP shares -> proportional: da=500*1000/1000=500, db=500*2000/1000=1000.
/// New pool: reserve_a=500, reserve_b=1000, total_lp=500.
///
/// Output layout MIRRORS the real CLI builder (cmd_pool.rs cmd_pool_remove):
///   [new Pool, DOLI out (Normal, 500), tokens out (FungibleAsset, 1000), fee change (Normal)].
/// Fee funding: a 1000-DOLI Normal input; fee_units=2 -> fee_change=998 Normal output.
///
/// The fee_change output is CRITICAL: a real remove almost always has DOLI change.
/// Any binding that sums ALL native outputs after the pool (doli_out + fee_change)
/// and compares to the reserve_a delta would falsely reject this — the bug INC-I-096
/// must NOT reintroduce. Conservation: native_input(1000)+old_reserve_a(1000)=2000 >=
/// native_output(500+998=1498)+new_reserve_a(500)=1998 (fee=2). ✓
fn build_valid_remove() -> (Transaction, MockUtxos, crypto::KeyPair) {
    let kp = crypto::KeyPair::from_seed([0x55; 32]);
    let user_pkh =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes());

    let asset_b = Hash::from_bytes([0xBB; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);

    let old_pool = Output::pool(pool_id, asset_b, 1000, 2000, 1000, 0, 100, 30, 100);
    let new_pool = Output::pool(pool_id, asset_b, 500, 1000, 500, 0, 101, 30, 100);

    let doli_out = Output::normal(500, user_pkh);
    let tokens_out = Output::fungible_asset(
        1000,
        user_pkh,
        asset_b,
        1_000_000,
        "TKN",
        &Condition::Signature(user_pkh),
    )
    .expect("fungible asset output must encode");
    // Real CLI fee-change output: 1000 DOLI fee input - 2 fee_units = 998 change.
    let fee_change = Output::normal(998, user_pkh);

    let old_lp = Output::lp_share(500, pool_id, user_pkh);

    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::RemoveLiquidity,
        inputs: vec![
            Input {
                prev_tx_hash: POOL_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
            Input {
                prev_tx_hash: LP_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
            Input {
                prev_tx_hash: FUND_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
        ],
        outputs: vec![new_pool, doli_out, tokens_out, fee_change],
        extra_data: vec![],
    };

    sign_remove_liquidity(&mut tx, &kp);

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
        (LP_PREV, 0),
        UtxoInfo {
            output: old_lp,
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );
    utxos.utxos.insert(
        (FUND_PREV, 0),
        UtxoInfo {
            output: Output::normal(1000, user_pkh),
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );

    (tx, utxos, kp)
}

// =============================================================================
// T1 (liveness, consensus): valid RemoveLiquidity passes when gate active.
// CURRENTLY FAILS with InsufficientFunds because released DOLI (500) is counted
// in total_output but pool reserve release is invisible to total_input.
// =============================================================================
#[test]
fn remove_liquidity_accepted_when_gate_active() {
    let (tx, utxos, _kp) = build_valid_remove();
    // Gate active: inc_i_096 = 0, inc_i_092 = 0
    let ctx = ctx_at(10, 0, 0);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    assert!(
        res.is_ok(),
        "T1 (INC-I-096): with inc_i_096 gate ACTIVE, a valid RemoveLiquidity that \
         releases 500 DOLI from reserves must pass pool-aware conservation. \
         Got: {:?}",
        res
    );
}

// =============================================================================
// T3 (security, consensus): malicious 1-share drain rejected with proportional
// binding when gate active. Burns only 1 LP share but decreases reserve_a by 500
// (the full proportional share of 500 LP shares). This would pass a blanket
// exemption but must fail proportional binding.
// =============================================================================
#[test]
fn malicious_1share_drain_rejected_when_gate_active() {
    let kp = crypto::KeyPair::from_seed([0x55; 32]);
    let user_pkh =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes());

    let asset_b = Hash::from_bytes([0xBB; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);

    // Old pool: 1000 DOLI, 2000 tokens, 1000 LP shares
    let old_pool = Output::pool(pool_id, asset_b, 1000, 2000, 1000, 0, 100, 30, 100);
    // Malicious: burn only 1 share but drain 500 DOLI from reserve
    // Proportional max for 1 share: 1*1000/1000 = 1 DOLI, 1*2000/1000 = 2 tokens
    let new_pool = Output::pool(pool_id, asset_b, 500, 1998, 999, 0, 101, 30, 100);

    let doli_out = Output::normal(500, user_pkh);
    let tokens_out = Output::fungible_asset(
        2,
        user_pkh,
        asset_b,
        1_000_000,
        "TKN",
        &Condition::Signature(user_pkh),
    )
    .expect("fungible asset output must encode");

    let old_lp = Output::lp_share(1, pool_id, user_pkh);

    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::RemoveLiquidity,
        inputs: vec![
            Input {
                prev_tx_hash: POOL_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
            Input {
                prev_tx_hash: LP_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
            Input {
                prev_tx_hash: FUND_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
        ],
        outputs: vec![new_pool, doli_out, tokens_out],
        extra_data: vec![],
    };

    sign_remove_liquidity(&mut tx, &kp);

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
        (LP_PREV, 0),
        UtxoInfo {
            output: old_lp,
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );
    utxos.utxos.insert(
        (FUND_PREV, 0),
        UtxoInfo {
            output: Output::normal(2, user_pkh),
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );

    let ctx = ctx_at(10, 0, 0);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    assert!(
        res.is_err(),
        "T3 (INC-I-096 security): a malicious RemoveLiquidity that burns only 1 LP \
         share but drains 500 DOLI from reserve_a MUST be rejected by proportional \
         binding (max DOLI out for 1 share = 1). Got Ok."
    );
    // Verify it's the right kind of rejection (proportional binding, not InsufficientFunds)
    if let Err(e) = &res {
        let msg = format!("{:?}", e);
        assert!(
            msg.contains("proportional") || msg.contains("InvalidLiquidity"),
            "T3: expected proportional binding rejection, got: {}",
            msg
        );
    }
}

// =============================================================================
// T4 (security, consensus): malicious token drain — tokens_out exceeds
// proportional reserve_b delta. Burns proportional shares for reserve_a but
// inflates token withdrawal.
// =============================================================================
#[test]
fn malicious_token_drain_rejected_when_gate_active() {
    let kp = crypto::KeyPair::from_seed([0x55; 32]);
    let user_pkh =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes());

    let asset_b = Hash::from_bytes([0xBB; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);

    // Old pool: 1000 DOLI, 2000 tokens, 1000 LP shares
    let old_pool = Output::pool(pool_id, asset_b, 1000, 2000, 1000, 0, 100, 30, 100);
    // Burn 500 shares. Proportional: da=500, db=1000. But attacker takes db=1500.
    // New pool: reserve_a=500 (correct), reserve_b=500 (should be 1000), total_lp=500
    let new_pool = Output::pool(pool_id, asset_b, 500, 500, 500, 0, 101, 30, 100);

    let doli_out = Output::normal(500, user_pkh);
    // Attacker claims 1500 tokens instead of proportional 1000
    let tokens_out = Output::fungible_asset(
        1500,
        user_pkh,
        asset_b,
        1_000_000,
        "TKN",
        &Condition::Signature(user_pkh),
    )
    .expect("fungible asset output must encode");

    let old_lp = Output::lp_share(500, pool_id, user_pkh);

    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::RemoveLiquidity,
        inputs: vec![
            Input {
                prev_tx_hash: POOL_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
            Input {
                prev_tx_hash: LP_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
            Input {
                prev_tx_hash: FUND_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
        ],
        outputs: vec![new_pool, doli_out, tokens_out],
        extra_data: vec![],
    };

    sign_remove_liquidity(&mut tx, &kp);

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
        (LP_PREV, 0),
        UtxoInfo {
            output: old_lp,
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );
    utxos.utxos.insert(
        (FUND_PREV, 0),
        UtxoInfo {
            output: Output::normal(2, user_pkh),
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );

    let ctx = ctx_at(10, 0, 0);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    assert!(
        res.is_err(),
        "T4 (INC-I-096 security): a RemoveLiquidity that takes 1500 tokens when \
         proportional max is 1000 MUST be rejected. Got Ok."
    );
}

// =============================================================================
// T5 (mixed-fleet safety): below gate, valid remove is STILL rejected
// (bit-identical to current behavior — no fork on mixed fleet).
// =============================================================================
#[test]
fn remove_liquidity_rejected_when_gate_inactive() {
    let (tx, utxos, _kp) = build_valid_remove();
    // Gate inactive: inc_i_096 = u64::MAX
    let ctx = ctx_at(10, 0, u64::MAX);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    match &res {
        Err(ValidationError::InsufficientFunds { .. }) => {
            // Expected: the old (buggy) conservation check fires because pool
            // reserve release is invisible. This is the current behavior that
            // must be preserved below the gate.
        }
        other => panic!(
            "T5 (INC-I-096 mixed-fleet): below the inc_i_096 gate, the valid \
             remove MUST still be rejected with InsufficientFunds (preserving \
             current consensus). Got: {:?}",
            other
        ),
    }
}

// =============================================================================
// T6 (rounding): valid remove where shares*reserve/total has a remainder is
// ACCEPTED when gate active (proves <= not ==). shares=333, total=1000,
// reserve_a=1000 -> exact: 333*1000/1000=333.0, floor=333.
// reserve_b=2000 -> exact: 333*2000/1000=666.0, floor=666.
// We set actual deltas to 333 and 666 (matching floor), which must pass.
// =============================================================================
#[test]
fn rounding_remainder_remove_accepted_when_gate_active() {
    let kp = crypto::KeyPair::from_seed([0x55; 32]);
    let user_pkh =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes());

    let asset_b = Hash::from_bytes([0xBB; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);

    // Old pool: 1000 DOLI, 2001 tokens, 1000 LP shares
    // Burn 333 shares -> proportional:
    //   da = 333*1000/1000 = 333 (exact)
    //   db = 333*2001/1000 = 666.333 -> floor = 666
    let old_pool = Output::pool(pool_id, asset_b, 1000, 2001, 1000, 0, 100, 30, 100);
    let new_pool = Output::pool(pool_id, asset_b, 667, 1335, 667, 0, 101, 30, 100);

    let doli_out = Output::normal(333, user_pkh);
    let tokens_out = Output::fungible_asset(
        666,
        user_pkh,
        asset_b,
        1_000_000,
        "TKN",
        &Condition::Signature(user_pkh),
    )
    .expect("fungible asset output must encode");

    let old_lp = Output::lp_share(333, pool_id, user_pkh);

    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::RemoveLiquidity,
        inputs: vec![
            Input {
                prev_tx_hash: POOL_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
            Input {
                prev_tx_hash: LP_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
            Input {
                prev_tx_hash: FUND_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
        ],
        outputs: vec![new_pool, doli_out, tokens_out],
        extra_data: vec![],
    };

    sign_remove_liquidity(&mut tx, &kp);

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
        (LP_PREV, 0),
        UtxoInfo {
            output: old_lp,
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );
    utxos.utxos.insert(
        (FUND_PREV, 0),
        UtxoInfo {
            output: Output::normal(2, user_pkh),
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );

    let ctx = ctx_at(10, 0, 0);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    assert!(
        res.is_ok(),
        "T6 (INC-I-096 rounding): a valid RemoveLiquidity where shares*reserve/total \
         has a remainder (floor division) must be accepted with <= binding, not ==. \
         Got: {:?}",
        res
    );
}

// =============================================================================
// T7 (no regression): valid Swap A->B and valid AddLiquidity still pass when
// gate active. CreatePool RC-B enforcement is also preserved.
// =============================================================================
#[test]
fn no_regression_swap_a_to_b_passes_when_gate_active() {
    let kp = crypto::KeyPair::from_seed([0x42; 32]);
    let user_pkh =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes());

    let asset_b = Hash::from_bytes([0xBB; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);

    // A->B swap: 100 DOLI in, tokens out. old_k=1_000_000, new_k=1_001_000.
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
            Input {
                prev_tx_hash: POOL_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
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

    for i in 0..tx.inputs.len() {
        let sh = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&sh, kp.private_key());
    }

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

    let ctx = ctx_at(10, 0, 0);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    assert!(
        res.is_ok(),
        "T7a (INC-I-096 no regression): a valid A->B swap must still pass when the \
         inc_i_096 gate is active. Got: {:?}",
        res
    );
}

#[test]
fn no_regression_add_liquidity_passes_when_gate_active() {
    let kp = crypto::KeyPair::from_seed([0x42; 32]);
    let user_pkh =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes());

    let asset_b = Hash::from_bytes([0xBB; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);

    // Add liquidity: 500 DOLI + 500 tokens to existing pool
    let old_pool = Output::pool(pool_id, asset_b, 1000, 1000, 707, 0, 100, 30, 100);
    let new_pool = Output::pool(pool_id, asset_b, 1500, 1500, 1060, 0, 101, 30, 100);
    let new_lp = Output::lp_share(353, pool_id, user_pkh);

    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::AddLiquidity,
        inputs: vec![
            Input {
                prev_tx_hash: POOL_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
            Input {
                prev_tx_hash: FUND_PREV,
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
        outputs: vec![new_pool, new_lp],
        extra_data: vec![],
    };

    // Sign with proper covenant witnesses for FungibleAsset input
    {
        // First pass: sign all inputs with standard signatures
        for i in 0..tx.inputs.len() {
            let sh = tx.signing_message_for_input(i);
            tx.inputs[i].signature = crypto::signature::sign_hash(&sh, kp.private_key());
        }

        // Build covenant witnesses: input[2] (FungibleAsset) needs a Witness.
        let sh2 = tx.signing_message_for_input(2);
        let sig2 = crypto::signature::sign_hash(&sh2, kp.private_key());
        let token_witness = Witness {
            signatures: vec![WitnessSignature {
                pubkey: *kp.public_key(),
                signature: sig2,
            }],
            preimage: None,
            or_branches: vec![],
        };
        let witnesses: Vec<Vec<u8>> = vec![
            vec![],                 // input[0]: Pool (non-conditioned)
            vec![],                 // input[1]: Normal DOLI (non-conditioned)
            token_witness.encode(), // input[2]: FungibleAsset (conditioned)
        ];
        tx.set_covenant_witnesses(&witnesses);

        // Re-sign after setting witnesses
        for i in 0..tx.inputs.len() {
            let sh = tx.signing_message_for_input(i);
            tx.inputs[i].signature = crypto::signature::sign_hash(&sh, kp.private_key());
        }
        let sh2 = tx.signing_message_for_input(2);
        let sig2 = crypto::signature::sign_hash(&sh2, kp.private_key());
        let token_witness = Witness {
            signatures: vec![WitnessSignature {
                pubkey: *kp.public_key(),
                signature: sig2,
            }],
            preimage: None,
            or_branches: vec![],
        };
        let witnesses: Vec<Vec<u8>> = vec![vec![], vec![], token_witness.encode()];
        tx.set_covenant_witnesses(&witnesses);
    }

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
            output: Output::normal(500, user_pkh),
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );
    let token_input = Output::fungible_asset(
        500,
        user_pkh,
        asset_b,
        1_000_000,
        "TKN",
        &Condition::Signature(user_pkh),
    )
    .expect("fungible asset output must encode");
    utxos.utxos.insert(
        (TOKEN_PREV, 0),
        UtxoInfo {
            output: token_input,
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );

    let ctx = ctx_at(10, 0, 0);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    assert!(
        res.is_ok(),
        "T7b (INC-I-096 no regression): a valid AddLiquidity must still pass when \
         the inc_i_096 gate is active. Got: {:?}",
        res
    );
}

// =============================================================================
// T10 (security, LP-input binding): a RemoveLiquidity that declares new_total_lp
// far below what the consumed LPShare inputs justify MUST be rejected. Attacker
// holds 1 LP share but declares new_total_lp=0 (claims to burn all 1000), which
// inflates shares_burned and lets the proportional binding pass while draining the
// ENTIRE pool. The fix (E3 LP-supply EXACT bind in verify_amm_conservation) binds
// new_total_lp to ACTUAL consumed LPShare inputs, closing the P5 drain.
// Drains reserve_a 1000->0 and reserve_b 2000->0 by burning a single share.
// =============================================================================
#[test]
fn malicious_lp_input_underburn_drain_rejected_when_gate_active() {
    let kp = crypto::KeyPair::from_seed([0x55; 32]);
    let user_pkh =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes());

    let asset_b = Hash::from_bytes([0xBB; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);

    // Old: 1000 DOLI, 2000 tokens, 1000 LP shares.
    let old_pool = Output::pool(pool_id, asset_b, 1000, 2000, 1000, 0, 100, 30, 100);
    // Attacker declares new_total_lp=0 (claims to burn all 1000) while only
    // consuming a 1-share LPShare input. Drains both reserves to ~0.
    let new_pool = Output::pool(pool_id, asset_b, 0, 0, 0, 0, 101, 30, 100);

    let doli_out = Output::normal(1000, user_pkh);
    let tokens_out = Output::fungible_asset(
        2000,
        user_pkh,
        asset_b,
        1_000_000,
        "TKN",
        &Condition::Signature(user_pkh),
    )
    .expect("fungible asset output must encode");

    // Attacker only owns ONE LP share.
    let old_lp = Output::lp_share(1, pool_id, user_pkh);

    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::RemoveLiquidity,
        inputs: vec![
            Input {
                prev_tx_hash: POOL_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
            Input {
                prev_tx_hash: LP_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
            Input {
                prev_tx_hash: FUND_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
        ],
        outputs: vec![new_pool, doli_out, tokens_out],
        extra_data: vec![],
    };

    sign_remove_liquidity(&mut tx, &kp);

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
        (LP_PREV, 0),
        UtxoInfo {
            output: old_lp,
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );
    utxos.utxos.insert(
        (FUND_PREV, 0),
        UtxoInfo {
            output: Output::normal(1000, user_pkh),
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );

    let ctx = ctx_at(10, 0, 0);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    assert!(
        res.is_err(),
        "T10 (INC-I-096 security): a RemoveLiquidity that declares new_total_lp=0 \
         while consuming only a 1-share LPShare input MUST be rejected (LP-input \
         binding). Otherwise the whole pool is drainable by burning 1 share. Got Ok."
    );
}

// =============================================================================
// T8 (security, token-inflation guard): a RemoveLiquidity that drops reserve_b by
// the PROPORTIONAL amount (passes the proportional reserve-delta binding) but pays
// the user MORE tokens than the reserve released MUST be rejected. There is no
// native conservation for FungibleAsset tokens, so the user-output binding
// (tokens_out <= reserve_b delta) is the ONLY guard against minting tokens from
// nothing. This test LOCKS that binding (must stay after the DOLI-side binding is
// removed for the fee-change fix).
// Burn 500/1000 shares: proportional max token delta = 1000. reserve_b drops 1000
// (legit), but tokens_out = 1500 (500 minted from nothing) -> reject.
// =============================================================================
#[test]
fn malicious_token_inflation_rejected_when_gate_active() {
    let kp = crypto::KeyPair::from_seed([0x55; 32]);
    let user_pkh =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes());

    let asset_b = Hash::from_bytes([0xBB; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);

    // Old: 1000 DOLI, 2000 tokens, 1000 LP. Burn 500 shares.
    let old_pool = Output::pool(pool_id, asset_b, 1000, 2000, 1000, 0, 100, 30, 100);
    // reserve_a drop 500 (proportional), reserve_b drop 1000 (proportional). Legit reserves.
    let new_pool = Output::pool(pool_id, asset_b, 500, 1000, 500, 0, 101, 30, 100);

    let doli_out = Output::normal(500, user_pkh);
    // INFLATED: claim 1500 tokens though reserve only released 1000.
    let tokens_out = Output::fungible_asset(
        1500,
        user_pkh,
        asset_b,
        1_000_000,
        "TKN",
        &Condition::Signature(user_pkh),
    )
    .expect("fungible asset output must encode");
    let fee_change = Output::normal(998, user_pkh);

    let old_lp = Output::lp_share(500, pool_id, user_pkh);

    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::RemoveLiquidity,
        inputs: vec![
            Input {
                prev_tx_hash: POOL_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
            Input {
                prev_tx_hash: LP_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
            Input {
                prev_tx_hash: FUND_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
        ],
        outputs: vec![new_pool, doli_out, tokens_out, fee_change],
        extra_data: vec![],
    };

    sign_remove_liquidity(&mut tx, &kp);

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
        (LP_PREV, 0),
        UtxoInfo {
            output: old_lp,
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );
    utxos.utxos.insert(
        (FUND_PREV, 0),
        UtxoInfo {
            output: Output::normal(1000, user_pkh),
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );

    let ctx = ctx_at(10, 0, 0);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    assert!(
        res.is_err(),
        "T8 (INC-I-096 security): a RemoveLiquidity that pays 1500 tokens when the \
         pool reserve only released 1000 MUST be rejected (token-inflation guard). Got Ok."
    );
}

// =============================================================================
// T9 (no regression, fee-change shape): a valid B->A Swap (tokens in, DOLI out)
// WITH a DOLI fee-change output must pass when the gate is active. This locks the
// fee-change fix: any B->A binding that sums all native outputs after the pool
// (swap DOLI out + fee change) and requires equality with the reserve_a delta
// would falsely reject this real-shape swap. Conservation + k-invariant are the
// correct (and sufficient) guards.
// old (1000 DOLI, 1000 tok, k=1e6); swap 100 tok in -> reserve_b=1100; reserve_a
// -> 910 (DOLI out from pool = 90); new_k = 910*1100 = 1,001,000 >= 1e6. ✓
// Outputs: new pool, 90 DOLI to user, 998 DOLI fee change.
// =============================================================================
#[test]
fn no_regression_swap_b_to_a_with_fee_change_passes_when_gate_active() {
    let kp = crypto::KeyPair::from_seed([0x42; 32]);
    let user_pkh =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes());

    let asset_b = Hash::from_bytes([0xBB; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);

    let old_pool = Output::pool(pool_id, asset_b, 1000, 1000, 707, 0, 100, 30, 100);
    let new_pool = Output::pool(pool_id, asset_b, 910, 1100, 707, 0, 101, 30, 100);
    let doli_out = Output::normal(90, user_pkh); // swap proceeds from reserve_a
    let fee_change = Output::normal(998, user_pkh); // 1000 fee input - 2 fee_units

    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Swap,
        inputs: vec![
            Input {
                prev_tx_hash: POOL_PREV,
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
            Input {
                prev_tx_hash: FUND_PREV,
                output_index: 0,
                signature: crypto::Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*kp.public_key()),
            },
        ],
        outputs: vec![new_pool, doli_out, fee_change],
        extra_data: vec![],
    };

    // input[1] = FungibleAsset token (conditioned) needs a covenant witness.
    {
        for i in 0..tx.inputs.len() {
            let sh = tx.signing_message_for_input(i);
            tx.inputs[i].signature = crypto::signature::sign_hash(&sh, kp.private_key());
        }
        let sh1 = tx.signing_message_for_input(1);
        let sig1 = crypto::signature::sign_hash(&sh1, kp.private_key());
        let token_witness = Witness {
            signatures: vec![WitnessSignature {
                pubkey: *kp.public_key(),
                signature: sig1,
            }],
            preimage: None,
            or_branches: vec![],
        };
        let witnesses: Vec<Vec<u8>> = vec![vec![], token_witness.encode(), vec![]];
        tx.set_covenant_witnesses(&witnesses);

        for i in 0..tx.inputs.len() {
            let sh = tx.signing_message_for_input(i);
            tx.inputs[i].signature = crypto::signature::sign_hash(&sh, kp.private_key());
        }
        let sh1 = tx.signing_message_for_input(1);
        let sig1 = crypto::signature::sign_hash(&sh1, kp.private_key());
        let token_witness = Witness {
            signatures: vec![WitnessSignature {
                pubkey: *kp.public_key(),
                signature: sig1,
            }],
            preimage: None,
            or_branches: vec![],
        };
        let witnesses: Vec<Vec<u8>> = vec![vec![], token_witness.encode(), vec![]];
        tx.set_covenant_witnesses(&witnesses);
    }

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
    let token_input = Output::fungible_asset(
        100,
        user_pkh,
        asset_b,
        1_000_000,
        "TKN",
        &Condition::Signature(user_pkh),
    )
    .expect("fungible asset output must encode");
    utxos.utxos.insert(
        (TOKEN_PREV, 0),
        UtxoInfo {
            output: token_input,
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );
    utxos.utxos.insert(
        (FUND_PREV, 0),
        UtxoInfo {
            output: Output::normal(1000, user_pkh),
            pubkey: Some(*kp.public_key()),
            spent: false,
        },
    );

    let ctx = ctx_at(10, 0, 0);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);
    assert!(
        res.is_ok(),
        "T9 (INC-I-096 no regression): a valid B->A swap WITH a DOLI fee-change \
         output must pass when the gate is active. Got: {:?}",
        res
    );
}

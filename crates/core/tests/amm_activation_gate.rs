// OUTPUT CONTRACT: fn validate_transaction(tx: &Transaction, ctx: &ValidationContext)
//   Outputs:
//     O1: returned Result<(), ValidationError> for the 4 AMM tx types
//         (CreatePool=19, AddLiquidity=20, RemoveLiquidity=21, Swap=22) against
//         ctx.amm_activation_height.
//   PATHS:
//     P1: tx.tx_type in AMM_TX_TYPES AND ctx.current_height < ctx.amm_activation_height
//         -> Err(ValidationError::AmmNotActivated { tx_type, activation_height, current_height })
//         with error_code == "AMM_NOT_ACTIVATED".
//     P2: tx.tx_type in AMM_TX_TYPES AND ctx.current_height >= ctx.amm_activation_height
//         -> MAY return Ok or a per-type validation Err, but NOT AmmNotActivated.
//   INPUT PARTITIONS:
//     P1: 4 partitions, one per AMM tx_type.
//     P2: 1 partition, boundary case (current_height == amm_activation_height).
//   MATRIX:
//     O1 x P1 x {4 AMM types}              -> 4 assertions (each Err AmmNotActivated)
//     O1 x P1 x {error_code}               -> 1 assertion (stable "AMM_NOT_ACTIVATED")
//     O1 x P1 x {to_structured_json}       -> 1 assertion (fields present)
//     O1 x P2 x {boundary}                 -> 1 assertion (NOT AmmNotActivated at == gate)
//
// History: Originally included O2/P3 independence tests verifying that
// non-AMM DeFi tx types (FractionalizeNft, RedeemNft) still hit
// DefiNotActivated when AMM gate is open. Those tx types were
// tombstoned in B.2 (DeFi L1 Foundations Architecture, 2026-05-26),
// making the independence tests moot. See tombstone_nft_frac_types.rs.

use crypto::Hash;
use doli_core::consensus::{ConsensusParams, GENESIS_TIME};
use doli_core::network::Network;
use doli_core::transaction::{Input, Output, Transaction, TxType};
use doli_core::validation::{self, ValidationContext, ValidationError};

// -----------------------------------------------------------------------
// Minimal valid-shape constructors for the 4 AMM tx types.
// -----------------------------------------------------------------------
fn create_pool_tx() -> Transaction {
    let asset_b = Hash::from_bytes([0xBB; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);
    let pool_output = Output::pool(pool_id, asset_b, 1000, 2000, 707, 0, 100, 30, 100);
    let lp_output = Output::lp_share(707, pool_id, Hash::from_bytes([0x01; 32]));
    Transaction {
        version: 1,
        tx_type: TxType::CreatePool,
        inputs: vec![Input::new(Hash::from_bytes([0xFF; 32]), 0)],
        outputs: vec![pool_output, lp_output],
        extra_data: vec![],
    }
}

fn add_liquidity_tx() -> Transaction {
    let asset_b = Hash::from_bytes([0xBB; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);
    let pool_output = Output::pool(pool_id, asset_b, 2000, 4000, 1414, 0, 100, 30, 100);
    let lp_output = Output::lp_share(707, pool_id, Hash::from_bytes([0x02; 32]));
    Transaction {
        version: 1,
        tx_type: TxType::AddLiquidity,
        inputs: vec![
            Input::new(Hash::from_bytes([0xF0; 32]), 0),
            Input::new(Hash::from_bytes([0xF1; 32]), 0),
        ],
        outputs: vec![pool_output, lp_output],
        extra_data: vec![],
    }
}

fn remove_liquidity_tx() -> Transaction {
    let asset_b = Hash::from_bytes([0xBB; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);
    let pool_output = Output::pool(pool_id, asset_b, 500, 1000, 353, 0, 100, 30, 100);
    Transaction {
        version: 1,
        tx_type: TxType::RemoveLiquidity,
        inputs: vec![
            Input::new(Hash::from_bytes([0xF0; 32]), 0),
            Input::new(Hash::from_bytes([0xF1; 32]), 0),
        ],
        outputs: vec![pool_output],
        extra_data: vec![],
    }
}

fn swap_tx() -> Transaction {
    let asset_b = Hash::from_bytes([0xBB; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);
    let pool_output = Output::pool(pool_id, asset_b, 1100, 1818, 707, 0, 100, 30, 100);
    let user_output = Output::normal(180, Hash::from_bytes([0x33; 32]));
    Transaction {
        version: 1,
        tx_type: TxType::Swap,
        inputs: vec![
            Input::new(Hash::from_bytes([0xF0; 32]), 0),
            Input::new(Hash::from_bytes([0xF1; 32]), 0),
        ],
        outputs: vec![pool_output, user_output],
        extra_data: vec![],
    }
}

type AmmTxCtor = (&'static str, fn() -> Transaction, u32);

const AMM_TX_CTORS: &[AmmTxCtor] = &[
    ("CreatePool", create_pool_tx, TxType::CreatePool as u32),
    (
        "AddLiquidity",
        add_liquidity_tx,
        TxType::AddLiquidity as u32,
    ),
    (
        "RemoveLiquidity",
        remove_liquidity_tx,
        TxType::RemoveLiquidity as u32,
    ),
    ("Swap", swap_tx, TxType::Swap as u32),
];

// -----------------------------------------------------------------------
// Context builders.
// -----------------------------------------------------------------------
fn pre_amm_activation_ctx() -> ValidationContext {
    ValidationContext::new(
        ConsensusParams::devnet(),
        Network::Devnet,
        GENESIS_TIME + 120,
        1,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_sig_verification_height(0)
}

fn post_amm_activation_ctx(amm_activation: u64, height: u64) -> ValidationContext {
    ValidationContext::new(
        ConsensusParams::devnet(),
        Network::Devnet,
        GENESIS_TIME + 10 * height,
        height,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_sig_verification_height(0)
    .with_amm_activation_height(amm_activation)
}

// ===================================================================
// O1 x P1 -- all 4 AMM tx types rejected pre-activation
// ===================================================================
#[test]
fn amm_tx_types_rejected_pre_activation() {
    let ctx = pre_amm_activation_ctx();
    assert_eq!(
        ctx.amm_activation_height,
        u64::MAX,
        "ValidationContext::new() must default amm_activation_height to u64::MAX"
    );

    for (name, ctor, tx_type_disc) in AMM_TX_CTORS {
        let tx = ctor();
        let res = validation::validate_transaction(&tx, &ctx);
        match res {
            Err(ValidationError::AmmNotActivated {
                tx_type,
                activation_height,
                current_height,
            }) => {
                assert_eq!(
                    tx_type, *tx_type_disc,
                    "{}: variant tx_type discriminant mismatch",
                    name
                );
                assert_eq!(
                    activation_height,
                    u64::MAX,
                    "{}: variant must echo current activation_height",
                    name
                );
                assert_eq!(
                    current_height, 1,
                    "{}: variant must echo ctx.current_height",
                    name
                );
            }
            other => panic!(
                "{}: expected Err(AmmNotActivated), got {:?}. \
                 Gate must fire BEFORE per-type validator runs.",
                name, other
            ),
        }
    }
}

// ===================================================================
// O1 x P1 -- stable machine-parseable error_code (REQ-AGENTIC-ERRORS).
// ===================================================================
#[test]
fn amm_not_activated_error_code_is_stable() {
    let ctx = pre_amm_activation_ctx();
    let tx = create_pool_tx();
    let err = validation::validate_transaction(&tx, &ctx).expect_err("must reject");
    assert_eq!(
        err.error_code(),
        "AMM_NOT_ACTIVATED",
        "error_code must be the stable string AMM_NOT_ACTIVATED for agentic consumers"
    );
}

// ===================================================================
// O1 x P1 -- structured JSON exposes all three fields.
// ===================================================================
#[test]
fn amm_not_activated_structured_json_exposes_fields() {
    let ctx = pre_amm_activation_ctx();
    let tx = swap_tx();
    let err = validation::validate_transaction(&tx, &ctx).expect_err("must reject");
    let json = err.to_structured_json();
    assert_eq!(json["error_code"], "AMM_NOT_ACTIVATED");
    assert!(json.get("tx_type").is_some(), "tx_type field required");
    assert!(
        json.get("activation_height").is_some(),
        "activation_height field required"
    );
    assert!(
        json.get("current_height").is_some(),
        "current_height field required"
    );
    // Swap = 22.
    assert_eq!(json["tx_type"], 22u32);
}

// ===================================================================
// O1 x P2 -- boundary: gate uses strict `<`, so height == gate is
// post-activation.
// ===================================================================
#[test]
fn amm_tx_types_pass_gate_at_activation_boundary() {
    let ctx = post_amm_activation_ctx(10, 10);
    let tx = create_pool_tx();
    let res = validation::validate_transaction(&tx, &ctx);
    if let Err(ValidationError::AmmNotActivated { .. }) = res {
        panic!("gate fired AT activation height -- comparison should be `<`, not `<=`")
    }
}

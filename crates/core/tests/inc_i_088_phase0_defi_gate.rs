// OUTPUT CONTRACT: fn validate_transaction(tx: &Transaction, ctx: &ValidationContext)
//   Outputs:
//     O1: returned Result<(), ValidationError> for the 2 remaining non-AMM
//         DeFi tx types (FractionalizeNft, RedeemNft) against
//         ctx.defi_activation_height.
//   PATHS:
//     P1: tx.tx_type in {FractionalizeNft, RedeemNft} AND
//         ctx.current_height < ctx.defi_activation_height
//         -> Err(ValidationError::DefiNotActivated { tx_type, activation_height, current_height })
//         with error_code == "DEFI_NOT_ACTIVATED"
//     P2: tx.tx_type in {FractionalizeNft, RedeemNft} AND
//         ctx.current_height >= ctx.defi_activation_height
//         -> MAY return Ok or a per-type validation Err, but NOT DefiNotActivated
//   INPUT PARTITIONS:
//     P1: one partition per remaining gated type (2 partitions):
//       FractionalizeNft, RedeemNft
//     P2: boundary case (current_height == defi_activation_height)
//   MATRIX:
//     O1 x P1 x {2 non-AMM DeFi tx types} -> 2 assertions (each Err DefiNotActivated)
//     O1 x P2 x {boundary}                -> 1 assertion  (NOT DefiNotActivated at == gate)
//     O1 x P1 x {error_code}              -> 1 assertion  (stable code string)
//     O1 x P1 x {to_structured_json}      -> 1 assertion  (fields present)
//
// History: Originally covered 7 non-AMM DeFi tx types (5 lending + 2 NFT-frac)
// and the INC-I-088 Collateral freeze guard. Lending types (24-28) and
// OutputType Collateral/LendingDeposit (11-12) were tombstoned in B.1
// (DeFi L1 Foundations Architecture, 2026-05-26). Tombstone regression
// tests are in `tombstone_lending_types.rs`.

use crypto::Hash;
use doli_core::consensus::{ConsensusParams, GENESIS_TIME};
use doli_core::network::Network;
use doli_core::transaction::{Input, Output, Transaction, TxType};
use doli_core::validation::{self, ValidationContext, ValidationError};

// ---------------------------------------------------------------------------
// Test context -- pre-activation by default (defi_activation_height = u64::MAX)
// ---------------------------------------------------------------------------
fn pre_activation_ctx() -> ValidationContext {
    ValidationContext::new(
        ConsensusParams::devnet(),
        Network::Devnet,
        GENESIS_TIME + 120,
        1,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_sig_verification_height(0)
    // defi_activation_height defaults to u64::MAX via ValidationContext::new()
}

fn post_activation_ctx(activation: u64, height: u64) -> ValidationContext {
    ValidationContext::new(
        ConsensusParams::devnet(),
        Network::Devnet,
        GENESIS_TIME + 10 * height,
        height,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_sig_verification_height(0)
    .with_defi_activation_height(activation)
}

// ---------------------------------------------------------------------------
// Remaining DeFi tx constructors (NFT-frac, still gated under defi_activation_height)
// ---------------------------------------------------------------------------

fn fractionalize_nft_tx() -> Transaction {
    Transaction {
        version: 1,
        tx_type: TxType::FractionalizeNft,
        inputs: vec![Input::new(Hash::from_bytes([0xAA; 32]), 0)],
        outputs: vec![
            Output::normal(1, Hash::from_bytes([0x01; 32])),
            Output::normal(1, Hash::from_bytes([0x02; 32])),
        ],
        extra_data: vec![],
    }
}

fn redeem_nft_tx() -> Transaction {
    Transaction {
        version: 1,
        tx_type: TxType::RedeemNft,
        inputs: vec![
            Input::new(Hash::from_bytes([0xAA; 32]), 0),
            Input::new(Hash::from_bytes([0xBB; 32]), 0),
        ],
        outputs: vec![Output::normal(1, Hash::from_bytes([0x01; 32]))],
        extra_data: vec![],
    }
}

type DefiTxCtor = (&'static str, fn() -> Transaction, u32);

const DEFI_TX_CTORS: &[DefiTxCtor] = &[
    // Lending types (24-28) tombstoned in B.1 (2026-05-26).
    // Tombstone regression: tombstone_lending_types.rs.
    (
        "FractionalizeNft",
        fractionalize_nft_tx,
        TxType::FractionalizeNft as u32,
    ),
    ("RedeemNft", redeem_nft_tx, TxType::RedeemNft as u32),
];

// ---------------------------------------------------------------------------
// O1 x P1 -- remaining 2 non-AMM DeFi tx types rejected pre-activation
// ---------------------------------------------------------------------------
#[test]
fn defi_tx_types_rejected_pre_activation() {
    let ctx = pre_activation_ctx();
    assert_eq!(
        ctx.defi_activation_height,
        u64::MAX,
        "ValidationContext::new() must default defi_activation_height to u64::MAX"
    );

    for (name, ctor, tx_type_disc) in DEFI_TX_CTORS {
        let tx = ctor();
        let res = validation::validate_transaction(&tx, &ctx);
        match res {
            Err(ValidationError::DefiNotActivated {
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
                "{}: expected Err(DefiNotActivated), got {:?}. \
                 Gate must fire BEFORE per-type validator runs.",
                name, other
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// O1 x P1 -- stable machine-parseable error_code
// ---------------------------------------------------------------------------
#[test]
fn defi_not_activated_error_code_is_stable() {
    let ctx = pre_activation_ctx();
    let tx = fractionalize_nft_tx();
    let err = validation::validate_transaction(&tx, &ctx).expect_err("must reject");
    assert_eq!(
        err.error_code(),
        "DEFI_NOT_ACTIVATED",
        "error_code must be the stable string DEFI_NOT_ACTIVATED for agentic consumers"
    );
}

// ---------------------------------------------------------------------------
// O1 x P1 -- structured JSON exposes all three fields
// ---------------------------------------------------------------------------
#[test]
fn defi_not_activated_structured_json_exposes_fields() {
    let ctx = pre_activation_ctx();
    let tx = fractionalize_nft_tx();
    let err = validation::validate_transaction(&tx, &ctx).expect_err("must reject");
    let json = err.to_structured_json();
    assert_eq!(json["error_code"], "DEFI_NOT_ACTIVATED");
    assert!(json.get("tx_type").is_some(), "tx_type field required");
    assert!(
        json.get("activation_height").is_some(),
        "activation_height field required"
    );
    assert!(
        json.get("current_height").is_some(),
        "current_height field required"
    );
    // FractionalizeNft = 29
    assert_eq!(json["tx_type"], 29u32);
}

// ---------------------------------------------------------------------------
// O1 x P2 -- boundary: gate uses strict `<`, so height == gate is post-activation
// ---------------------------------------------------------------------------
#[test]
fn defi_tx_types_pass_gate_at_activation_boundary() {
    let ctx = post_activation_ctx(10, 10);
    let tx = fractionalize_nft_tx();
    let res = validation::validate_transaction(&tx, &ctx);
    if let Err(ValidationError::DefiNotActivated { .. }) = res {
        panic!("gate fired AT activation height -- comparison should be `<`, not `<=`")
    }
}

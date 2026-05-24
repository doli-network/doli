// OUTPUT CONTRACT: fn validate_transaction(tx: &Transaction, ctx: &ValidationContext)
//   Outputs:
//     O1: returned Result<(), ValidationError> for the gated 11 DeFi tx types
//     O2: returned Result<(), ValidationError> for spending an OutputType::Collateral UTXO
//         with a single signature (via validate_transaction_with_utxos)
//   PATHS:
//     P1: tx.tx_type ∈ DEFI_TX_TYPES AND ctx.current_height < ctx.defi_activation_height
//         → Err(ValidationError::DefiNotActivated { tx_type, activation_height, current_height })
//         with error_code == "DEFI_NOT_ACTIVATED"
//     P2: tx.tx_type ∈ DEFI_TX_TYPES AND ctx.current_height >= ctx.defi_activation_height
//         → MAY return Ok or a per-type validation Err, but NOT DefiNotActivated
//     P3: input references OutputType::Collateral UTXO, tx attempts plain-signature spend
//         → Err(InvalidTransaction with prefix [ERRTX-DEFI001])
//         The deterministic hard-freeze in verify_input_conditions rejects
//         every Collateral spend regardless of the signature. The
//         is_conditioned() addition is secondary documentation; even with
//         the hard-freeze removed, decode_prefix on CollateralMetadata is
//         only probabilistically satisfiable, so the dual control is
//         required for a guaranteed freeze.
//   INPUT PARTITIONS:
//     For P1 — one partition per gated tx_type (11 partitions):
//       CreatePool, AddLiquidity, RemoveLiquidity, Swap, CreateLoan, RepayLoan,
//       LiquidateLoan, LendingDeposit, LendingWithdraw, FractionalizeNft, RedeemNft
//       Each is a distinct enum discriminant and distinct sub-validator;
//       a regression in any single arm of the gate match would slip past
//       partition-merged assertions.
//     For P2 — one partition (CreatePool boundary case): current_height ==
//       defi_activation_height. Verifies the gate uses `<` not `<=`.
//     For P3 — one partition (Collateral UTXO + valid Ed25519 signature over
//       signing_hash crafted to satisfy the pre-fix single-sig path). The
//       hard-freeze rejects it regardless.
//   MATRIX (outputs × paths × partitions):
//     O1 × P1 × {11 tx types}     → 11 assertions  (each Err DefiNotActivated)
//     O1 × P2 × {boundary}        → 1 assertion    (NOT DefiNotActivated at == gate)
//     O1 × P1 × {error_code}      → 1 assertion    (stable code string)
//     O1 × P1 × {to_structured_json}  → 1 assertion (fields present)
//     O2 × P3 × {plain-sig spend} → 1 assertion    (rejected with [ERRTX-DEFI001])
//
// Pre-fix expectation (TDD red phase, recorded for posterity): without the
// gate and without the hard-freeze, four tests FAIL (the gate never fires,
// and Collateral spend returns InvalidSignature instead of the structured
// hard-freeze code). With both fixes applied, all five PASS.

use crypto::{Hash, KeyPair, Signature};
use doli_core::consensus::{ConsensusParams, GENESIS_TIME};
use doli_core::network::Network;
use doli_core::transaction::{
    Input, Output, OutputType, SighashType, Transaction, TxType, COLLATERAL_METADATA_SIZE,
};
use doli_core::validation::{self, UtxoInfo, UtxoProvider, ValidationContext, ValidationError};
use std::collections::HashMap;

// ───────────────────────────────────────────────────────────────────────────
// Mock UTXO provider — minimal, lifted from crates/core/tests/p0001_exploit.rs
// ───────────────────────────────────────────────────────────────────────────
struct MockUtxos {
    utxos: HashMap<(Hash, u32), UtxoInfo>,
}

impl UtxoProvider for MockUtxos {
    fn get_utxo(&self, tx_hash: &Hash, index: u32) -> Option<UtxoInfo> {
        self.utxos.get(&(*tx_hash, index)).cloned()
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Test context — pre-activation by default (defi_activation_height = u64::MAX)
// ───────────────────────────────────────────────────────────────────────────
fn pre_activation_ctx() -> ValidationContext {
    // current_height = 1 < u64::MAX, so gate fires for any DeFi tx.
    // sig_verification_height = 0 so Collateral spend signature path is
    // exercised (relevant for the freeze test).
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

// ───────────────────────────────────────────────────────────────────────────
// Minimal valid-shape constructors for each DeFi tx type.
//
// These produce txs whose per-type STRUCTURAL validators would accept them
// (when the gate is open). Pre-activation the gate fires before any
// per-type validator runs, so any structural shortcut survives.
// ───────────────────────────────────────────────────────────────────────────
fn create_pool_tx() -> Transaction {
    let asset_b = Hash::from_bytes([0xBB; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b);
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
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b);
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
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b);
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
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b);
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

fn create_loan_tx() -> Transaction {
    let pool_id = Hash::from_bytes([0xAA; 32]);
    let borrower = Hash::from_bytes([0xBB; 32]);
    let asset_id = Hash::from_bytes([0xCC; 32]);
    let collateral = Output::collateral(500, pool_id, borrower, 100, 500, 42, 15000, asset_id);
    let borrow = Output::normal(100, borrower);
    Transaction {
        version: 1,
        tx_type: TxType::CreateLoan,
        inputs: vec![Input::new(Hash::from_bytes([0xFF; 32]), 0)],
        outputs: vec![collateral, borrow],
        extra_data: vec![],
    }
}

fn repay_loan_tx() -> Transaction {
    Transaction {
        version: 1,
        tx_type: TxType::RepayLoan,
        inputs: vec![
            Input::new(Hash::from_bytes([0xAA; 32]), 0),
            Input::new(Hash::from_bytes([0xBB; 32]), 0),
        ],
        outputs: vec![Output::normal(500, Hash::from_bytes([0xCC; 32]))],
        extra_data: vec![],
    }
}

fn liquidate_loan_tx() -> Transaction {
    Transaction {
        version: 1,
        tx_type: TxType::LiquidateLoan,
        inputs: vec![Input::new(Hash::from_bytes([0xAA; 32]), 0)],
        outputs: vec![Output::normal(500, Hash::from_bytes([0xCC; 32]))],
        extra_data: vec![],
    }
}

fn lending_deposit_tx() -> Transaction {
    let pool_id = Hash::from_bytes([0xDD; 32]);
    let depositor = Hash::from_bytes([0xEE; 32]);
    Transaction {
        version: 1,
        tx_type: TxType::LendingDeposit,
        inputs: vec![Input::new(Hash::from_bytes([0xFF; 32]), 0)],
        outputs: vec![Output::lending_deposit(1000, pool_id, depositor, 50)],
        extra_data: vec![],
    }
}

fn lending_withdraw_tx() -> Transaction {
    Transaction {
        version: 1,
        tx_type: TxType::LendingWithdraw,
        inputs: vec![Input::new(Hash::from_bytes([0xAA; 32]), 0)],
        outputs: vec![Output::normal(500, Hash::from_bytes([0xCC; 32]))],
        extra_data: vec![],
    }
}

fn fractionalize_nft_tx() -> Transaction {
    // We don't need this tx to pass the per-type validator (pre-activation the
    // gate fires first). Provide minimal-shape inputs/outputs to clear basic
    // structural rules in validate_transaction.
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
    ("CreateLoan", create_loan_tx, TxType::CreateLoan as u32),
    ("RepayLoan", repay_loan_tx, TxType::RepayLoan as u32),
    (
        "LiquidateLoan",
        liquidate_loan_tx,
        TxType::LiquidateLoan as u32,
    ),
    (
        "LendingDeposit",
        lending_deposit_tx,
        TxType::LendingDeposit as u32,
    ),
    (
        "LendingWithdraw",
        lending_withdraw_tx,
        TxType::LendingWithdraw as u32,
    ),
    (
        "FractionalizeNft",
        fractionalize_nft_tx,
        TxType::FractionalizeNft as u32,
    ),
    ("RedeemNft", redeem_nft_tx, TxType::RedeemNft as u32),
];

// ───────────────────────────────────────────────────────────────────────────
// O1 × P1 — all 11 DeFi tx types rejected pre-activation with DefiNotActivated
// (one assertion per type so a regression in one match arm cannot hide)
// ───────────────────────────────────────────────────────────────────────────
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

// ───────────────────────────────────────────────────────────────────────────
// O1 × P1 — stable machine-parseable error_code (REQ-AGENTIC-ERRORS)
// ───────────────────────────────────────────────────────────────────────────
#[test]
fn defi_not_activated_error_code_is_stable() {
    let ctx = pre_activation_ctx();
    let tx = create_pool_tx();
    let err = validation::validate_transaction(&tx, &ctx).expect_err("must reject");
    assert_eq!(
        err.error_code(),
        "DEFI_NOT_ACTIVATED",
        "error_code must be the stable string DEFI_NOT_ACTIVATED for agentic consumers"
    );
}

// ───────────────────────────────────────────────────────────────────────────
// O1 × P1 — structured JSON exposes all three fields
// ───────────────────────────────────────────────────────────────────────────
#[test]
fn defi_not_activated_structured_json_exposes_fields() {
    let ctx = pre_activation_ctx();
    let tx = swap_tx();
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
    // Specifically: Swap = 22
    assert_eq!(json["tx_type"], 22u32);
}

// ───────────────────────────────────────────────────────────────────────────
// O1 × P2 — boundary: gate uses strict `<`, so height == gate is post-activation
// (rejection here would mean an off-by-one in the comparison)
// ───────────────────────────────────────────────────────────────────────────
#[test]
fn defi_tx_types_pass_gate_at_activation_boundary() {
    // Set activation at exactly current_height. Gate is `<` so this must NOT
    // return DefiNotActivated. The per-type validator may still return its own
    // structural error (CreatePool here is valid) — that is allowed.
    let ctx = post_activation_ctx(10, 10);
    let tx = create_pool_tx();
    let res = validation::validate_transaction(&tx, &ctx);
    // Any other Ok/Err is acceptable here: the gate let the tx through and the
    // per-type validator's verdict is not under test. We only fail if the
    // DeFi gate fires AT the activation boundary (off-by-one in comparison).
    if let Err(ValidationError::DefiNotActivated { .. }) = res {
        panic!("gate fired AT activation height — comparison should be `<`, not `<=`")
    }
}

// ───────────────────────────────────────────────────────────────────────────
// O2 × P3 — Collateral UTXO hard-frozen.
//
// Pre-fix: Collateral is not handled in verify_input_conditions; spend goes
// through single-sig path; with a malicious CreateLoan that put the
// borrower's pubkey_hash in the Collateral output, the borrower's
// signature would succeed.
//
// Post-fix: verify_input_conditions checks OutputType::Collateral FIRST
// and returns [ERRTX-DEFI001] unconditionally (the lending subsystem is
// frozen until properly un-gated). The is_conditioned() addition is
// secondary documentation — even if the early-return is removed, the
// condition-path probabilistically rejects most metadata patterns.
// ───────────────────────────────────────────────────────────────────────────
#[test]
fn collateral_utxo_unspendable_with_plain_signature() {
    // Borrower owns a Collateral UTXO. The exploit scenario assumes a
    // malicious / unchecked CreateLoan that put the borrower's own
    // pubkey_hash in outputs[0] (validate_create_loan does NOT currently
    // enforce pubkey_hash = derived loan_addr).
    let borrower_kp = KeyPair::from_seed([0x42; 32]);
    let borrower_pkh =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, borrower_kp.public_key().as_bytes());

    // Construct a Collateral output with borrower's pubkey_hash (the
    // exploitable shape). Build the extra_data manually to match
    // COLLATERAL_METADATA_SIZE.
    let mut extra_data = vec![0u8; COLLATERAL_METADATA_SIZE];
    extra_data[0] = 1; // COLLATERAL_VERSION
    let collateral_utxo = Output {
        output_type: OutputType::Collateral,
        amount: 1_000_000_000,
        pubkey_hash: borrower_pkh, // EXPLOITABLE: borrower controls this UTXO
        lock_until: 0,
        extra_data,
    };

    let prev_hash = Hash::from_bytes([0x55; 32]);
    let mut utxos = MockUtxos {
        utxos: HashMap::new(),
    };
    utxos.utxos.insert(
        (prev_hash, 0),
        UtxoInfo {
            output: collateral_utxo,
            pubkey: Some(*borrower_kp.public_key()),
            spent: false,
        },
    );

    // Borrower attempts to drain the collateral via a plain Transfer.
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input {
            prev_tx_hash: prev_hash,
            output_index: 0,
            signature: Signature::from_bytes([0u8; 64]),
            sighash_type: SighashType::All,
            committed_output_count: 0,
            public_key: Some(*borrower_kp.public_key()),
        }],
        outputs: vec![Output {
            output_type: OutputType::Normal,
            amount: 900_000_000,
            pubkey_hash: borrower_pkh,
            lock_until: 0,
            extra_data: vec![],
        }],
        extra_data: vec![],
    };

    // Sign with the borrower's real key — Normal/Bond path would accept this.
    let sig = crypto::signature::sign_hash(&tx.signing_message(), borrower_kp.private_key());
    tx.inputs[0].signature = sig;

    let ctx = pre_activation_ctx();
    let res = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);

    match res {
        Err(ValidationError::InvalidTransaction(msg)) => {
            assert!(
                msg.contains("[ERRTX-DEFI001]"),
                "Collateral spend must be hard-frozen via [ERRTX-DEFI001] \
                 (INC-I-088 Phase 0), got: {}",
                msg
            );
        }
        other => panic!(
            "Collateral UTXO must be unspendable. Expected \
             Err(InvalidTransaction with [ERRTX-DEFI001]), got: {:?}",
            other
        ),
    }
}

// OUTPUT CONTRACT: fn validate_transaction_with_utxos(tx, ctx, utxo_provider)
//   Outputs:
//     O1: Result<(), ValidationError> for MintAsset tx where input[0] is NOT the
//         genesis UTXO (non-issuer attempts mint with a transferred FungibleAsset UTXO)
//     O2: Result<(), ValidationError> for MintAsset tx where input[0] IS the genesis
//         UTXO and the signer is the original issuer (legitimate mint)
//   PATHS:
//     P1: input[0].outpoint does NOT derive to the asset_id via compute_asset_id
//         → Err(InvalidMintAsset("only the original issuer can mint"))
//     P2: input[0].outpoint DOES derive to the asset_id (genesis UTXO, issuer signs)
//         → Ok(()) or other non-issuer-related error
//   INPUT PARTITIONS:
//     P1: non-issuer holds transferred FungibleAsset UTXO, uses it as input[0]
//     P2: issuer holds genesis FungibleAsset UTXO, uses it as input[0]
//   MATRIX:
//     O1 × P1 → 1 assertion (non-issuer mint REJECTED)
//     O2 × P2 → 1 assertion (issuer mint ACCEPTED at UTXO level)
//
// Pre-fix expectation (TDD red phase): without the issuer authorization fix,
// the non-issuer mint SUCCEEDS (bug confirmed) because genesis_utxo.pubkey is
// always None, making the `if let Some(ref pk)` block unreachable.
// Post-fix: non-issuer mint rejected with InvalidMintAsset.

use crypto::{Hash, KeyPair, Signature};
use doli_core::conditions::{Condition, Witness, WitnessSignature};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::transaction::{Input, Output, SighashType, Transaction, TxType};
use doli_core::validation::{self, UtxoInfo, UtxoProvider, ValidationContext, ValidationError};
use std::collections::HashMap;

// ───────────────────────────────────────────────────────────────────────────
// Mock UTXO provider
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
// Helpers
// ───────────────────────────────────────────────────────────────────────────

fn pubkey_hash_of(kp: &KeyPair) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes())
}

/// Create a FungibleAsset output with Condition::Signature(owner_pkh).
fn make_fungible_asset_output(
    amount: u64,
    owner_pkh: Hash,
    asset_id: Hash,
    total_supply: u64,
    ticker: &str,
) -> Output {
    Output::fungible_asset(
        amount,
        owner_pkh,
        asset_id,
        total_supply,
        ticker,
        &Condition::signature(owner_pkh),
    )
    .expect("fungible_asset output creation should succeed")
}

/// Build a witness encoding a single signature for condition evaluation.
fn build_witness(kp: &KeyPair, signing_hash: &Hash) -> Vec<u8> {
    let sig = crypto::signature::sign_hash(signing_hash, kp.private_key());
    let witness = Witness {
        signatures: vec![WitnessSignature {
            pubkey: *kp.public_key(),
            signature: sig,
        }],
        preimage: None,
        or_branches: vec![],
    };
    witness.encode()
}

/// Validation context with covenants activated (height >= 2000 for Mainnet)
/// and signature verification enforced from genesis.
fn test_ctx() -> ValidationContext {
    // Height 5000 is well past the Mainnet covenants activation height (2000).
    ValidationContext::new(
        ConsensusParams::mainnet(),
        Network::Mainnet,
        1_774_921_461 + 100_000,
        5000,
    )
    .with_prev_block(0, 1_774_921_461, Hash::from_bytes([0xBB; 32]))
    .with_sig_verification_height(0)
}

// ───────────────────────────────────────────────────────────────────────────
// TEST 1: Non-issuer MintAsset MUST be rejected
//
// Setup:
//   - Issuer (seed [1]) created the genesis FungibleAsset at outpoint (0xAA, 0)
//   - User B (seed [2]) received some tokens at outpoint (0xCC, 0)
//   - User B submits a MintAsset tx with input[0] = their own UTXO (0xCC, 0)
//   - The tx is properly signed by B (valid spender of that UTXO)
//
// Expected: REJECTED -- B is not the issuer
// Pre-fix: PASSES (bug -- issuer check is dead code)
// ───────────────────────────────────────────────────────────────────────────
#[test]
fn non_issuer_mint_asset_must_be_rejected() {
    let issuer_kp = KeyPair::from_seed([1u8; 32]);
    let issuer_pkh = pubkey_hash_of(&issuer_kp);

    let user_b_kp = KeyPair::from_seed([2u8; 32]);
    let user_b_pkh = pubkey_hash_of(&user_b_kp);

    // Genesis UTXO: created by issuer at outpoint (genesis_tx_hash=0xAA, index=0)
    let genesis_tx_hash = Hash::from_bytes([0xAA; 32]);
    let asset_id = Output::compute_asset_id(&genesis_tx_hash, 0);
    let total_supply: u64 = 1_000_000;

    let genesis_output =
        make_fungible_asset_output(total_supply, issuer_pkh, asset_id, total_supply, "TEST");

    // User B's UTXO: received 100 tokens from issuer at outpoint (0xCC, 0)
    let user_b_tx_hash = Hash::from_bytes([0xCC; 32]);
    let user_b_output = make_fungible_asset_output(100, user_b_pkh, asset_id, total_supply, "TEST");

    // Build MintAsset tx: user B uses their UTXO as input[0]
    let mint_output = make_fungible_asset_output(200, user_b_pkh, asset_id, total_supply, "TEST");

    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::MintAsset,
        inputs: vec![Input {
            prev_tx_hash: user_b_tx_hash,
            output_index: 0,
            signature: Signature::from_bytes([0u8; 64]), // placeholder
            sighash_type: SighashType::All,
            committed_output_count: 0,
            public_key: Some(*user_b_kp.public_key()),
        }],
        outputs: vec![mint_output],
        extra_data: vec![],
    };

    // Sign with user B's key (valid spender of user_b_tx_hash:0)
    let signing_hash = tx.signing_message_for_input(0);
    let sig = crypto::signature::sign_hash(&signing_hash, user_b_kp.private_key());
    tx.inputs[0].signature = sig;

    // Set covenant witness for the conditioned FungibleAsset spend
    let witness_bytes = build_witness(&user_b_kp, &signing_hash);
    tx.set_covenant_witnesses(&[witness_bytes]);

    // UTXO provider: both genesis and user_b UTXOs exist
    let mut utxos = MockUtxos {
        utxos: HashMap::new(),
    };
    utxos.utxos.insert(
        (genesis_tx_hash, 0),
        UtxoInfo {
            output: genesis_output,
            pubkey: None, // production: always None
            spent: false,
        },
    );
    utxos.utxos.insert(
        (user_b_tx_hash, 0),
        UtxoInfo {
            output: user_b_output,
            pubkey: None,
            spent: false,
        },
    );

    let ctx = test_ctx();
    let result = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);

    // MUST be rejected: user B is not the issuer.
    // Pre-fix: this assertion FAILS because the issuer check is dead code
    // (genesis_utxo.pubkey is always None, so the if-let never executes).
    // The non-issuer mint is silently accepted.
    assert!(
        result.is_err(),
        "BUG CONFIRMED: non-issuer MintAsset was ACCEPTED. \
         The issuer authorization check is dead code (pubkey: None bypass)."
    );

    // Verify the error is specifically about issuer mismatch (not some other error)
    match &result {
        Err(ValidationError::InvalidMintAsset(msg)) => {
            assert!(
                msg.contains("issuer"),
                "Expected issuer-related error, got: {}",
                msg
            );
        }
        Err(other) => {
            panic!(
                "Expected InvalidMintAsset with issuer error, got different error: {}",
                other
            );
        }
        Ok(()) => unreachable!(),
    }
}

// ───────────────────────────────────────────────────────────────────────────
// TEST 2: Legitimate issuer MintAsset MUST be accepted (at UTXO level)
//
// Setup:
//   - Issuer (seed [1]) created the genesis FungibleAsset at outpoint (0xAA, 0)
//   - Issuer submits a MintAsset tx with input[0] = genesis UTXO (0xAA, 0)
//
// Expected: ACCEPTED -- issuer is authorized
// ───────────────────────────────────────────────────────────────────────────
#[test]
fn issuer_mint_asset_must_be_accepted() {
    let issuer_kp = KeyPair::from_seed([1u8; 32]);
    let issuer_pkh = pubkey_hash_of(&issuer_kp);

    let genesis_tx_hash = Hash::from_bytes([0xAA; 32]);
    let asset_id = Output::compute_asset_id(&genesis_tx_hash, 0);
    let total_supply: u64 = 1_000_000;

    let genesis_output =
        make_fungible_asset_output(total_supply, issuer_pkh, asset_id, total_supply, "TEST");

    // Mint: issuer creates more tokens (within supply cap)
    let mint_output =
        make_fungible_asset_output(total_supply, issuer_pkh, asset_id, total_supply, "TEST");

    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::MintAsset,
        inputs: vec![Input {
            prev_tx_hash: genesis_tx_hash,
            output_index: 0,
            signature: Signature::from_bytes([0u8; 64]),
            sighash_type: SighashType::All,
            committed_output_count: 0,
            public_key: Some(*issuer_kp.public_key()),
        }],
        outputs: vec![mint_output],
        extra_data: vec![],
    };

    // Sign with issuer's key
    let signing_hash = tx.signing_message_for_input(0);
    let sig = crypto::signature::sign_hash(&signing_hash, issuer_kp.private_key());
    tx.inputs[0].signature = sig;

    // Set covenant witness
    let witness_bytes = build_witness(&issuer_kp, &signing_hash);
    tx.set_covenant_witnesses(&[witness_bytes]);

    // UTXO provider
    let mut utxos = MockUtxos {
        utxos: HashMap::new(),
    };
    utxos.utxos.insert(
        (genesis_tx_hash, 0),
        UtxoInfo {
            output: genesis_output,
            pubkey: None,
            spent: false,
        },
    );

    let ctx = test_ctx();
    let result = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);

    assert!(
        result.is_ok(),
        "Legitimate issuer MintAsset should be accepted, got: {:?}",
        result
    );
}

// ───────────────────────────────────────────────────────────────────────────
// TEST 3: Multi-input MintAsset where input[0] is genesis but input[1+] are
//         from the same issuer -- should be accepted (issuer consolidates)
// ───────────────────────────────────────────────────────────────────────────
#[test]
fn issuer_multi_input_mint_asset_accepted() {
    let issuer_kp = KeyPair::from_seed([1u8; 32]);
    let issuer_pkh = pubkey_hash_of(&issuer_kp);

    let genesis_tx_hash = Hash::from_bytes([0xAA; 32]);
    let asset_id = Output::compute_asset_id(&genesis_tx_hash, 0);
    let total_supply: u64 = 1_000_000;

    // Genesis UTXO owned by issuer
    let genesis_output =
        make_fungible_asset_output(total_supply, issuer_pkh, asset_id, total_supply, "TEST");

    // Second UTXO also owned by issuer
    let second_tx_hash = Hash::from_bytes([0xDD; 32]);
    let second_output = make_fungible_asset_output(500, issuer_pkh, asset_id, total_supply, "TEST");

    // Mint output
    let mint_output =
        make_fungible_asset_output(total_supply, issuer_pkh, asset_id, total_supply, "TEST");

    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::MintAsset,
        inputs: vec![
            Input {
                prev_tx_hash: genesis_tx_hash,
                output_index: 0,
                signature: Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*issuer_kp.public_key()),
            },
            Input {
                prev_tx_hash: second_tx_hash,
                output_index: 0,
                signature: Signature::from_bytes([0u8; 64]),
                sighash_type: SighashType::All,
                committed_output_count: 0,
                public_key: Some(*issuer_kp.public_key()),
            },
        ],
        outputs: vec![mint_output],
        extra_data: vec![],
    };

    // Sign each input
    let signing_hash_0 = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&signing_hash_0, issuer_kp.private_key());
    let witness_0 = build_witness(&issuer_kp, &signing_hash_0);

    let signing_hash_1 = tx.signing_message_for_input(1);
    tx.inputs[1].signature = crypto::signature::sign_hash(&signing_hash_1, issuer_kp.private_key());
    let witness_1 = build_witness(&issuer_kp, &signing_hash_1);

    tx.set_covenant_witnesses(&[witness_0, witness_1]);

    let mut utxos = MockUtxos {
        utxos: HashMap::new(),
    };
    utxos.utxos.insert(
        (genesis_tx_hash, 0),
        UtxoInfo {
            output: genesis_output,
            pubkey: None,
            spent: false,
        },
    );
    utxos.utxos.insert(
        (second_tx_hash, 0),
        UtxoInfo {
            output: second_output,
            pubkey: None,
            spent: false,
        },
    );

    let ctx = test_ctx();
    let result = validation::validate_transaction_with_utxos(&tx, &ctx, &utxos);

    assert!(
        result.is_ok(),
        "Issuer multi-input MintAsset should be accepted, got: {:?}",
        result
    );
}

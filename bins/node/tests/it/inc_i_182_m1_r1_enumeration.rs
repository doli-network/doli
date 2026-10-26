//! INC-I-182 M1 — R-1 enumeration: every tx type is classified by how it binds
//! its signer to the producer it changes, and each binding other than the new
//! producer signature is shown to refuse a third party today.
//!
// covers: producer_auth transaction types unbonding defaults env_loader network_params pool assembly tx_processing validation_checks withdrawal_holdings genesis register bonds withdrawal exit unbonding_tx core data block rpc_client
//
// OUTPUT CONTRACT: fn validate_transaction_with_utxos(&Transaction, &ValidationContext, &UtxoSet) -> Result<..>
//   O2 return for Exit per height and zero-flow gate
// OUTPUT CONTRACT: fn Node::handle_new_transaction(&self, Transaction) -> bool    O3 admission (Exit, delegation)
// OUTPUT CONTRACT: fn evidence_header_admissible(&BlockHeader, &Hash, Network) -> Result<(), ValidationError>   O4
// OUTPUT CONTRACT: fn rotate_stateless(&Transaction, &ValidationContext) -> Result<..>   O5
// PATHS: P-EXIT | P-SLASH | P-DELEG | P-ROTATE (P-AUTH and P-OTHER: inc_i_182_m1_r1_scope)
// MATRIX
//   O2, O3 x P-EXIT x heights {0, 1, 1_000, u64::MAX - 1} x zero-flow gate {0, MAX} -> refused
//     (inc_i_182_m1_req_pauth_001_r1_exit_unmineable_at_every_height)
//   O4 x P-SLASH x {accused-signed, signed by another key} -> Ok / signature refusal
//     (inc_i_182_m1_req_pauth_001_r1_slash_bound_by_accused_headers)
//   O3 x P-DELEG x {genuine, wrong-key} x {DelegateBond, RevokeDelegation} -> admitted / refused
//     (inc_i_182_m1_req_pauth_001_r1_delegation_bound_by_delegator_signature)
//   O5 x P-ROTATE x {input key of another, producer key with a bad signature} -> mismatch / bad signature
//     (inc_i_182_m1_req_pauth_001_r1_rotation_bound_by_input_key_and_signature)
// INPUT PARTITIONS:
//   signer: the bound party | another key; Exit height: genesis | early | late | near u64::MAX

use crypto::{BlsKeyPair, Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{Input, Output, RotateBlsData, Transaction, TxType};
use doli_core::validation::{
    evidence_header_admissible, rotate_stateless, validate_transaction_with_utxos,
    ValidationContext, ValidationError,
};
use doli_core::{Network, NetworkParams};
use storage::UtxoSet;

use super::inc_i_237_m8_common::{self as m8, Parties, Sig};
use super::inc_i_255_m1_common::{chain_genesis, evidence_header, world, SLOT};
use crate::block_sig::sign_header;

#[derive(Debug, PartialEq, Eq)]
pub enum Binding {
    ProducerSignature,
    Unmineable,
    AccusedHeaders,
    DelegatorSignature,
    RotationKeyAndSignature,
    NotProducerChanging,
}

/// Exhaustive on purpose: a new TxType must be classified here before this compiles.
pub fn binding(t: TxType) -> Binding {
    match t {
        TxType::Registration | TxType::AddBond | TxType::RequestWithdrawal => {
            Binding::ProducerSignature
        }
        TxType::Exit => Binding::Unmineable,
        TxType::SlashProducer => Binding::AccusedHeaders,
        TxType::DelegateBond | TxType::RevokeDelegation => Binding::DelegatorSignature,
        TxType::RotateBlsKey => Binding::RotationKeyAndSignature,
        TxType::Transfer
        | TxType::ClaimReward
        | TxType::ClaimBond
        | TxType::Coinbase
        | TxType::ClaimWithdrawal
        | TxType::EpochReward
        | TxType::RemoveMaintainer
        | TxType::AddMaintainer
        | TxType::ProtocolActivation
        | TxType::PriceAttestation
        | TxType::MintAsset
        | TxType::BurnAsset
        | TxType::CreatePool
        | TxType::AddLiquidity
        | TxType::RemoveLiquidity
        | TxType::Swap
        | TxType::ZKSettle => Binding::NotProducerChanging,
    }
}

fn ctx(height: u64) -> ValidationContext {
    ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, height)
}

// REQ-PAUTH-001 — Decision: a failure means Exit (a bare public key, no signature) became mineable, so any fee payer can exit any producer and Exit needs the producer-auth rule too.
#[tokio::test]
async fn inc_i_182_m1_req_pauth_001_r1_exit_unmineable_at_every_height() {
    let w = world().await;
    let exit = Transaction::new_exit(w.accused_pk());
    assert!(
        exit.inputs.is_empty() && exit.outputs.is_empty(),
        "Exit carries no inputs and no outputs"
    );
    let utxo = UtxoSet::new();
    for h in [0, 1, 1_000, u64::MAX - 1] {
        for zero_flow_gate in [0, u64::MAX] {
            let c = ctx(h).with_inc_i_173_activation_height(zero_flow_gate);
            let r = validate_transaction_with_utxos(&exit, &c, &utxo);
            assert!(
                r.is_err(),
                "Exit valid at {h} (zero-flow gate {zero_flow_gate})"
            );
        }
    }
    assert!(
        !w.node.handle_new_transaction(exit).await,
        "the node admits a bare Exit"
    );
}

// REQ-PAUTH-001 — Decision: a failure means slash evidence that the accused never signed is admissible, so anyone can burn a producer's bonds.
#[test]
fn inc_i_182_m1_req_pauth_001_r1_slash_bound_by_accused_headers() {
    let g = chain_genesis();
    let (accused, stranger) = (KeyPair::generate(), KeyPair::generate());
    let honest = evidence_header(&accused, g, SLOT, 0xA1, 0);
    let ok = evidence_header_admissible(&honest, &g, Network::Devnet);
    assert!(
        ok.is_ok(),
        "NON-VACUITY: the accused's own header is admissible: {ok:?}"
    );
    let mut forged = evidence_header(&stranger, g, SLOT, 0xA1, 0);
    forged.producer = *accused.public_key();
    sign_header(&mut forged, &stranger);
    let r = evidence_header_admissible(&forged, &g, Network::Devnet);
    assert!(
        matches!(&r, Err(ValidationError::InvalidSlash(m)) if m.contains("signature")),
        "a header naming the accused but signed by another key must be refused for its signature: {r:?}"
    );
}

// REQ-PAUTH-001 — Decision: a failure means a delegation or revocation signed by someone other than the delegator is admitted where delegation auth is armed, so a third party can move a producer's weight.
#[tokio::test]
async fn inc_i_182_m1_req_pauth_001_r1_delegation_bound_by_delegator_signature() {
    for network in [Network::Mainnet, Network::Testnet] {
        assert_eq!(
            NetworkParams::defaults(network).delegation_auth_activation_height,
            0,
            "{network:?}: delegation auth armed from genesis"
        );
    }
    let (node, _kp, _m, _t) = m8::auth_node().await;
    let forged_delegate =
        m8::delegate_bond(&Parties::registered(&[&node], false).await, Sig::Wrong);
    assert!(
        !node.handle_new_transaction(forged_delegate).await,
        "wrong-key DelegateBond admitted"
    );
    let forged_revoke = m8::revoke(&Parties::registered(&[&node], true).await, Sig::Wrong);
    assert!(
        !node.handle_new_transaction(forged_revoke).await,
        "wrong-key RevokeDelegation admitted"
    );
    let genuine = m8::genuine_delegate_bond(&[&node]).await;
    assert!(
        node.handle_new_transaction(genuine).await,
        "NON-VACUITY: genuine DelegateBond admitted"
    );
    let genuine = m8::genuine_revoke(&[&node]).await;
    assert!(
        node.handle_new_transaction(genuine).await,
        "NON-VACUITY: genuine RevokeDelegation admitted"
    );
}

fn rotation(producer: &KeyPair, revealed: &KeyPair) -> Transaction {
    let bls = BlsKeyPair::from_seed(&[0x5C; 32]).expect("FIXTURE: BLS seed");
    let mut new_bls_pubkey = [0u8; 48];
    new_bls_pubkey.copy_from_slice(bls.public_key().as_bytes());
    let payload = RotateBlsData {
        producer: *producer.public_key().as_bytes(),
        new_bls_pubkey,
        bls_pop: [0u8; 96],
        signature: [0u8; 64],
    };
    let mut input = Input::new(Hash::from_bytes([0x77; 32]), 0);
    input.public_key = Some(*revealed.public_key());
    Transaction {
        version: 1,
        tx_type: TxType::RotateBlsKey,
        inputs: vec![input],
        outputs: vec![Output::normal(1_000, Hash::from_bytes([0x78; 32]))],
        extra_data: payload.encode().to_vec(),
    }
}

// REQ-PAUTH-001 — Decision: a failure means a BLS rotation is accepted from an input that does not reveal the producer key, or without the producer's signature, so a third party can replace a producer's BLS key.
#[test]
fn inc_i_182_m1_req_pauth_001_r1_rotation_bound_by_input_key_and_signature() {
    let (producer, stranger) = (KeyPair::generate(), KeyPair::generate());
    let c = ctx(1_000).with_bls_key_rotation_activation_height(0);
    let r = rotate_stateless(&rotation(&producer, &stranger), &c);
    assert!(
        matches!(r, Err(ValidationError::RotateBlsProducerMismatch { .. })),
        "input key of another: {:?}",
        r.err()
    );
    let r = rotate_stateless(&rotation(&producer, &producer), &c);
    assert!(
        matches!(r, Err(ValidationError::RotateBlsBadSignature { .. })),
        "producer key without its signature: {:?}",
        r.err()
    );
}

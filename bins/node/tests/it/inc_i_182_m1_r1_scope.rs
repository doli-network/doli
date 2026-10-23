//! INC-I-182 M1 — R-1 rule scope: at the gate, check_producer_auth refuses
//! exactly the three producer-signature types and leaves every other type alone.
//!
// covers: producer_auth transaction types unbonding defaults env_loader network_params pool assembly tx_processing validation_checks withdrawal_holdings genesis register bonds withdrawal exit unbonding_tx core data block rpc_client
//
// OUTPUT CONTRACT: fn check_producer_auth(&Transaction, &ValidationContext) -> Result<(), ValidationError>
//   O1 return per TxType at the gate (pure)
// PATHS: P-AUTH Registration/AddBond/RequestWithdrawal | P-OTHER every other decodable type
// MATRIX
//   O1 x every type (exhaustive match in the R-1 file) -> coded for P-AUTH, Ok for P-OTHER
//     (inc_i_182_m1_req_pauth_001_r1_rule_scoped_to_three_types)
// INPUT PARTITIONS:
//   extra_data: 200 bytes of 0xAB (no valid producer signature); no inputs or outputs

use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{Transaction, TxType};
use doli_core::validation::producer_auth::check_producer_auth;
use doli_core::validation::{ValidationContext, ValidationError};
use doli_core::Network;

use super::inc_i_182_m1_node_fx::CODE;
use super::inc_i_182_m1_r1_enumeration::{binding, Binding};

// REQ-PAUTH-001 — Decision: a failure means the rule misses one of its three types (a third party keeps that path) or reaches a type it must not touch (valid txs of that type are refused at the gate).
#[test]
fn inc_i_182_m1_req_pauth_001_r1_rule_scoped_to_three_types() {
    let types: Vec<TxType> = (0..=255u32).filter_map(TxType::from_u32).collect();
    let producer_changing = types
        .iter()
        .filter(|t| binding(**t) != Binding::NotProducerChanging)
        .count();
    assert_eq!(
        producer_changing, 8,
        "NON-VACUITY: every producer-changing type decodes"
    );
    let ctx = ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, 1_000)
        .with_inc_i_182_producer_auth(0);
    let mut bad = Vec::new();
    for t in types {
        let tx = Transaction {
            version: 1,
            tx_type: t,
            inputs: Vec::new(),
            outputs: Vec::new(),
            extra_data: vec![0xAB; 200],
        };
        let r = check_producer_auth(&tx, &ctx);
        let coded = matches!(&r, Err(ValidationError::InvalidTransaction(m)) if m.contains(CODE));
        let want = binding(t) == Binding::ProducerSignature;
        if coded != want || (!want && r.is_err()) {
            bad.push(format!("{t:?} ({:?}): {r:?}", binding(t)));
        }
    }
    assert!(bad.is_empty(), "producer-auth scope:\n{}", bad.join("\n"));
}

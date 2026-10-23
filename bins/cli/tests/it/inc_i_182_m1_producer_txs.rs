//! INC-I-182 M1 — the CLI signs register, add-bond and withdraw (and the
//! withdrawal built by exit) with the producer key over the chain genesis it
//! reads from RPC chain info, after inputs and outputs are final; the result
//! validates with the rule at 0 and on a node without it.
//!
// covers: producer_auth transaction types unbonding defaults env_loader network_params pool assembly tx_processing validation_checks withdrawal_holdings genesis register bonds withdrawal exit unbonding_tx core data block rpc_client
//
// OUTPUT CONTRACT: fn doli_cli::producer_tx::build_registration_tx(&RegistrationSpec) -> Result<Transaction, String>
// OUTPUT CONTRACT: fn doli_cli::producer_tx::build_add_bond_tx(&AddBondSpec) -> Transaction
// OUTPUT CONTRACT: fn doli_cli::unbonding_tx::build_withdrawal_tx(&NetworkParams, u64, &WithdrawalSpec) -> Result<Transaction, String>
//   O1 return: the unsigned tx; pure
// OUTPUT CONTRACT: fn doli_cli::producer_tx::finalize_producer_tx(&mut Transaction, &KeyPair, &Hash)
//   O2 receiver: every input signed by the key (public_key set), extra_data = payload | producer-auth signature
// OUTPUT CONTRACT: fn doli_cli::producer_tx::chain_genesis_hash(&str) -> Result<Hash, String>   O3 return
// PATHS: P-REG | P-BOND | P-WD | P-EXIT (source census) | P-PARSE
// MATRIX
//   O1, O2 x {P-REG, P-BOND, P-WD} x {gate 0, gate off} -> valid; tail stripped / other genesis -> coded
//     (inc_i_182_m1_req_pauth_006_cli_txs_validate_at_gate_zero)
//   O2 x {P-REG, P-BOND, P-WD} -> equals input signing + sign_producer_auth
//     (inc_i_182_m1_req_pauth_006_finalize_signs_inputs_then_appends_auth)
//   O3 x P-PARSE x {chain info hex, bad hex, short, empty} -> inc_i_182_m1_req_pauth_006_chain_genesis_from_rpc_hex
//   source x {register, bonds, withdrawal, exit} -> inc_i_182_m1_req_pauth_006_every_producer_command_finalizes
// INPUT PARTITIONS:
//   genesis: devnet chain info hex | testnet; height: best 1_000, mined at 1_001

use crypto::{BlsKeyPair, Hash, KeyPair, PublicKey};
use doli_cli::producer_tx::{
    build_add_bond_tx, build_registration_tx, chain_genesis_hash, finalize_producer_tx,
    AddBondSpec, RegistrationSpec,
};
use doli_cli::unbonding_tx::{build_withdrawal_tx, WithdrawalSpec};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{sign_producer_auth, RegistrationData, Transaction, TxType};
use doli_core::validation::{validate_transaction, ValidationContext, ValidationError};
use doli_core::{Network, NetworkParams};

const CODE: &str = "[ERRTX-PAUTH001]";
const BEST: u64 = 1_000;
const KINDS: [TxType; 3] = [
    TxType::Registration,
    TxType::AddBond,
    TxType::RequestWithdrawal,
];

fn wallet() -> KeyPair {
    KeyPair::from_seed([0x57; 32])
}

fn pk() -> PublicKey {
    *wallet().public_key()
}

fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

fn unit() -> u64 {
    Network::Devnet.bond_unit()
}

fn devnet_genesis() -> Hash {
    ConsensusParams::devnet().genesis_hash
}

fn params() -> NetworkParams {
    let mut p = NetworkParams::defaults(Network::Devnet);
    p.inc_i_257_unbonding_activation_height = 0;
    p
}

fn ctx(gate: u64) -> ValidationContext {
    let p = params();
    ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, BEST + 1)
        .with_inc_i_234_activation_height(0)
        .with_inc_i_257_unbonding(p.inc_i_257_unbonding_activation_height, p.unbonding_period)
        .with_inc_i_182_producer_auth(gate)
}

fn reg_data() -> RegistrationData {
    let bls = BlsKeyPair::from_seed(&[0x58; 32]).expect("FIXTURE: BLS seed");
    RegistrationData {
        public_key: pk(),
        epoch: 0,
        vdf_output: vec![0x11; 32],
        vdf_proof: Vec::new(),
        prev_registration_hash: Hash::ZERO,
        sequence_number: 0,
        bond_count: 2,
        bls_pubkey: bls.public_key().as_bytes().to_vec(),
        bls_pop: bls.proof_of_possession().expect("PoP").as_bytes().to_vec(),
    }
}

/// The tx each command builds, before any signature.
fn built(kind: TxType) -> Transaction {
    let lock_until = BEST + 576 + 1_000;
    let inputs = vec![(Hash::from_bytes([0xA1; 32]), 0)];
    match kind {
        TxType::Registration => build_registration_tx(&RegistrationSpec {
            inputs,
            reg_data: reg_data(),
            bond_unit: unit(),
            lock_until,
            change: 5_000,
            change_to: addr(&pk()),
        })
        .expect("register build"),
        TxType::AddBond => build_add_bond_tx(&AddBondSpec {
            inputs,
            producer: pk(),
            count: 2,
            bond_unit: unit(),
            lock_until,
            change: 5_000,
            change_to: addr(&pk()),
        }),
        _ => build_withdrawal_tx(
            &params(),
            BEST,
            &WithdrawalSpec {
                producer: pk(),
                bond_count: 2,
                bond_inputs: vec![
                    (Hash::from_bytes([0xB1; 32]), 0),
                    (Hash::from_bytes([0xB2; 32]), 1),
                ],
                total_net: 2 * unit(),
                destination: addr(&pk()),
                fee_utxos: Vec::new(),
                fee: 1,
            },
        )
        .expect("withdraw build"),
    }
}

fn finalized(kind: TxType, genesis: &Hash) -> Transaction {
    let mut tx = built(kind);
    finalize_producer_tx(&mut tx, &wallet(), genesis);
    tx
}

fn coded(r: &Result<(), ValidationError>) -> bool {
    matches!(r, Err(ValidationError::InvalidTransaction(m)) if m.contains(CODE))
}

// REQ-PAUTH-006 — Decision: a failure means a tx built by the shipped CLI is refused once the rule is armed (or by a node without the rule), so producers cannot register, bond or withdraw with the client.
#[test]
fn inc_i_182_m1_req_pauth_006_cli_txs_validate_at_gate_zero() {
    let g = chain_genesis_hash(&devnet_genesis().to_hex()).expect("chain info genesis parses");
    assert_eq!(
        g,
        devnet_genesis(),
        "the CLI reads the genesis the chain reports"
    );
    let other = ConsensusParams::testnet().genesis_hash;
    for kind in KINDS {
        let tx = finalized(kind, &g);
        assert_eq!(tx.tx_type, kind, "FIXTURE: builder type");
        for gate in [0, u64::MAX] {
            let r = validate_transaction(&tx, &ctx(gate));
            assert!(r.is_ok(), "{kind:?} built by the CLI, gate {gate}: {r:?}");
        }
        let mut stripped = tx.clone();
        stripped.extra_data.truncate(tx.extra_data.len() - 64);
        assert!(
            coded(&validate_transaction(&stripped, &ctx(0))),
            "{kind:?}: the CLI tx without its auth tail must be refused"
        );
        let foreign = finalized(kind, &other);
        assert!(
            coded(&validate_transaction(&foreign, &ctx(0))),
            "{kind:?}: signed over another network's genesis must be refused"
        );
    }
}

// REQ-PAUTH-006 — Decision: a failure means the CLI signs inputs or the auth tail with the wrong key or bytes (a tx the node refuses), or its finalize differs from the core signer.
#[test]
fn inc_i_182_m1_req_pauth_006_finalize_signs_inputs_then_appends_auth() {
    let g = devnet_genesis();
    for kind in KINDS {
        let got = finalized(kind, &g);
        let mut want = built(kind);
        for i in 0..want.inputs.len() {
            want.inputs[i].public_key = Some(pk());
            let msg = want.signing_message_for_input(i);
            want.inputs[i].signature = crypto::signature::sign_hash(&msg, wallet().private_key());
        }
        sign_producer_auth(&mut want, &g, &wallet());
        assert_eq!(
            got, want,
            "{kind:?}: finalize = sign every input + sign_producer_auth"
        );
    }
}

// REQ-PAUTH-006 — Decision: a failure means the CLI signs with a genesis it did not read correctly from chain info, or proceeds with a garbage genesis instead of failing.
#[test]
fn inc_i_182_m1_req_pauth_006_chain_genesis_from_rpc_hex() {
    let g = devnet_genesis();
    assert_eq!(chain_genesis_hash(&g.to_hex()), Ok(g));
    let hex = g.to_hex();
    for bad in ["", "zz", &hex[..62]] {
        assert!(chain_genesis_hash(bad).is_err(), "{bad:?} must not parse");
    }
}

const COMMANDS: [(&str, &str); 4] = [
    (
        "register",
        include_str!("../../src/cmd_producer/register.rs"),
    ),
    ("add-bond", include_str!("../../src/cmd_producer/bonds.rs")),
    (
        "withdraw",
        include_str!("../../src/cmd_producer/withdrawal.rs"),
    ),
    ("exit", include_str!("../../src/cmd_producer/exit.rs")),
];

fn finalizes(src: &str) -> bool {
    src.contains("finalize_producer_tx(")
}

// REQ-PAUTH-006 — Decision: a failure names a producer command that still submits its tx without the producer-auth signature, so that command breaks once the rule is armed.
#[test]
fn inc_i_182_m1_req_pauth_006_every_producer_command_finalizes() {
    assert!(
        !finalizes("for i in 0..tx.inputs.len() { sign(i) }"),
        "NON-VACUITY: the scan flags a command that only signs inputs"
    );
    let missing: Vec<&str> = COMMANDS
        .iter()
        .filter(|(_, src)| !finalizes(src))
        .map(|(name, _)| *name)
        .collect();
    assert!(
        missing.is_empty(),
        "commands that do not finalize: {missing:?}"
    );
}

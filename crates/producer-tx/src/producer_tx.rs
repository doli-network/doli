//! Pure construction and signing of producer transactions (INC-I-182): the
//! Registration and AddBond shapes, and the final signing step shared by
//! register, add-bond, withdraw and exit. No I/O, no RPC.

use crypto::{signature, Hash, KeyPair, PublicKey};
use doli_core::transaction::{
    sign_producer_auth, Input, Output, RegistrationData, Transaction, TxType,
};

/// Inputs and outputs of a Registration; the caller resolves them from the wallet and RPC.
#[derive(Clone, Debug)]
pub struct RegistrationSpec {
    pub inputs: Vec<(Hash, u32)>,
    pub reg_data: RegistrationData,
    pub bond_unit: u64,
    pub lock_until: u64,
    pub change: u64,
    pub change_to: Hash,
}

/// Inputs and outputs of an AddBond.
#[derive(Clone, Debug)]
pub struct AddBondSpec {
    pub inputs: Vec<(Hash, u32)>,
    pub producer: PublicKey,
    pub count: u32,
    pub bond_unit: u64,
    pub lock_until: u64,
    pub change: u64,
    pub change_to: Hash,
}

fn inputs_of(outpoints: &[(Hash, u32)]) -> Vec<Input> {
    outpoints.iter().map(|(h, i)| Input::new(*h, *i)).collect()
}

fn push_change(tx: &mut Transaction, change: u64, to: Hash) {
    if change > 0 {
        tx.outputs.push(Output::normal(change, to));
    }
}

/// Unsigned Registration: one Bond output per bond, bincode payload, optional change.
pub fn build_registration_tx(spec: &RegistrationSpec) -> Result<Transaction, String> {
    let producer = &spec.reg_data.public_key;
    let owner = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, producer.as_bytes());
    let extra_data = bincode::serialize(&spec.reg_data)
        .map_err(|e| format!("Failed to serialize registration data: {e}"))?;
    let outputs = (0..spec.reg_data.bond_count)
        .map(|_| Output::bond(spec.bond_unit, owner, spec.lock_until, 0))
        .collect();
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Registration,
        inputs: inputs_of(&spec.inputs),
        outputs,
        extra_data,
    };
    // Defense-in-depth (bond-backing): a stock CLI builds exactly
    // `bond_count` Bond outputs of one `bond_unit`, so this never fires here —
    // but it guarantees this builder can NEVER emit a tx whose locked value
    // disagrees with the declared `bond_count`, the shape the on-chain exploit
    // used. The consensus rule (`validation::check_bond_backing`) is the real
    // enforcement; this just stops us shipping a malformed tx in the first
    // place. Uses checked arithmetic.
    let locked: u64 = tx
        .outputs
        .iter()
        .filter(|o| matches!(o.output_type, doli_core::transaction::OutputType::Bond))
        .map(|o| o.amount)
        .fold(0u64, |a, b| a.saturating_add(b));
    let expected = u64::from(spec.reg_data.bond_count)
        .checked_mul(spec.bond_unit)
        .ok_or_else(|| "registration bond_count * bond_unit overflows u64".to_string())?;
    if locked != expected {
        return Err(format!(
            "registration bond-backing mismatch: declared bond_count={} * bond_unit={} = {}, but Bond outputs lock {}",
            spec.reg_data.bond_count, spec.bond_unit, expected, locked
        ));
    }
    push_change(&mut tx, spec.change, spec.change_to);
    Ok(tx)
}

/// Unsigned AddBond with optional change.
pub fn build_add_bond_tx(spec: &AddBondSpec) -> Transaction {
    let amount = spec.bond_unit.saturating_mul(u64::from(spec.count));
    // Defense-in-depth (bond-backing): the locked amount is `count * bond_unit`
    // by construction; assert it so a future edit that breaks the relation
    // trips in debug/test before it can build a tx the consensus rule rejects.
    debug_assert_eq!(amount, spec.bond_unit.saturating_mul(u64::from(spec.count)));
    let mut tx = Transaction::new_add_bond(
        inputs_of(&spec.inputs),
        spec.producer,
        spec.count,
        amount,
        spec.lock_until,
    );
    push_change(&mut tx, spec.change, spec.change_to);
    tx
}

/// Sign every input with `keypair`, then append its producer-auth signature.
/// Call only after inputs and outputs are final.
pub fn finalize_producer_tx(tx: &mut Transaction, keypair: &KeyPair, genesis_hash: &Hash) {
    for i in 0..tx.inputs.len() {
        tx.inputs[i].public_key = Some(*keypair.public_key());
        let msg = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&msg, keypair.private_key());
    }
    sign_producer_auth(tx, genesis_hash, keypair);
}

/// Signed Registration: `build_registration_tx` then `finalize_producer_tx`.
pub fn build_signed_registration(
    spec: &RegistrationSpec,
    keypair: &KeyPair,
    genesis_hash: &Hash,
) -> Result<Transaction, String> {
    let mut tx = build_registration_tx(spec)?;
    finalize_producer_tx(&mut tx, keypair, genesis_hash);
    Ok(tx)
}

/// Parse `getChainInfo.genesis_hash`.
pub fn chain_genesis_hash(chain_info_genesis_hex: &str) -> Result<Hash, String> {
    Hash::from_hex(chain_info_genesis_hex)
        .ok_or_else(|| format!("invalid genesis hash from chain info: {chain_info_genesis_hex:?}"))
}

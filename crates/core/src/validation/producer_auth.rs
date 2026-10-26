//! INC-I-182 producer-auth rule: at and above the gate, Registration, AddBond
//! and RequestWithdrawal carry `canonical payload ‖ 64-byte Ed25519 signature`
//! by the producer the payload names. Below the gate nothing is checked.

use crypto::{PublicKey, Signature};

use crate::transaction::{
    producer_auth_digest, AddBondData, RegistrationData, Transaction, TxType,
    WithdrawalRequestData, PRODUCER_AUTH_SIG_LEN,
};

use super::{ValidationContext, ValidationError};

impl ValidationContext {
    /// Set the INC-I-182 producer-auth gate from `NetworkParams`.
    #[must_use]
    pub fn with_inc_i_182_producer_auth(mut self, activation_height: u64) -> Self {
        self.inc_i_182_producer_auth_activation_height = activation_height;
        self
    }
}

fn refuse(msg: String) -> ValidationError {
    ValidationError::InvalidTransaction(format!("[ERRTX-PAUTH001] {msg}"))
}

/// Judged at the INC-I-257 rule height (`current_height + 1` in the mempool).
pub fn check_producer_auth(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    let kind = tx.tx_type;
    if !matches!(
        kind,
        TxType::Registration | TxType::AddBond | TxType::RequestWithdrawal
    ) || ctx.inc_i_257_rule_height() < ctx.inc_i_182_producer_auth_activation_height
    {
        return Ok(());
    }
    let split = tx
        .extra_data
        .len()
        .checked_sub(PRODUCER_AUTH_SIG_LEN)
        .ok_or_else(|| refuse(format!("{kind:?} has no producer signature")))?;
    let (payload, tail) = tx.extra_data.split_at(split);
    let producer = canonical_producer(kind, payload).ok_or_else(|| {
        refuse(format!(
            "{kind:?} extra_data must be the exact payload followed by a 64-byte producer signature"
        ))
    })?;
    let digest = producer_auth_digest(&ctx.params.genesis_hash, tx, payload);
    Signature::try_from_slice(tail)
        .ok()
        .filter(|sig| crypto::signature::verify_hash(&digest, sig, &producer).is_ok())
        .map(|_| ())
        .ok_or_else(|| refuse(format!("{kind:?} is not signed by its producer")))
}

/// The producer named by `payload`, only if `payload` is its exact encoding.
fn canonical_producer(kind: TxType, payload: &[u8]) -> Option<PublicKey> {
    let (producer, canonical) = match kind {
        TxType::AddBond => {
            AddBondData::from_bytes(payload).map(|d| (d.producer_pubkey, d.to_bytes()))?
        }
        TxType::RequestWithdrawal => {
            WithdrawalRequestData::from_bytes(payload).map(|d| (d.producer_pubkey, d.to_bytes()))?
        }
        _ => {
            let data: RegistrationData = bincode::deserialize(payload).ok()?;
            (data.public_key, bincode::serialize(&data).ok()?)
        }
    };
    (canonical == payload).then_some(producer)
}

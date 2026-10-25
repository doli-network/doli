//! From the slash rule, a Registration or AddBond may not name a producer
//! that is Slashed or has a queued Slash.

use crypto::PublicKey;

use crate::transaction::{Transaction, TxType};

use super::slash_evidence::slash_rule_active;
use super::{ValidationContext, ValidationError};

/// The key a Registration registers or the producer an AddBond bonds (never
/// an input key). `None` for any other type or an undecodable payload.
pub fn slash_registrant(tx: &Transaction) -> Option<PublicKey> {
    match tx.tx_type {
        TxType::Registration => tx.registration_data().map(|d| d.public_key),
        TxType::AddBond => tx.add_bond_data().map(|d| d.producer_pubkey),
        _ => None,
    }
}

/// `[ERRTX071]` when the slash rule is active and the `slash_registrant` key
/// is `slashed` or `slash_queued` in `ctx.slash_targets`; `Ok` otherwise.
pub fn validate_not_slashed_registrant(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    if !slash_rule_active(ctx) {
        return Ok(());
    }
    let Some(key) = slash_registrant(tx) else {
        return Ok(());
    };
    match ctx.slash_targets.get(&key) {
        Some(state) if state.slashed || state.slash_queued => {
            Err(ValidationError::SlashedRegistrant {
                producer: key.to_hex(),
            })
        }
        _ => Ok(()),
    }
}

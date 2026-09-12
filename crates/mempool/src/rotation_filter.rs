//! INC-I-217 M6 — the shared RotateBlsKey admission verdict (REQ-ROT-009).
//!
//! One function, so the relay and the block builder cannot drift from
//! `validate_transaction`, which already refused invalid rotations through this
//! same `rotate_stateless`. M6 names the predicate; it closes no open hole.

use doli_core::transaction::{Transaction, TxType};
use doli_core::validation::rotate_bls::rotate_stateless;
use doli_core::validation::ValidationContext;

/// `Err` carries the bracketed `[ERRTX-ROTnnn]` code the fleet greps; the caller
/// skips (builder) or refuses (admission). Stateless: no snapshot, no UTXO pass.
/// Cost: a valid rotation pays its pairings twice per surface — rotations are
/// rare and `rotate_stateless` orders structural checks first, so an invalid one
/// never reaches a pairing.
pub fn rotation_admissible(tx: &Transaction, ctx: &ValidationContext) -> Result<(), String> {
    if tx.tx_type != TxType::RotateBlsKey {
        return Ok(());
    }
    rotate_stateless(tx, ctx)
        .map(|_| ())
        .map_err(|e| format!("[{}] {e}", e.error_code()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use doli_core::consensus::ConsensusParams;
    use doli_core::Network;

    fn ctx(height: u64, gate: u64) -> ValidationContext {
        ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, height)
            .with_bls_key_rotation_activation_height(gate)
    }

    #[test]
    fn a_non_rotation_is_not_this_predicates_business() {
        let tx = Transaction::new_transfer(Vec::new(), Vec::new());
        assert_eq!(rotation_admissible(&tx, &ctx(1_000, 500)), Ok(()));
    }

    #[test]
    fn a_rotation_below_the_gate_carries_the_consensus_code() {
        let mut tx = Transaction::new_transfer(Vec::new(), Vec::new());
        tx.tx_type = TxType::RotateBlsKey;
        let err = rotation_admissible(&tx, &ctx(499, 500)).unwrap_err();
        assert!(err.contains("[ERRTX-ROT002]"), "{err}");
    }
}

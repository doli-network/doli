//! Zero-knowledge proof verification for L2 settlement.
//!
//! This module exposes **one** function — `verify_zk_proof` — which is the
//! entire L1 surface for verifying L2 state transitions. Everything else
//! (circuit authorship, sequencer operation, bridges, data availability) is
//! the responsibility of the L2 builder, not DOLI.
//!
//! # Design invariants
//!
//! - **Pure.** No I/O, no side effects, no global state.
//! - **Deterministic.** Identical inputs MUST produce identical outputs on
//!   every supported platform. Non-deterministic verification is a silent
//!   fork generator — the worst class of consensus bug. See
//!   `specs/l2-settlement.md` §8.3 for the determinism harness requirement.
//! - **Bounded.** Respects the per-block cost budget supplied via
//!   `ZkVerifyContext`. A proof that would exceed the budget returns
//!   `ZkVerifyError::BudgetExceeded` rather than running past it.
//! - **Gated.** Until the `ProtocolActivation` hard fork fires at
//!   `ZK_SETTLE_ACTIVATION_HEIGHT`, every call returns
//!   `ZkVerifyError::NotYetActivated`. This is the safe default.
//!
//! See `specs/l2-settlement.md` for the full interface specification.

/// Per-call context for ZK verification.
///
/// The block-level cost budget is passed through this struct and decremented
/// as verifications proceed. Callers are responsible for threading the
/// remaining budget across multiple verifications in the same block.
#[derive(Debug, Clone)]
pub struct ZkVerifyContext {
    /// Microseconds of verification budget remaining in the current block.
    /// Callers must debit the returned `cost_us` from this value before the
    /// next call.
    pub budget_us_remaining: u64,
    /// Proof system selector — see `ZkRollupData::proof_system_id`.
    pub proof_system_id: u16,
    /// Current block height (used for the activation gate).
    pub height: u64,
}

/// Structured error returned by `verify_zk_proof`.
///
/// Every variant represents a **deterministic** rejection reason. A variant
/// that depended on wall-clock time or parallel scheduling would be a
/// fork source and is explicitly disallowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZkVerifyError {
    /// ZKSettle is not yet activated at the current block height.
    /// Set `ZK_SETTLE_ACTIVATION_HEIGHT` via `ProtocolActivation` to enable.
    NotYetActivated { height: u64, activation: u64 },
    /// The proof system id in the ZKRollup UTXO is not supported by this
    /// node version. Returned for ids outside the registered set.
    UnsupportedProofSystem(u16),
    /// The verifying key bytes were structurally invalid for the declared
    /// proof system (wrong length, bad magic, malformed header, etc.).
    VerifyingKeyMalformed,
    /// The proof exceeds `MAX_ZK_PROOF_SIZE`.
    ProofTooLarge { size: usize, max: usize },
    /// The proof is structurally well-formed but does not verify against
    /// the supplied (verifying_key, prev_root, next_root) tuple.
    InvalidProof,
    /// This verification would exceed the remaining block-level cost budget.
    /// The block must be rejected.
    BudgetExceeded { cost_us: u64, remaining_us: u64 },
}

impl std::fmt::Display for ZkVerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotYetActivated { height, activation } => write!(
                f,
                "ZKSettle not yet activated (height {} < activation {})",
                height, activation
            ),
            Self::UnsupportedProofSystem(id) => {
                write!(f, "unsupported ZK proof system id: {}", id)
            }
            Self::VerifyingKeyMalformed => write!(f, "verifying key is malformed"),
            Self::ProofTooLarge { size, max } => {
                write!(f, "proof size {} exceeds max {}", size, max)
            }
            Self::InvalidProof => write!(f, "proof did not verify"),
            Self::BudgetExceeded {
                cost_us,
                remaining_us,
            } => write!(
                f,
                "proof verification would exceed block budget (cost={}us remaining={}us)",
                cost_us, remaining_us
            ),
        }
    }
}

impl std::error::Error for ZkVerifyError {}

/// Proof system identifier constants. See `ZkRollupData::proof_system_id`.
pub mod proof_system {
    pub const UNASSIGNED: u16 = 0;
    pub const PLONKY2: u16 = 1;
    pub const HALO2: u16 = 2;
    pub const GROTH16: u16 = 3;
    pub const RISC0: u16 = 4;
}

/// Verify a zero-knowledge proof of an L2 state transition.
///
/// This is the **entire** L1 ZK surface. Pass a verifying key, a previous
/// state commitment, a next state commitment, a proof blob, and a context
/// carrying the remaining block-level verification budget. Returns
/// `Ok(cost_us)` on success (cost MUST be debited by the caller) or a
/// structured `ZkVerifyError` on any failure.
///
/// # Current status
///
/// This function is **intentionally a stub** in the current binary. It
/// returns `NotYetActivated` for every call until:
///
/// 1. A proof system is selected (see `specs/l2-settlement.md` §8.1).
/// 2. The corresponding verifier crate is vendored and pinned.
/// 3. The determinism harness passes on the full CI matrix (§8.3).
/// 4. `ZK_SETTLE_ACTIVATION_HEIGHT` is set to a real future height by a
///    `ProtocolActivation` transaction.
///
/// Until then, every `ZKSettle` transaction is rejected at the activation
/// gate. This is the correct safe default — the interface is published
/// (spec and code), but no proofs are actually accepted.
pub fn verify_zk_proof(
    _verifying_key: &[u8],
    _prev_state_root: &[u8; 32],
    _next_state_root: &[u8; 32],
    proof: &[u8],
    ctx: &ZkVerifyContext,
) -> Result<u64, ZkVerifyError> {
    // Gate 1: activation height. Until set via ProtocolActivation, this
    // short-circuits every call. Safe default.
    if ctx.height < crate::consensus::ZK_SETTLE_ACTIVATION_HEIGHT {
        return Err(ZkVerifyError::NotYetActivated {
            height: ctx.height,
            activation: crate::consensus::ZK_SETTLE_ACTIVATION_HEIGHT,
        });
    }

    // Gate 2: proof size cap. Cheap to check, bounded DoS surface.
    if proof.len() > crate::transaction::MAX_ZK_PROOF_SIZE {
        return Err(ZkVerifyError::ProofTooLarge {
            size: proof.len(),
            max: crate::transaction::MAX_ZK_PROOF_SIZE,
        });
    }

    // Gate 3: proof system dispatch. Every known id must have a handler.
    // Until a real verifier is wired, all known ids return
    // `UnsupportedProofSystem` — which is honest: no proof system is
    // actually supported yet.
    match ctx.proof_system_id {
        proof_system::UNASSIGNED => Err(ZkVerifyError::UnsupportedProofSystem(
            proof_system::UNASSIGNED,
        )),
        proof_system::PLONKY2
        | proof_system::HALO2
        | proof_system::GROTH16
        | proof_system::RISC0 => {
            // STUB: the real verifier wiring lives here. For now, every
            // valid activation-era call still fails closed.
            //
            // When the real verifier is wired:
            //   1. Debit the estimated cost from ctx.budget_us_remaining.
            //   2. Run the deterministic verifier.
            //   3. Return Ok(actual_cost_us) or Err(InvalidProof).
            //
            // See specs/l2-settlement.md §4.3 and §8.3.
            Err(ZkVerifyError::InvalidProof)
        }
        other => Err(ZkVerifyError::UnsupportedProofSystem(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_activation_returns_not_yet_activated() {
        let ctx = ZkVerifyContext {
            budget_us_remaining: 2_000_000,
            proof_system_id: proof_system::PLONKY2,
            height: 100,
        };
        let vk = vec![0u8; 32];
        let prev = [0u8; 32];
        let next = [1u8; 32];
        let proof = vec![0u8; 128];

        let result = verify_zk_proof(&vk, &prev, &next, &proof, &ctx);
        assert!(matches!(result, Err(ZkVerifyError::NotYetActivated { .. })));
    }

    #[test]
    fn proof_too_large_is_rejected_before_verifier_runs() {
        // Pick a height below activation so we'd normally short-circuit;
        // proof size check comes AFTER activation gate in the stub, so
        // construct a context where activation has fired (simulate by
        // using MAX as height) and the proof exceeds the cap.
        let ctx = ZkVerifyContext {
            budget_us_remaining: 2_000_000,
            proof_system_id: proof_system::PLONKY2,
            height: u64::MAX, // bypasses activation gate
        };
        let vk = vec![0u8; 32];
        let prev = [0u8; 32];
        let next = [1u8; 32];
        let oversized = vec![0u8; crate::transaction::MAX_ZK_PROOF_SIZE + 1];

        let result = verify_zk_proof(&vk, &prev, &next, &oversized, &ctx);
        assert!(matches!(result, Err(ZkVerifyError::ProofTooLarge { .. })));
    }

    #[test]
    fn unassigned_proof_system_is_rejected() {
        let ctx = ZkVerifyContext {
            budget_us_remaining: 2_000_000,
            proof_system_id: proof_system::UNASSIGNED,
            height: u64::MAX,
        };
        let vk = vec![0u8; 32];
        let prev = [0u8; 32];
        let next = [1u8; 32];
        let proof = vec![0u8; 128];

        let result = verify_zk_proof(&vk, &prev, &next, &proof, &ctx);
        assert!(matches!(
            result,
            Err(ZkVerifyError::UnsupportedProofSystem(0))
        ));
    }

    #[test]
    fn known_proof_system_stub_rejects_as_invalid() {
        // Until a real verifier is wired, activation-era calls still fail
        // closed with InvalidProof. This test locks the current behavior.
        let ctx = ZkVerifyContext {
            budget_us_remaining: 2_000_000,
            proof_system_id: proof_system::PLONKY2,
            height: u64::MAX,
        };
        let vk = vec![0u8; 32];
        let prev = [0u8; 32];
        let next = [1u8; 32];
        let proof = vec![0u8; 128];

        let result = verify_zk_proof(&vk, &prev, &next, &proof, &ctx);
        assert_eq!(result, Err(ZkVerifyError::InvalidProof));
    }
}

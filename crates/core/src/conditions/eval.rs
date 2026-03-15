//! Condition evaluation engine.

use crypto::hash::hash_with_domain;
use crypto::signature::verify_hash;
use crypto::Hash;

use crate::types::BlockHeight;

use super::{Condition, Witness, ADDRESS_DOMAIN, HASHLOCK_DOMAIN};

// =============================================================================
// EVALUATION
// =============================================================================

/// Context provided to the condition evaluator.
pub struct EvalContext<'a> {
    /// Current chain height at which the transaction is being validated.
    pub current_height: BlockHeight,
    /// The transaction's signing hash (for signature verification).
    pub signing_hash: &'a Hash,
}

/// Evaluate a condition tree against witness data.
///
/// Returns `true` if the condition is satisfied, `false` otherwise.
/// The witness's `or_branch_index` is used internally to track Or-branch consumption.
pub fn evaluate(
    condition: &Condition,
    witness: &Witness,
    ctx: &EvalContext,
    or_branch_idx: &mut usize,
) -> bool {
    match condition {
        Condition::Signature(expected_pkh) => {
            // Find a witness signature whose pubkey hashes to expected_pkh
            for ws in &witness.signatures {
                let actual_hash = hash_with_domain(ADDRESS_DOMAIN, ws.pubkey.as_bytes());
                if actual_hash.as_bytes()[..20] == expected_pkh.as_bytes()[..20]
                    && verify_hash(ctx.signing_hash, &ws.signature, &ws.pubkey).is_ok()
                {
                    return true;
                }
            }
            false
        }

        Condition::Multisig { threshold, keys } => {
            let mut satisfied = 0u8;
            for expected_pkh in keys {
                for ws in &witness.signatures {
                    let actual_hash = hash_with_domain(ADDRESS_DOMAIN, ws.pubkey.as_bytes());
                    if actual_hash.as_bytes()[..20] == expected_pkh.as_bytes()[..20]
                        && verify_hash(ctx.signing_hash, &ws.signature, &ws.pubkey).is_ok()
                    {
                        satisfied += 1;
                        break;
                    }
                }
                if satisfied >= *threshold {
                    return true;
                }
            }
            false
        }

        Condition::Hashlock(expected_hash) => {
            if let Some(preimage) = &witness.preimage {
                let computed = hash_with_domain(HASHLOCK_DOMAIN, preimage);
                computed == *expected_hash
            } else {
                false
            }
        }

        Condition::Timelock(min_height) => ctx.current_height >= *min_height,

        Condition::TimelockExpiry(max_height) => ctx.current_height >= *max_height,

        Condition::And(a, b) => {
            evaluate(a, witness, ctx, or_branch_idx) && evaluate(b, witness, ctx, or_branch_idx)
        }

        Condition::Or(a, b) => {
            // Use the next or_branch selection from witness to choose which branch
            let branch = if *or_branch_idx < witness.or_branches.len() {
                let b = witness.or_branches[*or_branch_idx];
                *or_branch_idx += 1;
                b
            } else {
                // No branch hint — try left first, then right
                if evaluate(a, witness, ctx, or_branch_idx) {
                    return true;
                }
                return evaluate(b, witness, ctx, or_branch_idx);
            };

            if branch {
                evaluate(b, witness, ctx, or_branch_idx)
            } else {
                evaluate(a, witness, ctx, or_branch_idx)
            }
        }

        Condition::Threshold { n, conditions } => {
            let mut satisfied = 0u8;
            for cond in conditions {
                if evaluate(cond, witness, ctx, or_branch_idx) {
                    satisfied += 1;
                    if satisfied >= *n {
                        return true;
                    }
                }
            }
            false
        }
    }
}

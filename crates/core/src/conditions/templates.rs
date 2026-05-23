//! Covenant template functions.
//!
//! Pre-built condition constructors for common transaction patterns.
//! Each function is pure — takes parameters, returns a `Condition` tree.
//! Mirrors the pattern from `crates/channels/src/conditions.rs`.
//!
//! # Templates
//!
//! 1. **Vault** — delayed-withdrawal with cosigner override
//! 2. **Escrow** — m-of-n release with timeout refund
//! 3. **HTLC Payment** — hash-locked payment with signed refund
//! 4. **Subscription** — time-gated bounded payment
//! 5. **Agent Allowance** — bounded delegation to an agent

use crypto::Hash;

use crate::conditions::Condition;
use crate::types::{Amount, BlockHeight};

/// Delayed-withdrawal vault with cosigner emergency override.
///
/// # Spending Paths
///
/// - **Delayed claim** (left/Or-false): owner signs after `unlock_height`.
///   Witness: `{ signatures: [owner_sig], or_branches: [false] }`
/// - **Immediate override** (right/Or-true): 2-of-2 multisig (owner + cosigner).
///   Witness: `{ signatures: [owner_sig, cosigner_sig], or_branches: [true] }`
///
/// # Parameters
///
/// - `owner_hash` — pubkey hash of the vault owner
/// - `cosigner_hash` — pubkey hash of the emergency cosigner
/// - `unlock_height` — absolute block height after which owner can withdraw solo.
///   The caller is responsible for computing this (e.g., `current_height + delay_blocks`).
pub fn vault(owner_hash: Hash, cosigner_hash: Hash, unlock_height: BlockHeight) -> Condition {
    Condition::Or(
        // Delayed: owner + timelock
        Box::new(Condition::And(
            Box::new(Condition::Signature(owner_hash)),
            Box::new(Condition::Timelock(unlock_height)),
        )),
        // Immediate: 2-of-2 multisig
        Box::new(Condition::multisig(2, vec![owner_hash, cosigner_hash])),
    )
}

/// Multi-party escrow with timeout refund.
///
/// # Spending Paths
///
/// - **Release** (left/Or-false): `threshold`-of-`parties.len()` multisig.
///   Witness: `{ signatures: [threshold sigs from parties], or_branches: [false] }`
/// - **Refund** (right/Or-true): refund signer after timeout.
///   Witness: `{ signatures: [refund_sig], or_branches: [true] }`
///
/// # Parameters
///
/// - `parties` — pubkey hashes of the escrow participants
/// - `threshold` — minimum number of parties required to release.
///   Caller's responsibility: `threshold > 0 && threshold <= parties.len()`.
///   Invalid values will be caught by `Condition::validate()` / `encode()`.
/// - `timeout_height` — absolute block height after which refund is enabled
/// - `refund_hash` — pubkey hash of the refund recipient
///
/// # Security Note
///
/// `parties.len()` is bounded by `MAX_MULTISIG_KEYS` (127) at encode time.
/// Exceeding this limit will cause `encode()` to return an error.
pub fn escrow(
    parties: Vec<Hash>,
    threshold: u8,
    timeout_height: BlockHeight,
    refund_hash: Hash,
) -> Condition {
    Condition::Or(
        // Release: m-of-n multisig
        Box::new(Condition::multisig(threshold, parties)),
        // Refund: signature + timeout
        Box::new(Condition::And(
            Box::new(Condition::Signature(refund_hash)),
            Box::new(Condition::TimelockExpiry(timeout_height)),
        )),
    )
}

/// Hash-locked payment with signed refund (HTLC).
///
/// Delegates to `Condition::htlc_signed_refund()` for naming consistency
/// within the template namespace.
///
/// # Spending Paths
///
/// - **Claim** (left/Or-false): reveal preimage after `lock_height`.
///   Witness: `{ preimage: Some(preimage_bytes), or_branches: [false] }`
/// - **Refund** (right/Or-true): refund signer after `expiry_height`.
///   Witness: `{ signatures: [refund_sig], or_branches: [true] }`
///
/// # Parameters
///
/// - `payment_hash` — BLAKE3 hash of the payment preimage
/// - `lock_height` — block height after which claim is possible
/// - `expiry_height` — block height after which refund is possible
/// - `refund_hash` — pubkey hash of the refund signer
pub fn htlc_payment(
    payment_hash: Hash,
    lock_height: BlockHeight,
    expiry_height: BlockHeight,
    refund_hash: Hash,
) -> Condition {
    Condition::htlc_signed_refund(payment_hash, lock_height, expiry_height, refund_hash)
}

/// Time-gated bounded payment for recurring allowances.
///
/// Combines recipient and amount guards with a time window. The spending
/// transaction must pay at least `required_amount` to `recipient_hash` at
/// `output_index`, and the spend must occur within the time window
/// `[interval_start, interval_end]`.
///
/// # Spending Paths
///
/// Single path — all four sub-conditions must be satisfied simultaneously:
/// - RecipientGuard: `tx.outputs[output_index].pubkey_hash == recipient_hash`
/// - AmountGuard: `tx.outputs[output_index].amount >= required_amount`
/// - Timelock: `current_height >= interval_start`
/// - TimelockExpiry: `current_height <= interval_end`
///
/// Witness: `{ signatures: [], or_branches: [] }` (guards consume no witness data)
///
/// # Parameters
///
/// - `recipient_hash` — pubkey hash that must receive the payment
/// - `required_amount` — minimum amount the recipient must receive at `output_index`
/// - `output_index` — index in the spending transaction's outputs to check
/// - `interval_start` — earliest block height the payment can be made
/// - `interval_end` — latest block height the payment can be made
///
/// # Depth
///
/// Nesting depth is 3 (And(And(..), And(..))). Fits within MAX_CONDITION_DEPTH=4
/// but leaves only 1 level for user composition around this template.
pub fn subscription(
    recipient_hash: Hash,
    required_amount: Amount,
    output_index: u8,
    interval_start: BlockHeight,
    interval_end: BlockHeight,
) -> Condition {
    Condition::And(
        // Guards: who gets paid and how much
        Box::new(Condition::And(
            Box::new(Condition::recipient_guard(recipient_hash, output_index)),
            Box::new(Condition::amount_guard(required_amount, output_index)),
        )),
        // Time window: when the payment can be made
        Box::new(Condition::And(
            Box::new(Condition::Timelock(interval_start)),
            Box::new(Condition::TimelockExpiry(interval_end)),
        )),
    )
}

/// Bounded delegation: agent signs, must pay recipient a minimum amount.
///
/// The agent-era flagship pattern. An agent can spend the UTXO but ONLY
/// if the spending transaction pays at least `required_amount` to
/// `recipient_hash` at `output_index`.
///
/// # Spending Paths
///
/// Single path — all three sub-conditions must be satisfied simultaneously:
/// - Signature: agent must sign the transaction
/// - RecipientGuard: `tx.outputs[output_index].pubkey_hash == recipient_hash`
/// - AmountGuard: `tx.outputs[output_index].amount >= required_amount`
///
/// Witness: `{ signatures: [agent_sig], or_branches: [] }`
///
/// # Parameters
///
/// - `agent_hash` — pubkey hash of the delegated agent
/// - `recipient_hash` — pubkey hash that must receive the payment
/// - `required_amount` — minimum amount the recipient must receive
/// - `output_index` — index in the spending transaction's outputs to check
///
/// # Depth
///
/// Nesting depth is 2 (And(And(..), ..)). Fits comfortably within
/// MAX_CONDITION_DEPTH=4.
pub fn agent_allowance(
    agent_hash: Hash,
    recipient_hash: Hash,
    required_amount: Amount,
    output_index: u8,
) -> Condition {
    Condition::And(
        // Agent signs + recipient must match
        Box::new(Condition::And(
            Box::new(Condition::Signature(agent_hash)),
            Box::new(Condition::recipient_guard(recipient_hash, output_index)),
        )),
        // Amount floor
        Box::new(Condition::amount_guard(required_amount, output_index)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::MAX_EXTRA_DATA_SIZE;
    use crypto::hash::hash;

    fn test_hash(val: u8) -> Hash {
        hash(&[val])
    }

    // ── OUTPUT CONTRACT ─────────────────────────────────────────────────
    //
    // | Template         | Input Partition        | Path       | Expected Output                                    |
    // |------------------|------------------------|------------|----------------------------------------------------|
    // | vault            | typical inputs         | constructor| Or(And(Sig(owner), Timelock(h)), Multisig(2,[o,c])) |
    // | vault            | round-trip             | enc/dec    | decode(encode(vault)) == vault                      |
    // | vault            | size + depth check     | validate   | validate() OK, size <= MAX_EXTRA_DATA_SIZE          |
    // | escrow           | 3-of-5 + timeout       | constructor| Or(Multisig(3,[5]), And(Sig(r), TimelockExpiry(t))) |
    // | escrow           | round-trip             | enc/dec    | decode(encode(escrow)) == escrow                    |
    // | escrow           | 1-of-2 (min threshold) | boundary   | Or(Multisig(1,[2]), And(Sig(r), TimelockExpiry(t))) |
    // | escrow           | size + depth check     | validate   | validate() OK, size <= MAX_EXTRA_DATA_SIZE          |
    // | htlc_payment     | standard               | delegation | matches htlc_signed_refund output                   |
    // | htlc_payment     | round-trip             | enc/dec    | decode(encode(htlc)) == htlc                        |
    // | subscription     | valid                  | constructor| nested And tree with guards + timelocks             |
    // | subscription     | round-trip             | enc/dec    | decode(encode(sub)) == sub                          |
    // | subscription     | size + depth check     | validate   | validate() OK, size <= MAX_EXTRA_DATA_SIZE          |
    // | agent_allowance  | valid                  | constructor| And(And(Sig, RecipGuard), AmountGuard)               |
    // | agent_allowance  | round-trip             | enc/dec    | decode(encode(aa)) == aa                            |
    // | agent_allowance  | size + depth check     | validate   | validate() OK, size <= MAX_EXTRA_DATA_SIZE          |

    // ── vault ────────────────────────────────────────────────────────────

    #[test]
    fn vault_structure() {
        let owner = test_hash(1);
        let cosigner = test_hash(2);
        let unlock_height = 1000;

        let cond = vault(owner, cosigner, unlock_height);

        match cond {
            Condition::Or(delayed, immediate) => {
                // Left: owner signature + timelock
                match *delayed {
                    Condition::And(sig, tl) => {
                        assert!(matches!(*sig, Condition::Signature(h) if h == owner));
                        assert!(matches!(*tl, Condition::Timelock(1000)));
                    }
                    _ => panic!("delayed path should be And(Sig, Timelock)"),
                }
                // Right: 2-of-2 multisig (owner + cosigner)
                match *immediate {
                    Condition::Multisig {
                        threshold,
                        ref keys,
                    } => {
                        assert_eq!(threshold, 2);
                        assert_eq!(keys.len(), 2);
                        assert!(keys.contains(&owner));
                        assert!(keys.contains(&cosigner));
                    }
                    _ => panic!("immediate path should be Multisig(2, [owner, cosigner])"),
                }
            }
            _ => panic!("vault should be Or(delayed, immediate)"),
        }
    }

    #[test]
    fn vault_roundtrip() {
        let cond = vault(test_hash(1), test_hash(2), 5000);
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);
    }

    #[test]
    fn vault_validates_and_fits() {
        let cond = vault(test_hash(1), test_hash(2), 1000);
        cond.validate().unwrap();
        let size = cond.encode().unwrap().len();
        assert!(size <= MAX_EXTRA_DATA_SIZE, "vault {size} > max");
    }

    // ── escrow ───────────────────────────────────────────────────────────

    #[test]
    fn escrow_structure_3_of_5() {
        let parties: Vec<Hash> = (1..=5).map(test_hash).collect();
        let refund = test_hash(10);
        let timeout = 50000;

        let cond = escrow(parties.clone(), 3, timeout, refund);

        match cond {
            Condition::Or(release, refund_path) => {
                // Left: 3-of-5 multisig
                match *release {
                    Condition::Multisig {
                        threshold,
                        ref keys,
                    } => {
                        assert_eq!(threshold, 3);
                        assert_eq!(keys.len(), 5);
                        assert_eq!(keys, &parties);
                    }
                    _ => panic!("release path should be Multisig"),
                }
                // Right: refund sig + timelock expiry
                match *refund_path {
                    Condition::And(sig, expiry) => {
                        assert!(matches!(*sig, Condition::Signature(h) if h == refund));
                        assert!(matches!(*expiry, Condition::TimelockExpiry(50000)));
                    }
                    _ => panic!("refund path should be And(Sig, TimelockExpiry)"),
                }
            }
            _ => panic!("escrow should be Or(release, refund)"),
        }
    }

    #[test]
    fn escrow_roundtrip() {
        let parties: Vec<Hash> = (1..=3).map(test_hash).collect();
        let cond = escrow(parties, 2, 10000, test_hash(20));
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);
    }

    #[test]
    fn escrow_min_threshold_1_of_2() {
        let parties: Vec<Hash> = (1..=2).map(test_hash).collect();
        let cond = escrow(parties.clone(), 1, 1000, test_hash(10));

        match cond {
            Condition::Or(release, _) => match *release {
                Condition::Multisig {
                    threshold,
                    ref keys,
                } => {
                    assert_eq!(threshold, 1);
                    assert_eq!(keys.len(), 2);
                }
                _ => panic!("release should be Multisig"),
            },
            _ => panic!("escrow should be Or"),
        }
    }

    #[test]
    fn escrow_validates_and_fits() {
        let parties: Vec<Hash> = (1..=5).map(test_hash).collect();
        let cond = escrow(parties, 3, 50000, test_hash(10));
        cond.validate().unwrap();
        let size = cond.encode().unwrap().len();
        assert!(size <= MAX_EXTRA_DATA_SIZE, "escrow {size} > max");
    }

    // ── htlc_payment ─────────────────────────────────────────────────────

    #[test]
    fn htlc_payment_matches_signed_refund() {
        let payment_hash = test_hash(1);
        let lock_height = 100;
        let expiry_height = 200;
        let refund = test_hash(2);

        let template = htlc_payment(payment_hash, lock_height, expiry_height, refund);
        let direct =
            Condition::htlc_signed_refund(payment_hash, lock_height, expiry_height, refund);
        assert_eq!(template, direct);
    }

    #[test]
    fn htlc_payment_roundtrip() {
        let cond = htlc_payment(test_hash(1), 100, 200, test_hash(2));
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);
    }

    // ── subscription ─────────────────────────────────────────────────────

    #[test]
    fn subscription_structure() {
        let recipient = test_hash(1);
        let required_amount: Amount = 500_000;
        let output_index: u8 = 0;
        let interval_start: BlockHeight = 1000;
        let interval_end: BlockHeight = 2000;

        let cond = subscription(
            recipient,
            required_amount,
            output_index,
            interval_start,
            interval_end,
        );

        // Expected: And(And(RecipientGuard, AmountGuard), And(Timelock, TimelockExpiry))
        match cond {
            Condition::And(guards, timelocks) => {
                match *guards {
                    Condition::And(ref rg, ref ag) => {
                        assert!(matches!(**rg, Condition::RecipientGuard {
                                expected_pubkey_hash,
                                output_index: idx,
                            } if expected_pubkey_hash == recipient && idx == 0));
                        assert!(matches!(**ag, Condition::AmountGuard {
                                min_amount,
                                output_index: idx,
                            } if min_amount == 500_000 && idx == 0));
                    }
                    _ => panic!("guards part should be And(RecipientGuard, AmountGuard)"),
                }
                match *timelocks {
                    Condition::And(ref tl, ref te) => {
                        assert!(matches!(**tl, Condition::Timelock(1000)));
                        assert!(matches!(**te, Condition::TimelockExpiry(2000)));
                    }
                    _ => panic!("timelocks part should be And(Timelock, TimelockExpiry)"),
                }
            }
            _ => panic!("subscription should be And(guards, timelocks)"),
        }
    }

    #[test]
    fn subscription_roundtrip() {
        let cond = subscription(test_hash(1), 1_000_000, 0, 500, 1500);
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);
    }

    #[test]
    fn subscription_validates_and_fits() {
        let cond = subscription(test_hash(1), 1_000_000, 0, 500, 1500);
        cond.validate().unwrap();
        let size = cond.encode().unwrap().len();
        assert!(size <= MAX_EXTRA_DATA_SIZE, "subscription {size} > max");
    }

    // ── agent_allowance ──────────────────────────────────────────────────

    #[test]
    fn agent_allowance_structure() {
        let agent = test_hash(1);
        let recipient = test_hash(2);
        let required_amount: Amount = 100_000;
        let output_index: u8 = 0;

        let cond = agent_allowance(agent, recipient, required_amount, output_index);

        // Expected: And(And(Signature(agent), RecipientGuard), AmountGuard)
        match cond {
            Condition::And(sig_and_recip, amount) => {
                match *sig_and_recip {
                    Condition::And(ref sig, ref rg) => {
                        assert!(matches!(**sig, Condition::Signature(h) if h == agent));
                        assert!(matches!(**rg, Condition::RecipientGuard {
                                expected_pubkey_hash,
                                output_index: idx,
                            } if expected_pubkey_hash == recipient && idx == 0));
                    }
                    _ => panic!("inner should be And(Sig, RecipientGuard)"),
                }
                assert!(matches!(*amount, Condition::AmountGuard {
                        min_amount,
                        output_index: idx,
                    } if min_amount == 100_000 && idx == 0));
            }
            _ => panic!("agent_allowance should be And(And(Sig, RecipGuard), AmountGuard)"),
        }
    }

    #[test]
    fn agent_allowance_roundtrip() {
        let cond = agent_allowance(test_hash(1), test_hash(2), 500_000, 1);
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);
    }

    #[test]
    fn agent_allowance_validates_and_fits() {
        let cond = agent_allowance(test_hash(1), test_hash(2), 100_000, 0);
        cond.validate().unwrap();
        let size = cond.encode().unwrap().len();
        assert!(size <= MAX_EXTRA_DATA_SIZE, "agent_allowance {size} > max");
    }
}

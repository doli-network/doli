//! Small helper functions for apply_block — extracted to keep mod.rs under 500 lines.

use crypto::PublicKey;
use doli_core::{Block, TxType};
use std::collections::HashSet;

/// Returns true if the block carries any transaction that mid-epoch mutates
/// the `ProducerSet` (registrations, bond changes, exits, delegation changes).
/// Used by the INC-I-071 fix to decide whether the per-block undo entry needs
/// a full ProducerSet snapshot or can use the empty-Vec sentinel.
///
/// Per CLAUDE.md: producer mutations driven by these tx types are DEFERRED
/// to the next epoch boundary, but they still mark the producer set as
/// pending-dirty — the safe rule is to snapshot whenever such a tx is present.
pub(super) fn block_mutates_producer_set(block: &Block) -> bool {
    block.transactions.iter().any(|tx| {
        matches!(
            tx.tx_type,
            TxType::Registration
                | TxType::Exit
                | TxType::AddBond
                | TxType::RequestWithdrawal
                | TxType::ClaimWithdrawal
                | TxType::DelegateBond
                | TxType::RevokeDelegation
                | TxType::RotateBlsKey
        )
    })
}

/// One producer that belonged to a block's attestation universe but did not attest.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct MissedAttestation {
    /// First 8 hex chars of the producer's public key — the identifier used in logs.
    pub short_hex: String,
    /// `true` when the producer sits in the epoch's base `producer_list` (i.e. it is
    /// scheduled for slots this epoch); `false` when it is an active producer that the
    /// epoch-boundary attestation filter left out of the schedule (the "extra" segment).
    pub scheduled: bool,
}

/// Enumerate the producers that did NOT attest in a block, over the attestation
/// universe `[base | extra sorted]` that the bitfield was decoded against.
///
/// `attested_indices` are the decoded bitfield indices; `base` is
/// `epoch_state.producer_list`; `extra` is the sorted tail of active producers that
/// are not in `base`. Index `i < base.len()` addresses `base[i]`, and `i >= base.len()`
/// addresses `extra[i - base.len()]` — the same layout the encoder uses.
///
/// Returns an empty vec when the block carries no decoded attestations at all, so a
/// block without attestation data is not reported as "everybody missed".
pub(super) fn missing_attesters(
    attested_indices: &[usize],
    base: &[PublicKey],
    extra: &[PublicKey],
) -> Vec<MissedAttestation> {
    if attested_indices.is_empty() {
        return Vec::new();
    }
    let base_len = base.len();
    let attested: HashSet<usize> = attested_indices.iter().copied().collect();
    base.iter()
        .chain(extra.iter())
        .enumerate()
        .filter(|(i, _)| !attested.contains(i))
        .map(|(i, pk)| {
            let h = hex::encode(pk.as_bytes());
            MissedAttestation {
                short_hex: h[..8].to_string(),
                scheduled: i < base_len,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pk(seed: u8) -> PublicKey {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        PublicKey::from_bytes(bytes)
    }

    fn short(seed: u8) -> String {
        hex::encode(pk(seed).as_bytes())[..8].to_string()
    }

    /// INC-I-154 F2: an active producer in the extra segment that never attests must be
    /// named. Pre-fix this returns empty — every base member attested, so the old
    /// `attested.len() >= base_len` guard short-circuits and the extra is never examined.
    #[test]
    fn inc_i_154_extra_segment_absentee_is_reported() {
        let base = vec![pk(1), pk(2), pk(3)];
        let extra = vec![pk(9)];
        // Base indices 0,1,2 attested; extra index 3 did not.
        let missing = missing_attesters(&[0, 1, 2], &base, &extra);

        assert_eq!(
            missing,
            vec![MissedAttestation {
                short_hex: short(9),
                scheduled: false,
            }],
            "an unscheduled (extra-segment) producer that did not attest must be reported"
        );
    }

    /// INC-I-154 F3: the guard must not compare a base+extra index count against
    /// `base_len`. Pre-fix, two attesting extras push the count to 4 >= base_len 3, which
    /// suppresses the line for a genuinely missing BASE producer.
    #[test]
    fn inc_i_154_attesting_extras_cannot_mask_a_base_miss() {
        let base = vec![pk(1), pk(2), pk(3)];
        let extra = vec![pk(9), pk(10)];
        // Base 0,1 attested, base 2 missing; both extras (3,4) attested.
        let missing = missing_attesters(&[0, 1, 3, 4], &base, &extra);

        assert_eq!(
            missing,
            vec![MissedAttestation {
                short_hex: short(3),
                scheduled: true,
            }],
            "a base miss must still be reported when attesting extras inflate the index count"
        );
    }

    /// INC-I-154: N>1 absentees spanning both segments are all named, base first.
    #[test]
    fn inc_i_154_multiple_absentees_across_both_segments() {
        let base = vec![pk(1), pk(2), pk(3)];
        let extra = vec![pk(9), pk(10)];
        // Only base 0 and extra 4 attested.
        let missing = missing_attesters(&[0, 4], &base, &extra);

        assert_eq!(
            missing,
            vec![
                MissedAttestation {
                    short_hex: short(2),
                    scheduled: true,
                },
                MissedAttestation {
                    short_hex: short(3),
                    scheduled: true,
                },
                MissedAttestation {
                    short_hex: short(9),
                    scheduled: false,
                },
            ]
        );
    }

    /// Regression guard: the pre-existing base-only reporting must keep working.
    #[test]
    fn base_miss_is_still_reported() {
        let base = vec![pk(1), pk(2), pk(3)];
        let missing = missing_attesters(&[0, 1], &base, &[]);

        assert_eq!(
            missing,
            vec![MissedAttestation {
                short_hex: short(3),
                scheduled: true,
            }]
        );
    }

    /// Regression guard: a block with no decoded attestations is silent, not
    /// "everybody missed".
    #[test]
    fn no_decoded_attestations_is_silent() {
        let base = vec![pk(1), pk(2), pk(3)];
        assert!(missing_attesters(&[], &base, &[pk(9)]).is_empty());
    }

    /// Regression guard: a fully-attested universe reports nothing.
    #[test]
    fn full_attestation_is_silent() {
        let base = vec![pk(1), pk(2), pk(3)];
        let extra = vec![pk(9)];
        assert!(missing_attesters(&[0, 1, 2, 3], &base, &extra).is_empty());
    }
}

/// INC-I-217 M7 — `block_mutates_producer_set` x `TxType`.
///
/// covers: REQ-ROT-006
///
/// OUTPUT CONTRACT: fn block_mutates_producer_set(&Block) -> bool
///   O1 return: the ONLY output. No mutable params, no receiver, no store, no statics.
///   Consumed at `apply_block/mod.rs:173-183`: `true` -> the undo entry carries a full
///   `ProducerSet` snapshot; `false` -> the empty sentinel and rollback SKIPS the restore.
/// PATHS: P1 block with a producer-mutating tx -> true. P2 block without one -> false.
/// INPUT PARTITIONS: I1 rotation only (P1, the new case). I2 transfer only (P2, the
///   non-vacuity control). I3 every `TxType` variant, one block each (P1+P2, the tripwire).
/// MATRIX: I1xO1=true, I2xO1=false, I3xO1 == `expected_producer_mutating(t)` for all t.
#[cfg(test)]
mod rotation_tests {
    use super::*;
    use doli_core::transaction::{Input, Output, Transaction};
    use doli_core::BlockHeader;

    fn pk(seed: u8) -> PublicKey {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        PublicKey::from_bytes(bytes)
    }

    /// Every `TxType` the wire can carry. Paired with the exhaustive match below,
    /// which the compiler breaks when a variant is added.
    const ALL_TX_TYPES: [TxType; 25] = [
        TxType::Transfer,
        TxType::Registration,
        TxType::Exit,
        TxType::ClaimReward,
        TxType::ClaimBond,
        TxType::SlashProducer,
        TxType::Coinbase,
        TxType::AddBond,
        TxType::RequestWithdrawal,
        TxType::ClaimWithdrawal,
        TxType::EpochReward,
        TxType::RemoveMaintainer,
        TxType::AddMaintainer,
        TxType::DelegateBond,
        TxType::RevokeDelegation,
        TxType::ProtocolActivation,
        TxType::PriceAttestation,
        TxType::MintAsset,
        TxType::BurnAsset,
        TxType::CreatePool,
        TxType::AddLiquidity,
        TxType::RemoveLiquidity,
        TxType::Swap,
        TxType::ZKSettle,
        TxType::RotateBlsKey,
    ];

    /// EXHAUSTIVE — no `_` arm. A new `TxType` fails to compile here, which is the
    /// point: a producer-mutating type omitted from `block_mutates_producer_set`
    /// compiles clean, skips the undo snapshot, and forks the chain on the first
    /// reorg across a block carrying it.
    fn expected_producer_mutating(t: TxType) -> bool {
        match t {
            TxType::Registration
            | TxType::Exit
            | TxType::AddBond
            | TxType::RequestWithdrawal
            | TxType::ClaimWithdrawal
            | TxType::DelegateBond
            | TxType::RevokeDelegation
            | TxType::RotateBlsKey => true,
            TxType::Transfer
            | TxType::ClaimReward
            | TxType::ClaimBond
            | TxType::SlashProducer
            | TxType::Coinbase
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
            | TxType::ZKSettle => false,
        }
    }

    fn tx_of(tx_type: TxType) -> Transaction {
        Transaction {
            version: 1,
            tx_type,
            inputs: vec![Input::new(crypto::hash::hash(b"m7-helpers-outpoint"), 0)],
            outputs: vec![Output::normal(1, crypto::hash::hash(b"m7-helpers-change"))],
            extra_data: Vec::new(),
        }
    }

    fn block_with(tx_types: &[TxType]) -> Block {
        let header = BlockHeader {
            version: 2,
            prev_hash: crypto::Hash::ZERO,
            merkle_root: crypto::Hash::ZERO,
            presence_root: crypto::Hash::ZERO,
            genesis_hash: crypto::Hash::ZERO,
            timestamp: 0,
            slot: 1,
            producer: pk(1),
            vdf_output: vdf::VdfOutput {
                value: vec![0u8; 32],
            },
            vdf_proof: vdf::VdfProof::empty(),
            missed_producers: Vec::new(),
            data_root: crypto::Hash::ZERO,
            fork_id: crypto::Hash::ZERO,
        };
        Block::new(header, tx_types.iter().copied().map(tx_of).collect())
    }

    // REQ-ROT-006 — Decision: whether a rotation-carrying block gets its undo snapshot. A
    // `false` here means `rollback.rs` writes the empty sentinel, the restore is skipped,
    // and a reorg across the rotation leaves the node holding a key its peers rolled back.
    #[test]
    fn rotation_block_mutates_the_producer_set() {
        assert!(block_mutates_producer_set(&block_with(&[
            TxType::RotateBlsKey
        ])));
        println!("ROT-UNDO-SNAPSHOT-NON-EMPTY");
    }

    // REQ-ROT-006 — Decision: whether the predicate is a constant `true`, which would make
    // the assertion above pass for any block and prove nothing.
    #[test]
    fn transfer_only_block_does_not_mutate_the_producer_set() {
        assert!(!block_mutates_producer_set(&block_with(&[
            TxType::Transfer
        ])));
    }

    // REQ-ROT-006 — Decision: whether the predicate agrees with the intended set for EVERY
    // wire type. This is the guard that makes the next producer-mutating type impossible to
    // forget: the exhaustive match above breaks the build, this test breaks on the verdict.
    #[test]
    fn predicate_agrees_with_the_exhaustive_intent_for_every_tx_type() {
        for t in ALL_TX_TYPES {
            assert_eq!(
                block_mutates_producer_set(&block_with(&[t])),
                expected_producer_mutating(t),
                "block_mutates_producer_set disagrees with the intended set for {t:?}"
            );
        }
    }

    // REQ-ROT-006 — Decision: whether a rotation is still seen when it shares a block with
    // non-mutating traffic, i.e. whether the scan is `any` and not "first tx only".
    #[test]
    fn rotation_is_seen_behind_a_coinbase_and_a_transfer() {
        let block = block_with(&[TxType::Coinbase, TxType::Transfer, TxType::RotateBlsKey]);
        assert!(block_mutates_producer_set(&block));
    }
}

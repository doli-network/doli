//! INC-I-180 M2 / S3 — `ValidationMode::Replay` strictness of the withdrawal
//! gate. Reviewer F2: `mode` is referenced NOWHERE in the M1 gate body
//! (`validation_checks.rs:599-874`), so a gate whose count binding is built on
//! the UTXO view fires against the degraded view Replay legitimately sees
//! (INC-I-064; `apply_block/tx_processing.rs:116-131` already tolerates per-tx
//! UTXO failure in Replay for exactly this reason).
//!
//! covers: validation_checks.rs (the mode carve-out), production/mod.rs,
//!         assembly.rs, pool.rs, rewards.rs (M2 siblings; unchanged by S3)
//!
//! ---------------------------------------------------------------------------
//! THE SPLIT THIS FILE PINS
//! ---------------------------------------------------------------------------
//! Carve-out (`warn!` + skip the tx) when `mode == Replay`, because the rule's
//! inputs are NOT recoverable from a degraded replay view:
//!   R0 `[ECON_WITHDRAWAL_UNKNOWN_PRODUCER]` — the operator `recover`/reindex
//!      tool rebuilds the ProducerSet as it walks, so "registered at this
//!      height" is not knowable from the partially-rebuilt set.
//!   R3 `[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]` and both R2 shapes
//!      (`[..._BOND_COUNT_MISMATCH]`, `[..._INCOMPLETE_DRAIN]`) — every term
//!      (`bond_inputs`, `all_bond_inputs`, `owned_live_bonds`) is read from the
//!      pre-block UTXO view, which is the degraded object.
//!
//! STRICT in all three modes, because the rule's inputs are mode-independent:
//!   R1 `[ECON_WITHDRAWAL_OVER_HOLDINGS]` — reads the ProducerSet allowance
//!      only; the reviewer's prescription keeps it strict and this file locks
//!      that.
//!   R4 `[ECON_WITHDRAWAL_SAME_BLOCK_INPUT]` — reads `earlier_tx_hashes`, i.e.
//!      the block itself, which Replay has in full. No UTXO term appears in it,
//!      so a carve-out would weaken Replay for no recoverable benefit.
//!
//! Blast radius of the carve-out is bounded and already measured: the only
//! non-test caller passing `Replay` is `bins/node/src/operations/chain.rs:354`.
//! Gossip-received blocks never use `Replay`.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! Function under test:
//!   `Node::validate_block_economics(&self, &Block, u64, ValidationMode)
//!        -> Result<()>`
//! It mutates nothing (`&self`, and every guard it takes is a read guard), so
//! the ONLY observable outputs are:
//!   O1  the `Ok`/`Err` verdict
//!   O2  the bracketed error code carried by an `Err` (the fleet greps on it,
//!       so "some rejection" is not the contract — the identity is)
//!   O3  cross-mode agreement: the verdict tuple
//!       (Full, Light, Replay) for ONE (ledger, block, height) triple
//!   NOT outputs: no producer-set mutation, no UTXO mutation, no store write.
//!
//! PATHS
//!   PA  post-AH, `ValidationMode::Replay`
//!   PB  post-AH, `ValidationMode::Full`
//!   PC  post-AH, `ValidationMode::Light`
//!   PD  pre-AH, all three modes (gate skipped whole)
//!
//! INPUT PARTITIONS (one per rule, at the height band named)
//!   IP-R0    withdrawal names a producer the ledger does not carry
//!   IP-BIND  every Bond input unresolvable in the replayed UTXO view
//!   IP-R3    Bond inputs mixed between the named producer and a foreign owner
//!   IP-R2D   declared == allowance, but the tx drains fewer than the owned
//!            Bond UTXOs (incomplete drain)
//!   IP-R1    declared > allowance, producer registered, inputs resolvable
//!   IP-R4    an input references a tx at a LOWER index in the same block
//!
//! MATRIX (every enumerated cell has an assertion)
//!   O1,O2 × PA × {IP-R0, IP-BIND, IP-R3, IP-R2D}
//!         → req_i180_003_replay_tolerates_the_admission_only_rules      [RED]
//!   O1,O2 × PA × {IP-R1, IP-R4}
//!         → req_i180_003_replay_keeps_the_allowance_and_same_block_rules_strict
//!   O1,O2,O3 × {PB,PC} × {IP-R0, IP-BIND, IP-R3, IP-R2D, IP-R1}
//!         → req_i180_003_full_and_light_verdicts_are_unchanged_by_the_carve_out
//!   O1,O3 × PD × {IP-R0, IP-BIND, IP-R3, IP-R2D, IP-R1}
//!         → req_i180_003_pre_activation_is_mode_invariant

use doli_core::transaction::Transaction;
use doli_core::validation::ValidationMode;

use crate::inc_i_180_common::{
    add_bond_tx, make_node, seed_bond_utxos, seed_bond_utxos_split, seed_owned_bond_utxos,
    verdict_in_mode, withdrawal_tx, withdrawal_tx_chained, withdrawal_tx_with_inputs, POST_AH,
    PRE_AH,
};

/// Ledger size shared by every partition: 4 flushed bonds, nothing in flight,
/// nothing already pending — so `allowance == 4` everywhere below.
const HELD: u32 = 4;

/// The six partitions, each built on a FRESH node so that `owned_live_bonds`
/// (an owner-index scan over the whole UTXO set) cannot leak between them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Partition {
    R0,
    Bind,
    R3,
    R2Drain,
    R1,
    R4,
}

impl Partition {
    /// The bracketed code the UNGATED (mode-independent) gate raises today.
    fn code(self) -> &'static str {
        match self {
            Partition::R0 => "[ECON_WITHDRAWAL_UNKNOWN_PRODUCER]",
            Partition::Bind => "[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]",
            Partition::R3 => "[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]",
            Partition::R2Drain => "[ECON_WITHDRAWAL_INCOMPLETE_DRAIN]",
            Partition::R1 => "[ECON_WITHDRAWAL_OVER_HOLDINGS]",
            Partition::R4 => "[ECON_WITHDRAWAL_SAME_BLOCK_INPUT]",
        }
    }

    /// Rules whose every term comes from the degraded replay view.
    fn is_admission_only(self) -> bool {
        matches!(
            self,
            Partition::R0 | Partition::Bind | Partition::R3 | Partition::R2Drain
        )
    }
}

/// Build the (ledger owner, block transactions) pair for one partition and run
/// `validate_block_economics` in `mode` at `height`. Returns the error STRING
/// so the assertion can bind to the bracketed code (QA OBS-R2-003).
async fn verdict(partition: Partition, mode: ValidationMode, height: u64) -> Result<(), String> {
    let (node, kp, _temp) = make_node().await;
    let pk = *kp.public_key();
    let other = crypto::KeyPair::generate();
    let other_pk = *other.public_key();

    let txs: Vec<Transaction> = match partition {
        Partition::R0 => {
            // Named producer is `other`, which the ledger never registers.
            let tx = withdrawal_tx(&other_pk, 1, 0x21);
            seed_bond_utxos(&node, &tx, &other_pk).await;
            vec![tx]
        }
        Partition::Bind => {
            // Deliberately UNSEEDED: this is the degraded-view shape. Every
            // input resolves to nothing, so `bond_inputs == 0` while the tx
            // declares 1 — the exact miscount INC-I-064 warns about.
            vec![withdrawal_tx(&pk, 1, 0x22)]
        }
        Partition::R3 => {
            let tx = withdrawal_tx_with_inputs(&pk, 2, 2, 0x23);
            seed_bond_utxos_split(&node, &tx, &pk, 1, &other_pk).await;
            vec![tx]
        }
        Partition::R2Drain => {
            // declared == allowance == 4 → full exit → must drain ALL owned.
            let tx = withdrawal_tx_with_inputs(&pk, HELD, 3, 0x24);
            seed_bond_utxos(&node, &tx, &pk).await;
            seed_owned_bond_utxos(&node, &pk, 0x25, 2).await;
            vec![tx]
        }
        Partition::R1 => {
            let tx = withdrawal_tx(&pk, HELD + 1, 0x26);
            seed_bond_utxos(&node, &tx, &pk).await;
            vec![tx]
        }
        Partition::R4 => {
            // `prior` sits at a LOWER index and its outputs are spent by the
            // withdrawal. `earlier_tx_hashes` is read from the block, not from
            // the UTXO view, so Replay sees it in full.
            let prior = add_bond_tx(&node, &pk, 1, 0x27);
            let tx = withdrawal_tx_chained(&pk, 2, 1, 0x28, &prior, 1);
            seed_bond_utxos(&node, &withdrawal_tx_with_inputs(&pk, 1, 1, 0x28), &pk).await;
            vec![prior, tx]
        }
    };

    verdict_in_mode(&node, &pk, HELD, 0, txs, height, mode).await
}

fn assert_rejected_with(result: &Result<(), String>, code: &str, ctx: &str) {
    let msg = result
        .as_ref()
        .err()
        .unwrap_or_else(|| panic!("{ctx}: expected a rejection, got Ok"));
    assert!(
        msg.contains(code),
        "{ctx}: rejected, but not with {code}. got: {msg}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// PA — Replay: the admission-only rules must yield
// ═══════════════════════════════════════════════════════════════════════════

/// O1,O2 × PA × {IP-R0, IP-BIND, IP-R3, IP-R2D} — **RED today.**
///
/// The M1 gate hard-`bail!`s in every mode, so the operator `recover` tool
/// (`operations/chain.rs:354`) aborts the WHOLE reindex on the first canonical
/// block whose Bond inputs are not yet in the replayed view — a block that is
/// already consensus-valid history.
#[tokio::test]
async fn req_i180_003_replay_tolerates_the_admission_only_rules() {
    for partition in [
        Partition::R0,
        Partition::Bind,
        Partition::R3,
        Partition::R2Drain,
    ] {
        assert!(partition.is_admission_only(), "harness: partition table");
        let result = verdict(partition, ValidationMode::Replay, POST_AH).await;
        assert!(
            result.is_ok(),
            "S3/F2: {partition:?} must be TOLERATED under ValidationMode::Replay. \
             Every term of this rule is read from the pre-block UTXO view, which \
             Replay legitimately sees degraded (INC-I-064). A hard bail here aborts \
             the operator reindex on already-canonical history. got: {}",
            result.unwrap_err()
        );
    }
}

/// O1,O2 × PA × {IP-R1, IP-R4} — **GREEN today, must STAY green.**
///
/// The counterweight to the test above: a carve-out that yields on every rule
/// makes Replay accept blocks Full rejects, which is the INC-I-034 divergence
/// class in a different costume.
#[tokio::test]
async fn req_i180_003_replay_keeps_the_allowance_and_same_block_rules_strict() {
    for partition in [Partition::R1, Partition::R4] {
        assert!(!partition.is_admission_only(), "harness: partition table");
        let result = verdict(partition, ValidationMode::Replay, POST_AH).await;
        assert_rejected_with(
            &result,
            partition.code(),
            &format!(
                "S3/F2: {partition:?} reads no term from the UTXO view \
                 (R1 reads the ProducerSet allowance, R4 reads the block's own \
                 earlier transaction hashes), so Replay must stay STRICT"
            ),
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PB, PC — Full and Light must not move
// ═══════════════════════════════════════════════════════════════════════════

/// O1,O2,O3 × {PB,PC} × all five reject partitions — **GREEN today, must STAY
/// green.** Post-AH determinism for the two ADMISSION modes is the property the
/// carve-out must not spend. Gossip-received blocks reach only these two.
#[tokio::test]
async fn req_i180_003_full_and_light_verdicts_are_unchanged_by_the_carve_out() {
    for partition in [
        Partition::R0,
        Partition::Bind,
        Partition::R3,
        Partition::R2Drain,
        Partition::R1,
    ] {
        let full = verdict(partition, ValidationMode::Full, POST_AH).await;
        let light = verdict(partition, ValidationMode::Light, POST_AH).await;
        assert_rejected_with(&full, partition.code(), &format!("{partition:?} in Full"));
        assert_rejected_with(&light, partition.code(), &format!("{partition:?} in Light"));
        // O3
        assert_eq!(
            full.is_ok(),
            light.is_ok(),
            "{partition:?}: Full and Light must agree post-AH (INC-I-034 class)"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PD — below AH #23 nothing is gated, in any mode
// ═══════════════════════════════════════════════════════════════════════════

/// O1,O3 × PD × all five reject partitions — **GREEN today, must STAY green.**
/// M1's guarantee is a zero-deletion diff below the gate; S3 must not spend it.
#[tokio::test]
async fn req_i180_003_pre_activation_is_mode_invariant() {
    for partition in [
        Partition::R0,
        Partition::Bind,
        Partition::R3,
        Partition::R2Drain,
        Partition::R1,
    ] {
        for mode in [
            ValidationMode::Full,
            ValidationMode::Light,
            ValidationMode::Replay,
        ] {
            let result = verdict(partition, mode, PRE_AH).await;
            assert!(
                result.is_ok(),
                "pre-AH invariance: {partition:?} in {mode:?} at h={PRE_AH} must be \
                 admitted — below AH #23 the gate is skipped whole and the historical \
                 silent-skip path is bit-identical. got: {}",
                result.unwrap_err()
            );
        }
    }
}

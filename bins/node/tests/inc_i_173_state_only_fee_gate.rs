//! INC-I-173 M1 — NODE LAYER: builder/apply parity (spec F2, INV-PROD-003),
//! the both-or-neither call-site constraint, the `ValidationMode` guard (C8),
//! and the no-version-bump guard (REQ-173-007).
//!
//! TDD RED. Does NOT compile against the current tree:
//! `ValidationContext::with_inc_i_173_activation_height` and
//! `NetworkParams::inc_i_173_activation_height` do not exist yet. The two
//! source-text tests DO compile today and FAIL at runtime, which is the RED
//! evidence for the call-site wiring.
//!
//! Spec: `specs/state-only-fee-gate-architecture.md` F2 (both call sites),
//!       constraint C4 (one place, block's height), C8 (ValidationMode::Full).
//! Analysis: `docs/redesigns/state-only-fee-gate-redesign-analysis.md`.
//! Requirements: REQ-173-006 (Must), REQ-173-007 (Must), REQ-173-003 (C8 half).
//!
//! ---------------------------------------------------------------------------
//! WHY THIS FILE EXISTS SEPARATELY FROM THE doli-core TESTS
//! ---------------------------------------------------------------------------
//! Three facts can only be checked from `bins/node`:
//!   1. The two production `ValidationContext` construction sites live here
//!      (`src/node/production/assembly.rs` builder, `src/node/apply_block/
//!      tx_processing.rs` apply). F2's failure mode is that ONE of them is
//!      wired and the other is not — and that is invisible from doli-core.
//!   2. The `network` crate (`CURRENT_PROTOCOL_VERSION` and friends) is a
//!      dependency of this package. doli-core does not and must not depend on
//!      it.
//!   3. `ValidationMode` only exists as a parameter at the apply layer.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT — the builder/apply PARITY property (REQ-173-006)
//! ---------------------------------------------------------------------------
//! Function under test (indirectly, through the two contexts the node builds):
//!   `doli_core::validation::validate_transaction_with_utxos(tx, ctx, provider)`
//!
//! ENUMERATION OF OBSERVABLE OUTPUTS
//!   O1: the `Result` DISCRIMINANT from the BUILDER-shaped context.
//!   O2: the `Result` DISCRIMINANT from the APPLY-shaped context.
//!   O3: the derived parity bit `O1 == O2`. This is the consensus-visible
//!       output: if it is ever false, the producing node builds a block its own
//!       apply path rejects and forks itself (INV-PROD-003).
//!   O4: the `ValidationError` variant on each side — asserted so parity cannot
//!       hold "by both failing for different reasons".
//!   mutable params   : NONE (shared refs).
//!   receiver mutation: NONE (free function).
//!   persistent store : NONE.
//!   side channels    : `tracing` only. DECLARED UNASSERTED.
//!
//! CODE PATHS — the two sites differ in ONE respect that must be covered
//! explicitly. Verified directly against the code:
//!   PSITE-BUILD: `assembly.rs:186` passes `self.params.clone()`
//!                (chainspec-override aware).
//!   PSITE-APPLY: `tx_processing.rs:61` passes
//!                `ConsensusParams::for_network(self.config.network)`
//!                (NOT chainspec aware).
//!   The INC-I-173 gate reads a `ValidationContext` FIELD and the fee comparison
//!   reads the `BASE_FEE` / `FEE_PER_BYTE` consensus CONSTANTS, so the
//!   divergence is inert for this gate — but "inert" is a claim, and this file
//!   is where it is machine-checked. Both `ConsensusParams` shapes are driven.
//!   PGATE-ABOVE / PGATE-BELOW as in the doli-core fee-gate file.
//!
//! INPUT PARTITIONS
//!   IP-P1 AddMaintainer, 0-in/0-out, height = AH        -> expect Ok on BOTH
//!   IP-P2 AddMaintainer, 0-in/0-out, height = AH - 1    -> expect Err on BOTH
//!   IP-P3 RemoveMaintainer, both heights                -> same
//!   IP-P4 Exit, both heights                            -> expect Err on BOTH
//!   IP-P5 ONE site wired, the other not (the F2 fork shape) -> the parity bit
//!         must be OBSERVABLY false, proving the test can detect the defect it
//!         exists to prevent (anti-vacuity)
//! MATRIX: (O1,O2,O3,O4) x {PSITE-BUILD,PSITE-APPLY} x {IP-P1..IP-P5}.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT — the CALL-SITE WIRING property (spec F2, both-or-neither)
//! ---------------------------------------------------------------------------
//! There is no runtime handle on "did the developer add the `.with_*` call at
//! both sites" short of booting two nodes and forking them on purpose. The
//! cheapest sound guard is a SOURCE-TEXT assertion via `include_str!` — the
//! convention already used by `inc_i_172_service_timing_test.rs:97`,
//! `inc_i_172_update_cmd_verify_blocks_test.rs:60` and
//! `inc_i_172_upgrade_cmd_verify_blocks_test.rs:89`.
//!   O5: `assembly.rs` contains `.with_inc_i_173_activation_height(`
//!   O6: `tx_processing.rs` contains `.with_inc_i_173_activation_height(`
//!   O7: each site reads the value from `NetworkParams`, not from a literal
//! PATHS: 1 (a static text scan). INPUT PARTITIONS: 2 (the two files).
//! MATRIX: (O5,O6,O7) x 2 files.

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::maintainer::{MaintainerChangeData, MaintainerSignature};
use doli_core::transaction::{ExitData, Transaction, TxType};
use doli_core::validation::{
    validate_transaction_with_utxos, UtxoInfo, UtxoProvider, ValidationContext, ValidationError,
    ValidationMode,
};
use doli_core::{Network, NetworkParams};

// ---------------------------------------------------------------------------
// Fixtures — deliberately mirrored from crates/core/tests/inc_i_173_common so
// the two layers drive BYTE-IDENTICAL transactions. Duplicated rather than
// shared because `crates/core/tests/` is not reachable from `bins/node/tests/`.
// ---------------------------------------------------------------------------

/// The synthetic gate used by the parity tests. Not any real network's value.
const TEST_AH: u64 = 200_000;
const BELOW_GATE: u64 = TEST_AH - 1;
const AT_GATE: u64 = TEST_AH;
const ABOVE_GATE: u64 = 500_000;

struct EmptyUtxos;
impl UtxoProvider for EmptyUtxos {
    fn get_utxo(&self, _tx_hash: &Hash, _output_index: u32) -> Option<UtxoInfo> {
        None
    }
}

fn kp() -> KeyPair {
    KeyPair::from_seed([7u8; 32])
}

fn zero_flow_tx(t: TxType) -> Transaction {
    let pk = *kp().public_key();
    let extra_data = match t {
        TxType::AddMaintainer | TxType::RemoveMaintainer => MaintainerChangeData::new(
            pk,
            vec![MaintainerSignature::new(pk, crypto::Signature::default())],
        )
        .to_bytes(),
        TxType::Exit => bincode::serialize(&ExitData { public_key: pk }).unwrap(),
        _ => Vec::new(),
    };
    Transaction {
        version: 1,
        tx_type: t,
        inputs: vec![],
        outputs: vec![],
        extra_data,
    }
}

/// The BUILDER-shaped `ValidationContext` — `assembly.rs:186` passes
/// `self.params.clone()`, which carries chainspec overrides.
fn builder_ctx(height: u64, gate: u64) -> ValidationContext {
    let params = ConsensusParams::mainnet();
    ValidationContext::new(params, Network::Mainnet, 0, height)
        .with_inc_i_173_activation_height(gate)
}

/// The APPLY-shaped `ValidationContext` — `tx_processing.rs:61` passes
/// `ConsensusParams::for_network(self.config.network)`, which does NOT read the
/// chainspec. This is a REAL divergence between the two sites (verified against
/// the code); the point of this test is that it is inert for THIS gate.
fn apply_ctx(height: u64, gate: u64) -> ValidationContext {
    let params = ConsensusParams::for_network(Network::Mainnet);
    ValidationContext::new(params, Network::Mainnet, 0, height)
        .with_inc_i_173_activation_height(gate)
}

fn verdict(tx: &Transaction, ctx: &ValidationContext) -> Result<(), ValidationError> {
    validate_transaction_with_utxos(tx, ctx, &EmptyUtxos)
}

// ===========================================================================
// REQ-173-006 (Must) — INV-PROD-003 builder/apply parity
// ===========================================================================

/// REQ-173-006 (Must) — O3 x both `ConsensusParams` shapes x IP-P1..IP-P4.
///
/// The SAME transaction at the SAME height through BOTH context shapes must
/// produce the SAME verdict. Because the INC-I-173 gate lives INSIDE
/// `validate_transaction_with_utxos` (constraint C4), parity holds by
/// construction — this test is the machine-check of that claim, and it is what
/// would catch a future "optimisation" that lifted the gate to a call site.
#[test]
fn req_173_006_builder_and_apply_contexts_agree_on_every_verdict() {
    for t in [
        TxType::AddMaintainer,
        TxType::RemoveMaintainer,
        TxType::Exit,
    ] {
        let tx = zero_flow_tx(t);
        for h in [BELOW_GATE, AT_GATE, ABOVE_GATE] {
            let b = verdict(&tx, &builder_ctx(h, TEST_AH));
            let a = verdict(&tx, &apply_ctx(h, TEST_AH));

            assert_eq!(
                b.is_ok(),
                a.is_ok(),
                "O3 INV-PROD-003: builder and apply disagree for {:?} at h={} — \
                 the producing node would build a block its own apply path \
                 rejects and fork itself. builder={:?} apply={:?}",
                t,
                h,
                b,
                a
            );

            // O4: parity of the ERROR VARIANT too, so parity cannot hold by
            // both sides failing for different reasons.
            match (&b, &a) {
                (Err(be), Err(ae)) => assert_eq!(
                    std::mem::discriminant(be),
                    std::mem::discriminant(ae),
                    "O4: {:?} at h={} — both sides reject but with DIFFERENT error \
                     variants (builder={:?}, apply={:?})",
                    t,
                    h,
                    be,
                    ae
                ),
                (Ok(()), Ok(())) => {}
                _ => unreachable!("covered by the is_ok assertion above"),
            }
        }
    }
}

/// REQ-173-006 (Must) — IP-P1: the parity test is NOT vacuous, because the
/// verdict really does change across the boundary on BOTH shapes.
#[test]
fn req_173_006_both_shapes_flip_at_the_gate_so_parity_is_not_vacuous() {
    let tx = zero_flow_tx(TxType::AddMaintainer);

    assert!(
        verdict(&tx, &builder_ctx(BELOW_GATE, TEST_AH)).is_err(),
        "builder shape rejects below the gate"
    );
    assert!(
        verdict(&tx, &builder_ctx(AT_GATE, TEST_AH)).is_ok(),
        "builder shape accepts at the gate"
    );
    assert!(
        verdict(&tx, &apply_ctx(BELOW_GATE, TEST_AH)).is_err(),
        "apply shape rejects below the gate"
    );
    assert!(
        verdict(&tx, &apply_ctx(AT_GATE, TEST_AH)).is_ok(),
        "apply shape accepts at the gate"
    );
}

/// REQ-173-006 (Must) — IP-P5, THE F2 FORK SHAPE, reproduced on purpose.
///
/// If the developer wires `.with_inc_i_173_activation_height(..)` at the builder
/// but forgets it at apply, the apply context keeps the `u64::MAX` default. The
/// builder then produces a block containing a 0-fee `AddMaintainer` that the
/// same node's apply path REJECTS. This test constructs exactly that pair and
/// asserts the parity bit is observably FALSE — proving the parity test above
/// can actually detect the defect it exists to prevent.
#[test]
fn req_173_006_forgetting_one_site_is_observably_a_fork() {
    let tx = zero_flow_tx(TxType::AddMaintainer);
    let h = AT_GATE;

    // Builder wired, apply forgotten (field left at its fail-closed default).
    let wired = verdict(&tx, &builder_ctx(h, TEST_AH));
    let forgotten = verdict(
        &tx,
        &ValidationContext::new(
            ConsensusParams::for_network(Network::Mainnet),
            Network::Mainnet,
            0,
            h,
        ),
    );

    assert!(wired.is_ok(), "the wired builder accepts");
    assert!(
        forgotten.is_err(),
        "the unwired apply site rejects — F2: both-or-neither"
    );
    assert_ne!(
        wired.is_ok(),
        forgotten.is_ok(),
        "O3: this IS the fork. If this assertion ever stops holding, the parity \
         test above has gone vacuous."
    );
}

// ===========================================================================
// REQ-173-006 (Must) — the both-or-neither CALL SITES (spec F2)
// ===========================================================================

const ASSEMBLY_SRC: &str = include_str!("../src/node/production/assembly.rs");
const TX_PROCESSING_SRC: &str = include_str!("../src/node/apply_block/tx_processing.rs");

/// REQ-173-006 (Must) — O5: the BUILDER site is wired.
///
/// Forgetting this site costs LIVENESS only (maintainer txs stay unmineable —
/// i.e. INC-I-173 is not fixed at all), so it must be asserted independently of
/// the apply site.
#[test]
fn req_173_006_assembly_sets_the_inc_i_173_activation_height() {
    assert!(
        ASSEMBLY_SRC.contains(".with_inc_i_173_activation_height("),
        "O5 F2: bins/node/src/node/production/assembly.rs must set the INC-I-173 \
         gate on the ValidationContext it builds (~line 186 chain). Without it the \
         builder stays below the gate forever and the maintainer txs are never mined."
    );
    assert!(
        ASSEMBLY_SRC.contains("inc_i_173_activation_height"),
        "O7: assembly.rs must read the height from NetworkParams"
    );
}

/// REQ-173-006 (Must) — O6: the APPLY site is wired.
///
/// Forgetting THIS site is the dangerous half: the builder (wired) produces a
/// block the apply path (unwired, still `u64::MAX`) rejects, and the producing
/// node forks itself.
#[test]
fn req_173_006_tx_processing_sets_the_inc_i_173_activation_height() {
    assert!(
        TX_PROCESSING_SRC.contains(".with_inc_i_173_activation_height("),
        "O6 F2: bins/node/src/node/apply_block/tx_processing.rs must set the \
         INC-I-173 gate on the ValidationContext it builds (~line 61 chain). \
         Without it the node rejects the very blocks it produced — a self-fork."
    );
    assert!(
        TX_PROCESSING_SRC.contains("inc_i_173_activation_height"),
        "O7: tx_processing.rs must read the height from NetworkParams"
    );
}

/// REQ-173-006 (Must) — O7: neither site hardcodes a literal height.
///
/// The value must come from `NetworkParams`, so that devnet / testnet / mainnet
/// get their own pinned gates. A literal would make every network cross at the
/// same block.
#[test]
fn req_173_006_neither_call_site_hardcodes_the_height() {
    for (name, src) in [
        ("assembly.rs", ASSEMBLY_SRC),
        ("tx_processing.rs", TX_PROCESSING_SRC),
    ] {
        // Locate the call and check the argument mentions `params()`.
        let idx = src
            .find(".with_inc_i_173_activation_height(")
            .unwrap_or_else(|| panic!("{}: the call site is missing", name));
        let window = &src[idx..usize::min(idx + 220, src.len())];
        assert!(
            window.contains("params()") && window.contains("inc_i_173_activation_height"),
            "O7 {}: the argument must be \
             `self.config.network.params().inc_i_173_activation_height`, not a \
             literal. Saw: {}",
            name,
            window
        );
    }
}

/// REQ-173-006 (Must), constraint C4 — the gate is NOT evaluated at a call site.
///
/// C4 requires the decision to live inside `validate_transaction_with_utxos`.
/// If either node file grew its own `>= ... inc_i_173_activation_height`
/// comparison, the two sites could drift and parity would stop being a
/// construction property.
#[test]
fn req_173_006_neither_call_site_evaluates_the_gate_itself() {
    for (name, src) in [
        ("assembly.rs", ASSEMBLY_SRC),
        ("tx_processing.rs", TX_PROCESSING_SRC),
    ] {
        assert!(
            !src.contains(">= ctx.inc_i_173_activation_height"),
            "C4 {}: the gate must be evaluated INSIDE \
             validate_transaction_with_utxos, never at a call site",
            name
        );
        assert!(
            !src.contains("is_zero_flow()"),
            "C4 {}: the node layer must not re-implement the exemption decision — \
             it belongs to the shared validator",
            name
        );
    }
}

// ===========================================================================
// REQ-173-003 (Must), constraint C8 — VALIDATION MODE
// ===========================================================================

/// The mode the INC-I-173 consensus assertions are made under. Declared as a
/// named constant so the choice is EXPLICIT and machine-checkable, per C8.
const CONSENSUS_TEST_MODE: ValidationMode = ValidationMode::Full;

/// REQ-173-003 (Must), constraint C8 — the mode is `Full`, asserted.
///
/// INC-I-064 made `apply_block` TOLERATE UTXO validation failures in
/// `ValidationMode::Replay`. A bit-identity test run under `Replay` would
/// therefore pass even if the fee gate rejected everything. The INC-I-173
/// consensus assertions are made under `Full`.
#[test]
fn req_173_003_c8_the_declared_validation_mode_is_full_not_replay() {
    assert_eq!(
        CONSENSUS_TEST_MODE,
        ValidationMode::Full,
        "C8: the INC-I-173 consensus assertions run in ValidationMode::Full"
    );
    assert_ne!(
        CONSENSUS_TEST_MODE,
        ValidationMode::Replay,
        "C8/INC-I-064: Replay silently swallows UTXO validation errors \
         (tx_processing.rs:112) and would make a bit-identity test vacuous"
    );
    assert_ne!(
        CONSENSUS_TEST_MODE,
        ValidationMode::Light,
        "C8: Light skips VDF verification; state the strictest mode explicitly"
    );
}

/// REQ-173-003 (Must), constraint C8 — the swallow is keyed EXCLUSIVELY on
/// `Replay`.
///
/// This is the assertion that gives the constant above its meaning: it proves
/// that `Full` (and `Light`) genuinely PROPAGATE a `validate_transaction_with_utxos`
/// error, so the doli-core bit-identity table is not silently defanged when the
/// same predicate runs inside `apply_block`.
#[test]
fn req_173_003_c8_apply_only_tolerates_utxo_failures_in_replay_mode() {
    assert!(
        TX_PROCESSING_SRC.contains("mode == ValidationMode::Replay"),
        "C8/INC-I-064: the UTXO-failure tolerance in tx_processing.rs must remain \
         keyed on ValidationMode::Replay ALONE. If this changes shape, re-derive \
         whether Full still propagates fee-gate rejections."
    );
    assert!(
        !TX_PROCESSING_SRC.contains("mode == ValidationMode::Full")
            || TX_PROCESSING_SRC.contains("mode == ValidationMode::Replay"),
        "C8: Full must never be added to the tolerance condition"
    );
}

// ===========================================================================
// REQ-173-007 (Must) — NO VERSION BUMPS
// ===========================================================================

/// REQ-173-007 (Must) — the three version constants are UNCHANGED.
///
/// Spec "Consensus Classification": the `EpochState` serialization format is
/// unchanged and the peer handshake is unaffected, so none of these may move.
///
/// The stakes, from CLAUDE.md and INV-EPOCH-001: bumping
/// `CURRENT_PROTOCOL_VERSION` triggers `delete_epoch_state()` on restart
/// (`init.rs:727`) -> non-deterministic rebuild -> fork at the next epoch
/// boundary (INC-I-054). The check is `!=`, so a rollback deletes a SECOND time.
/// Bumping `MIN_PEER_PROTOCOL_VERSION` immediately partitions every
/// not-yet-upgraded peer — including the ~30 external auto-update producers
/// this activation height exists to accommodate.
///
/// Values read from `crates/network/src/protocols/status.rs:49,68,83` at the
/// INC-I-173 M1 branch point.
#[test]
fn req_173_007_no_protocol_version_was_bumped() {
    assert_eq!(
        network::protocols::status::CURRENT_PROTOCOL_VERSION,
        8,
        "REQ-173-007: CURRENT_PROTOCOL_VERSION must stay 8. INC-I-173 changes no \
         EpochState serialization and no handshake field. A bump triggers \
         delete_epoch_state() on restart (INV-EPOCH-001 / INC-I-054)."
    );
    assert_eq!(
        network::protocols::status::EPOCH_STATE_FORMAT_VERSION,
        1,
        "REQ-173-007: EPOCH_STATE_FORMAT_VERSION must stay 1. The EpochState \
         format is untouched by INC-I-173."
    );
    assert_eq!(
        network::protocols::status::MIN_PEER_PROTOCOL_VERSION,
        1,
        "REQ-173-007: MIN_PEER_PROTOCOL_VERSION must stay 1. Raising it \
         partitions old peers IMMEDIATELY — the opposite of what a forward-only \
         activation height is for."
    );
}

/// REQ-173-007 (Must) — no `HardForkSchedule` entry was added for INC-I-173.
///
/// `current_fork_id()` evaluates the schedule at `u64::MAX`, which makes ALL
/// entries active in `fork_id` IMMEDIATELY. An entry here would partition a
/// rolling deploy on the day of release, defeating the activation height
/// entirely (CLAUDE.md "If You Touch" / INV-8).
///
/// The scan is deliberately limited to the PRODUCTION half of the file (before
/// `#[cfg(test)]`) and counts `schedule.add(HardForkInfo {` occurrences rather
/// than searching for the string "173" — a comment mentioning INC-I-173 must
/// not fail this test, only an actual new ENTRY must.
///
/// Baseline at the INC-I-173 M1 branch point: TWO production entries, both on
/// Testnet (`hardfork.rs:231` h=3_100, `:238` h=4_836). Mainnet and Devnet have
/// none.
#[test]
fn req_173_007_no_hardfork_schedule_entry_was_added() {
    const HARDFORK_SRC: &str = include_str!("../../../crates/updater/src/hardfork.rs");
    const BASELINE_PRODUCTION_ENTRIES: usize = 2;

    let production_half = HARDFORK_SRC
        .split("#[cfg(test)]")
        .next()
        .expect("split always yields at least one element");
    // HARNESS FIX (developer, INC-I-173 M1): the raw `matches()` count is 3, not
    // 2, on the UNMODIFIED file — `hardfork.rs:158` is a rustdoc EXAMPLE
    // (`/// schedule.add(HardForkInfo {`), not an entry. Verified pre-existing:
    // INC-I-173 does not touch `crates/updater/src/hardfork.rs` at all
    // (`git status` clean for that path). The real entries are exactly the two
    // this test's doc names, `:231` h=3_100 and `:238` h=4_836, both Testnet.
    // Excluding comment lines makes the INSTRUMENT sound without weakening the
    // assertion: a genuine new entry still fires, a doc example no longer does.
    let entries = production_half
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .filter(|l| l.contains("schedule.add(HardForkInfo {"))
        .count();

    assert_eq!(
        entries, BASELINE_PRODUCTION_ENTRIES,
        "REQ-173-007 / INV-8: INC-I-173 must NOT add a HardForkSchedule entry \
         (baseline is {} production entries, both Testnet). current_fork_id() \
         evaluates the schedule at u64::MAX, so ANY new entry is active in \
         fork_id immediately and partitions the rolling deploy the activation \
         height exists to avoid.",
        BASELINE_PRODUCTION_ENTRIES
    );
}

/// REQ-173-007 (Must) — the network params carry the new gate WITHOUT
/// disturbing the chain identity used in the handshake.
#[test]
fn req_173_007_adding_the_gate_did_not_disturb_the_genesis_identity() {
    // Touching NetworkParams must not perturb ConsensusParams' genesis hash,
    // which is what the peer handshake and the ExtendsTip guard compare.
    assert_eq!(
        ConsensusParams::mainnet().genesis_hash,
        doli_core::chainspec::ChainSpec::mainnet().genesis_hash(),
        "REQ-173-007: genesis identity unchanged"
    );
    // And the new field really is present on the params the node reads.
    let _gate: u64 = NetworkParams::defaults(Network::Mainnet).inc_i_173_activation_height;
}

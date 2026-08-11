//! INC-I-173 M3 — ITEM 5 / spec F4: delete `Transaction::is_state_only()`, route
//! both production callers on `is_zero_flow()`, and wire the activation height
//! into the four `ValidationContext` sites that are still unwired
//! (AUDIT-P3-002, AUDIT-P3-003, FM-10).
//!
//! TDD RED. Every assertion in this file COMPILES against the tree at
//! `32e0a650` and FAILS at runtime. That is deliberate: `is_state_only` still
//! exists, so a "the method is gone" test cannot be written as a call — a deleted
//! function is a COMPILE error, not an assertion. The sound instrument is a
//! SOURCE-TEXT scan, the idiom this suite already uses at
//! `bins/node/tests/inc_i_173_state_only_fee_gate.rs:282-283,493`.
//!
//! Contract: `docs/.workflow/inc-i-173-M3-design-contract.md` Item 5.
//!
//! ---------------------------------------------------------------------------
//! BASELINE MEASURED AGAINST THE TREE AT `32e0a650` (not inferred)
//! ---------------------------------------------------------------------------
//! | file | `ValidationContext::new(` | `with_inc_i_173_...(` | `with_sig_verification_height(` |
//! |---|---|---|---|
//! | `crates/mempool/src/pool.rs`                  | 2 | 0 | 2 |
//! | `bins/node/src/node/validation_checks.rs`     | 2 | 0 | 1 |
//! | `bins/node/src/node/production/assembly.rs`   | 1 | 1 | 1 |
//! | `bins/node/src/node/apply_block/tx_processing.rs` | 1 | 1 | 1 |
//! Production callers of `is_state_only()`: `crates/rpc/src/methods/transaction.rs:203`
//! and `bins/node/src/node/validation_checks.rs:915`. Definition:
//! `crates/core/src/transaction/core.rs:463`.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT — the SOURCE-TEXT property
//! ---------------------------------------------------------------------------
//!   O1: `crates/core/src/transaction/core.rs` no longer DEFINES `is_state_only`.
//!   O2: neither production caller mentions `is_state_only`.
//!   O3: both production callers route on `is_zero_flow()`.
//!   O4: `with_inc_i_173_activation_height(` appears once per
//!       `ValidationContext::new(` in each of the two newly-wired files.
//!   O5: each wiring reads the value from `NetworkParams`, never a literal.
//!   O6: the UNRELATED `with_sig_verification_height` drift at
//!       `validation_checks.rs:103` is NOT repaired (spec Option C, out of scope
//!       — it is its own consensus question and must not ride this commit).
//!   PATHS: 1 (a static text scan). INPUT PARTITIONS: 5 (the five files).
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT — the BEHAVIOURAL property (FM-10)
//! ---------------------------------------------------------------------------
//! Function under test: `Transaction::is_zero_flow()`, as the ROUTING predicate
//! that replaces `is_state_only()`.
//!   O7: the returned `bool`. Consumed as a routing decision: `true` sends the
//!       transaction down `Mempool::add_system_transaction` at `fee_rate = 0`;
//!       `false` sends it down `add_transaction`, where it is fee-accounted.
//!   mutable params   : NONE. receiver mutation: NONE. persistent store: NONE.
//!   side channels    : NONE.
//!   PATHS: the `inputs.is_empty() && outputs.is_empty() && allows_empty_io()`
//!          conjunction — three ways to be `false`, one to be `true`.
//!   INPUT PARTITIONS: the FM-10 delta types x {0-in/0-out, with outputs}.
//!
//! ---------------------------------------------------------------------------
//! BEHAVIOURAL DELTAS THIS ITEM SHIPS — stated, not discovered later
//! ---------------------------------------------------------------------------
//! Routing moves from TYPE-based to SHAPE-based, so a transaction takes the
//! 0-fee system lane only when it is genuinely 0-in/0-out AND its type is exempt:
//!   * `ClaimReward` / `ClaimBond` have OUTPUTS, so they stop being system-routed
//!     at `fee_rate = 0`. That is the point of FM-10: it closes the free-relay
//!     amplification that can evict legitimate 0-fee governance transactions via
//!     `evict_lowest_fee`.
//!   * `Exit` / `SlashProducer` / `RequestWithdrawal` with 0 inputs move from
//!     "admitted at fee 0, gossiped, then silently never mined" to "rejected at
//!     the mempool". An improvement — the same silent-limbo class as INC-I-173
//!     itself — but a USER-VISIBLE change.
//!   * `PriceAttestation` loses system routing (`allows_empty_io` is false for
//!     it). No live impact: `oracle_activation_height = u64::MAX` on every
//!     network. Reclassifying it is spec Option B and is explicitly NOT in M3
//!     scope.
//!
//! ---------------------------------------------------------------------------
//! CONSENSUS HONESTY — `validate_block_for_apply` (AUDIT-P3-003)
//! ---------------------------------------------------------------------------
//! Of the four newly-wired sites, `validation_checks.rs:283`
//! (`validate_block_for_apply`) is a CONSENSUS PATH. Left unwired it holds
//! `u64::MAX` while `apply_block/tx_processing.rs` is wired, so ABOVE the gate
//! the two paths DISAGREE: one rejects a block carrying a maintainer tx that the
//! other accepts. Wiring it is therefore consensus-visible above the gate, and it
//! is a DIVERGENCE FIX, not a cosmetic one. It needs no new height because it
//! rides the same already-committed `inc_i_173_activation_height`. The commit
//! message must say this under INV-12 rather than repeat the "F4 is entirely
//! non-consensus" shorthand.
//!
//! CORRECTED 2026-08-11 (M3 review iteration 1, REV-173-M3-001). This header used
//! to call that height "never-crossed (mainnet `u64::MAX`; testnet `133_000`, tip
//! ~130_291)". The testnet half is FALSE: the live testnet tip measured `134_159`
//! on v6.24.1 (agreed across RPC 8500/8501/8502), ~1,159 blocks ABOVE the gate and
//! climbing. **On testnet this wiring is active as soon as the binary lands**, so
//! the testnet deploy is a SYNCHRONIZED stop-all/start-all, never a rolling
//! restart (INV-8 / INC-I-062). Mainnet (`u64::MAX`) and devnet (`0`) are
//! unaffected, and no already-valid block becomes invalid: above the gate the
//! predicate is strictly MORE permissive. M2 re-pins the testnet height above the
//! then-current tip, and must re-verify the tip immediately before pinning.

use doli_core::maintainer::{MaintainerChangeData, MaintainerSignature};
use doli_core::transaction::{
    ClaimBondData, ClaimData, ExitData, Output, Transaction, TxType, WithdrawalRequestData,
};

// ---------------------------------------------------------------------------
// Source under scan
// ---------------------------------------------------------------------------

const TX_CORE_SRC: &str = include_str!("../../../crates/core/src/transaction/core.rs");
const RPC_TX_SRC: &str = include_str!("../../../crates/rpc/src/methods/transaction.rs");
const MEMPOOL_POOL_SRC: &str = include_str!("../../../crates/mempool/src/pool.rs");
const VALIDATION_CHECKS_SRC: &str = include_str!("../src/node/validation_checks.rs");

/// Count occurrences of `needle` in the NON-COMMENT lines of `src`.
///
/// Comment lines are excluded for the same reason the M1 hardfork scan excludes
/// them (`inc_i_173_state_only_fee_gate.rs:511-518`): a doc comment that
/// mentions a deleted symbol must not fail a test whose subject is the CODE. It
/// also keeps the assertions honest in the other direction — the developer
/// cannot satisfy them by writing the token into a comment.
fn code_occurrences(src: &str, needle: &str) -> usize {
    src.lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.starts_with("//") && !t.starts_with("*") && !t.starts_with("/*")
        })
        .filter(|l| l.contains(needle))
        .count()
}

// ===========================================================================
// O1 / O2 / O3 — `is_state_only` is gone; both callers route on `is_zero_flow`
// ===========================================================================

/// AUDIT-P3-002 (Must) — `Transaction::is_state_only()` is DELETED.
///
/// Its doc contract is false on three counts: it claims the listed types "have no
/// UTXO inputs by design" while `ClaimReward` and `ClaimBond` carry OUTPUTS; it
/// claims spam protection "by requiring a registered producer bond" while
/// `Exit`/`SlashProducer` verify no actor signature at all; and it had drifted
/// from `TxType::allows_empty_io()`, the exhaustive authority M1 introduced.
/// Keeping a second, hand-maintained list is exactly the defect INC-I-173 is.
///
/// A deleted function cannot be called from a test — the call would be a compile
/// error, not a failing assertion — so absence is asserted on the SOURCE TEXT.
#[test]
fn audit_p3_002_is_state_only_no_longer_exists() {
    assert_eq!(
        code_occurrences(TX_CORE_SRC, "fn is_state_only"),
        0,
        "AUDIT-P3-002 / F4: `Transaction::is_state_only()` must be DELETED from \
         crates/core/src/transaction/core.rs. A second hand-maintained \"state \
         only\" list next to the exhaustive `TxType::allows_empty_io()` is the \
         precise defect shape that made AddMaintainer/RemoveMaintainer relayable \
         but unmineable."
    );
}

/// AUDIT-P3-002 (Must) — the RPC caller routes on `is_zero_flow()`.
///
/// `crates/rpc/src/methods/transaction.rs:203` decides whether a
/// client-submitted transaction takes the 0-fee system lane. It is the
/// user-reachable half of the routing decision.
#[test]
fn audit_p3_002_rpc_send_transaction_routes_on_is_zero_flow() {
    assert_eq!(
        code_occurrences(RPC_TX_SRC, "is_state_only"),
        0,
        "AUDIT-P3-002: crates/rpc/src/methods/transaction.rs must no longer \
         mention `is_state_only`"
    );
    assert!(
        code_occurrences(RPC_TX_SRC, "is_zero_flow()") >= 1,
        "AUDIT-P3-002 / F4: the RPC submit path must route on `is_zero_flow()`. \
         Shape-based routing is what keeps a transaction carrying OUTPUTS out of \
         the 0-fee system lane (the mint guard, constraint C2, lives inside the \
         predicate)."
    );
}

/// AUDIT-P3-002 (Must) — the node's gossip-admission caller routes on
/// `is_zero_flow()`.
///
/// `bins/node/src/node/validation_checks.rs:915` is the peer-reachable half: it
/// decides how a transaction arriving over gossip is admitted. Both callers must
/// move together — one migrated and one not is a routing split between the RPC
/// and gossip entry points for the same transaction.
#[test]
fn audit_p3_002_node_gossip_admission_routes_on_is_zero_flow() {
    assert_eq!(
        code_occurrences(VALIDATION_CHECKS_SRC, "is_state_only"),
        0,
        "AUDIT-P3-002: bins/node/src/node/validation_checks.rs must no longer \
         mention `is_state_only`"
    );
    assert!(
        code_occurrences(VALIDATION_CHECKS_SRC, "is_zero_flow()") >= 1,
        "AUDIT-P3-002 / F4: the node's gossip-admission path must route on \
         `is_zero_flow()`, in step with the RPC path. One migrated and one not is \
         a routing split between the two entry points for the SAME transaction."
    );
}

// ===========================================================================
// O4 / O5 — AUDIT-P3-003: the four unwired ValidationContext sites
// ===========================================================================

/// AUDIT-P3-003 (Must) — BOTH mempool `ValidationContext` sites carry the gate.
///
/// `crates/mempool/src/pool.rs:363` (`add_transaction`) and `:742`
/// (`add_system_transaction`). Node-local admission policy. Unwired they hold the
/// fail-closed `u64::MAX` default, so above the gate the mempool refuses a
/// maintainer transaction the block validator would accept — the transaction
/// stays unmineable for the opposite reason to the original bug.
///
/// The count is compared against `ValidationContext::new(` in the same file, so
/// the assertion cannot be satisfied by wiring one site twice, and it keeps
/// holding if a third context is ever added.
#[test]
fn audit_p3_003_both_mempool_validation_contexts_carry_the_gate() {
    let contexts = code_occurrences(MEMPOOL_POOL_SRC, "ValidationContext::new(");
    let wired = code_occurrences(MEMPOOL_POOL_SRC, ".with_inc_i_173_activation_height(");
    assert_eq!(
        contexts, 2,
        "baseline: crates/mempool/src/pool.rs is expected to build exactly 2 \
         ValidationContexts (add_transaction, add_system_transaction); found {}. \
         Re-measure before changing this test.",
        contexts
    );
    assert_eq!(
        wired, contexts,
        "AUDIT-P3-003: every ValidationContext in crates/mempool/src/pool.rs must \
         carry `.with_inc_i_173_activation_height(...)` ({} built, {} wired). An \
         unwired context holds the fail-closed u64::MAX default, so above the gate \
         the mempool REFUSES the maintainer transaction the block validator \
         ACCEPTS.",
        contexts, wired
    );
    assert!(
        MEMPOOL_POOL_SRC.contains(
            ".with_inc_i_173_activation_height(self.network.params().inc_i_173_activation_height)"
        ) || MEMPOOL_POOL_SRC.contains("inc_i_173_activation_height,"),
        "AUDIT-P3-003 / O5: the mempool must read the height from \
         `NetworkParams`, never from a literal. A literal cannot be re-pinned per \
         network and silently diverges from the validator."
    );
}

/// AUDIT-P3-003 (Must) — BOTH `validation_checks.rs` `ValidationContext` sites
/// carry the gate.
///
/// `:103` (`check_producer_eligibility`, header-only — the gate is unreachable
/// there, and it is wired anyway so the two contexts in one file cannot drift
/// apart) and `:283` (`validate_block_for_apply` — a CONSENSUS PATH; see the
/// CONSENSUS HONESTY note at the top of this file).
#[test]
fn audit_p3_003_both_validation_checks_contexts_carry_the_gate() {
    let contexts = code_occurrences(VALIDATION_CHECKS_SRC, "ValidationContext::new(");
    let wired = code_occurrences(VALIDATION_CHECKS_SRC, ".with_inc_i_173_activation_height(");
    assert_eq!(
        contexts, 2,
        "baseline: bins/node/src/node/validation_checks.rs is expected to build \
         exactly 2 ValidationContexts (check_producer_eligibility, \
         validate_block_for_apply); found {}. Re-measure before changing this test.",
        contexts
    );
    assert_eq!(
        wired, contexts,
        "AUDIT-P3-003: every ValidationContext in \
         bins/node/src/node/validation_checks.rs must carry \
         `.with_inc_i_173_activation_height(...)` ({} built, {} wired). \
         `validate_block_for_apply` feeds `validate_block_with_mode`; left unwired \
         it holds u64::MAX while apply_block/tx_processing.rs is wired, so above \
         the gate the two paths DISAGREE about the same block.",
        contexts, wired
    );
}

/// AUDIT-P3-003 (Must) — O6: the UNRELATED `with_sig_verification_height` drift
/// is NOT repaired.
///
/// `validation_checks.rs` builds two contexts but wires
/// `.with_sig_verification_height` on only ONE of them. That is its own consensus
/// question (spec Option C) and is explicitly out of M3 scope. Repairing it here
/// would smuggle a second consensus-visible change into a commit whose INV-12
/// analysis covers only the INC-I-173 gate.
///
/// Baseline measured at `32e0a650`: exactly 1.
#[test]
fn audit_p3_003_the_unrelated_sig_verification_drift_is_not_repaired() {
    const BASELINE_SIG_VERIFICATION_WIRINGS: usize = 1;
    assert_eq!(
        code_occurrences(VALIDATION_CHECKS_SRC, ".with_sig_verification_height("),
        BASELINE_SIG_VERIFICATION_WIRINGS,
        "AUDIT-P3-003 / O6: M3 must add ONLY \
         `.with_inc_i_173_activation_height(...)` at these sites. The \
         `with_sig_verification_height` drift (1 of 2 contexts wired) is spec \
         Option C, a separate consensus question, and must NOT ride this commit."
    );
}

// ===========================================================================
// O7 — FM-10: the routing delta, behaviourally
// ===========================================================================

fn kp() -> crypto::KeyPair {
    crypto::KeyPair::from_seed([7u8; 32])
}

fn tx(t: TxType, outputs: Vec<Output>, extra_data: Vec<u8>) -> Transaction {
    Transaction {
        version: 1,
        tx_type: t,
        inputs: vec![],
        outputs,
        extra_data,
    }
}

fn one_output() -> Vec<Output> {
    vec![Output::normal(
        1_000_000,
        crypto::hash::hash(b"inc-i-173-m3-recipient"),
    )]
}

/// FM-10 (Must) — `ClaimReward` and `ClaimBond` are NOT zero-flow, so they leave
/// the 0-fee system lane.
///
/// THE FM-10 delta. Both types carry a value OUTPUT, and `is_state_only()`
/// classified them state-only anyway — that is the free-relay amplification: an
/// attacker submits them at `fee_rate = 0`, they fill the mempool, and
/// `evict_lowest_fee` then evicts the legitimate 0-fee GOVERNANCE transactions
/// this whole incident exists to make mineable.
///
/// Both shapes are driven: with an output (the real shape) and 0-in/0-out (the
/// degenerate one). Neither may be zero-flow — the first fails the shape
/// conjunct, the second fails `allows_empty_io`.
#[test]
fn fm_10_claim_reward_and_claim_bond_are_not_zero_flow() {
    let pk = *kp().public_key();
    let cases: Vec<(TxType, Vec<u8>)> = vec![
        (
            TxType::ClaimReward,
            bincode::serialize(&ClaimData { public_key: pk }).unwrap(),
        ),
        (
            TxType::ClaimBond,
            bincode::serialize(&ClaimBondData { public_key: pk }).unwrap(),
        ),
    ];

    for (t, payload) in cases {
        assert!(
            !tx(t, one_output(), payload.clone()).is_zero_flow(),
            "O7 / FM-10: {:?} carries a value OUTPUT, so it must NOT take the \
             0-fee system lane. `is_state_only()` routed it there anyway; that is \
             the free-relay amplification that evicts legitimate 0-fee governance \
             transactions through `evict_lowest_fee`.",
            t
        );
        assert!(
            !tx(t, vec![], payload).is_zero_flow(),
            "O7 / FM-10: {:?} is classified `false` by `allows_empty_io`, so even \
             the degenerate 0-in/0-out shape must not be system-routed",
            t
        );
    }
}

/// FM-10 / constraint C1 (Must) — `Exit`, `SlashProducer` and
/// `RequestWithdrawal` are NOT zero-flow.
///
/// The second documented behavioural delta. With 0 inputs these move from
/// "admitted at fee 0, gossiped, then silently never mined" to "rejected at the
/// mempool". That is an improvement — the same silent-limbo class as INC-I-173
/// itself — but it IS user-visible.
///
/// `Exit` and `SlashProducer` are excluded by AUTHORIZATION, not by shape
/// (constraint C1): `ExitData` carries no signature and `validate_exit_data` does
/// no crypto check, and `SlashData::reporter_signature` has zero verification
/// readers anywhere in the tree. Exempting them would hand an unauthenticated
/// actor a free lane.
#[test]
fn c1_exit_slash_and_request_withdrawal_are_not_zero_flow() {
    let pk = *kp().public_key();
    let cases: Vec<(TxType, Vec<u8>)> = vec![
        (
            TxType::Exit,
            bincode::serialize(&ExitData { public_key: pk }).unwrap(),
        ),
        (TxType::SlashProducer, Vec::new()),
        (
            TxType::RequestWithdrawal,
            bincode::serialize(&WithdrawalRequestData {
                producer_pubkey: pk,
                bond_count: 1,
                destination: crypto::hash::hash(b"dest"),
            })
            .unwrap(),
        ),
    ];

    for (t, payload) in cases {
        assert!(
            !tx(t, vec![], payload).is_zero_flow(),
            "O7 / C1: {:?} must NOT be zero-flow. Exit and SlashProducer are \
             excluded by AUTHORIZATION — their apply handlers accept an actor \
             identity without verifying any signature — and RequestWithdrawal is \
             not in `allows_empty_io` at all.",
            t
        );
    }
}

/// FM-10 (Must) — `PriceAttestation` loses system routing.
///
/// The third documented delta. `allows_empty_io` is `false` for it, so under
/// shape-based routing it is fee-accounted. No live impact:
/// `oracle_activation_height = u64::MAX` on every network. Reclassifying it is
/// spec Option B and explicitly NOT in M3 scope — the existing tests at
/// `crates/core/src/transaction/tests_price_attestation.rs:424` and
/// `bins/node/tests/oracle_integration.rs:142` must be UPDATED, not silently
/// deleted.
#[test]
fn fm_10_price_attestation_loses_system_routing() {
    assert!(
        !tx(TxType::PriceAttestation, vec![], Vec::new()).is_zero_flow(),
        "O7 / FM-10: PriceAttestation is `false` in `allows_empty_io`, so it must \
         not take the 0-fee system lane. Classifying it `true` is spec Option B \
         and is out of M3 scope."
    );
}

/// F4 (Must) — CONTROL: the five exempt types ARE zero-flow at the 0-in/0-out
/// shape, and STOP being zero-flow the moment an output is attached.
///
/// Anti-vacuity for every negative above: without this, an `is_zero_flow()` that
/// always returned `false` would pass the whole file while removing the 0-fee
/// lane from governance entirely — which is INC-I-173 all over again, from the
/// other direction. The second half is constraint C2, the MINT GUARD: exemption
/// is a property of (type AND shape), never of type alone.
#[test]
fn f4_control_the_five_exempt_types_are_zero_flow_only_at_the_exempt_shape() {
    let pk = *kp().public_key();
    let maintainer_payload = MaintainerChangeData::new(
        pk,
        vec![MaintainerSignature::new(pk, crypto::Signature::default())],
    )
    .to_bytes();

    let exempt: [(TxType, Vec<u8>); 5] = [
        (TxType::Registration, Vec::new()),
        (TxType::DelegateBond, Vec::new()),
        (TxType::RevokeDelegation, Vec::new()),
        (TxType::AddMaintainer, maintainer_payload.clone()),
        (TxType::RemoveMaintainer, maintainer_payload),
    ];

    for (t, payload) in exempt {
        assert!(
            tx(t, vec![], payload.clone()).is_zero_flow(),
            "O7 / CONTROL: {:?} at 0-in/0-out MUST be zero-flow. If the exempt set \
             is empty, shape-based routing has removed the 0-fee lane from \
             governance entirely — INC-I-173 again, from the other direction.",
            t
        );
        assert!(
            !tx(t, one_output(), payload).is_zero_flow(),
            "O7 / C2 MINT GUARD: {:?} carrying a value OUTPUT must NOT be \
             zero-flow. Exemption is a property of (type AND shape); a widened \
             type list must never on its own let a transaction with an output skip \
             the balance check.",
            t
        );
    }
}

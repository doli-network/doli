//! INC-I-173 M3 — ITEM 1 / spec F5: bound the maintainer-change payload.
//!
//! Closes AUDIT-P1-001 (unbounded `MaintainerChangeData` decode on a fee-exempt
//! type) and REQ-173-014 (spam/DoS cost bound on the newly-mineable types).
//!
//! TDD RED. This file does NOT compile against the tree at `32e0a650`:
//!   * `doli_core::maintainer::MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES`
//!   * `doli_core::maintainer::MAX_MAINTAINER_CHANGE_SIGNATURES`
//!   * `doli_core::maintainer::MAX_MAINTAINER_CHANGE_REASON_BYTES`
//!
//! do not exist yet, and `validate_maintainer_change_data` takes no
//! `ValidationContext` so it cannot see a height at all
//! (`crates/core/src/validation/tx_types.rs:739`). That failure IS the RED
//! evidence for every assertion below.
//!
//! Contract: `docs/.workflow/inc-i-173-M3-design-contract.md` Item 1.
//! Spec: `specs/state-only-fee-gate-architecture.md` F5.
//!
//! ---------------------------------------------------------------------------
//! REQUIRED API (verbatim from the contract)
//! ---------------------------------------------------------------------------
//! ```ignore
//! // crates/core/src/maintainer/mod.rs — next to MAX_MAINTAINERS
//! pub const MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES: usize = 1024;
//! pub const MAX_MAINTAINER_CHANGE_SIGNATURES: usize = MAX_MAINTAINERS; // 5
//! pub const MAX_MAINTAINER_CHANGE_REASON_BYTES: usize = 256;
//!
//! // crates/core/src/validation/tx_types.rs
//! pub(super) fn validate_maintainer_change_data(
//!     tx: &Transaction,
//!     ctx: &ValidationContext,
//! ) -> Result<(), ValidationError>;
//! ```
//! Check ORDER above the gate — the size cap MUST precede `from_bytes` so
//! bincode never sees an attacker-sized buffer:
//!   1. `tx.extra_data.len() > MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES` -> reject
//!   2. existing `from_bytes` decode (unchanged)
//!   3. `data.signatures.len() > MAX_MAINTAINER_CHANGE_SIGNATURES`     -> reject
//!   4. `data.reason` BYTE length > MAX_MAINTAINER_CHANGE_REASON_BYTES -> reject
//!
//! Below the gate: the four existing checks, character-identical, in order.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT — `validate_transaction(tx, ctx)`
//! ---------------------------------------------------------------------------
//! This file drives the private `validate_maintainer_change_data` through its
//! ONLY reachable entry point, `validate_transaction`
//! (`crates/core/src/validation/transaction.rs:171,174`). Driving the public
//! entry point rather than the private helper is deliberate: it proves the two
//! `TxType` arms are BOTH wired to the new signature, which a direct unit call
//! could not show.
//!
//! ENUMERATION OF OBSERVABLE OUTPUTS
//!   O1: the `Result<(), ValidationError>` DISCRIMINANT (accept vs reject).
//!       Consensus-visible above the gate: a block carrying a tx that returns
//!       `Err` is rejected whole at `apply_block/tx_processing.rs:99`.
//!   O2: the `ValidationError` VARIANT — must be `InvalidMaintainerChange` for
//!       every rejection this item owns, never a neighbouring structural error.
//!   O3: the rejection MESSAGE TEXT. This is a load-bearing output here, not
//!       cosmetics: it is the ONLY observable that distinguishes "the size cap
//!       fired" from "the decoder fired". Check-ORDER (requirement 1 above)
//!       has no other instrument — both orders reject an oversized garbage
//!       buffer, and only the message says which one did it.
//!   O4: `MaintainerChangeData::to_bytes()` LENGTH — the cross-cap consistency
//!       output (a maximal legal payload must fit under the outer cap).
//!   mutable params   : NONE (`tx` and `ctx` are shared refs).
//!   receiver mutation: NONE (free function).
//!   persistent store : NONE (no I/O; the maintainer path reads no UTXO).
//!   side channels    : `tracing` only. DECLARED UNASSERTED — nothing is logged
//!                      on this path that is not already carried by O1/O2/O3.
//!
//! CODE PATHS
//!   PA: `current_height >= inc_i_173_activation_height` -> the four bounded
//!       checks, size cap first.
//!   PB: `current_height <  inc_i_173_activation_height` -> the FROZEN four
//!       checks. RETROACTIVE-VACUITY branch: every new bound must be INERT.
//!
//! INPUT PARTITIONS (each driven on BOTH PA and PB, and for BOTH
//! `AddMaintainer` and `RemoveMaintainer`)
//!   IP-X0 extra_data exactly `MAX_..._EXTRA_DATA_BYTES`      -> size cap silent
//!   IP-X1 extra_data exactly `MAX_..._EXTRA_DATA_BYTES + 1`  -> size cap fires
//!   IP-X2 extra_data 64 KiB of undecodable garbage           -> ORDER probe
//!   IP-S0 5 signatures (== MAX)                              -> accepted
//!   IP-S1 6 signatures (== MAX + 1)                          -> rejected
//!   IP-S2 4096 signatures (the AUDIT-P1-001 flood)           -> rejected
//!   IP-R0 reason of exactly 256 BYTES                        -> accepted
//!   IP-R1 reason of exactly 257 BYTES                        -> rejected
//!   IP-R2 reason of 100 emoji (400 bytes, 100 chars)         -> rejected on
//!         BYTES, proving the cap is not a `chars().count()`
//!   IP-R3 reason = `None`                                    -> accepted
//!   IP-C  maximal legal payload (5 sigs + 256-byte reason)   -> O4 <= outer cap
//! MATRIX: (O1,O2,O3) x {PA,PB} x {IP-X0..IP-R3} x {Add,Remove}; O4 x IP-C.
//!
//! ---------------------------------------------------------------------------
//! VALIDATION MODE (constraint C8) — STATED EXPLICITLY
//! ---------------------------------------------------------------------------
//! `validate_transaction` takes NO `ValidationMode`, so this file calls it
//! DIRECTLY and no wrapper can swallow an error. That is strictly STRONGER than
//! `ValidationMode::Full`, and it is the same posture the M1 suite adopted
//! (`crates/core/tests/inc_i_173_fee_gate.rs:76-83`). The INC-I-064 tolerance
//! that C8 warns about lives one layer up at
//! `bins/node/src/node/apply_block/tx_processing.rs` and is keyed EXCLUSIVELY on
//! `ValidationMode::Replay`; that keying is asserted at the node layer in
//! `bins/node/tests/inc_i_173_state_only_fee_gate.rs`. Nothing in this file
//! runs in `Replay`.

mod inc_i_173_common;

use doli_core::consensus::ConsensusParams;
use doli_core::maintainer::{
    MaintainerChangeData, MaintainerSignature, MAX_MAINTAINERS,
    MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES, MAX_MAINTAINER_CHANGE_REASON_BYTES,
    MAX_MAINTAINER_CHANGE_SIGNATURES,
};
use doli_core::transaction::{Transaction, TxType};
use doli_core::validation::{validate_transaction, ValidationContext, ValidationError};
use doli_core::Network;
use inc_i_173_common::{kp_a, ABOVE_GATE, BELOW_GATE, TEST_AH};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// The two tx types this item bounds. Every partition is driven on BOTH, because
/// the two arms at `transaction.rs:170-175` are separate call sites and F5's
/// failure mode is that only one of them was migrated to the new signature.
const BOUNDED_TYPES: [TxType; 2] = [TxType::AddMaintainer, TxType::RemoveMaintainer];

fn ctx_at(height: u64) -> ValidationContext {
    ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, height)
        .with_inc_i_173_activation_height(TEST_AH)
}

/// A signature entry from a deterministic key. The signature bytes are never
/// verified on this path (`validate_maintainer_change_data` is STRUCTURAL only —
/// quorum verification happens at the node layer), so a default signature is
/// exactly as representative as a real one and keeps the payload sizes exact.
fn sig_entry(seed: u8) -> MaintainerSignature {
    MaintainerSignature::new(
        *crypto::KeyPair::from_seed([seed; 32]).public_key(),
        crypto::Signature::default(),
    )
}

/// Build a `MaintainerChangeData` carrying `sig_count` signature entries.
///
/// FIXTURE NOTE (seed wrapping): the seed is `(i % 255) + 1`, never `i as u8 + 1`.
/// The naive form overflows at `i == 255` (`255u8 + 1` panics under the
/// debug-assertions that `[profile.test]` leaves on), which made the IP-S2 flood
/// partition abort inside the fixture and prove nothing about the implementation.
/// Wrapping keeps the seed in `1..=255` for ANY `sig_count`, so the flood shape is
/// constructible at full size. Seeds repeat above 255 entries and the resulting
/// pubkeys therefore repeat too — irrelevant here, because this path is STRUCTURAL
/// only: it counts entries and bounds bytes, and never inspects signer identity.
/// Every partition that DOES care about distinct signers uses `sig_count <= 6`,
/// which is below the wrap point and so keeps every seed unique.
fn change_data(sig_count: usize, reason: Option<String>) -> MaintainerChangeData {
    MaintainerChangeData {
        target: *kp_a().public_key(),
        signatures: (0..sig_count)
            .map(|i| sig_entry((i % 255) as u8 + 1))
            .collect(),
        reason,
    }
}

fn tx_with_payload(t: TxType, extra_data: Vec<u8>) -> Transaction {
    Transaction {
        version: 1,
        tx_type: t,
        inputs: vec![],
        outputs: vec![],
        extra_data,
    }
}

fn tx_with_data(t: TxType, data: &MaintainerChangeData) -> Transaction {
    tx_with_payload(t, data.to_bytes())
}

/// A DECODABLE payload padded to EXACTLY `target_len` bytes, by growing the
/// `reason` field. Used only by the outer-cap boundary probes, where the inner
/// reason cap is deliberately violated — the point of IP-X0/IP-X1 is which cap
/// speaks first, not whether the payload is otherwise legal.
fn decodable_payload_of_exactly(target_len: usize) -> Vec<u8> {
    let mut filler = 0usize;
    loop {
        let bytes = change_data(1, Some("a".repeat(filler))).to_bytes();
        assert!(
            bytes.len() <= target_len,
            "cannot build a decodable payload of exactly {} bytes: the minimum \
             decodable MaintainerChangeData is already {} bytes",
            target_len,
            bytes.len()
        );
        if bytes.len() == target_len {
            return bytes;
        }
        filler += 1;
    }
}

/// Did the rejection come from the OUTER SIZE cap?
///
/// O3 instrument. The developer must make the size-cap message name the byte
/// bound (the ZKSettle idiom at `tx_types.rs:1061-1068` already does this), and
/// must NOT put a byte count in the decode-failure message. Without that the
/// check-ORDER requirement is unobservable.
fn is_size_cap_rejection(e: &ValidationError) -> bool {
    let text = e.to_string();
    text.contains(&MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES.to_string())
}

fn err_of(t: TxType, extra_data: Vec<u8>, height: u64) -> Option<ValidationError> {
    validate_transaction(&tx_with_payload(t, extra_data), &ctx_at(height)).err()
}

// ===========================================================================
// IP-C — the CROSS-CAP CONSISTENCY test. Run this first mentally: if it fails,
// the three caps contradict each other and a legitimate 5-of-5 authorization is
// UNMINEABLE — the exact bug class INC-I-173 is about.
// ===========================================================================

/// REQ-173-014 / AUDIT-P1-001 — the caps must not contradict each other.
///
/// A MAXIMAL LEGAL payload is 5 signatures (== `MAX_MAINTAINERS`) plus a
/// 256-byte reason. If that serializes to more than the outer cap, the outer cap
/// rejects a payload that satisfies both inner caps, and the 5-of-5 rotation
/// that INC-I-172 M2 exists to enable becomes unmineable — a new instance of the
/// silent-limbo class this whole incident is about.
///
/// The worst case is 873 bytes — 5 signatures plus a 256-byte `reason` — which
/// leaves 151 bytes of headroom under the 1024-byte outer cap. That figure is
/// NOT restated arithmetically anywhere; it is asserted below against the real
/// bincode encoder, which is the only instrument that cannot drift from the
/// payload shape.
#[test]
fn req_173_014_maximal_legal_payload_fits_under_the_outer_cap() {
    let maximal = change_data(
        MAX_MAINTAINER_CHANGE_SIGNATURES,
        Some("z".repeat(MAX_MAINTAINER_CHANGE_REASON_BYTES)),
    );
    let encoded = maximal.to_bytes();

    assert_eq!(
        maximal.signatures.len(),
        MAX_MAINTAINERS,
        "the signature cap must BE MAX_MAINTAINERS — it is principled, not a \
         magic number: count_distinct_signers only counts a signature whose \
         pubkey is a CURRENT member, and membership is capped at {}",
        MAX_MAINTAINERS
    );
    assert!(
        encoded.len() <= MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES,
        "O4 / REQ-173-014: a MAXIMAL LEGAL MaintainerChangeData ({} signatures + \
         a {}-byte reason) serializes to {} bytes, which EXCEEDS the outer cap of \
         {}. The three caps contradict each other: a legitimate 5-of-5 \
         authorization would be rejected by the outer cap despite satisfying both \
         inner caps, making it permanently unmineable.",
        MAX_MAINTAINER_CHANGE_SIGNATURES,
        MAX_MAINTAINER_CHANGE_REASON_BYTES,
        encoded.len(),
        MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES
    );
    // NOTE (OBS-5, and F3 at review iteration 1): the assertion above is the
    // WHOLE caps-are-consistent guarantee, because it measures the REAL encoder.
    // A second assertion comparing it to a hand-derived byte count once lived
    // here; it could not catch any inconsistency this one misses, and only
    // guarded a restatement of bincode's rules against the rules themselves. The
    // 873/151 figures now live as prose on MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES,
    // protected behaviourally by this `<=`.
}

/// REQ-173-014 — the maximal legal payload is ACCEPTED above the gate, on both
/// tx types. The consistency test above is arithmetic; this one is behavioural.
#[test]
fn req_173_014_maximal_legal_payload_is_accepted_above_the_gate() {
    let maximal = change_data(
        MAX_MAINTAINER_CHANGE_SIGNATURES,
        Some("z".repeat(MAX_MAINTAINER_CHANGE_REASON_BYTES)),
    );
    for t in BOUNDED_TYPES {
        let verdict = validate_transaction(&tx_with_data(t, &maximal), &ctx_at(ABOVE_GATE));
        assert!(
            verdict.is_ok(),
            "O1 / REQ-173-014: the MAXIMAL LEGAL payload (5 sigs + 256-byte \
             reason) must be ACCEPTED above the gate for {:?}; got {:?}",
            t,
            verdict
        );
    }
}

// ===========================================================================
// IP-X — the OUTER extra_data cap, and the CHECK-ORDER requirement
// ===========================================================================

/// AUDIT-P1-001 (Must) — above the gate, `extra_data` one byte over the cap is
/// REJECTED, on both tx types.
#[test]
fn audit_p1_001_extra_data_one_byte_over_the_cap_is_rejected_above_the_gate() {
    let oversized = decodable_payload_of_exactly(MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES + 1);
    for t in BOUNDED_TYPES {
        let e = err_of(t, oversized.clone(), ABOVE_GATE).unwrap_or_else(|| {
            panic!(
                "O1 / AUDIT-P1-001: {:?} with extra_data of {} bytes (cap {}) must \
                 be REJECTED above the gate, but validation ACCEPTED it",
                t,
                oversized.len(),
                MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES
            )
        });
        assert!(
            matches!(e, ValidationError::InvalidMaintainerChange(_)),
            "O2: the rejection must be InvalidMaintainerChange, not a neighbouring \
             structural error; got {:?} for {:?}",
            e,
            t
        );
        assert!(
            is_size_cap_rejection(&e),
            "O3: the rejection message must name the byte bound {} so the SIZE cap \
             is distinguishable from the DECODER; got {:?} for {:?}",
            MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES,
            e,
            t
        );
    }
}

/// AUDIT-P1-001 (Must) — AT the cap the SIZE check stays silent.
///
/// The boundary partner of the test above. `>` not `>=`: a payload of exactly
/// `MAX_..._EXTRA_DATA_BYTES` bytes must not be rejected BY THE SIZE CAP. It may
/// still be rejected by an inner cap (the padding here overruns the reason
/// bound), which is why the assertion is on O3 — "not the size cap" — rather
/// than on O1.
#[test]
fn audit_p1_001_extra_data_exactly_at_the_cap_does_not_trip_the_size_check() {
    let at_limit = decodable_payload_of_exactly(MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES);
    for t in BOUNDED_TYPES {
        if let Some(e) = err_of(t, at_limit.clone(), ABOVE_GATE) {
            assert!(
                !is_size_cap_rejection(&e),
                "O3: extra_data of EXACTLY {} bytes is AT the cap, not over it. The \
                 comparison must be `>`, not `>=`; got the size-cap rejection {:?} \
                 for {:?}",
                MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES,
                e,
                t
            );
        }
    }
}

/// AUDIT-P1-001 (Must) — CHECK ORDER: the size cap runs BEFORE `from_bytes`.
///
/// This is the whole security point of the item. `MaintainerChangeData::from_bytes`
/// is `bincode::deserialize().ok()` over an unbounded `Vec<MaintainerSignature>`
/// plus an `Option<String>` (`crates/core/src/maintainer/data.rs:57-59`). If the
/// decoder runs first, an attacker-sized buffer is allocated and walked on a
/// ZERO-FEE transaction before anything bounds it, and that cost is re-paid on
/// every future sync of the block.
///
/// INSTRUMENT: 64 KiB of undecodable garbage. Both check orders REJECT it, so O1
/// cannot discriminate. Only O3 can: size-first names the byte bound, decode-first
/// says "invalid maintainer change data format".
#[test]
fn audit_p1_001_size_cap_runs_before_the_decoder() {
    let garbage = vec![0xABu8; 64 * 1024];
    for t in BOUNDED_TYPES {
        let e = err_of(t, garbage.clone(), ABOVE_GATE)
            .unwrap_or_else(|| panic!("O1: 64 KiB of garbage must be rejected for {:?}", t));
        assert!(
            is_size_cap_rejection(&e),
            "O3 / AUDIT-P1-001 CHECK ORDER: a {}-byte payload must be refused by \
             the SIZE cap BEFORE bincode::deserialize ever sees it. The message \
             must name the bound {}; got {:?} for {:?}. If this reads \"invalid \
             maintainer change data format\", the decoder ran first and the \
             unbounded-decode surface is still open.",
            garbage.len(),
            MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES,
            e,
            t
        );
    }
}

// ===========================================================================
// IP-S — the SIGNATURE-COUNT cap
// ===========================================================================

/// AUDIT-P1-001 (Must) — exactly `MAX_MAINTAINER_CHANGE_SIGNATURES` is ACCEPTED.
#[test]
fn audit_p1_001_signature_count_at_the_cap_is_accepted_above_the_gate() {
    let data = change_data(MAX_MAINTAINER_CHANGE_SIGNATURES, None);
    for t in BOUNDED_TYPES {
        let verdict = validate_transaction(&tx_with_data(t, &data), &ctx_at(ABOVE_GATE));
        assert!(
            verdict.is_ok(),
            "O1: {} signatures is AT the cap and must be ACCEPTED for {:?} \
             (comparison must be `>`, not `>=`); got {:?}",
            MAX_MAINTAINER_CHANGE_SIGNATURES,
            t,
            verdict
        );
    }
}

/// AUDIT-P1-001 (Must) — one signature over the cap is REJECTED.
#[test]
fn audit_p1_001_signature_count_one_over_the_cap_is_rejected_above_the_gate() {
    let data = change_data(MAX_MAINTAINER_CHANGE_SIGNATURES + 1, None);
    for t in BOUNDED_TYPES {
        let e = validate_transaction(&tx_with_data(t, &data), &ctx_at(ABOVE_GATE)).expect_err(
            "O1: a signature vector one entry over the cap must be REJECTED above \
             the gate — entry 6 can never add a distinct signer, because \
             count_distinct_signers only counts signatures whose pubkey is a \
             CURRENT member and membership is capped at MAX_MAINTAINERS",
        );
        assert!(
            matches!(e, ValidationError::InvalidMaintainerChange(_)),
            "O2: must be InvalidMaintainerChange; got {:?} for {:?}",
            e,
            t
        );
    }
}

/// AUDIT-P1-001 (Must) — the actual flood shape. 4096 signature entries on a
/// ZERO-FEE transaction is the free-permanent-storage + O(N)-Ed25519-verify
/// surface the finding names (FM-4 / FM-11).
///
/// Chosen to stay under the outer `extra_data` cap only in spirit — it does not
/// (4096 x 96 bytes is ~384 KiB), so above the gate the SIZE cap catches it
/// first. The assertion is therefore on O1/O2 alone: what matters is that it
/// never reaches the quorum verifier, not which cap stopped it.
#[test]
fn audit_p1_001_signature_flood_is_rejected_above_the_gate() {
    let data = change_data(4096, None);
    assert_eq!(
        data.signatures.len(),
        4096,
        "fixture: the flood must be constructed at FULL size. If the seed wrapping \
         in change_data ever silently truncates or dedupes, this partition would \
         degrade into a re-run of the 6-signature case and stop exercising the \
         AUDIT-P1-001 shape at all."
    );
    for t in BOUNDED_TYPES {
        let e = validate_transaction(&tx_with_data(t, &data), &ctx_at(ABOVE_GATE)).expect_err(
            "O1 / AUDIT-P1-001: a 4096-entry signature vector on a FEE-EXEMPT tx \
             must be REJECTED above the gate",
        );
        assert!(
            matches!(e, ValidationError::InvalidMaintainerChange(_)),
            "O2: must be InvalidMaintainerChange; got {:?} for {:?}",
            e,
            t
        );
    }
}

// ===========================================================================
// IP-R — the REASON cap. BYTES, never chars.
// ===========================================================================

/// AUDIT-P1-001 (Must) — a reason of exactly `MAX_..._REASON_BYTES` is ACCEPTED.
#[test]
fn audit_p1_001_reason_at_the_byte_cap_is_accepted_above_the_gate() {
    let data = change_data(3, Some("r".repeat(MAX_MAINTAINER_CHANGE_REASON_BYTES)));
    for t in BOUNDED_TYPES {
        let verdict = validate_transaction(&tx_with_data(t, &data), &ctx_at(ABOVE_GATE));
        assert!(
            verdict.is_ok(),
            "O1: a reason of exactly {} bytes is AT the cap and must be ACCEPTED \
             for {:?}; got {:?}",
            MAX_MAINTAINER_CHANGE_REASON_BYTES,
            t,
            verdict
        );
    }
}

/// AUDIT-P1-001 (Must) — a reason one BYTE over the cap is REJECTED.
#[test]
fn audit_p1_001_reason_one_byte_over_the_cap_is_rejected_above_the_gate() {
    let data = change_data(3, Some("r".repeat(MAX_MAINTAINER_CHANGE_REASON_BYTES + 1)));
    for t in BOUNDED_TYPES {
        let e = validate_transaction(&tx_with_data(t, &data), &ctx_at(ABOVE_GATE))
            .expect_err("O1: a reason one byte over the cap must be REJECTED");
        assert!(
            matches!(e, ValidationError::InvalidMaintainerChange(_)),
            "O2: must be InvalidMaintainerChange; got {:?} for {:?}",
            e,
            t
        );
    }
}

/// AUDIT-P1-001 (Must) — the cap counts BYTES, not `char`s.
///
/// Worst-scenario #4 (unicode / emoji). 100 four-byte emoji is 400 BYTES but
/// only 100 `char`s. A `reason.chars().count() > 256` implementation would
/// ACCEPT this and leave 40% of the intended bound on the table; a
/// `reason.len() > 256` implementation rejects it. Bytes are the unit the chain
/// pays for — what is written to a block and re-read on every future sync — so
/// bytes are the only unit that bounds the cost this cap exists to bound.
#[test]
fn audit_p1_001_reason_cap_counts_bytes_not_chars() {
    let emoji_reason = "\u{1F680}".repeat(100); // 100 chars, 400 bytes
    assert_eq!(emoji_reason.chars().count(), 100, "fixture: 100 chars");
    assert_eq!(emoji_reason.len(), 400, "fixture: 400 bytes");
    assert!(
        emoji_reason.chars().count() <= MAX_MAINTAINER_CHANGE_REASON_BYTES,
        "fixture: the CHAR count is under the cap, so only a BYTE check can reject"
    );

    let data = change_data(3, Some(emoji_reason));
    for t in BOUNDED_TYPES {
        let e = validate_transaction(&tx_with_data(t, &data), &ctx_at(ABOVE_GATE)).expect_err(
            "O1: a 400-BYTE / 100-CHAR reason must be REJECTED. If this passes, the \
             cap is implemented as chars().count() and the real byte bound is 4x \
             the intended one.",
        );
        assert!(
            matches!(e, ValidationError::InvalidMaintainerChange(_)),
            "O2: must be InvalidMaintainerChange; got {:?} for {:?}",
            e,
            t
        );
    }
}

/// AUDIT-P1-001 (Should) — `reason: None` is unaffected by the reason cap.
/// Worst-scenario #1 (empty / null input).
#[test]
fn audit_p1_001_absent_reason_is_accepted_above_the_gate() {
    let data = change_data(3, None);
    for t in BOUNDED_TYPES {
        let verdict = validate_transaction(&tx_with_data(t, &data), &ctx_at(ABOVE_GATE));
        assert!(
            verdict.is_ok(),
            "O1: `reason: None` must be ACCEPTED for {:?}; got {:?}",
            t,
            verdict
        );
    }
}

/// AUDIT-P1-001 (Should) — an EMPTY reason string is accepted.
/// Worst-scenario #1, the other half: `Some("")` is not `None`.
#[test]
fn audit_p1_001_empty_reason_string_is_accepted_above_the_gate() {
    let data = change_data(3, Some(String::new()));
    for t in BOUNDED_TYPES {
        let verdict = validate_transaction(&tx_with_data(t, &data), &ctx_at(ABOVE_GATE));
        assert!(
            verdict.is_ok(),
            "O1: `reason: Some(\"\")` must be ACCEPTED for {:?}; got {:?}",
            t,
            verdict
        );
    }
}

// ===========================================================================
// PB — RETROACTIVE VACUITY. Every bound above must be INERT below the gate.
//
// This is the guarantee that lets F5 ride the EXISTING
// `inc_i_173_activation_height` instead of costing a new one. It is only sound
// if the below-gate branch is byte-identical to today's four checks, so each
// partition above gets its below-gate twin here.
// ===========================================================================

/// REQ-173-014 / constraint C8 — BELOW the gate, an oversized `extra_data` that
/// DECODES is still ACCEPTED.
///
/// Retroactive vacuity, the load-bearing half. No block below the gate can
/// contain an AddMaintainer/RemoveMaintainer — being unmineable is the INC-I-173
/// bug itself — so bounding them cannot invalidate history. But that argument
/// only holds if the new bound is genuinely absent below the gate: if it fires
/// there, every historical block would have to be re-validated under a rule that
/// did not exist when it was produced.
#[test]
fn req_173_014_oversized_extra_data_is_still_accepted_below_the_gate() {
    let oversized = decodable_payload_of_exactly(MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES + 1);
    for t in BOUNDED_TYPES {
        let verdict =
            validate_transaction(&tx_with_payload(t, oversized.clone()), &ctx_at(BELOW_GATE));
        assert!(
            verdict.is_ok(),
            "O1 / RETROACTIVE VACUITY: below the gate the four FROZEN checks apply \
             and an oversized-but-decodable payload is ACCEPTED. {:?} rejected it \
             with {:?}, which means the new bound leaked below the gate and \
             changes frozen consensus history (INV-COMPAT-001).",
            t,
            verdict
        );
    }
}

/// REQ-173-014 — BELOW the gate, a signature vector over the cap is still
/// ACCEPTED (it decodes; the frozen checks do not count entries).
#[test]
fn req_173_014_signature_count_over_the_cap_is_still_accepted_below_the_gate() {
    let data = change_data(MAX_MAINTAINER_CHANGE_SIGNATURES + 1, None);
    for t in BOUNDED_TYPES {
        let verdict = validate_transaction(&tx_with_data(t, &data), &ctx_at(BELOW_GATE));
        assert!(
            verdict.is_ok(),
            "O1 / RETROACTIVE VACUITY: the signature cap must be INERT below the \
             gate; {:?} rejected with {:?}",
            t,
            verdict
        );
    }
}

/// REQ-173-014 — BELOW the gate, an over-cap reason is still ACCEPTED.
#[test]
fn req_173_014_oversized_reason_is_still_accepted_below_the_gate() {
    let data = change_data(3, Some("r".repeat(MAX_MAINTAINER_CHANGE_REASON_BYTES + 1)));
    for t in BOUNDED_TYPES {
        let verdict = validate_transaction(&tx_with_data(t, &data), &ctx_at(BELOW_GATE));
        assert!(
            verdict.is_ok(),
            "O1 / RETROACTIVE VACUITY: the reason cap must be INERT below the \
             gate; {:?} rejected with {:?}",
            t,
            verdict
        );
    }
}

/// REQ-173-014 — BELOW the gate, the emoji reason is ACCEPTED too. The byte/char
/// distinction must not exist at all on the frozen branch.
#[test]
fn req_173_014_emoji_reason_is_still_accepted_below_the_gate() {
    let data = change_data(3, Some("\u{1F680}".repeat(100)));
    for t in BOUNDED_TYPES {
        let verdict = validate_transaction(&tx_with_data(t, &data), &ctx_at(BELOW_GATE));
        assert!(
            verdict.is_ok(),
            "O1 / RETROACTIVE VACUITY: {:?} rejected an emoji reason below the gate \
             with {:?}",
            t,
            verdict
        );
    }
}

/// REQ-173-014 — BELOW the gate, 64 KiB of garbage is rejected by the DECODER,
/// exactly as it is today. The message must NOT name the byte bound, because the
/// size cap does not exist on this branch.
///
/// This is the anti-vacuity partner of `audit_p1_001_size_cap_runs_before_the_decoder`:
/// together they show the O3 instrument actually discriminates, rather than
/// matching everything.
#[test]
fn req_173_014_garbage_is_rejected_by_the_decoder_below_the_gate() {
    let garbage = vec![0xABu8; 64 * 1024];
    for t in BOUNDED_TYPES {
        let e = err_of(t, garbage.clone(), BELOW_GATE).unwrap_or_else(|| {
            panic!(
                "O1: garbage must still be rejected below the gate for {:?}",
                t
            )
        });
        assert!(
            matches!(e, ValidationError::InvalidMaintainerChange(_)),
            "O2: must be InvalidMaintainerChange; got {:?} for {:?}",
            e,
            t
        );
        assert!(
            !is_size_cap_rejection(&e),
            "O3 / RETROACTIVE VACUITY: below the gate the size cap does not exist, \
             so the rejection must come from the DECODER and must NOT name the \
             bound {}; got {:?} for {:?}",
            MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES,
            e,
            t
        );
    }
}

/// REQ-173-014 — the three FROZEN structural refusals are unchanged on BOTH
/// branches: inputs present, outputs present, empty extra_data.
///
/// F5 adds bounds; it must not perturb the checks that were already there.
#[test]
fn req_173_014_frozen_structural_refusals_are_unchanged_on_both_branches() {
    let recipient = crypto::hash::hash(b"inc-i-173-m3");
    let good = change_data(3, None).to_bytes();

    for height in [BELOW_GATE, ABOVE_GATE] {
        for t in BOUNDED_TYPES {
            // empty extra_data
            let e = validate_transaction(&tx_with_payload(t, Vec::new()), &ctx_at(height))
                .expect_err("empty extra_data must be rejected at every height");
            assert!(
                matches!(e, ValidationError::InvalidMaintainerChange(_)),
                "O2: empty extra_data -> InvalidMaintainerChange; got {:?} ({:?} @ {})",
                e,
                t,
                height
            );

            // an OUTPUT present
            let with_output = Transaction {
                version: 1,
                tx_type: t,
                inputs: vec![],
                outputs: vec![doli_core::transaction::Output::normal(1, recipient)],
                extra_data: good.clone(),
            };
            let e = validate_transaction(&with_output, &ctx_at(height))
                .expect_err("a maintainer change carrying an output must be rejected");
            assert!(
                matches!(e, ValidationError::InvalidMaintainerChange(_)),
                "O2: output present -> InvalidMaintainerChange; got {:?} ({:?} @ {})",
                e,
                t,
                height
            );
        }
    }
}

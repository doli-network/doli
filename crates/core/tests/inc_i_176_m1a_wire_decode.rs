//! INC-I-176 **M1a** — what `MaintainerChangeData::from_bytes` does with HOSTILE
//! and FOREIGN input.
//!
//! Companion to `inc_i_176_m1a_wire_freeze.rs`, split from it for the 800-line
//! test-file budget (CLAUDE.md rule 19). The freeze file pins the ENCODING and
//! the real on-chain regression; this one pins the DECODER's behaviour on input
//! it was never given.
//!
//! Two properties, and the second is the reason M1a exists:
//!
//! 1. **Malformed input yields `None`, never a value and never a panic.**
//!    `from_bytes` runs on attacker-chosen bytes carried by a FEE-EXEMPT
//!    transaction, so a panic here is a remote node kill.
//! 2. **The attempt-1 `u64`-tail shape and the HEAD `Option<String>` shape are
//!    MUTUALLY AMBIGUOUS.** This is the measured evidence that the payload swap
//!    must move to M2.5 behind an activation height AND an explicit format
//!    discriminator — a try-new-then-fall-back-to-legacy decoder is unsound.
//!
//! TDD RED against the working tree as handed over (M1 attempt 1, uncommitted):
//! `MaintainerChangeData` has no `reason` field there, so this file does not
//! compile. That is the red evidence; revert the source, do not relax the file.
//!
//! Contract: `docs/.workflow/inc-i-176-M1a-output-contract.md`.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! X `MaintainerChangeData::from_bytes(&[u8]) -> Option<Self>`
//!   X-O3  return DISCRIMINANT — `Some` for a well-formed HEAD-shaped payload,
//!         `None` for every malformed or foreign-shaped input.
//!   X-O3c TERMINATION — the call returns at all: no panic, no unbounded
//!         allocation on an attacker-declared length.
//! V `bincode::deserialize::<U64TailShape>` (a LOCAL negative control, not
//!   production code)
//!   V-O3  return — used only to show that HEAD bytes are MISREAD rather than
//!         rejected, i.e. that the confusion is silent.
//!   mutable params / receiver / persistent store / side channels: NONE.
//!
//! CODE PATHS
//!   P-DEC+ `from_bytes` returns `Some`.
//!   P-DEC- `from_bytes` returns `None` (`.ok()` swallows the bincode error).
//!
//! INPUT PARTITIONS
//!   IP-U64TAIL-F  attempt-1 shape -> HEAD decoder. Expect `None` (fleet split, forward leg).
//!   IP-U64TAIL-B  HEAD `Some("")` -> u64-tail decoder. Expect SILENT misread as
//!                 `valid_before = 1` (backward leg). 57 B vs 56 B, so length
//!                 sniffing cannot separate them.
//!   IP-EMPTY      zero bytes                       -> worst scenario #1
//!   IP-TRUNC      truncated at 1 / 8 / 30 / 120 / len-1 bytes -> worst scenario #10
//!   IP-GARBAGE    64 bytes of `0xff`               -> invalid tags everywhere
//!   IP-OVERSIZE   declared signature count 1_000_000; declared reason length
//!                 `u64::MAX`                       -> worst scenario #8
//!   IP-REALKEY    every partition uses ON-CURVE keys. MEASURED: `PublicKey`
//!                 deserialization validates the point, so a raw byte-pattern key
//!                 would make EVERY decode return `None` and every assertion here
//!                 would pass for the wrong reason. Each test carries a positive
//!                 control that decodes.
//!   MATRIX: X-O3 × {IP-EMPTY, IP-TRUNC×5, IP-GARBAGE, IP-OVERSIZE×2, IP-U64TAIL-F};
//!           V-O3 × IP-U64TAIL-B; X-O3c × every partition (a panic fails the test).

use crypto::{KeyPair, PublicKey, Signature};
use doli_core::maintainer::{MaintainerChangeData, MaintainerSignature};
use serde::{Deserialize, Serialize};

/// A REAL, on-curve public key. Deterministic seed so payloads stay byte-stable.
fn real_pk(seed: u8) -> PublicKey {
    *KeyPair::from_seed([seed; 32]).public_key()
}

/// A signature entry whose PUBKEY is on-curve. The signature bytes are never
/// verified here — the decoder only frames them.
fn real_entry(seed: u8) -> MaintainerSignature {
    MaintainerSignature::new(real_pk(seed), Signature::default())
}

fn real_five() -> Vec<MaintainerSignature> {
    vec![
        real_entry(0x41),
        real_entry(0x52),
        real_entry(0x63),
        real_entry(0x74),
        real_entry(0x85),
    ]
}

fn real_payload(
    signatures: Vec<MaintainerSignature>,
    reason: Option<&str>,
) -> MaintainerChangeData {
    MaintainerChangeData {
        target: real_pk(0x21),
        signatures,
        reason: reason.map(str::to_string),
    }
}

/// Attempt 1's shape, kept ONLY as a negative control.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct U64TailShape {
    target: PublicKey,
    signatures: Vec<MaintainerSignature>,
    valid_before: u64,
}

/// X-O3 — NEGATIVE CONTROL. Attempt 1's `u64`-tail shape is NOT decodable by
/// HEAD's decoder, and HEAD's `Some("")` shape IS silently misread by a
/// `u64`-tail decoder.
///
/// This is the evidence that the deferral to M2.5 is necessary rather than
/// cautious, and it is why any future payload change needs an EXPLICIT
/// discriminator rather than a try-new-then-fall-back-to-legacy decoder:
/// * forward  — a new-shape payload EOFs on an old binary → old nodes reject a
///   block new nodes accept: an unauthenticated fleet split;
/// * backward — an old `Some("")` payload DECODES on a new binary, as
///   `valid_before = 1`, i.e. permanently expired. Silent, not loud.
///
/// The shapes are not even separable by length (old `Some("")` is 57 B, the new
/// shape 56 B), so length sniffing cannot rescue a naive fallback.
#[test]
fn req_176_m1a_wire_the_u64_tail_shape_is_ambiguous_with_the_head_shape() {
    // REAL keys on BOTH sides. With a raw fixture every decode would fail on the
    // curve check and the test would "pass" while proving nothing about the tail.
    let target = real_pk(0x21);

    // FORWARD: attempt 1's encoding, fed to HEAD's decoder.
    let attempt1 = U64TailShape {
        target,
        signatures: vec![],
        valid_before: u64::MAX,
    };
    let attempt1_bytes = bincode::serialize(&attempt1).expect("encodes");
    assert_eq!(
        attempt1_bytes.len(),
        56,
        "fixture: the u64-tail shape is 56 bytes where the HEAD `None` shape is 49"
    );
    assert!(
        MaintainerChangeData::from_bytes(&attempt1_bytes).is_none(),
        "X-O3 / IP-U64TAIL: a `u64`-tail payload must NOT decode as the HEAD shape. This is the \
         forward leg of the mixed-fleet split: an old binary REJECTS a block a new binary \
         accepts, and `validation/tx_types.rs:809` makes that rejection fatal."
    );

    // POSITIVE CONTROL: the same key material in the HEAD shape DOES decode, so
    // the `None` above is attributable to the TAIL and not to the key.
    let head_none = MaintainerChangeData {
        target,
        signatures: vec![],
        reason: None,
    }
    .to_bytes();
    assert_eq!(
        head_none.len(),
        49,
        "fixture: the HEAD `None` shape is 49 bytes"
    );
    assert!(
        MaintainerChangeData::from_bytes(&head_none).is_some(),
        "POSITIVE CONTROL: the identical key material in the HEAD shape must decode. Without \
         this, the assertion above could pass because the KEY was rejected."
    );

    // BACKWARD: HEAD's `Some("")` encoding, fed to a u64-tail decoder. 57 bytes
    // vs the new shape's 56 — indistinguishable by any length heuristic.
    let head_empty = MaintainerChangeData {
        target,
        signatures: vec![],
        reason: Some(String::new()),
    }
    .to_bytes();
    assert_eq!(
        head_empty.len(),
        57,
        "fixture: HEAD `Some(\"\")` is 57 bytes"
    );
    let misread: U64TailShape = bincode::deserialize(&head_empty)
        .expect("the ambiguity is that this SUCCEEDS — that is the whole finding");
    assert_eq!(
        misread.valid_before, 1,
        "X-O3 / IP-U64TAIL: HEAD's `Some(\"\")` tail `01 00 00 00 00 00 00 00` is read as \
         `valid_before = 1` by a u64-tail decoder — a silent, not loud, misread. Any future \
         payload change MUST carry an explicit version discriminator; a \
         try-new-then-fall-back-to-legacy decoder is unsound, and length sniffing cannot \
         separate 57 from 56 in general."
    );
}

/// X-O3 — malformed input returns `None` and never panics. IP-MALFORMED.
///
/// Worst scenarios #1 (empty), #8 (extremely large declared size) and #10
/// (truncation mid-structure). `from_bytes` is reachable from consensus with
/// attacker-chosen bytes on a FEE-EXEMPT transaction, so a panic here is a remote
/// node kill.
#[test]
fn req_176_m1a_wire_malformed_payloads_return_none_without_panicking() {
    // REAL keys: a malformed payload must be refused because it is MALFORMED, not
    // because `PublicKey` failed its curve check.
    let good = real_payload(real_five(), Some("rotation")).to_bytes();
    assert!(
        MaintainerChangeData::from_bytes(&good).is_some(),
        "POSITIVE CONTROL: the well-formed base payload must decode, or every `None` below is \
         attributable to the fixture rather than to the malformation"
    );

    let mut oversized_vec_len = good.clone();
    // Overwrite the signature-count prefix (immediately after the 8-byte length
    // prefix and the 32 target bytes) with a claim of 1_000_000 entries.
    oversized_vec_len[40..48].copy_from_slice(&1_000_000u64.to_le_bytes());

    let mut oversized_reason_len = good.clone();
    let tail = oversized_reason_len.len() - 8;
    oversized_reason_len[tail..].copy_from_slice(&u64::MAX.to_le_bytes());

    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", vec![]),
        ("one byte", vec![0x00]),
        ("target prefix only", good[..8].to_vec()),
        ("truncated mid-target", good[..30].to_vec()),
        ("truncated mid-signature", good[..120].to_vec()),
        (
            "truncated before the reason tag",
            good[..good.len() - 1].to_vec(),
        ),
        ("all 0xff", vec![0xffu8; 64]),
        ("oversized signature count", oversized_vec_len),
        ("oversized reason length", oversized_reason_len),
    ];

    for (name, bytes) in cases {
        assert!(
            MaintainerChangeData::from_bytes(&bytes).is_none(),
            "X-O3 / IP-MALFORMED ({name}): a malformed payload must decode to `None`, never to a \
             value and never by panicking. This decoder runs on attacker-chosen bytes carried by \
             a fee-exempt transaction."
        );
    }
}

//! INC-I-217 M4 — crate-internal DST separation for the rotation PoP.
//!
//! covers: REQ-ROT-SEC-002
//!
//! OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//!   The three DST constants are compile-time byte strings. There is no
//!   function under test: O1 return / O2 params / O3 receiver / O4 store /
//!   O5 statics / O6 channels are all N/A. The asserted output is the
//!   pairwise-distinctness relation over the three values.
//!   PATHS: none — declarative constants.
//!
//! INPUT PARTITIONS:
//!   I1 (ROTATE_POP_DST, POP_DST)         — the pair a lifted registration PoP uses
//!   I2 (ROTATE_POP_DST, ATTESTATION_DST) — the pair a lifted attestation half uses
//!   I3 (POP_DST, ATTESTATION_DST)        — the pre-existing separation, non-vacuity
//!
//! FALSIFIER: the planted violation is a DST derived from another by
//! concatenation or truncation; the prefix assertions below reject it, so the
//! test is not the constant declaration restated.

use crate::bls::{ATTESTATION_DST, POP_DST};
use crate::bls_rotation::ROTATE_POP_DST;

fn is_prefix(a: &[u8], b: &[u8]) -> bool {
    a.len() <= b.len() && &b[..a.len()] == a
}

// REQ-ROT-SEC-002 — Decision: whether a signature made in one BLS domain can be lifted into another, which is the C1 replay the whole rotation design rests on.
#[test]
fn req_rot_sec_002_the_three_bls_domains_are_pairwise_distinct() {
    assert_ne!(ROTATE_POP_DST, POP_DST);
    assert_ne!(ROTATE_POP_DST, ATTESTATION_DST);
    assert_ne!(POP_DST, ATTESTATION_DST);

    for dst in [ROTATE_POP_DST, POP_DST, ATTESTATION_DST] {
        assert!(
            !dst.is_empty(),
            "an empty DST is no domain separation at all"
        );
    }
}

// REQ-ROT-SEC-002 — Decision: whether a new DST was built by extending an existing one, which reads as distinct but collides under a truncating reader.
#[test]
fn req_rot_sec_002_no_bls_domain_is_a_prefix_of_another() {
    let all: [(&str, &[u8]); 3] = [
        ("ROTATE_POP_DST", ROTATE_POP_DST),
        ("POP_DST", POP_DST),
        ("ATTESTATION_DST", ATTESTATION_DST),
    ];

    for (name_a, a) in all {
        for (name_b, b) in all {
            if name_a == name_b {
                continue;
            }
            assert!(
                !is_prefix(a, b),
                "{name_a} is a prefix of {name_b}: the domains are not separated"
            );
        }
    }

    // Non-vacuity: the predicate does detect a prefix relation.
    assert!(is_prefix(b"DOLI", b"DOLI-ROTATE-POP-V1"));
}

// REQ-ROT-SEC-002 — Decision: whether the rotation DST still carries the version suffix a future V2 field-list change must bump.
#[test]
fn req_rot_sec_002_the_rotation_dst_is_the_pinned_byte_string() {
    assert_eq!(ROTATE_POP_DST, b"DOLI-ROTATE-POP-V1");
}

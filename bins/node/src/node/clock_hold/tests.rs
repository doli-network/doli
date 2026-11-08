use super::not_yet;
use crypto::Hash;
use doli_core::validation::ValidationError;

// OUTPUT CONTRACT: fn not_yet(&anyhow::Error) -> bool
//   O1 return
// PATHS: typed TimestampTooFuture | any other typed ValidationError | untyped anyhow error
// INPUT PARTITIONS: future-time variant | other time variants | body/header variants
//   | String-carrying variant with the future-time text | anyhow! string with the same text

fn of(e: ValidationError) -> anyhow::Error {
    anyhow::Error::from(e)
}

// REQ-CLOCK-SEC-001 — Decision: a failure means a block fault other than the future-time rule is classed not-yet and escapes today's handling.
#[test]
fn not_yet_only_for_timestamp_too_future() {
    assert!(not_yet(&of(ValidationError::TimestampTooFuture(60))));
    assert!(not_yet(&of(ValidationError::TimestampTooFuture(0))));
    assert!(not_yet(&of(ValidationError::TimestampTooFuture(u64::MAX))));

    let others = [
        ValidationError::SlotTooFuture {
            got: 6,
            current: 4,
            max_future: 1,
        },
        ValidationError::SlotTooPast {
            got: 1,
            current: 9,
            max_past: 1,
        },
        ValidationError::InvalidTimestamp {
            block: 60,
            expected: 61,
        },
        ValidationError::InvalidSlot {
            got: 6,
            expected: 5,
        },
        ValidationError::SlotNotAdvancing { got: 5, prev: 5 },
        ValidationError::InvalidMerkleRoot {
            header: Hash::ZERO,
            computed: crypto::hash::hash(b"x"),
        },
        ValidationError::GenesisHashMismatch {
            got: Hash::ZERO,
            expected: crypto::hash::hash(b"g"),
        },
        ValidationError::InvalidVersion(1),
        ValidationError::BlockTooLarge { size: 2, max: 1 },
        ValidationError::InvalidBlockSignature {
            reason: "timestamp too far in future".into(),
        },
        ValidationError::InvalidBlock("timestamp too far in future: 60".into()),
    ];
    for e in others {
        let shown = format!("{e:?}");
        assert!(!not_yet(&of(e)), "not not-yet: {shown}");
    }

    assert!(!not_yet(&anyhow::anyhow!("timestamp too far in future")));
    assert!(!not_yet(&anyhow::anyhow!(
        "timestamp too far in future: 60"
    )));
}

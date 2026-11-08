use super::hold::CLOCK_HOLD_CAP;
use super::{not_yet, ClockHold};
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

fn block_at(ts: u64, salt: u8) -> doli_core::Block {
    let header = doli_core::BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: ts,
        slot: 1,
        producer: *crypto::KeyPair::generate().public_key(),
        vdf_output: vdf::VdfOutput {
            value: vec![salt; 32],
        },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    doli_core::Block::new(header, Vec::new())
}

// REQ-CLOCK-SEC-003 — Decision: a failure means the hold grows past its cap or drops the newest entry.
#[test]
fn hold_caps_at_32_dropping_oldest() {
    let mut h = ClockHold::default();
    let blocks: Vec<_> = (0..=CLOCK_HOLD_CAP as u8)
        .map(|i| block_at(10, i))
        .collect();
    for b in &blocks {
        h.insert(b.clone(), network::PeerId::random());
    }
    assert_eq!(h.len(), CLOCK_HOLD_CAP);
    assert!(!h.contains(&blocks[0].hash()));
    assert!(h.contains(&blocks[1].hash()));
    assert!(h.contains(&blocks[CLOCK_HOLD_CAP].hash()));
}

// REQ-CLOCK-002 — Decision: a failure means one block takes two entries or loses its first supplier.
#[test]
fn hold_dedups_by_hash_keeping_first_supplier() {
    let mut h = ClockHold::default();
    let b = block_at(10, 0);
    let first = network::PeerId::random();
    h.insert(b.clone(), first);
    h.insert(b.clone(), network::PeerId::random());
    assert_eq!(h.len(), 1);
    let due = h.take_due(10);
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].1, first);
}

// REQ-CLOCK-003 — Decision: a failure means a block is released before its time or kept after it.
#[test]
fn take_due_releases_only_at_or_before_limit() {
    let mut h = ClockHold::default();
    let early = block_at(10, 0);
    let late = block_at(11, 1);
    h.insert(early.clone(), network::PeerId::random());
    h.insert(late.clone(), network::PeerId::random());
    assert!(h.take_due(9).is_empty());
    let due = h.take_due(10);
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].0.hash(), early.hash());
    assert!(h.contains(&late.hash()));
    assert_eq!(h.len(), 1);
}

//! INC-I-273 M2 — ProducerSetDigest parameter bounds at decode.
//!
//! OUTPUT CONTRACT: fn decode_digest(&[u8]) -> Result<ProducerBloomFilter, ProtoError>;
//!   fn ProducerGSet::delta_for_peer(&self, &ProducerBloomFilter) -> Vec<ProducerAnnouncement>
//!   O1: none (bytes borrowed)   O2: none (&self)
//!   O3: decode Result (Ok fields: hash_count, size_bits, element_count, to_bytes);
//!       delta_for_peer Vec (pubkeys)
//!   O4: none   O5: none   O6: none
//! PATHS: P1 honest digest -> Ok, fields equal, delta equal to the original filter's;
//!   P2 boundary-accept (size_bits == 8*len, size_bits inside the last byte, k = 1, k = 16) -> Ok;
//!   P3 boundary-reject (size_bits = 8*len + 1, k = 17) -> Err;
//!   P4 malformed (size_bits > 8*len with empty / 1-byte filter, size_bits = 0, k = 0,
//!      k = u32::MAX) -> Err, and the decode-then-delta path never runs
//! INPUT PARTITIONS: filter len {0, 1, 8, honest ceil(m/8)}; size_bits {0, 57, 64, 65, 65536,
//!   honest}; bloom_k {0, 1, 16, 17, u32::MAX, honest}; GSet size {1, 5, 100}
//! MATRIX: O3(decode) x P1..P4 and O3(delta) x {P1, P2, P4}; one test per partition.
// covers: bloom.rs proto.rs relay_decision.rs network_events.rs

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::{decode_digest, encode_digest, ProducerAnnouncement, ProducerGSet};

fn varint(mut v: u64, out: &mut Vec<u8>) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// ProducerSetDigest wire bytes; zero / empty fields are omitted like prost does.
pub fn digest_bytes(filter: &[u8], k: u32, count: u32, size_bits: u32) -> Vec<u8> {
    let mut out = Vec::new();
    if !filter.is_empty() {
        out.push(0x0a);
        varint(filter.len() as u64, &mut out);
        out.extend_from_slice(filter);
    }
    for (tag, v) in [(0x10u8, k), (0x18, count), (0x20, size_bits)] {
        if v != 0 {
            out.push(tag);
            varint(u64::from(v), &mut out);
        }
    }
    out
}

pub fn keypair(i: u32) -> KeyPair {
    let mut seed = [0x5Cu8; 32];
    seed[..4].copy_from_slice(&i.to_le_bytes());
    KeyPair::from_seed(seed)
}

pub fn gset_of(ids: impl IntoIterator<Item = u32>) -> ProducerGSet {
    let mut g = ProducerGSet::new(1, Hash::ZERO);
    for i in ids {
        g.merge_one(ProducerAnnouncement::new(&keypair(i), 1, 1, Hash::ZERO))
            .expect("setup: valid announcement");
    }
    g
}

fn pubkeys(anns: &[ProducerAnnouncement]) -> Vec<PublicKey> {
    anns.iter().map(|a| a.pubkey).collect()
}

/// The node's digest path: decode, then the delta against a populated GSet.
fn decode_then_delta(bytes: &[u8]) -> bool {
    let gset = gset_of(0..5);
    let decoded = decode_digest(bytes);
    if let Ok(b) = &decoded {
        let _ = gset.delta_for_peer(b);
    }
    decoded.is_err()
}

// REQ-VOTE-003 — Decision: a failure means a digest whose size_bits exceeds an empty filter reaches delta_for_peer.
#[test]
fn digest_with_size_bits_beyond_empty_filter_is_rejected_at_decode() {
    let bytes = digest_bytes(&[], 1, 0, 65_536);
    assert_eq!(
        bytes,
        [0x10, 0x01, 0x20, 0x80, 0x80, 0x04],
        "setup: wire vector"
    );
    assert!(
        decode_then_delta(&bytes),
        "size_bits 65536 with an empty filter must not decode"
    );
}

// REQ-VOTE-003 — Decision: a failure means a digest whose size_bits exceeds a 1-byte filter reaches delta_for_peer.
#[test]
fn digest_with_size_bits_beyond_one_byte_filter_is_rejected_at_decode() {
    let bytes = digest_bytes(&[0xFF], 1, 0, 65_536);
    assert!(
        decode_then_delta(&bytes),
        "size_bits 65536 with a 1-byte filter must not decode"
    );
}

// REQ-VOTE-003 — Decision: a failure means a digest with size_bits 0 and k >= 1 reaches the bit-index computation.
#[test]
fn digest_with_zero_size_bits_is_rejected_at_decode() {
    let bytes = digest_bytes(&[0u8; 8], 1, 0, 0);
    assert!(decode_then_delta(&bytes), "size_bits 0 must not decode");
}

// REQ-VOTE-003 — Decision: a failure means a peer-chosen hash count above the encoder cap of 16 is accepted.
#[test]
fn digest_with_hash_count_above_cap_is_rejected_at_decode() {
    for k in [17, u32::MAX] {
        let bytes = digest_bytes(&[0u8; 8], k, 0, 64);
        assert!(
            decode_digest(&bytes).is_err(),
            "bloom_k {k} must not decode"
        );
    }
}

// REQ-VOTE-003 — Decision: a failure means a digest with hash count 0 (matches every key) is accepted.
#[test]
fn digest_with_zero_hash_count_is_rejected_at_decode() {
    let bytes = digest_bytes(&[0u8; 8], 0, 0, 64);
    assert!(decode_then_delta(&bytes), "bloom_k 0 must not decode");
}

// REQ-VOTE-003 — Decision: a failure means the bound is off by one bit (size_bits = 8 * len + 1 accepted).
#[test]
fn digest_with_size_bits_one_past_filter_is_rejected_at_decode() {
    let bytes = digest_bytes(&[0u8; 8], 1, 0, 65);
    assert!(
        decode_digest(&bytes).is_err(),
        "size_bits 65 with an 8-byte filter must not decode"
    );
}

// REQ-VOTE-003 — Decision: a failure means the bound rejects digests whose size_bits fits the filter (honest peers lose delta sync).
#[test]
fn digest_with_size_bits_within_filter_is_accepted() {
    let filter = [0xA5u8; 8];
    for m in [64u32, 57] {
        let b = decode_digest(&digest_bytes(&filter, 3, 2, m))
            .unwrap_or_else(|e| panic!("size_bits {m} with 8 bytes must decode: {e:?}"));
        assert_eq!(
            (b.hash_count(), b.element_count(), b.size_bits()),
            (3, 2, m as usize),
            "size_bits {m}: decoded parameters"
        );
        assert_eq!(b.to_bytes(), filter, "size_bits {m}: filter bytes");
        assert!(!decode_then_delta(&digest_bytes(&filter, 3, 2, m)));
    }
}

// REQ-VOTE-003 — Decision: a failure means the hash-count bound rejects the encoder's own range 1..=16.
#[test]
fn digest_with_hash_count_at_bounds_is_accepted() {
    for k in [1u32, 16] {
        let b = decode_digest(&digest_bytes(&[0u8; 8], k, 0, 64))
            .unwrap_or_else(|e| panic!("bloom_k {k} must decode: {e:?}"));
        assert_eq!(b.hash_count(), k as usize, "bloom_k {k}");
        assert_eq!(b.size_bits(), 64, "bloom_k {k}: size_bits");
        let delta = gset_of(0..5).delta_for_peer(&b);
        assert_eq!(
            delta.len(),
            5,
            "bloom_k {k}: an empty filter lacks every producer"
        );
    }
}

// REQ-VOTE-003 — Decision: a failure means the fix changed what an honest digest decodes to or the delta it yields.
#[test]
fn honest_digest_round_trip_is_unchanged() {
    for n in [1u32, 5, 100] {
        let ours = gset_of(0..n);
        let bloom = ours.to_bloom_filter();
        let bytes = encode_digest(&bloom);
        let hand = digest_bytes(
            &bloom.to_bytes(),
            bloom.hash_count() as u32,
            bloom.element_count() as u32,
            bloom.size_bits() as u32,
        );
        assert_eq!(hand, bytes, "setup: hand encoder matches prost (n = {n})");

        let decoded = decode_digest(&bytes)
            .unwrap_or_else(|e| panic!("honest digest of {n} must decode: {e:?}"));
        assert_eq!(decoded.hash_count(), bloom.hash_count(), "n = {n}: k");
        assert_eq!(decoded.size_bits(), bloom.size_bits(), "n = {n}: m");
        assert_eq!(
            decoded.element_count(),
            bloom.element_count(),
            "n = {n}: count"
        );
        assert_eq!(decoded.to_bytes(), bloom.to_bytes(), "n = {n}: bits");

        let theirs = gset_of((n / 2)..(n + 20));
        let via_wire = pubkeys(&theirs.delta_for_peer(&decoded));
        let direct = pubkeys(&theirs.delta_for_peer(&bloom));
        assert_eq!(via_wire, direct, "n = {n}: delta through the wire");
        assert!(
            via_wire.iter().all(|pk| !ours.contains(pk)),
            "n = {n}: delta holds a producer the digest already has"
        );
        assert!(!via_wire.is_empty(), "n = {n}: the peer lacks producers");
    }
}

//! INC-I-273 M3 — ProducerSetDigest filter length equals ceil(size_bits / 8).
//!
//! OUTPUT CONTRACT: fn decode_digest(&[u8]) -> Result<ProducerBloomFilter, ProtoError>;
//!   fn ProducerGSet::delta_for_peer(&self, &ProducerBloomFilter) -> Vec<ProducerAnnouncement>
//!   O1: none (bytes borrowed)   O2: none (&self)
//!   O3: decode Result (Err variant; Ok fields hash_count, size_bits, element_count, to_bytes);
//!       delta_for_peer Vec (pubkeys)
//!   O4: none   O5: none   O6: none
//! PATHS: P1 filter longer than ceil(size_bits / 8) -> Err(InvalidDigest);
//!   P2 filter exactly ceil(size_bits / 8) -> Ok, fields and bytes equal;
//!   P3 honest encode -> decode round trip -> Ok, exact length, delta equal to the original's
//! INPUT PARTITIONS: (size_bits, filter bytes) reject {(8, 2 MiB), (8, 2), (64, 9), (57, 9)};
//!   accept {(64, 8), (57, 8), (8, 1), (1, 1)}; honest set size {1, 2, 7, 8, 63, 64, 100, 1000, 5000}
//! MATRIX: O3(decode) x P1..P3 and O3(delta) x P3; one test per reject partition.
// covers: bloom proto

use crypto::PublicKey;
use doli_core::{decode_digest, encode_digest, ProducerBloomFilter, ProtoError};

use super::inc_i_273_m2_digest_bounds::{digest_bytes, gset_of, keypair};

fn decode_len(size_bits: u32, len: usize) -> Result<ProducerBloomFilter, ProtoError> {
    decode_digest(&digest_bytes(&vec![0xA5u8; len], 1, 0, size_bits))
}

fn assert_invalid(size_bits: u32, len: usize) {
    let got = decode_len(size_bits, len);
    assert!(
        matches!(got, Err(ProtoError::InvalidDigest { .. })),
        "size_bits {size_bits} with a {len}-byte filter must fail with InvalidDigest, got {:?}",
        got.map(|b| b.to_bytes().len())
    );
}

/// The bloom GSet::to_bloom_filter builds for producers 0..n.
fn bloom_of(n: u32) -> ProducerBloomFilter {
    let mut b = ProducerBloomFilter::new((n as usize).max(1));
    for i in 0..n {
        b.insert(keypair(i).public_key());
    }
    b
}

fn sorted(mut keys: Vec<PublicKey>) -> Vec<PublicKey> {
    keys.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    keys
}

// REQ-VOTE-003 — Decision: a failure means a filter far longer than size_bits needs is decoded and kept.
#[test]
fn digest_with_filter_far_longer_than_size_bits_is_rejected_at_decode() {
    let got = decode_len(8, 2 * 1024 * 1024);
    let accepted = got.as_ref().map_or(0, |b| b.to_bytes().len());
    println!("M3-PROBE digest_filter_bytes_accepted={accepted}");
    assert!(
        matches!(got, Err(ProtoError::InvalidDigest { .. })),
        "size_bits 8 with a 2 MiB filter must fail with InvalidDigest ({accepted} bytes kept)"
    );
}

// REQ-VOTE-003 — Decision: a failure means one spare byte after a whole-byte size_bits is accepted.
#[test]
fn digest_with_one_spare_byte_after_eight_bits_is_rejected_at_decode() {
    assert_invalid(8, 2);
}

// REQ-VOTE-003 — Decision: a failure means one spare byte after the 64-bit minimum filter is accepted.
#[test]
fn digest_with_one_spare_byte_after_sixty_four_bits_is_rejected_at_decode() {
    assert_invalid(64, 9);
}

// REQ-VOTE-003 — Decision: a failure means the length check rounds wrongly when size_bits ends inside a byte.
#[test]
fn digest_with_one_spare_byte_after_partial_byte_is_rejected_at_decode() {
    assert_invalid(57, 9);
}

// REQ-VOTE-003 — Decision: a failure means the exact-length check rejects a filter of exactly ceil(size_bits / 8) bytes.
#[test]
fn digest_with_filter_of_exactly_ceil_size_bits_is_accepted() {
    for (m, len) in [(64u32, 8usize), (57, 8), (8, 1), (1, 1)] {
        let b = decode_len(m, len)
            .unwrap_or_else(|e| panic!("size_bits {m} with {len} bytes must decode: {e:?}"));
        assert_eq!(
            (b.hash_count(), b.size_bits()),
            (1, m as usize),
            "size_bits {m}: decoded parameters"
        );
        assert_eq!(
            b.to_bytes(),
            vec![0xA5u8; len],
            "size_bits {m}: filter bytes"
        );
    }
}

// REQ-VOTE-003 — Decision: a failure means an honest encoder output for some set size does not have the exact filter length, so the fix would cut honest delta sync.
#[test]
fn honest_digest_has_exact_filter_length_and_round_trips_for_every_set_size() {
    for n in [1u32, 2, 7, 8, 63, 64, 100, 1000, 5000] {
        let bloom = bloom_of(n);
        if n <= 100 {
            let via_gset = gset_of(0..n).to_bloom_filter();
            assert_eq!(
                (
                    via_gset.size_bits(),
                    via_gset.hash_count(),
                    via_gset.to_bytes()
                ),
                (bloom.size_bits(), bloom.hash_count(), bloom.to_bytes()),
                "setup: n = {n}: bloom_of matches GSet::to_bloom_filter"
            );
        }
        assert_eq!(
            bloom.to_bytes().len(),
            bloom.size_bits().div_ceil(8),
            "n = {n}: encoder filter length"
        );

        let decoded = decode_digest(&encode_digest(&bloom))
            .unwrap_or_else(|e| panic!("honest digest of {n} must decode: {e:?}"));
        assert_eq!(decoded.hash_count(), bloom.hash_count(), "n = {n}: k");
        assert_eq!(decoded.size_bits(), bloom.size_bits(), "n = {n}: m");
        assert_eq!(
            decoded.element_count(),
            bloom.element_count(),
            "n = {n}: count"
        );
        assert_eq!(decoded.to_bytes(), bloom.to_bytes(), "n = {n}: bits");

        let theirs = gset_of(n.saturating_sub(10)..n + 10);
        let pubkeys = |anns: Vec<doli_core::ProducerAnnouncement>| {
            sorted(anns.iter().map(|a| a.pubkey).collect())
        };
        let via_wire = pubkeys(theirs.delta_for_peer(&decoded));
        let direct = pubkeys(theirs.delta_for_peer(&bloom));
        assert_eq!(via_wire, direct, "n = {n}: delta through the wire");
        assert!(!via_wire.is_empty(), "n = {n}: the peer lacks producers");
        for i in n.saturating_sub(10)..n {
            assert!(
                !via_wire.contains(keypair(i).public_key()),
                "n = {n}: delta holds producer {i} the digest already has"
            );
        }
    }
}

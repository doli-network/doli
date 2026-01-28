//! Protocol Buffers Wire Format for Producer Discovery
//!
//! This module provides the protobuf serialization for producer announcements
//! and related types. It supports forward compatibility with unknown fields
//! and provides conversion traits between Rust types and protobuf messages.

use crypto::{PublicKey, Signature};
use prost::Message;

use super::{ProducerAnnouncement, ProducerBloomFilter};

// Include the generated protobuf code
pub mod producer {
    include!(concat!(env!("OUT_DIR"), "/doli.producer.rs"));
}

/// Error type for protobuf conversion failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtoError {
    /// Invalid public key bytes (wrong length).
    #[error("invalid public key: expected 32 bytes, got {0}")]
    InvalidPublicKey(usize),

    /// Invalid signature bytes (wrong length).
    #[error("invalid signature: expected 64 bytes, got {0}")]
    InvalidSignature(usize),

    /// Failed to decode protobuf message.
    #[error("protobuf decode error: {0}")]
    DecodeError(String),

    /// Missing required field in protobuf message.
    #[error("missing required field: {0}")]
    MissingField(&'static str),
}

impl From<prost::DecodeError> for ProtoError {
    fn from(e: prost::DecodeError) -> Self {
        ProtoError::DecodeError(e.to_string())
    }
}

// ============================================================================
// ProducerAnnouncement <-> proto::ProducerAnnouncement
// ============================================================================

impl From<ProducerAnnouncement> for producer::ProducerAnnouncement {
    fn from(ann: ProducerAnnouncement) -> Self {
        Self {
            pubkey: ann.pubkey.as_bytes().to_vec(),
            network_id: ann.network_id,
            sequence: ann.sequence,
            timestamp: ann.timestamp,
            signature: ann.signature.as_bytes().to_vec(),
        }
    }
}

impl TryFrom<producer::ProducerAnnouncement> for ProducerAnnouncement {
    type Error = ProtoError;

    fn try_from(proto: producer::ProducerAnnouncement) -> Result<Self, Self::Error> {
        // Validate and convert public key
        if proto.pubkey.len() != 32 {
            return Err(ProtoError::InvalidPublicKey(proto.pubkey.len()));
        }
        let pubkey_bytes: [u8; 32] = proto.pubkey.try_into().unwrap();
        let pubkey = PublicKey::from_bytes(pubkey_bytes);

        // Validate and convert signature
        if proto.signature.len() != 64 {
            return Err(ProtoError::InvalidSignature(proto.signature.len()));
        }
        let sig_bytes: [u8; 64] = proto.signature.try_into().unwrap();
        let signature = Signature::from_bytes(sig_bytes);

        Ok(Self {
            pubkey,
            network_id: proto.network_id,
            sequence: proto.sequence,
            timestamp: proto.timestamp,
            signature,
        })
    }
}

// ============================================================================
// ProducerBloomFilter <-> proto::ProducerSetDigest
// ============================================================================

impl From<&ProducerBloomFilter> for producer::ProducerSetDigest {
    fn from(bloom: &ProducerBloomFilter) -> Self {
        Self {
            bloom_filter: bloom.to_bytes(),
            bloom_k: bloom.hash_count() as u32,
            count: bloom.element_count() as u32,
            size_bits: bloom.size_bits() as u32,
        }
    }
}

impl From<producer::ProducerSetDigest> for ProducerBloomFilter {
    fn from(proto: producer::ProducerSetDigest) -> Self {
        ProducerBloomFilter::from_bytes(
            &proto.bloom_filter,
            proto.bloom_k as usize,
            proto.count as usize,
            proto.size_bits as usize,
        )
    }
}

// ============================================================================
// Vec<ProducerAnnouncement> <-> proto::ProducerSet
// ============================================================================

impl From<Vec<ProducerAnnouncement>> for producer::ProducerSet {
    fn from(announcements: Vec<ProducerAnnouncement>) -> Self {
        Self {
            producers: announcements.into_iter().map(Into::into).collect(),
        }
    }
}

impl TryFrom<producer::ProducerSet> for Vec<ProducerAnnouncement> {
    type Error = ProtoError;

    fn try_from(proto: producer::ProducerSet) -> Result<Self, Self::Error> {
        proto.producers.into_iter().map(TryInto::try_into).collect()
    }
}

// ============================================================================
// Encoding/Decoding Helpers
// ============================================================================

/// Encode a producer announcement to protobuf bytes.
pub fn encode_announcement(ann: &ProducerAnnouncement) -> Vec<u8> {
    let proto: producer::ProducerAnnouncement = ann.clone().into();
    proto.encode_to_vec()
}

/// Decode a producer announcement from protobuf bytes.
pub fn decode_announcement(bytes: &[u8]) -> Result<ProducerAnnouncement, ProtoError> {
    let proto = producer::ProducerAnnouncement::decode(bytes)?;
    proto.try_into()
}

/// Encode a producer set to protobuf bytes.
pub fn encode_producer_set(announcements: &[ProducerAnnouncement]) -> Vec<u8> {
    let proto: producer::ProducerSet = announcements.to_vec().into();
    proto.encode_to_vec()
}

/// Decode a producer set from protobuf bytes.
pub fn decode_producer_set(bytes: &[u8]) -> Result<Vec<ProducerAnnouncement>, ProtoError> {
    let proto = producer::ProducerSet::decode(bytes)?;
    proto.try_into()
}

/// Encode a bloom filter digest to protobuf bytes.
pub fn encode_digest(bloom: &ProducerBloomFilter) -> Vec<u8> {
    let proto: producer::ProducerSetDigest = bloom.into();
    proto.encode_to_vec()
}

/// Decode a bloom filter digest from protobuf bytes.
pub fn decode_digest(bytes: &[u8]) -> Result<ProducerBloomFilter, ProtoError> {
    let proto = producer::ProducerSetDigest::decode(bytes)?;
    Ok(proto.into())
}

/// Check if bytes look like a legacy bincode format vs new protobuf format.
///
/// Heuristic: bincode Vec<PublicKey> starts with a u64 length (little-endian).
/// For typical producer counts (1-1000), this will be < 1000.
/// Protobuf messages start with field tags, which have different patterns.
pub fn is_legacy_bincode_format(bytes: &[u8]) -> bool {
    if bytes.len() < 8 {
        return false;
    }

    // Try to read as bincode u64 length prefix
    let len = u64::from_le_bytes(bytes[0..8].try_into().unwrap());

    // Bincode format: length + (length * 32 bytes for pubkeys)
    // If length is reasonable and total size matches, likely bincode
    if len <= 10000 && bytes.len() == 8 + (len as usize * 32) {
        return true;
    }

    // Otherwise, try protobuf decode - if it fails, might be bincode
    producer::ProducerSet::decode(bytes).is_err()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::KeyPair;

    #[test]
    fn test_proto_announcement_roundtrip() {
        let keypair = KeyPair::generate();
        let rust_ann = ProducerAnnouncement::new(&keypair, 1, 0);

        // Convert to protobuf
        let proto_ann: producer::ProducerAnnouncement = rust_ann.clone().into();
        let bytes = proto_ann.encode_to_vec();

        // Decode back
        let decoded = producer::ProducerAnnouncement::decode(&bytes[..]).unwrap();
        let restored: ProducerAnnouncement = decoded.try_into().unwrap();

        assert_eq!(rust_ann.pubkey, restored.pubkey);
        assert_eq!(rust_ann.sequence, restored.sequence);
        assert_eq!(rust_ann.network_id, restored.network_id);
        assert_eq!(rust_ann.timestamp, restored.timestamp);
        assert!(restored.verify());
    }

    #[test]
    fn test_proto_forward_compatibility() {
        // Simulate receiving a message with unknown fields (future version)
        let keypair = KeyPair::generate();
        let ann = ProducerAnnouncement::new(&keypair, 1, 0);
        let proto_ann: producer::ProducerAnnouncement = ann.into();

        let mut bytes = proto_ann.encode_to_vec();
        // Append unknown field (field 99 with value 42)
        // Wire format: field 99, varint type = (99 << 3) | 0 = 792 = 0xF8 0x06
        // Value 42 = 0x2A
        bytes.extend_from_slice(&[0xF8, 0x06, 0x2A]);

        // Should decode successfully, ignoring unknown field
        let decoded = producer::ProducerAnnouncement::decode(&bytes[..]).unwrap();
        assert_eq!(decoded.pubkey.len(), 32);
    }

    #[test]
    fn test_proto_set_roundtrip() {
        let announcements: Vec<_> = (0..5)
            .map(|i| {
                let keypair = KeyPair::generate();
                ProducerAnnouncement::new(&keypair, 1, i)
            })
            .collect();

        let bytes = encode_producer_set(&announcements);
        let restored = decode_producer_set(&bytes).unwrap();

        assert_eq!(announcements.len(), restored.len());
        for (orig, rest) in announcements.iter().zip(restored.iter()) {
            assert_eq!(orig.pubkey, rest.pubkey);
            assert!(rest.verify());
        }
    }

    #[test]
    fn test_proto_digest_roundtrip() {
        let mut bloom = ProducerBloomFilter::new(100);
        for _ in 0..50 {
            let keypair = KeyPair::generate();
            bloom.insert(keypair.public_key());
        }

        let bytes = encode_digest(&bloom);
        let restored = decode_digest(&bytes).unwrap();

        assert_eq!(bloom.hash_count(), restored.hash_count());
        assert_eq!(bloom.element_count(), restored.element_count());
        assert_eq!(bloom.size_bits(), restored.size_bits());
    }

    #[test]
    fn test_proto_set_request_full() {
        let request = producer::ProducerSetRequest {
            request: Some(producer::producer_set_request::Request::FullSet(true)),
        };
        let bytes = request.encode_to_vec();
        let decoded = producer::ProducerSetRequest::decode(&bytes[..]).unwrap();
        assert!(matches!(
            decoded.request,
            Some(producer::producer_set_request::Request::FullSet(true))
        ));
    }

    #[test]
    fn test_proto_set_request_delta() {
        let bloom = ProducerBloomFilter::new(100);
        let digest: producer::ProducerSetDigest = (&bloom).into();
        let request = producer::ProducerSetRequest {
            request: Some(producer::producer_set_request::Request::Have(digest)),
        };
        let bytes = request.encode_to_vec();
        let decoded = producer::ProducerSetRequest::decode(&bytes[..]).unwrap();
        assert!(matches!(
            decoded.request,
            Some(producer::producer_set_request::Request::Have(_))
        ));
    }

    #[test]
    fn test_proto_response() {
        let keypair = KeyPair::generate();
        let ann = ProducerAnnouncement::new(&keypair, 1, 0);
        let proto_ann: producer::ProducerAnnouncement = ann.into();

        let response = producer::ProducerSetResponse {
            producers: vec![proto_ann],
            is_full_set: true,
            total_known: 1,
        };
        let bytes = response.encode_to_vec();
        let decoded = producer::ProducerSetResponse::decode(&bytes[..]).unwrap();
        assert_eq!(decoded.producers.len(), 1);
        assert!(decoded.is_full_set);
        assert_eq!(decoded.total_known, 1);
    }

    #[test]
    fn test_proto_invalid_pubkey_length() {
        let proto = producer::ProducerAnnouncement {
            pubkey: vec![0u8; 16], // Wrong length
            network_id: 1,
            sequence: 0,
            timestamp: 0,
            signature: vec![0u8; 64],
        };
        let result: Result<ProducerAnnouncement, _> = proto.try_into();
        assert!(matches!(result, Err(ProtoError::InvalidPublicKey(16))));
    }

    #[test]
    fn test_proto_invalid_signature_length() {
        let proto = producer::ProducerAnnouncement {
            pubkey: vec![0u8; 32],
            network_id: 1,
            sequence: 0,
            timestamp: 0,
            signature: vec![0u8; 32], // Wrong length
        };
        let result: Result<ProducerAnnouncement, _> = proto.try_into();
        assert!(matches!(result, Err(ProtoError::InvalidSignature(32))));
    }

    #[test]
    fn test_legacy_format_detection() {
        // Create a fake bincode format: 2 pubkeys
        let mut bincode_bytes = Vec::new();
        bincode_bytes.extend_from_slice(&2u64.to_le_bytes()); // length = 2
        bincode_bytes.extend_from_slice(&[0u8; 32]); // pubkey 1
        bincode_bytes.extend_from_slice(&[0u8; 32]); // pubkey 2
        assert!(is_legacy_bincode_format(&bincode_bytes));

        // Create protobuf format
        let keypair = KeyPair::generate();
        let ann = ProducerAnnouncement::new(&keypair, 1, 0);
        let proto_bytes = encode_producer_set(&[ann]);
        assert!(!is_legacy_bincode_format(&proto_bytes));
    }

    #[test]
    fn test_encode_decode_helpers() {
        let keypair = KeyPair::generate();
        let ann = ProducerAnnouncement::new(&keypair, 1, 42);

        // Test single announcement
        let bytes = encode_announcement(&ann);
        let decoded = decode_announcement(&bytes).unwrap();
        assert_eq!(ann.pubkey, decoded.pubkey);
        assert_eq!(ann.sequence, decoded.sequence);
        assert!(decoded.verify());
    }

    #[test]
    fn test_proto_error_display() {
        let err = ProtoError::InvalidPublicKey(16);
        assert!(err.to_string().contains("16"));

        let err = ProtoError::InvalidSignature(32);
        assert!(err.to_string().contains("32"));

        let err = ProtoError::MissingField("pubkey");
        assert!(err.to_string().contains("pubkey"));
    }

    #[test]
    fn test_proto_error_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ProtoError>();
    }

    #[test]
    fn test_proto_message_size() {
        let keypair = KeyPair::generate();
        let ann = ProducerAnnouncement::new(&keypair, 1, 0);
        let bytes = encode_announcement(&ann);

        // Single announcement should be ~130 bytes
        // pubkey (32) + signature (64) + network_id (1-5) + sequence (1-10) + timestamp (1-10)
        // Plus protobuf overhead (field tags, length prefixes)
        assert!(
            bytes.len() < 150,
            "Single announcement {} bytes, expected < 150",
            bytes.len()
        );
    }

    #[test]
    fn test_proto_batch_encoding() {
        let announcements: Vec<_> = (0..100)
            .map(|i| {
                let keypair = KeyPair::generate();
                ProducerAnnouncement::new(&keypair, 1, i)
            })
            .collect();

        let bytes = encode_producer_set(&announcements);

        // 100 announcements at ~130 bytes each = ~13KB
        // Plus some overhead for repeated field encoding
        assert!(
            bytes.len() < 15000,
            "100 announcements {} bytes, expected < 15KB",
            bytes.len()
        );
    }
}

//! Producer Announcement Type
//!
//! A cryptographically signed producer announcement with replay protection.
//! This is the fundamental building block for the producer discovery system.

use crypto::{signature, KeyPair, PrivateKey, PublicKey, Signature};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use super::PRODUCER_ANNOUNCEMENT_DOMAIN;

/// A cryptographically signed producer announcement.
///
/// This structure allows producers to announce their presence on the network
/// with protection against:
/// - Impersonation (signature verification)
/// - Replay attacks (sequence numbers)
/// - Cross-network attacks (network_id binding)
/// - Clock drift attacks (timestamp bounds)
///
/// # Example
///
/// ```rust
/// use doli_core::discovery::ProducerAnnouncement;
/// use crypto::KeyPair;
///
/// let keypair = KeyPair::generate();
/// let announcement = ProducerAnnouncement::new(&keypair, 1, 0);
/// assert!(announcement.verify());
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProducerAnnouncement {
    /// The producer's public key.
    pub pubkey: PublicKey,

    /// Network ID this announcement is for (prevents cross-network replay).
    pub network_id: u32,

    /// Monotonically increasing sequence number for this producer.
    /// Higher sequence numbers supersede lower ones.
    pub sequence: u64,

    /// Unix timestamp (seconds since epoch) when the announcement was created.
    pub timestamp: u64,

    /// Ed25519 signature over (network_id || sequence || timestamp || pubkey).
    pub signature: Signature,
}

impl ProducerAnnouncement {
    /// Create a new signed producer announcement.
    ///
    /// The announcement is signed with the provided keypair and includes:
    /// - The current timestamp
    /// - The provided network ID and sequence number
    /// - A signature over all fields for authenticity
    ///
    /// # Arguments
    ///
    /// * `keypair` - The producer's keypair for signing
    /// * `network_id` - The network ID (1=mainnet, 2=testnet, 99=devnet)
    /// * `sequence` - Monotonically increasing sequence number
    ///
    /// # Example
    ///
    /// ```rust
    /// use doli_core::discovery::ProducerAnnouncement;
    /// use crypto::KeyPair;
    ///
    /// let keypair = KeyPair::generate();
    /// let ann = ProducerAnnouncement::new(&keypair, 1, 0);
    /// assert!(ann.verify());
    /// ```
    #[must_use]
    pub fn new(keypair: &KeyPair, network_id: u32, sequence: u64) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_secs();

        let pubkey = *keypair.public_key();
        let message = Self::message_bytes_inner(network_id, sequence, timestamp, &pubkey);
        let signature = signature::sign_with_domain(
            PRODUCER_ANNOUNCEMENT_DOMAIN,
            &message,
            keypair.private_key(),
        );

        Self {
            pubkey,
            network_id,
            sequence,
            timestamp,
            signature,
        }
    }

    /// Create a new signed producer announcement with a specific timestamp.
    ///
    /// This is primarily useful for testing scenarios where you need
    /// control over the timestamp.
    ///
    /// # Arguments
    ///
    /// * `keypair` - The producer's keypair for signing
    /// * `network_id` - The network ID
    /// * `sequence` - Monotonically increasing sequence number
    /// * `timestamp` - Unix timestamp to use
    #[must_use]
    pub fn new_with_timestamp(
        keypair: &KeyPair,
        network_id: u32,
        sequence: u64,
        timestamp: u64,
    ) -> Self {
        let pubkey = *keypair.public_key();
        let message = Self::message_bytes_inner(network_id, sequence, timestamp, &pubkey);
        let signature = signature::sign_with_domain(
            PRODUCER_ANNOUNCEMENT_DOMAIN,
            &message,
            keypair.private_key(),
        );

        Self {
            pubkey,
            network_id,
            sequence,
            timestamp,
            signature,
        }
    }

    /// Verify the announcement's signature.
    ///
    /// Returns `true` if the signature is valid for the announcement's contents.
    ///
    /// # Example
    ///
    /// ```rust
    /// use doli_core::discovery::ProducerAnnouncement;
    /// use crypto::KeyPair;
    ///
    /// let keypair = KeyPair::generate();
    /// let ann = ProducerAnnouncement::new(&keypair, 1, 0);
    /// assert!(ann.verify());
    ///
    /// // Tampering with any field will cause verification to fail
    /// let mut tampered = ann.clone();
    /// tampered.sequence = 999;
    /// assert!(!tampered.verify());
    /// ```
    #[must_use]
    pub fn verify(&self) -> bool {
        let message = self.message_bytes();
        signature::verify_with_domain(
            PRODUCER_ANNOUNCEMENT_DOMAIN,
            &message,
            &self.signature,
            &self.pubkey,
        )
        .is_ok()
    }

    /// Get the bytes that are signed in this announcement.
    ///
    /// The message format is:
    /// - 4 bytes: network_id (little-endian)
    /// - 8 bytes: sequence (little-endian)
    /// - 8 bytes: timestamp (little-endian)
    /// - 32 bytes: pubkey
    ///
    /// Total: 52 bytes
    #[must_use]
    pub fn message_bytes(&self) -> Vec<u8> {
        Self::message_bytes_inner(self.network_id, self.sequence, self.timestamp, &self.pubkey)
    }

    /// Internal helper to construct message bytes.
    fn message_bytes_inner(
        network_id: u32,
        sequence: u64,
        timestamp: u64,
        pubkey: &PublicKey,
    ) -> Vec<u8> {
        let mut message = Vec::with_capacity(52);
        message.extend_from_slice(&network_id.to_le_bytes());
        message.extend_from_slice(&sequence.to_le_bytes());
        message.extend_from_slice(&timestamp.to_le_bytes());
        message.extend_from_slice(pubkey.as_bytes());
        message
    }

    /// Sign an announcement using only the private key.
    ///
    /// This allows creating announcements without a full KeyPair.
    #[must_use]
    pub fn new_from_private_key(private_key: &PrivateKey, network_id: u32, sequence: u64) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_secs();

        let pubkey = private_key.public_key();
        let message = Self::message_bytes_inner(network_id, sequence, timestamp, &pubkey);
        let signature =
            signature::sign_with_domain(PRODUCER_ANNOUNCEMENT_DOMAIN, &message, private_key);

        Self {
            pubkey,
            network_id,
            sequence,
            timestamp,
            signature,
        }
    }
}

impl PartialEq for ProducerAnnouncement {
    fn eq(&self, other: &Self) -> bool {
        self.pubkey == other.pubkey
            && self.network_id == other.network_id
            && self.sequence == other.sequence
            && self.timestamp == other.timestamp
            && self.signature == other.signature
    }
}

impl Eq for ProducerAnnouncement {}

impl std::hash::Hash for ProducerAnnouncement {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.pubkey.as_bytes().hash(state);
        self.network_id.hash(state);
        self.sequence.hash(state);
        self.timestamp.hash(state);
        self.signature.as_bytes().hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_announcement_create_and_verify() {
        let keypair = KeyPair::generate();
        let announcement = ProducerAnnouncement::new(&keypair, 1, 0);
        assert!(announcement.verify());
    }

    #[test]
    fn test_announcement_invalid_signature() {
        let keypair = KeyPair::generate();
        let mut announcement = ProducerAnnouncement::new(&keypair, 1, 0);
        announcement.sequence = 999; // Tamper with data
        assert!(!announcement.verify());
    }

    #[test]
    fn test_announcement_network_id_included() {
        let keypair = KeyPair::generate();
        let ann1 = ProducerAnnouncement::new_with_timestamp(&keypair, 1, 0, 1000);
        let ann2 = ProducerAnnouncement::new_with_timestamp(&keypair, 2, 0, 1000);
        // Different network_id should produce different signatures
        assert_ne!(ann1.signature, ann2.signature);
    }

    #[test]
    fn test_announcement_serialization_roundtrip() {
        let keypair = KeyPair::generate();
        let announcement = ProducerAnnouncement::new(&keypair, 1, 0);
        let bytes = bincode::serialize(&announcement).expect("serialize should succeed");
        let restored: ProducerAnnouncement =
            bincode::deserialize(&bytes).expect("deserialize should succeed");
        assert_eq!(announcement.pubkey, restored.pubkey);
        assert_eq!(announcement.network_id, restored.network_id);
        assert_eq!(announcement.sequence, restored.sequence);
        assert_eq!(announcement.timestamp, restored.timestamp);
        assert!(restored.verify());
    }

    #[test]
    fn test_announcement_timestamp_is_recent() {
        let keypair = KeyPair::generate();
        let announcement = ProducerAnnouncement::new(&keypair, 1, 0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_secs();
        assert!(announcement.timestamp <= now + 5); // Within 5 seconds
        assert!(announcement.timestamp >= now.saturating_sub(5));
    }

    #[test]
    fn test_announcement_message_bytes_length() {
        let keypair = KeyPair::generate();
        let announcement = ProducerAnnouncement::new(&keypair, 1, 0);
        let bytes = announcement.message_bytes();
        // 4 (network_id) + 8 (sequence) + 8 (timestamp) + 32 (pubkey) = 52
        assert_eq!(bytes.len(), 52);
    }

    #[test]
    fn test_announcement_different_sequences_different_signatures() {
        let keypair = KeyPair::generate();
        let ann1 = ProducerAnnouncement::new_with_timestamp(&keypair, 1, 0, 1000);
        let ann2 = ProducerAnnouncement::new_with_timestamp(&keypair, 1, 1, 1000);
        assert_ne!(ann1.signature, ann2.signature);
    }

    #[test]
    fn test_announcement_different_timestamps_different_signatures() {
        let keypair = KeyPair::generate();
        let ann1 = ProducerAnnouncement::new_with_timestamp(&keypair, 1, 0, 1000);
        let ann2 = ProducerAnnouncement::new_with_timestamp(&keypair, 1, 0, 2000);
        assert_ne!(ann1.signature, ann2.signature);
    }

    #[test]
    fn test_announcement_from_private_key() {
        let keypair = KeyPair::generate();
        let ann = ProducerAnnouncement::new_from_private_key(keypair.private_key(), 1, 0);
        assert!(ann.verify());
        assert_eq!(ann.pubkey, *keypair.public_key());
    }

    #[test]
    fn test_announcement_equality() {
        let keypair = KeyPair::generate();
        let ann1 = ProducerAnnouncement::new_with_timestamp(&keypair, 1, 0, 1000);
        let ann2 = ProducerAnnouncement::new_with_timestamp(&keypair, 1, 0, 1000);
        assert_eq!(ann1, ann2);
    }

    #[test]
    fn test_announcement_hash() {
        use std::collections::HashSet;
        let keypair = KeyPair::generate();
        let ann1 = ProducerAnnouncement::new_with_timestamp(&keypair, 1, 0, 1000);
        let ann2 = ProducerAnnouncement::new_with_timestamp(&keypair, 1, 1, 1000);

        let mut set = HashSet::new();
        set.insert(ann1.clone());
        set.insert(ann2.clone());
        assert_eq!(set.len(), 2);

        // Same announcement should be recognized
        set.insert(ann1.clone());
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_announcement_tamper_network_id() {
        let keypair = KeyPair::generate();
        let mut announcement = ProducerAnnouncement::new(&keypair, 1, 0);
        announcement.network_id = 2;
        assert!(!announcement.verify());
    }

    #[test]
    fn test_announcement_tamper_timestamp() {
        let keypair = KeyPair::generate();
        let mut announcement = ProducerAnnouncement::new(&keypair, 1, 0);
        announcement.timestamp += 1;
        assert!(!announcement.verify());
    }

    #[test]
    fn test_announcement_tamper_pubkey() {
        let keypair1 = KeyPair::generate();
        let keypair2 = KeyPair::generate();
        let mut announcement = ProducerAnnouncement::new(&keypair1, 1, 0);
        announcement.pubkey = *keypair2.public_key();
        assert!(!announcement.verify());
    }
}

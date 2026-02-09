//! ProducerBloomFilter - Probabilistic Set for Efficient Delta Sync
//!
//! This module implements a bloom filter for efficient delta synchronization
//! of producer sets between peers. Bloom filters allow peers to exchange
//! compact digests of their known producers and only transfer the difference.

use bitvec::prelude::*;
use crypto::hash::hash;
use crypto::PublicKey;

/// A bloom filter for efficient producer set synchronization.
///
/// This probabilistic data structure provides:
/// - O(1) insertion and lookup
/// - No false negatives (if inserted, always found)
/// - Configurable false positive rate (~1% by default)
/// - Compact serialization for network transfer
///
/// # Example
///
/// ```rust
/// use doli_core::discovery::ProducerBloomFilter;
/// use crypto::KeyPair;
///
/// let mut bloom = ProducerBloomFilter::new(100);
/// let keypair = KeyPair::generate();
///
/// bloom.insert(keypair.public_key());
/// assert!(bloom.probably_contains(keypair.public_key()));
/// ```
#[derive(Debug, Clone)]
pub struct ProducerBloomFilter {
    /// The bit array storing the filter state.
    bits: BitVec<u8, Lsb0>,

    /// Number of hash functions to use.
    k: usize,

    /// Number of elements inserted.
    n: usize,

    /// Size of the bit array in bits.
    m: usize,
}

impl ProducerBloomFilter {
    /// Create a new bloom filter sized for the expected number of elements.
    ///
    /// The filter is configured for approximately 1% false positive rate.
    ///
    /// # Arguments
    ///
    /// * `expected_elements` - Expected number of elements to insert
    ///
    /// # Panics
    ///
    /// Panics if expected_elements is 0.
    #[must_use]
    pub fn new(expected_elements: usize) -> Self {
        assert!(expected_elements > 0, "expected_elements must be > 0");

        // Calculate optimal size for 1% false positive rate
        // m = -n * ln(p) / (ln(2)^2)
        // where p = 0.01 (1% false positive rate)
        let ln_p = (0.01_f64).ln(); // ln(0.01) ≈ -4.605
        let ln_2_squared = std::f64::consts::LN_2.powi(2); // ln(2)^2 ≈ 0.480
        let m = (-(expected_elements as f64) * ln_p / ln_2_squared).ceil() as usize;

        // Calculate optimal number of hash functions
        // k = (m/n) * ln(2)
        let k = ((m as f64 / expected_elements as f64) * std::f64::consts::LN_2).ceil() as usize;

        // Ensure minimum sizes
        let m = m.max(64);
        let k = k.clamp(1, 16); // Cap at 16 hash functions

        Self {
            bits: bitvec![u8, Lsb0; 0; m],
            k,
            n: 0,
            m,
        }
    }

    /// Create a bloom filter with specific parameters.
    ///
    /// This is useful for creating a filter with known parameters
    /// (e.g., when deserializing).
    #[must_use]
    pub fn with_params(size_bits: usize, hash_count: usize) -> Self {
        Self {
            bits: bitvec![u8, Lsb0; 0; size_bits],
            k: hash_count,
            n: 0,
            m: size_bits,
        }
    }

    /// Insert a public key into the filter.
    ///
    /// # Arguments
    ///
    /// * `pubkey` - The public key to insert
    pub fn insert(&mut self, pubkey: &PublicKey) {
        for i in 0..self.k {
            let index = self.hash_index(pubkey, i);
            self.bits.set(index, true);
        }
        self.n += 1;
    }

    /// Check if a public key is probably in the filter.
    ///
    /// Returns `true` if the key might be in the filter (with ~1% false positive rate).
    /// Returns `false` if the key is definitely not in the filter.
    ///
    /// # Arguments
    ///
    /// * `pubkey` - The public key to check
    #[must_use]
    pub fn probably_contains(&self, pubkey: &PublicKey) -> bool {
        for i in 0..self.k {
            let index = self.hash_index(pubkey, i);
            if !self.bits[index] {
                return false;
            }
        }
        true
    }

    /// Serialize the bloom filter to bytes.
    ///
    /// The format is: [bits as raw bytes]
    /// The k and n parameters must be stored separately.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        self.bits.as_raw_slice().to_vec()
    }

    /// Create a bloom filter from serialized bytes.
    ///
    /// # Arguments
    ///
    /// * `bytes` - The serialized bit array
    /// * `k` - Number of hash functions
    /// * `n` - Number of elements that were inserted
    /// * `m` - Original size in bits (needed because bytes may have padding)
    #[must_use]
    pub fn from_bytes(bytes: &[u8], k: usize, n: usize, m: usize) -> Self {
        let bits = BitVec::<u8, Lsb0>::from_slice(bytes);
        Self { bits, k, n, m }
    }

    /// Get the number of hash functions used.
    #[must_use]
    pub fn hash_count(&self) -> usize {
        self.k
    }

    /// Get the number of elements inserted.
    #[must_use]
    pub fn element_count(&self) -> usize {
        self.n
    }

    /// Get the size of the bit array in bits.
    #[must_use]
    pub fn size_bits(&self) -> usize {
        self.m
    }

    /// Get the size of the bit array in bytes.
    #[must_use]
    pub fn size_bytes(&self) -> usize {
        self.m.div_ceil(8)
    }

    /// Calculate the theoretical false positive rate.
    #[must_use]
    pub fn false_positive_rate(&self) -> f64 {
        if self.n == 0 {
            return 0.0;
        }
        // FP rate = (1 - e^(-k*n/m))^k
        let exponent = -(self.k as f64 * self.n as f64 / self.m as f64);
        (1.0 - exponent.exp()).powi(self.k as i32)
    }

    /// Compute the bit index for a given public key and hash iteration.
    fn hash_index(&self, pubkey: &PublicKey, iteration: usize) -> usize {
        // Use double hashing: h(x, i) = (h1(x) + i * h2(x)) mod m
        // Where h1 and h2 are derived from BLAKE3 hash

        // Create input: pubkey || iteration
        let mut input = Vec::with_capacity(36);
        input.extend_from_slice(pubkey.as_bytes());
        input.extend_from_slice(&(iteration as u32).to_le_bytes());

        let hash_bytes = hash(&input);

        // Use first 8 bytes as the hash value
        let hash_value = u64::from_le_bytes(hash_bytes.as_bytes()[0..8].try_into().unwrap());

        (hash_value as usize) % self.m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::KeyPair;

    #[test]
    fn test_bloom_insert_and_query() {
        let mut bloom = ProducerBloomFilter::new(100);
        let keypair = KeyPair::generate();
        let pubkey = keypair.public_key();

        assert!(!bloom.probably_contains(pubkey));
        bloom.insert(pubkey);
        assert!(bloom.probably_contains(pubkey));
    }

    #[test]
    fn test_bloom_no_false_negatives() {
        let mut bloom = ProducerBloomFilter::new(1000);
        let keypairs: Vec<_> = (0..100).map(|_| KeyPair::generate()).collect();

        for kp in &keypairs {
            bloom.insert(kp.public_key());
        }

        for kp in &keypairs {
            assert!(
                bloom.probably_contains(kp.public_key()),
                "Bloom filter must not have false negatives"
            );
        }
    }

    #[test]
    fn test_bloom_false_positive_rate() {
        let mut bloom = ProducerBloomFilter::new(1000);
        let inserted: Vec<_> = (0..1000).map(|_| KeyPair::generate()).collect();

        for kp in &inserted {
            bloom.insert(kp.public_key());
        }

        // Test 10000 random keys not in the filter
        let mut false_positives = 0;
        for _ in 0..10000 {
            let kp = KeyPair::generate();
            if bloom.probably_contains(kp.public_key()) {
                false_positives += 1;
            }
        }

        // Should be around 1% (100 out of 10000), allow 3% margin
        assert!(
            false_positives < 300,
            "False positive rate {} is too high (expected ~1%, got {}%)",
            false_positives as f64 / 10000.0,
            false_positives as f64 / 100.0
        );
    }

    #[test]
    fn test_bloom_serialization_roundtrip() {
        let mut bloom = ProducerBloomFilter::new(100);
        let keypairs: Vec<_> = (0..50).map(|_| KeyPair::generate()).collect();

        for kp in &keypairs {
            bloom.insert(kp.public_key());
        }

        let bytes = bloom.to_bytes();
        let restored = ProducerBloomFilter::from_bytes(&bytes, bloom.k, bloom.n, bloom.m);

        for kp in &keypairs {
            assert!(restored.probably_contains(kp.public_key()));
        }
    }

    #[test]
    fn test_bloom_empty_filter() {
        let bloom = ProducerBloomFilter::new(100);
        let keypair = KeyPair::generate();

        assert!(!bloom.probably_contains(keypair.public_key()));
        assert_eq!(bloom.element_count(), 0);
    }

    #[test]
    fn test_bloom_parameters() {
        let bloom = ProducerBloomFilter::new(1000);

        // For 1000 elements at 1% FP rate:
        // m ≈ 9586 bits, k ≈ 7
        assert!(bloom.size_bits() >= 9000);
        assert!(bloom.hash_count() >= 6 && bloom.hash_count() <= 8);
    }

    #[test]
    fn test_bloom_with_params() {
        let bloom = ProducerBloomFilter::with_params(1024, 5);
        assert_eq!(bloom.size_bits(), 1024);
        assert_eq!(bloom.hash_count(), 5);
        assert_eq!(bloom.element_count(), 0);
    }

    #[test]
    fn test_bloom_multiple_inserts_same_key() {
        let mut bloom = ProducerBloomFilter::new(100);
        let keypair = KeyPair::generate();

        bloom.insert(keypair.public_key());
        bloom.insert(keypair.public_key());
        bloom.insert(keypair.public_key());

        assert!(bloom.probably_contains(keypair.public_key()));
        assert_eq!(bloom.element_count(), 3); // Counts all inserts
    }

    #[test]
    fn test_bloom_size_bytes() {
        let bloom = ProducerBloomFilter::new(100);
        let bytes = bloom.to_bytes();
        assert_eq!(bytes.len(), bloom.size_bytes());
    }

    #[test]
    fn test_bloom_theoretical_fp_rate() {
        let mut bloom = ProducerBloomFilter::new(1000);

        // Empty filter should have 0 FP rate
        assert_eq!(bloom.false_positive_rate(), 0.0);

        // Add elements
        for _ in 0..1000 {
            let kp = KeyPair::generate();
            bloom.insert(kp.public_key());
        }

        // Should be approximately 1%
        let fp_rate = bloom.false_positive_rate();
        assert!(
            fp_rate > 0.005 && fp_rate < 0.02,
            "Theoretical FP rate {} should be ~1%",
            fp_rate
        );
    }
}

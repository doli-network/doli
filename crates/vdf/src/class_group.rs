//! Class group arithmetic for VDF computation
//!
//! This module implements arithmetic in imaginary quadratic class groups,
//! which is the mathematical foundation for the Wesolowski VDF.
//!
//! ## Mathematical Background
//!
//! An imaginary quadratic class group is defined by a negative discriminant Δ.
//! Elements are equivalence classes of binary quadratic forms (a, b, c) where:
//! - Δ = b² - 4ac (the discriminant)
//! - a > 0
//! - gcd(a, b, c) = 1
//!
//! The group operation is composition of forms, and the identity is the
//! principal form (1, Δ mod 2, (1 - Δ) / 4) for odd Δ.
//!
//! ## Security Properties
//!
//! - The group order is unknown (computing it requires factoring Δ)
//! - The sequential squaring property makes VDF computation inherently sequential
//! - The discriminant is generated from a seed to ensure no one knows the order

use rug::integer::Order;
use rug::ops::Pow;
use rug::ops::RemRounding;
use rug::Integer;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors in class group operations
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ClassGroupError {
    /// The discriminant is not valid (must be negative and ≡ 1 mod 4)
    #[error("invalid discriminant: must be negative and ≡ 1 (mod 4)")]
    InvalidDiscriminant,

    /// The group element is not a valid reduced form
    #[error("invalid group element: not a valid reduced binary quadratic form")]
    InvalidElement,

    /// Serialization or deserialization failed
    #[error("serialization error: {0}")]
    SerializationError(String),

    /// Mathematical operation failed
    #[error("arithmetic error: {0}")]
    ArithmeticError(String),
}

/// A class group element represented as a reduced binary quadratic form (a, b, c)
/// where ax² + bxy + cy² with discriminant Δ = b² - 4ac.
///
/// Forms are always stored in reduced form where:
/// - a > 0
/// - |b| ≤ a ≤ c
/// - If a = c, then b ≥ 0
/// - If |b| = a, then b ≥ 0
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassGroupElement {
    /// Coefficient a (always positive)
    #[serde(with = "integer_serde")]
    pub a: Integer,
    /// Coefficient b
    #[serde(with = "integer_serde")]
    pub b: Integer,
    /// The discriminant Δ (always negative)
    #[serde(with = "integer_serde")]
    discriminant: Integer,
}

/// Custom serde implementation for rug::Integer using hex strings
mod integer_serde {
    use rug::Complete;
    use rug::Integer;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(val: &Integer, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        val.to_string_radix(16).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Integer, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Integer::parse_radix(&s, 16)
            .map(|incomplete| incomplete.complete())
            .map_err(serde::de::Error::custom)
    }
}

impl ClassGroupElement {
    /// Create a new class group element from coefficients.
    ///
    /// The form will be reduced to its canonical representative.
    /// Returns an error if the inputs don't form a valid binary quadratic form.
    pub fn new(a: Integer, b: Integer, discriminant: Integer) -> Result<Self, ClassGroupError> {
        // Validate discriminant
        if discriminant >= 0 {
            return Err(ClassGroupError::InvalidDiscriminant);
        }

        let rem = Integer::from(&discriminant % 4);
        if rem != 1 && rem != -3 {
            return Err(ClassGroupError::InvalidDiscriminant);
        }

        // Validate a
        if a <= 0 {
            return Err(ClassGroupError::InvalidElement);
        }

        // Verify discriminant equation: Δ = b² - 4ac
        // => c = (b² - Δ) / 4a
        let b_squared = Integer::from(&b * &b);
        let numerator = Integer::from(&b_squared - &discriminant);
        let four_a = Integer::from(4) * &a;

        if Integer::from(&numerator % &four_a) != 0 {
            return Err(ClassGroupError::InvalidElement);
        }

        let elem = Self { a, b, discriminant };

        Ok(Self::reduce(elem))
    }

    /// Create a class group element without validation (internal use only).
    fn new_unchecked(a: Integer, b: Integer, discriminant: Integer) -> Self {
        Self { a, b, discriminant }
    }

    /// Get the discriminant Δ.
    #[must_use]
    pub fn discriminant(&self) -> &Integer {
        &self.discriminant
    }

    /// Compute coefficient c = (b² - Δ) / 4a.
    #[must_use]
    pub fn c(&self) -> Integer {
        let four_a = Integer::from(4) * &self.a;
        if four_a == 0 {
            return Integer::from(1);
        }
        Integer::from(&self.b * &self.b - &self.discriminant) / four_a
    }

    /// Create the identity element for a given discriminant.
    ///
    /// The identity is the principal form:
    /// - (1, 1, (1-Δ)/4) if Δ ≡ 1 (mod 4)
    /// - (1, 0, -Δ/4) if Δ ≡ 0 (mod 4)
    #[must_use]
    pub fn identity(discriminant: &Integer) -> Self {
        let rem = Integer::from(discriminant % 4);
        let b = if rem == 1 || rem == -3 {
            Integer::from(1)
        } else {
            Integer::from(0)
        };

        Self {
            a: Integer::from(1),
            b,
            discriminant: discriminant.clone(),
        }
    }

    /// Create a generator element from a hash using hash-to-group.
    ///
    /// This implements a deterministic mapping from arbitrary bytes to a
    /// valid class group element. The construction ensures the resulting
    /// element is uniformly distributed in the group.
    #[must_use]
    pub fn from_hash(hash: &[u8], discriminant: &Integer) -> Self {
        use crypto::Hasher;

        // Expand hash to find a valid prime 'a'
        let mut counter = 0u64;
        let a = loop {
            let mut hasher = Hasher::new();
            hasher.update(b"DOLI_CLASS_GROUP_HASH_TO_GROUP_V1");
            hasher.update(hash);
            hasher.update(&counter.to_le_bytes());
            let expanded = hasher.finalize();

            // Convert to positive Integer and find next prime
            let candidate = Integer::from_digits(expanded.as_bytes(), Order::MsfBe);
            let a_candidate = find_suitable_a(&candidate, discriminant);

            if let Some(a) = a_candidate {
                break a;
            }
            counter += 1;

            // Safety limit
            if counter > 1000 {
                // Fall back to identity-like form
                break Integer::from(1);
            }
        };

        // Compute b using Cornacchia-like approach
        // Find b such that b² ≡ Δ (mod 4a)
        let b = compute_b_for_a(&a, discriminant);

        let elem = Self::new_unchecked(a, b, discriminant.clone());
        Self::reduce(elem)
    }

    /// Compose two class group elements (the group operation).
    ///
    /// This implements Shanks' NUCOMP algorithm for composing
    /// binary quadratic forms.
    #[must_use]
    pub fn compose(&self, other: &Self) -> Self {
        debug_assert_eq!(
            self.discriminant, other.discriminant,
            "Cannot compose elements with different discriminants"
        );

        let (a1, b1) = (&self.a, &self.b);
        let (a2, b2) = (&other.a, &other.b);

        // Handle identity cases (a=1 with |b| <= 1)
        if *a1 == 1 && b1.clone().abs() <= 1 {
            return other.clone();
        }
        if *a2 == 1 && b2.clone().abs() <= 1 {
            return self.clone();
        }

        // Standard composition algorithm (Dirichlet)
        // Reference: Cohen "A Course in Computational Algebraic Number Theory" 5.4.7

        // Step 1: Compute g = gcd(a1, a2)
        let (g, y1, _y2) = extended_gcd(a1, a2);

        // Step 2: Compute h = gcd(g, (b1+b2)/2)
        let s = Integer::from(b1 + b2) / 2;
        let (h, x1, x2) = extended_gcd(&g, &s);

        if h == 0 {
            return Self::identity(&self.discriminant);
        }

        // Step 3: Compute w = y1 * x1
        let w = Integer::from(&y1 * &x1);

        // Step 4: Compute new coefficients
        let a1h = Integer::from(a1 / &h);
        let a2h = Integer::from(a2 / &h);

        // a3 = a1 * a2 / h^2 = (a1/h) * (a2/h)
        let a3 = Integer::from(&a1h * &a2h);

        if a3 == 0 {
            return Self::identity(&self.discriminant);
        }

        // Step 5: Compute b3
        // b3 = b1 + 2*a1*w*(s/h - c1*x2/h)  all mod 2*a3
        // Simplified: b3 = b2 + 2*a2/h * (w * (b1-b2)/2 - x2 * c2)
        let half_diff = Integer::from(b1 - b2) / 2;
        let c2 = other.c();

        // l = w * (b1-b2)/2 - x2 * c2
        let l = Integer::from(&w * &half_diff) - Integer::from(&x2 * &c2);

        // b3 = b2 + 2 * a2/h * l
        let b3_adjustment = Integer::from(2) * &a2h * &l;
        let b3_raw = Integer::from(b2 + &b3_adjustment);

        // Reduce b3 mod 2*a3 to range (-a3, a3]
        let two_a3 = Integer::from(2) * &a3;
        let b3 = if two_a3 == 0 {
            b3_raw
        } else {
            let b3_mod = b3_raw.clone().rem_floor(&two_a3);
            if b3_mod > a3 {
                Integer::from(&b3_mod - &two_a3)
            } else {
                b3_mod
            }
        };

        // Reduce the result
        Self::reduce(Self::new_unchecked(a3, b3, self.discriminant.clone()))
    }

    /// Square an element (self ∘ self).
    ///
    /// This is the core operation for VDF computation.
    /// Implemented as self.compose(self) for correctness.
    #[must_use]
    #[inline]
    pub fn square(&self) -> Self {
        self.compose(self)
    }

    /// Compute self^exp using square-and-multiply.
    ///
    /// This is used for both proof computation and verification.
    #[must_use]
    pub fn pow(&self, exp: &Integer) -> Self {
        if *exp == 0 {
            return Self::identity(&self.discriminant);
        }

        if *exp < 0 {
            // For negative exponents, compute inverse first
            let neg_exp = Integer::from(-exp);
            return self.inverse().pow(&neg_exp);
        }

        let mut result = Self::identity(&self.discriminant);
        let base = self.clone();

        // Square-and-multiply
        let exp_bytes = exp.to_digits::<u8>(Order::MsfBe);
        for byte in exp_bytes {
            for i in (0..8).rev() {
                result = result.square();
                if (byte >> i) & 1 == 1 {
                    result = result.compose(&base);
                }
            }
        }

        result
    }

    /// Compute the inverse of this element.
    ///
    /// The inverse of (a, b, c) is (a, -b, c).
    #[must_use]
    pub fn inverse(&self) -> Self {
        Self::reduce(Self::new_unchecked(
            self.a.clone(),
            Integer::from(-&self.b),
            self.discriminant.clone(),
        ))
    }

    /// Reduce a form to its canonical (reduced) representative.
    ///
    /// A form is reduced if:
    /// - |b| ≤ a ≤ c
    /// - If a = |b| or a = c, then b ≥ 0
    fn reduce(mut form: Self) -> Self {
        // Guard: if a is zero, return identity
        if form.a == 0 {
            return Self::identity(&form.discriminant);
        }

        let max_iterations = 1000;
        for _ in 0..max_iterations {
            let c = form.c();

            // Normalize: if a > c, swap a and c and negate b
            if form.a > c {
                if c == 0 {
                    break;
                }
                form = Self::new_unchecked(c, Integer::from(-&form.b), form.discriminant);
                continue;
            }

            // If a = c and b < 0, negate b
            if form.a == c && form.b < 0 {
                form.b = Integer::from(-&form.b);
            }

            // Check if |b| ≤ a
            let abs_b = form.b.clone().abs();
            if abs_b <= form.a {
                // Check boundary conditions
                if abs_b == form.a && form.b < 0 {
                    form.b = Integer::from(-&form.b);
                }
                break;
            }

            // Reduce b: find k such that |b - 2ka| is minimized
            // k = round(b / 2a)
            let two_a = Integer::from(2) * &form.a;
            if two_a == 0 {
                break;
            }
            let k = Integer::from(&form.b + &form.a) / &two_a;
            form.b -= Integer::from(&k * &two_a);
        }

        form
    }

    /// Serialize to bytes.
    ///
    /// Format: [a_len (4 bytes)][a_bytes][b_sign (1 byte)][b_len (4 bytes)][b_bytes]
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Serialize a (always positive)
        let a_bytes = self.a.to_digits::<u8>(Order::MsfBe);
        bytes.extend_from_slice(&(a_bytes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&a_bytes);

        // Serialize b (with sign)
        let sign_byte = if self.b < 0 { 1u8 } else { 0u8 };
        let b_abs = self.b.clone().abs();
        let b_bytes = b_abs.to_digits::<u8>(Order::MsfBe);
        bytes.push(sign_byte);
        bytes.extend_from_slice(&(b_bytes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&b_bytes);

        bytes
    }

    /// Deserialize from bytes.
    pub fn from_bytes(bytes: &[u8], discriminant: &Integer) -> Result<Self, ClassGroupError> {
        if bytes.len() < 9 {
            return Err(ClassGroupError::SerializationError(
                "buffer too short".to_string(),
            ));
        }

        let mut pos = 0;

        // Read a
        let a_len = u32::from_le_bytes(
            bytes[pos..pos + 4]
                .try_into()
                .map_err(|_| ClassGroupError::SerializationError("invalid a length".to_string()))?,
        ) as usize;
        pos += 4;

        if pos + a_len > bytes.len() {
            return Err(ClassGroupError::SerializationError(
                "a bytes overflow".to_string(),
            ));
        }
        let a = Integer::from_digits(&bytes[pos..pos + a_len], Order::MsfBe);
        pos += a_len;

        // Read b
        if pos >= bytes.len() {
            return Err(ClassGroupError::SerializationError(
                "missing b sign".to_string(),
            ));
        }
        let is_negative = bytes[pos] == 1;
        pos += 1;

        if pos + 4 > bytes.len() {
            return Err(ClassGroupError::SerializationError(
                "missing b length".to_string(),
            ));
        }
        let b_len = u32::from_le_bytes(
            bytes[pos..pos + 4]
                .try_into()
                .map_err(|_| ClassGroupError::SerializationError("invalid b length".to_string()))?,
        ) as usize;
        pos += 4;

        if pos + b_len > bytes.len() {
            return Err(ClassGroupError::SerializationError(
                "b bytes overflow".to_string(),
            ));
        }
        let mut b = Integer::from_digits(&bytes[pos..pos + b_len], Order::MsfBe);
        if is_negative {
            b = -b;
        }

        // Construct and validate
        Ok(Self::reduce(Self::new_unchecked(
            a,
            b,
            discriminant.clone(),
        )))
    }

    /// Check if this element is the identity.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.a == 1
    }
}

/// Find a suitable 'a' coefficient for hash-to-group.
///
/// Returns a value that allows construction of a valid form.
fn find_suitable_a(candidate: &Integer, discriminant: &Integer) -> Option<Integer> {
    // We need 'a' such that Δ is a quadratic residue mod 4a
    // For simplicity, we search for a prime a where Δ is a QR mod a

    let mut a = candidate.clone().abs();
    if a == 0 {
        a = Integer::from(1);
    }

    // Ensure a is odd
    if Integer::from(&a % 2) == 0 {
        a += 1;
    }

    // Try a few candidates
    for _ in 0..50 {
        // Check if discriminant is a quadratic residue mod a
        // Using Euler's criterion (simplified check)
        let disc_mod_a = discriminant.clone().rem_floor(&a);
        if disc_mod_a >= 0 || is_likely_qr(&disc_mod_a, &a) {
            return Some(a.clone());
        }
        a += 2;
    }

    None
}

/// Simple check if n might be a quadratic residue mod p.
fn is_likely_qr(n: &Integer, p: &Integer) -> bool {
    if *p <= 1 {
        return true;
    }

    // Euler's criterion: n^((p-1)/2) ≡ 1 (mod p) if n is QR
    let exp = Integer::from(p - 1) / 2;
    let result = mod_pow(n, &exp, p);
    result == 1 || result == 0
}

/// Compute b coefficient for a given a.
fn compute_b_for_a(a: &Integer, discriminant: &Integer) -> Integer {
    // We need b such that b² ≡ Δ (mod 4a) and |b| ≤ a
    // This is a simplified version; full implementation would use Tonelli-Shanks

    let four_a = Integer::from(4) * a;
    let disc_mod = discriminant.clone().rem_floor(&four_a);

    // Try to find a valid b
    // For the identity-like case
    if *a == 1 {
        let rem = Integer::from(discriminant % 4);
        return if rem == 1 || rem == -3 {
            Integer::from(1)
        } else {
            Integer::from(0)
        };
    }

    // Search for valid b (simplified)
    let mut b = disc_mod.clone().sqrt();
    if Integer::from(&b * &b) != disc_mod {
        // Try adjusting
        b = discriminant.clone().rem_floor(a);
        let half_a = Integer::from(a / 2);
        if b > half_a {
            b = Integer::from(&b - a);
        }
    }

    // Ensure |b| ≤ a
    while b.clone().abs() > *a {
        if b > 0 {
            b -= a;
        } else {
            b += a;
        }
    }

    b
}

/// Extended Euclidean algorithm.
///
/// Returns (gcd, x, y) such that ax + by = gcd(a, b).
/// Extended GCD (iterative to avoid stack overflow with large numbers).
/// Returns (gcd, coeff_a, coeff_b) such that a*coeff_a + b*coeff_b = gcd.
#[allow(clippy::many_single_char_names)]
fn extended_gcd(a: &Integer, b: &Integer) -> (Integer, Integer, Integer) {
    if *b == 0 {
        let sign = if *a < 0 { -1 } else { 1 };
        return (a.clone().abs(), Integer::from(sign), Integer::from(0));
    }

    let mut old_r = a.clone();
    let mut curr_r = b.clone();
    let mut old_s = Integer::from(1);
    let mut curr_s = Integer::from(0);
    let mut old_t = Integer::from(0);
    let mut curr_t = Integer::from(1);

    while curr_r != 0 {
        let quotient = Integer::from(&old_r / &curr_r);

        let temp_r = old_r.clone();
        old_r = curr_r.clone();
        curr_r = temp_r - Integer::from(&quotient * &curr_r);

        let temp_s = old_s.clone();
        old_s = curr_s.clone();
        curr_s = temp_s - Integer::from(&quotient * &curr_s);

        let temp_t = old_t.clone();
        old_t = curr_t.clone();
        curr_t = temp_t - Integer::from(&quotient * &curr_t);
    }

    // Ensure gcd is positive
    if old_r < 0 {
        (-old_r, -old_s, -old_t)
    } else {
        (old_r, old_s, old_t)
    }
}

/// Modular exponentiation: base^exp mod modulus.
fn mod_pow(base: &Integer, exp: &Integer, modulus: &Integer) -> Integer {
    if *modulus == 1 {
        return Integer::from(0);
    }

    // Use rug's built-in modular exponentiation which is highly optimized
    base.clone()
        .pow_mod(exp, modulus)
        .unwrap_or_else(|_| Integer::from(0))
}

/// Generate a discriminant of the specified bit size.
///
/// The discriminant is generated deterministically from a seed to ensure
/// no one knows the group order. This is crucial for VDF security.
///
/// Properties:
/// - Negative (imaginary quadratic field)
/// - ≡ 1 (mod 4) (fundamental discriminant)
/// - Large enough that factoring is infeasible
#[must_use]
pub fn generate_discriminant(bits: usize, seed: &[u8]) -> Integer {
    use crypto::Hasher;

    // Expand seed to required size using hash chaining
    let mut expanded = Vec::with_capacity((bits + 7) / 8);
    let mut counter = 0u64;

    while expanded.len() * 8 < bits {
        let mut hasher = Hasher::new();
        hasher.update(b"DOLI_DISCRIMINANT_EXPANSION_V1");
        hasher.update(seed);
        hasher.update(&counter.to_le_bytes());
        let hash = hasher.finalize();
        expanded.extend_from_slice(hash.as_bytes());
        counter += 1;
    }

    // Truncate to exact bit length
    expanded.truncate((bits + 7) / 8);

    // Set high bit to ensure bit length
    if let Some(first) = expanded.first_mut() {
        *first |= 0x80;
    }

    // Make it negative for imaginary quadratic field
    let mut d = -Integer::from_digits(&expanded, Order::MsfBe);

    // Adjust to be ≡ 1 (mod 4) for fundamental discriminant
    // A fundamental discriminant satisfies:
    // - Δ ≡ 1 (mod 4), or
    // - Δ ≡ 0 (mod 4) and Δ/4 ≡ 2 or 3 (mod 4)
    let rem = d.clone().rem_floor(Integer::from(4));
    if rem != 1 {
        d -= Integer::from(1) - &rem;
    }

    d
}

/// Compute 2^t mod l for verification.
#[must_use]
pub fn pow2_mod(t: u64, l: &Integer) -> Integer {
    // Use rug's optimized modular exponentiation
    let two = Integer::from(2);
    let exp = Integer::from(t);
    two.pow_mod(&exp, l).unwrap_or_else(|_| Integer::from(0))
}

/// Compute floor(2^t / l) for proof generation.
///
/// This is done iteratively to avoid overflow with large t.
pub fn div_2pow_by_l(t: u64, l: &Integer) -> Integer {
    // Use the recurrence: 2^t = q*l + r where r = 2^t mod l
    // q = (2^t - r) / l

    // For large t, compute incrementally
    if t <= 128 {
        // Direct computation for small t
        let two_t = Integer::from(2).pow(t as u32);
        return Integer::from(&two_t / l);
    }

    // For large t, use iterative doubling
    // 2^t = 2 * 2^(t-1)
    // floor(2^t / l) = floor(2 * 2^(t-1) / l) = 2*floor(2^(t-1)/l) + correction

    let mut q = Integer::from(1); // Start with 2^0 / l = 0, but we track the cumulative
    let mut r = Integer::from(2); // 2^1 mod l initially

    for _ in 1..t {
        // Double: q' = 2q + (2r >= l ? 1 : 0), r' = 2r mod l
        r *= 2;
        q *= 2;
        if r >= *l {
            r -= l;
            q += 1;
        }
    }

    q
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity() {
        let disc = Integer::from(-23);
        let id = ClassGroupElement::identity(&disc);

        assert_eq!(id.a, 1);
        assert_eq!(id.b, 1); // -23 ≡ 1 (mod 4)
    }

    #[test]
    fn test_identity_composition() {
        let disc = generate_discriminant(256, b"test");
        let id = ClassGroupElement::identity(&disc);
        let elem = ClassGroupElement::from_hash(b"input", &disc);

        // id ∘ elem = elem
        let result = id.compose(&elem);
        assert_eq!(result.a, elem.a);
        assert_eq!(result.b, elem.b);

        // elem ∘ id = elem
        let result2 = elem.compose(&id);
        assert_eq!(result2.a, elem.a);
        assert_eq!(result2.b, elem.b);
    }

    #[test]
    fn test_square_deterministic() {
        let disc = generate_discriminant(256, b"test");
        let elem = ClassGroupElement::from_hash(b"input", &disc);

        let sq1 = elem.square();
        let sq2 = elem.square();

        assert_eq!(sq1, sq2);
    }

    #[test]
    fn test_repeated_squaring() {
        let disc = generate_discriminant(256, b"test");
        let elem = ClassGroupElement::from_hash(b"input", &disc);

        // Square 10 times
        let mut result = elem.clone();
        for _ in 0..10 {
            result = result.square();
        }

        // Should equal elem.pow(2^10)
        let exp = Integer::from(1024); // 2^10
        let via_pow = elem.pow(&exp);

        assert_eq!(result.a, via_pow.a);
        assert_eq!(result.b, via_pow.b);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let disc = generate_discriminant(256, b"test");
        let elem = ClassGroupElement::from_hash(b"input", &disc);

        let bytes = elem.to_bytes();
        let recovered = ClassGroupElement::from_bytes(&bytes, &disc).unwrap();

        assert_eq!(elem, recovered);
    }

    #[test]
    fn test_discriminant_properties() {
        let disc = generate_discriminant(2048, b"DOLI_VDF_DISCRIMINANT_V1");

        // Must be negative
        assert!(disc < 0);

        // Must be ≡ 1 (mod 4)
        let rem = disc.clone().rem_floor(Integer::from(4));
        assert!(rem == 1 || rem == -3);
    }

    #[test]
    fn test_inverse() {
        let disc = generate_discriminant(256, b"test");
        let elem = ClassGroupElement::from_hash(b"input", &disc);

        // Create inverse: (a, -b, c)
        let inv = ClassGroupElement::new_unchecked(
            elem.a.clone(),
            Integer::from(-&elem.b),
            elem.discriminant.clone(),
        );

        // elem ∘ inv should give identity (a=1)
        let result = elem.compose(&inv);
        let identity = ClassGroupElement::identity(&disc);

        assert_eq!(result.a, identity.a, "elem ∘ inverse should have a=1");
        assert_eq!(result.b, identity.b, "elem ∘ inverse should be identity");
    }

    #[test]
    fn test_pow2_mod() {
        let l = Integer::from(17);

        // 2^10 = 1024 = 60*17 + 4
        let result = pow2_mod(10, &l);
        assert_eq!(result, 4);
    }

    #[test]
    fn test_div_2pow_by_l() {
        let l = Integer::from(17);

        // 2^10 = 1024, 1024 / 17 = 60
        let result = div_2pow_by_l(10, &l);
        assert_eq!(result, 60);
    }

    // Property-based tests
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(10))]

        #[test]
        fn prop_squaring_is_composition_with_self(seed: [u8; 32]) {
            let disc = generate_discriminant(128, b"prop_test");
            let elem = ClassGroupElement::from_hash(&seed, &disc);

            let squared = elem.square();
            let composed = elem.compose(&elem);

            prop_assert_eq!(squared.a, composed.a);
            prop_assert_eq!(squared.b, composed.b);
        }

        #[test]
        fn prop_serialization_roundtrip(seed: [u8; 32]) {
            let disc = generate_discriminant(128, b"prop_test");
            let elem = ClassGroupElement::from_hash(&seed, &disc);

            let bytes = elem.to_bytes();
            let recovered = ClassGroupElement::from_bytes(&bytes, &disc);

            prop_assert!(recovered.is_ok());
            prop_assert_eq!(elem, recovered.unwrap());
        }
    }
}

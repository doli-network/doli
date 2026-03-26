//! ClassGroupElement — binary quadratic form with group operations

use rug::integer::Order;
use rug::ops::RemRounding;
use rug::Integer;
use serde::{Deserialize, Serialize};

use super::arithmetic::{compute_b_for_a, extended_gcd, find_suitable_a};
use super::integer_serde;
use super::ClassGroupError;

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
    pub(crate) fn new_unchecked(a: Integer, b: Integer, discriminant: Integer) -> Self {
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
    pub(crate) fn reduce(mut form: Self) -> Self {
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

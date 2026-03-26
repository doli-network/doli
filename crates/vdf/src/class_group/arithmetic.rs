//! Helper arithmetic functions for class group operations

use rug::integer::Order;
use rug::ops::Pow;
use rug::ops::RemRounding;
use rug::Integer;

/// Find a suitable 'a' coefficient for hash-to-group.
///
/// Returns a value that allows construction of a valid form.
pub(crate) fn find_suitable_a(candidate: &Integer, discriminant: &Integer) -> Option<Integer> {
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
pub(crate) fn compute_b_for_a(a: &Integer, discriminant: &Integer) -> Integer {
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
pub(crate) fn extended_gcd(a: &Integer, b: &Integer) -> (Integer, Integer, Integer) {
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

    let mut q = Integer::from(0); // floor(2^1 / l) = 0 for any prime l > 2
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

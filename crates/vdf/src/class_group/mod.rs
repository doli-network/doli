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

mod arithmetic;
mod element;

pub use arithmetic::{div_2pow_by_l, generate_discriminant, pow2_mod};
pub use element::ClassGroupElement;

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

/// Custom serde implementation for rug::Integer using hex strings
pub(crate) mod integer_serde {
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

#[cfg(test)]
mod tests;

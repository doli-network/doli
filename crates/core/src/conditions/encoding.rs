//! Condition encoding, decoding, and validation.

use crypto::Hash;

use super::{
    Condition, ConditionError, CONDITION_VERSION, MAX_CONDITION_DEPTH, MAX_CONDITION_OPS,
    MAX_MULTISIG_KEYS, MAX_THRESHOLD_CONDITIONS, TAG_AMOUNT_GUARD, TAG_AND, TAG_HASHLOCK,
    TAG_MULTISIG, TAG_OR, TAG_OUTPUT_TYPE_GUARD, TAG_RECIPIENT_GUARD, TAG_SIGNATURE, TAG_THRESHOLD,
    TAG_TIMELOCK, TAG_TIMELOCK_EXPIRY,
};

// =============================================================================
// ENCODING
// =============================================================================

impl Condition {
    /// Encode this condition into extra_data bytes (with version prefix).
    pub fn encode(&self) -> Result<Vec<u8>, ConditionError> {
        // Validate before encoding
        self.validate()?;

        let mut buf = Vec::with_capacity(64);
        buf.push(CONDITION_VERSION);
        self.encode_inner(&mut buf);

        if buf.len() > crate::transaction::MAX_EXTRA_DATA_SIZE {
            return Err(ConditionError::EncodingTooLarge { size: buf.len() });
        }
        Ok(buf)
    }

    /// Encode condition body (without version prefix) — recursive.
    fn encode_inner(&self, buf: &mut Vec<u8>) {
        match self {
            Condition::Signature(pkh) => {
                buf.push(TAG_SIGNATURE);
                buf.extend_from_slice(pkh.as_bytes());
            }
            Condition::Multisig { threshold, keys } => {
                buf.push(TAG_MULTISIG);
                buf.push(*threshold);
                buf.push(keys.len() as u8);
                for key in keys {
                    buf.extend_from_slice(key.as_bytes());
                }
            }
            Condition::Hashlock(expected) => {
                buf.push(TAG_HASHLOCK);
                buf.extend_from_slice(expected.as_bytes());
            }
            Condition::Timelock(height) => {
                buf.push(TAG_TIMELOCK);
                buf.extend_from_slice(&height.to_le_bytes());
            }
            Condition::TimelockExpiry(height) => {
                buf.push(TAG_TIMELOCK_EXPIRY);
                buf.extend_from_slice(&height.to_le_bytes());
            }
            Condition::And(a, b) => {
                buf.push(TAG_AND);
                a.encode_inner(buf);
                b.encode_inner(buf);
            }
            Condition::Or(a, b) => {
                buf.push(TAG_OR);
                a.encode_inner(buf);
                b.encode_inner(buf);
            }
            Condition::Threshold { n, conditions } => {
                buf.push(TAG_THRESHOLD);
                buf.push(*n);
                buf.push(conditions.len() as u8);
                for cond in conditions {
                    cond.encode_inner(buf);
                }
            }
            Condition::AmountGuard {
                min_amount,
                output_index,
            } => {
                buf.push(TAG_AMOUNT_GUARD);
                buf.extend_from_slice(&min_amount.to_le_bytes());
                buf.push(*output_index);
            }
            Condition::OutputTypeGuard {
                expected_type,
                output_index,
            } => {
                buf.push(TAG_OUTPUT_TYPE_GUARD);
                buf.push(*expected_type as u8);
                buf.push(*output_index);
            }
            Condition::RecipientGuard {
                expected_pubkey_hash,
                output_index,
            } => {
                buf.push(TAG_RECIPIENT_GUARD);
                buf.extend_from_slice(expected_pubkey_hash.as_bytes());
                buf.push(*output_index);
            }
        }
    }

    /// Decode a condition from extra_data bytes (with version prefix).
    pub fn decode(bytes: &[u8]) -> Result<Self, ConditionError> {
        if bytes.is_empty() {
            return Err(ConditionError::BufferTooShort);
        }

        let version = bytes[0];
        if version != CONDITION_VERSION {
            return Err(ConditionError::UnsupportedVersion { version });
        }

        let (cond, consumed) = Self::decode_inner(&bytes[1..], 0)?;

        let remaining = bytes.len() - 1 - consumed;
        if remaining > 0 {
            return Err(ConditionError::TrailingBytes { remaining });
        }

        // Validate the decoded condition
        cond.validate()?;

        Ok(cond)
    }

    /// Decode a condition from the start of a byte slice, allowing trailing data.
    /// Returns (condition, total_bytes_consumed) including the version prefix.
    /// Use this for output types that append metadata after the condition (NFT, FungibleAsset, BridgeHTLC).
    pub fn decode_prefix(bytes: &[u8]) -> Result<(Self, usize), ConditionError> {
        if bytes.is_empty() {
            return Err(ConditionError::BufferTooShort);
        }

        let version = bytes[0];
        if version != CONDITION_VERSION {
            return Err(ConditionError::UnsupportedVersion { version });
        }

        let (cond, consumed) = Self::decode_inner(&bytes[1..], 0)?;
        cond.validate()?;
        Ok((cond, 1 + consumed))
    }

    /// Decode condition body (without version prefix) — recursive.
    /// Returns (condition, bytes_consumed).
    fn decode_inner(bytes: &[u8], depth: usize) -> Result<(Self, usize), ConditionError> {
        if depth > MAX_CONDITION_DEPTH {
            return Err(ConditionError::TooDeep { depth });
        }
        if bytes.is_empty() {
            return Err(ConditionError::BufferTooShort);
        }

        let tag = bytes[0];
        let rest = &bytes[1..];

        match tag {
            TAG_SIGNATURE => {
                if rest.len() < 32 {
                    return Err(ConditionError::BufferTooShort);
                }
                let hash = Hash::from_bytes(rest[..32].try_into().unwrap());
                Ok((Condition::Signature(hash), 1 + 32))
            }

            TAG_MULTISIG => {
                if rest.len() < 2 {
                    return Err(ConditionError::BufferTooShort);
                }
                let threshold = rest[0];
                let count = rest[1];
                if threshold == 0 {
                    return Err(ConditionError::ZeroThreshold);
                }
                if threshold > count {
                    return Err(ConditionError::InvalidThreshold {
                        threshold,
                        keys: count,
                    });
                }
                if count as usize > MAX_MULTISIG_KEYS {
                    return Err(ConditionError::TooManyKeys { count });
                }
                let keys_start = 2;
                let keys_end = keys_start + (count as usize) * 32;
                if rest.len() < keys_end {
                    return Err(ConditionError::BufferTooShort);
                }
                let mut keys = Vec::with_capacity(count as usize);
                for i in 0..count as usize {
                    let offset = keys_start + i * 32;
                    let hash = Hash::from_bytes(rest[offset..offset + 32].try_into().unwrap());
                    keys.push(hash);
                }
                Ok((Condition::Multisig { threshold, keys }, 1 + keys_end))
            }

            TAG_HASHLOCK => {
                if rest.len() < 32 {
                    return Err(ConditionError::BufferTooShort);
                }
                let hash = Hash::from_bytes(rest[..32].try_into().unwrap());
                Ok((Condition::Hashlock(hash), 1 + 32))
            }

            TAG_TIMELOCK => {
                if rest.len() < 8 {
                    return Err(ConditionError::BufferTooShort);
                }
                let height = u64::from_le_bytes(rest[..8].try_into().unwrap());
                Ok((Condition::Timelock(height), 1 + 8))
            }

            TAG_TIMELOCK_EXPIRY => {
                if rest.len() < 8 {
                    return Err(ConditionError::BufferTooShort);
                }
                let height = u64::from_le_bytes(rest[..8].try_into().unwrap());
                Ok((Condition::TimelockExpiry(height), 1 + 8))
            }

            TAG_AND => {
                let (a, consumed_a) = Self::decode_inner(rest, depth + 1)?;
                let (b, consumed_b) = Self::decode_inner(&rest[consumed_a..], depth + 1)?;
                Ok((
                    Condition::And(Box::new(a), Box::new(b)),
                    1 + consumed_a + consumed_b,
                ))
            }

            TAG_OR => {
                let (a, consumed_a) = Self::decode_inner(rest, depth + 1)?;
                let (b, consumed_b) = Self::decode_inner(&rest[consumed_a..], depth + 1)?;
                Ok((
                    Condition::Or(Box::new(a), Box::new(b)),
                    1 + consumed_a + consumed_b,
                ))
            }

            TAG_THRESHOLD => {
                if rest.len() < 2 {
                    return Err(ConditionError::BufferTooShort);
                }
                let n = rest[0];
                let count = rest[1];
                if n == 0 {
                    return Err(ConditionError::ZeroThreshold);
                }
                if n > count {
                    return Err(ConditionError::ThresholdExceedsCount { n, count });
                }
                if count as usize > MAX_THRESHOLD_CONDITIONS {
                    return Err(ConditionError::TooManyConditions { count });
                }
                let mut conditions = Vec::with_capacity(count as usize);
                let mut offset = 2;
                for _ in 0..count {
                    let (cond, consumed) = Self::decode_inner(&rest[offset..], depth + 1)?;
                    offset += consumed;
                    conditions.push(cond);
                }
                Ok((Condition::Threshold { n, conditions }, 1 + offset))
            }

            TAG_AMOUNT_GUARD => {
                if rest.len() < 9 {
                    return Err(ConditionError::BufferTooShort);
                }
                let min_amount = u64::from_le_bytes(rest[..8].try_into().unwrap());
                let output_index = rest[8];
                Ok((
                    Condition::AmountGuard {
                        min_amount,
                        output_index,
                    },
                    1 + 9,
                ))
            }

            TAG_OUTPUT_TYPE_GUARD => {
                if rest.len() < 2 {
                    return Err(ConditionError::BufferTooShort);
                }
                let expected_type = crate::transaction::OutputType::from_u8(rest[0])
                    .ok_or_else(|| ConditionError::UnknownTag { tag: rest[0] })?;
                let output_index = rest[1];
                Ok((
                    Condition::OutputTypeGuard {
                        expected_type,
                        output_index,
                    },
                    1 + 2,
                ))
            }

            TAG_RECIPIENT_GUARD => {
                if rest.len() < 33 {
                    return Err(ConditionError::BufferTooShort);
                }
                let expected_pubkey_hash = Hash::from_bytes(rest[..32].try_into().unwrap());
                let output_index = rest[32];
                Ok((
                    Condition::RecipientGuard {
                        expected_pubkey_hash,
                        output_index,
                    },
                    1 + 33,
                ))
            }

            _ => Err(ConditionError::UnknownTag { tag }),
        }
    }

    /// Returns true if this condition tree contains any guard conditions
    /// (AmountGuard, OutputTypeGuard, RecipientGuard).
    pub fn contains_guard(&self) -> bool {
        match self {
            Condition::AmountGuard { .. }
            | Condition::OutputTypeGuard { .. }
            | Condition::RecipientGuard { .. } => true,
            Condition::And(a, b) | Condition::Or(a, b) => a.contains_guard() || b.contains_guard(),
            Condition::Threshold { conditions, .. } => {
                conditions.iter().any(|c| c.contains_guard())
            }
            _ => false,
        }
    }

    /// Count the number of cryptographic operations (sig verifications + hash checks).
    pub fn ops_count(&self) -> usize {
        match self {
            Condition::Signature(_) => 1,
            Condition::Multisig { keys, .. } => keys.len(),
            Condition::Hashlock(_) => 1,
            Condition::Timelock(_)
            | Condition::TimelockExpiry(_)
            | Condition::AmountGuard { .. }
            | Condition::OutputTypeGuard { .. }
            | Condition::RecipientGuard { .. } => 0,
            Condition::And(a, b) | Condition::Or(a, b) => a.ops_count() + b.ops_count(),
            Condition::Threshold { conditions, .. } => {
                conditions.iter().map(|c| c.ops_count()).sum()
            }
        }
    }

    /// Calculate the maximum depth of the condition tree.
    fn depth(&self) -> usize {
        match self {
            Condition::Signature(_)
            | Condition::Multisig { .. }
            | Condition::Hashlock(_)
            | Condition::Timelock(_)
            | Condition::TimelockExpiry(_)
            | Condition::AmountGuard { .. }
            | Condition::OutputTypeGuard { .. }
            | Condition::RecipientGuard { .. } => 0,
            Condition::And(a, b) | Condition::Or(a, b) => 1 + a.depth().max(b.depth()),
            Condition::Threshold { conditions, .. } => {
                1 + conditions.iter().map(|c| c.depth()).max().unwrap_or(0)
            }
        }
    }

    /// Validate the condition tree (ops count, depth, structural integrity).
    pub fn validate(&self) -> Result<(), ConditionError> {
        let ops = self.ops_count();
        if ops > MAX_CONDITION_OPS {
            return Err(ConditionError::TooManyOperations { ops });
        }
        let depth = self.depth();
        if depth > MAX_CONDITION_DEPTH {
            return Err(ConditionError::TooDeep { depth });
        }
        // Structural validation for specific condition types
        self.validate_structure()
    }

    /// Validate structural invariants recursively.
    fn validate_structure(&self) -> Result<(), ConditionError> {
        match self {
            Condition::Multisig { threshold, keys } => {
                if *threshold == 0 {
                    return Err(ConditionError::ZeroThreshold);
                }
                if *threshold > keys.len() as u8 {
                    return Err(ConditionError::InvalidThreshold {
                        threshold: *threshold,
                        keys: keys.len() as u8,
                    });
                }
                if keys.len() > MAX_MULTISIG_KEYS {
                    return Err(ConditionError::TooManyKeys {
                        count: keys.len() as u8,
                    });
                }
                Ok(())
            }
            Condition::Threshold { n, conditions } => {
                if *n == 0 {
                    return Err(ConditionError::ZeroThreshold);
                }
                if *n > conditions.len() as u8 {
                    return Err(ConditionError::ThresholdExceedsCount {
                        n: *n,
                        count: conditions.len() as u8,
                    });
                }
                if conditions.len() > MAX_THRESHOLD_CONDITIONS {
                    return Err(ConditionError::TooManyConditions {
                        count: conditions.len() as u8,
                    });
                }
                for cond in conditions {
                    cond.validate_structure()?;
                }
                Ok(())
            }
            Condition::And(a, b) | Condition::Or(a, b) => {
                a.validate_structure()?;
                b.validate_structure()
            }
            _ => Ok(()),
        }
    }
}

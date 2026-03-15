//! Witness data types and encoding for conditioned outputs.

use crypto::{PublicKey, Signature};

use super::ConditionError;

// =============================================================================
// WITNESS DATA
// =============================================================================

/// Witness data for spending a conditioned output.
///
/// The witness is provided by the spender to satisfy the output's conditions.
/// It contains signatures, preimages, and branch choices for composite conditions.
#[derive(Clone, Debug, Default)]
pub struct Witness {
    /// Signatures indexed by pubkey_hash position in the condition.
    /// For single-sig: one entry. For multisig: threshold entries.
    pub signatures: Vec<WitnessSignature>,
    /// Preimage for hashlock conditions (32 bytes).
    pub preimage: Option<[u8; 32]>,
    /// Branch selection for Or conditions (false=left, true=right).
    pub or_branches: Vec<bool>,
}

/// A signature paired with the signer's public key.
#[derive(Clone, Debug)]
pub struct WitnessSignature {
    pub pubkey: PublicKey,
    pub signature: Signature,
}

// =============================================================================
// WITNESS ENCODING
// =============================================================================

/// Witness encoding tags.
const WITNESS_VERSION: u8 = 1;
const WITNESS_TAG_SIGS: u8 = 0x01;
const WITNESS_TAG_PREIMAGE: u8 = 0x02;
const WITNESS_TAG_BRANCHES: u8 = 0x03;

/// Maximum witness size (signatures + preimage + overhead).
/// 7 sigs × (32B pubkey + 64B sig) + 32B preimage + overhead ≈ 750 bytes.
pub const MAX_WITNESS_SIZE: usize = 1024;

impl Witness {
    /// Encode witness data into bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(128);
        buf.push(WITNESS_VERSION);

        // Signatures section
        if !self.signatures.is_empty() {
            buf.push(WITNESS_TAG_SIGS);
            buf.push(self.signatures.len() as u8);
            for ws in &self.signatures {
                buf.extend_from_slice(ws.pubkey.as_bytes());
                buf.extend_from_slice(ws.signature.as_bytes());
            }
        }

        // Preimage section
        if let Some(preimage) = &self.preimage {
            buf.push(WITNESS_TAG_PREIMAGE);
            buf.extend_from_slice(preimage);
        }

        // Or-branch selections
        if !self.or_branches.is_empty() {
            buf.push(WITNESS_TAG_BRANCHES);
            buf.push(self.or_branches.len() as u8);
            for &branch in &self.or_branches {
                buf.push(if branch { 1 } else { 0 });
            }
        }

        buf
    }

    /// Decode witness data from bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, ConditionError> {
        if bytes.is_empty() {
            // Empty witness is valid (for Normal/Bond outputs)
            return Ok(Self::default());
        }

        if bytes[0] != WITNESS_VERSION {
            return Err(ConditionError::UnsupportedVersion { version: bytes[0] });
        }

        let mut witness = Witness::default();
        let mut pos = 1;

        while pos < bytes.len() {
            let tag = bytes[pos];
            pos += 1;

            match tag {
                WITNESS_TAG_SIGS => {
                    if pos >= bytes.len() {
                        return Err(ConditionError::BufferTooShort);
                    }
                    let count = bytes[pos] as usize;
                    pos += 1;
                    for _ in 0..count {
                        if pos + 32 + 64 > bytes.len() {
                            return Err(ConditionError::BufferTooShort);
                        }
                        let pubkey_bytes: [u8; 32] = bytes[pos..pos + 32].try_into().unwrap();
                        pos += 32;
                        let sig_bytes: [u8; 64] = bytes[pos..pos + 64].try_into().unwrap();
                        pos += 64;
                        let pubkey = PublicKey::try_from_slice(&pubkey_bytes)
                            .map_err(|_| ConditionError::InvalidPublicKey)?;
                        witness.signatures.push(WitnessSignature {
                            pubkey,
                            signature: Signature::from_bytes(sig_bytes),
                        });
                    }
                }

                WITNESS_TAG_PREIMAGE => {
                    if pos + 32 > bytes.len() {
                        return Err(ConditionError::BufferTooShort);
                    }
                    let mut preimage = [0u8; 32];
                    preimage.copy_from_slice(&bytes[pos..pos + 32]);
                    witness.preimage = Some(preimage);
                    pos += 32;
                }

                WITNESS_TAG_BRANCHES => {
                    if pos >= bytes.len() {
                        return Err(ConditionError::BufferTooShort);
                    }
                    let count = bytes[pos] as usize;
                    pos += 1;
                    if pos + count > bytes.len() {
                        return Err(ConditionError::BufferTooShort);
                    }
                    for i in 0..count {
                        witness.or_branches.push(bytes[pos + i] != 0);
                    }
                    pos += count;
                }

                _ => {
                    // Unknown tag — skip to end (forward compat)
                    break;
                }
            }
        }

        Ok(witness)
    }
}

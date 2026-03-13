//! Payment request / invoice system (Phase 2).
//!
//! Invoices encode payment details into a compact `doli:pay:<base64>` format
//! that can be shared between peers.

use doli_core::Amount;
use serde::{Deserialize, Serialize};

/// A payment invoice.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Invoice {
    /// Payment hash (hashlock).
    pub payment_hash: [u8; 32],
    /// Requested amount (0 = any amount).
    pub amount: Amount,
    /// Human-readable description.
    pub description: String,
    /// Payee's pubkey hash.
    pub payee_pubkey_hash: [u8; 32],
    /// Expiry timestamp (seconds since epoch).
    pub expiry_timestamp: u64,
    /// Creation timestamp.
    pub created_at: u64,
}

impl Invoice {
    /// Create a new invoice.
    pub fn new(
        payment_hash: [u8; 32],
        amount: Amount,
        description: &str,
        payee_pubkey_hash: [u8; 32],
        expiry_secs: u64,
    ) -> Self {
        let now = chrono::Utc::now().timestamp() as u64;
        Self {
            payment_hash,
            amount,
            description: description.to_string(),
            payee_pubkey_hash,
            expiry_timestamp: now + expiry_secs,
            created_at: now,
        }
    }

    /// Encode to `doli:pay:<base64>` format.
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).unwrap_or_default();
        let b64 = base64_encode(&json);
        format!("doli:pay:{}", b64)
    }

    /// Decode from `doli:pay:<base64>` format.
    pub fn decode(s: &str) -> Option<Self> {
        let data = s.strip_prefix("doli:pay:")?;
        let bytes = base64_decode(data)?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Check if the invoice has expired.
    pub fn is_expired(&self) -> bool {
        let now = chrono::Utc::now().timestamp() as u64;
        now > self.expiry_timestamp
    }
}

/// Simple base64 encode (no external dep, uses basic alphabet).
fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        result.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(ALPHABET[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

/// Simple base64 decode.
fn base64_decode(data: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            b'=' => Some(0),
            _ => None,
        }
    }

    let bytes = data.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }

    let mut result = Vec::new();
    for chunk in bytes.chunks(4) {
        let a = val(chunk[0])?;
        let b = val(chunk[1])?;
        let c = val(chunk[2])?;
        let d = val(chunk[3])?;
        let triple = (a << 18) | (b << 12) | (c << 6) | d;
        result.push(((triple >> 16) & 0xFF) as u8);
        if chunk[2] != b'=' {
            result.push(((triple >> 8) & 0xFF) as u8);
        }
        if chunk[3] != b'=' {
            result.push((triple & 0xFF) as u8);
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invoice_encode_decode_roundtrip() {
        let inv = Invoice::new([42u8; 32], 100_000, "test payment", [1u8; 32], 3600);
        let encoded = inv.encode();
        assert!(encoded.starts_with("doli:pay:"));

        let decoded = Invoice::decode(&encoded).unwrap();
        assert_eq!(decoded.payment_hash, inv.payment_hash);
        assert_eq!(decoded.amount, inv.amount);
        assert_eq!(decoded.description, inv.description);
    }

    #[test]
    fn invoice_expiry() {
        let inv = Invoice {
            payment_hash: [0u8; 32],
            amount: 0,
            description: String::new(),
            payee_pubkey_hash: [0u8; 32],
            expiry_timestamp: 0, // already expired
            created_at: 0,
        };
        assert!(inv.is_expired());
    }
}

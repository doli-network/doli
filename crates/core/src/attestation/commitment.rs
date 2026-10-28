//! Presence commitment — REQ-BLS-003 (INC-I-178 D6).

use crypto::Hash;

/// `BLAKE3(len_le(bitfield) ‖ bitfield ‖ len_le(aggregate) ‖ aggregate)`.
///
/// Both parts are length-prefixed so no byte can move across the split without
/// changing the commitment. Unconditional: empty/empty is a real hash, never the
/// `Hash::ZERO` sentinel the legacy decoders read as "no attestation data".
pub fn presence_commitment(bitfield: &[u8], aggregate: &[u8]) -> Hash {
    let mut preimage = Vec::with_capacity(8 + bitfield.len() + aggregate.len());
    preimage.extend_from_slice(&(bitfield.len() as u32).to_le_bytes());
    preimage.extend_from_slice(bitfield);
    preimage.extend_from_slice(&(aggregate.len() as u32).to_le_bytes());
    preimage.extend_from_slice(aggregate);
    crypto::hash::hash(&preimage)
}

/// The `presence_root` a block at `height` carries for its body: below `ah`, BLAKE3(bitfield).
pub fn presence_root_at(ah: u64, height: u64, bitfield: &[u8], aggregate: &[u8]) -> Hash {
    if height >= ah {
        presence_commitment(bitfield, aggregate)
    } else {
        crypto::hash::hash(bitfield)
    }
}

/// True when `presence_root` commits to the body at `height`. Below `ah` an empty
/// bitfield is not committed; at an unknown height either arm is accepted.
pub fn presence_root_matches(
    ah: u64,
    height: Option<u64>,
    presence_root: &Hash,
    bitfield: &[u8],
    aggregate: &[u8],
) -> bool {
    let post = || *presence_root == presence_commitment(bitfield, aggregate);
    let pre = || bitfield.is_empty() || *presence_root == crypto::hash::hash(bitfield);
    match height {
        Some(h) if h >= ah => post(),
        Some(_) => pre(),
        None => pre() || post(),
    }
}

impl crate::Block {
    /// The body matches every header commitment: merkle root and presence root at `height`.
    pub fn body_matches_header(&self, ah: u64, height: Option<u64>) -> bool {
        self.verify_merkle_root()
            && presence_root_matches(
                ah,
                height,
                &self.header.presence_root,
                &self.attestation_bitfield,
                &self.aggregate_bls_signature,
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_root_matches_per_height_arm() {
        let bf = [0b01u8];
        let agg = [7u8; 4];
        let post = presence_commitment(&bf, &agg);
        let pre = crypto::hash::hash(&bf);
        assert!(presence_root_matches(10, Some(10), &post, &bf, &agg));
        assert!(!presence_root_matches(10, Some(10), &pre, &bf, &agg));
        assert!(presence_root_matches(10, Some(9), &pre, &bf, &agg));
        assert!(!presence_root_matches(10, Some(9), &post, &bf, &agg));
        assert!(presence_root_matches(10, Some(9), &Hash::ZERO, &[], &agg));
        assert!(!presence_root_matches(10, Some(10), &Hash::ZERO, &[], &[]));
        assert!(presence_root_matches(10, None, &post, &bf, &agg));
        assert!(presence_root_matches(10, None, &pre, &bf, &agg));
        assert!(!presence_root_matches(10, None, &Hash::ZERO, &bf, &agg));
        assert_eq!(presence_root_at(10, 9, &bf, &agg), pre);
        assert_eq!(presence_root_at(10, 10, &bf, &agg), post);
    }
}

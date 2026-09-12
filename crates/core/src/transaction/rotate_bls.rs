//! `RotateBlsKey` payload codec and the producer authorisation preimage.
//!
//! Wire contract: `specs/bls-key-rotation-architecture.md`, "Preimage byte
//! layouts". A field-list change bumps the `-V1` tag to `-V2`.

/// Domain tag of the Ed25519 authorisation. Private: the published surface is
/// [`rotation_auth_preimage`], never the tag on its own.
const ROTATE_BLS_DOMAIN: &[u8] = b"DOLI-ROTATE-BLS-V1";

/// Canonical on-wire length of a `RotateBlsKey` `extra_data` payload.
pub const ROTATE_BLS_DATA_LEN: usize = 32 + 48 + 96 + 64;

const PRODUCER_END: usize = 32;
const NEW_KEY_END: usize = PRODUCER_END + 48;
const POP_END: usize = NEW_KEY_END + 96;

/// Bytes after the genesis hash in the authorisation preimage.
const FIXED_TAIL_LEN: usize = 48 + 32 + 4;

/// `extra_data` payload of a `RotateBlsKey` transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RotateBlsData {
    /// Ed25519 producer key that authorises the rotation and verifies
    /// [`Self::signature`].
    pub producer: [u8; 32],
    /// BLS public key to install.
    pub new_bls_pubkey: [u8; 48],
    /// Proof of possession of `new_bls_pubkey` under the rotation DST.
    pub bls_pop: [u8; 96],
    /// Producer signature over [`rotation_auth_digest`].
    pub signature: [u8; 64],
}

impl RotateBlsData {
    /// Serialize to `producer ‖ new_bls_pubkey ‖ bls_pop ‖ signature`.
    #[must_use]
    pub fn encode(&self) -> [u8; ROTATE_BLS_DATA_LEN] {
        let mut out = [0u8; ROTATE_BLS_DATA_LEN];
        out[..PRODUCER_END].copy_from_slice(&self.producer);
        out[PRODUCER_END..NEW_KEY_END].copy_from_slice(&self.new_bls_pubkey);
        out[NEW_KEY_END..POP_END].copy_from_slice(&self.bls_pop);
        out[POP_END..].copy_from_slice(&self.signature);
        out
    }

    /// Parse the canonical layout.
    ///
    /// Length-exact: any input whose length is not [`ROTATE_BLS_DATA_LEN`]
    /// yields `None`, so trailing bytes the signature does not cover are a
    /// rejection rather than slack.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != ROTATE_BLS_DATA_LEN {
            return None;
        }
        Some(Self {
            producer: bytes[..PRODUCER_END].try_into().ok()?,
            new_bls_pubkey: bytes[PRODUCER_END..NEW_KEY_END].try_into().ok()?,
            bls_pop: bytes[NEW_KEY_END..POP_END].try_into().ok()?,
            signature: bytes[POP_END..].try_into().ok()?,
        })
    }
}

/// Build the exact bytes the producer signs:
/// `"DOLI-ROTATE-BLS-V1" ‖ genesis_hash ‖ new_bls_pubkey ‖ prev_tx_hash ‖
/// output_index (u32 LE)`.
///
/// No length prefixes and no key-kind byte: the genesis hash is the only
/// variable-width field and every following field is fixed width.
#[must_use]
pub fn rotation_auth_preimage(
    genesis_hash: &[u8],
    new_bls_pubkey: &[u8; 48],
    prev_tx_hash: &[u8; 32],
    output_index: u32,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(ROTATE_BLS_DOMAIN.len() + genesis_hash.len() + FIXED_TAIL_LEN);
    buf.extend_from_slice(ROTATE_BLS_DOMAIN);
    buf.extend_from_slice(genesis_hash);
    buf.extend_from_slice(new_bls_pubkey);
    buf.extend_from_slice(prev_tx_hash);
    buf.extend_from_slice(&output_index.to_le_bytes());
    buf
}

/// BLAKE3-256 of [`rotation_auth_preimage`], with no second domain tag.
#[must_use]
pub fn rotation_auth_digest(
    genesis_hash: &[u8],
    new_bls_pubkey: &[u8; 48],
    prev_tx_hash: &[u8; 32],
    output_index: u32,
) -> [u8; 32] {
    let preimage = rotation_auth_preimage(genesis_hash, new_bls_pubkey, prev_tx_hash, output_index);
    *crypto::hash::hash(&preimage).as_bytes()
}

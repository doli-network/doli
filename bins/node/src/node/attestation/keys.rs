//! INC-I-178 M5 — the one layer that reads `doli-core` and `storage` together.

use crypto::{BlsPublicKey, PublicKey};
use doli_core::decode_attestation_bitfield_vec;
use storage::ProducerSet;

/// On-chain BLS keys of the SET bits, in universe order.
///
/// The walk is `decode_attestation_bitfield_vec`, the same LSB-first helper the
/// encoder pairs with, so index parity with the bitfield cannot drift.
/// `Err(pk)` names the first set bit whose producer is absent from the set, or
/// whose `bls_pubkey` is empty or not a valid 48-byte compressed key.
pub(crate) fn set_bit_bls_pubkeys(
    universe: &[PublicKey],
    bitfield: &[u8],
    producers: &ProducerSet,
) -> Result<Vec<BlsPublicKey>, PublicKey> {
    let indices = decode_attestation_bitfield_vec(bitfield, universe.len());
    let mut keys = Vec::with_capacity(indices.len());
    for idx in indices {
        let pk = universe[idx];
        let raw = producers
            .get_by_pubkey(&pk)
            .map(|p| p.bls_pubkey.as_slice())
            .unwrap_or_default();
        keys.push(BlsPublicKey::try_from_slice(raw).map_err(|_| pk)?);
    }
    Ok(keys)
}

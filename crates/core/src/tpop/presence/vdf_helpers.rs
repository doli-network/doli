//! VDF computation helpers for presence chains.
//!
//! Provides functions to compute new VDF links and create genesis links.

use crypto::PublicKey;

use crate::types::Slot;

use super::types::VdfLink;
use super::PRESENCE_VDF_ITERATIONS;

/// Compute the next VDF link in a presence chain.
///
/// This is called continuously by producers to maintain their presence chain.
pub fn compute_next_presence_vdf(
    prev_link: &VdfLink,
    current_slot: Slot,
    producer: &PublicKey,
) -> Result<VdfLink, &'static str> {
    // Compute input from previous output
    let input = VdfLink::compute_input(&prev_link.output.value, current_slot, producer);

    // Compute VDF (this takes ~55 seconds)
    let (output, proof) =
        vdf::compute(&input, PRESENCE_VDF_ITERATIONS).map_err(|_| "VDF computation failed")?;

    Ok(VdfLink::new(
        prev_link.sequence + 1,
        current_slot,
        &prev_link.output.value,
        producer,
        output,
        proof,
    ))
}

/// Create the genesis VDF link for a new producer.
pub fn create_genesis_vdf_link(slot: Slot, producer: &PublicKey) -> Result<VdfLink, &'static str> {
    let genesis_output = vec![0u8; 32];
    let input = VdfLink::compute_input(&genesis_output, slot, producer);

    let (output, proof) =
        vdf::compute(&input, PRESENCE_VDF_ITERATIONS).map_err(|_| "VDF computation failed")?;

    Ok(VdfLink::new(
        0,
        slot,
        &genesis_output,
        producer,
        output,
        proof,
    ))
}

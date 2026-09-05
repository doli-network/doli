//! INC-I-178 M6 R5 — the post-AH body-bitfield width contract (REQ-BLS-014).

use doli_core::network_params::NetworkParams;

/// The activation gate a width decision is judged against. Two impls because the
/// gate lives in shipped params, and every consensus site reads the `Node` mirror
/// `init.rs` copies out of them.
pub trait AttestationBlsGate {
    fn attestation_bls_activation_height(&self) -> u64;
}

impl AttestationBlsGate for NetworkParams {
    fn attestation_bls_activation_height(&self) -> u64 {
        self.inc_i_178_attestation_bls_activation_height
    }
}

impl AttestationBlsGate for u64 {
    fn attestation_bls_activation_height(&self) -> u64 {
        *self
    }
}

/// Post-AH `ceil(universe_len / 8)` is the ONLY accepted body width. Below the
/// gate every width today's guard tolerates stays accepted; this predicate is
/// consulted in addition to `validate_attestation_bitfield_vec`, never instead
/// of it — it sees lengths, not the padding and stray-bit content that guard checks.
pub fn bitfield_width_accepted_at(
    bitfield_len: usize,
    universe_len: usize,
    height: u64,
    gate: &impl AttestationBlsGate,
) -> bool {
    if height >= gate.attestation_bls_activation_height() {
        bitfield_len == universe_len.div_ceil(8)
    } else {
        // INC-I-178 M6 PRE-AH TOLERANT ARM
        true
    }
}

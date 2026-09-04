//! Attestation types for the DOLI finality gadget.
//!
//! Producers sign an attestation for each block they observe. Attendance is
//! tracked per minute and committed into the next block's attestation bitfield;
//! the BLS half of each attestation is held in a bounded, node-local pool keyed
//! by the attested parent hash.

pub mod bitfield;
pub mod message;
pub mod pool;
pub mod tracker;
pub mod universe;

pub use bitfield::{
    attestation_minute, attestation_minutes_per_epoch, attestation_qualification_threshold,
    decode_attestation_bitfield, decode_attestation_bitfield_vec, encode_attestation_bitfield,
    encode_attestation_bitfield_vec, validate_attestation_bitfield,
    validate_attestation_bitfield_vec, ATTESTATION_MINUTES_PER_EPOCH,
    ATTESTATION_QUALIFICATION_THRESHOLD, SLOTS_PER_ATTESTATION_MINUTE,
};
pub use message::{bls_attest_msg, Attestation, AttestationError};
pub use pool::ParentSignaturePool;
pub use tracker::MinuteAttestationTracker;
pub use universe::attestation_universe;

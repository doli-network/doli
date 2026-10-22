use crypto::{PublicKey, Signature};

use super::{SlashData, SlashingEvidence};
use crate::block::BlockHeader;

impl SlashData {
    /// Canonical DoubleProduction evidence: headers ordered by hash ascending,
    /// default reporter signature. One encoding, so one tx id, per header pair.
    pub fn double_production(producer: PublicKey, h1: BlockHeader, h2: BlockHeader) -> Self {
        let (block_header_1, block_header_2) = if h1.hash().as_bytes() <= h2.hash().as_bytes() {
            (h1, h2)
        } else {
            (h2, h1)
        };
        Self {
            producer_pubkey: producer,
            evidence: SlashingEvidence::DoubleProduction {
                block_header_1,
                block_header_2,
            },
            reporter_signature: Signature::default(),
        }
    }
}

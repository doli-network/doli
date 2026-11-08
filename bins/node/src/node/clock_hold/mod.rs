use doli_core::validation::ValidationError;
use doli_core::Block;

use super::Node;

#[cfg(test)]
mod tests;

/// True only for the typed future-time rule failure.
pub(crate) fn not_yet(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<ValidationError>(),
        Some(ValidationError::TimestampTooFuture(_))
    )
}

impl Node {
    /// Wall-clock seconds since the Unix epoch, plus the test clock offset (0 in production).
    pub fn wall_now_secs(&self) -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now.saturating_add_signed(self.clock_offset_secs)
    }

    /// Moves this node's wall clock by `secs` for tests.
    #[doc(hidden)]
    #[allow(dead_code)]
    pub fn set_clock_offset_for_test(&mut self, secs: i64) {
        self.clock_offset_secs = secs;
    }

    /// True when `err` is the future-time rule and the body is whole and within size at `height`.
    pub(crate) fn not_yet_block(&self, err: &anyhow::Error, block: &Block, height: u64) -> bool {
        not_yet(err)
            && block.body_matches_header(
                self.inc_i_178_attestation_bls_activation_height,
                Some(height),
            )
            && block.size() <= doli_core::consensus::max_block_size(height)
    }
}

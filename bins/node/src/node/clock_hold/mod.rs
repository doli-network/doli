use super::Node;

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
}

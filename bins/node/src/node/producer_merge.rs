//! Single-flight slot for the GSet announcement merge task (INC-I-237 R3).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Held by the one running merge task; frees the slot on drop.
pub(crate) struct MergeSlot(Arc<AtomicBool>);

impl MergeSlot {
    /// Take the slot, or `None` while another merge holds it.
    pub(crate) fn acquire(busy: &Arc<AtomicBool>) -> Option<Self> {
        busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(busy.clone()))
    }
}

impl Drop for MergeSlot {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    //! OUTPUT CONTRACT: fn MergeSlot::acquire(&Arc<AtomicBool>) -> Option<MergeSlot>
    //!   O1: none  O2: the shared busy flag  O3: Some / None  O4-O6: none
    //! PATHS: P1 free -> Some, flag set; P2 held -> None; P3 drop -> flag cleared
    //! INPUT PARTITIONS: flag {free, held}; slot {alive, dropped}
    //! MATRIX: O2 x P1-P3, O3 x P1-P2 — asserted below.
    use super::*;

    // REQ-GRV-007 — Decision: a failure means a producer-announcement flood starts a second concurrent GSet merge (unbounded tasks) or a finished merge never frees the slot so every later batch is dropped.
    #[test]
    fn req_grv_007_merge_slot_is_single_flight_and_freed_on_drop() {
        let busy = Arc::new(AtomicBool::new(false));
        let slot = MergeSlot::acquire(&busy).expect("free slot");
        assert!(busy.load(Ordering::Acquire), "acquire marks the slot busy");
        assert!(MergeSlot::acquire(&busy).is_none(), "second merge refused");
        drop(slot);
        assert!(!busy.load(Ordering::Acquire), "drop frees the slot");
        assert!(
            MergeSlot::acquire(&busy).is_some(),
            "freed slot re-acquired"
        );
    }
}

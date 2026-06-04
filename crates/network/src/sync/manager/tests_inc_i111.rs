//! INC-I-111 regression tests — gap=0 stale-tip blind spot in recovery.
//!
//! N9 (mainnet, ai5) stalled at h=359607 for 6 minutes on 2026-06-03
//! while all 20 connected peers reported the same height. The recovery
//! classifier saw `gap = network_tip_height - local_height = 0` and
//! returned `RecoveryAction::None` for every call (308 of them in 392s)
//! despite obvious stuck-ness.
//!
//! Three fixes close this class of failure:
//!  - `recovery.rs`: drop `gap > 0` requirement on `stale_and_behind`
//!  - `periodic.rs`: drop `gap > 0` guard on `report_stale_tip()`
//!  - `mod.rs`: expose `request_mass_status_refresh()` so the node layer
//!    can force an immediate peer-status fan-out when local apply has
//!    been silent >60s (without waiting for the 30s periodic refresh).
//!
//! These tests pin the consequences of those changes.
//!
//! ====================================================================
//! OUTPUT CONTRACT: fn SyncManager::request_mass_status_refresh(&mut self)
//!                  fn SyncManager::take_needs_mass_status_refresh(&mut self) -> bool
//!
//!   Outputs:
//!     O1: receiver state — `self.needs_mass_status_refresh: bool` (mutated)
//!     O2: return value of `take_needs_mass_status_refresh` — `bool`
//!     (no other observable side effects — pure flag flip)
//!
//!   Paths:
//!     Pa: fresh manager, no setter called
//!     Pb: `request_mass_status_refresh()` called (flag false → true)
//!     Pc: `take_needs_mass_status_refresh()` after Pb (flag true → false, returns true)
//!     Pd: `take_needs_mass_status_refresh()` again (flag false, returns false)
//!
//!   INPUT PARTITIONS (state of `self.needs_mass_status_refresh` BEFORE the op):
//!     P1: flag = false, op = take                     — exercises Pa
//!     P2: flag = false, op = request_mass_status_refresh — exercises Pb
//!     P3: flag = true,  op = take                     — exercises Pc
//!     P4: flag = false, op = take (after a previous take) — exercises Pd
//!                                                       (idempotency / single-shot)
//!
//!   MATRIX: 2 outputs × 4 partitions = 8 cells, all asserted in one linear trace:
//!     P1: O1 stays false (verified via O2)            ; O2 = false  (asserted)
//!     P2: O1 transitions false → true (verified at P3); (no return)
//!     P3: O1 transitions true → false (verified at P4); O2 = true   (asserted)
//!     P4: O1 stays false (verified via O2)            ; O2 = false  (asserted)
//! ====================================================================

use crypto::Hash;

use super::{SyncConfig, SyncManager};

/// INC-I-111: `request_mass_status_refresh` sets the flag; `take_*` is single-shot.
///
/// Linear trace covering all four input partitions for the
/// `needs_mass_status_refresh` flag setter / consumer pair.
#[test]
fn t_inc_i_111_request_mass_status_refresh_sets_and_consumes() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // P1: fresh manager — flag starts false. Asserts O2 = false.
    assert!(
        !manager.take_needs_mass_status_refresh(),
        "P1: flag must start false on fresh manager"
    );

    // P2: request flips flag (O1: false → true). Transition verified at P3.
    manager.request_mass_status_refresh();

    // P3: take returns true (O2) and resets flag (O1: true → false).
    assert!(
        manager.take_needs_mass_status_refresh(),
        "P3: flag must be true after request_mass_status_refresh"
    );

    // P4: take is single-shot — second take returns false (O2), flag stays
    // false (O1). Confirms P3 reset the flag.
    assert!(
        !manager.take_needs_mass_status_refresh(),
        "P4: flag must be consumed by take (single-shot semantics)"
    );
}

/// INC-I-111: multiple requests are idempotent — flag stays true until taken.
///
/// Demonstrates that repeated `request_mass_status_refresh()` calls do
/// NOT advance to a "doubly-requested" state. There is one flag, one
/// take. This pins the contract that the node layer can call
/// `request_mass_status_refresh()` in a hot loop without spamming N status
/// requests — only ONE refresh fires on the next periodic tick.
///
/// Exercises an additional input partition on top of the matrix above:
///   P5: flag = true, op = request_mass_status_refresh — no-op (stays true)
#[test]
fn t_inc_i_111_repeated_requests_are_idempotent() {
    let mut manager = SyncManager::new(SyncConfig::default(), Hash::ZERO);

    // P2 then P5: two requests in a row.
    manager.request_mass_status_refresh();
    manager.request_mass_status_refresh();
    manager.request_mass_status_refresh();

    // P3: a single take suffices to consume — returns true.
    assert!(
        manager.take_needs_mass_status_refresh(),
        "three requests then one take must return true (flag is single-bit)"
    );

    // P4: second take confirms a single take consumed all three requests.
    assert!(
        !manager.take_needs_mass_status_refresh(),
        "one take consumes all prior requests (no request counter)"
    );
}

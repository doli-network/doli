// INC-I-172 M1 security audit, AUDIT-P2-013 — an absent or zeroed `first_notified_at`
// must FAIL CLOSED.
// REQ-172-012 (Must).
//
// THE DEFECT this locks shut: F7(b) made `first_notified_at` the SOLE clock the install
// gate runs on (`service.rs::check_veto_status`), replacing the attacker-supplied
// `published_at`. But the field kept `#[serde(default)]`, and `pending_update.json` is
// unauthenticated node-local state written by the node itself. An absent field defaulted
// to `0` — the Unix epoch — so `veto_deadline(0)` is decades in the past,
// `current_timestamp() >= deadline` is true on the very next 60 s tick, and the update is
// APPROVED and installed with no veto window at all. Deleting one line from a JSON file
// became an install trigger.
//
// ============================================================================
// OUTPUT CONTRACT
// ============================================================================
// Functions under test:
//   `doli_node::updater::PendingUpdate::load(&Path) -> Option<PendingUpdate>`
//   `updater::veto_deadline(u64) -> u64`
//
// ENUMERATION OF OBSERVABLE OUTPUTS.
//   - return value     : O1 `load`'s Option discriminant; O2 the loaded value's
//                        `first_notified_at`; O3 `veto_deadline`'s u64.
//   - mutable params   : NONE.
//   - persistent store : the data dir is READ ONLY by `load`. `load` does not delete or
//                        rewrite the file — asserted, because a "repair" that rewrites
//                        the timestamp would be a silently-restarted veto window.
//   - side channel     : one `warn!` tracing record on the rejected file. DECLARED
//                        UNASSERTED — the decision is fully visible in O1.
//
// CODE PATHS:
//   P1: file with `first_notified_at` ABSENT       -> None  (no serde default)
//   P2: file with `first_notified_at` == 0         -> None  (explicit rejection)
//   P3: file with a real `first_notified_at`       -> Some, value preserved (control)
//   P4: `veto_deadline` at u64::MAX                -> saturates, does not wrap
//
// INPUT PARTITIONS: the three JSON shapes above are the partition. P3 is the control
// that keeps P1/P2 from passing because `load` returns `None` for everything.
// ============================================================================

use std::path::Path;

use doli_node::updater::PendingUpdate;

/// A `pending_update.json` body. `first_notified` is injected verbatim so the test can
/// write a file with the field ABSENT, which is not expressible through the struct.
fn pending_json(first_notified: Option<u64>) -> String {
    let clock = match first_notified {
        Some(v) => format!(r#""first_notified_at": {v},"#),
        None => String::new(),
    };
    format!(
        r#"{{
  "release": {{
    "version": "9.9.9",
    "binary_sha256": "{sha}",
    "binary_url_template": "https://example.invalid/doli-{{platform}}.tar.gz",
    "changelog": "",
    "published_at": 1,
    "signatures": []
  }},
  {clock}
  "approved": false
}}"#,
        sha = "00".repeat(32)
    )
}

fn write_pending(dir: &Path, body: &str) {
    std::fs::write(dir.join("pending_update.json"), body).expect("writing the fixture must work");
}

/// REQ-172-012 (Must). RED before the fix.
/// Acceptance: a `pending_update.json` with NO `first_notified_at` is not loaded at all.
/// With `#[serde(default)]` it loaded as `0`, which places the veto deadline at the Unix
/// epoch and installs the update on the next tick.
/// [P1 -> O1]
#[test]
fn an_absent_first_notified_at_is_not_loaded() {
    let dir = tempfile::tempdir().unwrap();
    write_pending(dir.path(), &pending_json(None));

    assert!(
        PendingUpdate::load(dir.path()).is_none(),
        "a pending_update.json without `first_notified_at` must not load. It used to \
         default to 0 — the Unix epoch — which makes the veto deadline decades old and \
         auto-installs the release on the next 60s tick (AUDIT-P2-013). Deleting one line \
         from an unauthenticated JSON file must not be an install trigger."
    );

    // And the rejection must not "repair" the file by stamping a fresh timestamp: that
    // would restart the veto window from a state the node cannot vouch for.
    assert!(
        dir.path().join("pending_update.json").exists(),
        "load() must not delete or rewrite the file it refused — it is read-only state"
    );
}

/// REQ-172-012 (Must). RED before the fix.
/// Acceptance: an explicit `first_notified_at: 0` is refused too. Removing the serde
/// default alone would leave the identical outcome reachable by writing the zero.
/// [P2 -> O1]
#[test]
fn a_zeroed_first_notified_at_is_not_loaded() {
    let dir = tempfile::tempdir().unwrap();
    write_pending(dir.path(), &pending_json(Some(0)));

    assert!(
        PendingUpdate::load(dir.path()).is_none(),
        "`first_notified_at: 0` is the Unix epoch, not a notification time. It must be \
         refused as loudly as an absent field — otherwise the fix is defeated by writing \
         the value the default used to supply."
    );
}

/// REQ-172-012 (Must). GREEN-lock.
/// Acceptance: an honest file still loads with its timestamp intact. Without this the two
/// tests above would pass on a `load` that always returns `None`.
/// [P3 -> O1, O2]
#[test]
fn an_honest_first_notified_at_still_loads_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let first_seen = 1_700_000_000u64;
    write_pending(dir.path(), &pending_json(Some(first_seen)));

    let loaded = PendingUpdate::load(dir.path())
        .expect("a pending_update.json with a real notification time must still load");
    assert_eq!(
        loaded.first_notified_at, first_seen,
        "the node-local clock must survive the disk round trip unchanged"
    );
}

/// REQ-172-012 (Must).
/// Acceptance: the deadline arithmetic saturates. `first_notified_at` is read from
/// unauthenticated JSON, so a value near `u64::MAX` is representable; a wrapping add
/// would produce a deadline in the PAST — the same fail-open outcome as the zeroed
/// field, reached from the opposite end of the range.
/// [P4 -> O3]
#[test]
fn the_veto_deadline_saturates_instead_of_wrapping() {
    let deadline = updater::veto_deadline(u64::MAX);
    assert_eq!(
        deadline,
        u64::MAX,
        "veto_deadline(u64::MAX) must saturate to u64::MAX ('never'), not wrap to a past \
         instant that ends the veto window immediately"
    );
    assert!(
        !updater::veto_period_ended(u64::MAX),
        "a saturated deadline must never read as 'the veto period has ended'"
    );
}

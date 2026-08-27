// INC-I-172 M1 — the veto window must not be derived from attacker-supplied data.
// REQ-172-012 (Must)  [spec F7(b) / FM-09; api-contract §6(b)]
//
// STATE: **RED**. This file compiles today; the RED is observable as test failures.
// Two of the five tests are BEHAVIOURAL (t01, t02) — `PendingUpdate` and its
// `days_remaining()/hours_remaining()` accessors are public through
// `doli_node::updater`, so the defect is reachable without touching private items.
// Three are STRUCTURAL (t03, t04, t05) because `UpdateService::check_veto_status`
// and `auto_apply` are private methods on a struct in a private module
// (`bins/node/src/updater/mod.rs` declares `mod service;`), so `include_str!` is the
// only available seam — the same convention as bins/cli/tests/logrotate_dropin_test.rs.
//
// THE DEFECT. `service.rs:315`:
//     let veto_deadline = p.release.published_at + self.config.veto_period_secs;
// `published_at` arrives inside the release metadata, is NOT covered by the signed
// message (`verification.rs:62` signs only "{version}:{binary_sha256}"), and
// `download.rs:206-209` defaults it to 0 when the field is absent or unparseable.
// A publisher who sets it to a past value — or simply omits it — makes
// `now >= deadline` true on the FIRST poll, so the veto period is over before any
// producer can vote. The same forged field also sets `enforcement_time`
// (`service.rs:356`), so it collapses the grace period too.
//
// ============================================================================
// OUTPUT CONTRACT
// ============================================================================
// Behavioural subject:
//   `doli_node::updater::PendingUpdate::days_remaining(&self) -> u64`
//   `doli_node::updater::PendingUpdate::hours_remaining(&self) -> u64`
//   (mod.rs:108-121; both call `updater::veto_deadline(&self.release)` =
//    `release.published_at + VETO_PERIOD`)
// Structural subject:
//   the SOURCE of `bins/node/src/updater/service.rs`, sliced to `check_veto_status`
//   and to `auto_apply`.
//
// ENUMERATION OF OBSERVABLE OUTPUTS (behavioural subject).
//   - mutable params      : NONE (`&self`, no args).
//   - receiver mutation   : NONE (`&self`, no interior mutability — plain u64/String
//                           fields).
//   - persistent store    : NONE. (`PendingUpdate::save/load` DO write
//                           `pending_update.json`; that store is exercised as an
//                           input+output pair by O3 below.)
//   - return value        : u64 remaining-time — the only value channel.
//   - process/global state: none.
//
//   O1: `days_remaining()`   — MUST be a function of node-local state only.
//   O2: `hours_remaining()`  — same; asserted separately because they are separate
//                              fns that could be fixed inconsistently.
//   O3: `pending_update.json` round-trip — `first_notified_at` survives save/load
//                              and is independent of `published_at`. Without this
//                              the fix has no durable field to key off across the
//                              restart that `service.rs:55` performs.
//   O4: `check_veto_status` source — does NOT read `published_at`; DOES read
//                              `first_notified_at`.
//   O5: `auto_apply` source — re-verifies against the current trust root before
//                              installing (api-contract §6(a), F7(a)).
//
// CODE PATHS:
//   P1: forged-PAST / absent `published_at`  (the `.unwrap_or(0)` case)  -> window collapses
//   P2: forged-FUTURE `published_at`                                     -> window inflates
//   P3: honest `published_at` == first_notified_at                       -> control
//   P4: `check_veto_status` body (source)
//   P5: `auto_apply` body (source)
//
// INPUT PARTITIONS: `published_at` relative to `first_notified_at` — BEFORE (P1),
//   AFTER (P2), EQUAL (P3). These three exhaust the orderings of the only two
//   timestamps in play, and the defect is exactly "the answer depends on which
//   ordering holds". A partition over `version`/`binary_sha256` cannot change the
//   arithmetic and would be provably blind.
//
// MATRIX (every cell asserted by the test named in it):
//
//  path | O1                  | O2                  | O3            | O4       | O5
//  -----|---------------------|---------------------|---------------|----------|------
//  P1   | == P3 (see note)[t01]| == P3 (note)  [t01]| n/a           | [t03]    | n/a
//  P2   | == P3 result  [t01] | == P3 result  [t01] | n/a           | [t03]    | n/a
//  P3   | control       [t01] | control       [t01] | preserved[t02]| n/a      | n/a
//  P4   | n/a                 | n/a                 | n/a           | [t03,t04]| n/a
//  P5   | n/a                 | n/a                 | n/a           | n/a      | [t05]
//
// NOTE on the P1 x O1/O2 cell — RESOLUTION LIMIT, stated rather than hidden.
//   `days_remaining()`/`hours_remaining()` divide by 86400 and 3600, while
//   `VETO_PERIOD` on this build is 300 SECONDS (constants.rs:13). An honest window
//   is therefore already 0 days / 0 hours, so the COLLAPSE direction (published_at
//   in the past, or the `.unwrap_or(0)` default) is below the resolution of these
//   two accessors: 0 == 0 today and 0 == 0 after the fix. That cell is asserted as
//   a NON-REGRESSION lock here, and its RED evidence lives at [t03], which forbids
//   `check_veto_status` — where the collapse is actually consumed, as
//   `now >= published_at + veto_period_secs` at service.rs:315-316 — from reading
//   the field at all. The INFLATE direction (P2) exceeds the day boundary and IS
//   observable, and it proves the same dependency: `days_remaining()` is a function
//   of `published_at`. One dependency, two directions; the accessors can only
//   resolve one of them.
// ============================================================================

use doli_node::updater::{PendingUpdate, Release, VoteTracker};

const SRC: &str = include_str!("../src/updater/service.rs");

const TEN_YEARS_SECS: u64 = 10 * 365 * 86_400;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_secs()
}

/// A pending update that this node first saw at `first_notified_at`, carrying a
/// release whose `published_at` is whatever the publisher claimed.
fn pending(published_at: u64, first_notified_at: u64) -> PendingUpdate {
    let version = "9.9.9".to_string();
    PendingUpdate {
        release: Release {
            version: version.clone(),
            binary_sha256: "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"
                .to_string(),
            binary_url_template: String::new(),
            changelog: String::new(),
            published_at,
            signatures: Vec::new(),
            target_networks: Vec::new(),
        },
        vote_tracker: VoteTracker::new(version),
        first_notified_at,
        approved: false,
        enforcement: None,
    }
}

// ---------------------------------------------------------------------------
// Behavioural
// ---------------------------------------------------------------------------

/// REQ-172-012 (Must). RED. BEHAVIOURAL.
/// Acceptance: the remaining veto time a node reports does not change when the
/// publisher changes `published_at`. Three releases that this node first saw at the
/// same instant must report the same remaining window, whether the publisher
/// claimed a date ten years in the past (or omitted it, which becomes 0), ten years
/// in the future, or the truth.
/// [P1,P2,P3 x O1,O2]
///
/// `published_at` is not covered by the signed message and is defaulted to 0 by
/// `download.rs:206-209`, so it is fully attacker-controlled. Keying the veto window
/// off it lets a publisher decide how long the community gets to object
/// (api-contract §6(b)).
#[test]
fn req_172_012_veto_window_is_independent_of_attacker_supplied_published_at() {
    let first_seen = now_secs();

    // P3 (control): the publisher told the truth.
    let honest = pending(first_seen, first_seen);
    let honest_days = honest.days_remaining();
    let honest_hours = honest.hours_remaining();

    // P1: absent / forged-past date. `download.rs` `.unwrap_or(0)` produces exactly
    // this when the release metadata has no parseable timestamp.
    //
    // NON-REGRESSION LOCK, not a RED cell — see the RESOLUTION LIMIT note in the
    // OUTPUT CONTRACT above. VETO_PERIOD is 300s, so an honest window is already
    // 0 days / 0 hours and the collapse direction is below the resolution of these
    // accessors. The RED evidence for the collapse is
    // `req_172_012_check_veto_status_does_not_read_published_at`, which covers the
    // place the collapse is consumed (`now >= deadline`, service.rs:315-316).
    let collapsed = pending(0, first_seen);
    assert_eq!(
        collapsed.days_remaining(),
        honest_days,
        "a release with published_at=0 reports {} days of veto window instead of {}. \
         `published_at` is unsigned and defaults to 0 (download.rs:206-209), so a \
         publisher who simply omits the field ends the veto period before any producer \
         can vote (FM-09). The window must be measured from the node-local \
         `first_notified_at` (api-contract §6(b)).",
        collapsed.days_remaining(),
        honest_days
    );
    assert_eq!(
        collapsed.hours_remaining(),
        honest_hours,
        "hours_remaining() is derived from the same poisoned field and must be fixed \
         with days_remaining(), not separately"
    );

    // P2: forged-future date — the mirror image. The publisher can also make the
    // node wait, which is a denial-of-update against a SECURITY release.
    let inflated = pending(first_seen + TEN_YEARS_SECS, first_seen);
    assert_eq!(
        inflated.days_remaining(),
        honest_days,
        "a release with a published_at ten years in the future reports {} days of veto \
         window instead of {}. The same unsigned field that can collapse the window can \
         also stall a security update indefinitely.",
        inflated.days_remaining(),
        honest_days
    );
    assert_eq!(
        inflated.hours_remaining(),
        honest_hours,
        "hours_remaining() must also be independent of published_at"
    );
}

/// REQ-172-012 (Must). GREEN-lock. BEHAVIOURAL.
/// Acceptance: `first_notified_at` — the node-local timestamp the fix must key off —
/// is durable across the save/load cycle that `service.rs:55` performs on restart,
/// and is not overwritten by anything in the release.
/// [P3 -> O3]
///
/// This pins the precondition for the fix. `UpdateService::new` restores the pending
/// update from `pending_update.json` on every start; if `first_notified_at` did not
/// survive that round trip, keying the deadline off it would reset the veto clock on
/// each restart and a restart loop would extend the window forever.
#[test]
fn req_172_012_first_notified_at_is_node_local_and_durable() {
    let dir = tempfile::tempdir().unwrap();
    let first_seen = now_secs();

    let p = pending(first_seen + TEN_YEARS_SECS, first_seen);
    p.save(dir.path()).expect("pending_update.json must save");

    let loaded = PendingUpdate::load(dir.path()).expect("pending_update.json must load back");

    assert_eq!(
        loaded.first_notified_at, first_seen,
        "first_notified_at must survive the disk round trip unchanged — it is the only \
         timestamp in PendingUpdate that the release publisher does not control"
    );
    assert_eq!(
        loaded.release.published_at,
        first_seen + TEN_YEARS_SECS,
        "published_at round-trips too (it is still displayed); the requirement is that \
         it stops GATING anything, not that it disappears"
    );
    assert_ne!(
        loaded.first_notified_at, loaded.release.published_at,
        "fixture sanity: the two fields must differ, or this test proves nothing"
    );
}

// ---------------------------------------------------------------------------
// Structural — the private decision points
// ---------------------------------------------------------------------------

/// Body of a method inside `impl UpdateService`: from its signature to the next
/// method or the end of the impl block. Brace counting is unusable — the bodies are
/// full of `format!`/`info!` strings containing `{}`.
fn method_body(sig: &str) -> &'static str {
    let start = SRC.find(sig).unwrap_or_else(|| {
        panic!(
            "method not found in service.rs: {sig:?}. If it was renamed or inlined, \
             re-anchor this test — REQ-172-012 still requires the veto deadline to be \
             keyed off node-local state."
        )
    });
    let rest = &SRC[start + sig.len()..];
    let end = [
        "\n    ///",
        "\n    async fn ",
        "\n    fn ",
        "\n    pub fn ",
        "\n    pub async fn ",
        "\n}",
    ]
    .iter()
    .filter_map(|m| rest.find(m))
    .min()
    .unwrap_or(rest.len());
    &rest[..end]
}

/// REQ-172-012 (Must). RED. STRUCTURAL.
/// Acceptance: the veto/enforcement decision no longer reads `published_at`.
/// [P4 -> O4]
#[test]
fn req_172_012_check_veto_status_does_not_read_published_at() {
    let body = method_body("async fn check_veto_status");
    assert!(
        !body.contains("published_at"),
        "check_veto_status still derives its deadline from `release.published_at`. \
         That field is attacker-supplied, is not covered by the signed message \
         (verification.rs:62 signs only \"{{version}}:{{binary_sha256}}\") and is \
         defaulted to 0 by download.rs:206-209, so a forged or omitted value collapses \
         the veto window to zero (FM-09). Key BOTH `veto_deadline` (service.rs:315) and \
         `enforcement_time` (service.rs:356) off `p.first_notified_at`. \
         Do NOT add published_at to the signed message — api-contract §2 forbids \
         changing the signed message format in this milestone.\n--- body ---\n{body}"
    );
}

/// REQ-172-012 (Must). RED. STRUCTURAL.
/// Acceptance: the decision is keyed off the node-local `first_notified_at`.
/// [P4 -> O4]
///
/// Asserted separately from the negative above so that deleting the timing logic
/// altogether cannot make this file pass.
#[test]
fn req_172_012_check_veto_status_uses_first_notified_at() {
    let body = method_body("async fn check_veto_status");
    assert!(
        body.contains("first_notified_at"),
        "check_veto_status must compute the veto deadline from the node-local \
         `first_notified_at` (set from `updater::current_timestamp()` at service.rs:237 \
         and durable across restarts). Removing the deadline entirely is not a fix — the \
         veto period must still end.\n--- body ---\n{body}"
    );
}

/// REQ-172-012 (Must). RED. STRUCTURAL.
/// Acceptance (spec F7(a), api-contract §6(a)): the last gate before install
/// re-verifies the pending release against the CURRENT trust root.
/// [P5 -> O5]
///
/// `service.rs:55` restores a pending update from disk with zero re-verification,
/// and `auto_apply` (:457) installs it. Between the check at `:222` and the install
/// there is a veto period plus an arbitrary number of restarts — so a maintainer key
/// revoked in the meantime still authorises the install. Revocation that cannot reach
/// an in-flight update is not revocation.
#[test]
fn req_172_012_auto_apply_reverifies_against_the_current_trust_root() {
    let body = method_body("async fn auto_apply");
    let reverifies = body.contains("TrustRoot")
        || body.contains("verify_release")
        || body.contains("maintainer_keys");
    assert!(
        reverifies,
        "auto_apply installs the pending release without re-checking its signatures \
         against the CURRENT trust root. The pending update may have been restored from \
         disk across a restart (service.rs:55) after its signers were revoked. \
         Re-verify immediately before install and DROP any pending update whose signers \
         are no longer trusted (api-contract §6(a)).\n--- body ---\n{body}"
    );
}

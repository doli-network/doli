// INC-I-215 M2 — auto-update is dead on every `doli service install` node.
//
// STATE: **RED**. The file does not compile today: `doli_node::updater::preflight`
// does not exist. The structural halves are independently red — neither
// `bins/node/src/updater/staged_apply.rs` nor `preflight.rs` exists, and
// `service.rs` mentions neither `target_dir_is_writable` nor `staged_ready_version`.
// One test (`auto_apply_still_contains_the_trust_root_reverification`) is a
// GREEN-LOCK: it passes today and must keep passing.
//
// Sources are read at RUNTIME, never `include_str!`, so a missing module is a
// behavioural assertion failure with an actionable message instead of a compile
// error that says nothing about the contract.
//
// ============================================================================
// OUTPUT CONTRACT
// ============================================================================
// OUTPUT CONTRACT: async fn UpdateService::auto_apply(&self, version, maintainer_keys_fn)
//   O1 = the install/stage DECISION plus every side effect it owns:
//        (a) in-memory `*self.pending`, (b) on-disk `{data_dir}/pending_update.json`,
//        (c) the process image via `restart_node()` (exec, never returns),
//        (d) outbound network fetch of the release, (e) the operator log line.
//   Paths:
//     O1.P1  `ready` marker already names this version -> one INFO, pending KEPT,
//            no fetch, no restart.
//     O1.P2  install target dir writable -> `auto_apply_from_github`, pending REMOVED,
//            `restart_node()`. Unchanged from today.
//     O1.P3  install target dir NOT writable -> stage, pending KEPT, no restart.
//     O1.P4  trust-root re-verification fails -> pending DROPPED, nothing installed.
//
// OUTPUT CONTRACT: async fn stage_for_privileged_install(data_dir, version, sha, sigs)
//                  -> Result<PathBuf>
//   O2 = the staged artifact set under `{data_dir}/updates/` + the single operator line.
//   Paths:
//     O2.P1  staging succeeds -> tarball + CHECKSUMS.txt + SIGNATURES.json + `ready`,
//            exactly one INFO naming version, staging dir and the privileged helper.
//     O2.P2  fetch/verify/write fails -> Err, no `ready` marker. NOT covered here:
//            the artifact writer and its failure modes are M1's contract
//            (`crates/updater` tests); M2 only pins the caller's decision.
//
// OUTPUT CONTRACT: fn preflight_verdict(target, target_writable, helper_present)
//                  -> Option<(tracing::Level, String)>
//                  fn helper_unit_watches(units_dir, ready_marker) -> bool
//   O3 = the once-per-process startup diagnostic. Pure: the verdict is the RETURN
//        value; the caller (`report_install_target`) is the only thing that logs.
//   Paths:
//     O3.P1  writable -> None (no line at all).
//     O3.P2  not writable + helper present -> Some((INFO, "will stage / helper installs")).
//     O3.P3  not writable + no helper -> Some((WARN, token + target + reason + fix)).
//     O3.P4  units dir missing / empty / non-matching -> helper_present == false, no panic.
//
// MATRIX — input partition x output x covering test  (S = structural, B = behavioural)
//  I1  approved release, target dir writable            O1.P2  S  auto_apply_probes_writability_before_installing
//  I2  pending restored across restart, root rotated    O1.P4  S  auto_apply_still_contains_the_trust_root_reverification
//  I3  approved release, target NOT writable, staged    O1.P3  S  auto_apply_does_not_clear_pending_on_the_staging_path
//  I4  `ready` marker already names this version        O1.P1  S  auto_apply_checks_the_ready_marker_before_fetch
//  I5  not writable, no helper unit on the host         O3.P3  B+S preflight_warns_once_naming_target_reason_and_fix_when_no_helper
//  I6  not writable, `.path` unit watches this marker   O3.P2  B  preflight_is_info_not_warn_when_a_helper_watches_this_staging_path
//  I7  writable, helper both present and absent         O3.P1  B  preflight_is_silent_when_the_target_is_writable
//  I8  units dir missing/empty/non-matching/wrong ext   O3.P4  B  helper_scan_tolerates_missing_or_unreadable_units_dir
//  I9  staging succeeds                                 O2.P1  S  staging_success_emits_exactly_one_info_line
//  I10 the milestone's own file set (Rule 19 bound)     O1-O3  S  service_rs_is_within_the_module_budget

use doli_node::updater::preflight::{helper_unit_watches, preflight_verdict};
use std::path::{Path, PathBuf};
use tracing::Level;

const SERVICE_RS: &str = "src/updater/service.rs";
const STAGED_APPLY_RS: &str = "src/updater/staged_apply.rs";
const PREFLIGHT_RS: &str = "src/updater/preflight.rs";

/// Tokens that prove `auto_apply` delegated the install decision to the staging
/// branch instead of walking straight into `auto_apply_from_github`.
const DELEGATION: &[&str] = &[
    "target_dir_is_writable",
    "stage_for_privileged_install",
    "already_staged",
    "staged_apply::",
];

/// Tokens that prove the `ready` marker was consulted.
const READY_CHECK: &[&str] = &["staged_ready_version", "already_staged", "READY_MARKER"];

/// Tokens whose presence on the staging arm would destroy the idempotency latch
/// at `service_checks.rs:107` or reboot into a binary that was never installed.
const FORBIDDEN_ON_STAGING_ARM: &[&str] = &[
    "PendingUpdate::remove",
    "restart_node",
    "*pending = None",
    "pending = None",
];

fn src_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Read a source file at RUNTIME. A missing file is a contract failure, not a
/// compile error — the panic message states which milestone owes the module.
fn read_src(rel: &str) -> String {
    let path = src_path(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "INC-I-215 M2 requires {} but it could not be read: {e}. \
             The staging branch of `auto_apply` belongs in `staged_apply.rs` and the \
             startup diagnostic in `preflight.rs`; neither may be inlined into \
             service.rs, which has {} lines of its 500-line budget already spent.",
            path.display(),
            read_src_lenient(SERVICE_RS).lines().count()
        )
    })
}

fn read_src_lenient(rel: &str) -> String {
    std::fs::read_to_string(src_path(rel)).unwrap_or_default()
}

/// Slice the body that follows `sig`, stopping at the first terminator in `ends`.
/// Same structural seam as `bins/node/tests/inc_i_172_service_timing_test.rs:246`:
/// `UpdateService`'s methods are private on a private module, so text is the only
/// available surface.
fn body_after(src: &str, sig: &str, ends: &[&str], file: &str) -> String {
    let start = src.find(sig).unwrap_or_else(|| {
        panic!(
            "`{sig}` was not found in {file}. If it was renamed or inlined, re-anchor \
             this test — INC-I-215 still requires the install decision to probe target \
             writability before it tries to write the binary."
        )
    });
    let rest = &src[start + sig.len()..];
    let end = ends
        .iter()
        .filter_map(|m| rest.find(m))
        .min()
        .unwrap_or(rest.len());
    rest[..end].to_string()
}

/// The body of an `impl UpdateService` method (indent 4).
fn method_body(sig: &str) -> String {
    body_after(&read_src(SERVICE_RS), sig, &["\n    }"], SERVICE_RS)
}

/// The body of a module-level `fn` (indent 0).
fn free_fn_body(src: &str, sig: &str, file: &str) -> String {
    body_after(src, sig, &["\n}"], file)
}

fn first_index(hay: &str, needles: &[&str]) -> Option<usize> {
    needles.iter().filter_map(|n| hay.find(n)).min()
}

fn line_count(rel: &str) -> usize {
    read_src(rel).lines().count()
}

// ============================================================================
// O1 — the auto_apply install/stage decision
// ============================================================================

/// REQ-215-003 (Must) — Decision: a failure here means the node still calls
/// `install_binary` on a read-only `/usr/bin`, so every sandboxed host keeps
/// burning a full download every 10 minutes and never upgrades.
/// RED. STRUCTURAL. [I1 -> O1.P2 / O1.P3]
/// Acceptance: `auto_apply` probes install-target writability, after the trust-root
/// re-verification and before `auto_apply_from_github`.
#[test]
fn auto_apply_probes_writability_before_installing() {
    let body = method_body("async fn auto_apply");

    let probe = first_index(&body, DELEGATION).unwrap_or_else(|| {
        panic!(
            "`auto_apply` never probes whether the install target is writable. It calls \
             `auto_apply_from_github` unconditionally, and `install_binary` writes \
             `target.with_extension(\"new\")` INTO the target's directory (apply.rs:181-196) \
             — on a `doli service install` host that directory is a read-only \
             `ProtectSystem=full` mount and the write returns EROFS. Insert the branch \
             from `staged_apply.rs` here: resolve `updater::current_binary_path()`, call \
             `updater::target_dir_is_writable(&target)`, and only take the existing \
             GitHub install path when it is true.\n--- body ---\n{body}"
        )
    });

    let install = body.find("auto_apply_from_github").unwrap_or_else(|| {
        panic!(
            "`auto_apply` no longer calls `auto_apply_from_github`. REQ-215-003 requires \
             the writable-target path to stay EXACTLY as it is today: install, remove \
             `pending_update.json`, `restart_node()`. Do not rewrite it — branch around \
             it.\n--- body ---\n{body}"
        )
    });
    assert!(
        probe < install,
        "the writability probe runs AFTER `auto_apply_from_github`, which is too late: \
         the download, the extraction and the failed `install_binary` have already \
         happened. Probe first, then choose the path.\n--- body ---\n{body}"
    );

    let verify = body
        .find("verify_release_with_trust_root")
        .unwrap_or_else(|| {
            panic!(
                "`verify_release_with_trust_root` vanished from `auto_apply` (INC-I-172 F7(a)). \
             See `auto_apply_still_contains_the_trust_root_reverification`.\n--- body ---\n{body}"
            )
        });
    assert!(
        verify < probe,
        "the staging branch was inserted BEFORE the trust-root re-verification. A release \
         whose signers were revoked while it sat in `pending_update.json` would then be \
         written into `{{data_dir}}/updates/` and handed to a ROOT helper. Re-verify \
         first, branch second.\n--- body ---\n{body}"
    );
}

/// INC-I-172 F7(a) / REQ-215-003 (Must) — Decision: a failure here means M2 moved
/// or deleted the last revocation checkpoint before install, re-opening the
/// INC-I-172 hole through a new root-privileged path.
/// GREEN-LOCK. STRUCTURAL. Must pass BEFORE and AFTER the change. [I2 -> O1.P4]
/// Acceptance: the last gate before any install or stage still re-verifies the
/// pending release against the CURRENT trust root.
#[test]
fn auto_apply_still_contains_the_trust_root_reverification() {
    let body = method_body("async fn auto_apply");
    assert!(
        body.contains("verify_release_with_trust_root("),
        "`verify_release_with_trust_root(` is no longer inside `auto_apply`'s body. \
         `inc_i_172_service_timing_test.rs:318` slices the same body and will go red too. \
         A pending update can be restored from disk across any number of restarts \
         (service.rs:64-79), so the root that authorised it may since have been rotated — \
         and INC-I-215 now feeds that same release to a ROOT helper. Keep the \
         re-verification textually inside `auto_apply`; if the branch moved to \
         `staged_apply.rs`, the check must move with it AND stay here.\n--- body ---\n{body}"
    );
}

/// REQ-215-004 (Must) — Decision: a failure here means a staged node clears its
/// pending latch, so `check_for_updates` re-detects the same release on the next
/// tick and re-downloads it forever — the exact 10-minute loop this incident is about.
/// RED. STRUCTURAL. [I3 -> O1.P3]
/// Acceptance: the staging success arm clears no pending state and never restarts.
#[test]
fn auto_apply_does_not_clear_pending_on_the_staging_path() {
    let body = method_body("async fn auto_apply");
    let start = first_index(&body, DELEGATION).unwrap_or_else(|| {
        panic!(
            "there is no staging branch in `auto_apply` yet, so its arm cannot be checked. \
             See `auto_apply_probes_writability_before_installing`.\n--- body ---\n{body}"
        )
    });
    let end = body[start..]
        .find("auto_apply_from_github")
        .map(|i| start + i)
        .unwrap_or(body.len());
    let arm = &body[start..end];

    for token in FORBIDDEN_ON_STAGING_ARM {
        assert!(
            !arm.contains(token),
            "the staging arm of `auto_apply` contains `{token}`. Staging is NOT an install: \
             the new binary is still sitting in `{{data_dir}}/updates/` waiting for the root \
             helper. Clearing `pending` destroys the idempotency latch at \
             `service_checks.rs:104-112` and the node re-downloads the same release every \
             check cycle; `restart_node()` would exec the OLD binary. Keep `pending` and \
             `pending_update.json`, return Ok, and let `UpdateService::new` drain the state \
             after the helper installs (service.rs:64-79).\n--- staging arm ---\n{arm}"
        );
    }

    let staged = read_src(STAGED_APPLY_RS);
    for token in FORBIDDEN_ON_STAGING_ARM {
        assert!(
            !staged.contains(token),
            "`staged_apply.rs` contains `{token}`. The staging module writes artifacts and \
             logs one line — it owns no pending-state lifecycle and no process lifecycle. \
             Move that responsibility back to `service.rs`."
        );
    }
}

/// REQ-215-005 (Must) — Decision: a failure here means an approved-then-restarted
/// node re-downloads and re-stages a release it has already staged, on every
/// `ActivateEnforcement` transition (service.rs:318-357).
/// RED. STRUCTURAL. [I4 -> O1.P1]
/// Acceptance: the `ready` marker for this exact version is read before any fetch.
#[test]
fn auto_apply_checks_the_ready_marker_before_fetch() {
    let body = method_body("async fn auto_apply");
    let staged = read_src(STAGED_APPLY_RS);

    let delegation = first_index(&body, DELEGATION).unwrap_or_else(|| {
        panic!(
            "`auto_apply` reaches neither the ready-marker check nor the writability probe. \
             See `auto_apply_probes_writability_before_installing`.\n--- body ---\n{body}"
        )
    });
    if let Some(install) = body.find("auto_apply_from_github") {
        assert!(
            delegation < install,
            "the staging branch is entered after `auto_apply_from_github`.\n--- body ---\n{body}"
        );
    }

    let ready = first_index(&staged, READY_CHECK).unwrap_or_else(|| {
        panic!(
            "`staged_apply.rs` never reads the `ready` marker. REQ-215-005: when \
             `staged_ready_version(&staging_dir(&data_dir)) == Some(version)` the node must \
             log ONE line and return Ok immediately — no GitHub call, no download, no \
             re-stage, and above all no clearing of `pending`. Without it the \
             `ActivateEnforcement` transition (service.rs:318-357) re-stages the same \
             version after every restart."
        )
    });

    let fetch = staged.find("fetch_verified_release").unwrap_or_else(|| {
        panic!(
            "`staged_apply.rs` does not call `updater::fetch_verified_release`. The staging \
             path must reuse the M1 fetch+verify prefix — REQ-215-017 forbids a second copy \
             of the L1-L4 chain."
        )
    });
    assert!(
        ready < fetch,
        "the `ready`-marker check happens AFTER `fetch_verified_release`, so an \
         already-staged node still pays a full GitHub round trip and a tarball download \
         before it discovers it has nothing to do. Check the marker first."
    );
}

// ============================================================================
// O2 — the staged artifact set and its single operator line
// ============================================================================

/// REQ-215-015 (Should) — Decision: a failure here means the operator either never
/// learns a manual step is owed, or learns it once per tick and stops reading the log.
/// RED. STRUCTURAL. [I9 -> O2.P1]
/// Acceptance: one INFO per staging event, naming version, staging dir and the helper.
#[test]
fn staging_success_emits_exactly_one_info_line() {
    let staged = read_src(STAGED_APPLY_RS);
    let body = free_fn_body(&staged, "fn stage_for_privileged_install", STAGED_APPLY_RS);

    let infos = body.matches("info!").count();
    assert_eq!(
        infos, 1,
        "`stage_for_privileged_install` emits {infos} `info!` lines; REQ-215-015 allows \
         exactly one. The idempotency line belongs on the short-circuit path in \
         `service.rs`, not here, and staging progress is not an operator \
         concern.\n--- body ---\n{body}"
    );

    let at = body.find("info!").expect("counted one info! above");
    let line_end = body[at..].find(';').map(|i| at + i).unwrap_or(body.len());
    let line = &body[at..line_end];

    assert!(
        line.contains("--from-staged"),
        "the staging INFO line does not name the next step. An operator reading it must be \
         able to act without reading the source: say that a privileged helper must finish \
         the install and spell the command `doli upgrade --from-staged`.\n--- line ---\n{line}"
    );
    assert!(
        line.contains("version"),
        "the staging INFO line does not name the version that was staged.\n--- line ---\n{line}"
    );
    assert!(
        line.contains("dir") || line.contains("path") || line.contains("staging"),
        "the staging INFO line does not name the staging directory the helper must read. \
         Multi-node hosts stage into different `{{data_dir}}/updates/` paths, so an \
         unqualified message is unactionable.\n--- line ---\n{line}"
    );
}

// ============================================================================
// O3 — the once-per-process startup diagnostic
// ============================================================================

/// REQ-215-014 (Must) — Decision: a failure here means a host whose auto-update is
/// structurally dead looks healthy in the log, which is how this incident survived
/// undetected across the whole external fleet.
/// RED. BEHAVIOURAL + STRUCTURAL. [I5 -> O3.P3]
/// Acceptance: one WARN naming the target, the reason, the fix and a greppable token,
/// emitted once per process start and never on a timer.
#[test]
fn preflight_warns_once_naming_target_reason_and_fix_when_no_helper() {
    let target = Path::new("/usr/bin/doli-node");
    let (level, msg) = preflight_verdict(target, false, false).unwrap_or_else(|| {
        panic!(
            "`preflight_verdict` returned None for a non-writable target with no helper \
             unit. That is the exact state of every `doli service install` host today and \
             it must be loud."
        )
    });

    assert_eq!(
        level,
        Level::WARN,
        "a node that cannot ever install an update is a WARN, not an INFO: nothing on the \
         host will fix it without operator action. Got {level} — message was: {msg}"
    );
    assert!(
        msg.contains("UPDATE_TARGET_NOT_WRITABLE"),
        "the WARN carries no fixed greppable token. Monitoring cannot alert on prose that \
         a later refactor will reword. Include the literal \
         `UPDATE_TARGET_NOT_WRITABLE`.\n--- message ---\n{msg}"
    );
    assert!(
        msg.contains("/usr/bin/doli-node"),
        "the WARN does not name the install target. On a multi-node host the operator must \
         know WHICH binary path is unwritable.\n--- message ---\n{msg}"
    );
    assert!(
        msg.contains("not writable"),
        "the WARN does not state the reason. `ProtectSystem=full` makes the failure look \
         like a permissions bug; say plainly that the target directory is not writable by \
         this process.\n--- message ---\n{msg}"
    );
    assert!(
        msg.contains("sudo doli service install"),
        "the WARN does not state the fix. A diagnostic that names a problem and not its \
         remedy costs the operator a support round trip: name \
         `sudo doli service install`.\n--- message ---\n{msg}"
    );

    // Structural half: once per process start, never on a timer.
    let run = method_body("pub async fn run(");
    let call = first_index(&run, &["preflight::", "report_install_target"]).unwrap_or_else(|| {
        panic!(
            "`UpdateService::run` never reports the install target. Call \
             `preflight::report_install_target(&self.data_dir)` once, right after the \
             `info!(\"Update service started ...\")` anchor (service.rs:132-135) and before \
             the `loop`.\n--- run body ---\n{run}"
        )
    });
    let timer = first_index(&run, &["loop {", ".tick()"]).unwrap_or_else(|| {
        panic!("`UpdateService::run` has no ticker loop any more; re-anchor this test.\n{run}")
    });
    assert!(
        call < timer,
        "the preflight report sits inside `run`'s ticker loop, so it repeats every check \
         interval forever. REQ-215-014 is ONE line per process start — a repeating WARN is \
         noise the operator learns to filter.\n--- run body ---\n{run}"
    );
}

/// REQ-215-014 (Must) — Decision: a failure here means correctly-configured hosts
/// get a permanent false alarm, which trains operators to ignore the real one.
/// RED. BEHAVIOURAL. [I6 -> O3.P2]
/// Acceptance: a `.path` unit watching THIS node's ready marker downgrades WARN to INFO.
#[test]
fn preflight_is_info_not_warn_when_a_helper_watches_this_staging_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let units = tmp.path().join("systemd");
    std::fs::create_dir_all(&units).expect("create units dir");
    let ready = tmp.path().join("data").join("updates").join("ready");
    std::fs::write(
        units.join("doli-installer.path"),
        format!(
            "[Unit]\nDescription=DOLI staged upgrade\n\n[Path]\nPathExists={}\n\n\
             [Install]\nWantedBy=paths.target\n",
            ready.display()
        ),
    )
    .expect("write unit");

    assert!(
        helper_unit_watches(&units, &ready),
        "`helper_unit_watches` missed a `.path` unit whose `PathExists=` names this node's \
         ready marker at {}. Detection must be name-independent: `--name` custom services \
         and multi-node hosts install units under arbitrary names, so match on the CONTENT \
         of every `*.path` file in the directory.",
        ready.display()
    );

    let target = Path::new("/usr/bin/doli-node");
    let (level, msg) = preflight_verdict(target, false, true)
        .expect("a non-writable target is still worth one INFO line: it explains the handoff");
    assert_eq!(
        level,
        Level::INFO,
        "a host with the helper installed is CORRECTLY configured — staging is the designed \
         path there, not a fault. Warning about it makes the real WARN unreadable. Got \
         {level} — message was: {msg}"
    );
    assert!(
        !msg.contains("UPDATE_TARGET_NOT_WRITABLE"),
        "the INFO carries the alert token, so monitoring will page on healthy hosts. The \
         token belongs to the no-helper WARN only.\n--- message ---\n{msg}"
    );
    assert!(
        msg.contains("stage") && msg.contains("helper"),
        "the INFO does not explain the handoff. Say that the node will stage the update and \
         the helper unit will install it.\n--- message ---\n{msg}"
    );
}

/// REQ-215-014 (Must) — Decision: a failure here means every ordinary root-installed
/// node gains a startup line about a problem it does not have.
/// RED. BEHAVIOURAL. [I7 -> O3.P1]
/// Acceptance: a writable target produces no diagnostic at all, helper or not.
#[test]
fn preflight_is_silent_when_the_target_is_writable() {
    let target = Path::new("/usr/bin/doli-node");
    for helper_present in [false, true] {
        let verdict = preflight_verdict(target, true, helper_present);
        assert!(
            verdict.is_none(),
            "`preflight_verdict` produced {verdict:?} for a WRITABLE target \
             (helper_present={helper_present}). Nothing is wrong on that host and nothing \
             is owed by the operator: return None so the startup log stays quiet."
        );
    }
}

/// REQ-215-014 (Must) — Decision: a failure here means the update service panics or
/// mis-reports on hosts without systemd, inside containers, or when `/etc` is
/// unreadable — turning a diagnostic into an outage.
/// RED. BEHAVIOURAL. [I8 -> O3.P4]
/// Acceptance: any scan that cannot prove a helper watches this marker returns false.
#[test]
fn helper_scan_tolerates_missing_or_unreadable_units_dir() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ready = tmp.path().join("data").join("updates").join("ready");

    let missing = tmp.path().join("no-such-dir");
    assert!(
        !helper_unit_watches(&missing, &ready),
        "a nonexistent units directory must degrade to \"no helper\", never panic. \
         `/etc/systemd/system` is absent on launchd hosts and in minimal containers."
    );

    let empty = tmp.path().join("empty");
    std::fs::create_dir_all(&empty).expect("create empty dir");
    assert!(
        !helper_unit_watches(&empty, &ready),
        "an empty units directory reported a helper. Absence of evidence is not a helper."
    );

    let other = tmp.path().join("other");
    std::fs::create_dir_all(&other).expect("create other dir");
    std::fs::write(
        other.join("doli-installer.path"),
        "[Path]\nPathExists=/srv/other-node/updates/ready\n",
    )
    .expect("write foreign unit");
    assert!(
        !helper_unit_watches(&other, &ready),
        "a `.path` unit watching a DIFFERENT node's staging directory was accepted. On a \
         multi-node host that silences the WARN for the node that actually has no helper."
    );

    let wrong_ext = tmp.path().join("wrong-ext");
    std::fs::create_dir_all(&wrong_ext).expect("create wrong-ext dir");
    std::fs::write(
        wrong_ext.join("doli-installer.service"),
        format!(
            "[Service]\nExecStart=/usr/bin/doli upgrade --from-staged {}\n",
            ready.display()
        ),
    )
    .expect("write service unit");
    assert!(
        !helper_unit_watches(&wrong_ext, &ready),
        "a `.service` file was treated as a watcher. Only a `.path` unit makes systemd react \
         to the marker appearing; a `.service` alone is never triggered and the node would \
         stage into silence."
    );
}

// ============================================================================
// Module budget (Rule 19)
// ============================================================================

/// REQ-215-016 (Must) — Decision: a failure here means the fix landed as more lines
/// inside the file that is already the hardest to reason about in the update path.
/// RED. STRUCTURAL. [I10 -> O1/O2/O3 maintainability bound]
/// Acceptance: service.rs stays <= 500 lines; each new module is <= 500 lines.
#[test]
fn service_rs_is_within_the_module_budget() {
    for rel in [SERVICE_RS, STAGED_APPLY_RS, PREFLIGHT_RS] {
        let lines = line_count(rel);
        assert!(
            lines <= 500,
            "{rel} is {lines} lines, over the 500-line module budget (Rule 19). \
             `service.rs` was 478 before this milestone and may take at most ~15 new lines: \
             the staging branch belongs in `staged_apply.rs` and the diagnostic in \
             `preflight.rs`."
        );
    }
}

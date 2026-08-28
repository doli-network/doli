// INC-I-188 M1 — `sudo doli upgrade` finished while the node's systemd unit stayed DOWN.
// The unit was in systemd start-limit lock (4x status=203/EXEC during the binary swap
// window). Plain `systemctl restart` is REFUSED in that state; only
// `systemctl reset-failed <unit>` clears it. Neither restart call site in
// `bins/cli/src/upgrade_restart.rs` ever issues reset-failed, so the restart path cannot
// recover from the very failure mode the upgrade itself can create.
//
// covers: upgrade_restart, upgrade_systemd_plan, lib
//
// ============================================================================
// OUTPUT CONTRACT: fn systemd_restart_plan(unit: &str) -> Vec<SystemdStep>, plus a
// structural check of bins/cli/src/upgrade_restart.rs
// ============================================================================
// TWO subjects, deliberately (mirrors bins/cli/tests/inc_i_172_cli_trust_root_resolution_test.rs):
//
//   (A) BEHAVIOURAL — `upgrade_systemd_plan::systemd_restart_plan(unit)`, the pure command
//       plan both call sites must consume. systemd is unavailable on this build host
//       (macOS), so the command SEQUENCE has to be assertable as DATA, not as a live
//       process exit status. This is where the real assertions live.
//
//   (B) STRUCTURAL — the SOURCE of `bins/cli/src/upgrade_restart.rs`, via
//       `std::fs::read_to_string` (the crate has a lib target, but `restart_specific_service`
//       and `try_restart_systemd` are `pub(crate)`/private and cfg-gated to Linux, so they
//       are not directly callable from an integration test on macOS). It asserts only the
//       WIRING: that neither call site still hardcodes a raw `["systemctl", "restart", ...]`
//       argv literal, and that the plan function is the route both of them take.
//
// ENUMERATION OF OBSERVABLE OUTPUTS.
//   (A) `systemd_restart_plan` — return value only (O3). No mutable params, no receiver,
//       no store, no side channel: it is a pure `&str -> Vec<SystemdStep>` mapping.
//       O3 has four facets per call: plan length, step[0].args, step[0].required,
//       step[1].args, step[1].required.
//   (B) the source text itself — O_B1 (no raw restart-literal argv), O_B2 (>=2 call
//       sites route through `systemd_restart_plan(`).
//
// CODE PATHS of (A): `systemd_restart_plan` has no branches — it always returns the same
// two-step shape for any unit string. The property under test is NOT which path executes
// (there is one) but whether the OUTPUT is a hardcoded constant in disguise (a fix that
// special-cases one unit name, or forgets to thread the unit into both steps, would still
// "return two steps" and pass a weaker test).
//
// PATHS: P1: build the two-step plan (no branches — see below for why partitions still apply).
// INPUT PARTITIONS:
//   P1a: a realistic multi-node unit name ("doli-mainnet-n3.service") — baseline.
//   P1b: a DIFFERENT realistic unit name ("doli-testnet-seed.service") — same test shape,
//        different string. Catches a hardcoded literal that only happens to look right for
//        P1a's fixture.
//   P1c: an adversarial unit string containing shell metacharacters — proves the plan
//        threads the string byte-for-byte with no shell interpretation or reformatting
//        (worst-scenario #4: strings with special characters).
//
// MATRIX: 1 output (O3, 5 facets) x 3 partitions (P1a, P1b, P1c) = 15 cells, all covered
// below. (B) is a single structural path, not part of this matrix (see EXAMPLES pattern
// in inc_i_172_cli_trust_root_resolution_test.rs for why structural assertions run
// separately from the behavioural matrix).
// ============================================================================

use doli_cli::upgrade_systemd_plan::{systemd_restart_plan, SystemdStep};

fn args_of(strs: &[&str]) -> Vec<String> {
    strs.iter().map(|s| s.to_string()).collect()
}

fn assert_step(step: &SystemdStep, expected_args: &[&str], expected_required: bool, label: &str) {
    assert_eq!(
        step.args,
        args_of(expected_args),
        "{label}: argv mismatch (unit must be threaded verbatim into the array, not \
         reformatted or dropped)"
    );
    assert_eq!(
        step.required, expected_required,
        "{label}: required flag mismatch"
    );
}

/// REQ-188-001 (Must) — Decision: reveals whether a future edit reorders reset-failed
/// after restart, which would silently reintroduce the start-limit-lock deadlock this
/// incident diagnosed, because "contains reset-failed" alone cannot detect a swap.
/// [P1a -> O3 len, O3 step0, O3 step1]
#[test]
fn test_systemd_restart_plan_orders_reset_failed_before_restart() {
    let unit = "doli-mainnet-n3.service";
    let plan = systemd_restart_plan(unit);

    assert_eq!(
        plan.len(),
        2,
        "the plan must be exactly reset-failed then restart, no more, no fewer steps"
    );
    assert_step(
        &plan[0],
        &["systemctl", "reset-failed", unit],
        false,
        "step 0 (must be reset-failed, first)",
    );
    assert_step(
        &plan[1],
        &["systemctl", "restart", unit],
        true,
        "step 1 (must be restart, second)",
    );
}

/// REQ-188-002 (Should) — Decision: reveals whether a future edit marks reset-failed as
/// required (aborting the restart attempt when the unit was never in start-limit lock,
/// e.g. `systemctl reset-failed` on an unknown/never-failed unit returns non-zero) or
/// marks restart as best-effort (silently swallowing a genuine restart failure so the
/// operator believes the node is back up when it is not).
/// [P1a -> O3 step0.required, O3 step1.required]
#[test]
fn test_systemd_restart_plan_marks_reset_failed_best_effort_and_restart_required() {
    let plan = systemd_restart_plan("doli-mainnet-n3.service");

    assert!(
        !plan[0].required,
        "reset-failed is best-effort prep (REQ-188-002): a unit that was never failed \
         must not abort the restart attempt"
    );
    assert!(
        plan[1].required,
        "restart is the operation the caller actually needs to succeed — it must stay \
         required or a real failure goes unreported"
    );
}

/// REQ-188-001 (Must) — Decision: reveals whether the unit name is threaded from the
/// argument or silently hardcoded to one fixture value, which P1a alone cannot catch
/// because a hardcoded "doli-mainnet-n3.service" would pass P1a too.
/// [P1b -> O3 len, O3 step0, O3 step1]
#[test]
fn test_systemd_restart_plan_threads_a_different_unit_verbatim() {
    let unit = "doli-testnet-seed.service";
    let plan = systemd_restart_plan(unit);

    assert_eq!(plan.len(), 2);
    assert_step(
        &plan[0],
        &["systemctl", "reset-failed", unit],
        false,
        "step 0 with a second, distinct unit fixture",
    );
    assert_step(
        &plan[1],
        &["systemctl", "restart", unit],
        true,
        "step 1 with a second, distinct unit fixture",
    );
}

/// REQ-188-001 (Must) — Decision: reveals whether the plan builder does any shell-style
/// interpretation or escaping of the unit string, which would corrupt the argv Command::args
/// receives and could turn a defensive threading bug into a command-argument confusion.
/// [P1c -> O3 len, O3 step0, O3 step1]
#[test]
fn test_systemd_restart_plan_threads_adversarial_unit_verbatim() {
    let unit = "doli-mainnet-n3.service; rm -rf / #";
    let plan = systemd_restart_plan(unit);

    assert_eq!(plan.len(), 2);
    assert_step(
        &plan[0],
        &["systemctl", "reset-failed", unit],
        false,
        "step 0 with an adversarial unit string",
    );
    assert_step(
        &plan[1],
        &["systemctl", "restart", unit],
        true,
        "step 1 with an adversarial unit string — must NOT be split, escaped, or truncated",
    );
    // The adversarial string must land as a SINGLE argv element, not be split on the
    // embedded ';' or '#' — proves no shell/string-splitting path exists in the plan.
    assert_eq!(
        plan[0].args.len(),
        3,
        "reset-failed step must stay a 3-element argv"
    );
    assert_eq!(
        plan[1].args.len(),
        3,
        "restart step must stay a 3-element argv"
    );
}

/// Body of `bins/cli/src/upgrade_restart.rs`, whitespace-normalised so formatting changes
/// (multi-line `.args([...])`, trailing commas, spacing) cannot defeat the substring checks.
fn upgrade_restart_source_normalised() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/upgrade_restart.rs");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("could not read {}: {e}", path.display());
    });
    raw.chars().filter(|c| !c.is_whitespace()).collect()
}

/// REQ-188-001 (Must) — Decision: reveals whether either call site still hardcodes the
/// unguarded `systemctl restart` argv literal directly, which would mean the fix added the
/// plan but never actually routed the two live call sites through it — the exact
/// detection-without-action shape the root cause analysis already found once (a fallback
/// message that "fails for the same reason").
/// [B -> O_B1]
#[test]
fn test_upgrade_restart_source_has_no_raw_restart_argv_literal() {
    let normalised = upgrade_restart_source_normalised();

    assert!(
        !normalised.contains(r#"["systemctl","restart","#),
        "bins/cli/src/upgrade_restart.rs still constructs a raw `[\"systemctl\", \"restart\", \
         ...]` argv literal. Both `restart_specific_service` and `try_restart_systemd` must \
         issue their systemctl commands from `upgrade_systemd_plan::systemd_restart_plan`'s \
         steps, never a hand-written literal — that literal is the exact bug INC-I-188 \
         diagnosed (restart-only, no reset-failed, refused by a start-limit-locked unit)."
    );
}

/// REQ-188-001 (Must) — Decision: reveals whether only ONE of the two live call sites was
/// wired to the plan, leaving the other one silently unfixed (a partial fix would still make
/// the prior test pass once literals are gone from the fixed site, but this test requires
/// BOTH `restart_specific_service` and `try_restart_systemd` to reach the plan function).
/// [B -> O_B2]
#[test]
fn test_upgrade_restart_source_routes_both_call_sites_through_plan() {
    let normalised = upgrade_restart_source_normalised();
    let needle: String = "systemd_restart_plan("
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    let occurrences = normalised.matches(needle.as_str()).count();
    assert!(
        occurrences >= 2,
        "bins/cli/src/upgrade_restart.rs must call `systemd_restart_plan(` from BOTH \
         `restart_specific_service` and `try_restart_systemd` — found {occurrences} \
         occurrence(s). A single wired call site leaves the other route (the one exercised \
         by whichever tier actually fires on a given host) still unable to recover a \
         start-limit-locked unit."
    );
}

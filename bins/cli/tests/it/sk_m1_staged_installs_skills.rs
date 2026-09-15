// SK-M1 — the staged upgrade installs skills too (TDD RED).
// REQ-SKILLS-002 (Must) — Decision: whether a host that upgrades through the INC-I-215
//   staged handoff (every `doli service install` host, because the sandboxed node cannot
//   write /usr/bin) gets skills at all. Today `staged_install()` has ZERO skills calls, so
//   on those hosts the skills tree simply never updates and nobody finds out.
//
// STATE: RED. `bins/cli/src/cmd_upgrade_staged.rs` contains no skills call, and
// `bins/cli/src/cmd_upgrade.rs` still re-derives `$HOME` to print where the files went.
//
// REQUIRED (binding on the developer) — one shared entry point, so the direct path and
// the staged path cannot drift:
//
//   // crates/updater/src/skills.rs, re-exported from crates/updater/src/lib.rs
//   pub fn install_skills_to_resolved_dir(tarball: &[u8])
//       -> updater::Result<(usize, std::path::PathBuf)>
//
// It resolves with `skills_dir()` and installs with `install_skills_into`, returning the
// directory actually used so the caller prints the truth instead of re-deriving `$HOME`.
// Both `cmd_upgrade.rs` and `cmd_upgrade_staged.rs` must call it, best-effort.
//
// ============================================================================
// OUTPUT CONTRACT (.claude/protocols/output-contract.md)
// ============================================================================
// Subject: fn staged_install(..) and fn cmd_upgrade(..) — their skills side effect.
//   Both are private, bin-only and async, and the real ones install binaries and restart
//   services. `staged_install` is unreachable from an integration test; the SOURCE TEXT of
//   the two call sites is therefore the observable, which is the idiom already used by
//   inc_i_222_helper_refresh_paths.rs in this same binary.
// ENUMERATED OUTPUTS:
//   O1 source text of bins/cli/src/cmd_upgrade_staged.rs — a skills call is present.
//   O2 source text of bins/cli/src/cmd_upgrade_staged.rs — that call is BEST-EFFORT: a
//      skills failure must never fail an upgrade whose binary is already installed.
//   O3 source text of bins/cli/src/cmd_upgrade.rs — the same shared entry point, and the
//      cosmetic `$HOME` re-derivation is gone.
//   mutable params / receiver / return value: not observable here (see above).
// CODE PATHS: A) staged install reaches its skills step; B) direct upgrade reaches its
//   skills step. Both are unconditional in their function bodies, so presence in the
//   source is presence on the path.
// INPUT PARTITIONS: P1 the source text of cmd_upgrade_staged.rs;
//                   P2 the source text of cmd_upgrade.rs.
// MATRIX: O1 x P1 -> g1 ; O2 x P1 -> g2 ; O3 x P2 -> g3.
// NOT COVERED (deliberate): spawning `doli upgrade --from-staged` end to end. That path
//   installs a binary and restarts services; the existing inc_i_215_from_staged.rs fixture
//   exists precisely because doing it carelessly bounces a live local node fleet. The
//   skills step adds no new branch that a spawn would reveal.

use std::path::Path;

fn read_src(rel: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

const SHARED_ENTRY: &str = "install_skills_to_resolved_dir";

// REQ-SKILLS-002 (Must) — Acceptance: the staged path installs skills like the direct path.
#[test]
fn g1_staged_install_installs_skills() {
    let src = read_src("src/cmd_upgrade_staged.rs");
    assert!(
        src.contains(SHARED_ENTRY),
        "O1/P1 REQ-SKILLS-002: cmd_upgrade_staged.rs must call `updater::{SHARED_ENTRY}`. Every \
         `doli service install` host upgrades through this path, and it installs no skills today"
    );
}

// REQ-SKILLS-002 (Must) — Acceptance: a skills failure never fails a staged upgrade.
#[test]
fn g2_the_staged_skills_call_is_best_effort() {
    let src = read_src("src/cmd_upgrade_staged.rs");
    let call = src
        .find(SHARED_ENTRY)
        .unwrap_or_else(|| panic!("O2/P1: cmd_upgrade_staged.rs never calls `{SHARED_ENTRY}`"));
    let window: String = src[call..].chars().take(200).collect();
    assert!(
        !window.contains(")?"),
        "O2/P1 REQ-SKILLS-002: the staged skills call must not propagate with `?`. The binary is \
         already installed by this point, so a skills error would abort the upgrade after the \
         fact. Window:\n{window}"
    );
}

// REQ-SKILLS-002 (Must) — Acceptance: both upgrade paths share one entry point, and the
// direct path reports the directory it really used rather than re-deriving `$HOME`.
#[test]
fn g3_direct_upgrade_uses_the_same_entry_point_and_stops_re_deriving_home() {
    let src = read_src("src/cmd_upgrade.rs");
    assert!(
        src.contains(SHARED_ENTRY),
        "O3/P2 REQ-SKILLS-002: cmd_upgrade.rs must call the same `updater::{SHARED_ENTRY}` as the \
         staged path — two resolutions drift, and the one that drifts is the one under sudo"
    );
    assert!(
        !src.contains("/.doli/skills/"),
        "O3/P2 REQ-SKILLS-002: cmd_upgrade.rs must print the path the installer returned, not a \
         hardcoded `$HOME/.doli/skills/` that is wrong under sudo"
    );
}

// SK-M1 — the skills-directory resolver (TDD RED).
// REQ-SKILLS-001 (Must) — Decision: whether `sudo doli upgrade` puts skills where the
//   operator's agent reads them, or in root's home where nothing ever looks.
// REQ-SKILLS-002 (Must) — Decision: whether an explicit DOLI_SKILLS_DIR can override a
//   wrong automatic guess, which is the only escape hatch a packaged host has.
//
// THE DEFECT: `install_skills_from_tarball` (crates/updater/src/skills.rs) reads only
// HOME/USERPROFILE. Under sudo that is root's home.
//
// STATE: RED. `updater::SkillsEnv` and `updater::skills_dir_for` do not exist, so this
// module does not compile. That compile error IS the contract.
//
// REQUIRED SIGNATURES (binding on the developer):
//   pub struct SkillsEnv {              // all four fields `pub`, so tests can build one
//       pub skills_dir: Option<String>,     // DOLI_SKILLS_DIR
//       pub sudo_user: Option<String>,      // SUDO_USER
//       pub home: Option<String>,           // HOME
//       pub user_profile: Option<String>,   // USERPROFILE
//   }
//   pub fn skills_dir_for(env: &SkillsEnv) -> updater::Result<std::path::PathBuf>
//   pub fn skills_dir() -> updater::Result<std::path::PathBuf>   // reads the process env
//   Both re-exported from crates/updater/src/lib.rs.
//
// ============================================================================
// OUTPUT CONTRACT (.claude/protocols/output-contract.md)
// ============================================================================
// Subject: fn skills_dir_for(&SkillsEnv) -> Result<PathBuf>.
// ENUMERATED OUTPUTS:
//   O1 return Ok(PathBuf) — the resolved directory. The primary observable.
//   O2 return Err — neither HOME nor USERPROFILE is determinable.
//   mutable params : NONE (shared ref).      receiver : NONE (free fn).
//   persistent store: NONE — the resolver is pure. Asserted indirectly: every injected
//     home below is a path that does not exist, so a resolver that stat-ed its input
//     could not return Ok.
//   process env    : NONE WRITTEN. Load-bearing — no test in this file mutates
//     process-wide environment, so these tests are safe under a threaded harness.
// CODE PATHS:
//   A) DOLI_SKILLS_DIR set             -> O1 = that path verbatim
//   B) SUDO_USER non-empty and != root -> O1 = that user's real home + .doli/skills
//   C) otherwise                       -> O1 = HOME (else USERPROFILE) + .doli/skills
//   D) no HOME and no USERPROFILE      -> O2
// INPUT PARTITIONS:
//   P1 override + sudo_user + home all set          -> A
//   P2 override + home, no sudo_user                -> A
//   P3 sudo_user = the invoking user, fake home     -> B
//   P4 sudo_user absent, HOME and USERPROFILE both  -> C
//   P5 sudo_user = ""                               -> C
//   P6 sudo_user = "root"                           -> C
//   P7 no HOME, USERPROFILE set                     -> C
//   P8 nothing set                                  -> D
// MATRIX: O1 x P1..P7 -> t1..t7 ; O2 x P8 -> t8. Every cell has a test.
// NOT COVERED: `skills_dir()` itself — it only reads the process env and hands off to
//   skills_dir_for; asserting it would require process-wide env mutation, which is the
//   exact hazard the injectable form exists to avoid.

use std::path::PathBuf;

use updater::SkillsEnv;

/// A home that cannot exist. Any Ok result derived from it proves the resolver is pure.
const FAKE_HOME: &str = "/nonexistent/sk-m1-process-home";
const OVERRIDE: &str = "/nonexistent/sk-m1-override";
/// A login name no host resolves, used where path B must NOT be taken.
const UNRESOLVABLE_USER: &str = "sk-m1-not-a-real-user";

fn env(
    skills_dir: Option<&str>,
    sudo_user: Option<&str>,
    home: Option<&str>,
    user_profile: Option<&str>,
) -> SkillsEnv {
    SkillsEnv {
        skills_dir: skills_dir.map(str::to_string),
        sudo_user: sudo_user.map(str::to_string),
        home: home.map(str::to_string),
        user_profile: user_profile.map(str::to_string),
    }
}

/// The login name that started this test process, or None when it is unusable.
fn invoking_user() -> Option<String> {
    let raw = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .ok()?;
    let user = raw.trim().to_string();
    if user.is_empty() || user == "root" {
        return None;
    }
    Some(user)
}

// REQ-SKILLS-002 (Must) — Acceptance: DOLI_SKILLS_DIR is checked first and beats SUDO_USER.
#[test]
fn t1_explicit_override_wins_over_sudo_user_and_over_home() {
    let e = env(
        Some(OVERRIDE),
        Some(UNRESOLVABLE_USER),
        Some(FAKE_HOME),
        None,
    );
    assert_eq!(
        updater::skills_dir_for(&e).expect("O1/P1: the override always resolves"),
        PathBuf::from(OVERRIDE),
        "O1/P1 REQ-SKILLS-002: DOLI_SKILLS_DIR must be checked before SUDO_USER and HOME"
    );
}

// REQ-SKILLS-002 (Must) — Acceptance: the override is the directory, not its parent.
#[test]
fn t2_explicit_override_is_used_verbatim_with_nothing_appended() {
    let e = env(Some(OVERRIDE), None, Some(FAKE_HOME), None);
    assert_eq!(
        updater::skills_dir_for(&e).expect("O1/P2"),
        PathBuf::from(OVERRIDE),
        "O1/P2 REQ-SKILLS-002: `.doli/skills` must NOT be appended to an explicit override"
    );
}

// REQ-SKILLS-001 (Must) — Acceptance: SUDO_USER yields that user's home, not the process HOME.
#[test]
fn t3_sudo_user_resolves_to_that_users_real_home_not_the_process_home() {
    let Some(user) = invoking_user() else {
        println!("SKIP t3: no usable USER/LOGNAME in this environment");
        return;
    };
    let Ok(real_home) = std::env::var("HOME") else {
        println!("SKIP t3: the test process has no HOME to compare against");
        return;
    };

    let e = env(None, Some(&user), Some(FAKE_HOME), None);
    let got = updater::skills_dir_for(&e).expect("O1/P3: a real user must resolve");

    assert!(
        got.is_absolute(),
        "O1/P3 REQ-SKILLS-001: resolved path must be absolute, got {}",
        got.display()
    );
    assert!(
        !got.starts_with(FAKE_HOME),
        "O1/P3 REQ-SKILLS-001: SUDO_USER was set, so the process HOME must be ignored; got {}",
        got.display()
    );
    assert!(
        got.ends_with(".doli/skills"),
        "O1/P3 REQ-SKILLS-001: resolved path must end with .doli/skills, got {}",
        got.display()
    );
    assert_eq!(
        got,
        PathBuf::from(&real_home).join(".doli").join("skills"),
        "O1/P3 REQ-SKILLS-001: SUDO_USER must resolve to that user's real home"
    );
}

// REQ-SKILLS-001 (Must) — Acceptance: no SUDO_USER falls back to HOME, and HOME beats USERPROFILE.
#[test]
fn t4_absent_sudo_user_falls_back_to_process_home_and_home_beats_user_profile() {
    let e = env(
        None,
        None,
        Some(FAKE_HOME),
        Some("/nonexistent/sk-m1-profile"),
    );
    assert_eq!(
        updater::skills_dir_for(&e).expect("O1/P4"),
        PathBuf::from(FAKE_HOME).join(".doli").join("skills"),
        "O1/P4 REQ-SKILLS-001: with no SUDO_USER the process HOME wins, ahead of USERPROFILE"
    );
}

// REQ-SKILLS-001 (Must) — Acceptance: an empty SUDO_USER is not a user.
#[test]
fn t5_empty_sudo_user_falls_back_to_process_home() {
    let e = env(None, Some(""), Some(FAKE_HOME), None);
    assert_eq!(
        updater::skills_dir_for(&e).expect("O1/P5"),
        PathBuf::from(FAKE_HOME).join(".doli").join("skills"),
        "O1/P5 REQ-SKILLS-001: SUDO_USER=\"\" must not be treated as a login name"
    );
}

// REQ-SKILLS-001 (Must) — Acceptance: SUDO_USER=root is the no-op case, not a lookup.
#[test]
fn t6_sudo_user_root_falls_back_to_process_home() {
    let e = env(None, Some("root"), Some(FAKE_HOME), None);
    assert_eq!(
        updater::skills_dir_for(&e).expect("O1/P6"),
        PathBuf::from(FAKE_HOME).join(".doli").join("skills"),
        "O1/P6 REQ-SKILLS-001: SUDO_USER=root means root already owns the session"
    );
}

// REQ-SKILLS-001 (Must) — Acceptance: USERPROFILE covers the host with no HOME.
#[test]
fn t7_user_profile_is_used_when_home_is_absent() {
    let profile = "/nonexistent/sk-m1-profile";
    let e = env(None, None, None, Some(profile));
    assert_eq!(
        updater::skills_dir_for(&e).expect("O1/P7"),
        PathBuf::from(profile).join(".doli").join("skills"),
        "O1/P7 REQ-SKILLS-001: USERPROFILE is the fallback when HOME is unset"
    );
}

// REQ-SKILLS-001 (Must) — Acceptance: an undeterminable home is an error, never a guess.
#[test]
fn t8_no_home_and_no_user_profile_is_an_error() {
    let e = env(None, None, None, None);
    assert!(
        updater::skills_dir_for(&e).is_err(),
        "O2/P8 REQ-SKILLS-001: with nothing to resolve the resolver must fail, not invent a path"
    );
}

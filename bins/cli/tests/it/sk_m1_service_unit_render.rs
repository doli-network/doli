// SK-M1 — the systemd unit carries the skills directory (TDD RED).
// REQ-SKILLS-002 (Must) — Decision: whether a node installed as a service can ever write
//   skills anywhere an operator reads. The unit runs `ProtectSystem=full` with a
//   `ReadWritePaths=` allow-list, so a skills path that is absent from the unit is not a
//   wrong path — it is an unwritable one, and the auto-updater's install silently fails.
//
// STATE: RED. The renderer below does not exist yet: `install_systemd` builds the unit
// with an inline `format!` inside `bins/cli/src/cmd_service.rs`, which is bin-only and
// 931 lines (over the 500-line budget — SK-M1 forbids growing it).
//
// REQUIRED (binding on the developer) — the SMALLEST change that makes this reachable:
//   1. A NEW small module `bins/cli/src/service_unit.rs`, declared in the EXISTING
//      `bins/cli/src/lib.rs` as `pub mod service_unit;` (lib.rs already exists, so no new
//      target and no Cargo.toml change is needed).
//   2. The unit body lifted verbatim out of `install_systemd` into a pure function:
//
//        pub fn render_service_unit(
//            network: &str,
//            exec_start: &str,
//            data_dir: &str,
//            user: &str,
//            group: &str,
//            skills_dir: &std::path::Path,
//        ) -> String
//
//   3. `install_systemd` calls it instead of its inline `format!`, and pre-creates the
//      skills directory in a binding named `skills_dir`, chowned to the service user.
//      The name matters: t5 below reads the source text, which is the only observable a
//      bin-only function offers.
//
// ============================================================================
// OUTPUT CONTRACT (.claude/protocols/output-contract.md)
// ============================================================================
// Subject: fn render_service_unit(..) -> String, plus its call site.
// ENUMERATED OUTPUTS:
//   O1 return String — the unit text. Asserted as independently breakable facets:
//      O1a the exact `Environment=DOLI_SKILLS_DIR=<path>` line,
//      O1b the skills path inside the `ReadWritePaths=` line,
//      O1c the pre-existing data dir and log dir still inside `ReadWritePaths=`,
//      O1d the pre-existing hardening/identity lines survive the extraction.
//   mutable params: NONE (all shared refs). receiver: NONE. store: NONE (pure).
//   stdout: NONE. env: NONE READ — host-independent by construction.
//   O2 SOURCE TEXT of cmd_service.rs — the call site and the directory pre-creation.
//      Source text is the observable because `install_systemd` writes
//      /etc/systemd/system as root and cannot be executed from a test.
// CODE PATHS: one — the function is a single `format!`. The branching that picks its
//   arguments (`detect_service_user`, `which_doli_node`) is host-dependent and stays
//   outside the pure seam, which is the point of extracting it.
// INPUT PARTITIONS:
//   P1 ordinary arguments, a skills path under a user home
//   P2 a skills path carrying a space (a home directory may contain one)
//   P3 the source text of bins/cli/src/cmd_service.rs
// MATRIX: O1a x P1 -> t1 ; O1b,O1c x P1 -> t2 ; O1a,O1b x P2 -> t3 ; O1d x P1 -> t4 ;
//         O2 x P3 -> t5.
// NOT COVERED: writing the unit file and running `systemctl` — root-only side effects on
//   a live host; those are ops checks, not test-suite work.

use std::path::{Path, PathBuf};

fn read_src(rel: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

fn line_with<'a>(text: &'a str, needle: &str, what: &str) -> &'a str {
    text.lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("{what}: no line contains `{needle}`\n--- text ---\n{text}"))
}

const DATA_DIR: &str = "/var/lib/doli/mainnet";
const EXEC: &str = "/usr/local/bin/doli-node --network mainnet";

fn render(skills: &Path) -> String {
    doli_cli::service_unit::render_service_unit("mainnet", EXEC, DATA_DIR, "doli", "doli", skills)
}

// REQ-SKILLS-002 (Must) — Acceptance: the unit exports DOLI_SKILLS_DIR to the node.
#[test]
fn t1_unit_exports_doli_skills_dir_for_the_service() {
    let skills = PathBuf::from("/var/lib/doli/agent/.doli/skills");
    let unit = render(&skills);
    let expected = format!("Environment=DOLI_SKILLS_DIR={}", skills.display());
    assert!(
        unit.lines().any(|l| l.trim() == expected),
        "O1a/P1 REQ-SKILLS-002: the unit must carry the exact line `{expected}`\n--- unit ---\n{unit}"
    );
}

// REQ-SKILLS-002 (Must) — Acceptance: the skills path is writable under ProtectSystem=full.
#[test]
fn t2_read_write_paths_grants_the_skills_directory_without_losing_the_old_entries() {
    let skills = PathBuf::from("/var/lib/doli/agent/.doli/skills");
    let unit = render(&skills);
    let rwp = line_with(&unit, "ReadWritePaths=", "O1b/P1");

    assert!(
        rwp.contains(&skills.display().to_string()),
        "O1b/P1 REQ-SKILLS-002: ReadWritePaths= must grant the skills directory, got `{rwp}`"
    );
    assert!(
        rwp.contains(DATA_DIR),
        "O1c/P1 REQ-SKILLS-002: ReadWritePaths= must keep the data dir, got `{rwp}`"
    );
    assert!(
        rwp.contains("/var/log/doli"),
        "O1c/P1 REQ-SKILLS-002: ReadWritePaths= must keep the log dir, got `{rwp}`"
    );
}

// REQ-SKILLS-002 (Must) — Acceptance: a home directory with a space still yields one
// usable path in both places (systemd splits ReadWritePaths= on whitespace).
#[test]
fn t3_a_skills_path_with_a_space_is_quoted_in_read_write_paths() {
    let skills = PathBuf::from("/Users/a b/.doli/skills");
    let unit = render(&skills);
    let expected = format!("Environment=DOLI_SKILLS_DIR={}", skills.display());

    assert!(
        unit.lines().any(|l| l.trim() == expected),
        "O1a/P2 REQ-SKILLS-002: the exported value must be the raw path\n--- unit ---\n{unit}"
    );
    let rwp = line_with(&unit, "ReadWritePaths=", "O1b/P2");
    assert!(
        rwp.contains(&format!("\"{}\"", skills.display())),
        "O1b/P2 REQ-SKILLS-002: a path containing a space must be quoted in ReadWritePaths=, \
         otherwise systemd reads it as two paths. got `{rwp}`"
    );
}

// REQ-SKILLS-002 (Must) — Acceptance: extracting the renderer drops nothing that was
// already in the unit.
#[test]
fn t4_extraction_preserves_the_existing_identity_and_hardening_lines() {
    let unit = render(Path::new("/var/lib/doli/agent/.doli/skills"));
    for needle in [
        "[Unit]",
        "[Service]",
        "[Install]",
        "Type=simple",
        "User=doli",
        "Group=doli",
        "Restart=always",
        "NoNewPrivileges=true",
        "ProtectSystem=full",
        "WantedBy=multi-user.target",
        EXEC,
    ] {
        assert!(
            unit.contains(needle),
            "O1d/P1 REQ-SKILLS-002: the extracted renderer dropped `{needle}`\n--- unit ---\n{unit}"
        );
    }
}

// REQ-SKILLS-002 (Must) — Acceptance: the installer uses this renderer and pre-creates
// the directory, so there is exactly ONE unit template on the host.
#[test]
fn t5_install_systemd_uses_the_shared_renderer_and_precreates_the_skills_dir() {
    let src = read_src("src/cmd_service.rs");

    assert!(
        src.contains("render_service_unit("),
        "O2/P3 REQ-SKILLS-002: cmd_service.rs must call the shared renderer, not keep its own \
         inline unit template"
    );
    assert!(
        !src.contains("ReadWritePaths={data_dir}"),
        "O2/P3 REQ-SKILLS-002: the inline unit template must be gone from cmd_service.rs — two \
         templates drift, and the second one is the one that ships"
    );
    assert!(
        src.contains("create_dir_all(&skills_dir)"),
        "O2/P3 REQ-SKILLS-002: `doli service install` must pre-create the skills directory it \
         just granted in ReadWritePaths="
    );
    let chown_grants_skills = src.match_indices("chown").any(|(i, _)| {
        src[i..]
            .chars()
            .take(400)
            .collect::<String>()
            .contains("skills_dir")
    });
    assert!(
        chown_grants_skills,
        "O2/P3 REQ-SKILLS-002: no chown near the skills directory — the service user must own \
         the directory the unit just granted it, or the node still cannot write there"
    );
}

// OUTPUT CONTRACT:
//   ADMIN_METHODS (const slice): the 14 names, order = crates/rpc/src/server.rs ADMIN_METHODS.
//   is_admin_method(method) -> bool: true iff method is in ADMIN_METHODS (exact, case-sensitive).
//   resolve_admin_token(env, dir) -> Option<String>: trimmed env | trimmed file | None. Writes nothing.
//   unauthorized_hint(dir) -> String: one line naming ENV_TOKEN and dir/TOKEN_FILE_NAME.
//   TOKEN_FILE_NAME / ENV_TOKEN / UNAUTHORIZED_CODE: wire constants shared with the node.
//   Paths x outputs:
//     env non-empty            -> Some(trim(env))            [token_resolves_from_env_first]
//     env absent/blank + file  -> Some(trim(file))           [token_resolves_from_file_when_env_absent_or_empty]
//     env absent/blank, no file or blank file -> None        [token_none_when_neither]
//     admin name / public name -> true / false               [admin_methods_match_rpc_server_list, public_method_is_not_admin]
//     hint                     -> contains env + full path, 1 line [unauthorized_hint_names_env_and_file]
// INPUT PARTITIONS:
//   env: None | "" | whitespace-only | padded value | plain value
//   file: missing | "<hex>\n" | "<hex>\r\n" | whitespace-only ; dir: existing | nonexistent
//   method: each of 14 admin | public | empty | case variant of admin

use super::*;
use std::path::Path;

const TOKEN_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn server_admin_methods() -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/rpc/src/server.rs");
    let src = std::fs::read_to_string(&path).expect("read crates/rpc/src/server.rs");
    let start = src
        .find("pub const ADMIN_METHODS: &[&str] = &[")
        .expect("server ADMIN_METHODS block");
    let rest = &src[start..];
    let end = rest.find("];").expect("end of server ADMIN_METHODS block");
    rest[..end]
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|l| l.starts_with('"'))
        .map(|l| l.trim_end_matches(',').trim_matches('"').to_string())
        .collect()
}

/// R-5 — Decision: a drift between the CLI copy and the node list means the CLI sends no token to a gated method.
#[test]
fn admin_methods_match_rpc_server_list() {
    let expected = [
        "pauseProduction",
        "resumeProduction",
        "createCheckpoint",
        "pruneBlocks",
        "backfillFromPeer",
        "enterRecoveryMode",
        "exitRecoveryMode",
        "bridgeFromArchive",
        "getUtxoDiff",
        "getStateSnapshot",
        "getStateRootDebug",
        "verifyChainIntegrity",
        "repairArchiveFromPeer",
        "forceReorgTo",
    ];
    assert_eq!(ADMIN_METHODS.len(), 14);
    assert_eq!(ADMIN_METHODS, &expected[..]);

    let server = server_admin_methods();
    assert_eq!(server.len(), ADMIN_METHODS.len(), "server list: {server:?}");
    for m in ADMIN_METHODS {
        assert!(
            server.iter().any(|s| s == m),
            "{m} missing from server list {server:?}"
        );
        assert!(is_admin_method(m), "{m} must classify as admin");
    }
    assert_eq!(
        server,
        ADMIN_METHODS
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    );
}

/// R-5 — Decision: a public method classified as admin would leak the token to every public call.
#[test]
fn public_method_is_not_admin() {
    for m in [
        "getChainInfo",
        "getBalance",
        "getBlockByHeight",
        "",
        "pauseproduction",
        "PauseProduction",
    ] {
        assert!(!is_admin_method(m), "{m:?} must not classify as admin");
    }
}

/// R-5 — Decision: a usable (>=32-char) env must override the file so an operator can point a tool at another node.
#[test]
fn token_resolves_from_env_first() {
    let env_tok = "e".repeat(40);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(TOKEN_FILE_NAME), format!("{TOKEN_HEX}\n")).unwrap();
    assert_eq!(
        resolve_admin_token(Some(&format!("  {env_tok} \n")), dir.path()),
        Some(env_tok.clone())
    );
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(
        resolve_admin_token(Some(&env_tok), empty.path()),
        Some(env_tok.clone())
    );
}

/// INC-I-308 P2-a — Decision: a failure means a short env token (which the node ignores) shadows the valid token file, so guardian halt gets -32008.
#[test]
fn token_short_env_falls_back_to_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(TOKEN_FILE_NAME), format!("{TOKEN_HEX}\n")).unwrap();
    let file = Some(TOKEN_HEX.to_string());
    let c31 = "s".repeat(31);
    let c32 = "t".repeat(32);
    assert_eq!(resolve_admin_token(Some("legacy-short"), dir.path()), file);
    assert_eq!(resolve_admin_token(Some(&c31), dir.path()), file);
    assert_eq!(
        resolve_admin_token(Some(&c32), dir.path()),
        Some(c32.clone())
    );
    assert_eq!(
        resolve_admin_token(Some(&format!(" {c31}\n")), dir.path()),
        file
    );
    assert_eq!(env_token(Some(&c31)), None);
    assert_eq!(env_token(Some(&c32)), Some(c32));
}

fn const_usize(src: &str, decl: &str) -> Option<usize> {
    let rest = &src[src.find(decl)? + decl.len()..];
    rest[..rest.find(';')?].trim().parse().ok()
}

/// INC-I-308 P2-a — Decision: a failure means the CLI length rule drifts from the node rule, so the CLI sends a token the node ignores.
#[test]
fn min_token_len_matches_node() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let node = std::fs::read_to_string(root.join("../../crates/rpc/src/admin_token.rs")).unwrap();
    let cli = std::fs::read_to_string(root.join("src/rpc_admin_auth.rs")).unwrap();
    let decl = "pub const MIN_ADMIN_TOKEN_LEN: usize =";
    let node_len = const_usize(&node, decl).expect("node MIN_ADMIN_TOKEN_LEN");
    assert_eq!(node_len, 32);
    assert_eq!(
        const_usize(&cli, decl),
        Some(node_len),
        "CLI rpc_admin_auth.rs must declare {decl} {node_len};"
    );
}

fn quoted_names(text: &str) -> Vec<String> {
    let mut v: Vec<String> = text
        .split(|c: char| c == '"' || c == '\'' || c.is_whitespace() || c == '|' || c == ',')
        .filter(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(str::to_string)
        .collect();
    v.sort();
    v.dedup();
    v
}

fn between<'a>(src: &'a str, start: &str, end: &str) -> &'a str {
    let s = src
        .find(start)
        .unwrap_or_else(|| panic!("missing {start:?}"))
        + start.len();
    let e = src[s..]
        .find(end)
        .unwrap_or_else(|| panic!("missing end {end:?}"));
    &src[s..s + e]
}

/// INC-I-308 P3-c — Decision: a failure means a script helper omits an admin method, so that script calls it without a token and gets -32008.
#[test]
fn script_admin_method_lists_match_rpc_server_list() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read =
        |p: &str| std::fs::read_to_string(root.join(p)).unwrap_or_else(|e| panic!("{p}: {e}"));
    let mut server = server_admin_methods();
    server.sort();
    assert_eq!(server.len(), 14);
    let sh = read("scripts/lib/rpc-admin-token.sh");
    let py = read("scripts/lib/rpc_admin_token.py");
    let census = read("scripts/tests/test-rpc-admin-token.sh");
    let lists = [
        (
            "rpc-admin-token.sh RPC_ADMIN_METHODS",
            quoted_names(between(&sh, "RPC_ADMIN_METHODS=\"", "\"")),
        ),
        (
            "rpc_admin_token.py ADMIN_METHODS",
            quoted_names(between(&py, "ADMIN_METHODS = frozenset((", "))")),
        ),
        (
            "test-rpc-admin-token.sh ADMIN",
            quoted_names(between(&census, "\nADMIN='", "'")),
        ),
    ];
    for (name, list) in lists {
        assert_eq!(list, server, "{name} differs from server.rs ADMIN_METHODS");
    }
}

/// R-5 — Decision: a missing or untrimmed file read breaks every operator tool after the upgrade.
#[test]
fn token_resolves_from_file_when_env_absent_or_empty() {
    assert_eq!(TOKEN_FILE_NAME, "rpc-admin.token");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(TOKEN_FILE_NAME), format!("{TOKEN_HEX}\n")).unwrap();
    assert_eq!(
        resolve_admin_token(None, dir.path()),
        Some(TOKEN_HEX.to_string())
    );
    assert_eq!(
        resolve_admin_token(Some(""), dir.path()),
        Some(TOKEN_HEX.to_string())
    );
    assert_eq!(
        resolve_admin_token(Some("  \n\t"), dir.path()),
        Some(TOKEN_HEX.to_string())
    );

    let crlf = tempfile::tempdir().unwrap();
    std::fs::write(
        crlf.path().join(TOKEN_FILE_NAME),
        format!("{TOKEN_HEX}\r\n"),
    )
    .unwrap();
    assert_eq!(
        resolve_admin_token(None, crlf.path()),
        Some(TOKEN_HEX.to_string())
    );
}

/// R-5 — Decision: sending an empty bearer instead of none would mask the unauthorized hint.
#[test]
fn token_none_when_neither() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(resolve_admin_token(None, dir.path()), None);
    assert_eq!(resolve_admin_token(Some(" "), dir.path()), None);

    std::fs::write(dir.path().join(TOKEN_FILE_NAME), "\n  \n").unwrap();
    assert_eq!(resolve_admin_token(None, dir.path()), None);
    assert_eq!(resolve_admin_token(Some(""), dir.path()), None);

    let missing = dir.path().join("does-not-exist");
    assert_eq!(resolve_admin_token(None, &missing), None);
}

/// R-5 — Decision: an operator refused with -32008 must learn both ways to supply the token.
#[test]
fn unauthorized_hint_names_env_and_file() {
    assert_eq!(ENV_TOKEN, "DOLI_RPC_ADMIN_TOKEN");
    assert_eq!(UNAUTHORIZED_CODE, -32008);
    let dir = tempfile::tempdir().unwrap();
    let hint = unauthorized_hint(dir.path());
    assert!(hint.contains("DOLI_RPC_ADMIN_TOKEN"), "hint: {hint}");
    let file = dir.path().join("rpc-admin.token");
    assert!(hint.contains(&file.display().to_string()), "hint: {hint}");
    assert_eq!(
        hint.trim_end().lines().count(),
        1,
        "hint must be one line: {hint:?}"
    );
}

/// INC-I-308 P2-a — Decision: a failure means a -32008 with a short env token tells the operator to set a variable that is already set.
#[test]
fn unauthorized_hint_names_short_env_cause() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(TOKEN_FILE_NAME);
    let hint = unauthorized_hint_for(Some("legacy-short"), &file);
    assert!(hint.contains("shorter than 32"), "hint: {hint}");
    assert!(hint.contains(&file.display().to_string()), "hint: {hint}");
    assert_eq!(hint.trim_end().lines().count(), 1, "hint: {hint:?}");
    let unset = unauthorized_hint_for(None, &file);
    assert!(!unset.contains("shorter than 32"), "hint: {unset}");
    assert!(unset.contains(&file.display().to_string()), "hint: {unset}");
}

/// INC-I-308 M12 — Decision: a failure means a -32008 from a remote node points the operator at a token file the CLI never sends there.
#[test]
fn unauthorized_hint_remote_names_env_only_cause() {
    let hint = unauthorized_hint_remote(Some("legacy-short"));
    assert!(hint.contains("shorter than 32"), "hint: {hint}");
    assert!(hint.contains("ignored"), "hint: {hint}");
    assert!(hint.contains("no token file is sent"), "hint: {hint}");
    assert!(!hint.contains(TOKEN_FILE_NAME), "hint: {hint}");
    assert_eq!(hint.trim_end().lines().count(), 1, "hint: {hint:?}");
    let unset = unauthorized_hint_remote(None);
    assert!(unset.contains("at least 32"), "hint: {unset}");
    assert!(unset.contains("no token file is sent"), "hint: {unset}");
    assert!(!unset.contains(TOKEN_FILE_NAME), "hint: {unset}");
}

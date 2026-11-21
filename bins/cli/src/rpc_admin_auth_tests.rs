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

/// R-5 — Decision: env must override the file so an operator can point a tool at another node.
#[test]
fn token_resolves_from_env_first() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(TOKEN_FILE_NAME), format!("{TOKEN_HEX}\n")).unwrap();
    assert_eq!(
        resolve_admin_token(Some("  env-token \n"), dir.path()),
        Some("env-token".to_string())
    );
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(
        resolve_admin_token(Some("env-only"), empty.path()),
        Some("env-only".to_string())
    );
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

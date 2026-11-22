//! INC-I-308 M1 (run 732) — `resolve_admin_token`, the token every node always has.
//!
//! OUTPUT CONTRACT: fn resolve_admin_token(env, config, data_dir) -> io::Result<(String, TokenSource)>
//!   O1 return: trimmed token + its source, or `Err` when the file cannot be written.
//!   O2 disk: `<data_dir>/rpc-admin.token` (reused | created | written from env/config), content, unix mode.
//!   O3 disk: `data_dir` created when missing; no temp file left in it.
//!   PATHS: env usable · env unusable -> config usable · both unusable -> file reused |
//!          file invalid -> recreated | no file -> created · unwritable -> Err.
//! INPUT PARTITIONS: env/config in {None, "", 31 chars, 32 chars, whitespace-padded >= 32};
//!   file in {absent, valid 0644, valid 0600, "", short, whitespace}; data_dir in
//!   {exists, missing, under a regular file}.

use std::fs;
use std::path::Path;

use tempfile::TempDir;

use super::{
    is_usable_admin_token, resolve_admin_token, token_file_path, TokenSource, ADMIN_TOKEN_FILE,
    MIN_ADMIN_TOKEN_LEN,
};

const ENV_TOKEN: &str = "e0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff";
const CFG_TOKEN: &str = "c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf";
const SHORT_31: &str = "0123456789abcdef0123456789abcde";

fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn token_path(dir: &Path) -> std::path::PathBuf {
    dir.join("rpc-admin.token")
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).expect("metadata").permissions().mode() & 0o777
}

/// Decision: a failure means the file name, the minimum length or the trim rule drifted
/// from the contract the CLI and the scripts read (M3 reads the same path).
#[test]
fn contract_constants_and_usable_token_boundary() {
    assert_eq!(ADMIN_TOKEN_FILE, "rpc-admin.token");
    assert_eq!(MIN_ADMIN_TOKEN_LEN, 32);
    let dir = Path::new("/var/lib/doli");
    assert_eq!(token_file_path(dir), dir.join("rpc-admin.token"));
    assert_eq!(SHORT_31.len(), 31);
    assert!(!is_usable_admin_token(""));
    assert!(!is_usable_admin_token("   \n"));
    assert!(!is_usable_admin_token(SHORT_31));
    assert!(!is_usable_admin_token(&format!("  {SHORT_31}  \n")));
    assert!(is_usable_admin_token(&ENV_TOKEN[..32]));
    assert!(is_usable_admin_token(ENV_TOKEN));
}

/// Decision: a failure means an operator's env token is ignored (tools holding it get
/// refused), or local tools that read the data dir cannot find the token the node uses.
#[test]
fn usable_env_token_wins_and_is_written_to_the_token_file() {
    let d = TempDir::new().unwrap();
    let got = resolve_admin_token(
        Some(ENV_TOKEN.to_string()),
        Some(CFG_TOKEN.into()),
        d.path(),
    )
    .unwrap();
    assert_eq!(got, (ENV_TOKEN.to_string(), TokenSource::Env));
    let path = token_path(d.path());
    assert_eq!(
        fs::read_to_string(&path).expect("env token must be written to the token file"),
        format!("{ENV_TOKEN}\n")
    );
    #[cfg(unix)]
    assert_eq!(mode(&path), 0o600);
}

/// Decision: a failure means a stale token file (an old token, a token from another
/// source) stays on disk, so tools that read it are refused.
#[test]
fn env_token_replaces_a_token_file_with_a_different_token() {
    let d = TempDir::new().unwrap();
    let path = token_path(d.path());
    fs::write(&path, format!("{CFG_TOKEN}\n")).unwrap();
    let got = resolve_admin_token(Some(ENV_TOKEN.to_string()), None, d.path()).unwrap();
    assert_eq!(got, (ENV_TOKEN.to_string(), TokenSource::Env));
    assert_eq!(fs::read_to_string(&path).unwrap(), format!("{ENV_TOKEN}\n"));
    #[cfg(unix)]
    assert_eq!(mode(&path), 0o600);
}

/// Decision: a failure means a token file that already matches is rewritten on every
/// start (needless writes) or is left readable by other users.
#[cfg(unix)]
#[test]
fn matching_token_file_is_not_rewritten_only_tightened() {
    use std::os::unix::fs::PermissionsExt;
    let d = TempDir::new().unwrap();
    let path = token_path(d.path());
    fs::write(&path, format!("  {ENV_TOKEN}  \n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let got = resolve_admin_token(Some(ENV_TOKEN.to_string()), None, d.path()).unwrap();
    assert_eq!(got, (ENV_TOKEN.to_string(), TokenSource::Env));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        format!("  {ENV_TOKEN}  \n"),
        "matching file must not be rewritten"
    );
    assert_eq!(mode(&path), 0o600);
}

/// Decision: a failure means an unwritable data dir stops the node from serving RPC
/// when it already has a usable env token.
#[test]
fn env_token_still_used_when_the_token_file_cannot_be_written() {
    let d = TempDir::new().unwrap();
    let file = d.path().join("regular-file");
    fs::write(&file, "x").unwrap();
    let got = resolve_admin_token(Some(ENV_TOKEN.to_string()), None, &file.join("data")).unwrap();
    assert_eq!(got, (ENV_TOKEN.to_string(), TokenSource::Env));
}

/// Decision: a failure means a padded env value (`export X=" tok\n"`) is either refused
/// or kept with whitespace, so no bearer the tools send can ever match.
#[test]
fn env_token_with_surrounding_whitespace_is_used_trimmed() {
    let d = TempDir::new().unwrap();
    let got = resolve_admin_token(Some(format!("  {ENV_TOKEN}\n")), None, d.path()).unwrap();
    assert_eq!(got, (ENV_TOKEN.to_string(), TokenSource::Env));
}

/// Decision: a failure means an empty or short env value still counts as configured —
/// the root cause of R-3 (an empty `DOLI_RPC_ADMIN_TOKEN=` arms a guessable token).
#[test]
fn empty_or_short_env_token_is_not_used() {
    for bad in ["", "   ", SHORT_31] {
        let d = TempDir::new().unwrap();
        let got = resolve_admin_token(Some(bad.into()), Some(CFG_TOKEN.into()), d.path()).unwrap();
        assert_eq!(
            got,
            (CFG_TOKEN.to_string(), TokenSource::Config),
            "env {bad:?}"
        );

        let d2 = TempDir::new().unwrap();
        let (tok, src) = resolve_admin_token(Some(bad.into()), None, d2.path()).unwrap();
        assert!(is_hex64(&tok), "env {bad:?} -> {tok:?}");
        assert_eq!(src, TokenSource::FileCreated(token_path(d2.path())));
    }
}

/// Decision: a failure means a config-file token is ignored or kept padded, or a short
/// config token still arms the gate.
#[test]
fn config_token_used_when_env_absent_and_short_config_ignored() {
    let d = TempDir::new().unwrap();
    let got = resolve_admin_token(None, Some(format!("{CFG_TOKEN}\n")), d.path()).unwrap();
    assert_eq!(got, (CFG_TOKEN.to_string(), TokenSource::Config));
    assert_eq!(
        fs::read_to_string(token_path(d.path())).unwrap(),
        format!("{CFG_TOKEN}\n")
    );

    let d2 = TempDir::new().unwrap();
    let (tok, src) = resolve_admin_token(None, Some(SHORT_31.into()), d2.path()).unwrap();
    assert!(is_hex64(&tok));
    assert_eq!(src, TokenSource::FileCreated(token_path(d2.path())));
}

/// Decision: a failure means a fresh install is exposed (no token), its token is weak or
/// world-readable, or a restart rotates it and every operator tool loses access.
#[test]
fn fresh_dir_creates_owner_only_64_hex_token_then_reuses_it() {
    let d = TempDir::new().unwrap();
    let path = token_path(d.path());
    let (tok, src) = resolve_admin_token(None, None, d.path()).unwrap();
    assert!(is_hex64(&tok), "token {tok:?}");
    assert_eq!(src, TokenSource::FileCreated(path.clone()));
    let content = fs::read_to_string(&path).unwrap();
    assert_eq!(content, format!("{tok}\n"));
    #[cfg(unix)]
    assert_eq!(mode(&path), 0o600);

    let again = resolve_admin_token(None, None, d.path()).unwrap();
    assert_eq!(again, (tok, TokenSource::FileReused(path.clone())));
    #[cfg(unix)]
    assert_eq!(mode(&path), 0o600);
}

/// Decision: a failure means an existing operator token is rotated on restart, or a
/// token file left readable by group/other keeps leaking to other local users.
#[cfg(unix)]
#[test]
fn existing_valid_file_is_reused_and_its_mode_tightened() {
    use std::os::unix::fs::PermissionsExt;
    let d = TempDir::new().unwrap();
    let path = token_path(d.path());
    fs::write(&path, format!("  {CFG_TOKEN}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let got = resolve_admin_token(None, None, d.path()).unwrap();
    assert_eq!(
        got,
        (CFG_TOKEN.to_string(), TokenSource::FileReused(path.clone()))
    );
    assert_eq!(mode(&path), 0o600);
}

/// Decision: a failure means an empty, short or blank token file arms the gate with a
/// guessable token instead of being replaced by a fresh random one.
#[test]
fn existing_unusable_file_is_replaced_by_a_fresh_token() {
    for bad in ["", SHORT_31, "  \n\t\n"] {
        let d = TempDir::new().unwrap();
        let path = token_path(d.path());
        fs::write(&path, bad).unwrap();
        let (tok, src) = resolve_admin_token(None, None, d.path()).unwrap();
        assert!(is_hex64(&tok), "file {bad:?} -> {tok:?}");
        assert_eq!(src, TokenSource::FileCreated(path.clone()), "file {bad:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), format!("{tok}\n"));
        #[cfg(unix)]
        assert_eq!(mode(&path), 0o600);
    }
}

/// Decision: a failure means tokens are not random (constant or derived), so one
/// node's token opens every other node.
#[test]
fn two_fresh_dirs_get_different_tokens() {
    let (a, b) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let (ta, _) = resolve_admin_token(None, None, a.path()).unwrap();
    let (tb, _) = resolve_admin_token(None, None, b.path()).unwrap();
    assert_ne!(ta, tb);
}

/// Decision: a failure means a first start on a new data dir fails (or runs with no
/// token) because the dir does not exist yet when the token is resolved.
#[test]
fn missing_data_dir_is_created() {
    let d = TempDir::new().unwrap();
    let nested = d.path().join("a").join("b");
    let (tok, src) = resolve_admin_token(None, None, &nested).unwrap();
    assert!(is_hex64(&tok));
    assert_eq!(src, TokenSource::FileCreated(token_path(&nested)));
    assert!(token_path(&nested).is_file());
}

/// Decision: a failure means an unwritable location is reported as success, so the node
/// starts with a token no tool can read (or panics instead of returning the error).
#[test]
fn unwritable_location_is_an_error() {
    let d = TempDir::new().unwrap();
    let file = d.path().join("plain-file");
    fs::write(&file, b"x").unwrap();
    assert!(resolve_admin_token(None, None, &file.join("data")).is_err());
}

/// Decision: a failure means the atomic write leaves a temp copy of the token in the
/// data dir (a second readable copy, or debris that a later start mistakes for state).
#[test]
fn no_temp_file_is_left_after_creation() {
    let d = TempDir::new().unwrap();
    resolve_admin_token(None, None, d.path()).unwrap();
    let names: Vec<String> = fs::read_dir(d.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["rpc-admin.token".to_string()]);
}

//! INC-I-308 send-back 2 — the node's admin-token wiring at RPC start (INV-RPC-003).
//!
//! OUTPUT CONTRACT: provision_admin_token(env, config, data_dir) -> Option<String> and
//!   `<data_dir>/rpc-admin.token`.
//! INPUT PARTITIONS: {usable env token, no env + no config, unwritable data dir}.

use super::provision_admin_token;
use tempfile::TempDir;

const ENV_TOK: &str = "e0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff";

/// Decision: a failure means the node serves a token other than its env token, or the
/// local tools cannot read that token from rpc-admin.token.
#[test]
fn env_token_is_used_and_written_to_the_token_file() {
    let d = TempDir::new().unwrap();
    let got = provision_admin_token(Some(ENV_TOK.to_string()), None, d.path());
    assert_eq!(got.as_deref(), Some(ENV_TOK));
    let file = std::fs::read_to_string(d.path().join("rpc-admin.token")).unwrap();
    assert_eq!(file.trim(), ENV_TOK);
}

/// Decision: a failure means a node with no configured token serves no admin RPC, or
/// serves a token no local tool can read.
#[test]
fn no_env_no_config_creates_the_token_file() {
    let d = TempDir::new().unwrap();
    let got = provision_admin_token(None, None, d.path()).expect("token");
    let file = std::fs::read_to_string(d.path().join("rpc-admin.token")).unwrap();
    assert_eq!(file.trim(), got);
    assert!(got.len() >= 32);
}

/// Decision: a failure means a node that cannot write its token file still enables admin
/// RPC with a token nobody can read.
#[test]
fn unwritable_data_dir_disables_admin_rpc() {
    let d = TempDir::new().unwrap();
    let blocker = d.path().join("file");
    std::fs::write(&blocker, "x").unwrap();
    assert_eq!(
        provision_admin_token(None, None, &blocker.join("data")),
        None
    );
}

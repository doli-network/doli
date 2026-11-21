//! Admin RPC token: method classification, token resolution, refusal hint.

use std::path::Path;

/// Copy of `ADMIN_METHODS` in `crates/rpc/src/server.rs` (same order).
pub const ADMIN_METHODS: &[&str] = &[
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

/// Token file the node creates in its data dir.
pub const TOKEN_FILE_NAME: &str = "rpc-admin.token";

/// Env var that overrides the token file.
pub const ENV_TOKEN: &str = "DOLI_RPC_ADMIN_TOKEN";

/// JSON-RPC error code the node returns for a refused admin call.
pub const UNAUTHORIZED_CODE: i64 = -32008;

/// True iff `method` is an admin method (exact match).
pub fn is_admin_method(method: &str) -> bool {
    ADMIN_METHODS.contains(&method)
}

/// Node rule: a token shorter than this is ignored (`crates/rpc/src/admin_token.rs`).
pub const MIN_ADMIN_TOKEN_LEN: usize = 32;

/// Valid trimmed `env`, else the non-empty trimmed `<data_dir>/rpc-admin.token`.
pub fn resolve_admin_token(env: Option<&str>, data_dir: &Path) -> Option<String> {
    env_token(env).or_else(|| {
        std::fs::read_to_string(data_dir.join(TOKEN_FILE_NAME))
            .ok()
            .and_then(|s| non_empty(&s))
    })
}

/// Trimmed env value with at least `MIN_ADMIN_TOKEN_LEN` chars only.
pub fn env_token(env: Option<&str>) -> Option<String> {
    env.and_then(non_empty)
        .filter(|t| t.chars().count() >= MIN_ADMIN_TOKEN_LEN)
}

/// Char count of a set but too short trimmed env value.
pub fn short_env_len(env: Option<&str>) -> Option<usize> {
    let n = env.and_then(non_empty)?.chars().count();
    (n < MIN_ADMIN_TOKEN_LEN).then_some(n)
}

/// " (... shorter than 32 chars ...)" for a short env value, else "".
pub fn short_env_note(env: Option<&str>) -> String {
    short_env_len(env)
        .map(|n| format!(" ({ENV_TOKEN} is set but shorter than {MIN_ADMIN_TOKEN_LEN} chars: {n} chars, so no token was sent)"))
        .unwrap_or_default()
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// One-line operator hint for a -32008 refusal.
pub fn unauthorized_hint(data_dir: &Path) -> String {
    unauthorized_hint_for(None, &data_dir.join(TOKEN_FILE_NAME))
}

/// One-line -32008 hint for the given env value and token file.
pub fn unauthorized_hint_for(env: Option<&str>, token_file: &Path) -> String {
    let file = token_file.display();
    match short_env_len(env) {
        Some(n) => format!(
            "admin RPC method refused ({UNAUTHORIZED_CODE}): {ENV_TOKEN} is set but shorter than {MIN_ADMIN_TOKEN_LEN} chars ({n} chars), so the node ignores it; the CLI then sent the token file {file}, which is missing, unreadable, or not this node's token; unset {ENV_TOKEN} or set it to the node's full token, or point DOLI_DATA_DIR at that node's data dir"
        ),
        None => format!(
            "admin RPC method refused ({UNAUTHORIZED_CODE}): set {ENV_TOKEN} or make the node token file {file} readable; on a multi-node host point DOLI_DATA_DIR at that node's data dir"
        ),
    }
}

/// One-line -32008 hint for a remote endpoint: only the env token is ever sent there.
pub fn unauthorized_hint_remote(env: Option<&str>) -> String {
    let need = format!("set {ENV_TOKEN} to that node's full token (at least {MIN_ADMIN_TOKEN_LEN} chars); no token file is sent to a remote endpoint");
    match short_env_len(env) {
        Some(n) => format!(
            "admin RPC method refused ({UNAUTHORIZED_CODE}): {ENV_TOKEN} is shorter than {MIN_ADMIN_TOKEN_LEN} chars ({n} chars), so it was ignored and no token was sent; {need}"
        ),
        None => format!("admin RPC method refused ({UNAUTHORIZED_CODE}): {need}"),
    }
}

#[cfg(test)]
#[path = "rpc_admin_auth_tests.rs"]
mod tests;

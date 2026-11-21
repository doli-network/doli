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

/// Non-empty trimmed `env`, else the non-empty trimmed `<data_dir>/rpc-admin.token`.
pub fn resolve_admin_token(env: Option<&str>, data_dir: &Path) -> Option<String> {
    env_token(env).or_else(|| {
        std::fs::read_to_string(data_dir.join(TOKEN_FILE_NAME))
            .ok()
            .and_then(|s| non_empty(&s))
    })
}

/// Non-empty trimmed env value only.
pub fn env_token(env: Option<&str>) -> Option<String> {
    env.and_then(non_empty)
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// One-line operator hint for a -32008 refusal.
pub fn unauthorized_hint(data_dir: &Path) -> String {
    format!(
        "admin RPC method refused ({UNAUTHORIZED_CODE}): set {ENV_TOKEN} or make the node token file {} readable; on a multi-node host point DOLI_DATA_DIR at that node's data dir",
        data_dir.join(TOKEN_FILE_NAME).display()
    )
}

#[cfg(test)]
#[path = "rpc_admin_auth_tests.rs"]
mod tests;

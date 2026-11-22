//! `doli snap` data-dir wipe: remove chain data, keep keys, wallet, config and the
//! admin RPC token file.

use std::path::Path;

/// Entries of the data dir that `doli snap` never deletes.
pub(crate) const SNAP_PRESERVE: &[&str] = &[
    "keys",
    ".env",
    "node_key",
    "wallet.json",
    "wallet.seed.txt",
    "config.toml",
    doli_cli::rpc_admin_auth::TOKEN_FILE_NAME,
];

/// Delete every entry of `data_dir` not in `SNAP_PRESERVE`, then force-remove the
/// RocksDB dirs (stale .sst/.log files corrupt a reopened database).
pub(crate) fn wipe_chain_data(data_dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(data_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if SNAP_PRESERVE.contains(&name.as_str()) {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                if let Err(e) = std::fs::remove_dir_all(&path) {
                    eprintln!("  Warning: failed to remove {:?}: {}", path, e);
                }
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    for dir_name in ["state_db", "blocks"] {
        let dir = data_dir.join(dir_name);
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

#[cfg(test)]
mod tests {
    //! OUTPUT CONTRACT: wipe_chain_data(dir) — disk: preserved entries stay byte-equal, all others gone.
    //! INPUT PARTITIONS: entries in {preserved file, preserved dir, token file, chain dir, other file}.
    use super::*;

    /// Decision: a failure means `doli snap` deletes the node's admin token file (local
    /// tools are refused until restart) or a key/wallet file, or leaves chain data behind.
    #[test]
    fn snap_wipe_keeps_the_admin_token_file_and_keys() {
        let d = tempfile::TempDir::new().unwrap();
        let p = d.path();
        std::fs::write(p.join("rpc-admin.token"), "tok\n").unwrap();
        std::fs::write(p.join("wallet.json"), "w").unwrap();
        std::fs::create_dir_all(p.join("keys")).unwrap();
        std::fs::create_dir_all(p.join("state_db")).unwrap();
        std::fs::create_dir_all(p.join("blocks")).unwrap();
        std::fs::write(p.join("signed_slots.db"), "s").unwrap();
        wipe_chain_data(p);
        assert_eq!(
            std::fs::read_to_string(p.join("rpc-admin.token"))
                .ok()
                .as_deref(),
            Some("tok\n"),
            "snap must keep rpc-admin.token"
        );
        assert!(p.join("wallet.json").is_file());
        assert!(p.join("keys").is_dir());
        assert!(!p.join("state_db").exists());
        assert!(!p.join("blocks").exists());
        assert!(!p.join("signed_slots.db").exists());
    }
}

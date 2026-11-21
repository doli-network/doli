//! INC-I-308: the admin bearer token every node always has (env, config, or an
//! owner-only file in the data dir).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use rand::rngs::OsRng;
use rand::RngCore;
use tracing::error;

pub const ADMIN_TOKEN_FILE: &str = "rpc-admin.token";
pub const MIN_ADMIN_TOKEN_LEN: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenSource {
    Env,
    Config,
    FileReused(PathBuf),
    FileCreated(PathBuf),
}

/// `<data_dir>/rpc-admin.token`.
pub fn token_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join(ADMIN_TOKEN_FILE)
}

/// True when the trimmed token has at least `MIN_ADMIN_TOKEN_LEN` chars.
pub fn is_usable_admin_token(token: &str) -> bool {
    token.trim().chars().count() >= MIN_ADMIN_TOKEN_LEN
}

/// Resolve the admin token: usable env, else usable config, else the token file
/// (reused when usable, otherwise created as 64 random hex chars, mode 0600).
pub fn resolve_admin_token(
    env: Option<String>,
    config: Option<String>,
    data_dir: &Path,
) -> std::io::Result<(String, TokenSource)> {
    if let Some(tok) = usable("DOLI_RPC_ADMIN_TOKEN env", env) {
        return Ok((tok, TokenSource::Env));
    }
    if let Some(tok) = usable("config rpc.admin_token", config) {
        return Ok((tok, TokenSource::Config));
    }

    fs::create_dir_all(data_dir)?;
    let path = token_file_path(data_dir);
    if let Ok(content) = fs::read_to_string(&path) {
        if is_usable_admin_token(&content) {
            tighten_mode(&path)?;
            return Ok((content.trim().to_string(), TokenSource::FileReused(path)));
        }
    }

    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    write_atomic(data_dir, &path, &token)?;
    Ok((token, TokenSource::FileCreated(path)))
}

fn usable(source: &str, value: Option<String>) -> Option<String> {
    let value = value?;
    if is_usable_admin_token(&value) {
        return Some(value.trim().to_string());
    }
    error!("{source} admin token is shorter than {MIN_ADMIN_TOKEN_LEN} chars; ignored");
    None
}

#[cfg(unix)]
fn tighten_mode(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(path)?.permissions().mode();
    if mode & 0o077 != 0 {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn tighten_mode(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn write_atomic(dir: &Path, target: &Path, token: &str) -> std::io::Result<()> {
    let mut nonce = [0u8; 8];
    OsRng.fill_bytes(&mut nonce);
    let tmp = dir.join(format!(".{ADMIN_TOKEN_FILE}.{}.tmp", hex::encode(nonce)));
    let result = (|| {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(format!("{token}\n").as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests;

use std::io::{self, Read};
use std::path::Path;

use serde_json::Value;

use super::write::{sync_dir, write_durable};

/// Larger manifests are never read: treated as absent.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// `manifest.json` keys, each `None` when absent or of the wrong JSON type.
#[derive(Default)]
pub(super) struct Manifest {
    pub latest_height: Option<u64>,
    pub latest_hash: Option<String>,
    pub genesis_hash: Option<String>,
    pub verified_height: Option<u64>,
}

impl Manifest {
    /// `None` when the file is missing, above [`MAX_MANIFEST_BYTES`] or is
    /// not a JSON object.
    pub(super) fn load(dir: &Path) -> Option<Self> {
        let path = dir.join("manifest.json");
        if std::fs::metadata(&path).ok()?.len() > MAX_MANIFEST_BYTES {
            return None;
        }
        let mut data = String::new();
        std::fs::File::open(&path)
            .ok()?
            .take(MAX_MANIFEST_BYTES + 1)
            .read_to_string(&mut data)
            .ok()?;
        if data.len() as u64 > MAX_MANIFEST_BYTES {
            return None;
        }
        let v: Value = serde_json::from_str(&data).ok()?;
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        v.as_object()?;
        Some(Self {
            latest_height: v.get("latest_height").and_then(Value::as_u64),
            latest_hash: text("latest_hash"),
            genesis_hash: text("genesis_hash"),
            verified_height: v.get("verified_height").and_then(Value::as_u64),
        })
    }
}

/// Read the latest archived height from the manifest file.
pub fn manifest_height(dir: &Path) -> Option<u64> {
    read_manifest_height_from(dir)
}

/// Read the genesis_hash from the archive manifest file.
pub fn manifest_genesis_hash(dir: &Path) -> Option<String> {
    read_manifest_genesis_hash(dir)
}

pub(super) fn read_manifest_height_from(dir: &Path) -> Option<u64> {
    Manifest::load(dir)?.latest_height
}

pub(super) fn read_manifest_genesis_hash(dir: &Path) -> Option<String> {
    Manifest::load(dir)?.genesis_hash
}

/// One atomic, durable write of every key.
pub(super) fn save(
    dir: &Path,
    latest_height: u64,
    latest_hash: &str,
    genesis_hash: Option<&str>,
    verified_height: Option<u64>,
) -> io::Result<()> {
    let mut m = serde_json::json!({
        "latest_height": latest_height,
        "latest_hash": latest_hash,
    });
    if let Some(g) = genesis_hash {
        m["genesis_hash"] = Value::from(g);
    }
    if let Some(h) = verified_height {
        m["verified_height"] = Value::from(h);
    }
    let text = serde_json::to_string_pretty(&m).map_err(io::Error::other)?;
    write_durable(&dir.join("manifest.json"), text.as_bytes())?;
    sync_dir(dir);
    Ok(())
}

/// Live-writer save: `verified_height` is kept only while `genesis_hash` is
/// unchanged; with no genesis to write, both existing keys are kept.
pub(super) fn save_carrying(
    dir: &Path,
    latest_height: u64,
    latest_hash: &crypto::Hash,
    genesis_hash: Option<&crypto::Hash>,
) -> io::Result<()> {
    let prev = Manifest::load(dir).unwrap_or_default();
    let (genesis, verified) = match genesis_hash.map(|g| g.to_string()) {
        Some(g) => {
            let same = prev.genesis_hash.as_deref() == Some(g.as_str());
            (Some(g), prev.verified_height.filter(|_| same))
        }
        None => (prev.genesis_hash, prev.verified_height),
    };
    save(
        dir,
        latest_height,
        &latest_hash.to_string(),
        genesis.as_deref(),
        verified,
    )
}

//! Maintainer set persistence — caches the chain-derived MaintainerSet.
//!
//! The MaintainerSet is derived deterministically from the blockchain,
//! but re-deriving from genesis on every restart is wasteful. This module
//! caches the derived set with the height it was derived at, so on restart
//! we only need to replay from `last_derived_height` to chain tip.
//!
//! At 150K nodes this matters: startup time stays constant regardless of
//! chain length because we only replay the delta.

use std::path::{Path, PathBuf};

use doli_core::maintainer::MaintainerSet;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::maintainer_wellformed::validate_persisted_set;
use crate::StorageError;

const MAINTAINER_STATE_FILE: &str = "maintainer_state.bin";

/// Staging name for the atomic [`MaintainerState::save`]. Lives in the same directory
/// as the target so `rename` stays within one filesystem.
const MAINTAINER_STATE_TMP_FILE: &str = "maintainer_state.bin.tmp";

/// File magic. A bare integer version tag CANNOT work here: the pre-INC-I-172 file
/// starts with bincode's `u64` length prefix for `set.members`, so a 4-byte tag read
/// off the front of a legacy file yields the MEMBER COUNT. A real file from a live
/// node begins `05 00 00 00 00 00 00 00` (5 members) and would be reported as
/// "format version 5"; a 1-member legacy file would be misread as "version 1" and
/// then misparsed field by field. The magic removes the aliasing: no legacy file can
/// begin with these bytes, because doing so would require a `set.members` length of
/// 1,414,745,412 (`0x54534D44`, the little-endian reading of `DMST`) — about 1.41
/// billion members, against a real maintainer set of 5.
const MAGIC: [u8; 4] = *b"DMST";

/// Width of the version tag that follows [`MAGIC`] (u32, little-endian).
const VERSION_TAG_LEN: usize = 4;

/// Total header width: `MAGIC || VERSION`.
const HEADER_LEN: usize = MAGIC.len() + VERSION_TAG_LEN;

/// On-disk format version of [`MaintainerState`] (INC-I-172 F5).
///
/// Written into the file HEADER, immediately after [`MAGIC`], so the decoder can read
/// it before committing to any interpretation of the bytes that follow. Bump this
/// whenever the persisted BODY shape changes; an unknown value is a loud, defined
/// failure, never a silent default.
///
/// This is a NODE-LOCAL file version. It is not the peer handshake
/// (`CURRENT_PROTOCOL_VERSION`) and not the epoch-state format
/// (`EPOCH_STATE_FORMAT_VERSION`) — the file is never gossiped and never hashed,
/// so bumping it cannot fork the chain.
pub const MAINTAINER_STATE_VERSION: u32 = 1;

/// The serialized BODY of the file — byte-for-byte the pre-INC-I-172 (legacy) shape.
///
/// Keeping the body identical to the legacy encoding is what makes the migration
/// lossless and trivial: the ONLY difference between a legacy file and a current one
/// is the 8-byte header. The same decoder therefore reads both, and a migrated set is
/// preserved bit-for-bit.
#[derive(Serialize, Deserialize)]
struct MaintainerStateBody {
    set: MaintainerSet,
    last_derived_height: u64,
}

/// Cached maintainer set with derivation height.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaintainerState {
    /// On-disk format version this value was read from, and the one [`save`] stamps.
    /// It lives in the file HEADER, not in the serialized body — see [`MAGIC`].
    ///
    /// [`save`]: MaintainerState::save
    pub version: u32,
    /// The cached maintainer set
    pub set: MaintainerSet,
    /// Block height at which this set was derived
    pub last_derived_height: u64,
}

impl Default for MaintainerState {
    fn default() -> Self {
        Self {
            version: MAINTAINER_STATE_VERSION,
            set: MaintainerSet::new(),
            last_derived_height: 0,
        }
    }
}

impl MaintainerState {
    /// Load cached state from disk.
    ///
    /// FAIL-CLOSED (INC-I-172 F5). This value is the node's release-verification
    /// trust root. Degrading a file we cannot read to `MaintainerState::default()`
    /// yields an empty set with threshold 0, which re-arms the compiled bootstrap
    /// keys (FM-06) and makes a zero-signature `AddMaintainer` vacuously acceptable
    /// (FM-02) — fleet-wide and simultaneously, on any format change.
    ///
    /// The security property is "never SILENTLY become an EMPTY root", not "refuse to
    /// read old files". Refusing to boot on a legacy file is not a migration: it is a
    /// fleet outage delivered through the auto-updater itself (the INC-I-153 class).
    /// So the four branches are:
    ///
    /// | file state                          | outcome                                  |
    /// |-------------------------------------|------------------------------------------|
    /// | MISSING                             | `Ok(default())` — a fresh node is legitimate |
    /// | magic present, version known        | `Ok(decoded)`, or `Err` if the body is bad |
    /// | magic present, version UNKNOWN      | `Err(UnsupportedFormatVersion)` — fail closed |
    /// | no magic (legacy, pre-INC-I-172)    | MIGRATE: decode, warn, re-save, `Ok(preserved)` |
    /// | no magic and undecodable (corrupt)  | `Err(Serialization)` — never a default     |
    ///
    /// INC-I-172 M2 / AUDIT-P1-019: a body that DECODES is still not authority. Both
    /// decoding branches now run `maintainer_wellformed::validate_persisted_set`, which
    /// refuses a set no live
    /// path can produce — duplicate members, more members than `MAX_MAINTAINERS`, or a
    /// threshold that does not match its own member count. That refusal happens BEFORE
    /// the legacy branch's eager re-save, so a rejected file is left exactly as found.
    pub fn load(data_dir: &Path) -> Result<Self, StorageError> {
        let path = Self::file_path(data_dir);
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = std::fs::read(&path)?;

        if data.len() >= HEADER_LEN && data[..MAGIC.len()] == MAGIC {
            Self::decode_current(&path, &data)
        } else {
            // No magic ⇒ this file was written before INC-I-172. Migrate it; do not
            // reinterpret it field-by-field against the current header.
            Self::migrate_legacy(data_dir, &path, &data)
        }
    }

    /// Decode a file that carries [`MAGIC`]. Unknown version ⇒ loud, fail-closed error.
    ///
    /// This branch is forward-only: no file in the wild carries the magic today, so it
    /// can never brick the current fleet.
    fn decode_current(path: &Path, data: &[u8]) -> Result<Self, StorageError> {
        let found = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        if found != MAINTAINER_STATE_VERSION {
            return Err(StorageError::UnsupportedFormatVersion {
                file: path.display().to_string(),
                found: found.to_string(),
                expected: MAINTAINER_STATE_VERSION,
            });
        }
        let body: MaintainerStateBody = bincode::deserialize(&data[HEADER_LEN..]).map_err(|e| {
            StorageError::Serialization(format!(
                "{}: could not decode the maintainer state body (format version {}): {}. \
                     Refusing to load rather than degrade the release-verification trust root \
                     to an empty set.",
                path.display(),
                MAINTAINER_STATE_VERSION,
                e
            ))
        })?;
        Self::from_body(path, body)
    }

    /// Migrate a pre-INC-I-172 (unversioned) file to the current layout.
    ///
    /// The legacy body schema IS the current body schema, so the set is preserved
    /// bit-for-bit; only the 8-byte header is added. The re-save is eager so the
    /// migration happens once, but a re-save FAILURE (read-only data dir, full disk)
    /// is a warning, not an error: the in-memory value is already correct, and a write
    /// failure must never brick a node.
    fn migrate_legacy(data_dir: &Path, path: &Path, data: &[u8]) -> Result<Self, StorageError> {
        let body: MaintainerStateBody = bincode::deserialize(data).map_err(|e| {
            StorageError::Serialization(format!(
                "{}: could not decode the maintainer state. The file carries no \"{}\" magic, so \
                 it was read as a pre-INC-I-172 (unversioned) file, and that failed too: {}. \
                 Refusing to load rather than degrade the release-verification trust root to an \
                 empty set — inspect or remove this file deliberately.",
                path.display(),
                String::from_utf8_lossy(&MAGIC),
                e
            ))
        })?;
        // Validate BEFORE the eager re-save: a refused file must be left exactly as
        // found, so an operator can inspect the bytes that were rejected.
        let state = Self::from_body(path, body)?;

        warn!(
            "Migrating legacy maintainer_state.bin to format version {}: {} ({} member(s), \
             threshold {}, derived at height {}). The maintainer set is preserved exactly; only \
             the file header changes.",
            MAINTAINER_STATE_VERSION,
            path.display(),
            state.set.members.len(),
            state.set.threshold,
            state.last_derived_height
        );

        if let Err(e) = state.save(data_dir) {
            warn!(
                "Could not re-save {} in format version {}: {}. The maintainer set held in memory \
                 is correct and the node continues; the migration is retried on the next start.",
                path.display(),
                MAINTAINER_STATE_VERSION,
                e
            );
        }
        Ok(state)
    }

    fn from_body(path: &Path, body: MaintainerStateBody) -> Result<Self, StorageError> {
        validate_persisted_set(path, &body.set)?;
        Ok(Self {
            version: MAINTAINER_STATE_VERSION,
            set: body.set,
            last_derived_height: body.last_derived_height,
        })
    }

    /// Save cached state to disk as `MAGIC || VERSION || bincode(body)`.
    ///
    /// Always stamps `MAINTAINER_STATE_VERSION`, so the header describes the encoder
    /// that actually wrote the file.
    ///
    /// ATOMIC (INC-I-172 F4): temp file in the SAME directory → `sync_all` → `rename`.
    /// A bare `fs::write` is create + TRUNCATE + `write_all`, so a crash or power loss
    /// between the truncate and the write leaves a zero-byte file — and a file that
    /// decodes as neither layout is FATAL at startup by design (see [`load`]). The
    /// migration performs exactly this write on every node's first boot after the
    /// upgrade, inside the rolling-deploy window, on a restart the auto-updater itself
    /// triggers: the INC-I-153 failure class arriving through a different door. With
    /// the rename, the target is only ever the old complete file or the new complete
    /// file. `install_binary` already uses this pattern for the same reason.
    ///
    /// [`load`]: MaintainerState::load
    pub fn save(&self, data_dir: &Path) -> Result<(), StorageError> {
        use std::io::Write;

        let path = Self::file_path(data_dir);
        let body = MaintainerStateBody {
            set: self.set.clone(),
            last_derived_height: self.last_derived_height,
        };
        let encoded =
            bincode::serialize(&body).map_err(|e| StorageError::Serialization(e.to_string()))?;

        let mut out = Vec::with_capacity(HEADER_LEN + encoded.len());
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&MAINTAINER_STATE_VERSION.to_le_bytes());
        out.extend_from_slice(&encoded);

        // Same directory as the target: `rename` is only atomic within a filesystem.
        let tmp = data_dir.join(MAINTAINER_STATE_TMP_FILE);
        let write_result = (|| -> std::io::Result<()> {
            let mut f = create_owner_only(&tmp)?;
            f.write_all(&out)?;
            // Durability before visibility: without the fsync, the rename can be
            // ordered ahead of the data on a crash and publish an empty inode.
            f.sync_all()?;
            Ok(())
        })();
        if let Err(e) = write_result {
            // Leave the existing target untouched and take the temp file with us.
            let _ = std::fs::remove_file(&tmp);
            return Err(e.into());
        }
        if let Err(e) = std::fs::rename(&tmp, &path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.into());
        }
        Ok(())
    }

    /// Update the cached set and persist.
    pub fn update(
        &mut self,
        set: MaintainerSet,
        height: u64,
        data_dir: &Path,
    ) -> Result<(), StorageError> {
        self.set = set;
        self.last_derived_height = height;
        self.save(data_dir)
    }

    fn file_path(data_dir: &Path) -> PathBuf {
        data_dir.join(MAINTAINER_STATE_FILE)
    }
}

/// Create `path` for writing with OWNER-ONLY permissions (0600 on Unix).
///
/// INC-I-172 M1, AUDIT-P2-014. This file decides which keys may authorise a ROOT binary
/// install on this host, and `File::create` applies the process umask — commonly 022,
/// which yields a world-READABLE 0644, and on a lax umask a group- or world-WRITABLE
/// file. The mode is set at creation, on the TEMP file, before any bytes are written, so
/// there is no window in which the content exists at wider permissions; `rename` carries
/// the mode onto the target.
///
/// This is a floor, not a fix. It bounds WHO can read or rewrite the trust root to the
/// node user (and root). It does NOT make the file authentic: anything that can write as
/// the node user still controls the install root, because the file carries no MAC and is
/// never reconciled against chain state. That is the remaining part of AUDIT-P2-016.
fn create_owner_only(path: &Path) -> std::io::Result<std::fs::File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_maintainer_state_default() {
        let state = MaintainerState::default();
        assert_eq!(state.last_derived_height, 0);
        assert!(state.set.members.is_empty());
    }

    #[test]
    fn test_maintainer_state_save_load() {
        let dir = tempfile::tempdir().unwrap();

        let state = MaintainerState {
            last_derived_height: 42,
            ..Default::default()
        };
        state.save(dir.path()).unwrap();

        let loaded = MaintainerState::load(dir.path()).unwrap();
        assert_eq!(loaded.last_derived_height, 42);
    }

    #[test]
    fn test_maintainer_state_load_missing() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = MaintainerState::load(dir.path()).unwrap();
        assert_eq!(loaded.last_derived_height, 0);
    }

    #[test]
    fn test_maintainer_state_update() {
        let dir = tempfile::tempdir().unwrap();

        let mut state = MaintainerState::default();
        let set = MaintainerSet::new();
        state.update(set, 100, dir.path()).unwrap();

        let loaded = MaintainerState::load(dir.path()).unwrap();
        assert_eq!(loaded.last_derived_height, 100);
    }

    /// The header is what makes a legacy file distinguishable, so its exact shape is
    /// pinned here: `MAGIC` then the version as a little-endian `u32`, then the body.
    #[test]
    fn test_saved_file_carries_magic_then_version() {
        let dir = tempfile::tempdir().unwrap();
        MaintainerState::default().save(dir.path()).unwrap();

        let bytes = std::fs::read(dir.path().join(MAINTAINER_STATE_FILE)).unwrap();
        assert!(bytes.len() > HEADER_LEN, "the body must follow the header");
        assert_eq!(&bytes[..MAGIC.len()], &MAGIC, "file must start with MAGIC");
        assert_eq!(
            u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            MAINTAINER_STATE_VERSION,
            "the version tag follows the magic, little-endian"
        );
    }

    /// The body encoding must stay byte-identical to the legacy one, because that is
    /// what makes the migration lossless. If this fails, the migration path is decoding
    /// legacy bytes with a schema that no longer matches them.
    #[test]
    fn test_body_encoding_is_identical_to_the_legacy_layout() {
        let dir = tempfile::tempdir().unwrap();
        let set = MaintainerSet::with_members(
            vec![
                crypto::PrivateKey::from_bytes([3u8; 32]).public_key(),
                crypto::PrivateKey::from_bytes([4u8; 32]).public_key(),
            ],
            7,
        );
        let state = MaintainerState {
            version: MAINTAINER_STATE_VERSION,
            set: set.clone(),
            last_derived_height: 11,
        };
        state.save(dir.path()).unwrap();

        let written = std::fs::read(dir.path().join(MAINTAINER_STATE_FILE)).unwrap();
        let legacy = bincode::serialize(&MaintainerStateBody {
            set,
            last_derived_height: 11,
        })
        .unwrap();
        assert_eq!(
            &written[HEADER_LEN..],
            legacy.as_slice(),
            "the body after the header must be exactly the pre-INC-I-172 encoding"
        );
    }
}

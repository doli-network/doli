// INC-I-172 M1 security audit, AUDIT-P2-014 — `maintainer_state.bin` must be created
// OWNER-ONLY (0600), not at whatever the process umask happens to allow.
// REQ-172-011 (Must).
//
// THE DEFECT this narrows: M1 makes this file the sole decider of which keys may
// authorise a ROOT binary install on this host, and root-run commands (`doli upgrade`,
// `doli-node upgrade`, `doli-node update apply`) read it. `File::create` applies the
// process umask — commonly 022, which yields a world-READABLE 0644, and on a lax umask a
// group- or world-WRITABLE file. A file that anything on the box can rewrite is a file
// that anything on the box can use to choose the fleet's install authority.
//
// HONEST SCOPE, stated rather than implied: 0600 is a FLOOR, not the fix. It bounds WHO
// can read or rewrite the trust root to the node user and root. It does NOT make the file
// authentic — anything running AS the node user still controls it, because the file
// carries no MAC and is never reconciled against chain state. That residual is the rest
// of AUDIT-P2-016 and is recorded, not closed, by this test.
//
// ============================================================================
// OUTPUT CONTRACT
// ============================================================================
// Function under test:
//   `storage::MaintainerState::save(&self, data_dir: &Path) -> Result<(), StorageError>`
//
// ENUMERATION OF OBSERVABLE OUTPUTS.
//   - return value     : O1 Result discriminant.
//   - mutable params   : NONE (`&self`, `&Path`).
//   - persistent store : O2 the file CONTENT (must still round-trip — a permission fix
//                        that corrupted the file would be worse than the finding);
//                        O3 the file MODE, which is the finding;
//                        O4 the staging file must not survive.
//   - side channel     : none.
//
// CODE PATHS:
//   P1: first save (no existing target)   -> 0600
//   P2: overwrite of an existing target   -> still 0600 (the atomic rename replaces the
//                                            inode, so the mode comes from the TEMP file,
//                                            not from the old target — this is the path
//                                            a mode set on the wrong file would miss)
//
// INPUT PARTITIONS: the process umask is the hidden input. The test sets a PERMISSIVE
// umask (0o000) for the duration, because a restrictive ambient umask would mask the
// defect entirely and the test would pass against `File::create`. Restored afterwards.
//
// Unix only: `PermissionsExt` has no Windows meaning and the node does not target it.
// ============================================================================

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use doli_core::maintainer::MaintainerSet;
use storage::MaintainerState;

fn pubkey(seed: u8) -> crypto::PublicKey {
    crypto::PrivateKey::from_bytes([seed; 32]).public_key()
}

fn mode_of(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// Run `f` with the process umask forced to `mask`, restoring it afterwards.
///
/// The umask is process-global, so this test must not run beside another that creates
/// files and asserts on their modes. Nothing else in this crate's test suite does.
fn with_umask<R>(mask: libc::mode_t, f: impl FnOnce() -> R) -> R {
    // SAFETY: `umask` is always successful and returns the previous value.
    let previous = unsafe { libc::umask(mask) };
    let out = f();
    unsafe { libc::umask(previous) };
    out
}

/// AUDIT-P2-014. RED before the fix.
/// Acceptance: the trust-root file is owner-only even under a fully permissive umask.
/// [P1, P2 -> O1, O2, O3, O4]
#[test]
fn audit_p2_014_maintainer_state_is_written_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("maintainer_state.bin");

    with_umask(0o000, || {
        // P1 — first save.
        let mut state = MaintainerState::default();
        state
            .update(
                MaintainerSet::with_members(vec![pubkey(1), pubkey(2), pubkey(3)], 10),
                4242,
                dir.path(),
            )
            .expect("the first save must succeed");

        assert_eq!(
            mode_of(&path),
            0o600,
            "maintainer_state.bin was created at {:o} under a permissive umask. This file \
             decides which keys may authorise a ROOT binary install; `File::create` \
             inherits the umask, so on a default 022 it is world-readable and on a lax one \
             it is world-WRITABLE (AUDIT-P2-014).",
            mode_of(&path)
        );

        // P2 — overwrite. The atomic save renames a TEMP file over the target, so the
        // surviving inode is the temp file's. A mode applied to the wrong file, or applied
        // after the rename, would pass P1 and fail here.
        state
            .update(
                MaintainerSet::with_members(vec![pubkey(4), pubkey(5)], 20),
                4343,
                dir.path(),
            )
            .expect("the overwrite must succeed");

        assert_eq!(
            mode_of(&path),
            0o600,
            "the atomic rename replaced maintainer_state.bin with an inode at {:o}; the \
             mode must come from the staging file, which is what the rename publishes",
            mode_of(&path)
        );
    });

    // O4 — no staging file left behind.
    assert!(
        !dir.path().join("maintainer_state.bin.tmp").exists(),
        "the staging file must not survive a successful save"
    );

    // O2 — the permission change must not have broken the file.
    let loaded = MaintainerState::load(dir.path()).expect("the file must still load");
    assert_eq!(loaded.last_derived_height, 4343);
    assert_eq!(loaded.set.members.len(), 2);
}

// Test module for `cmd_upgrade.rs`, split into its own file so the parent stays inside
// the 500-line module budget (REQ-215-016). Included via `#[path]`, so `use super::*`
// still resolves to `cmd_upgrade`.

use super::*;

// INC-I-199. Observed live on vm-server: a non-sudo `doli upgrade` reported
// "io error: Permission denied (os error 13)" and then advised that the file was
// damaged and should be restored or removed. Acting on that deletes a healthy
// trust root and drops the host to the compiled keys — the leaked five on any
// binary predating the INC-I-196 cutover.
//
// OUTPUT CONTRACT
//   Under test : `trust_root_load_advice(&StorageError, &Path) -> String`
//   Outputs    : O1 the advice text. No mutable params, no I/O, no side channel.
//   Code paths : P1 Io/PermissionDenied, P2 Io/NotFound, P3 Io/other,
//                P4 non-Io (decode, unsupported version, malformed value)
//   Partitions : the error variant IS the partition.

fn io(kind: std::io::ErrorKind) -> storage::StorageError {
    storage::StorageError::Io(std::io::Error::new(kind, "test"))
}

fn dir() -> &'static Path {
    Path::new("/var/lib/doli/mainnet")
}

/// REQ-199-001 (Must). The regression: EACCES must never be called damage.
/// [P1 -> O1]
#[test]
fn permission_denied_says_sudo_and_never_suggests_deleting_the_file() {
    let msg = trust_root_load_advice(&io(std::io::ErrorKind::PermissionDenied), dir());

    assert!(
        msg.contains("sudo"),
        "EACCES must tell the operator to re-run with sudo. Got: {msg}"
    );
    // Assert on the harmful CLAIM, not the word: the corrected text legitimately
    // says "not a damaged file", so a bare contains("damaged") would fail on a
    // correct message.
    assert!(
        !msg.contains("damaged rather than merely old"),
        "a permission error must NOT be reported as a damaged file: {msg}"
    );
    assert!(
        !msg.contains("remove it deliberately") && !msg.contains("Restore it from a backup"),
        "advising deletion here destroys a healthy trust root and drops the host to \
         the compiled (pre-INC-I-196: leaked) keys: {msg}"
    );
}

/// REQ-199-002 (Must). No I/O failure implies bad content.
/// [P2, P3 -> O1]
#[test]
fn other_io_errors_do_not_advise_deletion_either() {
    for kind in [
        std::io::ErrorKind::NotFound,
        std::io::ErrorKind::Interrupted,
        std::io::ErrorKind::UnexpectedEof,
    ] {
        let msg = trust_root_load_advice(&io(kind), dir());
        assert!(
            !msg.contains("remove it deliberately"),
            "{kind:?}: an I/O failure is not evidence the contents are bad: {msg}"
        );
        assert!(
            !msg.contains("damaged rather than merely old"),
            "{kind:?}: must not claim damage from an I/O error: {msg}"
        );
    }
}

/// REQ-199-003 (Must). GREEN-lock: a genuine CONTENT failure must keep the original
/// restore-or-remove guidance — the fix must not blunt real corruption reporting.
/// [P4 -> O1]
#[test]
fn a_decode_failure_still_gets_the_restore_or_remove_guidance() {
    let msg = trust_root_load_advice(
        &storage::StorageError::Serialization("bad body".into()),
        dir(),
    );
    assert!(
        msg.contains("damaged") && msg.contains("remove it deliberately"),
        "a real decode failure must still tell the operator how to recover: {msg}"
    );
}

/// REQ-199-004 (Must). Every path names the file and keeps the fail-closed rationale,
/// so the message is actionable regardless of which branch produced it.
/// [P1, P2, P3, P4 -> O1]
#[test]
fn every_branch_names_the_path_and_states_why_it_refuses() {
    let errs = [
        io(std::io::ErrorKind::PermissionDenied),
        io(std::io::ErrorKind::NotFound),
        io(std::io::ErrorKind::Other),
        storage::StorageError::Serialization("bad".into()),
    ];
    for e in &errs {
        let msg = trust_root_load_advice(e, dir());
        assert!(
            msg.contains("/var/lib/doli/mainnet"),
            "must name the path: {msg}"
        );
        assert!(
            msg.contains("falling back to the compiled bootstrap keys"),
            "must keep the fail-closed rationale: {msg}"
        );
    }
}

// INC-I-235 M2 — the archive and relay roles imply `--no-snap-sync`.
//
// covers: config.rs, run.rs, cli.rs
//
// OUTPUT CONTRACT: fn snap_sync_forbidden(no_snap_sync: bool, relay_server: bool, archive_to_set: bool) -> bool
//   O1: mutable params — none (three bools by value).
//   O2: receiver mutation — none (free function).
//   O3: return — false only when all three inputs are false; true otherwise.
// PATHS: P1: plain node (no flag, no role) -> false; P2: any flag or role set -> true
// INPUT PARTITIONS:
//   P1a: (F,F,F) — a plain node keeps snap sync (non-archive behaviour unchanged)
//   P2a: archive role only (F,F,T)
//   P2b: relay role only (F,T,F)
//   P2c: explicit operator flag only (T,F,F)
//   P2d: two or three set (T,T,F), (T,F,T), (F,T,T), (T,T,T) — OR, never a parity
// MATRIX: 1 output (O3) x 8 input combinations = 8 cells, all in the truth-table test;
//   P1a, P2a, P2b, P2c also have a named test each.

use super::snap_sync_forbidden;

// REQ-ARCHSYNC-001 — Decision: a failure reveals the fix disabled snap sync for every node, not only for the archive and relay roles.
#[test]
fn plain_node_keeps_snap_sync() {
    assert!(!snap_sync_forbidden(false, false, false));
}

// REQ-ARCHSYNC-001 — Decision: a failure reveals a node started with --archive-to can still snap and leave a block-history hole.
#[test]
fn archive_role_forbids_snap_sync() {
    assert!(snap_sync_forbidden(false, false, true));
}

// REQ-ARCHSYNC-001 — Decision: a failure reveals a node started with --relay-server can still snap and serve a block-history hole.
#[test]
fn relay_role_forbids_snap_sync() {
    assert!(snap_sync_forbidden(false, true, false));
}

// REQ-ARCHSYNC-001 — Decision: a failure reveals the role wiring dropped the explicit --no-snap-sync operator flag.
#[test]
fn explicit_flag_forbids_snap_sync() {
    assert!(snap_sync_forbidden(true, false, false));
}

// REQ-ARCHSYNC-001 — Decision: a failure reveals a combination of flags (for example a parity bug) re-enables snap on an archive or relay node.
#[test]
fn snap_sync_forbidden_truth_table() {
    // (no_snap_sync, relay_server, archive_to_set) -> forbidden
    const TABLE: [((bool, bool, bool), bool); 8] = [
        ((false, false, false), false),
        ((false, false, true), true),
        ((false, true, false), true),
        ((false, true, true), true),
        ((true, false, false), true),
        ((true, false, true), true),
        ((true, true, false), true),
        ((true, true, true), true),
    ];
    for ((no_snap_sync, relay_server, archive_to_set), expected) in TABLE {
        assert_eq!(
            snap_sync_forbidden(no_snap_sync, relay_server, archive_to_set),
            expected,
            "no_snap_sync={no_snap_sync} relay_server={relay_server} \
             archive_to_set={archive_to_set}"
        );
    }
}

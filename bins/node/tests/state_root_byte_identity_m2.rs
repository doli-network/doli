//! State-Root Lazy Tier-0 — M2 byte-identity-after-deletion lock (RUN 460).
//!
//! M2 DELETES the eager per-block compute (`apply_block/state_update.rs`
//! Phase 2/3). This file locks that the deletion changes only WHEN the root is
//! computed, NEVER the bytes it produces (spec: "Formula, root value, wire
//! format: BYTE-IDENTICAL at every height").
//!
//! Requirements:
//!   REQ-SROOT-001/007 (Must) — at a fixed height, ALL four root-producing
//!   paths agree byte-for-byte with legacy `storage::compute_state_root`:
//!     (a) the SERVED root                 — `Node::serve_state_root()`
//!     (b) the direct legacy compute       — `storage::compute_state_root(cs,utxo,ps)`
//!     (c) the snap-sync BUILD root        — `StateSnapshot::create(..).state_root` (snapshot.rs:242)
//!     (d) the snap-sync INSTALL root      — `compute_state_root_from_bytes(..)` on the
//!                                            snapshot's serialized component bytes
//!
//! (b) is the reference the deleted eager compute used to cache; (a) is the
//! post-M1 serve seam; (c)/(d) are the two snap-sync seams (build on the source,
//! verify on the installer). If any diverges, snap-sync quorum breaks. The
//! `storage::compute_state_root` FORMULA itself is locked separately by
//! `crates/storage/tests/state_root_golden_identity_test.rs` (unchanged by M2);
//! this file locks that the node/snapshot paths route to that exact value.
//
// OUTPUT CONTRACT: cross-path root identity at a fixed Node fixture (genesis tip).
// O1: served_root  = serve_state_root().state_root
// O2: legacy_root  = storage::compute_state_root(cs, utxo, ps)
// O3: build_root   = StateSnapshot::create(cs, utxo, ps).state_root
// O4: install_root = compute_state_root_from_bytes(cs_bytes, utxo_bytes, ps_bytes)
// PATHS: P1 — one representative committed state (new_for_test genesis tip).
// MATRIX: O1×P1 == O2×P1 == O3×P1 == O4×P1  (all four byte-identical).
//   Covered by test_all_state_root_paths_byte_identical (the AND of all pairs).
// INPUT PARTITIONS: single class — a valid committed Node state. The formula's
//   per-component sensitivity/order-independence is out of scope here (locked in
//   the storage golden-identity test); this file's job is exclusively the
//   cross-path agreement that the eager-compute deletion must preserve.

use crypto::{Hash, KeyPair};
use doli_node::node::Node;
use network::protocols::SyncResponse;
use tempfile::TempDir;

async fn make_node(n_producers: usize) -> (Node, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers)
        .await
        .expect("Node::new_for_test failed");
    (node, temp)
}

fn served_root(resp: SyncResponse) -> Hash {
    match resp {
        SyncResponse::StateRoot { state_root, .. } => state_root,
        other => panic!("expected SyncResponse::StateRoot, got {:?}", other),
    }
}

/// REQ-SROOT-001/007 (Must) — the four root-producing paths are byte-identical.
/// Locks that removing the eager per-block compute did not change the root bytes:
/// the served root, the legacy direct compute, the snap-build root, and the
/// snap-install root all agree at a fixed committed state.
#[tokio::test]
async fn test_all_state_root_paths_byte_identical() {
    let (node, _tmp) = make_node(5).await;

    // (a) SERVED root — the post-M1 lazy serve seam.
    let served = served_root(node.serve_state_root().await);

    // Snapshot the committed 3-state under read guards, derive the other three
    // roots, then drop the guards. Block-apply is serialized by the event loop,
    // so this is a consistent snapshot.
    let (legacy, build_root, install_root) = {
        let cs = node.chain_state.read().await;
        let utxo = node.utxo_set.read().await;
        let ps = node.producer_set.read().await;

        // (b) legacy direct compute — the value the deleted eager path cached.
        let legacy = storage::compute_state_root(&cs, &utxo, &ps).expect("compute_state_root");

        // (c) snap-sync BUILD root (snapshot.rs:242).
        let snap = storage::StateSnapshot::create(&cs, &utxo, &ps).expect("StateSnapshot::create");
        let build_root = snap.state_root;

        // (d) snap-sync INSTALL root — recomputed from the serialized component
        // bytes the installer receives over the wire.
        let install_root = storage::compute_state_root_from_bytes(
            &snap.chain_state_bytes,
            &snap.utxo_set_bytes,
            &snap.producer_set_bytes,
        )
        .expect("compute_state_root_from_bytes");

        (legacy, build_root, install_root)
    };

    assert_eq!(
        served, legacy,
        "served root must equal legacy compute_state_root (byte-identical)"
    );
    assert_eq!(
        legacy, build_root,
        "snap-sync BUILD root must equal legacy compute_state_root"
    );
    assert_eq!(
        build_root, install_root,
        "snap-sync INSTALL root (from bytes) must equal the BUILD root"
    );
    // Transitive full-loop assertion (explicit, so a failure names the served path).
    assert_eq!(
        served, install_root,
        "served root must equal the snap-install root — all four paths byte-identical"
    );
}

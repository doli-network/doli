//! B3 M1 / R-2 (INC-I-260) — a block whose valid AddMaintainer or
//! RemoveMaintainer runs and whose later transaction then fails leaves the
//! maintainer set, in memory and in `maintainer_state.bin`, exactly as on a
//! control node that never saw the block. One test per intake path.
//!
// covers: slash_redeliver.rs slash_reported.rs producer_undo.rs apply_block/mod.rs governance.rs maintainer_rewind/mod.rs maintainer_rewind/commit.rs
//
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()> (and its callers)
//   O1 return: Err naming the ghost transfer (direct); callers keep the tip / break / apply the next sibling
//   O2 in-memory maintainer root: members, threshold, last_updated, last_derived_height, set digest
//   O3 persisted maintainer_state.bin bytes
//   O4 chain tip (unchanged by F; moved by the valid sibling on the drain path)
// PATHS: P-GOSSIP handle_new_block(F) | P-SYNC pending sync blocks applied by run_periodic_tasks
//   | P-DRAIN drain_cached_children with F and a valid sibling | P-DIRECT apply_block(F, Light)
//   | P-OK the same AddMaintainer in a block that applies
// INPUT PARTITIONS: F = [coinbase, maintainer tx, Transfer of a missing outpoint]
//   maintainer tx: AddMaintainer (4 -> 5 members) | RemoveMaintainer (4 -> 3 members)
//     | AddMaintainer with one signature (below threshold 3: the root never moves)
//   control: twin rig that never saw F (same keys, same seeded root)
//   O5 maintainer_state.bin inode (a save renames a new file in; no save keeps it)
// MATRIX: O2-O4 x {P-GOSSIP, P-SYNC, P-DRAIN, P-DIRECT} x Add + O1-O4 x P-DIRECT x Remove
//   + O1-O3 x P-OK x Add + O1-O3,O5 x P-DIRECT x unverified Add = 7 tests;
//   each failing-path test also proves the tx valid on the control

use crypto::KeyPair;
use doli_core::transaction::Transaction;
use network::PeerId;
use storage::MaintainerState;
use tracing_subscriber::layer::SubscriberExt;

use super::b3_m1_maintainer_fx::*;
use super::inc_i_182_m1_common::Logs;
use super::inc_i_255_m2_common::Rig;
use super::inc_i_255_m4_common::err_text;
use super::inc_i_256_m1_common::{block_at, tip};

const MID_EPOCH_TIP: u64 = 5;

/// NON-VACUITY: `gov` applied in a clean block on the control changes its root.
async fn assert_gov_applies_on(c: &mut Rig, gov: Transaction, what: &str) {
    let before = mobs(&c.node).await;
    let (h, slot, prev) = c.best().await;
    let clean = block_at(c, h + 1, slot + 1, prev, vec![gov]);
    let r = c.apply(clean).await;
    assert!(
        r.is_ok(),
        "{what}: NON-VACUITY: a clean block with the same maintainer tx applies; got {}",
        err_text(&r)
    );
    assert_ne!(
        mobs(&c.node).await.members,
        before.members,
        "{what}: NON-VACUITY: the maintainer tx F carried is valid and changes the root"
    );
}

// REQ-B3-002 — Decision: a failure means a gossiped block that fails after its valid AddMaintainer leaves an extra trust-root key in memory and on disk on this node only.
#[tokio::test]
async fn b3_m1_gossip_failed_apply_leaves_no_maintainer_residue() {
    let (mut s, mut c, members) = maintained_twins(MID_EPOCH_TIP).await;
    let want = mobs(&c.node).await;
    let before = tip(&s).await;
    let (h, slot, prev) = s.best().await;
    let newcomer = KeyPair::generate();
    let gov = add_tx(&s, &members, &newcomer, h + 1);
    let f = failing_gov_block(&s, h + 1, slot + 1, prev, gov);
    s.node.floor_fallback_window = true;
    let r = s.node.handle_new_block(f, PeerId::random()).await;
    let got = mobs(&s.node).await;
    assert!(
        r.is_ok(),
        "gossip intake returns Ok on a rejected block: {}",
        err_text(&r)
    );
    assert_eq!(
        tip(&s).await,
        before,
        "PRECONDITION: F must not move the tip"
    );
    assert_mobs(&got, &want, "handle_new_block(F) with AddMaintainer");
    let again = add_tx(&c, &members, &newcomer, h + 1);
    assert_gov_applies_on(&mut c, again, "gossip").await;
}

// REQ-B3-002 — Decision: a failure means a block fetched by header-first sync and applied by the periodic loop leaves maintainer residue that this node then serves to peers.
#[tokio::test]
async fn b3_m1_sync_failed_apply_leaves_no_maintainer_residue() {
    let (mut s, mut c, members) = maintained_twins(MID_EPOCH_TIP).await;
    let want = mobs(&c.node).await;
    let before = tip(&s).await;
    let (h, slot, prev) = s.best().await;
    let newcomer = KeyPair::generate();
    let f = failing_gov_block(
        &s,
        h + 1,
        slot + 1,
        prev,
        add_tx(&s, &members, &newcomer, h + 1),
    );
    let g = block_at(&s, h + 2, slot + 2, f.hash(), Vec::new());
    let k = block_at(&s, h + 3, slot + 3, g.hash(), Vec::new());
    let chain = vec![f, g, k.clone()];
    let peer = PeerId::random();
    s.node
        .sync_manager
        .write()
        .await
        .add_peer(peer, h + 3, k.hash(), k.header.slot);
    let served = serve_sync(&s, peer, &chain).await;
    let logs = Logs::default();
    let periodic = {
        let _g =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
        s.node.run_periodic_tasks().await
    };
    let got = mobs(&s.node).await;
    assert!(
        served.contains(&"headers") && served.contains(&"bodies"),
        "PRECONDITION: header-first sync fetched F's header and body; served {served:?}"
    );
    assert!(
        !logs
            .mentioning("Failed to apply pending sync block")
            .is_empty(),
        "PRECONDITION: the periodic pending-sync loop applied F and saw it fail"
    );
    assert!(periodic.is_ok(), "run_periodic_tasks: {:?}", periodic.err());
    assert_eq!(
        tip(&s).await,
        before,
        "PRECONDITION: F must not move the tip"
    );
    assert_mobs(
        &got,
        &want,
        "periodic pending-sync apply of F with AddMaintainer",
    );
    let again = add_tx(&c, &members, &newcomer, h + 1);
    assert_gov_applies_on(&mut c, again, "sync").await;
}

// REQ-B3-002 — Decision: a failure means the fork-cache drain commits a valid sibling on top of a failed sibling's maintainer residue.
#[tokio::test]
async fn b3_m1_fork_cache_failed_apply_leaves_no_maintainer_residue() {
    let (mut s, mut c, members) = maintained_twins(MID_EPOCH_TIP - 1).await;
    let (h, slot, prev) = s.best().await;
    let newcomer = KeyPair::generate();
    let t = block_at(&s, h + 1, slot + 1, prev, Vec::new());
    let gov = add_tx(&s, &members, &newcomer, h + 2);
    let f = failing_gov_block(&s, h + 2, slot + 2, t.hash(), gov);
    let b2 = block_at(&s, h + 2, slot + 3, t.hash(), Vec::new());
    c.apply(t.clone()).await.expect("control applies T");
    c.apply(b2.clone()).await.expect("control applies B2");
    {
        let mut cache = s.node.fork_block_cache.write().await;
        cache.insert(f.hash(), f.clone());
        cache.insert(b2.hash(), b2.clone());
    }
    s.node.floor_fallback_window = true;
    let r = s.node.handle_new_block(t, PeerId::random()).await;
    let got = mobs(&s.node).await;
    let want = mobs(&c.node).await;
    assert!(r.is_ok(), "handle_new_block(T): {}", err_text(&r));
    assert_eq!(
        tip(&s).await,
        (h + 2, b2.hash()),
        "PRECONDITION: the drain tried F (lower slot) and then applied B2"
    );
    assert!(
        !s.node.fork_block_cache.read().await.contains_key(&f.hash()),
        "PRECONDITION: the drain consumed F"
    );
    assert_mobs(
        &got,
        &want,
        "drain: F with AddMaintainer failed, then B2 applied",
    );
    let again = add_tx(&c, &members, &newcomer, h + 3);
    assert_gov_applies_on(&mut c, again, "drain").await;
}

// REQ-B3-002 — Decision: a failure means apply_block itself returns Err with the maintainer set and its file mutated, which every caller inherits.
#[tokio::test]
async fn b3_m1_direct_failed_apply_leaves_no_maintainer_residue() {
    let (mut s, mut c, members) = maintained_twins(MID_EPOCH_TIP).await;
    let want = mobs(&c.node).await;
    let before = tip(&s).await;
    let (h, slot, prev) = s.best().await;
    let newcomer = KeyPair::generate();
    let f = failing_gov_block(
        &s,
        h + 1,
        slot + 1,
        prev,
        add_tx(&s, &members, &newcomer, h + 1),
    );
    let ghost = f.transactions[2].hash().to_string();
    let r = s.apply(f).await;
    let got = mobs(&s.node).await;
    let text = err_text(&r);
    assert!(
        r.is_err() && text.contains(&ghost),
        "PRECONDITION: F must fail inside the tx loop at the ghost transfer {ghost}; got {text}"
    );
    assert_eq!(
        tip(&s).await,
        before,
        "PRECONDITION: F must not move the tip"
    );
    assert_mobs(
        &got,
        &want,
        "apply_block(F with AddMaintainer, Light) returned Err",
    );
    let again = add_tx(&c, &members, &newcomer, h + 1);
    assert_gov_applies_on(&mut c, again, "direct add").await;
}

// REQ-B3-002 — Decision: a failure means a failed block that ran a valid RemoveMaintainer leaves a seated key missing from this node's trust root only.
#[tokio::test]
async fn b3_m1_direct_failed_remove_maintainer_leaves_no_residue() {
    let (mut s, mut c, members) = maintained_twins(MID_EPOCH_TIP).await;
    let want = mobs(&c.node).await;
    let before = tip(&s).await;
    let (h, slot, prev) = s.best().await;
    let f = failing_gov_block(&s, h + 1, slot + 1, prev, remove_tx(&s, &members, h + 1));
    let ghost = f.transactions[2].hash().to_string();
    let r = s.apply(f).await;
    let got = mobs(&s.node).await;
    let text = err_text(&r);
    assert!(
        r.is_err() && text.contains(&ghost),
        "PRECONDITION: F must fail inside the tx loop at the ghost transfer {ghost}; got {text}"
    );
    assert_eq!(
        tip(&s).await,
        before,
        "PRECONDITION: F must not move the tip"
    );
    assert!(
        got.members.contains(members[MEMBERS - 1].public_key()),
        "the member F's RemoveMaintainer named must still be seated after F failed"
    );
    assert_mobs(
        &got,
        &want,
        "apply_block(F with RemoveMaintainer, Light) returned Err",
    );
    let again = remove_tx(&c, &members, h + 1);
    assert_gov_applies_on(&mut c, again, "direct remove").await;
}

// REQ-B3-002 — Decision: a failure means the failed-apply restore also reverts a maintainer change in a block that applied, so a canonical rotation is lost.
#[tokio::test]
async fn b3_m1_successful_maintainer_block_still_applies() {
    let (mut s, c, members) = maintained_twins(MID_EPOCH_TIP).await;
    let pre = mobs(&s.node).await;
    let (h, slot, prev) = s.best().await;
    let newcomer = KeyPair::generate();
    let ok = block_at(
        &s,
        h + 1,
        slot + 1,
        prev,
        vec![add_tx(&s, &members, &newcomer, h + 1)],
    );
    let r = s.apply(ok.clone()).await;
    let got = mobs(&s.node).await;
    assert!(
        r.is_ok(),
        "a clean AddMaintainer block applies: {}",
        err_text(&r)
    );
    assert_eq!(
        tip(&s).await,
        (h + 1, ok.hash()),
        "the clean block moves the tip"
    );
    assert_eq!(got.members.len(), MEMBERS + 1, "the newcomer is seated");
    assert!(
        got.members.contains(newcomer.public_key()),
        "the newcomer is seated"
    );
    assert_eq!(
        got.last_updated,
        h + 1,
        "the rotation is stamped at its height"
    );
    assert_ne!(
        got.digest, pre.digest,
        "the set digest moves with the rotation"
    );
    assert!(got.file != pre.file, "maintainer_state.bin is rewritten");
    let disk = MaintainerState::load(&s.node.config.data_dir).expect("the root file decodes");
    assert_eq!(
        disk.set.members, got.members,
        "the file holds the in-memory set"
    );
    assert_ne!(
        got.members,
        mobs(&c.node).await.members,
        "the control, which never saw the block, keeps the old set"
    );
}

fn root_inode(r: &Rig) -> u64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(r.node.config.data_dir.join("maintainer_state.bin"))
        .expect("FIXTURE: stat the root file")
        .ino()
}

// REQ-B3-002 — Decision: a failure means an invalid block whose rotation never verified still rewrites maintainer_state.bin when it fails, so any gossiped bad block costs a durable write of the install authority.
#[tokio::test]
async fn b3_m1_failed_apply_with_unverified_rotation_writes_no_root_file() {
    let (mut s, _c, members) = maintained_twins(MID_EPOCH_TIP).await;
    let want = mobs(&s.node).await;
    let inode = root_inode(&s);
    let (h, slot, prev) = s.best().await;
    let newcomer = KeyPair::generate();
    let unverified = maintainer_tx(&s.node, true, newcomer.public_key(), &[&members[0]], h + 1);
    let f = failing_gov_block(&s, h + 1, slot + 1, prev, unverified);
    let ghost = f.transactions[2].hash().to_string();
    let r = s.apply(f).await;
    let text = err_text(&r);
    assert!(
        r.is_err() && text.contains(&ghost),
        "PRECONDITION: F must fail inside the tx loop at the ghost transfer {ghost}; got {text}"
    );
    assert_mobs(
        &mobs(&s.node).await,
        &want,
        "apply_block(F with an unverified AddMaintainer) returned Err",
    );
    assert_eq!(
        root_inode(&s),
        inode,
        "a failed apply whose rotation never moved the root must not rewrite maintainer_state.bin"
    );
    let (h, slot, prev) = s.best().await;
    let ok = block_at(
        &s,
        h + 1,
        slot + 1,
        prev,
        vec![add_tx(&s, &members, &newcomer, h + 1)],
    );
    s.apply(ok)
        .await
        .expect("NON-VACUITY: a clean rotation applies");
    assert_ne!(
        root_inode(&s),
        inode,
        "NON-VACUITY: a save on this filesystem is visible as a new inode"
    );
}

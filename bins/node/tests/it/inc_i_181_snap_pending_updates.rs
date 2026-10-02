//! INC-I-181 — a snap install must not install `pending_updates` the verified
//! state root does not authenticate.
//!
//! The quorum agrees on `state_root` only, and the payload comes from ONE peer.
//! A peer holding the honest state can serve the honest root R together with a
//! ProducerSet whose `pending_updates` queue carries a forged entry. Today the
//! root preimage (`ProducerSet::serialize_canonical`) omits the queue, so both
//! install arms accept the forgery and the next epoch flush applies it.
//!
//! REQ-181-SEC-001 — Decision: a failure reveals a snap client installs a queued
//! producer mutation that no quorum authenticated, so one download peer can fork
//! every fresh joiner at the next epoch boundary.
//! REQ-181-003 — Decision: a failure reveals the fix discards or rewrites a
//! legitimate mid-epoch queue, so an HONEST snap node forks at the next boundary.
//!
//! Activation note: these tests run at the devnet heights `Node::new_for_test`
//! builds (0..=6). After the fix, the devnet/test value of the new pending-root
//! activation height must be <= those heights, or 001/005 stay red.
//!
// OUTPUT CONTRACT: fn apply_snap_snapshot(&mut self, snapshot: VerifiedSnapshot) -> Result<()>
//   O1: self.producer_set — pending_updates queue (in memory) after the call
//   O1b: self.chain_state / self.utxo_set — tip + UTXO content (refusal = unchanged)
//   O2: state_db — META_PENDING_UPDATES (get_pending_updates) + rebuild marker
//   O3: return value — Ok (legacy refusal / success) or Err (staged halt)
//   O4 (derived): the next flush of O1 (apply_pending_updates) — the forged
//       mutation must not reach `producers`
// PATHS: P1 legacy arm (utxo_staged = None); P2 staged arm (utxo_staged = Some)
// INPUT PARTITIONS:
//   IPa forged queue under the honest root R (served queue != authenticated queue)
//   IPb honest non-empty queue under the root the server computed over it
// MATRIX:
//   P1 x IPa x {O1,O1b,O2,O4} -> test_181_001_legacy_forged_pending_update_refused
//   P2 x IPa x {O1,O2,O3,O4}  -> test_181_005_staged_forged_pending_update_refused
//   P1 x IPb x {O1,O1b,O2,O3,O4} -> test_181_003_honest_midepoch_pending_installs_and_flushes_identically
//   P2 x IPb: not covered — the staged arm shares the post-install root check
//       with P1 x IPb; the honest-queue parity is a property of the root formula.
//   TEST-181-004 (below-AH golden parity) is a storage unit test that needs the
//       gated helper API, which does not exist yet — the developer adds it with the fix.

use crate::inc_i_156_m1_harness as h;
use crate::m3_common;
use crypto::{Hash, PublicKey};
use doli_node::node::Node;
use network::{StagedUtxoMarker, VerifiedSnapshot};
use std::sync::atomic::Ordering;
use storage::{PendingProducerUpdate, ProducerSet, ProducerStatus};
use tempfile::TempDir;

const CHAIN_LEN: u64 = 6;
const N_PRODUCERS: usize = 3;
const EXTRA_UTXOS: usize = 32;
const STAGED_SET: usize = 600;
const CHUNK_BYTES: usize = 64 * 1024;
const STAGED_SEED: u64 = m3_common::FIXTURE_SEED ^ 0x0181;

fn decode_ps(bytes: &[u8]) -> ProducerSet {
    bincode::deserialize(bytes).expect("fixture: ProducerSet bytes must decode")
}

fn encode_ps(ps: &ProducerSet) -> Vec<u8> {
    bincode::serialize(ps).expect("fixture: ProducerSet must encode")
}

/// The forged mutation: slash an honest, active producer at the next flush.
fn forged_slash(victim: PublicKey, height: u64) -> PendingProducerUpdate {
    PendingProducerUpdate::Slash {
        pubkey: victim,
        height,
    }
}

fn is_forged_slash(u: &PendingProducerUpdate, victim: &PublicKey) -> bool {
    matches!(u, PendingProducerUpdate::Slash { pubkey, .. } if pubkey == victim)
}

fn is_slashed(ps: &ProducerSet, victim: &PublicKey) -> bool {
    matches!(
        ps.get_by_pubkey(victim).map(|p| &p.status),
        Some(ProducerStatus::Slashed { .. })
    )
}

/// The first active producer: the forged Slash's victim.
fn victim_of(ps: &ProducerSet) -> PublicKey {
    let p = ps
        .all_producers()
        .into_iter()
        .find(|p| matches!(p.status, ProducerStatus::Active))
        .expect("non-vacuity: the fixture must hold an active producer to slash");
    p.public_key
}

/// A legacy-arm snapshot at the tip: the node's own 3-state plus `EXTRA_UTXOS`, so an
/// install is observable as a changed UTXO set. Returns the snapshot and the honest
/// ProducerSet bytes it carries.
async fn legacy_fixture() -> (Node, VerifiedSnapshot, TempDir) {
    let (mut node, producers, temp) = h::make_node(N_PRODUCERS).await;
    let params = node.params.clone();
    h::install_production_utxo_backend(&node).await;
    h::apply_plain_up_to(&mut node, &producers, CHAIN_LEN, &params).await;
    assert!(
        !node.recovery_mode.load(Ordering::Relaxed),
        "fixture: recovery_mode must be false or the install path is never reached"
    );

    let snapshot = {
        let cs = node.chain_state.read().await;
        let utxo = node.utxo_set.read().await;
        let ps = node.producer_set.read().await;
        let base = storage::StateSnapshot::create(&cs, &utxo, &ps).expect("fixture: create");
        let mut widened = storage::UtxoSet::deserialize_canonical(&base.utxo_set_bytes)
            .expect("fixture: own canonical bytes must decode");
        for (op, entry) in m3_common::randomized_entries(EXTRA_UTXOS, STAGED_SEED) {
            widened.insert(op, entry).expect("fixture: insert");
        }
        let utxo_bytes = widened.serialize_canonical();
        let root = storage::compute_state_root_from_bytes(
            &base.chain_state_bytes,
            &utxo_bytes,
            &base.producer_set_bytes,
        )
        .expect("fixture: root");
        VerifiedSnapshot {
            block_hash: base.block_hash,
            block_height: base.block_height,
            chain_state: base.chain_state_bytes,
            utxo_set: utxo_bytes,
            utxo_staged: None,
            producer_set: base.producer_set_bytes,
            state_root: root,
            block_header_bytes: None,
            epoch_bond_snapshot_bytes: None,
            epoch_accumulators_bytes: None,
            epoch_state_bytes: Some(node.epoch_state.serialize()),
        }
    };
    assert_eq!(
        snapshot.block_height, CHAIN_LEN,
        "fixture: snapshot at the tip"
    );
    (node, snapshot, temp)
}

/// A staged-arm snapshot: the incoming UTXO set is already streamed into
/// `cf_utxo_staging`, and the root is the honest root over the node's ProducerSet.
async fn staged_fixture() -> (Node, VerifiedSnapshot, TempDir) {
    let (node, _kp, temp) = h::make_node(N_PRODUCERS).await;
    h::install_production_utxo_backend(&node).await;

    let mut set = storage::utxo::UtxoSet::new();
    for (o, e) in m3_common::randomized_entries(STAGED_SET, STAGED_SEED) {
        set.insert(o, e).expect("fixture insert");
    }
    let image = set.serialize_canonical();
    let digest = set.canonical_digest().expect("fixture: digest");
    let count = set.utxo_count();
    let sink = doli_node::node::StateDbChunkSink::new(node.state_db.clone());
    for body in image[8..].chunks(CHUNK_BYTES) {
        network::sync::UtxoChunkSink::stage(&sink, body).expect("fixture: stage chunk");
    }
    assert_eq!(
        node.state_db.staged_utxo_len() as u64,
        count,
        "fixture: the transfer must be complete, or the install refuses before promoting"
    );

    let snapshot = {
        let cs = node.chain_state.read().await;
        let utxo = node.utxo_set.read().await;
        let ps = node.producer_set.read().await;
        let base = storage::StateSnapshot::create(&cs, &utxo, &ps).expect("fixture: create");
        let root = storage::compute_state_root_from_bytes(
            &base.chain_state_bytes,
            &image,
            &base.producer_set_bytes,
        )
        .expect("fixture: root");
        VerifiedSnapshot {
            block_hash: base.block_hash,
            block_height: base.block_height,
            chain_state: base.chain_state_bytes,
            utxo_set: Vec::new(),
            utxo_staged: Some(StagedUtxoMarker {
                utxo_hash: digest,
                utxo_count: count,
            }),
            producer_set: base.producer_set_bytes,
            state_root: root,
            block_header_bytes: None,
            epoch_bond_snapshot_bytes: None,
            epoch_accumulators_bytes: None,
            epoch_state_bytes: Some(node.epoch_state.serialize()),
        }
    };
    (node, snapshot, temp)
}

/// Replace the snapshot's ProducerSet with the honest one plus a forged queued Slash,
/// keeping the served `state_root` (the quorum root R). Returns the victim.
fn inject_forged_slash(snapshot: &mut VerifiedSnapshot) -> PublicKey {
    let mut forged = decode_ps(&snapshot.producer_set);
    assert_eq!(
        forged.pending_update_count(),
        0,
        "fixture: the honest queue must be empty, so any queued entry after install is the forgery"
    );
    let victim = victim_of(&forged);
    forged.queue_update(forged_slash(victim, snapshot.block_height));
    snapshot.producer_set = encode_ps(&forged);
    victim
}

async fn utxo_fingerprint(node: &Node) -> (u64, Hash, Hash, u64) {
    let utxo = node.utxo_set.read().await;
    let cs = node.chain_state.read().await;
    (
        utxo.utxo_count(),
        crypto::hash::hash(&utxo.serialize_canonical()),
        cs.best_hash,
        cs.best_height,
    )
}

// ==================== P1 x IPa — legacy arm, forged queue ====================

// Requirement: REQ-181-SEC-001 (Must)
// REQ-181-SEC-001 — Decision: a failure reveals the legacy arm installs a forged queue under the honest root.
// Acceptance: nothing forged reaches memory or META_PENDING_UPDATES; the install is refused.
#[tokio::test]
async fn test_181_001_legacy_forged_pending_update_refused() {
    let (mut node, mut snapshot, _temp) = legacy_fixture().await;
    let victim = inject_forged_slash(&mut snapshot);
    let before = utxo_fingerprint(&node).await;
    assert!(
        before.0 > 0,
        "non-vacuity: the node must already hold state"
    );
    assert!(
        !is_slashed(&*node.producer_set.read().await, &victim),
        "non-vacuity: the victim must not be slashed before the install"
    );

    node.apply_snap_snapshot(snapshot)
        .await
        .expect("a refused legacy snapshot must not propagate an error");

    let (queued_forgery, pending_len) = {
        let ps = node.producer_set.read().await;
        (
            ps.pending_updates_for(&victim)
                .iter()
                .any(|u| is_forged_slash(u, &victim)),
            ps.pending_update_count(),
        )
    };
    assert!(
        !queued_forgery,
        "INC-I-181: the in-memory pending_updates holds the forged Slash of {:?} although the \
         verified root R does not authenticate it (queue len {})",
        victim, pending_len
    );
    assert!(
        !node
            .state_db
            .get_pending_updates()
            .iter()
            .any(|u| is_forged_slash(u, &victim)),
        "INC-I-181: META_PENDING_UPDATES on disk holds the forged Slash — a restart reloads it"
    );
    assert_eq!(
        utxo_fingerprint(&node).await,
        before,
        "INC-I-181: a snapshot whose queue the root does not authenticate must install nothing \
         (utxo_count, utxo hash, best_hash, best_height unchanged)"
    );

    // O4: the next epoch flush must not slash the victim.
    let mut flushed = node.producer_set.read().await.clone();
    flushed.apply_pending_updates_with_cap(0);
    assert!(
        !is_slashed(&flushed, &victim),
        "INC-I-181: the epoch flush applied the forged Slash to an honest producer — the snap \
         node forks at the next boundary"
    );
}

// ==================== P2 x IPa — staged arm, forged queue ====================

// Requirement: REQ-181-SEC-001 (Must)
// REQ-181-SEC-001 — Decision: a failure reveals the staged arm promotes and keeps a forged queue under the honest root.
// Acceptance: the staged install fails (Err / halted) and no forged entry is live in memory or durable without a halt.
#[tokio::test(flavor = "multi_thread")]
async fn test_181_005_staged_forged_pending_update_refused() {
    let (mut node, mut snapshot, _temp) = staged_fixture().await;
    let victim = inject_forged_slash(&mut snapshot);

    let outcome = node.apply_snap_snapshot(snapshot).await;

    let queued_forgery = node
        .producer_set
        .read()
        .await
        .pending_updates_for(&victim)
        .iter()
        .any(|u| is_forged_slash(u, &victim));
    assert!(
        !queued_forgery,
        "INC-I-181: the staged install left the forged Slash of {:?} in the in-memory \
         pending_updates (outcome: {:?})",
        victim,
        outcome.as_ref().map(|_| ())
    );

    let durable_forgery = node
        .state_db
        .get_pending_updates()
        .iter()
        .any(|u| is_forged_slash(u, &victim));
    let halted = node.state_db.get_rebuild_in_progress().is_some();
    assert!(
        !durable_forgery || halted,
        "INC-I-181: META_PENDING_UPDATES holds the forged Slash and the node is NOT halted — \
         a restart serves and flushes an unauthenticated queue"
    );
    assert!(
        outcome.is_err() || !durable_forgery,
        "INC-I-181: apply_snap_snapshot returned Ok over a durable forged queue — the pipeline \
         treats a poisoned install as healthy"
    );

    let mut flushed = node.producer_set.read().await.clone();
    flushed.apply_pending_updates_with_cap(0);
    assert!(
        !is_slashed(&flushed, &victim),
        "INC-I-181: the epoch flush applied the forged Slash on the staged arm"
    );
}

// ==================== P1 x IPb — honest mid-epoch queue (control) ====================

// Requirement: REQ-181-003 (Must)
// REQ-181-003 — Decision: a failure reveals the fix discards or reorders a legitimate queue, so honest snap nodes fork.
// Acceptance: an honest snapshot with a non-empty queue installs, the queue is preserved in
// order in memory and on disk, and the flush equals the serving node's flush.
#[tokio::test]
async fn test_181_003_honest_midepoch_pending_installs_and_flushes_identically() {
    let (mut node, mut snapshot, _temp) = legacy_fixture().await;

    // The serving node's honest state: two queued, order-sensitive entries.
    let mut honest = decode_ps(&snapshot.producer_set);
    let victim = victim_of(&honest);
    honest.queue_update(PendingProducerUpdate::Exit {
        pubkey: victim,
        height: snapshot.block_height,
    });
    honest.queue_update(forged_slash(victim, snapshot.block_height));
    snapshot.producer_set = encode_ps(&honest);
    // The root the honest server computes over the state it serves (gated: the
    // fixture height is at/above the devnet INC-I-181 activation height).
    snapshot.state_root = storage::compute_state_root_from_bytes_gated(
        &snapshot.chain_state,
        &snapshot.utxo_set,
        &snapshot.producer_set,
        node.config
            .network
            .params()
            .inc_i_181_pending_root_activation_height,
    )
    .expect("honest root");
    let expected_root = snapshot.state_root;
    let before = utxo_fingerprint(&node).await;

    node.apply_snap_snapshot(snapshot)
        .await
        .expect("an honest snapshot must install");

    assert_ne!(
        utxo_fingerprint(&node).await,
        before,
        "an honest snapshot with a legitimate queue was refused — every mid-epoch snap fails"
    );
    let cached = node.cached_state_root.read().await.map(|(r, _, _)| r);
    assert_eq!(
        cached,
        Some(expected_root),
        "the installed state must hash to the root the honest server served"
    );

    let installed = node.producer_set.read().await.clone();
    // Every queued entry targets `victim`, so `pending_updates_for` is the whole queue in
    // order. bincode of the full struct is not compared: `producers` is a HashMap.
    let queue_of = |ps: &ProducerSet| {
        bincode::serialize(
            &ps.pending_updates_for(&victim)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>(),
        )
        .unwrap()
    };
    assert_eq!(
        installed.pending_update_count(),
        2,
        "the honest two-entry queue must be preserved, not discarded or truncated"
    );
    assert_eq!(
        queue_of(&installed),
        queue_of(&honest),
        "the installed queue must equal the served queue in content and order"
    );
    assert_eq!(
        installed.serialize_canonical(),
        honest.serialize_canonical(),
        "the installed producers/exit_history must equal the served ones"
    );
    assert_eq!(
        bincode::serialize(&node.state_db.get_pending_updates()).unwrap(),
        queue_of(&honest),
        "META_PENDING_UPDATES must persist the honest queue in order"
    );

    let mut mine = installed;
    let mut theirs = honest;
    mine.apply_pending_updates_with_cap(0);
    theirs.apply_pending_updates_with_cap(0);
    assert_eq!(
        mine.serialize_canonical(),
        theirs.serialize_canonical(),
        "after the epoch flush the snap node's producers must equal the continuous node's"
    );
}

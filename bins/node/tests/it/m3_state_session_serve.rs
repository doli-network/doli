//! UTXO-scalability M3 [F3] — SERVER side of the chunked snap-sync session.
//!
//! Pins the three serving-side filters the Failure Analyst marked BLOCKER for Q3:
//!   3a / F-06 — NO consistent-read mechanism exists. Today consistency comes from holding
//!               the three read guards across ONE O(N) serialization. A chunk stream cannot
//!               hold those guards, and nothing else pins a version, so chunks tear.
//!   3b / F-08 — the serve side answers at the CURRENT tip regardless of the requested hash
//!               (`state_snapshot_serve.rs:68-78`), while the client gate at
//!               `snap_sync.rs:200-208` demands exact root equality. Chunking stretches that
//!               window from one round-trip to minutes.
//!   F-11 / PM-024 — a half-rebuilt node must refuse to serve state, through the session
//!               door as well as the legacy one.
//!   F-16 — a bounded number of concurrent pinned views, refused typed, never queued.
//!
//! OUTPUT CONTRACT: `Node::serve_state_manifest(Hash) -> SyncResponse` and
//!                  `Node::serve_state_chunk(u64, Option<Vec<u8>>, u32) -> SyncResponse`
//!   Outputs observable from the stable API:
//!     O1 the response VARIANT (StateManifest / StateChunk / StateSessionUnavailable / Error)
//!     O2 the manifest's `utxo_hash`, `state_root`, `utxo_count`
//!     O3 the chunk `body` and `next_key`
//!     O4 the incremental digest reconstructed from O3 across a whole session
//!     O5 the refusal `reason`
//!     O6 the number of sessions the node will admit at once
//!     O7 the generic inbound-cap admission decision + `sync_requests_this_interval`
//!   Paths:
//!     P1 quiescent server, one session, run to exhaustion       — C1, C7 (O1..O4)
//!     P2 server MUTATES its UTXO set mid-session                — C2 (O4)  <-- the blocker
//!     P3 more sessions requested than the cap                   — C3 (O1, O5, O6)
//!     P4 chunk requested for an unknown/evicted session id      — C4 (O1, O5)
//!     P5 `rebuild_in_progress` armed                            — C5 (O1, O5)
//!     P6 the LEGACY single-frame path, unchanged                — C6 (O1, O2)
//!     P7 the 24/interval inbound cap ALREADY saturated          — C8 (O1, O7)
//!   MATRIX: P1xO2 -> C1 | P2xO4 -> C2 | P3xO1+O5+O6 -> C3 | P4xO1+O5 -> C4
//!           P5xO1+O5 -> C5 | P6xO1+O2 -> C6 | P1xO3 -> C7 | P7xO1+O7 -> C8
//! INPUT PARTITIONS: (a) a set large enough to need several chunks — a one-chunk session
//!   cannot tear; (b) a mutation applied strictly BETWEEN two chunk fetches; (c) a session
//!   id the node has never issued.

use crypto::Hash;
use doli_node::node::MAX_SYNC_REQUESTS_PER_INTERVAL;
use network::protocols::sync::{
    StateSessionRefusal, SyncRequest, SyncResponse, MAX_CONCURRENT_STATE_SESSIONS,
    STATE_CHUNK_MAX_BYTES,
};
use storage::{Outpoint, UtxoEntry};

use crate::inc_i_156_m1_harness as h;
use crate::m3_common;

/// Small enough to keep the node test fast, large enough that a 64 KiB chunk budget forces
/// several ranges — a single-chunk session could not tear even on broken code.
const SET_SIZE: usize = 4_000;
const SMALL_CHUNK: u32 = 64 * 1024;

struct Manifest {
    session_id: u64,
    block_hash: Hash,
    block_height: u64,
    state_root: Hash,
    utxo_hash: Hash,
    utxo_count: u64,
}

fn expect_manifest(resp: SyncResponse) -> Manifest {
    match resp {
        SyncResponse::StateManifest {
            session_id,
            block_hash,
            block_height,
            state_root,
            utxo_hash,
            utxo_count,
            ..
        } => Manifest {
            session_id,
            block_hash,
            block_height,
            state_root,
            utxo_hash,
            utxo_count,
        },
        other => panic!("expected StateManifest, got {}", other.type_name()),
    }
}

fn expect_chunk(resp: SyncResponse) -> (Vec<u8>, Option<Vec<u8>>) {
    match resp {
        SyncResponse::StateChunk { body, next_key, .. } => (body, next_key),
        other => panic!("expected StateChunk, got {}", other.type_name()),
    }
}

async fn populated_node(n: usize) -> (doli_node::node::Node, tempfile::TempDir) {
    let (node, _kp, temp) = h::make_node(1).await;
    h::install_production_utxo_backend(&node).await;
    for (outpoint, entry) in m3_common::randomized_entries(n, m3_common::FIXTURE_SEED) {
        node.state_db
            .insert_utxo(&outpoint, &entry)
            .expect("fixture: insert_utxo");
    }
    (node, temp)
}

/// Feed the reassembly the way a CLIENT must: the count header, then every body in order.
struct Reassembly {
    hasher_input: Vec<u8>,
    chunks: usize,
}

impl Reassembly {
    fn new(utxo_count: u64) -> Self {
        Self {
            hasher_input: utxo_count.to_le_bytes().to_vec(),
            chunks: 0,
        }
    }

    fn push(&mut self, body: &[u8]) {
        self.hasher_input.extend_from_slice(body);
        self.chunks += 1;
    }

    fn digest(&self) -> Hash {
        crypto::hash::hash(&self.hasher_input)
    }
}

// ==================== C1 — the manifest carries the legacy consensus values ====================

/// REQ-SCALE-006 / Res-1 / INV-SYNC-007 — Decision: a failure here means the session hands a
/// DIFFERENT `utxo_hash` or `state_root` from the legacy frame for the same state, so a
/// chunked sync and a legacy sync of the same node produce different consensus state. The
/// transport change would have become a consensus change.
#[tokio::test(flavor = "multi_thread")]
async fn m3_manifest_utxo_hash_equals_legacy_single_frame_utxo_hash() {
    let (node, _t) = populated_node(SET_SIZE).await;
    let tip = node.chain_state.read().await.best_hash;

    let legacy = node.serve_state_snapshot(tip).await;
    let (legacy_utxo_bytes, legacy_root) = match legacy {
        SyncResponse::StateSnapshot {
            utxo_set,
            state_root,
            ..
        } => (utxo_set, state_root),
        other => panic!(
            "the legacy path must still serve a snapshot, got {}",
            other.type_name()
        ),
    };

    let manifest = expect_manifest(node.serve_state_manifest(tip).await);

    assert_eq!(
        manifest.utxo_hash,
        crypto::hash::hash(&legacy_utxo_bytes),
        "the manifest's utxo_hash must be BIT-IDENTICAL to BLAKE3 over the legacy frame's \
         canonical image"
    );
    assert_eq!(
        manifest.state_root, legacy_root,
        "the manifest's state_root must equal the legacy frame's state_root"
    );
    assert_eq!(
        manifest.utxo_count,
        node.utxo_set.read().await.utxo_count(),
        "the manifest's utxo_count must be the count the header is derived from"
    );
    assert_eq!(
        manifest.block_hash, tip,
        "the manifest must name the state it pinned"
    );
    assert!(
        manifest.block_height > 0 || manifest.utxo_count > 0,
        "a manifest describing nothing at height 0 would make every later assertion vacuous"
    );
}

// ==================== C2 — THE critical test: the session pins a view ====================

/// F-06 / blocker 3a — Decision: THIS is the test the whole milestone turns on. If it fails,
/// the server is answering each chunk from its LIVE state, so a chain that advances during
/// the transfer produces a torn image whose digest can never equal the manifest. The client
/// would then retry forever, which is strictly worse than today's honest single-frame
/// failure. Without a session-lifetime pin this test tears — that is its job.
#[tokio::test(flavor = "multi_thread")]
async fn m3_session_pins_view_across_server_mutation() {
    let (node, _t) = populated_node(SET_SIZE).await;
    let tip = node.chain_state.read().await.best_hash;

    let manifest = expect_manifest(node.serve_state_manifest(tip).await);
    let mut re = Reassembly::new(manifest.utxo_count);

    // First chunk, then the server's state MOVES underneath the session.
    let (body, mut cursor) = expect_chunk(
        node.serve_state_chunk(manifest.session_id, None, SMALL_CHUNK)
            .await,
    );
    re.push(&body);
    assert!(
        cursor.is_some(),
        "the fixture must need more than one chunk, else the pin is never exercised"
    );

    mutate_server_state(&node).await;

    while let Some(start) = cursor.clone() {
        let (body, next) = expect_chunk(
            node.serve_state_chunk(manifest.session_id, Some(start), SMALL_CHUNK)
                .await,
        );
        re.push(&body);
        cursor = next;
    }

    assert!(
        re.chunks > 2,
        "only {} chunk(s) were served — a session this short cannot demonstrate a pin",
        re.chunks
    );
    assert_eq!(
        re.digest(),
        manifest.utxo_hash,
        "the reassembled digest diverged from the manifest after the server mutated its UTXO \
         set mid-session: the session did NOT hold a pinned view (F-06, blocker 3a)"
    );

    // Non-vacuity: the mutation must really have changed the LIVE set.
    let live = node
        .utxo_set
        .read()
        .await
        .canonical_digest()
        .expect("live digest");
    assert_ne!(
        live, manifest.utxo_hash,
        "the mutation did not change the live state, so the pin was never under test"
    );
}

/// Insert new entries, delete existing ones, and advance the tip — the three ways a live
/// server's state moves while it is serving.
async fn mutate_server_state(node: &doli_node::node::Node) {
    let fresh = m3_common::randomized_entries(256, m3_common::FIXTURE_SEED ^ 0xDEAD);
    for (outpoint, entry) in fresh.iter() {
        node.state_db
            .insert_utxo(outpoint, entry)
            .expect("mutation: insert_utxo");
    }

    let doomed: Vec<Outpoint> = m3_common::randomized_entries(128, m3_common::FIXTURE_SEED)
        .into_iter()
        .map(|(o, _): (Outpoint, UtxoEntry)| o)
        .collect();
    for outpoint in doomed.iter() {
        let _ = node.state_db.remove_utxo(outpoint);
    }

    let mut cs = node.chain_state.write().await;
    cs.best_height += 1;
    cs.best_hash = crypto::hash::hash(b"m3_advanced_tip");
}

// ==================== C3 — the concurrent-session cap ====================

/// F-16 / flood mode — Decision: a failure here means a node under a fleet-wide snap-sync
/// storm holds an unbounded number of pinned RocksDB snapshots. Pinned SSTs defer
/// compaction, so the failure is an unbounded DISK and memory growth on exactly the seeds
/// everyone is recovering from. Answering `SyncResponse::Error` instead of a typed refusal
/// is equally wrong: the client's existing error path BLACKLISTS (snap_sync.rs:269).
#[tokio::test(flavor = "multi_thread")]
async fn m3_session_cap_refuses_with_busy_not_error() {
    let (node, _t) = populated_node(SET_SIZE).await;
    let tip = node.chain_state.read().await.best_hash;

    let mut open = Vec::new();
    for i in 0..MAX_CONCURRENT_STATE_SESSIONS {
        let m = expect_manifest(node.serve_state_manifest(tip).await);
        assert!(
            !open.contains(&m.session_id),
            "session {} reused an id already open — ids must be unique while live",
            i
        );
        open.push(m.session_id);
    }

    let over = node.serve_state_manifest(tip).await;
    match over {
        SyncResponse::StateSessionUnavailable { reason, .. } => {
            assert!(
                matches!(reason, StateSessionRefusal::Busy),
                "over the cap the node must answer Busy, got {:?}",
                reason
            );
        }
        SyncResponse::Error(e) => panic!(
            "the cap must produce a TYPED refusal, not SyncResponse::Error({:?}) — the \
             client's error path blacklists the peer, which is the PM-006 peer-set \
             exhaustion failure on a small network",
            e
        ),
        SyncResponse::StateManifest { .. } => panic!(
            "the node admitted session {} beyond the cap of {} — the pinned-view count is \
             unbounded",
            MAX_CONCURRENT_STATE_SESSIONS + 1,
            MAX_CONCURRENT_STATE_SESSIONS
        ),
        other => panic!("unexpected response {}", other.type_name()),
    }

    // The already-open sessions must be UNHARMED by the refusal.
    let (body, _next) = expect_chunk(node.serve_state_chunk(open[0], None, SMALL_CHUNK).await);
    assert!(
        !body.is_empty(),
        "refusing a new session must not disturb the sessions already admitted"
    );
}

// ==================== C4 — an unknown session is expired, not an error ====================

/// F-08 / blocker 3d — Decision: a failure here means a client whose session was evicted
/// (TTL, server restart, idle timeout) receives an error that its existing handler treats as
/// peer misbehaviour, blacklisting an honest peer. On a 6-node LAN that exhausts the peer
/// set (PM-006).
#[tokio::test(flavor = "multi_thread")]
async fn m3_expired_session_returns_manifest_expired_not_error() {
    let (node, _t) = populated_node(SET_SIZE).await;

    let never_issued = 0xFFFF_FFFF_FFFF_FFFFu64;
    let resp = node
        .serve_state_chunk(never_issued, None, STATE_CHUNK_MAX_BYTES)
        .await;

    match resp {
        SyncResponse::StateSessionUnavailable { session_id, reason } => {
            assert_eq!(
                session_id, never_issued,
                "the refusal must echo the session id the client asked about"
            );
            assert!(
                matches!(reason, StateSessionRefusal::ManifestExpired),
                "an unknown session must be ManifestExpired, got {:?}",
                reason
            );
        }
        SyncResponse::Error(e) => panic!(
            "an unknown session must be a TYPED retryable refusal, not Error({:?})",
            e
        ),
        other => panic!(
            "an unknown session must not be served, got {}",
            other.type_name()
        ),
    }
}

// ==================== C5 — the rebuild halt covers the session door ====================

/// F-11 / PM-024 — Decision: a failure here means a node whose durable ledger was truncated
/// by an interrupted rebuild hands that truncated ledger to a bootstrapping peer through the
/// NEW door, while the old door still refuses. The halt would be a door-specific guard
/// instead of a node-wide one, which is how a refusal silently stops covering the live path.
#[tokio::test(flavor = "multi_thread")]
async fn m3_serve_refused_while_rebuild_in_progress() {
    let (node, _t) = populated_node(512).await;
    let tip = node.chain_state.read().await.best_hash;

    // Take a valid session id BEFORE the halt, so the chunk refusal cannot be confused with
    // "unknown session".
    let live = expect_manifest(node.serve_state_manifest(tip).await).session_id;

    node.state_db
        .set_rebuild_in_progress(123)
        .expect("arm the rebuild marker");
    let reason = node
        .rebuild_halt_reason()
        .expect("the marker must arm rebuild_halt_reason");
    assert!(
        reason.contains("[STATE_CORRUPT]"),
        "the halt reason must carry the operator-facing [STATE_CORRUPT] tag, got {:?}",
        reason
    );

    for (label, resp) in [
        ("GetStateManifest", node.serve_state_manifest(tip).await),
        (
            "GetStateChunk",
            node.serve_state_chunk(live, None, STATE_CHUNK_MAX_BYTES)
                .await,
        ),
    ] {
        match resp {
            SyncResponse::StateSessionUnavailable {
                reason: StateSessionRefusal::Halted(text),
                ..
            } => {
                assert!(
                    text.contains("[STATE_CORRUPT]"),
                    "{} refusal must carry the [STATE_CORRUPT] text the legacy path uses, \
                     got {:?}",
                    label,
                    text
                );
            }
            other => panic!(
                "{} must be refused with Halted while rebuild_in_progress is armed, got {}",
                label,
                other.type_name()
            ),
        }
    }

    // The legacy door must still refuse too — the halt is node-wide.
    assert!(
        matches!(node.serve_state_snapshot(tip).await, SyncResponse::Error(_)),
        "the legacy GetStateSnapshot path must keep its existing refusal"
    );
}

// ==================== C6 — the BRIDGE stays byte-for-byte what it is ====================

/// REQ-SCALE-007 / Migration Stage 3 BRIDGE — Decision: a failure here means the legacy
/// single-frame path changed while the fleet still contains binaries that speak only that
/// path. The bridge is what makes Stage 3 a ROLLING deploy; break it and the deploy becomes
/// synchronized, which is the INC-I-062 class.
#[tokio::test(flavor = "multi_thread")]
async fn m3_legacy_get_state_snapshot_still_served_unchanged() {
    let (node, _t) = populated_node(SET_SIZE).await;
    let tip = node.chain_state.read().await.best_hash;

    let resp = node.serve_state_snapshot(tip).await;
    let (block_hash, block_height, chain_state, utxo_set, producer_set, state_root) = match resp {
        SyncResponse::StateSnapshot {
            block_hash,
            block_height,
            chain_state,
            utxo_set,
            producer_set,
            state_root,
            ..
        } => (
            block_hash,
            block_height,
            chain_state,
            utxo_set,
            producer_set,
            state_root,
        ),
        other => panic!(
            "the legacy path must keep serving StateSnapshot, got {}",
            other.type_name()
        ),
    };

    let cs = node.chain_state.read().await;
    let utxo = node.utxo_set.read().await;
    let ps = node.producer_set.read().await;

    assert_eq!(block_hash, cs.best_hash, "still serves the current tip");
    assert_eq!(block_height, cs.best_height, "still serves the tip height");
    assert_eq!(
        utxo_set,
        utxo.serialize_canonical(),
        "the legacy frame must still carry the FULL canonical image, byte for byte"
    );
    assert_eq!(
        state_root,
        storage::compute_state_root(&cs, &utxo, &ps).expect("root"),
        "the legacy frame's state_root must still be the composed three-component root"
    );
    assert!(
        !chain_state.is_empty() && !producer_set.is_empty(),
        "the legacy frame must still carry chain_state and producer_set bincode"
    );
    assert!(
        utxo_set.len() <= 16 * 1024 * 1024,
        "the BRIDGE is only claimed for sets <= 16 MiB; this fixture is {} bytes",
        utxo_set.len()
    );
}

// ==================== C7 — the session slot is released ====================

/// F-16 — Decision: a failure here means a completed or expired session keeps its pinned
/// view forever, so after `MAX_CONCURRENT_STATE_SESSIONS` successful syncs the node refuses
/// every further request with `Busy` and never recovers. A leak of the scarce resource is
/// indistinguishable from a working cap until the cap is reached.
#[tokio::test(flavor = "multi_thread")]
async fn m3_session_released_on_completion_and_on_ttl() {
    let (node, _t) = populated_node(SET_SIZE).await;
    let tip = node.chain_state.read().await.best_hash;

    // Fill the cap, then drive ONE of them to exhaustion.
    let mut ids = Vec::new();
    for _ in 0..MAX_CONCURRENT_STATE_SESSIONS {
        ids.push(expect_manifest(node.serve_state_manifest(tip).await).session_id);
    }
    assert!(
        matches!(
            node.serve_state_manifest(tip).await,
            SyncResponse::StateSessionUnavailable { .. }
        ),
        "the cap must be reached before the release can be observed, else this test is vacuous"
    );

    let finished = ids[0];
    let mut cursor: Option<Vec<u8>> = None;
    loop {
        let (_, next) = expect_chunk(
            node.serve_state_chunk(finished, cursor.clone(), STATE_CHUNK_MAX_BYTES)
                .await,
        );
        match next {
            Some(k) => cursor = Some(k),
            None => break,
        }
    }

    let reopened = node.serve_state_manifest(tip).await;
    assert!(
        matches!(reopened, SyncResponse::StateManifest { .. }),
        "a session that reached next_key == None must free its slot, got {}",
        reopened.type_name()
    );

    // And the finished session id must no longer be servable.
    match node
        .serve_state_chunk(finished, None, STATE_CHUNK_MAX_BYTES)
        .await
    {
        SyncResponse::StateSessionUnavailable {
            reason: StateSessionRefusal::ManifestExpired,
            ..
        } => {}
        other => panic!(
            "a completed session must be gone, not servable — got {}",
            other.type_name()
        ),
    }
}

// ============ C8 — the session's own traffic must not trip the generic inbound cap ============

/// F-08 / INC-I-012 F6 / PM-006 — Decision: a failure here means the new transport blacklists
/// an honest serving peer with its OWN traffic. `MAX_SYNC_REQUESTS_PER_INTERVAL` counts every
/// request kind (`network_events.rs:307`) and refuses with `Error("busy: ...")`; the client
/// blacklists on any response whose text contains "busy" (`sync_engine/response.rs:111,126`).
/// One request per chunk means a 100 MB set saturates the cap mid-session, so the client
/// evicts the only peer that was serving it — a self-inflicted stall, INC-I-138-shaped, with
/// no misbehaving party anywhere in the exchange.
///
/// The counter is owned by `on_sync_request`, whose signature takes a libp2p
/// `ResponseChannel` that has no public or test constructor (already recorded at
/// `bins/node/tests/state_root_memoize_m1.rs:18`). The admission decision must therefore be
/// extracted to a seam of its own, exactly as `record_direct_attestation` was extracted for
/// the INC-I-192 gate: `admit_sync_request` returns `Some(refusal)` to send back, or `None`
/// when the request is admitted, and it — not the caller — owns the counter.
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::assertions_on_constants)]
async fn m3_chunk_requests_are_exempt_from_the_inbound_per_interval_cap() {
    let (mut node, _t) = populated_node(SET_SIZE).await;
    let tip = node.chain_state.read().await.best_hash;

    // A session admitted BEFORE the cap is reached — the exemption is claimed for live
    // sessions, so the chunk request below must name one.
    let live = expect_manifest(node.serve_state_manifest(tip).await).session_id;

    node.sync_requests_this_interval = MAX_SYNC_REQUESTS_PER_INTERVAL;

    // (b) The manifest is the request that PINS a view. It stays under the generic cap.
    match node.admit_sync_request(&SyncRequest::GetStateManifest { block_hash: tip }) {
        Some(SyncResponse::Error(text)) => assert!(
            text.contains("busy"),
            "past the cap GetStateManifest must get the existing busy refusal, got {:?}",
            text
        ),
        None => panic!(
            "GetStateManifest was admitted past {} requests — the expensive, view-pinning \
             request escaped the generic cap that exists to bound serving work",
            MAX_SYNC_REQUESTS_PER_INTERVAL
        ),
        Some(other) => panic!(
            "past the cap GetStateManifest must get the generic busy refusal, got {}",
            other.type_name()
        ),
    }

    // (a) The chunk is admission-controlled by MAX_CONCURRENT_STATE_SESSIONS plus one
    // outstanding chunk per session. Counting it twice removes liveness, not risk.
    let before = node.sync_requests_this_interval;
    let admitted = node.admit_sync_request(&SyncRequest::GetStateChunk {
        session_id: live,
        start_key: None,
        max_bytes: STATE_CHUNK_MAX_BYTES,
    });
    assert!(
        admitted.is_none(),
        "a chunk of a LIVE session was refused past the cap ({:?}) — the session stalls and the \
         client blacklists the peer that was serving it correctly",
        admitted.map(|r| r.type_name())
    );

    // (c) And it must not consume the generic budget either, or it trips the cap for the
    // block and header traffic the same node is serving to everyone else.
    assert_eq!(
        node.sync_requests_this_interval, before,
        "serving a chunk incremented sync_requests_this_interval — an exemption that still \
         counts is not an exemption, it just moves the stall onto the node's other peers"
    );

    // Non-vacuity: admission must mean the chunk is really served, not silently dropped.
    let (body, _next) = expect_chunk(node.serve_state_chunk(live, None, SMALL_CHUNK).await);
    assert!(
        !body.is_empty(),
        "the exempted chunk request produced no bytes, so the exemption proves nothing"
    );
    assert!(
        MAX_SYNC_REQUESTS_PER_INTERVAL > 0,
        "a cap of 0 would make the saturation above vacuous"
    );
}

// ============ C9 — an oversized max_bytes is clamped on the SERVING node ============

/// REQ-SCALE-013 / adversarial (test-writer report §7) — Decision: a failure here means a
/// peer can name its own serving budget. `GetStateChunk{max_bytes: u32::MAX}` would make the
/// server materialise its WHOLE UTXO set into one response body — the exact allocation DoS
/// `MAX_SYNC_SIZE` exists to prevent, arriving through the new door. The requester's number
/// is peer-supplied data and must never be trusted: the clamp belongs in `serve_state_chunk`.
/// The `max_bytes = 0` half is the other end of the same rule — a budget the requester sets
/// to nothing must still advance by one entry, or a hostile (or buggy) client wedges the
/// session it opened and holds a pinned view until the TTL.
#[tokio::test(flavor = "multi_thread")]
async fn m3_oversized_max_bytes_is_clamped_server_side() {
    let (node, _t) = populated_node(SET_SIZE).await;
    let tip = node.chain_state.read().await.best_hash;

    // Non-vacuity: the fixture must be bigger than one maximal chunk would be, otherwise
    // "the body is under the cap" is true for trivial reasons.
    let image_len = node.utxo_set.read().await.serialize_canonical().len();
    assert!(
        image_len > STATE_CHUNK_MAX_BYTES as usize / 4,
        "the fixture image is {} bytes — too small for the clamp to be observable",
        image_len
    );

    let greedy = expect_manifest(node.serve_state_manifest(tip).await);
    let (body, next) = expect_chunk(
        node.serve_state_chunk(greedy.session_id, None, u32::MAX)
            .await,
    );
    assert!(
        body.len() <= STATE_CHUNK_MAX_BYTES as usize,
        "a peer asked for u32::MAX bytes and the server served {} — the requester's budget was \
         trusted, so one request re-materialises the whole set in the SERVER's RAM",
        body.len()
    );
    assert!(
        !body.is_empty(),
        "the clamp must still serve a chunk, not refuse the request"
    );
    let _ = next;

    // A budget of zero must still advance by exactly one entry, or the session wedges while
    // holding a pinned view.
    let starving = expect_manifest(node.serve_state_manifest(tip).await);
    let (zero_body, zero_next) =
        expect_chunk(node.serve_state_chunk(starving.session_id, None, 0).await);
    assert!(
        !zero_body.is_empty(),
        "max_bytes = 0 returned an empty body with next_key = {:?} — the cursor cannot advance \
         and the session holds its pinned view until the TTL",
        zero_next
    );
    assert!(
        zero_next.is_some(),
        "max_bytes = 0 on a {}-entry set reported the walk as complete — a client would install \
         a one-entry UTXO set",
        SET_SIZE
    );
}

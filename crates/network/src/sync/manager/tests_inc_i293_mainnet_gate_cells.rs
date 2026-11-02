// covers: manager/fork_walk.rs (walk_intake, reply_skips_walk) manager/snap_descent.rs
//! INC-I-293 ZB-2: the INC-I-288 fork-choice gate on MAINNET parameters, in the
//! unit each network-crate site reads. Walk intake reads the block at
//! `local_height + 1`; the snap descent proof reads the local tip itself. The
//! cell heights are literals, so a moved mainnet pin fails here instead of
//! silently moving the cells with it.
//!
// OUTPUT CONTRACT (heights = local tip: 866_998 | 866_999 | 867_000)
//   F1 SyncManager::reply_skips_walk(walk peer, answered) -> bool
//     answered = GetHeaders (not the walk fetch):     false | true  | true
//     answered = GetBlockByHash(the awaited hash):    false | false | false
//     answered = None (unknown request):              false | false | false
//   F2 SyncManager::walk_intake(walk peer, Some(padded pending answer))
//     O1 return (consumed):                           true  | true  | true
//     O2 fork_walk_pending_fetch:                     None  | Some(peer, hash) | Some(peer, hash)
//        fork_recovery_current_parent:               Some(root) | None | None
//     O3 take_refused_body_suppliers:                 []    | [(peer, slot)] | [(peer, slot)]
//     O4 completed walk bodies (check_fork_recovery_connection(true)):
//                                                     [padded, seed] | not reachable (fetch
//                                                     waits on its request timeout) | same
//   F3 SyncManager::snap_descent_next_request(anchor) / next_request dispatch /
//      accept_snap_descent_chain / snap_descent_proven
//     anchor within the bound: narrow     Ok(None) | Ok(None) | Ok(Some(GetHeaders f, 500))
//                              dispatch   Manifest | Manifest | GetHeaders f, 500
//                              accept     false    | false    | true
//                              proven     false    | false    | true
//     anchor over the bound:   narrow     Ok(None) | Ok(None) | Err(())

use std::time::Instant;

use crypto::{Hash, KeyPair};
use doli_core::{Block, BlockHeader, FinalityCheckpoint, Network, NetworkParams, Transaction};
use libp2p::PeerId;
use vdf::{VdfOutput, VdfProof};

use super::snap_descent::SNAP_DESCENT_PROOF_MAX_HEADERS;
use crate::protocols::SyncRequest;
use crate::sync::manager::{SyncConfig, SyncManager, SyncPhase, SyncPipelineData, SyncState};

/// The operator pin. The cells below use literal tips around it.
const MAINNET_GATE: u64 = 867_000;
const TIP_OFF: u64 = 866_998;
const TIP_EDGE: u64 = 866_999;
const TIP_ON: u64 = 867_000;
/// Snap cells: the finalized checkpoint, and a proof span over one 500-header batch.
const FH: u64 = 866_990;
const SPAN: u64 = 600;
const PAD: usize = 64 * 1024;
const TS: u64 = 1_700_000_000;

/// The sync config the node builds from mainnet `NetworkParams` (init.rs), for
/// the three gates these sites read.
fn mainnet_config() -> SyncConfig {
    let p = NetworkParams::defaults(Network::Mainnet);
    SyncConfig {
        inc_i_288_fork_choice_activation_height: p.inc_i_288_fork_choice_activation_height,
        inc_i_236_block_signature_activation_height: p.inc_i_236_block_signature_activation_height,
        inc_i_178_attestation_bls_activation_height: p.inc_i_178_attestation_bls_activation_height,
        ..SyncConfig::default()
    }
}

// ZB-2 INC-I-293 (Must, premise) — Decision: a failure means the mainnet pin is not 867_000 (or the signature/BLS gates are not crossed at the cell tips), so the three-height cells below do not test the operator's boundary.
#[test]
fn zb2_mainnet_gate_is_867_000_and_cells_isolate_it() {
    let c = mainnet_config();
    assert_eq!(
        c.inc_i_288_fork_choice_activation_height, MAINNET_GATE,
        "mainnet INC-I-288 fork-choice gate"
    );
    assert!(
        c.inc_i_236_block_signature_activation_height < TIP_OFF
            && c.inc_i_178_attestation_bls_activation_height < TIP_OFF,
        "the signature and BLS gates are crossed at every cell tip, so only INC-I-288 differs"
    );
}

// ---- walk intake fixtures (mirror fork_recovery_walk_intake_tests.rs) ----

struct WalkRig {
    m: SyncManager,
    peer: PeerId,
    seed: Block,
    /// The seed's parent: the walk's first by-hash fetch.
    target: Block,
    root: Hash,
}

fn walk_peer() -> PeerId {
    libp2p::identity::Keypair::ed25519_from_bytes([0x2au8; 32])
        .expect("fixed ed25519 seed")
        .public()
        .to_peer_id()
}

/// An honest signed block nobody attested: empty bitfield, empty aggregate.
fn honest_block(prev: Hash, slot: u32, kp: &KeyPair) -> Block {
    let mut t = Transaction::new_transfer(vec![], vec![]);
    t.extra_data = slot.to_le_bytes().to_vec();
    let txs = vec![t];
    let mut h = BlockHeader {
        version: 1,
        prev_hash: prev,
        merkle_root: doli_core::block::compute_merkle_root(&txs),
        presence_root: doli_core::presence_commitment(&[], &[]),
        genesis_hash: Hash::ZERO,
        timestamp: TS + slot as u64,
        slot,
        producer: *kp.public_key(),
        vdf_output: VdfOutput { value: vec![0; 32] },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    h.vdf_proof.pi =
        crypto::signature::sign(h.signing_message().as_bytes(), kp.private_key()).to_vec();
    Block::new(h, txs)
}

/// Same header (same hash), aggregate padded: the gated intake check refuses it.
fn padded(honest: &Block) -> Block {
    let mut b = honest.clone();
    b.aggregate_bls_signature
        .extend(std::iter::repeat_n(0x5Au8, PAD));
    b
}

/// Local tip at `tip` on mainnet params; a walk started from a signed seed whose
/// parent `target` sits on `root`.
fn walk_rig(tip: u64) -> WalkRig {
    let mut m = SyncManager::new(mainnet_config(), Hash::ZERO);
    let local_hash = Hash::from_bytes([0x11; 32]);
    m.update_local_tip(tip, local_hash, tip as u32);
    let peer = walk_peer();
    m.add_peer(peer, tip, local_hash, tip as u32);
    let kp = KeyPair::from_seed([0x6e; 32]);
    let root = Hash::from_bytes([0x33; 32]);
    let target = honest_block(root, 2_000, &kp);
    let seed = honest_block(target.hash(), 2_001, &kp);
    assert!(
        m.start_fork_recovery(seed.clone(), peer),
        "setup: walk starts"
    );
    WalkRig {
        m,
        peer,
        seed,
        target,
        root,
    }
}

/// The walk asks the walk peer for `want` (the fetch becomes pending).
fn ask(r: &mut WalkRig, want: Hash) {
    let (to, req) = r.m.next_request().expect("the walk issues its fetch");
    assert_eq!(to, r.peer, "the walk fetch goes to the walk peer");
    assert!(
        matches!(req, SyncRequest::GetBlockByHash { hash } if hash == want),
        "the walk asks for the seed's parent"
    );
}

// walk intake judges the block at local_height + 1; the gate is in block units, so tip 866_999 judges block 867_000
// ZB-2 INC-I-293 (Must) — Decision: a failure means reply_skips_walk on mainnet turns on at a different tip than 866_999 (next block 867_000), so the U-2 relayer rule is active one block early/late versus the documented unit, or it misfires on the walk's own fetch.
#[test]
fn zb2_reply_skips_walk_mainnet_tip_plus_one() {
    for (tip, active) in [(TIP_OFF, false), (TIP_EDGE, true), (TIP_ON, true)] {
        let mut r = walk_rig(tip);
        let want = r.target.hash();
        ask(&mut r, want);
        let other = SyncRequest::get_headers(want, 10);
        assert_eq!(
            r.m.reply_skips_walk(r.peer, Some(&other)),
            active,
            "F1 tip {tip} (next block {}): a reply to another request skips the walk only at/above the gate",
            tip + 1
        );
        let own = SyncRequest::GetBlockByHash { hash: want };
        assert!(
            !r.m.reply_skips_walk(r.peer, Some(&own)),
            "F1 tip {tip}: the reply to the walk's own by-hash fetch never skips"
        );
        assert!(
            !r.m.reply_skips_walk(r.peer, None),
            "F1 tip {tip}: an unknown request keeps the old rule"
        );
    }
}

// walk intake judges the block at local_height + 1; the gate is in block units, so tip 866_999 judges block 867_000
// ZB-2 INC-I-293 (Must) — Decision: a failure means the walk-intake body check (fork_walk.rs `gated`) on mainnet turns on at a different tip than 866_999, so a padded walk body is held (or refused and blamed) on the wrong side of the documented boundary.
#[test]
fn zb2_walk_intake_gated_check_mainnet_tip_plus_one() {
    for (tip, gated) in [(TIP_OFF, false), (TIP_EDGE, true), (TIP_ON, true)] {
        let mut r = walk_rig(tip);
        let honest = r.target.clone();
        let bad = padded(&honest);
        assert_eq!(
            bad.hash(),
            honest.hash(),
            "setup: padding is outside the hash"
        );
        ask(&mut r, honest.hash());
        assert!(
            r.m.walk_intake(r.peer, Some(bad.clone())),
            "F2 O1 tip {tip}: the pending answer is consumed by the walk"
        );
        let blamed = r.m.take_refused_body_suppliers();
        if gated {
            assert_eq!(
                r.m.fork_walk_pending_fetch(),
                Some((r.peer, honest.hash())),
                "F2 O2 tip {tip} (next block {}): refused, the fetch stays pending on the same hash",
                tip + 1
            );
            assert_eq!(
                r.m.fork_recovery_current_parent(),
                None,
                "F2 O2 tip {tip}: the walk did not advance past the refused body"
            );
            assert_eq!(
                blamed,
                vec![(r.peer, honest.header.slot)],
                "F2 O3 tip {tip}: the walk peer is blamed for the refused body"
            );
        } else {
            assert_eq!(
                r.m.fork_walk_pending_fetch(),
                None,
                "F2 O2 tip {tip} (next block {}): below the gate the answer is taken",
                tip + 1
            );
            assert_eq!(
                r.m.fork_recovery_current_parent(),
                Some(r.root),
                "F2 O2 tip {tip}: below the gate the walk advances past it"
            );
            assert!(blamed.is_empty(), "F2 O3 tip {tip}: nobody blamed");
            let done =
                r.m.check_fork_recovery_connection(true)
                    .expect("F2 O4: below-gate walk completes");
            assert_eq!(
                done.blocks,
                vec![bad, r.seed.clone()],
                "F2 O4 tip {tip}: the padded body reaches fork choice below the gate"
            );
        }
    }
}

// ---- snap descent fixtures (mirror tests_inc_i289_m1_snap_descent.rs) ----

fn header(prev_hash: Hash, slot: u32) -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_hash,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: slot as u64,
        slot,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: VdfOutput { value: vec![0; 32] },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    }
}

/// Headers FH+1..=FH+n on `f`.
fn chain(f: Hash, n: u64) -> Vec<BlockHeader> {
    let mut prev = f;
    (1..=n)
        .map(|i| {
            let hd = header(prev, (FH + i) as u32);
            prev = hd.hash();
            hd
        })
        .collect()
}

/// Local tip `tip` on mainnet params, finalized checkpoint `f` at FH, snap
/// download toward `anchor` from one peer.
fn snap_rig(tip: u64, f: Hash, anchor: (u64, Hash)) -> (SyncManager, PeerId) {
    let mut m = SyncManager::new(mainnet_config(), Hash::ZERO);
    m.local_height = tip;
    m.restore_finality(FinalityCheckpoint {
        block_hash: f,
        height: FH,
        slot: FH as u32,
        attestation_weight: 1,
        total_weight: 1,
    });
    let peer = PeerId::random();
    m.add_peer(peer, anchor.0, anchor.1, anchor.0 as u32);
    m.pipeline_data = SyncPipelineData::SnapDownloading {
        target_hash: anchor.1,
        target_height: anchor.0,
        quorum_root: Hash::ZERO,
        peer,
        alternate_peers: vec![],
    };
    m.state = SyncState::Syncing {
        phase: SyncPhase::SnapDownloading,
        started_at: Instant::now(),
    };
    (m, peer)
}

fn is_first_batch(req: &SyncRequest, f: Hash) -> bool {
    matches!(req, SyncRequest::GetHeaders { start_hash, max_count }
        if *start_hash == f && *max_count == 500)
}

// snap descent judges the local tip itself (no + 1): tip 866_999 is below the gate, tip 867_000 is at it
// ZB-2 INC-I-293 (Must) — Decision: a failure means mainnet snap sync asks for (or skips) the finality descent proof on the wrong side of tip 867_000, so a node either downloads a snapshot that does not descend from its finalized block or falls back to block sync before the pin.
#[test]
fn zb2_snap_descent_proof_mainnet_local_tip() {
    let f = crypto::hash::hash(b"zb2-f");
    let headers = chain(f, SPAN);
    let anchor = (FH + SPAN, headers.last().expect("chain").hash());
    let far = (
        FH + SNAP_DESCENT_PROOF_MAX_HEADERS + 1,
        Hash::from_bytes([0x77; 32]),
    );
    for (tip, required) in [(TIP_OFF, false), (TIP_EDGE, false), (TIP_ON, true)] {
        let (mut m, peer) = snap_rig(tip, f, anchor);
        let narrow = m.snap_descent_next_request(anchor.0, anchor.1);
        let dispatched = m.next_request();
        if required {
            assert!(
                matches!(&narrow, Ok(Some(r)) if is_first_batch(r, f)),
                "F3 tip {tip}: at the gate the first proof batch is asked from f: {narrow:?}"
            );
            assert!(
                matches!(&dispatched, Some((to, r)) if *to == peer && is_first_batch(r, f)),
                "F3 tip {tip}: the dispatcher sends the proof batch before the manifest: {dispatched:?}"
            );
        } else {
            assert!(
                matches!(narrow, Ok(None)),
                "F3 tip {tip}: below the gate no proof is asked: {narrow:?}"
            );
            assert!(
                matches!(&dispatched, Some((to, SyncRequest::GetStateManifest { .. })) if *to == peer),
                "F3 tip {tip}: below the gate the manifest goes out directly: {dispatched:?}"
            );
        }
        assert_eq!(
            m.accept_snap_descent_chain(anchor.0, anchor.1, &headers),
            required,
            "F3 tip {tip}: a served chain is accepted only when a proof is required"
        );
        assert_eq!(
            m.snap_descent_proven(anchor.0, anchor.1, f),
            required,
            "F3 tip {tip}: the proof is recorded only when required"
        );

        let (mut over, _) = snap_rig(tip, f, far);
        let over_span = over.snap_descent_next_request(far.0, far.1);
        if required {
            assert!(
                matches!(over_span, Err(())),
                "F3 tip {tip}: at the gate an anchor over the bound falls back before any download: {over_span:?}"
            );
        } else {
            assert!(
                matches!(over_span, Ok(None)),
                "F3 tip {tip}: below the gate the span bound does not apply: {over_span:?}"
            );
        }
    }
}

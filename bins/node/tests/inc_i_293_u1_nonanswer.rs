//! INC-I-293 run 651 U-1 (operator ruling 2, AUDIT-P2-649-001): one rule for every
//! non-answer of a by-hash walk peer. At c041859e only a `None` answer marks the
//! walk peer and hands the same walk to the next evidence peer
//! (`fork_recovery.rs:232-246`). Silence waits REQUEST_TIMEOUT and then fails over
//! to the tallest alternate (`fork_recovery.rs:176-192`, "failing over to"); a
//! refused body (`walk_intake` per-hop `body_matches_header`) is blamed and the
//! walk keeps waiting on the same peer until that timeout; a junk answer (a block
//! that is not the requested hash) passes through `handle_block` and the walk keeps
//! waiting too. The ruling: every non-answer marks the peer (never asked again in
//! this walk) and hands the same walk to the next evidence peer before any
//! alternate. Only with no evidence peer left may the walk fall back or cancel.
//!
//! The cells drive the production intake (`next_request` -> `on_sync_response`)
//! with mock peers, as `inc_i_293_s2_walk_peer.rs` does. REQUEST_TIMEOUT is a
//! private 10 s `std::time::Instant` deadline, so each cell sleeps past it once
//! in real time (the cells run in parallel; the binary takes about 13 s).
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(orphan R3, sender) (walk start),
//   fn SyncManager::next_request() (walk fetch, F11 failover), fn
//   Node::on_sync_response(peer, Block(None|Some)) -> walk_intake -> handle_block
//   O1: every outbound GetBlockByHash of the walk: target peer, hash, round, answer
//   O2: the first by-hash fetch after the walk peer's non-answer: peer and hash
//   O3: the non-answering peer: asked again by the walk after its non-answer
//   O4: walk completion (check_fork_recovery_connection) handed to fork choice
// PATHS: S (P_e1 silent past REQUEST_TIMEOUT) -> u1a_silence red; B (P_e1 answers
//   R2 with a body that fails its merkle root) -> u1a_refused red; J (P_e1 answers
//   a block that is not R2) -> u1a_junk red; N (P_e1 answers None) -> control;
//   S below AH (Wedged walk on P_tall, silent; alternate P_tall2) -> pin
// INPUT PARTITIONS: AH = 0 (devnet, above) | AH = 1e9 (below); non-answer kind
//   {silence, refused body, junk, None}; evidence peers P_e1, P_e2 (F+3, tip R3)
//   below the local height; P_tall (F+6, our tip) the only alternate above it
// MATRIX: S x O1 O2 O3 O4 -> u1a_silence · B x O1 O2 O3 O4 -> u1a_refused
//   · J x O1 O2 O3 O4 -> u1a_junk · N x O1 O2 O3 O4 -> control · S(below) x O1 O4 -> pin
//! covers: fork_recovery (next_fetch, handle_block) sync/manager/fork_walk
//! (walk_intake) node/fork_walk (evidence_peers) network_events (on_sync_response)

#[path = "support/block_sig.rs"]
mod block_sig;
#[path = "it/inc_i_289_common.rs"]
mod inc_i_289_common;

use std::collections::HashSet;
use std::time::Duration;

use crypto::{Hash, KeyPair};
use doli_core::{Block, Network, NetworkParams, Transaction};
use inc_i_289_common::*;
use network::protocols::{SyncRequest, SyncResponse};
use network::PeerId;

/// REQUEST_TIMEOUT (`fork_recovery.rs:28`, 10 s) plus a margin.
const PAST_REQUEST_TIMEOUT: Duration = Duration::from_millis(11_500);

fn short(h: &Hash) -> String {
    h.to_string()[..8].to_string()
}

fn assert_above_gate() {
    let ah = NetworkParams::load(Network::Devnet).inc_i_288_fork_choice_activation_height;
    assert!(ah <= F, "precondition: devnet AH {ah} is at or below F");
}

#[derive(Clone, Copy, PartialEq)]
enum Tip {
    /// Our own top D5 (canonical).
    Ours,
    /// R3, off our chain.
    R3,
}

/// How a mock peer answers every by-hash fetch it gets.
#[derive(Clone, Copy, PartialEq)]
enum Answer {
    /// The requested R block (None when it is not an R block).
    Serve,
    NoBlock,
    /// Never answers.
    Silent,
    /// The requested R block with a body that fails its header's merkle root.
    Refused,
    /// A block that is not the requested hash (our own D1).
    Junk,
}

impl Answer {
    fn label(self) -> &'static str {
        match self {
            Answer::Serve => "block",
            Answer::NoBlock => "None",
            Answer::Silent => "silent",
            Answer::Refused => "refused body",
            Answer::Junk => "junk",
        }
    }
}

struct PeerSpec {
    name: &'static str,
    height: u64,
    tip: Tip,
    answer: Answer,
}

const fn spec(name: &'static str, height: u64, tip: Tip, answer: Answer) -> PeerSpec {
    PeerSpec {
        name,
        height,
        tip,
        answer,
    }
}

enum Trigger {
    Orphan(&'static str),
    Wedged,
}

struct Walk {
    r: Rig,
    /// R1..R3 at F+1..F+3.
    rb: Vec<Block>,
    peers: Vec<(PeerId, &'static str, Answer)>,
}

struct Fetch {
    round: usize,
    peer: &'static str,
    hash: Hash,
    answer: &'static str,
}

#[derive(Default)]
struct Obs {
    fetched: Vec<Fetch>,
    completed: bool,
    active_at_end: bool,
}

fn walk_keys() -> Vec<KeyPair> {
    (1..=3u8).map(|i| KeyPair::from_seed([i; 32])).collect()
}

fn is_led_by(keys: &[KeyPair], s: u32, by: &[usize]) -> bool {
    let l = leader(keys, s).public_key();
    by.iter().any(|i| keys[*i].public_key() == l)
}

/// D1..D5 (keys[0], keys[1]) applied on F; R1..R3 on F in the 3 slots after D5;
/// `peers` registered with their reported (height, tip).
async fn arrange(peers: &[PeerSpec]) -> Walk {
    let keys = walk_keys();
    let d_slots: Vec<u32> = (F as u32 + 1..)
        .filter(|s| is_led_by(&keys, *s, &[0, 1]))
        .take(5)
        .collect();
    let r = rig_with_keys(keys, &d_slots).await;
    let d5 = *d_slots.last().unwrap();
    let rb = chain_on(&r, r.f_hash, F + 1, &[d5 + 1, d5 + 2, d5 + 3]);
    let r_by: HashSet<_> = rb.iter().map(|b| b.header.producer).collect();
    assert_eq!(r_by.len(), 3, "fixture: R by 3 producers");
    let top = r.ours.last().unwrap().clone();
    let mut ids = Vec::new();
    for p in peers {
        let (h, s) = match p.tip {
            Tip::Ours => (top.hash(), top.header.slot),
            Tip::R3 => (rb[2].hash(), rb[2].header.slot),
        };
        ids.push((peer_at(&r, p.height, h, s).await, p.name, p.answer));
    }
    Walk { r, rb, peers: ids }
}

fn id(w: &Walk, name: &str) -> PeerId {
    (w.peers.iter())
        .find(|p| p.1 == name)
        .expect("fixture peer")
        .0
}

/// `want` with a body its header does not commit to (the hash is unchanged).
fn refused_copy(want: &Block) -> Block {
    let mut bad = want.clone();
    let pool = doli_core::consensus::reward_pool_pubkey_hash();
    bad.transactions = vec![Transaction::new_coinbase(1, pool, 0, 0)];
    assert!(
        bad.hash() == want.hash() && !bad.verify_merkle_root(),
        "fixture: the refused body keeps the hash and fails the merkle root"
    );
    bad
}

/// One drive of the production intake: every by-hash fetch is answered as its
/// target peer's `Answer` says; a connected walk is handed to fork choice.
async fn round(w: &mut Walk, round: usize, obs: &mut Obs) {
    for _ in 0..8 {
        let req = w.r.node.sync_manager.write().await.next_request();
        let Some((q, req)) = req else { break };
        let SyncRequest::GetBlockByHash { hash } = req else {
            continue;
        };
        let (name, answer) = (w.peers.iter())
            .find(|p| p.0 == q)
            .map_or(("other", Answer::NoBlock), |p| (p.1, p.2));
        let want = w.rb.iter().find(|b| b.hash() == hash).cloned();
        obs.fetched.push(Fetch {
            round,
            peer: name,
            hash,
            answer: answer.label(),
        });
        let reply = match answer {
            Answer::Silent => continue,
            Answer::Serve => want,
            Answer::NoBlock => None,
            Answer::Refused => want.as_ref().map(refused_copy),
            Answer::Junk => Some(w.r.ours[0].clone()),
        };
        (w.r.node
            .on_sync_response(q, SyncResponse::Block(reply))
            .await)
            .expect("on_sync_response returns Ok");
        let parent = (w.r.node.sync_manager.read().await).fork_recovery_current_parent();
        let canonical = parent
            .is_some_and(|p| matches!(w.r.node.block_store.get_height_by_hash(&p), Ok(Some(_))));
        if canonical {
            let done = (w.r.node.sync_manager.write().await).check_fork_recovery_connection(true);
            if let Some(done) = done {
                obs.completed = true;
                let res = w.r.node.handle_completed_fork_recovery(done).await;
                eprintln!("[U1] completed walk -> {res:?}");
            }
        }
    }
}

/// Trigger, one round, sleep past REQUEST_TIMEOUT, two more rounds.
async fn walk(w: &mut Walk, trigger: Trigger) -> Obs {
    let mut obs = Obs::default();
    match trigger {
        Trigger::Orphan(from) => {
            let (p, r3) = (id(w, from), w.rb[2].clone());
            (w.r.node.handle_new_block(r3, p).await).expect("handle_new_block returns Ok");
        }
        Trigger::Wedged => {
            let r3 = w.rb[2].clone();
            (w.r.node.fork_block_cache.write().await).insert(r3.hash(), r3);
            assert!(
                w.r.node.try_trigger_fork_recovery().await,
                "precondition: the Wedged entry starts a walk from the cached R3"
            );
        }
    }
    round(w, 1, &mut obs).await;
    tokio::time::sleep(PAST_REQUEST_TIMEOUT).await;
    round(w, 2, &mut obs).await;
    round(w, 3, &mut obs).await;
    obs.active_at_end = (w.r.node.sync_manager.read().await).is_fork_recovery_active();
    obs
}

fn describe(w: &Walk, obs: &Obs) -> String {
    let f: Vec<String> = (obs.fetched.iter())
        .map(|x| {
            let h = short(&x.hash);
            format!("round {}: {} <- {h} ({})", x.round, x.peer, x.answer)
        })
        .collect();
    format!(
        "by-hash fetches: [{}]; R1 {} R2 {}; walk completed {}, active at end {}",
        f.join(", "),
        short(&w.rb[0].hash()),
        short(&w.rb[1].hash()),
        obs.completed,
        obs.active_at_end
    )
}

const E2: PeerSpec = spec("P_e2", F + 3, Tip::R3, Answer::Serve);
/// Taller than our F+5 tip: the walk's only F11 alternate above the gate.
const TALL: PeerSpec = spec("P_tall", F + 6, Tip::Ours, Answer::NoBlock);

/// The orphan walk from P_e1 (answering `answer`) with P_e2 and P_tall: after
/// P_e1's non-answer the same walk asks P_e2 for the same hash before any
/// alternate, never asks P_e1 again, and completes from P_e2's blocks.
async fn assert_hands_over(answer: Answer, what: &str, label: &str) {
    assert_above_gate();
    let e1 = spec("P_e1", F + 3, Tip::R3, answer);
    let mut w = arrange(&[e1, E2, TALL]).await;
    let obs = walk(&mut w, Trigger::Orphan("P_e1")).await;
    let roots = [w.rb[0].hash(), w.rb[1].hash()];
    let first = (obs.fetched.iter()).position(|x| roots.contains(&x.hash));
    assert!(
        first.is_some_and(|i| obs.fetched[i].peer == "P_e1"),
        "{label} precondition: the orphan walk asks its sender P_e1 first: {}",
        describe(&w, &obs)
    );
    let i = first.unwrap();
    let asked_hash = obs.fetched[i].hash;
    let next = obs.fetched.get(i + 1);
    let got = next.map(|x| x.peer);
    assert!(
        got == Some("P_e2") && next.is_some_and(|x| x.hash == asked_hash),
        "{label}: after P_e1 {what} the walk asked {got:?} (hash {}), expected the next \
         evidence peer P_e2 for the same hash {}; {}",
        next.map_or("-".to_string(), |x| short(&x.hash)),
        short(&asked_hash),
        describe(&w, &obs)
    );
    let again = obs.fetched.iter().skip(i + 1).any(|x| x.peer == "P_e1");
    assert!(
        !again,
        "{label}: P_e1 {what} but the walk asked it again (not marked): {}",
        describe(&w, &obs)
    );
    assert!(
        obs.completed && !(obs.fetched.iter()).any(|x| x.peer == "P_tall"),
        "{label}: after the hand-over the same walk did not complete from P_e2 without \
         asking an alternate: {}",
        describe(&w, &obs)
    );
}

// ------------------------------------------------------------------ reds (Must)

// U-1 INC-I-293 run 651 AUDIT-P2-649-001 (Must) — Decision: a failure means a silent evidence peer still makes the walk fail over to the tallest peer (our own lighter branch, answers None) instead of the next evidence peer that showed the branch, so a silent peer costs a request timeout and a wasted ask of the wrong side.
#[tokio::test]
async fn u1a_silence_hands_over_to_next_evidence_peer() {
    assert_hands_over(Answer::Silent, "stayed silent", "u1a silence (Must)").await;
}

// U-1 INC-I-293 run 651 AUDIT-P2-649-001 (Must) — Decision: a failure means a walk peer whose body fails the per-hop header check keeps the walk (it waits out the request timeout on that peer, then asks an alternate), so one junk-body peer stalls the heal instead of the walk moving to the next evidence peer.
#[tokio::test]
async fn u1a_refused_body_hands_over_to_next_evidence_peer() {
    let what = "answered a body that fails its header";
    assert_hands_over(Answer::Refused, what, "u1a refused body (Must)").await;
}

// U-1 INC-I-293 run 651 AUDIT-P2-649-001 (Must) — Decision: a failure means a walk peer that answers a block other than the requested hash still holds the walk until the request timeout and then loses it to an alternate, so a peer can stall the walk with junk answers.
#[tokio::test]
async fn u1a_junk_answer_hands_over_to_next_evidence_peer() {
    let what = "answered a block that is not the requested hash";
    assert_hands_over(Answer::Junk, what, "u1a junk (Must)").await;
}

// ------------------------------------------------------------- control (passes today)

// U-1 INC-I-293 run 651 control — Decision: a failure means the None hand-over itself (run 649 S-2) broke in this rig, so the three reds would not isolate the non-None non-answers.
#[tokio::test]
async fn u1a_none_still_hands_over() {
    assert_hands_over(Answer::NoBlock, "answered None", "u1a None control").await;
}

// ----------------------------------------------------------- below-gate pin (INV-12)

// U-1 INC-I-293 run 651 below-gate pin — Decision: a failure means today's below-AH (mainnet) silence failover changed: below the gate the Wedged walk asks the tallest peer P_tall, after its silence fails over to the F11 alternate P_tall2, whose None ends the walk, and never asks an evidence peer; a gated fix must keep that bit-identical.
#[tokio::test]
async fn u1a_below_gate_old_failover_pinned() {
    if !in_below_ah_child("u1a_below_gate_old_failover_pinned") {
        return;
    }
    let tall = spec("P_tall", F + 8, Tip::Ours, Answer::Silent);
    let tall2 = spec("P_tall2", F + 7, Tip::Ours, Answer::NoBlock);
    let e1 = spec("P_e1", F + 3, Tip::R3, Answer::Serve);
    let mut w = arrange(&[e1, E2, tall, tall2]).await;
    let obs = walk(&mut w, Trigger::Wedged).await;
    let seq: Vec<(&str, usize)> = obs.fetched.iter().map(|x| (x.peer, x.round)).collect();
    assert_eq!(
        seq,
        vec![("P_tall", 1), ("P_tall2", 2)],
        "u1a pin: below the gate the silence failover changed: {}",
        describe(&w, &obs)
    );
    assert!(
        obs.fetched.iter().all(|x| x.hash == w.rb[1].hash()) && !obs.active_at_end,
        "u1a pin: below the gate the walk asks R2 twice and ends on P_tall2's None: {}",
        describe(&w, &obs)
    );
    assert_ne!(
        tip_hash(&w.r).await,
        w.rb[2].hash(),
        "u1a pin: below gate on R3"
    );
}

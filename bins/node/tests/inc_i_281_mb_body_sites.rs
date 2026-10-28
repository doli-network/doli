// INC-I-281 run 620 M-B D2 — the remaining body sites use the one owner (body_retry): a body copy
// is untrusted until apply succeeds, and a body failure never condemns its header.
//
// covers: body_retry fork_walk force_reorg block_handling fork_cache fork_recovery reorg_refusals body_commitments reorg_restore network_events periodic body_auth body_intake types response
//
// OUTPUT CONTRACT: node on [.., F, D_B, D_{B+1}] (boundary fixture), or at A (h=6, INC-I-236
//   fixture) for GetBodies. Functions: Node::handle_new_block, handle_completed_fork_recovery,
//   try_consume_force_reorg, on_sync_response.
// OUTPUTS:
//   O1 chain_state best_hash / best_height, canonical hashes, persisted tip
//   O2 stored body of each honest block
//   O3 bls_ingress_scorer InvalidBlock record on the failed-copy supplier; none on honest peers
//   O4 `[BODY_REFETCH]` lines: asked peer + hash; never a peer whose copy of that hash failed
//   O5 refused set ("refused by a reorg before") on the honest blocks
//   O6 `is a learned anchor` lines; O7 LB-9 memo is_fork_branch_rejected(honest tip, local)
//   O8 `Reorg apply_block failed` / `[REORG_RESTORE]` lines
// PATHS: relay_junk = junk then the honest copy, both from relay R by gossip | walk_replay = a
//   walk that replays a refused variant, then the honest walk | force_reorg_stored = forceReorgTo
//   over a stored honest body with a cached variant | extends_tip = a tip-extending variant
//   first, the honest copy only by refetch | getbodies_junk = a GetBodies reply with a junk body.
// INPUT PARTITIONS: commitment junk (attestation bit, presence variant) | input-signature variant
//   (passes the commitments, fails apply).
// MATRIX: relay_junk O1-O8 | walk_replay O1 O3-O5 O7 | force_reorg_stored O1 O2 O8 |
//   extends_tip O1-O4 | getbodies_junk O1 O3.

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;
mod inc_i_281_boundary_fx;
mod inc_i_281_m2_support;

use crypto::{Hash, KeyPair};
use doli_core::transaction::{Input, Output, OutputType, Transaction, TxType};
use doli_core::Block;
use doli_node::node::{ForceReorgOutcome, Node};
use inc_i_236_m2_reorg_support as r236;
use inc_i_281_boundary_fx::*;
use inc_i_281_m2_support::Logs;
use network::protocols::{SyncRequest, SyncResponse};
use network::sync::CompletedRecovery;
use network::{Infraction, PeerId};
use storage::{Outpoint, UtxoEntry};

const FUNDING: u64 = 100_000;
const REFETCH: &str = "[BODY_REFETCH]";

fn probe(row: &str, wrong: &[String]) {
    println!(
        "INC281_PROBE bodyretry={row} wrong={}",
        u8::from(!wrong.is_empty())
    );
}

fn short(h: &Hash) -> String {
    h.to_hex()[..8].to_string()
}

fn prefix(h: &Hash) -> String {
    h.to_hex()[..16].to_string()
}

fn logged() -> (Logs, tracing::subscriber::DefaultGuard) {
    let logs = Logs::default();
    let g = tracing::subscriber::set_default(logs.subscriber());
    (logs, g)
}

async fn setup() -> Fx {
    let fx = fixture_at(post_genesis_offset()).await;
    fx.assert_past_genesis();
    fx
}

fn presence_root(ah: u64, height: u64, bf: &[u8], agg: &[u8]) -> Hash {
    if height >= ah {
        doli_core::presence_commitment(bf, agg)
    } else if bf.is_empty() {
        Hash::ZERO
    } else {
        crypto::hash::hash(bf)
    }
}

fn rebody(fx: &Fx, mut b: Block, height: u64, bf: &[u8], txs: &[Transaction]) -> Block {
    b.transactions.extend_from_slice(txs);
    b.attestation_bitfield = bf.to_vec();
    b.header.merkle_root = doli_core::block::compute_merkle_root(&b.transactions);
    let ah = fx.node.inc_i_178_attestation_bls_activation_height;
    b.header.presence_root = presence_root(ah, height, bf, &[]);
    let by = fx.key(&b.header.producer);
    block_sig::sign_header(&mut b.header, by);
    b
}

/// [C_B, C_{B+1}, C_{B+2}]; C_{B+1} carries `bf` and `txs`.
fn honest(fx: &Fx, bf: &[u8], txs: &[Transaction]) -> Vec<Block> {
    let b = fx.b();
    let by1 = fx.key(&at(&fx.l_canon, fx.s1));
    let c1 = rebody(fx, fx.mk(fx.c_b.hash(), b + 1, fx.s1, by1), b + 1, bf, txs);
    let by2 = fx.key(&at(&fx.l_canon, fx.s2));
    let c2 = rebody(fx, fx.mk(c1.hash(), b + 2, fx.s2, by2), b + 2, bf, &[]);
    vec![fx.c_b.clone(), c1, c2]
}

fn pkh(k: &KeyPair) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, k.public_key().as_bytes())
}

fn normal(owner: &KeyPair, amount: u64) -> Output {
    Output {
        amount,
        pubkey_hash: pkh(owner),
        output_type: OutputType::Normal,
        lock_until: 0,
        extra_data: vec![],
    }
}

/// A funded spend by a fresh owner.
async fn spend(fx: &Fx) -> Transaction {
    let owner = KeyPair::generate();
    let op = Outpoint::new(crypto::hash::hash(b"inc-i-281-mb-sites-funding"), 0);
    let entry = UtxoEntry {
        output: normal(&owner, FUNDING),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    (fx.node.utxo_set.write().await)
        .insert(op, entry.clone())
        .expect("fixture: fund in memory");
    fx.node
        .state_db
        .insert_utxo(&op, &entry)
        .expect("fixture: fund state_db");
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input {
            prev_tx_hash: op.tx_hash,
            output_index: op.index,
            signature: crypto::Signature::default(),
            sighash_type: doli_core::transaction::SighashType::All,
            committed_output_count: 0,
            public_key: Some(*owner.public_key()),
        }],
        outputs: vec![normal(&owner, FUNDING - 1_000)],
        extra_data: vec![],
    };
    let sighash = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&sighash, owner.private_key());
    tx
}

/// `b` with one byte of its spend's input signature flipped: same hash and commitments.
fn sig_flip(b: &Block) -> Block {
    let mut v = b.clone();
    let input = &mut v.transactions[1].inputs[0];
    let mut s = *input.signature.as_bytes();
    s[0] ^= 1;
    input.signature = crypto::Signature::from_bytes(s);
    assert_eq!(v.hash(), b.hash(), "fixture: same header hash");
    assert!(v.verify_merkle_root(), "fixture: the merkle root holds");
    assert_ne!(v, *b, "fixture: a different body");
    v
}

async fn register(fx: &Fx, tip: &Block, height: u64, peers: &[PeerId]) {
    let mut sm = fx.node.sync_manager.write().await;
    for p in peers {
        sm.add_peer(*p, height, tip.hash(), tip.header.slot);
    }
}

async fn deliver(fx: &mut Fx, b: &Block, from: PeerId) -> Option<String> {
    let res = fx.node.handle_new_block(b.clone(), from).await;
    res.err()
        .map(|e| format!("handle_new_block of {} returned Err: {e}", short(&b.hash())))
}

/// Answers each `[BODY_REFETCH]` line with the honest block it names, from the asked peer.
async fn serve(fx: &mut Fx, logs: &Logs, peers: &[PeerId], honest: &[Block]) -> Vec<String> {
    let (mut wrong, mut seen) = (Vec::new(), 0);
    for _ in 0..10 {
        let lines = logs.lines();
        let new: Vec<String> = lines[seen..]
            .iter()
            .filter(|l| l.contains(REFETCH))
            .cloned()
            .collect();
        seen = lines.len();
        if new.is_empty() {
            break;
        }
        for line in new {
            let peer = peers.iter().find(|p| line.contains(&p.to_string()));
            let block = honest.iter().find(|b| line.contains(&prefix(&b.hash())));
            match (peer, block) {
                (Some(p), Some(b)) => wrong.extend(deliver(fx, &b.clone(), *p).await),
                _ => wrong.push(format!(
                    "refetch names no registered peer or honest hash: {}",
                    line.chars().take(200).collect::<String>()
                )),
            }
        }
    }
    wrong
}

/// `[BODY_REFETCH]` lines from index `from` that ask `peer` for `hash`.
fn asked_of(logs: &Logs, from: usize, hash: &Hash, peer: &PeerId) -> usize {
    let (p, id) = (prefix(hash), peer.to_string());
    logs.lines()[from..]
        .iter()
        .filter(|l| l.contains(REFETCH) && l.contains(&p) && l.contains(&id))
        .count()
}

fn penalized(node: &Node, peer: &PeerId) -> bool {
    node.bls_ingress_scorer.get_score(peer).is_some_and(|s| {
        s.infractions
            .iter()
            .any(|(_, i)| matches!(i, Infraction::InvalidBlock { .. }))
    })
}

/// `Some` unless `br` is canonical from B on, its last block is the persisted tip, and every
/// stored body is the honest one.
async fn landed(fx: &Fx, br: &[Block]) -> Option<String> {
    let st = fx.state().await;
    let tip = br.last().unwrap().hash();
    let store = &fx.node.block_store;
    let off = br.iter().enumerate().find(|(i, b)| {
        let h = fx.b() + *i as u64;
        store.get_hash_by_height(h).expect("store") != Some(b.hash())
            || store.get_block(&b.hash()).expect("store").as_ref() != Some(*b)
    });
    let ok = st.best_hash == tip && st.persisted == Some(tip) && off.is_none();
    (!ok).then(|| {
        format!(
            "the honest branch did not land: best={} h={} first_off={:?}",
            short(&st.best_hash),
            st.best_height,
            off.map(|(i, _)| i)
        )
    })
}

async fn refused(fx: &Fx, blocks: &[Block]) -> Vec<String> {
    let mut wrong = Vec::new();
    for b in blocks {
        if let Err(e) = fx.node.check_producer_eligibility(b).await {
            if e.to_string().contains("refused by a reorg before") {
                wrong.push(format!("honest {} is in the refused set", short(&b.hash())));
            }
        }
    }
    wrong
}

async fn marked(fx: &Fx, tip: &Block) -> bool {
    let best = fx.state().await.best_hash;
    let sm = fx.node.sync_manager.read().await;
    [fx.d[0].hash(), fx.d[1].hash(), fx.f_hash, best]
        .iter()
        .any(|local| sm.is_fork_branch_rejected(tip.hash(), *local))
}

fn restores(logs: &Logs) -> Vec<String> {
    let n = logs.mentioning("[REORG_RESTORE]") + logs.mentioning("Reorg apply_block failed");
    (n > 0)
        .then(|| format!("{n} x apply failure / restore lines"))
        .into_iter()
        .collect()
}

// G-2 row 1 (Must) — Decision: a failure means a commitment-junk copy relayed by an honest peer makes the node refuse that relay's later honest copy, ask that relay for the block, or refuse, anchor or mark the honest branch, so junk relayed by honest gossip locks the slot.
#[tokio::test]
async fn relay_junk_then_honest_copy() {
    let mut fx = setup().await;
    let br = honest(&fx, &[0b01], &[]);
    let (relay, h1) = (PeerId::random(), PeerId::random());
    register(&fx, &br[2], fx.b() + 2, &[relay, h1]).await;
    let mut junk = br[1].clone();
    junk.attestation_bitfield[0] ^= 0b10;
    assert_eq!(junk.hash(), br[1].hash(), "fixture: same header hash");
    assert!(!junk.body_matches_header(fx.node.inc_i_178_attestation_bls_activation_height, None));
    let (logs, g) = logged();
    let mut wrong = Vec::new();
    wrong.extend(deliver(&mut fx, &br[0], relay).await);
    let _ = deliver(&mut fx, &junk, relay).await;
    wrong.extend(deliver(&mut fx, &br[1], relay).await);
    wrong.extend(deliver(&mut fx, &br[2], relay).await);
    if let Some(w) = landed(&fx, &br).await {
        wrong.push(format!(
            "before any refetch reply, relay R's honest copy: {w}"
        ));
    }
    wrong.extend(serve(&mut fx, &logs, &[relay, h1], &br).await);
    drop(g);
    wrong.extend(landed(&fx, &br).await);
    if asked_of(&logs, 0, &br[1].hash(), &relay) > 0 {
        wrong.push("relay R was asked for C_B+1".into());
    }
    if !penalized(&fx.node, &relay) {
        wrong.push("relay R holds no InvalidBlock record for its junk copy".into());
    }
    if penalized(&fx.node, &h1) {
        wrong.push("honest H1 holds an InvalidBlock record".into());
    }
    wrong.extend(refused(&fx, &br[1..]).await);
    let anchors = logs.mentioning("is a learned anchor");
    if anchors > 0 {
        wrong.push(format!("{anchors} x learned anchor"));
    }
    if marked(&fx, &br[2]).await {
        wrong.push("LB-9 memo holds C_B+2".into());
    }
    wrong.extend(restores(&logs));
    probe("relay_junk", &wrong);
    assert!(
        wrong.is_empty(),
        "junk then honest copy from relay R: {wrong:?}"
    );
}

// G-2 row 2 (Must) — Decision: a failure means a walk that replays a body content a reorg refused marks the honest branch tip in the LB-9 memo, leaves its walk supplier unpenalized or asks no other peer, so one replay keeps the node off the honest branch.
#[tokio::test]
async fn walk_replays_a_refused_variant() {
    let mut fx = setup().await;
    let tx = spend(&fx).await;
    let br = honest(&fx, &[], &[tx]);
    let [a, r, h1, h2] = [(); 4].map(|_| PeerId::random());
    register(&fx, &br[2], fx.b() + 2, &[a, r, h1, h2]).await;
    let v = sig_flip(&br[1]);
    let (logs, g) = logged();
    let _ = deliver(&mut fx, &br[0], h1).await;
    let _ = deliver(&mut fx, &v, a).await;
    let _ = deliver(&mut fx, &br[2], h1).await;
    let pre = fx.node.check_producer_eligibility(&v).await;
    assert!(
        pre.is_err_and(|e| e.to_string().contains("refused by a reorg before")),
        "precondition: a reorg refused the variant's content"
    );
    let from = logs.lines().len();
    let mut wrong = Vec::new();
    let replay = CompletedRecovery {
        blocks: vec![br[0].clone(), v.clone(), br[2].clone()],
        connection_point: fx.f_hash,
        suppliers: vec![Some(r); 3],
    };
    if let Err(e) = fx.node.handle_completed_fork_recovery(replay).await {
        wrong.push(format!("replay walk returned Err: {e}"));
    }
    if marked(&fx, &br[2]).await {
        wrong.push("LB-9 memo holds the honest tip C_B+2 after the replay walk".into());
    }
    if !penalized(&fx.node, &r) {
        wrong.push("walk supplier R of the replayed variant holds no InvalidBlock record".into());
    }
    let c1 = br[1].hash();
    if asked_of(&logs, from, &c1, &h1) + asked_of(&logs, from, &c1, &h2) == 0 {
        wrong.push("no refetch of C_B+1 was asked of H1 or H2 after the replay walk".into());
    }
    for (p, n) in [(a, "A"), (r, "R")] {
        if asked_of(&logs, from, &c1, &p) > 0 {
            wrong.push(format!("the failed supplier {n} was asked for C_B+1"));
        }
    }
    let walk = CompletedRecovery {
        blocks: br.clone(),
        connection_point: fx.f_hash,
        suppliers: vec![Some(h1); 3],
    };
    if let Err(e) = fx.node.handle_completed_fork_recovery(walk).await {
        wrong.push(format!("honest walk returned Err: {e}"));
    }
    drop(g);
    wrong.extend(landed(&fx, &br).await);
    wrong.extend(refused(&fx, &br[1..]).await);
    probe("walk_replay", &wrong);
    assert!(
        wrong.is_empty(),
        "walk replaying a refused variant: {wrong:?}"
    );
}

/// The spend branch, extended under L_new(C_B) until 3 producers corroborate it.
async fn corroborated_spend_branch(fx: &Fx) -> Vec<Block> {
    let tx = spend(fx).await;
    let mut br = honest(fx, &[], &[tx]);
    let distinct = |br: &[Block]| {
        let mut p: Vec<_> = br.iter().map(|b| b.header.producer).collect();
        p.sort_by(|x, y| x.as_bytes().cmp(y.as_bytes()));
        p.dedup();
        p.len()
    };
    let mut slot = fx.s2;
    while distinct(&br) < 3 && br.len() < 6 {
        slot += 1;
        let prev = br.last().unwrap().hash();
        let h = fx.b() + br.len() as u64;
        br.push(fx.mk(prev, h, slot, fx.key(&at(&fx.l_canon, slot))));
    }
    br
}

// G-2 row 3 (Should) — Decision: a failure means forceReorgTo checks the stored honest body of a branch block but applies a different cached copy, so a cached variant costs a rollback and restore and the operator reorg does not land.
#[tokio::test]
async fn force_reorg_applies_the_checked_body() {
    let mut fx = setup().await;
    let br = corroborated_spend_branch(&fx).await;
    let store = &fx.node.block_store;
    store
        .put_block(&br[1], fx.b() + 1)
        .expect("fixture: store the honest C_B+1 body");
    assert_eq!(
        store.get_height_by_hash(&br[1].hash()).expect("store"),
        None,
        "precondition: the stored C_B+1 is not canonical"
    );
    let mut seeded = br.clone();
    seeded[1] = sig_flip(&br[1]);
    r236::seed_cache(&fx.node, &seeded).await;
    let target = br.last().unwrap().hash();
    let (logs, g) = logged();
    fx.node.sync_manager.write().await.arm_force_reorg(target);
    let outcome = fx.node.try_consume_force_reorg().await;
    drop(g);
    let mut wrong = restores(&logs);
    if outcome != Some(ForceReorgOutcome::Executed) {
        wrong.push(format!("outcome {outcome:?}, want Executed"));
    }
    wrong.extend(landed(&fx, &br).await);
    probe("force_reorg_stored", &wrong);
    assert!(
        wrong.is_empty(),
        "forceReorgTo over a stored body: {wrong:?}"
    );
}

// G-2 row 4 (Must) — Decision: a failure means a tip-extending variant that fails apply leaves its supplier unpenalized and the body is not fetched again by hash from a different peer, so the node waits for sync to fetch a block it was already offered.
#[tokio::test]
async fn extends_tip_variant_then_refetch() {
    let mut fx = setup().await;
    fx.node.floor_fallback_window = true;
    let tx = spend(&fx).await;
    let h = fx.b() + 2;
    let by = fx.key(&at(&fx.l_dead, fx.s1));
    let e = rebody(&fx, fx.mk(fx.d[1].hash(), h, fx.s1, by), h, &[], &[tx]);
    let v = sig_flip(&e);
    let (a, hp) = (PeerId::random(), PeerId::random());
    register(&fx, &e, h, &[a, hp]).await;
    let (logs, g) = logged();
    let _ = deliver(&mut fx, &v, a).await;
    let mut wrong = Vec::new();
    if fx.state().await.best_hash != fx.d[1].hash() {
        wrong.push("the variant moved the tip".into());
    }
    wrong.extend(serve(&mut fx, &logs, &[a, hp], std::slice::from_ref(&e)).await);
    drop(g);
    let st = fx.state().await;
    let stored = fx.node.block_store.get_block(&e.hash()).expect("store");
    if (st.best_hash, st.best_height, st.persisted) != (e.hash(), h, Some(e.hash()))
        || stored.as_ref() != Some(&e)
    {
        wrong.push(format!(
            "the honest block did not land: best={} h={}",
            short(&st.best_hash),
            st.best_height
        ));
    }
    if !penalized(&fx.node, &a) {
        wrong.push("supplier A of the variant holds no InvalidBlock record".into());
    }
    if penalized(&fx.node, &hp) {
        wrong.push("honest H holds an InvalidBlock record".into());
    }
    if asked_of(&logs, 0, &e.hash(), &hp) == 0 {
        wrong.push("no refetch of the block was asked of H".into());
    }
    if asked_of(&logs, 0, &e.hash(), &a) > 0 {
        wrong.push("the failed supplier A was asked again".into());
    }
    probe("extends_tip", &wrong);
    assert!(wrong.is_empty(), "tip-extending variant first: {wrong:?}");
}

/// X7..X10 on A, each by its slot leader.
fn chain_on_a(fx: &r236::Fx) -> Vec<Block> {
    let mut chain: Vec<Block> = Vec::new();
    let mut prev = fx.block_a.hash();
    for (i, slot) in (203u32..207).enumerate() {
        let by = r236::leader(&fx.producers, slot);
        let b = r236::block(7 + i as u64, slot, prev, by, r236::Sig::Valid, &fx.params);
        prev = b.hash();
        chain.push(b);
    }
    chain
}

// G-2 row 5 (Must) — Decision: a failure means a GetBodies body that fails its header commitments leaves its supplier without an invalid-block record.
#[tokio::test]
async fn getbodies_junk_supplier_is_penalized() {
    let mut fx = r236::fixture().await;
    let chain = chain_on_a(&fx);
    let mut junk = chain[0].clone();
    junk.attestation_bitfield.push(1);
    assert_eq!(junk.hash(), chain[0].hash(), "fixture: same header hash");
    let tip = chain.last().unwrap().clone();
    let tip_height = fx.node.chain_state.read().await.best_height + chain.len() as u64;
    for _ in 0..2 {
        let p = PeerId::random();
        let mut sm = fx.node.sync_manager.write().await;
        sm.add_peer(p, tip_height, tip.hash(), tip.header.slot);
    }
    let mut supplier = None;
    for _ in 0..16 {
        let Some((to, req)) = fx.node.sync_manager.write().await.next_request() else {
            break;
        };
        let resp = match req {
            SyncRequest::GetHeaders {
                start_hash,
                max_count,
            } => {
                let from = chain
                    .iter()
                    .position(|b| b.header.prev_hash == start_hash)
                    .unwrap_or(chain.len());
                let headers = chain[from..].iter().take(max_count as usize);
                SyncResponse::Headers(headers.map(|b| b.header.clone()).collect())
            }
            SyncRequest::GetBodies { hashes } => {
                let mut bodies = Vec::new();
                for b in chain.iter().filter(|b| hashes.contains(&b.hash())) {
                    if supplier.is_none() && b.hash() == junk.hash() {
                        supplier = Some(to);
                        bodies.push(junk.clone());
                    } else {
                        bodies.push(b.clone());
                    }
                }
                SyncResponse::Bodies(bodies)
            }
            _ => continue,
        };
        fx.node
            .on_sync_response(to, resp)
            .await
            .expect("on_sync_response returns Ok");
    }
    let supplier = supplier.expect("precondition: a GetBodies reply carried the junk body");
    let mut wrong = Vec::new();
    if !penalized(&fx.node, &supplier) {
        wrong
            .push("the supplier of the refused GetBodies body holds no InvalidBlock record".into());
    }
    let best = fx.node.chain_state.read().await.best_hash;
    if best != tip.hash() {
        wrong.push(format!("did not reach X10: tip {}", short(&best)));
    }
    probe("getbodies_junk", &wrong);
    assert!(wrong.is_empty(), "GetBodies junk body: {wrong:?}");
}

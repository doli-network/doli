// INC-I-281 run 619 M-A body sweep: a peer body variant under an honest header hash, at the
// sites of docs/.workflow/inc-i-281-body-sweep.md rows 4, 19-23, 26 and 31.
//
// covers: body_commitments commitment block fork_walk fork_recovery fork_cache block_handling reorg_refusals reorg_restore body_intake body_auth force_reorg network_events wedge_escape
//
// OUTPUT CONTRACT: node on [.., F, D_B, D_{B+1}] (boundary fx), on F after a fork-remedy rollback
//   (m2 rig) or on A at h=6 (inc_i_236 fx); a peer delivers a body variant of an honest header.
// Functions: Node::handle_completed_fork_recovery, Node::handle_new_block,
//   Node::try_consume_force_reorg, Node::on_sync_response (driven by SyncManager::next_request).
// OUTPUTS:
//   O1 return value (Ok / ForceReorgOutcome)
//   O2 chain_state best_hash / best_height; O3 canonical hash per height; O4 persisted tip
//   O5 fork cache entry under the variant's hash; O6 stored body under that hash
//   O7 LB-9 memo is_fork_branch_rejected(branch tip, local tip)
//   O8 logs: rollback / restore lines, `presence_root mismatch` (the variant reached apply_block),
//      `is a learned anchor`
//   O9 GetBodies re-request of the refused hash
// PATHS: walk | walk apply failure | REAPPLY of a rolled-back block | forceReorgTo | GetBodies
// INPUT PARTITIONS: variant = honest block plus one attestation bit (pre-AH: merkle root holds,
//   presence_root fails); walk variant at index 0 x {no honest copy cached, honest copy cached
//   first}; apply failure = missing spend (new hash) at {index 0 boundary, index 1 tail};
//   forceReorgTo variant at index 1 (an applied prefix before it); honest GetBodies bodies with
//   an attestation body at heights below, at and above the BLS AH (control).
// MATRIX: walk_presence_junk O1-O5 O7 O8 + honest lands | walk_apply_failure_no_mark O1-O5 O7 O8 |
//   reapply_uses_stored_body O2-O6 O8 vs the honest delivery | force_reorg_cached_variant O1-O4
//   O7 O8 + honest lands | getbodies_presence_junk O2 O4 O8 O9 + honest lands |
//   getbodies_honest_across_bls_ah O2-O4 O6 O8 O9.
// Row 34 (backfillFromPeer) is not built here: see the run report.

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;
mod inc_i_281_boundary_fx;
mod inc_i_281_m2_support;

use std::collections::{HashMap, HashSet};

use crypto::{bls_aggregate, bls_sign, BlsKeyPair, Hash, KeyPair};
use doli_core::attestation::bls_attest_msg;
use doli_core::validation::ValidationMode;
use doli_core::Block;
use doli_node::node::{ForceReorgOutcome, Node};
use inc_i_236_m2_reorg_support as r236;
use inc_i_281_boundary_fx::*;
use inc_i_281_m2_support as m2;
use m2::{rollback_lines, with_missing_spend, Logs};
use network::protocols::{SyncRequest, SyncResponse};
use network::sync::{SyncConfig, SyncManager};
use network::PeerId;
use tempfile::TempDir;

/// apply_block's pre-AH commitment error: the variant reached apply_block.
const PRESENCE_LINE: &str = "presence_root mismatch";
const ANCHOR_LINE: &str = "is a learned anchor";
const RESTORE_LINE: &str = "[REORG_RESTORE]";

fn probe(cell: &str, wrong: bool) {
    println!("INC281_PROBE bodyvariant={cell} wrong={}", wrong as u8);
}

/// `b` with one attestation bit flipped: same header hash, merkle root holds, presence_root fails.
fn presence_variant(b: &Block) -> Block {
    let mut v = b.clone();
    match v.attestation_bitfield.first_mut() {
        Some(byte) => *byte ^= 1,
        None => v.attestation_bitfield.push(1),
    }
    assert_eq!(v.hash(), b.hash(), "fixture: same header hash");
    assert!(v.verify_merkle_root(), "fixture: merkle root holds");
    assert_ne!(v, *b, "fixture: a different body");
    v
}

fn logged() -> (Logs, tracing::subscriber::DefaultGuard) {
    let logs = Logs::default();
    let g = tracing::subscriber::set_default(logs.subscriber());
    (logs, g)
}

fn lines(logs: &Logs, needles: &[&str]) -> Vec<String> {
    needles
        .iter()
        .map(|l| (*l, logs.mentioning(l)))
        .filter(|(_, n)| *n > 0)
        .map(|(l, n)| format!("{n} x `{l}`"))
        .collect()
}

async fn setup() -> Fx {
    let fx = fixture_at(post_genesis_offset()).await;
    fx.assert_past_genesis();
    fx
}

async fn rejected(fx: &Fx, branch_tip: Hash) -> bool {
    fx.node
        .sync_manager
        .read()
        .await
        .is_fork_branch_rejected(branch_tip, fx.d[1].hash())
}

/// Rollback / fork-apply / restore lines, and a tip that moved.
async fn moved(fx: &Fx, before: &State, logs: &Logs) -> Vec<String> {
    let mut wrong: Vec<String> = rollback_lines(logs)
        .into_iter()
        .map(|(l, n)| format!("{n} x `{l}`"))
        .collect();
    let st = fx.state().await;
    if st != *before {
        wrong.push(format!("tip moved: {st:?}"));
    }
    wrong
}

/// Tip = br's last block, `br` canonical from B on, persisted.
async fn landed(fx: &Fx, br: &[Block]) -> bool {
    let st = fx.state().await;
    let tip = br.last().unwrap().hash();
    let canonical = br.iter().enumerate().all(|(i, b)| {
        let h = fx.b() + i as u64;
        fx.node.block_store.get_hash_by_height(h).expect("store") == Some(b.hash())
    });
    st.best_hash == tip
        && st.best_height == fx.b() + br.len() as u64 - 1
        && st.persisted == Some(tip)
        && canonical
}

fn tagged(tag: &str, wrong: Vec<String>) -> Vec<String> {
    wrong.into_iter().map(|w| format!("{tag}: {w}")).collect()
}

/// Walks [C_B', C_B+1, C_B+2] (C_B' = variant of C_B), then gossips the honest branch.
async fn walk_junk_wrongs(honest_first: bool) -> Vec<String> {
    let mut fx = setup().await;
    let canon = fx.canonical();
    let variant = presence_variant(&canon[0]);
    if honest_first {
        r236::seed_cache(&fx.node, &canon[..1]).await;
    }
    let mut walked = canon.clone();
    walked[0] = variant.clone();
    let before = fx.state().await;
    let (logs, g) = logged();
    let f = fx.f_hash;
    let result = run_walk(&mut fx, &walked, f).await;
    drop(g);
    let mut wrong = lines(&logs, &[PRESENCE_LINE, RESTORE_LINE]);
    if !honest_first {
        wrong.extend(moved(&fx, &before, &logs).await);
    }
    if let Err(e) = result {
        wrong.push(format!("walk returned Err({e})"));
    }
    if rejected(&fx, canon[2].hash()).await {
        wrong.push("LB-9 memo holds the honest branch tip C_B+2 on D_B+1".to_string());
    }
    let c_b_canonical = fx.state().await.at_b == Some(canon[0].hash());
    match r236::cached(&fx.node, &canon[0].hash()).await {
        Some(b) if b == variant => wrong.push("the variant is in the fork cache".to_string()),
        None if honest_first && !c_b_canonical => {
            wrong.push("the honest cached C_B was evicted".to_string())
        }
        _ => {}
    }
    for b in &canon {
        gossip(&mut fx, b).await;
    }
    if !landed(&fx, &canon).await {
        let st = fx.state().await;
        wrong.push(format!("the honest branch by gossip did not land: {st:?}"));
    }
    let tag = if honest_first { "cached" } else { "uncached" };
    tagged(tag, wrong)
}

// F-3 rows 19-21 (Must) — Decision: a failure means a walked presence-root variant of an honest fork block rolls the node back, marks the honest branch in the LB-9 memo or replaces its cached body, so a zero-stake relay blocks the canonical branch.
#[tokio::test]
async fn walk_presence_junk() {
    let mut wrong = walk_junk_wrongs(false).await;
    wrong.extend(walk_junk_wrongs(true).await);
    probe("walk_presence_junk", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "walk with a presence-root variant of C_B: {wrong:?}"
    );
}

/// `(branch, failing block)`: boundary = [C_B*, Y1, Y2]; tail = [C_B, C_B+1*, Y2].
fn failing_branch(fx: &Fx, tail: bool) -> (Vec<Block>, Block) {
    let canon = fx.canonical();
    let i = usize::from(tail);
    let bad = with_missing_spend(&canon[i], fx.key(&canon[i].header.producer));
    let mut br = canon[..i].to_vec();
    br.push(bad.clone());
    for (h, s) in [(fx.b() + 1, fx.s1), (fx.b() + 2, fx.s2)]
        .into_iter()
        .skip(i)
    {
        let prev = br.last().unwrap().hash();
        br.push(fx.mk(prev, h, s, fx.key(&at(&fx.l_canon, s))));
    }
    (br, bad)
}

async fn apply_failure_wrongs(tail: bool) -> Vec<String> {
    let mut fx = setup().await;
    let (br, bad) = failing_branch(&fx, tail);
    let before = fx.state().await;
    let (logs, g) = logged();
    let f = fx.f_hash;
    let result = run_walk(&mut fx, &br, f).await;
    drop(g);
    let mut wrong = lines(&logs, &[ANCHOR_LINE]);
    if let Err(e) = result {
        wrong.push(format!("walk returned Err({e})"));
    }
    let st = fx.state().await;
    if st != before {
        wrong.push(format!("not back on the prior tip: {st:?}"));
    }
    if fx.applied(&bad) {
        wrong.push("the failing block is stored".to_string());
    }
    if r236::cached(&fx.node, &bad.hash()).await.is_some() {
        wrong.push("the failing body is still in the fork cache".to_string());
    }
    if rejected(&fx, br.last().unwrap().hash()).await {
        wrong.push("LB-9 memo holds the branch tip on D_B+1".to_string());
    }
    tagged(if tail { "tail" } else { "boundary" }, wrong)
}

// F-3 rows 20/22/26 (Must) — Decision: a failure means a fork body that passes merkle and presence but fails apply in a walk leaves its body cached, marks its branch tip header-keyed or learns an anchor, so a later honest body of that branch is skipped.
#[tokio::test]
async fn walk_apply_failure_no_mark() {
    let mut wrong = apply_failure_wrongs(false).await;
    wrong.extend(apply_failure_wrongs(true).await);
    probe("walk_apply_failure_no_mark", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "walk whose fork block fails apply: {wrong:?}"
    );
}

// F-3 row 4 (Must) — Decision: a failure means REAPPLY evaluates a peer variant of a stored, rolled-back block instead of the stored body, so one junk copy keeps the node off a block it already holds.
#[tokio::test]
async fn reapply_uses_stored_body() {
    let mut ctl = m2::rig().await;
    m2::roll_back_to_f(&mut ctl).await;
    let honest = ctl.dead[0].clone();
    m2::gossip(&mut ctl, &honest).await;
    let want_h = m2::obs(&ctl).await.best_height;
    assert!(
        want_h > m2::F,
        "precondition: the honest D1 is re-adopted at once ({:?})",
        m2::obs(&ctl).await
    );

    let mut r = m2::rig().await;
    m2::roll_back_to_f(&mut r).await;
    let d1 = r.dead[0].clone();
    let variant = presence_variant(&d1);
    let (logs, g) = logged();
    m2::gossip(&mut r, &variant).await;
    drop(g);
    let o = m2::obs(&r).await;
    let want = r.dead[(want_h - m2::F - 1) as usize].hash();
    let mut wrong = lines(&logs, &[PRESENCE_LINE]);
    if (o.best_hash, o.best_height, o.persisted, o.sync_height)
        != (want, want_h, Some(want), want_h)
    {
        wrong.push(format!(
            "outcome differs from the honest delivery (tip h={want_h}): {o:?}"
        ));
    }
    let stored = r.node.block_store.get_block(&d1.hash()).expect("store");
    if stored.as_ref() != Some(&d1) {
        wrong.push("the stored body of D1 is not the honest one".to_string());
    }
    if r236::cached(&r.node, &d1.hash()).await.as_ref() == Some(&variant) {
        wrong.push("the variant is in the fork cache".to_string());
    }
    probe("reapply_uses_stored_body", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "variant of the rolled-back D1 by gossip: {wrong:?}"
    );
}

fn distinct_producers(br: &[Block]) -> usize {
    br.iter()
        .map(|b| b.header.producer)
        .collect::<HashSet<_>>()
        .len()
}

/// The canonical branch, extended under L_new(C_B) until 3 of 4 producers corroborate it.
fn corroborated_branch(fx: &Fx) -> Vec<Block> {
    let mut br = fx.canonical();
    let mut slot = fx.s2;
    while distinct_producers(&br) < 3 && br.len() < 6 {
        slot += 1;
        let prev = br.last().unwrap().hash();
        let h = fx.b() + br.len() as u64;
        br.push(fx.mk(prev, h, slot, fx.key(&at(&fx.l_canon, slot))));
    }
    br
}

// F-3 row 31 (Should) — Decision: a failure means forceReorgTo resolves a cached presence-root variant of a branch block and rolls the node back with it, then refuses, anchors or marks the honest branch.
#[tokio::test]
async fn force_reorg_cached_variant() {
    let mut fx = setup().await;
    let br = corroborated_branch(&fx);
    let mut seeded = br.clone();
    seeded[1] = presence_variant(&br[1]);
    r236::seed_cache(&fx.node, &seeded).await;
    let target = br.last().unwrap().hash();
    let before = fx.state().await;
    let (logs, g) = logged();
    fx.node.sync_manager.write().await.arm_force_reorg(target);
    let outcome = fx.node.try_consume_force_reorg().await;
    drop(g);
    let mut wrong = moved(&fx, &before, &logs).await;
    wrong.extend(lines(&logs, &[PRESENCE_LINE, ANCHOR_LINE]));
    if rejected(&fx, target).await {
        wrong.push("LB-9 memo holds the honest branch tip on D_B+1".to_string());
    }
    r236::seed_cache(&fx.node, &br).await;
    fx.node.sync_manager.write().await.arm_force_reorg(target);
    let honest = fx.node.try_consume_force_reorg().await;
    if honest != Some(ForceReorgOutcome::Executed) || !landed(&fx, &br).await {
        let st = fx.state().await;
        wrong.push(format!(
            "honest bodies then: {honest:?}, {st:?} (variant run: {outcome:?})"
        ));
    }
    probe("force_reorg_cached_variant", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "forceReorgTo with a cached variant of C_B+1 (outcome {outcome:?}): {wrong:?}"
    );
}

// Path coverage (developer, run 619) — OUTPUTS: O2-O4, O5 (failing body not cached), O7, O8
//   rollback lines. PATH: a direct reorg meets a body the refusal memory holds by content.
//   INPUT: [Z1*, Z2] on D_B, Z1* scheduled under L_new(D_B) plus a missing spend.
#[tokio::test]
async fn content_refused_body_dropped_before_rollback() {
    let mut fx = setup().await;
    let z1 = fx.mk(
        fx.d[0].hash(),
        fx.b() + 1,
        fx.s1,
        fx.key(&at(&fx.l_dead, fx.s1)),
    );
    let bad = with_missing_spend(&z1, fx.key(&z1.header.producer));
    let z2 = fx.mk(
        bad.hash(),
        fx.b() + 2,
        fx.s2,
        fx.key(&at(&fx.l_dead, fx.s2)),
    );
    let br = vec![bad.clone(), z2];
    let before = fx.state().await;
    let (first_logs, g) = logged();
    run_direct_reorg(&mut fx, &br).await.expect("first reorg");
    drop(g);
    assert!(
        first_logs.mentioning("Reorg apply_block failed") >= 1 && fx.state().await == before,
        "precondition: Z1* failed apply after the rollback and the node was restored"
    );
    let (logs, g) = logged();
    let second = run_direct_reorg(&mut fx, &br).await;
    drop(g);
    let mut wrong = moved(&fx, &before, &logs).await;
    if second.is_err() {
        wrong.push(format!("second reorg returned {second:?}"));
    }
    if logs.mentioning("has a body a reorg refused before") == 0 {
        wrong.push("no content-refusal stop line".to_string());
    }
    if r236::cached(&fx.node, &bad.hash()).await.is_some() {
        wrong.push("the refused body is still in the fork cache".to_string());
    }
    if rejected(&fx, br[1].hash()).await {
        wrong.push("LB-9 memo holds the branch tip on D_B+1".to_string());
    }
    probe("content_refused_body", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "second reorg over a refused body: {wrong:?}"
    );
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

/// Two peers at `chain`'s tip, then up to 16 `next_request` rounds served from `chain`; `junk`
/// replaces its block in the first GetBodies reply that names it. Returns the served kinds and
/// the GetBodies request count per hash.
async fn serve(
    fx: &mut r236::Fx,
    chain: &[Block],
    junk: Option<&Block>,
) -> (Vec<&'static str>, HashMap<Hash, usize>) {
    let tip = chain.last().unwrap();
    let tip_height = fx.node.chain_state.read().await.best_height + chain.len() as u64;
    for _ in 0..2 {
        fx.node.sync_manager.write().await.add_peer(
            PeerId::random(),
            tip_height,
            tip.hash(),
            tip.header.slot,
        );
    }
    let (mut served, mut asked) = (Vec::new(), HashMap::<Hash, usize>::new());
    for _ in 0..16 {
        let Some((to, req)) = fx.node.sync_manager.write().await.next_request() else {
            break;
        };
        let resp = match req {
            SyncRequest::GetHeaders {
                start_hash,
                max_count,
            } => {
                served.push("headers");
                let from = chain
                    .iter()
                    .position(|b| b.header.prev_hash == start_hash)
                    .unwrap_or(chain.len());
                let headers = chain[from..].iter().take(max_count as usize);
                SyncResponse::Headers(headers.map(|b| b.header.clone()).collect())
            }
            SyncRequest::GetBodies { hashes } => {
                served.push("bodies");
                let mut bodies = Vec::new();
                for b in chain.iter().filter(|b| hashes.contains(&b.hash())) {
                    let n = asked.entry(b.hash()).or_default();
                    *n += 1;
                    bodies.push(match junk {
                        Some(j) if *n == 1 && j.hash() == b.hash() => j.clone(),
                        _ => b.clone(),
                    });
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
    (served, asked)
}

/// `Some` when the node is not at `tip` (h=10), persisted.
async fn not_at_tip(fx: &r236::Fx, tip: &Block) -> Option<String> {
    let (best, height) = {
        let cs = fx.node.chain_state.read().await;
        (cs.best_hash, cs.best_height)
    };
    let persisted = fx.node.state_db.get_chain_state().map(|c| c.best_hash);
    ((best, height, persisted) != (tip.hash(), 10, Some(tip.hash())))
        .then(|| format!("did not reach X10: h={height} tip {best:.16}"))
}

// F-3 row 23 (Must) — Decision: a failure means a GetBodies reply carrying a presence-root variant for a requested hash is accepted and applied, and the hash is no longer needed, so header-first sync stalls on one junk reply.
#[tokio::test]
async fn getbodies_presence_junk() {
    let mut fx = r236::fixture().await;
    let chain = chain_on_a(&fx);
    let variant = presence_variant(&chain[0]);
    let tip = chain.last().unwrap().clone();
    let (logs, g) = logged();
    let (served, asked) = serve(&mut fx, &chain, Some(&variant)).await;
    drop(g);
    let variant_asked = asked.get(&variant.hash()).copied().unwrap_or(0);
    assert!(
        variant_asked >= 1,
        "precondition: X7's body was requested and the variant served; served {served:?}"
    );
    let mut wrong = lines(&logs, &[PRESENCE_LINE]);
    if variant_asked < 2 {
        wrong.push(format!(
            "X7 was not asked again after its variant; served {served:?}"
        ));
    }
    wrong.extend(not_at_tip(&fx, &tip).await);
    probe("getbodies_presence_junk", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "GetBodies reply with a presence-root variant of X7: {wrong:?}"
    );
}

/// `r236::fixture()` with BLS AH `ah` in the node and in a sync manager set before any apply,
/// and a BLS key published for every producer.
async fn fixture_with_bls_ah(ah: u64) -> (r236::Fx, Vec<BlsKeyPair>) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    node.inc_i_178_attestation_bls_activation_height = ah;
    let genesis = node.chain_state.read().await.genesis_hash;
    let config = SyncConfig {
        inc_i_178_attestation_bls_activation_height: ah,
        ..SyncConfig::default()
    };
    let mut sm = SyncManager::new(config, genesis);
    sm.set_bootstrap_grace_period_secs(0);
    sm.set_min_peers_for_production(0);
    *node.sync_manager.write().await = sm;
    let params = node.params.clone();
    let mut prev = Hash::ZERO;
    for h in 1..=5u64 {
        let b = r236::block(h, h as u32, prev, &producers[0], r236::Sig::Valid, &params);
        prev = b.hash();
        node.apply_block(b, ValidationMode::Light)
            .await
            .expect("fixture: base block must apply");
    }
    let block_a = r236::block(6, 100, prev, &producers[0], r236::Sig::Valid, &params);
    node.apply_block(block_a.clone(), ValidationMode::Light)
        .await
        .expect("fixture: A must apply");
    let mut bls = Vec::new();
    for kp in &producers {
        let k = BlsKeyPair::generate();
        let mut ps = node.producer_set.write().await;
        ps.get_by_pubkey_mut(kp.public_key())
            .expect("fixture: producer in the set")
            .bls_pubkey = k.public_key().as_bytes().to_vec();
        bls.push(k);
    }
    let fx = r236::Fx {
        node,
        producers,
        params,
        base_tip: prev,
        block_a,
        _tmp: temp,
    };
    (fx, bls)
}

/// X7..X10 on A, every bit set and the aggregate of all `bls` over the parent, re-signed:
/// root BLAKE3(bitfield) below `ah`, presence_commitment(bitfield, aggregate) at and above it.
fn chain_across_ah(fx: &r236::Fx, bls: &[BlsKeyPair], ah: u64) -> Vec<Block> {
    let bf = [(1u8 << bls.len()) - 1];
    let mut chain: Vec<Block> = Vec::new();
    let mut prev = fx.block_a.hash();
    for (i, slot) in (203u32..207).enumerate() {
        let h = 7 + i as u64;
        let by = r236::leader(&fx.producers, slot);
        let mut b = r236::block(h, slot, prev, by, r236::Sig::Valid, &fx.params);
        let msg = bls_attest_msg(&prev);
        let sigs: Vec<_> = bls
            .iter()
            .map(|k| bls_sign(&msg, k.secret_key()).expect("BLS sign"))
            .collect();
        let agg = bls_aggregate(&sigs)
            .expect("BLS aggregate")
            .as_bytes()
            .to_vec();
        b.header.presence_root = if h >= ah {
            doli_core::presence_commitment(&bf, &agg)
        } else {
            crypto::hash::hash(&bf)
        };
        b.attestation_bitfield = bf.to_vec();
        b.aggregate_bls_signature = agg;
        block_sig::sign_header(&mut b.header, by);
        prev = b.hash();
        chain.push(b);
    }
    chain
}

// F-3 row 23 (Must, control) — Decision: a failure means the GetBodies commitment gate judges an honest body on one side of the BLS AH by the other side's presence root, so header-first sync refuses honest bodies and stalls at the AH.
#[tokio::test]
async fn getbodies_honest_across_bls_ah() {
    const AH: u64 = 9;
    let (mut fx, bls) = fixture_with_bls_ah(AH).await;
    let chain = chain_across_ah(&fx, &bls, AH);
    let (bf, agg) = (
        &chain[1].attestation_bitfield,
        &chain[1].aggregate_bls_signature,
    );
    assert_ne!(
        crypto::hash::hash(bf),
        doli_core::presence_commitment(bf, agg),
        "fixture: the pre-AH and post-AH roots of one body differ"
    );
    let tip = chain.last().unwrap().clone();
    let (logs, g) = logged();
    let (served, asked) = serve(&mut fx, &chain, None).await;
    drop(g);
    assert!(
        chain.iter().all(|b| asked.contains_key(&b.hash())),
        "precondition: every body was requested by GetBodies; served {served:?}, asked {asked:?}"
    );
    let mut wrong = lines(&logs, &[PRESENCE_LINE]);
    for (i, b) in chain.iter().enumerate() {
        let h = 7 + i as u64;
        if asked[&b.hash()] > 1 {
            wrong.push(format!("X{h} was refused and asked again"));
        }
        if fx
            .node
            .block_store
            .get_block(&b.hash())
            .expect("store")
            .as_ref()
            != Some(b)
        {
            wrong.push(format!("X{h} is not stored with its honest body"));
        }
        if fx.node.block_store.get_hash_by_height(h).expect("store") != Some(b.hash()) {
            wrong.push(format!("X{h} is not canonical"));
        }
    }
    wrong.extend(not_at_tip(&fx, &tip).await);
    assert!(
        wrong.is_empty(),
        "honest bodies across the BLS AH (h={AH}); served {served:?}: {wrong:?}"
    );
}

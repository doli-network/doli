// INC-I-281 run 619 M-A — AUDIT-P1-618-001 / AUDIT-P2-618-001: a same-header body variant of an
// honest block (header hash and merkle root unchanged) must never replace the honest body, block
// the reorg onto the honest branch, or mark the honest header / branch refused.
//
// covers: body_commitments commit verify reorg_refusals reorg_restore fork_cache block_handling fork_walk wedge_escape fork_recovery validation_checks periodic commitment block body_intake body_auth force_reorg network_events backfill lib mod init types context
//
// OUTPUT CONTRACT: node on [.., F, D_B, D_{B+1}] (post-genesis); honest branch D = [C_B, C_{B+1},
//   C_{B+2}] on F; a variant V shares a header with C_{B+1}, C_{B+2} or the node's own D_{B+1}.
// Functions: Node::handle_new_block (wedge / orphan-retained intake), Node::handle_completed_fork_recovery
//   (walk door), both -> execute_reorg -> run_reorg (Normal, Restore).
// OUTPUTS:
//   O1 return value (Ok)
//   O2 chain_state best_hash / best_height; O3 canonical hash at B, B+1; O4 persisted tip
//   O5 stored body of C_{B+1} after landing (honest bytes)
//   O6 fork_block_cache entry under the shared hash (honest body / variant gone)
//   O7 refused set: check_producer_eligibility error "refused by a reorg before" for honest and V
//   O8 learned anchors: `is a learned anchor` log line
//   O9 LB-9 memo: is_fork_branch_rejected(C_{B+2}, local tip) for every local tip the cycle reached
//   O10 rollback / fork-apply / restore log lines; undo records at B, B+1
// PATHS: V after the honest copy, then the next honest tip (wedge) | V of the tip first, then the
//   honest tip (wedge) | V inside a walked branch (walk door, pre-rollback) | V of an own block in
//   the cache during a refused reorg's restore | V the header cannot see: first (apply-time
//   refusal) or after the honest copy (kept out) | honest branch only (control)
// INPUT PARTITIONS: bitfield bit flip (pre-AH root BLAKE3(bitfield)) | aggregate byte flip (post-AH
//   root presence_commitment) | transaction input signature byte flip (no header commitment) |
//   own-block bitfield added under a ZERO root.
// MATRIX: bitflip_honest_first O1-O4 O6-O10 | aggregate_honest_first O1-O4 O6-O10 |
//   bitflip_variant_first O1-O4 O7-O9 | variant_in_plan O1-O4 O6-O10 + landed |
//   restore_source O1 O2 O4 | sig_variant_honest_first O2-O10 | sig_variant (variant first)
//   O2 O6 O8 O9 + landed | honest_control O2-O5 O10.

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;
mod inc_i_281_boundary_fx;
mod inc_i_281_m2_support;

use crypto::{Hash, KeyPair};
use doli_core::transaction::{Input, Output, OutputType, Transaction, TxType};
use doli_core::Block;
use inc_i_236_m2_reorg_support::{cached, seed_cache};
use inc_i_281_boundary_fx::*;
use inc_i_281_m2_support::{rollback_lines, with_missing_spend, Logs};
use storage::{Outpoint, UtxoEntry};

const AGGREGATE: [u8; 96] = [0xA5; 96];
const FUNDING: u64 = 100_000;

fn probe(cell: &str, wrong: bool) {
    println!("INC281_PROBE bodyvariant={cell} wrong={}", wrong as u8);
}

async fn setup() -> Fx {
    let fx = fixture_at(post_genesis_offset()).await;
    fx.assert_past_genesis();
    fx
}

fn logged() -> (Logs, tracing::subscriber::DefaultGuard) {
    let logs = Logs::default();
    let g = tracing::subscriber::set_default(logs.subscriber());
    (logs, g)
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

/// `b` with attestation body (`bf`, `agg`) and `txs` appended, both roots recomputed, re-signed.
fn rebody(fx: &Fx, mut b: Block, height: u64, bf: &[u8], agg: &[u8], txs: &[Transaction]) -> Block {
    b.transactions.extend_from_slice(txs);
    b.attestation_bitfield = bf.to_vec();
    b.aggregate_bls_signature = agg.to_vec();
    b.header.merkle_root = doli_core::block::compute_merkle_root(&b.transactions);
    let ah = fx.node.inc_i_178_attestation_bls_activation_height;
    b.header.presence_root = presence_root(ah, height, bf, agg);
    let by = fx.key(&b.header.producer);
    block_sig::sign_header(&mut b.header, by);
    b
}

/// [C_B, C_{B+1}, C_{B+2}] by the canonical leaders; C_{B+1} and C_{B+2} carry (`bf`, `agg`).
fn honest(fx: &Fx, bf: &[u8], agg: &[u8], txs: &[Transaction]) -> Vec<Block> {
    let b = fx.b();
    let by1 = fx.key(&at(&fx.l_canon, fx.s1));
    let c1 = fx.mk(fx.c_b.hash(), b + 1, fx.s1, by1);
    let c1 = rebody(fx, c1, b + 1, bf, agg, txs);
    let by2 = fx.key(&at(&fx.l_canon, fx.s2));
    let c2 = fx.mk(c1.hash(), b + 2, fx.s2, by2);
    let c2 = rebody(fx, c2, b + 2, bf, agg, &[]);
    vec![fx.c_b.clone(), c1, c2]
}

fn pkh(k: &KeyPair) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, k.public_key().as_bytes())
}

/// A spendable output of `owner`, in the in-memory set (the reorg persists it) and in state_db.
async fn fund(fx: &Fx, owner: &KeyPair) -> Outpoint {
    let op = Outpoint::new(crypto::hash::hash(b"inc-i-281-ma-funding"), 0);
    let entry = UtxoEntry {
        output: Output {
            amount: FUNDING,
            pubkey_hash: pkh(owner),
            output_type: OutputType::Normal,
            lock_until: 0,
            extra_data: vec![],
        },
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
    op
}

fn signed_transfer(spend: Outpoint, owner: &KeyPair) -> Transaction {
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input {
            prev_tx_hash: spend.tx_hash,
            output_index: spend.index,
            signature: crypto::Signature::default(),
            sighash_type: doli_core::transaction::SighashType::All,
            committed_output_count: 0,
            public_key: Some(*owner.public_key()),
        }],
        outputs: vec![Output {
            amount: FUNDING - 1_000,
            pubkey_hash: pkh(owner),
            output_type: OutputType::Normal,
            lock_until: 0,
            extra_data: vec![],
        }],
        extra_data: vec![],
    };
    let sighash = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&sighash, owner.private_key());
    tx
}

#[derive(Debug, Clone, Copy)]
enum Body {
    Bitfield,
    Aggregate,
    Spend,
}

/// The honest branch for `body`; Aggregate moves the BLS AH to B+1 (C_B stays pre-AH).
async fn branch(fx: &mut Fx, body: Body) -> Vec<Block> {
    match body {
        Body::Bitfield => honest(fx, &[0b01], &[], &[]),
        Body::Aggregate => {
            fx.node.inc_i_178_attestation_bls_activation_height = fx.b() + 1;
            honest(fx, &[0b01], &AGGREGATE, &[])
        }
        Body::Spend => {
            let owner = KeyPair::generate();
            let op = fund(fx, &owner).await;
            honest(fx, &[], &[], &[signed_transfer(op, &owner)])
        }
    }
}

/// `b`'s header with `edit` applied to its body: same hash, valid merkle root, other bytes.
fn variant(b: &Block, edit: impl FnOnce(&mut Block)) -> Block {
    let mut v = b.clone();
    edit(&mut v);
    assert_eq!(v.hash(), b.hash(), "fixture: same header hash");
    assert!(v.verify_merkle_root(), "fixture: merkle root still valid");
    assert_ne!(v, *b, "fixture: body differs");
    v
}

fn flip(b: &Block, body: Body) -> Block {
    variant(b, |v| match body {
        Body::Bitfield => v.attestation_bitfield[0] ^= 0b10,
        Body::Aggregate => v.aggregate_bls_signature[0] ^= 1,
        Body::Spend => {
            let input = &mut v.transactions[1].inputs[0];
            let mut s = *input.signature.as_bytes();
            s[0] ^= 1;
            input.signature = crypto::Signature::from_bytes(s);
        }
    })
}

/// `Some` when `br` is not the canonical chain at B..B+2 with C_{B+1}'s honest body stored.
async fn not_landed(fx: &Fx, br: &[Block]) -> Option<String> {
    let st = fx.state().await;
    let tip = br[2].hash();
    let stored = fx.node.block_store.get_block(&br[1].hash()).expect("store");
    let ok = st.best_hash == tip
        && st.best_height == fx.b() + 2
        && st.at_b == Some(br[0].hash())
        && st.at_b1 == Some(br[1].hash())
        && st.persisted == Some(tip)
        && stored.as_ref() == Some(&br[1]);
    (!ok).then(|| format!("the honest branch did not land: {st:?}"))
}

/// Blocks the door refuses as "refused by a reorg before".
async fn refusals(fx: &Fx, blocks: &[&Block], names: &[&str]) -> Vec<String> {
    let mut wrong = Vec::new();
    for (b, name) in blocks.iter().zip(names) {
        if let Err(e) = fx.node.check_producer_eligibility(b).await {
            if e.to_string().contains("refused by a reorg before") {
                wrong.push(format!("{name} is in the refused set"));
            }
        }
    }
    wrong
}

/// Learned anchor lines and LB-9 marks of `tip` on any local tip the cycle could reach.
async fn marks(fx: &Fx, tip: Hash, logs: &Logs) -> Vec<String> {
    let mut wrong = Vec::new();
    let n = logs.mentioning("is a learned anchor");
    if n > 0 {
        wrong.push(format!("{n} x learned anchor"));
    }
    let best = fx.state().await.best_hash;
    let sm = fx.node.sync_manager.read().await;
    for local in [fx.d[0].hash(), fx.d[1].hash(), fx.f_hash, best] {
        if sm.is_fork_branch_rejected(tip, local) {
            wrong.push(format!("LB-9 memo holds C_B+2 on local tip {local:.16}"));
            break;
        }
    }
    wrong
}

fn restores(logs: &Logs) -> Vec<String> {
    let n = logs.mentioning("[REORG_RESTORE]") + logs.mentioning("Reorg apply_block failed");
    (n > 0)
        .then(|| format!("{n} x apply failure / restore lines"))
        .into_iter()
        .collect()
}

fn undo(fx: &Fx) -> Vec<String> {
    (fx.b()..=fx.b() + 1)
        .map(|h| format!("{:?}", fx.node.state_db.get_undo(h)))
        .collect()
}

/// Honest C_B, C_{B+1} by gossip, then `v` (same header as C_{B+1}), then the honest tip C_{B+2}.
async fn honest_first(fx: &mut Fx, br: &[Block], v: &Block) -> Vec<String> {
    let (logs, g) = logged();
    gossip(fx, &br[0]).await;
    gossip(fx, &br[1]).await;
    gossip(fx, v).await;
    let mut wrong = Vec::new();
    if cached(&fx.node, &br[1].hash()).await.as_ref() != Some(&br[1]) {
        wrong.push("the variant replaced the honest cached body of C_B+1".to_string());
    }
    gossip(fx, &br[2]).await;
    drop(g);
    wrong.extend(not_landed(fx, br).await);
    wrong.extend(restores(&logs));
    wrong.extend(refusals(fx, &[&br[1], v], &["honest C_B+1", "variant"]).await);
    wrong.extend(marks(fx, br[2].hash(), &logs).await);
    wrong
}

// F-1 (Must) — Decision: a failure means a bit-flipped bitfield copy of a cached honest block replaces its body, so the next honest tip rolls the node back, fails apply and refuses or marks the honest branch.
#[tokio::test]
async fn bitflip_honest_first() {
    let mut fx = setup().await;
    let br = branch(&mut fx, Body::Bitfield).await;
    let v = flip(&br[1], Body::Bitfield);
    let wrong = honest_first(&mut fx, &br, &v).await;
    probe("bitflip_honest_first", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "bit-flipped copy of C_B+1 after the honest one: {wrong:?}"
    );
}

// F-1 (Must) — Decision: a failure means a post-AH copy with one aggregate byte changed replaces the honest body and pins the honest branch.
#[tokio::test]
async fn aggregate_honest_first() {
    let mut fx = setup().await;
    let br = branch(&mut fx, Body::Aggregate).await;
    let v = flip(&br[1], Body::Aggregate);
    let wrong = honest_first(&mut fx, &br, &v).await;
    probe("aggregate_honest_first", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "aggregate-flipped copy of C_B+1 after the honest one (post-AH): {wrong:?}"
    );
}

// F-1 (Must) — Decision: a failure means a variant of the honest tip that arrives first triggers a failed reorg whose memo then skips the honest tip, so the node stays on its dead branch.
#[tokio::test]
async fn bitflip_variant_first() {
    let mut fx = setup().await;
    let br = branch(&mut fx, Body::Bitfield).await;
    let v = flip(&br[2], Body::Bitfield);
    let (logs, g) = logged();
    gossip(&mut fx, &br[0]).await;
    gossip(&mut fx, &br[1]).await;
    gossip(&mut fx, &v).await;
    gossip(&mut fx, &br[2]).await;
    drop(g);
    let mut wrong: Vec<String> = not_landed(&fx, &br).await.into_iter().collect();
    wrong.extend(refusals(&fx, &[&br[2], &v], &["honest C_B+2", "variant"]).await);
    wrong.extend(marks(&fx, br[2].hash(), &logs).await);
    probe("bitflip_variant_first", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "bit-flipped copy of the tip C_B+2 before the honest one: {wrong:?}"
    );
}

// F-1 (Must) — Decision: a failure means a planned reorg whose branch holds a variant body rolls back before any commitment check, or leaves the variant cached, refused or marked, so the honest copy cannot land.
#[tokio::test]
async fn variant_in_plan() {
    let mut fx = setup().await;
    let br = branch(&mut fx, Body::Bitfield).await;
    let v = flip(&br[1], Body::Bitfield);
    let walked = vec![br[0].clone(), v.clone(), br[2].clone()];
    let (before, undo_before) = (fx.state().await, undo(&fx));
    let (logs, g) = logged();
    let f = fx.f_hash;
    let result = run_walk(&mut fx, &walked, f).await;
    drop(g);
    let mut wrong: Vec<String> = rollback_lines(&logs)
        .into_iter()
        .map(|(l, n)| format!("{n} x `{l}`"))
        .collect();
    if result.is_err() {
        wrong.push(format!("walk returned {result:?}"));
    }
    let st = fx.state().await;
    if st != before {
        wrong.push(format!("tip moved: {st:?}"));
    }
    if undo(&fx) != undo_before {
        wrong.push("undo records at B, B+1 rewritten".to_string());
    }
    if cached(&fx.node, &v.hash()).await.as_ref() == Some(&v) {
        wrong.push("the variant is still cached".to_string());
    }
    wrong.extend(refusals(&fx, &[&br[1], &v], &["honest C_B+1", "variant"]).await);
    wrong.extend(marks(&fx, br[2].hash(), &logs).await);
    for b in &br {
        gossip(&mut fx, b).await;
    }
    wrong.extend(not_landed(&fx, &br).await);
    probe("variant_in_plan", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "walked [C_B, V_B+1, C_B+2] then the honest copy: {wrong:?}"
    );
}

/// [C_B*, Y_{B+1}, Y_{B+2}]: C_B* spends a missing UTXO, so apply fails after the rollback.
fn failing_branch(fx: &Fx) -> Vec<Block> {
    let bad = with_missing_spend(&fx.c_b, fx.key(&fx.c_b.header.producer));
    let y1 = fx.mk(
        bad.hash(),
        fx.b() + 1,
        fx.s1,
        fx.key(&at(&fx.l_canon, fx.s1)),
    );
    let y2 = fx.mk(
        y1.hash(),
        fx.b() + 2,
        fx.s2,
        fx.key(&at(&fx.l_canon, fx.s2)),
    );
    vec![bad, y1, y2]
}

// F-3 / AUDIT-P2-618-001 (Must) — Decision: a failure means a cached same-hash variant of the node's own block is what the restore re-applies, so a refused reorg leaves the node below its prior tip.
#[tokio::test]
async fn restore_source() {
    let mut fx = setup().await;
    let own = variant(&fx.d[1], |b| b.attestation_bitfield = vec![0b01]);
    seed_cache(&fx.node, std::slice::from_ref(&own)).await;
    let br = failing_branch(&fx);
    let before = fx.state().await;
    let (logs, g) = logged();
    let f = fx.f_hash;
    let result = run_walk(&mut fx, &br, f).await;
    drop(g);
    let refused = logs.mentioning("Reorg apply_block failed");
    let st = fx.state().await;
    let mut wrong = Vec::new();
    if result.is_err() {
        wrong.push(format!("walk returned {result:?}"));
    }
    if (st.best_hash, st.best_height) != (before.best_hash, before.best_height) {
        wrong.push(format!("not back on D_B+1: {st:?}"));
    }
    if st.persisted != before.persisted {
        wrong.push(format!("persisted tip {:?}", st.persisted));
    }
    probe("restore_source", !wrong.is_empty());
    assert!(
        refused >= 1,
        "precondition: the reorg rolled back and failed apply"
    );
    assert!(
        wrong.is_empty(),
        "restore with a variant of D_B+1 cached: {wrong:?}"
    );
}

// F-1 (Must) — Decision: a failure means a signature variant of a cached honest C_B+1 (no header commitment sees it) replaces the honest body, so the next honest tip fails apply instead of landing.
#[tokio::test]
async fn sig_variant_honest_first() {
    let mut fx = setup().await;
    let br = branch(&mut fx, Body::Spend).await;
    let v = flip(&br[1], Body::Spend);
    let wrong = honest_first(&mut fx, &br, &v).await;
    probe("sig_variant_honest_first", !wrong.is_empty());
    assert!(
        wrong.is_empty(),
        "signature-flipped copy of C_B+1 after the honest one: {wrong:?}"
    );
}

// F-3 (Must) — Decision: a failure means a variant in a part no header commitment covers (an input signature), cached first, makes its apply failure anchor or mark the honest branch, or stays cached, so the honest copy never lands.
#[tokio::test]
async fn sig_variant() {
    let mut fx = setup().await;
    let br = branch(&mut fx, Body::Spend).await;
    let v = flip(&br[1], Body::Spend);
    let (logs, g) = logged();
    gossip(&mut fx, &br[0]).await;
    gossip(&mut fx, &v).await;
    gossip(&mut fx, &br[1]).await;
    let before = fx.state().await;
    gossip(&mut fx, &br[2]).await;
    let apply_failed = logs.mentioning("Reorg apply_block failed");
    let mut wrong = Vec::new();
    let st = fx.state().await;
    if st != before {
        wrong.push(format!("not back on the prior tip: {st:?}"));
    }
    if cached(&fx.node, &v.hash()).await.as_ref() == Some(&v) {
        wrong.push("the variant is still cached".to_string());
    }
    wrong.extend(marks(&fx, br[2].hash(), &logs).await);
    gossip(&mut fx, &br[1]).await;
    gossip(&mut fx, &br[2]).await;
    drop(g);
    wrong.extend(not_landed(&fx, &br).await);
    probe("sig_variant", !wrong.is_empty());
    assert!(
        apply_failed >= 1,
        "precondition: apply refused the signature variant in the reorg"
    );
    assert!(
        wrong.is_empty(),
        "signature-flipped copy of C_B+1: {wrong:?}"
    );
}

// F-1 (Must, control) — Decision: a failure means an honest branch with a bitfield, a post-AH aggregate or a signed spend does not land by gossip, so the variant cells measure a broken fixture.
#[tokio::test]
async fn honest_control() {
    let mut wrong = Vec::new();
    for body in [Body::Bitfield, Body::Aggregate, Body::Spend] {
        let mut fx = setup().await;
        let br = branch(&mut fx, body).await;
        let (logs, g) = logged();
        for b in &br {
            gossip(&mut fx, b).await;
        }
        drop(g);
        let tag = |w: String| format!("{body:?}: {w}");
        wrong.extend(not_landed(&fx, &br).await.map(tag));
        wrong.extend(restores(&logs).into_iter().map(tag));
    }
    probe("honest_control", !wrong.is_empty());
    assert!(wrong.is_empty(), "honest branches: {wrong:?}");
}

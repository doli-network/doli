// INC-I-281 run 620 M-B — SEC-FIXVERIFY-001: a body copy is untrusted until apply succeeds. A
// variant body of an honest fork block that reaches the node FIRST is evicted, its supplier is
// penalized, and the body is fetched again by hash from a different peer (at most 3 per height).
//
// covers: body_retry body_commitments fork_cache block_handling fork_walk fork_recovery reorg_refusals reorg_restore force_reorg wedge_escape network_events periodic event_loop mod init validation_checks door_epoch scoring types body_intake body_auth response dispatch peers manager sync_engine lib backfill context restore commitment block apply_block tests_inc_i236_m2_intake
//
// OUTPUT CONTRACT: node on [.., F, D_B, D_{B+1}] (post-genesis); honest branch [C_B, C_{B+1},
//   C_{B+2}] on F; V shares C_{B+1}'s header hash and differs in body bytes. Peers A (attacker),
//   H1, H2 (honest) are registered in the sync manager at height B+2, hash C_{B+2}, slot s2 (the
//   honest_control row lands with them registered).
// Functions: Node::handle_new_block (gossip intake), Node::handle_completed_fork_recovery (walk).
// OUTPUTS:
//   O1 chain_state best_hash / best_height; O2 canonical hash at B, B+1, B+2; O3 persisted tip
//   O4 stored body of C_{B+1} (honest bytes)
//   O5 refused set: check_producer_eligibility "refused by a reorg before" on honest C_{B+1}, C_{B+2}
//   O6 learned anchors: `is a learned anchor` lines; O7 LB-9 memo is_fork_branch_rejected(C_{B+2}, local)
//   O8 bls_ingress_scorer: InvalidBlock infraction on every failed-copy supplier, none on honest peers
//   O9 `[BODY_REFETCH]` lines: asked peer (PeerId Display) + requested hash (first 16 hex chars);
//      never a peer whose copy of that hash failed; <= 3 per height
//   O10 `Reorg apply_block failed` / `[REORG_RESTORE]` lines
// PATHS: gossip = C_B(H1), V(A), C_{B+1}(H1), C_{B+2}(H1) | walk = C_B(H1), V(A), walk [C_B,
//   C_{B+1}, C_{B+2}] at F | bound = C_B(S1), V(A), C_{B+2}(S1), every refetch answered with a fresh
//   variant | honest_control = honest branch from H1 only. Refetch requests are served by the test
//   (honest body from an honest peer); the test never re-sends C_{B+1} or the tip itself.
// INPUT PARTITIONS: input signature byte (no header commitment, passes intake) | pre-AH bitfield
//   bit | post-AH aggregate byte | appended junk tx (merkle breaks) | empty body (no txs, no
//   attestation).
// MATRIX: gossip_* O1-O9 | walk_* O1-O10 (walk_sig: O10 instead of A's O8) | bound O1 O3 O5-O9 |
//   honest_control O1-O4 O8-O10.

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;
mod inc_i_281_boundary_fx;
mod inc_i_281_m2_support;

use crypto::{Hash, KeyPair};
use doli_core::transaction::{Input, Output, OutputType, Transaction, TxType};
use doli_core::Block;
use inc_i_281_boundary_fx::*;
use inc_i_281_m2_support::{missing_spend, Logs};
use network::{Infraction, PeerId};
use storage::{Outpoint, UtxoEntry};

const AGGREGATE: [u8; 96] = [0xA5; 96];
const FUNDING: u64 = 100_000;
const REFETCH: &str = "[BODY_REFETCH]";
const MAX_REFETCH: usize = 3;
const ROUNDS: usize = 10;

fn probe(row: &str, wrong: bool) {
    println!("INC281_PROBE bodyretry={row} wrong={}", wrong as u8);
}

fn short(h: &Hash) -> String {
    h.to_hex()[..8].to_string()
}

fn prefix(h: &Hash) -> String {
    h.to_hex()[..16].to_string()
}

fn short_opt(h: &Option<Hash>) -> String {
    h.as_ref().map(short).unwrap_or_else(|| "none".into())
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

async fn fund(fx: &Fx, owner: &KeyPair) -> Outpoint {
    let op = Outpoint::new(crypto::hash::hash(b"inc-i-281-mb-funding"), 0);
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

fn same_header(b: &Block, v: Block) -> Block {
    assert_eq!(v.hash(), b.hash(), "fixture: same header hash");
    assert_ne!(v, *b, "fixture: body differs");
    v
}

fn sig_flip(b: &Block, byte: usize) -> Block {
    let mut v = b.clone();
    let input = &mut v.transactions[1].inputs[0];
    let mut s = *input.signature.as_bytes();
    s[byte] ^= 1;
    input.signature = crypto::Signature::from_bytes(s);
    assert!(
        v.verify_merkle_root(),
        "fixture: sig variant keeps the merkle root"
    );
    same_header(b, v)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Var {
    Sig,
    Bitfield,
    Aggregate,
    Junk,
    Empty,
}

const VARS: [Var; 5] = [
    Var::Sig,
    Var::Bitfield,
    Var::Aggregate,
    Var::Junk,
    Var::Empty,
];

impl Var {
    fn name(self) -> &'static str {
        match self {
            Var::Sig => "sig",
            Var::Bitfield => "bitfield",
            Var::Aggregate => "aggregate",
            Var::Junk => "junk",
            Var::Empty => "empty",
        }
    }

    fn base(self) -> Body {
        match self {
            Var::Sig => Body::Spend,
            Var::Bitfield | Var::Junk => Body::Bitfield,
            Var::Aggregate | Var::Empty => Body::Aggregate,
        }
    }

    fn of(self, c1: &Block) -> Block {
        let mut v = c1.clone();
        match self {
            Var::Sig => return sig_flip(c1, 0),
            Var::Bitfield => v.attestation_bitfield[0] ^= 0b10,
            Var::Aggregate => v.aggregate_bls_signature[0] ^= 1,
            Var::Junk => {
                v.transactions.push(missing_spend());
                assert!(
                    !v.verify_merkle_root(),
                    "fixture: junk breaks the merkle root"
                );
            }
            Var::Empty => {
                v.transactions.clear();
                v.attestation_bitfield.clear();
                v.aggregate_bls_signature.clear();
            }
        }
        same_header(c1, v)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Path {
    Gossip,
    Walk,
}

struct Rig {
    fx: Fx,
    br: Vec<Block>,
    peers: Vec<(PeerId, &'static str)>,
    failed: Vec<(Hash, PeerId)>,
    seen: usize,
    flips: usize,
    wrong: Vec<String>,
}

async fn rig(body: Body, names: &[&'static str]) -> Rig {
    let mut fx = setup().await;
    let br = branch(&mut fx, body).await;
    let peers: Vec<(PeerId, &'static str)> = names.iter().map(|n| (PeerId::random(), *n)).collect();
    {
        let tip = &br[2];
        let mut sm = fx.node.sync_manager.write().await;
        for (p, _) in &peers {
            sm.add_peer(*p, fx.b() + 2, tip.hash(), tip.header.slot);
        }
    }
    Rig {
        fx,
        br,
        peers,
        failed: Vec::new(),
        seen: 0,
        flips: 0,
        wrong: Vec::new(),
    }
}

fn clip(line: &str) -> String {
    line.chars().take(200).collect()
}

impl Rig {
    fn peer(&self, name: &str) -> PeerId {
        self.peers
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(p, _)| *p)
            .expect("fixture: peer")
    }

    fn name(&self, peer: &PeerId) -> &'static str {
        self.peers
            .iter()
            .find(|(p, _)| p == peer)
            .map(|(_, n)| *n)
            .unwrap_or("unregistered")
    }

    async fn deliver(&mut self, b: &Block, from: PeerId) {
        let is_honest = self.br.contains(b);
        let res = self.fx.node.handle_new_block(b.clone(), from).await;
        if let (true, Err(e)) = (is_honest, res) {
            self.wrong.push(format!(
                "handle_new_block of honest {} from {} returned Err: {e}",
                short(&b.hash()),
                self.name(&from)
            ));
        }
    }

    async fn send(&mut self, i: usize, from: &str) {
        let b = self.br[i].clone();
        let p = self.peer(from);
        self.deliver(&b, p).await;
    }

    async fn plant(&mut self, v: &Block, from: &str) {
        let p = self.peer(from);
        self.failed.push((v.hash(), p));
        self.deliver(v, p).await;
    }

    /// Answers every new `[BODY_REFETCH]` line: honest bytes, or in `bound` a fresh
    /// signature variant of C_{B+1} whose supplier is then a failed supplier.
    async fn serve(&mut self, logs: &Logs, bound: bool) {
        let c1 = self.br[1].clone();
        for _ in 0..ROUNDS {
            let lines = logs.lines();
            let new: Vec<String> = lines[self.seen..]
                .iter()
                .filter(|l| l.contains(REFETCH))
                .cloned()
                .collect();
            self.seen = lines.len();
            if new.is_empty() {
                return;
            }
            for line in new {
                let Some(peer) = self
                    .peers
                    .iter()
                    .map(|(p, _)| *p)
                    .find(|p| line.contains(&p.to_string()))
                else {
                    self.wrong
                        .push(format!("refetch names no registered peer: {}", clip(&line)));
                    continue;
                };
                let asked = self
                    .br
                    .iter()
                    .filter_map(|b| line.find(&prefix(&b.hash())).map(|at| (at, b.clone())))
                    .min_by_key(|(at, _)| *at)
                    .map(|(_, b)| b);
                let Some(asked) = asked else {
                    self.wrong
                        .push(format!("refetch names no branch hash: {}", clip(&line)));
                    continue;
                };
                let hash = asked.hash();
                if self.failed.contains(&(hash, peer)) {
                    self.wrong.push(format!(
                        "asked the penalized supplier {} for {}",
                        self.name(&peer),
                        short(&hash)
                    ));
                    continue;
                }
                let reply = if bound && hash == c1.hash() {
                    self.flips += 1;
                    self.failed.push((hash, peer));
                    sig_flip(&c1, self.flips)
                } else {
                    asked
                };
                self.deliver(&reply, peer).await;
            }
        }
        let left = logs.lines()[self.seen..]
            .iter()
            .filter(|l| l.contains(REFETCH))
            .count();
        if left > 0 {
            self.wrong.push(format!(
                "{left} refetch requests left after {ROUNDS} rounds"
            ));
        }
    }

    fn refetches(&self, logs: &Logs, hash: &Hash, by: &[&str]) -> usize {
        let p = prefix(hash);
        let ids: Vec<String> = by.iter().map(|n| self.peer(n).to_string()).collect();
        logs.lines()
            .iter()
            .filter(|l| l.contains(REFETCH) && l.contains(&p))
            .filter(|l| by.is_empty() || ids.iter().any(|id| l.contains(id)))
            .count()
    }

    fn penalized(&self, peer: &PeerId) -> bool {
        self.fx
            .node
            .bls_ingress_scorer
            .get_score(peer)
            .is_some_and(|s| {
                s.infractions
                    .iter()
                    .any(|(_, i)| matches!(i, Infraction::InvalidBlock { .. }))
            })
    }

    fn infractions(&self, peer: &PeerId) -> usize {
        self.fx
            .node
            .bls_ingress_scorer
            .get_score(peer)
            .map_or(0, |s| s.infractions.len())
    }

    fn honest_clean(&self, names: &[&str]) -> Vec<String> {
        names
            .iter()
            .filter_map(|n| {
                let k = self.infractions(&self.peer(n));
                (k > 0).then(|| format!("honest {n} holds {k} infractions"))
            })
            .collect()
    }

    fn bound_check(&self, logs: &Logs) -> Vec<String> {
        let n = self.refetches(logs, &self.br[1].hash(), &[]);
        (n > MAX_REFETCH)
            .then(|| format!("{n} refetches of C_B+1 (bound {MAX_REFETCH})"))
            .into_iter()
            .collect()
    }

    async fn not_landed(&self) -> Option<String> {
        let st = self.fx.state().await;
        let br = &self.br;
        let tip = br[2].hash();
        let stored = self
            .fx
            .node
            .block_store
            .get_block(&br[1].hash())
            .expect("store");
        let honest_body = stored.as_ref() == Some(&br[1]);
        let ok = st.best_hash == tip
            && st.best_height == self.fx.b() + 2
            && st.at_b == Some(br[0].hash())
            && st.at_b1 == Some(br[1].hash())
            && st.at_b2 == Some(tip)
            && st.persisted == Some(tip)
            && honest_body;
        (!ok).then(|| {
            format!(
                "the honest branch did not land: best={} h={} at_b={} at_b1={} at_b2={} persisted={} c1_body_honest={honest_body}",
                short(&st.best_hash),
                st.best_height,
                short_opt(&st.at_b),
                short_opt(&st.at_b1),
                short_opt(&st.at_b2),
                short_opt(&st.persisted)
            )
        })
    }

    async fn refusals(&self) -> Vec<String> {
        let mut wrong = Vec::new();
        for (b, name) in self.br[1..].iter().zip(["honest C_B+1", "honest C_B+2"]) {
            if let Err(e) = self.fx.node.check_producer_eligibility(b).await {
                if e.to_string().contains("refused by a reorg before") {
                    wrong.push(format!("{name} is in the refused set"));
                }
            }
        }
        wrong
    }

    async fn marks(&self, logs: &Logs) -> Vec<String> {
        let mut wrong = Vec::new();
        let n = logs.mentioning("is a learned anchor");
        if n > 0 {
            wrong.push(format!("{n} x learned anchor"));
        }
        let tip = self.br[2].hash();
        let fx = &self.fx;
        let best = fx.state().await.best_hash;
        let sm = fx.node.sync_manager.read().await;
        for local in [fx.d[0].hash(), fx.d[1].hash(), fx.f_hash, best] {
            if sm.is_fork_branch_rejected(tip, local) {
                wrong.push(format!(
                    "LB-9 memo holds C_B+2 on local tip {}",
                    short(&local)
                ));
                break;
            }
        }
        wrong
    }
}

fn restores(logs: &Logs) -> Vec<String> {
    let n = logs.mentioning("[REORG_RESTORE]") + logs.mentioning("Reorg apply_block failed");
    (n > 0)
        .then(|| format!("{n} x apply failure / restore lines"))
        .into_iter()
        .collect()
}

async fn scripted(path: Path, var: Var) -> Vec<String> {
    let mut r = rig(var.base(), &["A", "H1", "H2"]).await;
    let v = var.of(&r.br[1]);
    let (logs, g) = logged();
    r.send(0, "H1").await;
    r.plant(&v, "A").await;
    match path {
        Path::Gossip => {
            r.send(1, "H1").await;
            r.send(2, "H1").await;
        }
        Path::Walk => {
            let (br, f) = (r.br.clone(), r.fx.f_hash);
            if let Err(e) = run_walk(&mut r.fx, &br, f).await {
                r.wrong.push(format!("walk returned Err: {e}"));
            }
        }
    }
    r.serve(&logs, false).await;
    drop(g);
    let mut wrong = std::mem::take(&mut r.wrong);
    wrong.extend(r.not_landed().await);
    wrong.extend(r.refusals().await);
    wrong.extend(r.marks(&logs).await);
    let a = r.peer("A");
    if (path, var) != (Path::Walk, Var::Sig) && !r.penalized(&a) {
        wrong.push("supplier A of the failed copy holds no InvalidBlock infraction".into());
    }
    wrong.extend(r.honest_clean(&["H1", "H2"]));
    wrong.extend(r.bound_check(&logs));
    if (path, var) == (Path::Gossip, Var::Sig)
        && r.refetches(&logs, &r.br[1].hash(), &["H1", "H2"]) == 0
    {
        wrong.push("no refetch of C_B+1 was asked of H1 or H2".into());
    }
    if path == Path::Walk {
        wrong.extend(restores(&logs));
    }
    wrong
}

async fn bound() -> Vec<String> {
    let mut r = rig(Body::Spend, &["A", "S1", "S2", "S3", "S4"]).await;
    let before = r.fx.state().await;
    let v = sig_flip(&r.br[1], 0);
    let (logs, g) = logged();
    r.send(0, "S1").await;
    r.plant(&v, "A").await;
    r.send(2, "S1").await;
    r.serve(&logs, true).await;
    drop(g);
    let mut wrong = std::mem::take(&mut r.wrong);
    if r.refetches(&logs, &r.br[1].hash(), &[]) == 0 {
        wrong.push("no refetch of C_B+1 was asked".into());
    }
    wrong.extend(r.bound_check(&logs));
    let mut suppliers: Vec<PeerId> = r.failed.iter().map(|(_, p)| *p).collect();
    suppliers.dedup();
    for p in suppliers {
        if !r.penalized(&p) {
            wrong.push(format!(
                "supplier {} of a failed copy holds no InvalidBlock infraction",
                r.name(&p)
            ));
        }
    }
    wrong.extend(r.refusals().await);
    wrong.extend(r.marks(&logs).await);
    let st = r.fx.state().await;
    if st.persisted != Some(st.best_hash)
        || (st.best_hash, st.best_height) != (before.best_hash, before.best_height)
    {
        wrong.push(format!(
            "not on the prior tip D_B+1 {}: best={} h={} persisted={}",
            short(&before.best_hash),
            short(&st.best_hash),
            st.best_height,
            short_opt(&st.persisted)
        ));
    }
    wrong
}

async fn honest_control() -> Vec<String> {
    let mut r = rig(Body::Spend, &["A", "H1", "H2"]).await;
    let (logs, g) = logged();
    for i in 0..3 {
        r.send(i, "H1").await;
    }
    r.serve(&logs, false).await;
    drop(g);
    let mut wrong = std::mem::take(&mut r.wrong);
    wrong.extend(r.not_landed().await);
    wrong.extend(restores(&logs));
    let n = logs.mentioning(REFETCH);
    if n > 0 {
        wrong.push(format!("{n} refetch lines on an honest branch"));
    }
    wrong.extend(r.honest_clean(&["A", "H1", "H2"]));
    wrong
}

// G-1 (Must) — Decision: a failure means a variant body that reaches the node first (input signature, attestation byte, junk or empty) still wins over the honest copy, leaves its supplier unpenalized, is not fetched again by hash from a different peer within 3 tries, or condemns the honest header, so a zero-stake peer keeps the node off the honest branch.
#[tokio::test]
async fn variant_first_body_retry() {
    let mut report: Vec<(String, Vec<String>)> = Vec::new();
    for (path, tag) in [(Path::Gossip, "gossip"), (Path::Walk, "walk")] {
        for var in VARS {
            let row = format!("{tag}_{}", var.name());
            let wrong = scripted(path, var).await;
            probe(&row, !wrong.is_empty());
            report.push((row, wrong));
        }
    }
    let wrong = bound().await;
    probe("bound", !wrong.is_empty());
    report.push(("bound".into(), wrong));
    let wrong = honest_control().await;
    probe("honest_control", !wrong.is_empty());
    report.push(("honest_control".into(), wrong));
    let all = report
        .iter()
        .map(|(row, w)| format!("{row}: {w:?}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        report.iter().all(|(_, w)| w.is_empty()),
        "variant-first body retry rows:\n{all}"
    );
}

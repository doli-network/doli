//! INC-I-237 M8 / REQ-GRV-SEC-007 — the two RPC zero-flow admission paths
//! (`sendTransaction`, `submitMaintainerChange`) refuse what apply would refuse,
//! put nothing in the mempool and broadcast nothing; genuine txs still relay.
//!
//! OUTPUT CONTRACT: RpcContext::send_transaction(params) / submit_maintainer_change(params)
//!   -> Result<Value, RpcError>
//!   O1: none   O2: ctx.mempool len   O3: Ok(hash | accepted) / Err
//!   O4: none   O5: none   O6: broadcast_tx permit calls (sink)
//! PATHS: P1 forged zero-flow -> Err, mempool unchanged, 0 broadcasts;
//!   P2 genuine zero-flow -> Ok, admitted, exactly 1 broadcast (INV-RPC-002)
//! INPUT PARTITIONS: type {DelegateBond, RevokeDelegation, AddMaintainer,
//!   RemoveMaintainer}; delegation signature {genuine, absent, malformed, wrong
//!   key}; delegation parties registered and active in the context's producer
//!   set (REQ-GRV-038); maintainer signatures (3 entries, so the count pre-check passes)
//!   {3 of 4 members, 2 members + 1 outsider, 3 outsiders, members over other
//!   bytes, target as signer}; params {testnet, tip past every maintainer AH}.
// covers: zero_flow_auth pool lib gossip_txs validation_checks mod transaction governance context verdict relay_decision init startup gossip-relay-verdict-status gossip-relay-verdict-architecture

use std::sync::{Arc, Mutex};

use super::*;
use crypto::{KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::maintainer::{signing_message_at, MAINTAINER_AUTH_VALID_BEFORE_UNSET};
use doli_core::network::Network;
use doli_core::transaction::{DelegateBondData, RevokeDelegationData};
use doli_core::{MaintainerSet, Transaction};
use mempool::{Mempool, MempoolPolicy};
use serde_json::{json, Value};
use storage::{BlockStore, ChainState, MaintainerState, ProducerSet, UtxoSet};
use tempfile::TempDir;
use tokio::sync::RwLock;

const TIP: u64 = 30_000;

struct H {
    ctx: RpcContext,
    producers: Arc<RwLock<ProducerSet>>,
    sent: Arc<Mutex<Vec<Transaction>>>,
    m: Vec<KeyPair>,
    params: ConsensusParams,
    _dir: TempDir,
}

fn harness() -> H {
    let dir = TempDir::new().expect("tempdir");
    let params = ConsensusParams::for_network(Network::Testnet);
    let mut cs = ChainState::new(params.genesis_hash);
    cs.best_height = TIP;
    cs.best_slot = TIP as u32;
    let mempool = Mempool::new(MempoolPolicy::testnet(), params.clone(), Network::Testnet);
    let sent: Arc<Mutex<Vec<Transaction>>> = Arc::default();
    let sink = sent.clone();
    let producers: Arc<RwLock<ProducerSet>> = Arc::default();
    let mut ctx = RpcContext::new_for_network(
        Arc::new(RwLock::new(cs)),
        Arc::new(BlockStore::open(dir.path()).expect("blockstore")),
        Arc::new(RwLock::new(UtxoSet::new())),
        Arc::new(RwLock::new(mempool)),
        params.clone(),
        Network::Testnet,
    )
    .with_producer_set(producers.clone())
    .with_broadcast(move || {
        let sink = sink.clone();
        Some(
            Box::new(move |tx: Transaction| sink.lock().unwrap().push(tx))
                as crate::TxBroadcastPermit,
        )
    });
    let m: Vec<KeyPair> = (0..4).map(|_| KeyPair::generate()).collect();
    let state = MaintainerState {
        set: MaintainerSet::with_members(m.iter().map(|k| *k.public_key()).collect(), 0),
        ..Default::default()
    };
    ctx.maintainer_state = Some(Arc::new(RwLock::new(state)));
    H {
        ctx,
        producers,
        sent,
        m,
        params,
        _dir: dir,
    }
}

impl H {
    async fn pool_len(&self) -> usize {
        self.ctx.mempool.read().await.len()
    }

    fn sent(&self) -> usize {
        self.sent.lock().unwrap().len()
    }

    fn message(&self, is_add: bool, target: &PublicKey) -> Vec<u8> {
        signing_message_at(
            self.params.genesis_hash.as_bytes(),
            is_add,
            target,
            MAINTAINER_AUTH_VALID_BEFORE_UNSET,
            TIP + 1,
            Network::Testnet
                .params()
                .inc_i_176_auth_binding_activation_height,
        )
    }

    fn sigs(
        &self,
        is_add: bool,
        target: &PublicKey,
        signers: &[&KeyPair],
        msg: Option<&[u8]>,
    ) -> Vec<Value> {
        let own = self.message(is_add, target);
        let msg = msg.unwrap_or(&own);
        signers
            .iter()
            .map(|k| json!({"pubkey": k.public_key().to_hex(), "signature": crypto::signature::sign(msg, k.private_key()).to_hex()}))
            .collect()
    }

    /// Signature sets that fail the current 4-member set's multisig (threshold 3).
    fn forged_sig_sets(&self, is_add: bool, target: &PublicKey) -> Vec<(&'static str, Vec<Value>)> {
        let out: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
        let m = &self.m;
        let mut rows = vec![
            (
                "2 members + 1 outsider",
                self.sigs(is_add, target, &[&m[0], &m[1], &out[0]], None),
            ),
            (
                "3 outsiders",
                self.sigs(is_add, target, &[&out[0], &out[1], &out[2]], None),
            ),
            (
                "3 members over other bytes",
                self.sigs(is_add, target, &[&m[0], &m[1], &m[2]], Some(b"other")),
            ),
        ];
        if !is_add {
            rows.push((
                "target as signer",
                self.sigs(is_add, target, &[&m[1], &m[2], &m[3]], None),
            ));
        }
        rows
    }

    fn target(&self, is_add: bool) -> PublicKey {
        if is_add {
            *KeyPair::generate().public_key()
        } else {
            *self.m[3].public_key()
        }
    }
}

/// Delegation tx by fresh parties registered and active in `h`'s producer set (delegated for a revoke).
async fn delegation(h: &H, revoke: bool, sig: &str) -> Transaction {
    let k = KeyPair::generate();
    let other = *KeyPair::generate().public_key();
    {
        let unit = Network::Testnet.bond_unit();
        let mut ps = h.producers.write().await;
        ps.register_genesis_producer(*k.public_key(), 2, unit)
            .expect("register delegator");
        ps.register_genesis_producer(other, 1, unit)
            .expect("register delegate");
        if revoke {
            ps.delegate_bonds(k.public_key(), &other, 1)
                .expect("fixture delegation");
        }
    }
    let signer = if sig == "wrong" {
        KeyPair::generate()
    } else {
        k.clone()
    };
    let forge = |msg: &crypto::Hash| match sig {
        "malformed" => crypto::Signature::from_bytes([0xFF; 64]),
        _ => crypto::signature::sign_hash(msg, signer.private_key()),
    };
    if revoke {
        let mut d = RevokeDelegationData::new(*k.public_key(), other);
        d.signature = forge(&d.signing_message());
        let mut tx = Transaction::new_revoke_delegation(d.clone());
        if sig == "absent" {
            tx.extra_data = d.to_bytes_legacy();
        }
        tx
    } else {
        let mut d = DelegateBondData::new(*k.public_key(), other, 1);
        d.signature = forge(&d.signing_message());
        let mut tx = Transaction::new_delegate_bond(d.clone());
        if sig == "absent" {
            tx.extra_data = d.to_bytes_legacy();
        }
        tx
    }
}

fn maintainer_tx(is_add: bool, target: PublicKey, sigs: &[Value]) -> Transaction {
    let s = sigs
        .iter()
        .map(|v| doli_core::maintainer::MaintainerSignature {
            pubkey: PublicKey::from_hex(v["pubkey"].as_str().unwrap()).unwrap(),
            signature: crypto::Signature::from_hex(v["signature"].as_str().unwrap()).unwrap(),
        })
        .collect();
    if is_add {
        Transaction::new_add_maintainer(target, s)
    } else {
        Transaction::new_remove_maintainer(target, s, None)
    }
}

async fn send(h: &H, tx: &Transaction) -> Result<Value, crate::error::RpcError> {
    h.ctx
        .send_transaction(json!({"tx": hex::encode(tx.serialize())}))
        .await
}

// REQ-GRV-SEC-007 (sendTransaction) — Decision: a failure means the public RPC still admits and broadcasts a zero-flow tx apply refuses (forged delegation or below-threshold maintainer change).
#[tokio::test]
async fn req_grv_sec_007_send_transaction_refuses_forged_zero_flow_txs() {
    let h = harness();
    let mut rows: Vec<(String, Transaction)> = Vec::new();
    for revoke in [false, true] {
        for sig in ["absent", "malformed", "wrong"] {
            rows.push((
                format!("revoke={revoke} {sig}"),
                delegation(&h, revoke, sig).await,
            ));
        }
    }
    for is_add in [true, false] {
        let target = h.target(is_add);
        for (row, sigs) in h.forged_sig_sets(is_add, &target) {
            rows.push((
                format!("is_add={is_add} {row}"),
                maintainer_tx(is_add, target, &sigs),
            ));
        }
    }
    let mut bad = Vec::new();
    for (row, tx) in rows {
        let (pool, sent) = (h.pool_len().await, h.sent());
        let res = send(&h, &tx).await;
        if res.is_ok() || h.pool_len().await != pool || h.sent() != sent {
            bad.push(format!(
                "{row}: ok={} pool_grew={} broadcast={}",
                res.is_ok(),
                h.pool_len().await != pool,
                h.sent() != sent
            ));
        }
    }
    assert!(bad.is_empty(), "REQ-GRV-SEC-007 sendTransaction: {bad:#?}");
}

// INV-RPC-002 / REQ-GRV-029 (sendTransaction) — Decision: a failure means M8 made the RPC refuse, or stop broadcasting, a genuine zero-flow tx of one of the 4 types.
#[tokio::test]
async fn inv_rpc_002_send_transaction_admits_and_broadcasts_genuine_zero_flow_txs() {
    let h = harness();
    let m = &h.m;
    let add_t = h.target(true);
    let rm_t = h.target(false);
    let rows = vec![
        ("DelegateBond", delegation(&h, false, "genuine").await),
        ("RevokeDelegation", delegation(&h, true, "genuine").await),
        (
            "AddMaintainer",
            maintainer_tx(
                true,
                add_t,
                &h.sigs(true, &add_t, &[&m[0], &m[1], &m[2]], None),
            ),
        ),
        (
            "RemoveMaintainer",
            maintainer_tx(
                false,
                rm_t,
                &h.sigs(false, &rm_t, &[&m[0], &m[1], &m[2]], None),
            ),
        ),
    ];
    for (row, tx) in rows {
        let sent = h.sent();
        let res = send(&h, &tx).await;
        assert!(res.is_ok(), "{row}: genuine refused: {res:?}");
        assert!(
            h.ctx.mempool.read().await.contains(&tx.hash()),
            "{row} admitted"
        );
        assert_eq!(h.sent(), sent + 1, "{row}: broadcast exactly once");
    }
}

// REQ-GRV-SEC-007 (submitMaintainerChange) — Decision: a failure means the maintainer RPC admits and broadcasts a change whose 3 signatures do not meet the CURRENT set's threshold.
#[tokio::test]
async fn req_grv_sec_007_submit_maintainer_change_refuses_below_threshold() {
    let h = harness();
    let mut bad = Vec::new();
    for is_add in [true, false] {
        let target = h.target(is_add);
        for (row, sigs) in h.forged_sig_sets(is_add, &target) {
            let action = if is_add { "add" } else { "remove" };
            let (pool, sent) = (h.pool_len().await, h.sent());
            let res = h
                .ctx
                .submit_maintainer_change(
                    json!({"action": action, "target_pubkey": target.to_hex(), "signatures": sigs}),
                )
                .await;
            if res.is_ok() || h.pool_len().await != pool || h.sent() != sent {
                bad.push(format!(
                    "{action} {row}: ok={} pool_grew={} broadcast={}",
                    res.is_ok(),
                    h.pool_len().await != pool,
                    h.sent() != sent
                ));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "REQ-GRV-SEC-007 submitMaintainerChange: {bad:#?}"
    );
}

// INV-RPC-002 (submitMaintainerChange) — Decision: a failure means M8 refused or stopped broadcasting a threshold-meeting change signed by the current set.
#[tokio::test]
async fn inv_rpc_002_submit_maintainer_change_genuine_still_broadcasts() {
    let h = harness();
    let m = &h.m;
    for is_add in [true, false] {
        let target = h.target(is_add);
        let action = if is_add { "add" } else { "remove" };
        let sigs = h.sigs(is_add, &target, &[&m[0], &m[1], &m[2]], None);
        let sent = h.sent();
        let res = h
            .ctx
            .submit_maintainer_change(
                json!({"action": action, "target_pubkey": target.to_hex(), "signatures": sigs}),
            )
            .await;
        assert!(res.is_ok(), "{action}: genuine refused: {res:?}");
        assert_eq!(h.sent(), sent + 1, "{action}: broadcast exactly once");
    }
}

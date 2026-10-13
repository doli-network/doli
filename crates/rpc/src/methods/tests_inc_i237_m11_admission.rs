//! INC-I-237 M11 / AUDIT-P1-509 — the two RPC zero-flow admission paths
//! (`sendTransaction`, `submitMaintainerChange`) refuse keyless variants of a
//! resident maintainer change with an explicit error, admit and broadcast
//! nothing for them, and refuse a failed state-effect precondition with its own code.
//!
//! OUTPUT CONTRACT: RpcContext::send_transaction(params) / submit_maintainer_change(params)
//!   -> Result<Value, RpcError>
//!   O1: none   O2: ctx.mempool len   O3: Ok(hash | accepted) / Err(code, message, data)
//!   O4: none   O5: none   O6: broadcast_tx permit calls (sink)
//! PATHS: P1 genuine T -> Ok, admitted, 1 broadcast; P2 variant of resident T ->
//!   Err, mempool unchanged, 0 broadcasts (reason / order rows: TX_ALREADY_EXISTS);
//!   P3 precondition fails -> Err carrying M11_PRECONDITION_CODE, not MPTX010
//! INPUT PARTITIONS: rewrite {reason, trailing byte, reordered entries, duplicated
//!   entry}; action {add, remove}; precondition row {Add existing member, Remove
//!   non-member}; params {testnet, 4-member set, tip past every maintainer AH}.
// covers: zero_flow_auth pool lib gossip_txs validation_checks mod transaction governance context verdict relay_decision init startup gossip-relay-verdict-status gossip-relay-verdict-architecture

use std::sync::{Arc, Mutex};

use super::*;
use crypto::{KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::maintainer::{
    signing_message_at, MaintainerChangeData, MaintainerSignature,
    MAINTAINER_AUTH_VALID_BEFORE_UNSET,
};
use doli_core::network::Network;
use doli_core::{MaintainerSet, Transaction};
use mempool::{Mempool, MempoolPolicy};
use serde_json::{json, Value};
use storage::{BlockStore, ChainState, MaintainerState, UtxoSet};
use tempfile::TempDir;
use tokio::sync::RwLock;

/// The developer aligns this with the code chosen in docs/error-codes.md (next free after MPTX010).
const M11_PRECONDITION_CODE: &str = "MPTX011";
const TIP: u64 = 30_000;

struct H {
    ctx: RpcContext,
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
    let mut ctx = RpcContext::new_for_network(
        Arc::new(RwLock::new(cs)),
        Arc::new(BlockStore::open(dir.path()).expect("blockstore")),
        Arc::new(RwLock::new(UtxoSet::new())),
        Arc::new(RwLock::new(mempool)),
        params.clone(),
        Network::Testnet,
    )
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

    fn sigs(
        &self,
        is_add: bool,
        target: &PublicKey,
        signers: &[usize],
    ) -> Vec<MaintainerSignature> {
        let msg = signing_message_at(
            self.params.genesis_hash.as_bytes(),
            is_add,
            target,
            MAINTAINER_AUTH_VALID_BEFORE_UNSET,
            TIP + 1,
            Network::Testnet
                .params()
                .inc_i_176_auth_binding_activation_height,
        );
        signers
            .iter()
            .map(|i| {
                let k = &self.m[*i];
                MaintainerSignature::new(
                    *k.public_key(),
                    crypto::signature::sign(&msg, k.private_key()),
                )
            })
            .collect()
    }

    fn change(&self, is_add: bool, target: PublicKey, signers: &[usize]) -> Transaction {
        let s = self.sigs(is_add, &target, signers);
        if is_add {
            Transaction::new_add_maintainer(target, s)
        } else {
            Transaction::new_remove_maintainer(target, s, None)
        }
    }

    async fn send(&self, tx: &Transaction) -> Result<Value, crate::error::RpcError> {
        self.ctx
            .send_transaction(json!({"tx": hex::encode(tx.serialize())}))
            .await
    }

    async fn submit(
        &self,
        action: &str,
        target: &PublicKey,
        sigs: &[MaintainerSignature],
        reason: Option<&str>,
    ) -> Result<Value, crate::error::RpcError> {
        let entries: Vec<Value> = sigs
            .iter()
            .map(|s| json!({"pubkey": s.pubkey.to_hex(), "signature": s.signature.to_hex()}))
            .collect();
        let mut p =
            json!({"action": action, "target_pubkey": target.to_hex(), "signatures": entries});
        if let Some(r) = reason {
            p["reason"] = json!(r);
        }
        self.ctx.submit_maintainer_change(p).await
    }
}

fn rewrite(tx: &Transaction, f: impl FnOnce(&mut MaintainerChangeData)) -> Transaction {
    let mut d = MaintainerChangeData::from_bytes(&tx.extra_data).expect("change data");
    f(&mut d);
    let mut out = tx.clone();
    out.extra_data = d.to_bytes();
    out
}

fn error_code(e: &crate::error::RpcError) -> String {
    let data = e
        .data
        .as_ref()
        .and_then(|d| d["error_code"].as_str())
        .unwrap_or("");
    format!("{data} {}", e.message)
}

// REQ-GRV-033 / REQ-GRV-035 (sendTransaction) — Decision: a failure means the public RPC still admits and broadcasts a keyless rewrite of a resident maintainer change as a new fee-0 tx (AUDIT-P1-509).
#[tokio::test]
async fn m11_send_transaction_refuses_keyless_variants_of_a_resident_change() {
    let h = harness();
    let target = *KeyPair::generate().public_key();
    let t = h.change(true, target, &[0, 1, 2]);
    assert!(h.send(&t).await.is_ok(), "non-vacuity: genuine T admitted");
    let rows: Vec<(&str, Transaction, bool)> = vec![
        (
            "different reason",
            rewrite(&t, |d| d.reason = Some("rewritten".into())),
            true,
        ),
        (
            "one trailing byte",
            {
                let mut v = t.clone();
                v.extra_data.push(0x00);
                v
            },
            false,
        ),
        (
            "reordered signature entries",
            rewrite(&t, |d| d.signatures.reverse()),
            true,
        ),
        (
            "duplicated signature entry",
            rewrite(&t, |d| {
                let first = d.signatures[0].clone();
                d.signatures.push(first);
            }),
            false,
        ),
    ];
    let mut bad = Vec::new();
    for (row, v, newness_only) in rows {
        let res = h.send(&v).await;
        let (len, sent) = (h.pool_len().await, h.sent());
        let exists = res
            .as_ref()
            .err()
            .is_some_and(|e| error_code(e).contains("TX_ALREADY_EXISTS"));
        if res.is_ok() || len != 1 || sent != 1 || (newness_only && !exists) {
            bad.push(format!(
                "{row}: ok={} pool_len={len} broadcasts={sent} err={:?}",
                res.is_ok(),
                res.as_ref().err().map(error_code)
            ));
        }
    }
    assert!(bad.is_empty(), "AUDIT-P1-509 sendTransaction: {bad:#?}");
}

// REQ-GRV-033 / REQ-GRV-035 (submitMaintainerChange) — Decision: a failure means a second submission of the same (action, target) with another reason or entry order is admitted and broadcast, or is dropped without an explicit error.
#[tokio::test]
async fn m11_submit_maintainer_change_second_call_for_the_same_change_errors() {
    let h = harness();
    let add_t = *KeyPair::generate().public_key();
    let rm_t = *h.m[3].public_key();
    let mut bad = Vec::new();
    for (action, is_add, target) in [("add", true, add_t), ("remove", false, rm_t)] {
        let sigs = h.sigs(is_add, &target, &[0, 1, 2]);
        let first = h.submit(action, &target, &sigs, Some("first")).await;
        assert!(first.is_ok(), "non-vacuity: {action} first call: {first:?}");
        let mut rev = sigs.clone();
        rev.reverse();
        let mut dup = sigs.clone();
        dup.push(sigs[0].clone());
        let rows: Vec<(&str, Vec<MaintainerSignature>, &str)> = vec![
            ("different reason", sigs.clone(), "second"),
            ("reordered signatures", rev, "first"),
            ("duplicated signature entry", dup, "first"),
        ];
        for (row, s, reason) in rows {
            let (len, sent) = (h.pool_len().await, h.sent());
            let res = h.submit(action, &target, &s, Some(reason)).await;
            let (len2, sent2) = (h.pool_len().await, h.sent());
            if res.is_ok() || len2 != len || sent2 != sent {
                bad.push(format!(
                    "{action} {row}: ok={} pool_grew={} broadcast={}",
                    res.is_ok(),
                    len2 != len,
                    sent2 != sent
                ));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "AUDIT-P1-509 submitMaintainerChange: {bad:#?}"
    );
}

// REQ-GRV-034 (sendTransaction) — Decision: a failure means a change apply would turn into a no-op is admitted and broadcast, or is refused under the authorization code MPTX010 so operators cannot tell a bad signature from a stale change.
#[tokio::test]
async fn m11_send_transaction_precondition_refusal_carries_its_own_code() {
    let h = harness();
    let rows = vec![
        (
            "Add existing member",
            h.change(true, *h.m[3].public_key(), &[0, 1, 2]),
        ),
        (
            "Remove non-member",
            h.change(false, *KeyPair::generate().public_key(), &[0, 1, 2]),
        ),
    ];
    let mut bad = Vec::new();
    for (row, tx) in rows {
        let res = h.send(&tx).await;
        let code = res.as_ref().err().map(error_code).unwrap_or_default();
        if res.is_ok()
            || !code.contains(M11_PRECONDITION_CODE)
            || code.contains("MPTX010")
            || h.sent() != 0
        {
            bad.push(format!(
                "{row}: ok={} err={code:?} broadcasts={}",
                res.is_ok(),
                h.sent()
            ));
        }
    }
    assert!(bad.is_empty(), "REQ-GRV-034 precondition code: {bad:#?}");
}

// REQ-GRV-035 (GUARD, sendTransaction) — Decision: a failure means the fix gated `reason` and the RPC refuses a genuine change apply accepts.
#[tokio::test]
async fn m11_send_transaction_admits_a_genuine_change_with_a_reason() {
    let h = harness();
    let t = h.change(false, *h.m[3].public_key(), &[0, 1, 2]);
    let t = rewrite(&t, |d| d.reason = Some("key rotation".into()));
    let res = h.send(&t).await;
    assert!(
        res.is_ok(),
        "genuine RemoveMaintainer + reason refused: {res:?}"
    );
    assert_eq!(h.sent(), 1, "broadcast exactly once");
}

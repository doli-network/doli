//! INC-I-237 M12.2 — fixtures: the node's mined-check counters, a DelegateBond
//! mined on the canonical chain that only the mined check refuses, a
//! maintainer-change world with its keys, the `submitMaintainerChange` request,
//! and the two-site cheap-refusal row check.
//!
//! OUTPUT CONTRACT: N/A — fixture file (helpers only, asserts nothing but setup).
//! INPUT PARTITIONS: N/A — fixture file.
// covers: zero_flow_mined zero_flow_auth zero_flow_policy zero_flow_standing zero_flow_newness pool lib mod gossip_txs validation_checks network_events transaction governance error-codes rpc_reference gossip-relay-verdict-status protocol security_model

use crypto::KeyPair;
use doli_core::{Transaction, TxType};
use doli_node::node::Node;
use network::Verdict;
use rpc::types::JsonRpcRequest;
use rpc::{RpcContext, RpcError};
use serde_json::{json, Value};
use tempfile::TempDir;

use super::inc_i_237_m11_common as m11;
use super::inc_i_237_m12_common::{self as m12, delegate_tx, World, BONDS};
use super::inc_i_237_m8_common as m8;

pub async fn reads(node: &Node) -> u64 {
    node.mempool.read().await.mined_check().body_reads()
}

pub async fn gets(node: &Node) -> u64 {
    node.mempool.read().await.mined_check().index_gets()
}

pub async fn cached(node: &Node, tx: &Transaction) -> bool {
    node.mempool
        .read()
        .await
        .mined_check()
        .is_cached(&tx.hash())
}

/// Store `txs` in a fresh canonical block at `height`; each is then in the chain tx index there.
pub fn mine(node: &Node, height: u64, txs: &[Transaction]) {
    m11::store_block(node, height, txs.to_vec(), true);
    for tx in txs {
        assert_eq!(
            node.block_store.get_tx_block_height(&tx.hash()).unwrap(),
            Some(height),
            "fixture: tx indexed at {height}"
        );
    }
}

/// DelegateBond D -> P the boundary applies (D's delegation revoked), mined at `height`.
pub async fn mined_delegation(w: &World, height: u64) -> Transaction {
    let d = w.delegator(Some(&w.p)).await;
    w.node
        .producer_set
        .write()
        .await
        .revoke_delegation(d.public_key())
        .expect("fixture revoke");
    let t = delegate_tx(&d, w.p.public_key(), BONDS);
    assert!(
        w.boundary_effect(&t).await,
        "fixture: every check but the mined one passes T"
    );
    mine(&w.node, height, std::slice::from_ref(&t));
    t
}

/// Auth node with its 4 maintainer keys and RPC context.
pub struct Gov {
    pub node: Node,
    pub m: Vec<KeyPair>,
    pub rpc_ctx: RpcContext,
    _t: TempDir,
}

pub async fn gov() -> Gov {
    let (node, _kp, m, t) = m8::auth_node().await;
    let rpc_ctx = m12::rpc_context(&node);
    Gov {
        node,
        m,
        rpc_ctx,
        _t: t,
    }
}

/// `submitMaintainerChange` carrying `tx`'s action, target, signature entries and reason.
pub async fn submit_change(ctx: &RpcContext, tx: &Transaction) -> Result<Value, RpcError> {
    let d = m11::data(tx);
    let action = if tx.tx_type == TxType::AddMaintainer {
        "add"
    } else {
        "remove"
    };
    let signatures: Vec<Value> = d
        .signatures
        .iter()
        .map(|s| json!({"pubkey": s.pubkey.to_hex(), "signature": s.signature.to_hex()}))
        .collect();
    let resp = ctx
        .handle_request(JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: "submitMaintainerChange".into(),
            params: json!({
                "action": action,
                "target_pubkey": d.target.to_hex(),
                "signatures": signatures,
                "reason": d.reason,
            }),
            id: json!(1),
        })
        .await;
    match resp.error {
        Some(e) => Err(e),
        None => Ok(resp.result.unwrap_or(Value::Null)),
    }
}

pub fn text(res: &Result<Value, RpcError>) -> String {
    match res {
        Ok(v) => format!("Ok({v})"),
        Err(e) => m12::error_text(e),
    }
}

/// Non-resident `tx`: gossip Ignore + not admitted, sendTransaction refused with `code`, 0 body reads; else the row's failure.
pub async fn cheap_refusal_failure(
    node: &Node,
    ctx: &RpcContext,
    row: &str,
    tx: &Transaction,
    code: &str,
) -> Option<String> {
    let before = reads(node).await;
    let v = node.handle_gossip_txs(vec![tx.clone()]).await;
    let resident = m8::has(node, tx).await;
    let rpc = text(&m12::send_rpc(ctx, tx).await);
    let rpc_resident = m8::has(node, tx).await;
    let spent = reads(node).await.saturating_sub(before);
    let ok = v == Verdict::Ignore && !resident && rpc.contains(code) && !rpc_resident && spent == 0;
    (!ok).then(|| {
        format!(
            "{row}: verdict={v:?} resident={resident} rpc={rpc} rpc_resident={rpc_resident} body_reads={spent}"
        )
    })
}

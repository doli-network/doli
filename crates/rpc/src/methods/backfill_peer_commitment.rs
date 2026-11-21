//! Phase 2 peer chain commitment for `backfillFromPeer` (INC-I-308 M5).

use crypto::Hash;
use serde_json::{json, Value};
use storage::BlockStore;
use tracing::warn;

/// The peer's chain commitment, or why the divergence check is skipped.
/// `token` is the PEER's admin token; the local node's token is never sent.
pub(super) async fn fetch(
    client: &reqwest::Client,
    url: &str,
    token: Option<&str>,
) -> Result<String, String> {
    let mut req = client.post(url).json(&json!({
        "jsonrpc": "2.0", "method": "verifyChainIntegrity", "params": [], "id": 1
    }));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let body: Value = match req.send().await {
        Ok(resp) => resp
            .json()
            .await
            .map_err(|_| "peer reply to verifyChainIntegrity is not JSON".to_string())?,
        Err(_) => return Err("peer unreachable for verifyChainIntegrity".to_string()),
    };
    if let Some(err) = body.get("error").filter(|e| !e.is_null()) {
        let code = err.get("code").cloned().unwrap_or(Value::Null);
        return Err(format!(
            "peer refused verifyChainIntegrity (code {code}, pass peer_token)"
        ));
    }
    match body
        .pointer("/result/chainCommitment")
        .and_then(Value::as_str)
    {
        Some(c) if !c.is_empty() => Ok(c.to_string()),
        _ => Err("peer sent no chainCommitment".to_string()),
    }
}

/// Phase 2, report only: compare chain commitments and name the fork point. Never writes.
pub(super) async fn report(
    store: &BlockStore,
    peer_url: &str,
    token: Option<&str>,
    local_commitment: Hash,
    tip_height: u64,
) -> String {
    let client = reqwest::Client::new();
    let peer_commitment = match fetch(&client, peer_url, token).await {
        Err(reason) => {
            warn!("Backfill: {reason}: divergence check skipped");
            return format!("skipped: {reason}");
        }
        Ok(c) => c,
    };
    if peer_commitment == hex::encode(local_commitment.as_bytes()) {
        return "ok".to_string();
    }
    let mut fork_point = None;
    for h in (1..=tip_height).rev() {
        let Ok(Some(local_block)) = store.get_block_by_height(h) else {
            continue;
        };
        let Ok(resp) = client
            .post(peer_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0", "method": "getBlockByHeight", "params": [h], "id": h
            }))
            .send()
            .await
        else {
            continue;
        };
        let Ok(body) = resp.json::<serde_json::Value>().await else {
            continue;
        };
        let peer_hash = body["result"]["hash"].as_str().unwrap_or("");
        if peer_hash.is_empty() {
            continue;
        }
        if peer_hash == local_block.hash().to_hex() {
            break;
        }
        fork_point = Some(h);
    }
    match fork_point {
        Some(h) => format!("diverged: peer differs from h={h}; not repaired"),
        None => "diverged: commitment differs, no block divergence found; not repaired".to_string(),
    }
}

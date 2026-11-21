//! Phase 2 peer chain commitment for `backfillFromPeer` (INC-I-308 M5).

use serde_json::{json, Value};

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

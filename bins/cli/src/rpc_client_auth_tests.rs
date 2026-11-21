// OUTPUT CONTRACT:
//   RpcClient::with_admin_token(self, Option<String>) -> Self (receiver carries the token).
//   call_raw(method, params) -> Result<Value>; side effect = HTTP request head sent to the node.
//   verify_chain_integrity() (typed call path) -> Result<_>; same request-head side effect.
//   Paths x outputs:
//     admin method + token      -> request has exactly one "Authorization: Bearer <tok>"; Ok(result)
//     public method + token     -> request has no Authorization header, token bytes absent; Ok(result)
//     admin method + -32008     -> Err whose text contains DOLI_RPC_ADMIN_TOKEN and rpc-admin.token
//     typed admin call + token  -> request has the Bearer header
// INPUT PARTITIONS:
//   method: admin (pauseProduction, createCheckpoint, verifyChainIntegrity) | public (getChainInfo)
//   reply: JSON-RPC result | JSON-RPC error -32008
//   token: valid 64-hex | wrong value

use crate::rpc_client::RpcClient;
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;

const TOKEN: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

fn mock_server(reply_body: String) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let head_end = loop {
            let n = stream.read(&mut chunk).expect("read head");
            assert!(n > 0, "client closed before head end");
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let content_length = head
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                if k.trim().eq_ignore_ascii_case("content-length") {
                    v.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);
        while buf.len() < head_end + content_length {
            let n = stream.read(&mut chunk).expect("read body");
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            reply_body.len(),
            reply_body
        );
        stream.write_all(response.as_bytes()).expect("write reply");
        stream.flush().unwrap();
        String::from_utf8_lossy(&buf).to_string()
    });
    (url, handle)
}

fn ok_reply() -> String {
    json!({"jsonrpc": "2.0", "id": 1, "result": {"ok": true}}).to_string()
}

fn authorization_values(raw: &str) -> Vec<String> {
    raw.split("\r\n\r\n")
        .next()
        .unwrap_or("")
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            if k.trim().eq_ignore_ascii_case("authorization") {
                Some(v.trim().to_string())
            } else {
                None
            }
        })
        .collect()
}

/// R-5 — Decision: without the header every operator admin command is refused by the gated node.
#[tokio::test]
async fn admin_call_carries_bearer_header() {
    let (url, server) = mock_server(ok_reply());
    let client = RpcClient::new(&url).with_admin_token(Some(TOKEN.to_string()));
    let result = client.call_raw("pauseProduction", json!({})).await;
    let raw = server.join().expect("mock thread");
    assert!(result.is_ok(), "call_raw: {result:?}");
    assert!(raw.contains("pauseProduction"), "request: {raw}");
    assert_eq!(
        authorization_values(&raw),
        vec![format!("Bearer {TOKEN}")],
        "request: {raw}"
    );
}

/// R-5 — Decision: the typed path (verify_chain_integrity) must carry the token like call_raw.
#[tokio::test]
async fn typed_admin_call_carries_bearer_header() {
    let (url, server) = mock_server(ok_reply());
    let client = RpcClient::new(&url).with_admin_token(Some(TOKEN.to_string()));
    let _ = client.verify_chain_integrity().await;
    let raw = server.join().expect("mock thread");
    assert!(raw.contains("verifyChainIntegrity"), "request: {raw}");
    assert_eq!(
        authorization_values(&raw),
        vec![format!("Bearer {TOKEN}")],
        "request: {raw}"
    );
}

/// R-5 — Decision: a token on public calls leaks the admin secret to any public endpoint the CLI talks to.
#[tokio::test]
async fn public_call_carries_no_auth_header() {
    let (url, server) = mock_server(ok_reply());
    let client = RpcClient::new(&url).with_admin_token(Some(TOKEN.to_string()));
    let result = client.call_raw("getChainInfo", json!({})).await;
    let raw = server.join().expect("mock thread");
    assert!(result.is_ok(), "call_raw: {result:?}");
    assert!(raw.contains("getChainInfo"), "request: {raw}");
    assert!(authorization_values(&raw).is_empty(), "request: {raw}");
    assert!(!raw.contains(TOKEN), "token leaked: {raw}");
}

/// R-5 — Decision: a bare "Unauthorized" leaves the operator without a path to the token.
#[tokio::test]
async fn admin_unauthorized_reply_prints_hint() {
    let reply = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "error": {"code": -32008, "message": "Unauthorized"}
    })
    .to_string();
    let (url, server) = mock_server(reply);
    let client = RpcClient::new(&url).with_admin_token(Some("wrong-token".to_string()));
    let result = client.call_raw("createCheckpoint", json!({})).await;
    let raw = server.join().expect("mock thread");
    assert_eq!(
        authorization_values(&raw),
        vec!["Bearer wrong-token".to_string()]
    );
    let err = result.expect_err("-32008 must surface as Err").to_string();
    assert!(err.contains("DOLI_RPC_ADMIN_TOKEN"), "error text: {err}");
    assert!(err.contains("rpc-admin.token"), "error text: {err}");
}

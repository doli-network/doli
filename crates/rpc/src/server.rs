//! RPC HTTP server

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{ConnectInfo, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use tokio::sync::broadcast;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tracing::{error, info, warn};

use crate::error::RpcError;
use crate::methods::RpcContext;
use crate::types::{JsonRpcRequest, JsonRpcResponse};
use crate::ws::{self, WsEvent};

/// Maximum request body size.
/// Must accommodate hex-encoded NFT data: 512KB binary × 2 (hex) + 64KB JSON overhead = ~1.1MB.
/// Set to 2MB to cover Era 0 max_extra_data_size (512KB) with headroom.
const MAX_BODY_SIZE: usize = 2 * 1024 * 1024;

/// Methods that require admin authentication (bearer token).
/// These methods can halt production, delete data, or trigger outbound requests.
pub const ADMIN_METHODS: &[&str] = &[
    "pauseProduction",
    "resumeProduction",
    "createCheckpoint",
    "pruneBlocks",
    "backfillFromPeer",
    "enterRecoveryMode",
    "exitRecoveryMode",
    "bridgeFromArchive",
    // AUDIT-RPC2-011: Debug/diagnostic methods that expose full state or trigger
    // expensive scans. Previously unauthenticated — now require admin token.
    "getUtxoDiff",
    "getStateSnapshot",
    "getStateRootDebug",
    "verifyChainIntegrity",
    // ISSUE-174 NEW-1: outbound HTTP fetcher — unauthenticated SSRF if open.
    // Accepts caller-supplied URL and makes HTTP POST request on the node's behalf.
    "repairArchiveFromPeer",
];

/// RPC server configuration
#[derive(Clone, Debug)]
pub struct RpcServerConfig {
    /// Listen address
    pub listen_addr: SocketAddr,
    /// Enable CORS
    pub enable_cors: bool,
    /// Allowed origins (if CORS enabled). Empty = deny all cross-origin.
    pub allowed_origins: Vec<String>,
    /// Bearer token for admin methods. None = admin methods disabled when RPC is network-accessible.
    pub admin_token: Option<String>,
    /// ISSUE-174 #1: IPs of trusted reverse proxies (e.g., Nginx). When the immediate
    /// TCP peer is one of these, `X-Real-IP` / `X-Forwarded-For` is parsed to obtain
    /// the actual client IP for the admin-network trust check. Empty (default) =
    /// header parsing disabled, peer IP is used directly.
    pub trusted_proxies: Vec<std::net::IpAddr>,
}

impl Default for RpcServerConfig {
    /// Creates default config with mainnet RPC port (8500).
    ///
    /// **Note**: For network-aware configuration, prefer constructing
    /// RpcServerConfig explicitly with `NetworkParams::load(network).default_rpc_port`.
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:8500".parse().expect("valid socket addr"),
            enable_cors: false,
            allowed_origins: vec![],
            admin_token: None,
            trusted_proxies: vec![],
        }
    }
}

/// RPC server
pub struct RpcServer {
    config: RpcServerConfig,
    context: Arc<RpcContext>,
    ws_sender: Arc<broadcast::Sender<WsEvent>>,
}

impl RpcServer {
    /// Create a new RPC server
    pub fn new(
        config: RpcServerConfig,
        context: RpcContext,
        ws_sender: broadcast::Sender<WsEvent>,
    ) -> Self {
        Self {
            config,
            context: Arc::new(context),
            ws_sender: Arc::new(ws_sender),
        }
    }

    /// Run the server
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Build shared state: context + admin token + trusted proxy list
        let shared = Arc::new(RpcSharedState {
            context: self.context.clone(),
            admin_token: self.config.admin_token.clone(),
            trusted_proxies: self.config.trusted_proxies.clone(),
        });

        let rpc_router = Router::new()
            .route("/", post(handle_rpc))
            .with_state(shared)
            .layer(RequestBodyLimitLayer::new(MAX_BODY_SIZE));

        let ws_router = Router::new()
            .route("/ws", get(ws::ws_handler))
            .with_state(self.ws_sender.clone());

        let mut app = rpc_router.merge(ws_router);

        // Add CORS if enabled — use allowed_origins if specified, never wildcard
        if self.config.enable_cors {
            let cors = if self.config.allowed_origins.is_empty() {
                // No origins configured: allow only same-origin (no CORS header)
                warn!(
                    "CORS enabled but no allowed_origins configured — using restrictive defaults"
                );
                CorsLayer::new().allow_methods(Any).allow_headers(Any)
            } else {
                let origins: Vec<_> = self
                    .config
                    .allowed_origins
                    .iter()
                    .filter_map(|o| o.parse().ok())
                    .collect();
                CorsLayer::new()
                    .allow_origin(AllowOrigin::list(origins))
                    .allow_methods(Any)
                    .allow_headers(Any)
            };
            app = app.layer(cors);
        }

        info!("RPC server listening on {}", self.config.listen_addr);

        let listener = tokio::net::TcpListener::bind(self.config.listen_addr).await?;
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await?;

        Ok(())
    }

    /// Run the server in the background
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            if let Err(e) = self.run().await {
                error!("RPC server error: {}", e);
            }
        })
    }
}

/// Shared state passed to the Axum handler: RPC context + auth config.
struct RpcSharedState {
    context: Arc<RpcContext>,
    admin_token: Option<String>,
    trusted_proxies: Vec<std::net::IpAddr>,
}

/// Check whether a request is authorized for an admin method.
///
/// Rules:
/// - Request from loopback or private IP: admin methods allowed without token
/// - Public IP + no token configured: admin methods DENIED
/// - Public IP + token configured: require `Authorization: Bearer <token>`
///
/// ISSUE-174 #1: When the immediate TCP peer is in `trusted_proxies`, the
/// `X-Real-IP` / `X-Forwarded-For` headers are resolved to the real client IP
/// before the trust-network check. This closes the historical bypass where
/// Nginx (or any reverse proxy) made every request appear as 127.0.0.1.
fn check_admin_auth(
    shared: &RpcSharedState,
    headers: &HeaderMap,
    method: &str,
    client_addr: SocketAddr,
) -> Result<(), RpcError> {
    if !ADMIN_METHODS.contains(&method) {
        return Ok(());
    }

    // Resolve the effective client IP. If the TCP peer is one of our configured
    // trusted reverse proxies, the proxy MUST set X-Real-IP / X-Forwarded-For
    // and we use that. Otherwise headers are ignored — an attacker cannot forge
    // their way to a trusted IP by setting their own X-Forwarded-For.
    let effective_ip = resolve_client_ip(client_addr.ip(), headers, &shared.trusted_proxies);

    // Trusted networks: loopback (127.x) and private (RFC 1918).
    // All operator servers communicate over private IPs — these are trusted.
    // Public IPs require token auth to prevent external abuse.
    if is_trusted_network(effective_ip) {
        return Ok(());
    }

    // Network-accessible: require token
    match &shared.admin_token {
        None => {
            warn!(
                "Admin method '{}' rejected: no admin_token configured for network-accessible RPC",
                method
            );
            Err(RpcError::unauthorized())
        }
        Some(expected) => {
            let provided = headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "));

            match provided {
                // AUDIT-RPC3-001: Use constant-time comparison to prevent
                // timing side-channel attacks on the admin bearer token.
                Some(token) if constant_time_eq(token.as_bytes(), expected.as_bytes()) => Ok(()),
                _ => {
                    warn!(
                        "Admin method '{}' rejected: invalid or missing bearer token",
                        method
                    );
                    Err(RpcError::unauthorized())
                }
            }
        }
    }
}

/// Constant-time byte comparison to prevent timing side-channel attacks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Check if an IP is in a trusted network (loopback or RFC 1918 private).
fn is_trusted_network(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_loopback() || v4.is_private(),
        std::net::IpAddr::V6(v6) => v6.is_loopback(),
    }
}

/// Resolve the effective client IP for trust-check purposes.
///
/// If `peer_ip` is in `trusted_proxies`, look at `X-Real-IP` first (single IP),
/// then the LEFTMOST entry of `X-Forwarded-For` (canonical original-client position).
/// Returns `peer_ip` unchanged when no proxy is trusted or headers are absent/invalid —
/// an attacker reaching the node directly cannot forge a "trusted" client IP.
fn resolve_client_ip(
    peer_ip: std::net::IpAddr,
    headers: &HeaderMap,
    trusted_proxies: &[std::net::IpAddr],
) -> std::net::IpAddr {
    if trusted_proxies.is_empty() || !trusted_proxies.contains(&peer_ip) {
        return peer_ip;
    }

    if let Some(real_ip) = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<std::net::IpAddr>().ok())
    {
        return real_ip;
    }

    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = xff.split(',').next() {
            if let Ok(ip) = first.trim().parse::<std::net::IpAddr>() {
                return ip;
            }
        }
    }

    peer_ip
}

/// Handle JSON-RPC request — manually parse body so malformed JSON returns
/// a proper JSON-RPC error instead of Axum's default plain-text 422.
async fn handle_rpc(
    State(shared): State<Arc<RpcSharedState>>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Parse JSON body manually
    let request: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => {
            let resp = JsonRpcResponse::error(serde_json::Value::Null, RpcError::parse_error());
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                Json(resp),
            );
        }
    };

    // Validate JSON-RPC version
    if request.jsonrpc != "2.0" {
        let resp = JsonRpcResponse::error(request.id, RpcError::invalid_request());
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            Json(resp),
        );
    }

    // Check admin authorization
    if let Err(e) = check_admin_auth(&shared, &headers, &request.method, client_addr) {
        let resp = JsonRpcResponse::error(request.id, e);
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            Json(resp),
        );
    }

    let response = shared.context.handle_request(request).await;
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(response),
    )
}

/// Handle batch JSON-RPC requests
#[allow(dead_code)]
async fn handle_batch_rpc(
    State(context): State<Arc<RpcContext>>,
    Json(requests): Json<Vec<JsonRpcRequest>>,
) -> impl IntoResponse {
    let mut responses = Vec::with_capacity(requests.len());

    for request in requests {
        if request.jsonrpc != "2.0" {
            responses.push(JsonRpcResponse::error(
                request.id,
                RpcError::invalid_request(),
            ));
            continue;
        }

        let response = context.handle_request(request).await;
        responses.push(response);
    }

    Json(responses)
}

#[cfg(test)]
mod proxy_tests {
    use super::*;
    use axum::http::HeaderValue;
    use std::net::{IpAddr, Ipv4Addr};

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn headers_xff(val: &'static str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", HeaderValue::from_static(val));
        h
    }
    fn headers_xrip(val: &'static str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-real-ip", HeaderValue::from_static(val));
        h
    }

    #[test]
    fn no_trusted_proxies_returns_peer_ip_even_with_forged_header() {
        // ISSUE-174 #1 regression: an attacker cannot reach a "trusted" IP just by
        // setting X-Forwarded-For unless the operator opted in to header parsing.
        let peer = ip("203.0.113.42");
        let headers = headers_xff("127.0.0.1");
        assert_eq!(resolve_client_ip(peer, &headers, &[]), peer);
    }

    #[test]
    fn peer_not_in_trusted_proxies_returns_peer_ip() {
        // Random public peer claims to be Nginx — must be ignored.
        let peer = ip("203.0.113.42");
        let headers = headers_xrip("127.0.0.1");
        let trusted = vec![ip("127.0.0.1")];
        assert_eq!(resolve_client_ip(peer, &headers, &trusted), peer);
    }

    #[test]
    fn trusted_proxy_x_real_ip_takes_precedence() {
        // Nginx is the peer (127.0.0.1) and reports the real client.
        let peer = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let headers = headers_xrip("203.0.113.42");
        let trusted = vec![peer];
        assert_eq!(
            resolve_client_ip(peer, &headers, &trusted),
            ip("203.0.113.42")
        );
    }

    #[test]
    fn trusted_proxy_x_forwarded_for_leftmost_is_used() {
        let peer = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let headers = headers_xff("198.51.100.7, 10.0.0.1");
        let trusted = vec![peer];
        assert_eq!(
            resolve_client_ip(peer, &headers, &trusted),
            ip("198.51.100.7")
        );
    }

    #[test]
    fn trusted_proxy_no_headers_falls_back_to_peer() {
        // Misconfigured Nginx (not setting headers) → behavior is "trust peer",
        // which is the same vulnerable behavior we had before. Operator must
        // configure Nginx correctly when opting into header parsing.
        let peer = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let trusted = vec![peer];
        assert_eq!(resolve_client_ip(peer, &HeaderMap::new(), &trusted), peer);
    }

    #[test]
    fn malformed_header_falls_back_to_peer() {
        let peer = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let headers = headers_xrip("not-an-ip");
        let trusted = vec![peer];
        assert_eq!(resolve_client_ip(peer, &headers, &trusted), peer);
    }
}

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
    // INC-I-204 M4.1 / REQ-FORK-012: retracts applied state across the finality
    // marker on operator authority. Never reachable without the bearer token.
    "forceReorgTo",
];

/// Admin methods with extra operator rules beyond the bearer token (which every
/// admin method needs from every address). Each entry must also be in `ADMIN_METHODS`.
pub const OPERATOR_TOKEN_METHODS: &[&str] = &["forceReorgTo"];

/// RPC server configuration
#[derive(Clone, Debug)]
pub struct RpcServerConfig {
    /// Listen address
    pub listen_addr: SocketAddr,
    /// Enable CORS
    pub enable_cors: bool,
    /// Allowed origins (if CORS enabled). Empty = deny all cross-origin.
    pub allowed_origins: Vec<String>,
    /// Bearer token for admin methods (from every address). None or shorter than
    /// `MIN_ADMIN_TOKEN_LEN` = every admin method refused.
    pub admin_token: Option<String>,
    /// ISSUE-174 #1: IPs of trusted reverse proxies (e.g., Nginx). When the immediate
    /// TCP peer is one of these, `X-Real-IP` / `X-Forwarded-For` is parsed to obtain
    /// the actual client IP for admin-auth logging. Empty (default) =
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

/// Admin methods need `Authorization: Bearer <token>` from every address; with no
/// usable token configured every admin call is refused. Public methods pass.
fn check_admin_auth(
    shared: &RpcSharedState,
    headers: &HeaderMap,
    method: &str,
    client_addr: SocketAddr,
) -> Result<(), RpcError> {
    if !ADMIN_METHODS.contains(&method) {
        return Ok(());
    }

    let effective_ip = resolve_client_ip(client_addr.ip(), headers, &shared.trusted_proxies);

    let expected = match shared.admin_token.as_deref() {
        Some(t) if crate::admin_token::is_usable_admin_token(t) => t,
        _ => {
            warn!(
                "Admin method '{}' from {} rejected: no usable admin_token configured",
                method, effective_ip
            );
            return Err(RpcError::unauthorized());
        }
    };

    let provided = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    match provided {
        Some(token) if constant_time_eq(token.as_bytes(), expected.as_bytes()) => Ok(()),
        _ => {
            warn!(
                "Admin method '{}' from {} rejected: invalid or missing bearer token",
                method, effective_ip
            );
            Err(RpcError::unauthorized())
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

/// Resolve the effective client IP for admin-auth logging.
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
mod proxy_tests;

#[cfg(test)]
mod auth_tests;

#[cfg(test)]
mod tests {
    use super::ADMIN_METHODS;

    /// INC-I-204 M4.1. REQ-FORK-012 — Decision: a failure means `forceReorgTo` is
    /// reachable without the admin bearer token, so anyone who can reach the RPC
    /// port can retract a finalized block — a strictly worse door than the LB-4
    /// bypass this milestone replaces.
    #[test]
    fn force_reorg_to_is_admin_gated() {
        assert!(
            ADMIN_METHODS.contains(&"forceReorgTo"),
            "`forceReorgTo` retracts applied state across the finality marker. \
             `check_admin_auth` is the ONLY auth surface that exists, and it acts \
             solely on ADMIN_METHODS membership — an absent entry is an open door."
        );
    }

    /// REQ-FORK-012 — Decision: a failure means adding the new entry silently
    /// dropped an existing one, quietly un-gating a method that halts production,
    /// deletes data, or makes outbound requests on the node's behalf.
    #[test]
    fn the_admin_gate_is_only_ever_widened() {
        for method in [
            "pauseProduction",
            "resumeProduction",
            "createCheckpoint",
            "pruneBlocks",
            "backfillFromPeer",
            "enterRecoveryMode",
            "exitRecoveryMode",
            "bridgeFromArchive",
            "getUtxoDiff",
            "getStateSnapshot",
            "getStateRootDebug",
            "verifyChainIntegrity",
            "repairArchiveFromPeer",
        ] {
            assert!(
                ADMIN_METHODS.contains(&method),
                "`{method}` was admin-gated before INC-I-204 M4.1 and must stay gated"
            );
        }

        let mut seen: Vec<&str> = ADMIN_METHODS.to_vec();
        seen.sort_unstable();
        let len = seen.len();
        seen.dedup();
        assert_eq!(len, seen.len(), "ADMIN_METHODS must not contain duplicates");
    }
}

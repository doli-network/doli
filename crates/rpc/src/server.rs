//! RPC HTTP server

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::State,
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

/// Maximum request body size (256 KB)
const MAX_BODY_SIZE: usize = 256 * 1024;

/// Methods that require admin authentication (bearer token).
/// These methods can halt production, delete data, or trigger outbound requests.
pub const ADMIN_METHODS: &[&str] = &[
    "pauseProduction",
    "resumeProduction",
    "createCheckpoint",
    "pruneBlocks",
    "backfillFromPeer",
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
        // Build shared state: context + admin token
        let shared = Arc::new(RpcSharedState {
            context: self.context.clone(),
            admin_token: self.config.admin_token.clone(),
            is_localhost: self.config.listen_addr.ip().is_loopback(),
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
        axum::serve(listener, app).await?;

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
    is_localhost: bool,
}

/// Check whether a request is authorized for an admin method.
///
/// Rules:
/// - Localhost binding: admin methods allowed without token (backward compat)
/// - Network-accessible binding + no token configured: admin methods DENIED
/// - Network-accessible binding + token configured: require `Authorization: Bearer <token>`
fn check_admin_auth(
    shared: &RpcSharedState,
    headers: &HeaderMap,
    method: &str,
) -> Result<(), RpcError> {
    if !ADMIN_METHODS.contains(&method) {
        return Ok(());
    }

    // Localhost is trusted (backward compatible)
    if shared.is_localhost {
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
                Some(token) if token == expected.as_str() => Ok(()),
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

/// Handle JSON-RPC request — manually parse body so malformed JSON returns
/// a proper JSON-RPC error instead of Axum's default plain-text 422.
async fn handle_rpc(
    State(shared): State<Arc<RpcSharedState>>,
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
    if let Err(e) = check_admin_auth(&shared, &headers, &request.method) {
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

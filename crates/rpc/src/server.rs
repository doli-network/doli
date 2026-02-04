//! RPC HTTP server

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::post, Json, Router};
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info};

use crate::error::RpcError;
use crate::methods::RpcContext;
use crate::types::{JsonRpcRequest, JsonRpcResponse};

/// RPC server configuration
#[derive(Clone, Debug)]
pub struct RpcServerConfig {
    /// Listen address
    pub listen_addr: SocketAddr,
    /// Enable CORS
    pub enable_cors: bool,
    /// Allowed origins (if CORS enabled)
    pub allowed_origins: Vec<String>,
}

impl Default for RpcServerConfig {
    /// Creates default config with mainnet RPC port (8545).
    ///
    /// **Note**: For network-aware configuration, prefer constructing
    /// RpcServerConfig explicitly with `NetworkParams::load(network).default_rpc_port`.
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:8545".parse().expect("valid socket addr"),
            enable_cors: true,
            allowed_origins: vec!["*".to_string()],
        }
    }
}

/// RPC server
pub struct RpcServer {
    config: RpcServerConfig,
    context: Arc<RpcContext>,
}

impl RpcServer {
    /// Create a new RPC server
    pub fn new(config: RpcServerConfig, context: RpcContext) -> Self {
        Self {
            config,
            context: Arc::new(context),
        }
    }

    /// Run the server
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut app = Router::new()
            .route("/", post(handle_rpc))
            .with_state(self.context.clone());

        // Add CORS if enabled
        if self.config.enable_cors {
            let cors = CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any);
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

/// Handle JSON-RPC request
async fn handle_rpc(
    State(context): State<Arc<RpcContext>>,
    Json(request): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    // Validate JSON-RPC version
    if request.jsonrpc != "2.0" {
        return Json(JsonRpcResponse::error(
            request.id,
            RpcError::invalid_request(),
        ));
    }

    let response = context.handle_request(request).await;
    Json(response)
}

/// Handle batch JSON-RPC requests
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

//! WebSocket subscription handler
//!
//! Clients connect to `/ws` and receive real-time events:
//! - `new_block`: emitted when a new block is applied
//! - `new_tx`: emitted when a new transaction enters the mempool

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use serde::Serialize;
use tokio::sync::broadcast;
use tracing::{debug, warn};

/// Maximum concurrent WebSocket connections
const MAX_WS_CONNECTIONS: usize = 100;

/// Global WebSocket connection counter
static WS_CONNECTION_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Events broadcast to WebSocket clients
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsEvent {
    /// New block applied to the canonical chain
    NewBlock {
        hash: String,
        height: u64,
        slot: u32,
        timestamp: u64,
        producer: String,
        tx_count: usize,
    },
    /// New transaction entered the mempool
    NewTx {
        hash: String,
        tx_type: String,
        size: usize,
        fee: u64,
    },
}

/// Create a new broadcast channel for WebSocket events
pub fn broadcast_channel() -> (broadcast::Sender<WsEvent>, broadcast::Receiver<WsEvent>) {
    broadcast::channel(256)
}

/// Axum handler for WebSocket upgrade
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(sender): State<Arc<broadcast::Sender<WsEvent>>>,
) -> Result<impl IntoResponse, axum::http::StatusCode> {
    let current = WS_CONNECTION_COUNT.load(Ordering::Relaxed);
    if current >= MAX_WS_CONNECTIONS {
        warn!(
            "WebSocket connection rejected: limit reached ({}/{})",
            current, MAX_WS_CONNECTIONS
        );
        return Err(axum::http::StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(ws.on_upgrade(move |socket| handle_socket(socket, sender)))
}

async fn handle_socket(mut socket: WebSocket, sender: Arc<broadcast::Sender<WsEvent>>) {
    WS_CONNECTION_COUNT.fetch_add(1, Ordering::Relaxed);
    let mut rx = sender.subscribe();

    loop {
        tokio::select! {
            // Forward broadcast events to the WebSocket client
            event = rx.recv() => {
                match event {
                    Ok(ev) => {
                        let json = match serde_json::to_string(&ev) {
                            Ok(j) => j,
                            Err(_) => continue,
                        };
                        if socket.send(Message::Text(json)).await.is_err() {
                            break; // Client disconnected
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        debug!("WebSocket client lagged by {} events", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            // Handle incoming messages (ping/pong, close)
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    #[allow(clippy::collapsible_match)]
                    Some(Ok(Message::Ping(data))) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        warn!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {} // Ignore text/binary from clients
                }
            }
        }
    }

    WS_CONNECTION_COUNT.fetch_sub(1, Ordering::Relaxed);
}

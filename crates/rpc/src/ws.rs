//! WebSocket subscription handler
//!
//! Clients connect to `/ws` and receive real-time events:
//! - `new_block`: emitted when a new block is applied
//! - `new_tx`: emitted when a new transaction enters the mempool

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use serde::Serialize;
use tokio::sync::broadcast;
use tracing::{debug, warn};

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
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, sender))
}

async fn handle_socket(mut socket: WebSocket, sender: Arc<broadcast::Sender<WsEvent>>) {
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
}

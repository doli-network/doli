//! Peer discovery
//!
//! Provides two discovery mechanisms:
//! - **Kademlia DHT** (TCP) — existing, uses libp2p's Kademlia for peer routing
//! - **Discv5** (UDP) — stateless UDP discovery, scales to thousands of peers
//!
//! Discv5 is the primary discovery mechanism. Kademlia is kept as fallback.

pub mod discv5_service;
pub mod kademlia;

pub use kademlia::{new_kademlia, KAD_PROTOCOL};

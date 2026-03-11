//! Persistent peer cache for fast reconnection after restart.
//!
//! Stores recently-seen peer addresses to disk so the node can reconnect
//! without relying on DNS seeds or bootstrap nodes.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use libp2p::Multiaddr;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

const MAX_CACHED_PEERS: usize = 100;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CachedPeer {
    pub peer_id: String,
    pub address: String,
    pub last_seen: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PeerCache {
    pub peers: Vec<CachedPeer>,
}

impl PeerCache {
    /// Load cache from disk. Returns None if missing or corrupt.
    pub fn load(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        match bincode::deserialize::<Self>(&bytes) {
            Ok(cache) => {
                debug!(
                    "Loaded {} cached peers from {}",
                    cache.peers.len(),
                    path.display()
                );
                Some(cache)
            }
            Err(e) => {
                warn!("Corrupt peer cache at {}: {}", path.display(), e);
                None
            }
        }
    }

    /// Save cache to disk atomically (write .tmp, then rename).
    pub fn save(&self, path: &Path) {
        let tmp = PathBuf::from(format!("{}.tmp", path.display()));
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                warn!("Failed to create peer cache directory: {}", e);
                return;
            }
        }
        let bytes = match bincode::serialize(self) {
            Ok(b) => b,
            Err(e) => {
                warn!("Failed to serialize peer cache: {}", e);
                return;
            }
        };
        if let Err(e) = std::fs::write(&tmp, &bytes) {
            warn!("Failed to write peer cache tmp file: {}", e);
            return;
        }
        if let Err(e) = std::fs::rename(&tmp, path) {
            warn!("Failed to rename peer cache: {}", e);
        }
    }

    /// Upsert a peer, update last_seen, trim to MAX_CACHED_PEERS.
    pub fn add(&mut self, peer_id: &str, address: &str) {
        // Never cache loopback addresses — they cause self-dial loops on remote nodes
        if address.contains("/ip4/127.") || address.contains("/ip6/::1") {
            return;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Remove existing entry so we always append to the end (most recent last)
        self.peers.retain(|p| p.peer_id != peer_id);
        self.peers.push(CachedPeer {
            peer_id: peer_id.to_string(),
            address: address.to_string(),
            last_seen: now,
        });

        // Trim oldest from the front, keeping most recent at the tail
        if self.peers.len() > MAX_CACHED_PEERS {
            let drain_count = self.peers.len() - MAX_CACHED_PEERS;
            self.peers.drain(..drain_count);
        }
    }

    /// Remove a peer by peer_id.
    pub fn remove(&mut self, peer_id: &str) {
        self.peers.retain(|p| p.peer_id != peer_id);
    }

    /// Return cached addresses for dialing.
    pub fn addresses(&self) -> Vec<Multiaddr> {
        self.peers
            .iter()
            .filter_map(|p| p.address.parse().ok())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_add_and_trim() {
        let mut cache = PeerCache::default();
        for i in 0..150 {
            cache.add(&format!("peer_{}", i), &format!("/ip4/1.2.3.4/tcp/{}", i));
        }
        assert_eq!(cache.peers.len(), MAX_CACHED_PEERS);
        // Most recent should be kept
        assert!(cache.peers.iter().any(|p| p.peer_id == "peer_149"));
    }

    #[test]
    fn test_upsert() {
        let mut cache = PeerCache::default();
        cache.add("peer_1", "/ip4/1.2.3.4/tcp/100");
        cache.add("peer_1", "/ip4/5.6.7.8/tcp/200");
        assert_eq!(cache.peers.len(), 1);
        assert_eq!(cache.peers[0].address, "/ip4/5.6.7.8/tcp/200");
    }

    #[test]
    fn test_remove() {
        let mut cache = PeerCache::default();
        cache.add("peer_1", "/ip4/1.2.3.4/tcp/100");
        cache.add("peer_2", "/ip4/5.6.7.8/tcp/200");
        cache.remove("peer_1");
        assert_eq!(cache.peers.len(), 1);
        assert_eq!(cache.peers[0].peer_id, "peer_2");
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.cache");

        let mut cache = PeerCache::default();
        cache.add("peer_1", "/ip4/1.2.3.4/tcp/30300");
        cache.save(&path);

        let loaded = PeerCache::load(&path).unwrap();
        assert_eq!(loaded.peers.len(), 1);
        assert_eq!(loaded.peers[0].peer_id, "peer_1");
    }

    #[test]
    fn test_load_missing_file() {
        let result = PeerCache::load(Path::new("/nonexistent/peers.cache"));
        assert!(result.is_none());
    }

    #[test]
    fn test_load_corrupt_file() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"not valid bincode").unwrap();
        let result = PeerCache::load(tmp.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_addresses() {
        let mut cache = PeerCache::default();
        cache.add("peer_1", "/ip4/1.2.3.4/tcp/30300");
        cache.add("peer_2", "invalid_addr");
        let addrs = cache.addresses();
        assert_eq!(addrs.len(), 1);
    }

    #[test]
    fn test_reject_loopback() {
        let mut cache = PeerCache::default();
        cache.add("peer_1", "/ip4/127.0.0.1/tcp/30300/p2p/12D3KooWTest");
        cache.add("peer_2", "/ip6/::1/tcp/30300/p2p/12D3KooWTest2");
        cache.add("peer_3", "/ip4/198.51.100.1/tcp/30300/p2p/12D3KooWReal");
        assert_eq!(cache.peers.len(), 1);
        assert_eq!(cache.peers[0].peer_id, "peer_3");
    }
}

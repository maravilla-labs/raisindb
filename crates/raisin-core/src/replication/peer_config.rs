//! Peer configuration for CRDT replication
//!
//! This module defines the configuration structures for managing peers
//! in a distributed RaisinDB cluster.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Configuration for a single replication peer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerConfig {
    /// Unique identifier for this peer
    pub peer_id: String,

    /// Base URL for the peer's HTTP API (e.g., "https://peer1.example.com")
    pub url: String,

    /// Whether synchronization with this peer is enabled
    pub enabled: bool,

    /// How often to synchronize with this peer (in seconds)
    /// Default: 60 seconds (1 minute)
    #[serde(default = "default_sync_interval")]
    pub sync_interval_secs: u64,

    /// Maximum number of operations to fetch per sync request
    /// Default: 1000
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,

    /// Timeout for HTTP requests to this peer (in seconds)
    /// Default: 30 seconds
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,

    /// Authentication token for this peer (if required)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,

    /// Branches to subscribe to for replication
    /// If None or empty, subscribes to all branches
    /// If Some(vec), only syncs operations from specified branches
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_filter: Option<Vec<String>>,

    /// Retry configuration
    #[serde(default)]
    pub retry_config: RetryConfig,
}

fn default_sync_interval() -> u64 {
    60
}

fn default_batch_size() -> usize {
    1000
}

fn default_timeout_secs() -> u64 {
    30
}

/// Retry configuration for peer synchronization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    /// Default: 3
    #[serde(default = "default_max_retries")]
    pub max_retries: usize,

    /// Initial backoff delay in seconds
    /// Default: 5 seconds
    #[serde(default = "default_initial_backoff_secs")]
    pub initial_backoff_secs: u64,

    /// Maximum backoff delay in seconds
    /// Default: 300 seconds (5 minutes)
    #[serde(default = "default_max_backoff_secs")]
    pub max_backoff_secs: u64,
}

fn default_max_retries() -> usize {
    3
}

fn default_initial_backoff_secs() -> u64 {
    5
}

fn default_max_backoff_secs() -> u64 {
    300
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            initial_backoff_secs: default_initial_backoff_secs(),
            max_backoff_secs: default_max_backoff_secs(),
        }
    }
}

impl PeerConfig {
    /// Create a new peer configuration
    pub fn new(peer_id: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            peer_id: peer_id.into(),
            url: url.into(),
            enabled: true,
            sync_interval_secs: default_sync_interval(),
            batch_size: default_batch_size(),
            timeout_secs: default_timeout_secs(),
            auth_token: None,
            branch_filter: None,
            retry_config: RetryConfig::default(),
        }
    }

    /// Disable synchronization with this peer
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Set the synchronization interval
    pub fn with_sync_interval(mut self, secs: u64) -> Self {
        self.sync_interval_secs = secs;
        self
    }

    /// Set the batch size
    pub fn with_batch_size(mut self, size: usize) -> Self {
        self.batch_size = size;
        self
    }

    /// Set the HTTP timeout
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    /// Set the authentication token
    pub fn with_auth_token(mut self, token: impl Into<String>) -> Self {
        self.auth_token = Some(token.into());
        self
    }

    /// Set branch filter for selective synchronization
    ///
    /// Only operations from the specified branches will be synced.
    /// If None or empty, all branches are synchronized.
    pub fn with_branch_filter(mut self, branches: Vec<String>) -> Self {
        self.branch_filter = if branches.is_empty() {
            None
        } else {
            Some(branches)
        };
        self
    }
}

/// Registry for managing multiple replication peers
#[derive(Debug, Clone)]
pub struct PeerRegistry {
    /// Map of peer_id -> PeerConfig
    peers: Arc<RwLock<HashMap<String, PeerConfig>>>,
}

impl PeerRegistry {
    /// Create a new empty peer registry
    pub fn new() -> Self {
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a peer registry from a list of peer configurations
    pub fn from_configs(configs: Vec<PeerConfig>) -> Self {
        let mut peers = HashMap::new();
        for config in configs {
            peers.insert(config.peer_id.clone(), config);
        }

        Self {
            peers: Arc::new(RwLock::new(peers)),
        }
    }

    /// Add a peer to the registry
    pub fn add_peer(&self, config: PeerConfig) {
        let mut peers = self.peers.write().expect("peer registry lock poisoned");
        peers.insert(config.peer_id.clone(), config);
    }

    /// Remove a peer from the registry
    pub fn remove_peer(&self, peer_id: &str) -> Option<PeerConfig> {
        let mut peers = self.peers.write().expect("peer registry lock poisoned");
        peers.remove(peer_id)
    }

    /// Get a peer configuration by ID
    pub fn get_peer(&self, peer_id: &str) -> Option<PeerConfig> {
        let peers = self.peers.read().expect("peer registry lock poisoned");
        peers.get(peer_id).cloned()
    }

    /// List all registered peers
    pub fn list_peers(&self) -> Vec<PeerConfig> {
        let peers = self.peers.read().expect("peer registry lock poisoned");
        peers.values().cloned().collect()
    }

    /// List all enabled peers
    pub fn list_enabled_peers(&self) -> Vec<PeerConfig> {
        let peers = self.peers.read().expect("peer registry lock poisoned");
        peers.values().filter(|p| p.enabled).cloned().collect()
    }

    /// Update a peer configuration
    pub fn update_peer(&self, config: PeerConfig) {
        let mut peers = self.peers.write().expect("peer registry lock poisoned");
        peers.insert(config.peer_id.clone(), config);
    }

    /// Enable a peer
    pub fn enable_peer(&self, peer_id: &str) {
        let mut peers = self.peers.write().expect("peer registry lock poisoned");
        if let Some(peer) = peers.get_mut(peer_id) {
            peer.enabled = true;
        }
    }

    /// Disable a peer
    pub fn disable_peer(&self, peer_id: &str) {
        let mut peers = self.peers.write().expect("peer registry lock poisoned");
        if let Some(peer) = peers.get_mut(peer_id) {
            peer.enabled = false;
        }
    }

    /// Get the number of registered peers
    pub fn peer_count(&self) -> usize {
        let peers = self.peers.read().expect("peer registry lock poisoned");
        peers.len()
    }

    /// Get the number of enabled peers
    pub fn enabled_peer_count(&self) -> usize {
        let peers = self.peers.read().expect("peer registry lock poisoned");
        peers.values().filter(|p| p.enabled).count()
    }
}

impl Default for PeerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peer_config_builder() {
        let config = PeerConfig::new("peer1", "https://peer1.example.com")
            .with_sync_interval(30)
            .with_batch_size(500)
            .with_auth_token("secret123");

        assert_eq!(config.peer_id, "peer1");
        assert_eq!(config.url, "https://peer1.example.com");
        assert_eq!(config.sync_interval_secs, 30);
        assert_eq!(config.batch_size, 500);
        assert_eq!(config.auth_token, Some("secret123".to_string()));
        assert!(config.enabled);
    }

    #[test]
    fn test_peer_registry() {
        let registry = PeerRegistry::new();

        // Add peers
        registry.add_peer(PeerConfig::new("peer1", "https://peer1.example.com"));
        registry.add_peer(PeerConfig::new("peer2", "https://peer2.example.com").disabled());

        assert_eq!(registry.peer_count(), 2);
        assert_eq!(registry.enabled_peer_count(), 1);

        // Get peer
        let peer1 = registry.get_peer("peer1").unwrap();
        assert_eq!(peer1.peer_id, "peer1");
        assert!(peer1.enabled);

        // Disable peer
        registry.disable_peer("peer1");
        assert_eq!(registry.enabled_peer_count(), 0);

        // Enable peer
        registry.enable_peer("peer1");
        assert_eq!(registry.enabled_peer_count(), 1);

        // Remove peer
        let removed = registry.remove_peer("peer2");
        assert!(removed.is_some());
        assert_eq!(registry.peer_count(), 1);
    }

    #[test]
    fn test_list_enabled_peers() {
        let registry = PeerRegistry::new();

        registry.add_peer(PeerConfig::new("peer1", "https://peer1.example.com"));
        registry.add_peer(PeerConfig::new("peer2", "https://peer2.example.com").disabled());
        registry.add_peer(PeerConfig::new("peer3", "https://peer3.example.com"));

        let enabled = registry.list_enabled_peers();
        assert_eq!(enabled.len(), 2);
        assert!(enabled.iter().any(|p| p.peer_id == "peer1"));
        assert!(enabled.iter().any(|p| p.peer_id == "peer3"));
        assert!(!enabled.iter().any(|p| p.peer_id == "peer2"));
    }
}

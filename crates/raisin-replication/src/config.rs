//! Cluster and replication configuration
//!
//! This module defines configuration structures for peer-to-peer replication.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Complete cluster configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterConfig {
    /// This node's unique identifier in the cluster
    pub node_id: String,

    /// TCP port for replication connections (default: 9001)
    #[serde(default = "default_replication_port")]
    pub replication_port: u16,

    /// Bind address for replication server (default: "0.0.0.0")
    #[serde(default = "default_bind_address")]
    pub bind_address: String,

    /// List of peer nodes to connect to
    #[serde(default)]
    pub peers: Vec<PeerConfig>,

    /// Sync configuration
    #[serde(default)]
    pub sync: SyncConfig,

    /// Connection pool configuration
    #[serde(default)]
    pub connection: ConnectionConfig,

    /// List of tenant/repo pairs to sync (default: [["default", "default"]])
    /// Each pair is [tenant_id, repo_id]
    #[serde(default = "default_sync_tenants")]
    pub sync_tenants: Vec<(String, String)>,
}

/// Configuration for a single peer node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerConfig {
    /// Unique identifier for the peer node
    pub node_id: String,

    /// Hostname or IP address
    pub host: String,

    /// TCP port for replication (default: 9001)
    #[serde(default = "default_replication_port")]
    pub port: u16,

    /// Optional branch filter for selective sync
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_filter: Option<Vec<String>>,

    /// Whether this peer is enabled (default: true)
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Optional priority for connection order (higher = connect first)
    #[serde(default)]
    pub priority: i32,
}

/// Synchronization behavior configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    /// Interval for periodic pull sync in seconds (default: 60)
    #[serde(default = "default_sync_interval")]
    pub interval_seconds: u64,

    /// Maximum operations per batch (default: 1000)
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,

    /// Enable real-time push on commit (default: true)
    #[serde(default = "default_true")]
    pub realtime_push: bool,

    /// Compression algorithm ("none", "zstd", "gzip")
    #[serde(default = "default_compression")]
    pub compression: String,

    /// Compression level (1-22 for zstd, 1-9 for gzip)
    #[serde(default = "default_compression_level")]
    pub compression_level: i32,

    /// Retry configuration
    #[serde(default)]
    pub retry: RetryConfig,
}

/// Connection pooling and management configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    /// Heartbeat interval in seconds (default: 30)
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_seconds: u64,

    /// Connection timeout in seconds (default: 10)
    #[serde(default = "default_connection_timeout")]
    pub connect_timeout_seconds: u64,

    /// Read timeout in seconds (default: 30)
    #[serde(default = "default_read_timeout")]
    pub read_timeout_seconds: u64,

    /// Write timeout in seconds (default: 30)
    #[serde(default = "default_write_timeout")]
    pub write_timeout_seconds: u64,

    /// Maximum concurrent connections per peer (default: 4)
    #[serde(default = "default_max_connections")]
    pub max_connections_per_peer: usize,

    /// Keep-alive interval in seconds (default: 60)
    #[serde(default = "default_keepalive")]
    pub keepalive_seconds: u64,
}

/// Retry behavior configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Base delay for exponential backoff in milliseconds (default: 100)
    #[serde(default = "default_retry_base_ms")]
    pub base_delay_ms: u64,

    /// Maximum retry attempts (default: 10)
    #[serde(default = "default_max_retries")]
    pub max_attempts: usize,

    /// Maximum backoff delay in milliseconds (default: 60000 = 1 minute)
    #[serde(default = "default_max_backoff_ms")]
    pub max_backoff_ms: u64,

    /// Jitter factor (0.0 - 1.0) to randomize retry delays (default: 0.1)
    #[serde(default = "default_jitter")]
    pub jitter_factor: f64,
}

// Default value functions

fn default_replication_port() -> u16 {
    9001
}

fn default_bind_address() -> String {
    "0.0.0.0".to_string()
}

fn default_sync_interval() -> u64 {
    60
}

fn default_batch_size() -> usize {
    1000
}

fn default_true() -> bool {
    true
}

fn default_compression() -> String {
    "zstd".to_string()
}

fn default_compression_level() -> i32 {
    3
}

fn default_heartbeat_interval() -> u64 {
    30
}

fn default_connection_timeout() -> u64 {
    10
}

fn default_read_timeout() -> u64 {
    30
}

fn default_write_timeout() -> u64 {
    30
}

fn default_max_connections() -> usize {
    4
}

fn default_keepalive() -> u64 {
    60
}

fn default_retry_base_ms() -> u64 {
    100
}

fn default_max_retries() -> usize {
    10
}

fn default_max_backoff_ms() -> u64 {
    60_000
}

fn default_jitter() -> f64 {
    0.1
}

fn default_sync_tenants() -> Vec<(String, String)> {
    vec![("default".to_string(), "default".to_string())]
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            interval_seconds: default_sync_interval(),
            batch_size: default_batch_size(),
            realtime_push: true,
            compression: default_compression(),
            compression_level: default_compression_level(),
            retry: RetryConfig::default(),
        }
    }
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval_seconds: default_heartbeat_interval(),
            connect_timeout_seconds: default_connection_timeout(),
            read_timeout_seconds: default_read_timeout(),
            write_timeout_seconds: default_write_timeout(),
            max_connections_per_peer: default_max_connections(),
            keepalive_seconds: default_keepalive(),
        }
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            base_delay_ms: default_retry_base_ms(),
            max_attempts: default_max_retries(),
            max_backoff_ms: default_max_backoff_ms(),
            jitter_factor: default_jitter(),
        }
    }
}

impl ClusterConfig {
    /// Create a minimal configuration for single-node testing
    pub fn single_node(node_id: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            replication_port: default_replication_port(),
            bind_address: default_bind_address(),
            peers: vec![],
            sync: SyncConfig::default(),
            connection: ConnectionConfig::default(),
            sync_tenants: default_sync_tenants(),
        }
    }

    /// Add a peer to the configuration
    pub fn with_peer(mut self, peer: PeerConfig) -> Self {
        self.peers.push(peer);
        self
    }

    /// Load configuration from TOML file
    pub fn from_toml_file(path: impl AsRef<std::path::Path>) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path).map_err(|e| ConfigError::Io(e.to_string()))?;
        Self::from_toml_str(&content)
    }

    /// Parse configuration from TOML string
    pub fn from_toml_str(toml: &str) -> Result<Self, ConfigError> {
        toml::from_str(toml).map_err(|e| ConfigError::Parse(e.to_string()))
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate node_id is not empty
        if self.node_id.is_empty() {
            return Err(ConfigError::Validation(
                "node_id cannot be empty".to_string(),
            ));
        }

        // Validate peer node_ids are unique
        let mut seen = std::collections::HashSet::new();
        for peer in &self.peers {
            if !seen.insert(&peer.node_id) {
                return Err(ConfigError::Validation(format!(
                    "Duplicate peer node_id: {}",
                    peer.node_id
                )));
            }
        }

        // Validate no self-reference
        for peer in &self.peers {
            if peer.node_id == self.node_id {
                return Err(ConfigError::Validation(
                    "Cannot configure self as peer".to_string(),
                ));
            }
        }

        Ok(())
    }
}

impl PeerConfig {
    /// Create a new peer configuration
    pub fn new(node_id: impl Into<String>, host: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            host: host.into(),
            port: default_replication_port(),
            branch_filter: None,
            enabled: true,
            priority: 0,
        }
    }

    /// Set the port
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Set branch filter for selective sync
    pub fn with_branch_filter(mut self, branches: Vec<String>) -> Self {
        self.branch_filter = Some(branches);
        self
    }

    /// Set priority
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Get the connection address (host:port)
    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

impl RetryConfig {
    /// Calculate delay for a given attempt number
    pub fn delay_for_attempt(&self, attempt: usize) -> Duration {
        if attempt == 0 {
            return Duration::from_millis(0);
        }

        // Exponential backoff: base * 2^(attempt-1)
        let exp_delay = self.base_delay_ms * 2u64.pow((attempt - 1) as u32);
        let capped_delay = exp_delay.min(self.max_backoff_ms);

        // Add jitter
        let jitter = (capped_delay as f64 * self.jitter_factor) as u64;
        let jitter_range = if jitter > 0 {
            rand::random::<u64>() % jitter
        } else {
            0
        };

        Duration::from_millis(capped_delay + jitter_range)
    }
}

/// Configuration errors
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Validation error: {0}")]
    Validation(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ClusterConfig::single_node("node1");
        assert_eq!(config.node_id, "node1");
        assert_eq!(config.replication_port, 9001);
        assert_eq!(config.peers.len(), 0);
    }

    #[test]
    fn test_peer_config() {
        let peer = PeerConfig::new("node2", "10.0.1.2")
            .with_port(9002)
            .with_priority(10)
            .with_branch_filter(vec!["main".to_string()]);

        assert_eq!(peer.node_id, "node2");
        assert_eq!(peer.host, "10.0.1.2");
        assert_eq!(peer.port, 9002);
        assert_eq!(peer.priority, 10);
        assert_eq!(peer.branch_filter, Some(vec!["main".to_string()]));
        assert_eq!(peer.address(), "10.0.1.2:9002");
    }

    #[test]
    fn test_retry_delay() {
        let config = RetryConfig::default();

        // First attempt (0) should have no delay
        assert_eq!(config.delay_for_attempt(0).as_millis(), 0);

        // Subsequent attempts should grow exponentially
        let delay1 = config.delay_for_attempt(1).as_millis();
        let delay2 = config.delay_for_attempt(2).as_millis();
        let delay3 = config.delay_for_attempt(3).as_millis();

        assert!(delay1 >= 100); // At least base delay
        assert!(delay2 > delay1);
        assert!(delay3 > delay2);
    }

    #[test]
    fn test_config_validation() {
        // Valid config
        let config =
            ClusterConfig::single_node("node1").with_peer(PeerConfig::new("node2", "10.0.1.2"));
        assert!(config.validate().is_ok());

        // Empty node_id
        let mut invalid = ClusterConfig::single_node("");
        assert!(invalid.validate().is_err());

        // Duplicate peer
        invalid = ClusterConfig::single_node("node1")
            .with_peer(PeerConfig::new("node2", "10.0.1.2"))
            .with_peer(PeerConfig::new("node2", "10.0.1.3"));
        assert!(invalid.validate().is_err());

        // Self-reference
        invalid =
            ClusterConfig::single_node("node1").with_peer(PeerConfig::new("node1", "10.0.1.2"));
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn test_toml_parsing() {
        let toml = r#"
            node_id = "node1"
            replication_port = 9001
            bind_address = "0.0.0.0"

            [[peers]]
            node_id = "node2"
            host = "10.0.1.2"
            port = 9001

            [[peers]]
            node_id = "node3"
            host = "10.0.1.3"
            port = 9001
            branch_filter = ["main", "develop"]

            [sync]
            interval_seconds = 30
            batch_size = 500
            realtime_push = true
        "#;

        let config = ClusterConfig::from_toml_str(toml).unwrap();
        assert_eq!(config.node_id, "node1");
        assert_eq!(config.peers.len(), 2);
        assert_eq!(config.sync.interval_seconds, 30);
        assert_eq!(config.sync.batch_size, 500);
        assert_eq!(
            config.peers[1].branch_filter,
            Some(vec!["main".to_string(), "develop".to_string()])
        );
    }
}

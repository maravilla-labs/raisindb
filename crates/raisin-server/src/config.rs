// Configuration file support for RaisinDB Server

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Replication peer configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationPeer {
    /// Unique peer identifier
    pub peer_id: String,
    /// Peer's IP address or hostname
    pub address: String,
    /// Peer's replication TCP port
    pub port: u16,
}

/// Replication configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationConfig {
    /// Enable replication
    #[serde(default)]
    pub enabled: bool,
    /// This node's unique ID
    pub node_id: Option<String>,
    /// Replication TCP port for this node
    pub port: Option<u16>,
    /// Bind address for replication server
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
    /// List of peer nodes
    #[serde(default)]
    pub peers: Vec<ReplicationPeer>,
}

fn default_bind_address() -> String {
    "127.0.0.1".to_string()
}

impl Default for ReplicationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            node_id: None,
            port: None,
            bind_address: default_bind_address(),
            peers: Vec::new(),
        }
    }
}

/// Monitoring configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    /// Enable monitoring and metrics collection
    #[serde(default)]
    pub enabled: bool,
    /// Metrics collection interval in seconds
    #[serde(default = "default_monitoring_interval")]
    pub interval_secs: u64,
    /// Port for metrics endpoint (optional - uses main HTTP port if not specified)
    pub port: Option<u16>,
}

fn default_monitoring_interval() -> u64 {
    30
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: default_monitoring_interval(),
            port: None,
        }
    }
}

/// PostgreSQL wire protocol configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PgWireConfig {
    /// Enable PostgreSQL wire protocol server
    #[serde(default)]
    pub enabled: bool,
    /// Bind address for pgwire server
    #[serde(default = "default_pgwire_bind_address")]
    pub bind_address: String,
    /// Port for pgwire server
    #[serde(default = "default_pgwire_port")]
    pub port: u16,
    /// Maximum concurrent connections
    #[serde(default = "default_pgwire_max_connections")]
    pub max_connections: usize,
}

fn default_pgwire_bind_address() -> String {
    "127.0.0.1".to_string()
}

fn default_pgwire_port() -> u16 {
    5432
}

fn default_pgwire_max_connections() -> usize {
    100
}

impl Default for PgWireConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_address: default_pgwire_bind_address(),
            port: default_pgwire_port(),
            max_connections: default_pgwire_max_connections(),
        }
    }
}

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfigFile {
    /// HTTP server configuration
    #[serde(default)]
    pub server: HttpServerConfig,
    /// Replication configuration
    #[serde(default)]
    pub replication: ReplicationConfig,
    /// Monitoring configuration
    #[serde(default)]
    pub monitoring: MonitoringConfig,
    /// PostgreSQL wire protocol configuration
    #[serde(default)]
    pub pgwire: PgWireConfig,
}

/// HTTP server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpServerConfig {
    /// HTTP server port
    #[serde(default = "default_http_port")]
    pub port: u16,
    /// Bind address for HTTP server
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
    /// Data directory path
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
    /// Initial admin password (optional)
    pub initial_admin_password: Option<String>,
    /// Enable anonymous access for unauthenticated requests.
    /// When true, HTTP and WebSocket requests without authentication
    /// will be auto-authenticated as the "anonymous" user with
    /// permissions from the "anonymous" role in access_control.
    #[serde(default)]
    pub anonymous_enabled: bool,
    /// CORS allowed origins for cross-origin requests.
    /// Example: ["http://localhost:5173", "https://app.example.com"]
    /// If empty, CORS will not be configured.
    #[serde(default)]
    pub cors_allowed_origins: Vec<String>,
}

fn default_http_port() -> u16 {
    8080
}

fn default_data_dir() -> String {
    "./.data/rocksdb".to_string()
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            port: default_http_port(),
            bind_address: default_bind_address(),
            data_dir: default_data_dir(),
            initial_admin_password: None,
            anonymous_enabled: false,
            cors_allowed_origins: Vec::new(),
        }
    }
}

impl ServerConfigFile {
    /// Load configuration from a TOML file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file: {}", e))?;

        toml::from_str(&content).map_err(|e| format!("Failed to parse config file: {}", e))
    }

    /// Parse peers from a comma-separated string format:
    /// "node2=127.0.0.1:9002,node3=127.0.0.1:9003"
    pub fn parse_peers_string(peers_str: &str) -> Result<Vec<ReplicationPeer>, String> {
        let mut peers = Vec::new();

        for peer_spec in peers_str.split(',') {
            let peer_spec = peer_spec.trim();
            if peer_spec.is_empty() {
                continue;
            }

            let parts: Vec<&str> = peer_spec.split('=').collect();
            if parts.len() != 2 {
                return Err(format!(
                    "Invalid peer format: '{}'. Expected 'node_id=address:port'",
                    peer_spec
                ));
            }

            let peer_id = parts[0].trim().to_string();
            let addr_parts: Vec<&str> = parts[1].trim().split(':').collect();

            if addr_parts.len() != 2 {
                return Err(format!(
                    "Invalid address format: '{}'. Expected 'address:port'",
                    parts[1]
                ));
            }

            let address = addr_parts[0].to_string();
            let port = addr_parts[1]
                .parse::<u16>()
                .map_err(|_| format!("Invalid port number: '{}'", addr_parts[1]))?;

            peers.push(ReplicationPeer {
                peer_id,
                address,
                port,
            });
        }

        Ok(peers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_peers_string() {
        let peers_str = "node2=127.0.0.1:9002,node3=192.168.1.10:9003";
        let peers = ServerConfigFile::parse_peers_string(peers_str).unwrap();

        assert_eq!(peers.len(), 2);
        assert_eq!(peers[0].peer_id, "node2");
        assert_eq!(peers[0].address, "127.0.0.1");
        assert_eq!(peers[0].port, 9002);
        assert_eq!(peers[1].peer_id, "node3");
        assert_eq!(peers[1].address, "192.168.1.10");
        assert_eq!(peers[1].port, 9003);
    }

    #[test]
    fn test_parse_peers_string_invalid() {
        assert!(ServerConfigFile::parse_peers_string("invalid").is_err());
        assert!(ServerConfigFile::parse_peers_string("node1=invalid").is_err());
        assert!(ServerConfigFile::parse_peers_string("node1=127.0.0.1:notaport").is_err());
    }
}

//! CLI argument parsing and configuration merging.
//!
//! This module handles command-line arguments, TOML configuration files,
//! and merging all configuration sources with proper precedence.

use crate::config;
use clap::Parser;

/// RaisinDB Server - Multi-tenant document database with real-time replication
#[derive(Parser, Debug)]
#[command(name = "raisin-server")]
#[command(version, about, long_about = None)]
pub struct ServerConfig {
    /// Path to TOML configuration file (optional - CLI args override config file)
    #[arg(short, long, env = "RAISIN_CONFIG")]
    pub config: Option<String>,

    /// HTTP server port
    #[arg(short, long, env = "RAISIN_PORT")]
    pub port: Option<u16>,

    /// Data directory path for RocksDB storage
    #[arg(short, long, env = "RAISIN_DATA_DIR")]
    pub data_dir: Option<String>,

    /// Initial admin password (optional - auto-generated if not provided)
    #[arg(long, env = "RAISIN_ADMIN_PASSWORD")]
    pub initial_admin_password: Option<String>,

    /// Cluster node ID for replication (optional - required for multi-node setups)
    #[arg(long, env = "RAISIN_CLUSTER_NODE_ID")]
    pub cluster_node_id: Option<String>,

    /// Replication TCP port (optional - required for multi-node setups)
    #[arg(long, env = "RAISIN_REPLICATION_PORT")]
    pub replication_port: Option<u16>,

    /// Replication peers in format: "node2=127.0.0.1:9002,node3=127.0.0.1:9003"
    #[arg(long, env = "RAISIN_REPLICATION_PEERS")]
    pub replication_peers: Option<String>,

    /// Bind address for HTTP server
    #[arg(long, env = "RAISIN_BIND_ADDRESS")]
    pub bind_address: Option<String>,

    /// Enable monitoring and metrics collection
    #[arg(long, env = "RAISIN_MONITORING_ENABLED")]
    pub monitoring_enabled: Option<bool>,

    /// Metrics collection interval in seconds
    #[arg(long, env = "RAISIN_MONITORING_INTERVAL_SECS", default_value = "30")]
    pub monitoring_interval_secs: Option<u64>,

    /// Port for metrics endpoint (optional - uses main HTTP port if not specified)
    #[arg(long, env = "RAISIN_MONITORING_PORT")]
    pub monitoring_port: Option<u16>,

    /// Enable PostgreSQL wire protocol server
    #[arg(long, env = "RAISIN_PGWIRE_ENABLED")]
    pub pgwire_enabled: Option<bool>,

    /// Bind address for pgwire server
    #[arg(long, env = "RAISIN_PGWIRE_BIND_ADDRESS")]
    pub pgwire_bind_address: Option<String>,

    /// Port for pgwire server
    #[arg(long, env = "RAISIN_PGWIRE_PORT")]
    pub pgwire_port: Option<u16>,

    /// Maximum concurrent connections for pgwire
    #[arg(long, env = "RAISIN_PGWIRE_MAX_CONNECTIONS")]
    pub pgwire_max_connections: Option<usize>,

    /// Enable development mode (allows insecure defaults for secrets).
    /// NEVER use in production.
    #[arg(long, env = "RAISIN_DEV_MODE")]
    pub dev_mode: bool,
}

/// Merged configuration from all sources
pub struct MergedConfig {
    pub port: u16,
    pub data_dir: String,
    pub bind_address: String,
    pub initial_admin_password: Option<String>,
    pub cluster_node_id: Option<String>,
    pub replication_enabled: bool,
    pub replication_port: Option<u16>,
    pub replication_peers: Vec<config::ReplicationPeer>,
    pub monitoring_enabled: bool,
    pub monitoring_interval_secs: u64,
    pub monitoring_port: Option<u16>,
    pub pgwire_enabled: bool,
    pub pgwire_bind_address: String,
    pub pgwire_port: u16,
    pub pgwire_max_connections: usize,
    /// Enable anonymous access for unauthenticated requests
    pub anonymous_enabled: bool,
    /// CORS allowed origins for cross-origin requests
    pub cors_allowed_origins: Vec<String>,
    /// Development mode — allows insecure defaults for secrets.
    pub dev_mode: bool,
}

impl ServerConfig {
    /// Merge configuration from CLI args, TOML file, and defaults
    /// Priority: CLI args > TOML file > defaults
    pub fn merge(&self) -> Result<MergedConfig, String> {
        // Load TOML config if provided
        let toml_config = if let Some(ref config_path) = self.config {
            Some(config::ServerConfigFile::from_file(config_path)?)
        } else {
            None
        };

        // Merge with priority: CLI > TOML > defaults
        let port = self
            .port
            .or_else(|| toml_config.as_ref().map(|c| c.server.port))
            .unwrap_or(8080);

        let data_dir = self
            .data_dir
            .clone()
            .or_else(|| toml_config.as_ref().map(|c| c.server.data_dir.clone()))
            .unwrap_or_else(|| "./.data/rocksdb".to_string());

        let bind_address = self
            .bind_address
            .clone()
            .or_else(|| toml_config.as_ref().map(|c| c.server.bind_address.clone()))
            .unwrap_or_else(|| "127.0.0.1".to_string());

        let initial_admin_password = self.initial_admin_password.clone().or_else(|| {
            toml_config
                .as_ref()
                .and_then(|c| c.server.initial_admin_password.clone())
        });

        let cluster_node_id = self.cluster_node_id.clone().or_else(|| {
            toml_config
                .as_ref()
                .and_then(|c| c.replication.node_id.clone())
        });

        let replication_enabled = toml_config
            .as_ref()
            .map(|c| c.replication.enabled)
            .unwrap_or(false);

        let replication_port = self
            .replication_port
            .or_else(|| toml_config.as_ref().and_then(|c| c.replication.port));

        // Parse replication peers from CLI arg or TOML
        let mut replication_peers = Vec::new();

        // First, add peers from TOML file
        if let Some(ref toml) = toml_config {
            replication_peers.extend(toml.replication.peers.clone());
        }

        // Then, add/override with peers from CLI arg
        if let Some(ref peers_str) = self.replication_peers {
            let cli_peers = config::ServerConfigFile::parse_peers_string(peers_str)?;
            // Override any peers with same peer_id, add new ones
            for cli_peer in cli_peers {
                if let Some(existing) = replication_peers
                    .iter_mut()
                    .find(|p| p.peer_id == cli_peer.peer_id)
                {
                    *existing = cli_peer;
                } else {
                    replication_peers.push(cli_peer);
                }
            }
        }

        // Merge monitoring configuration
        let monitoring_enabled = self
            .monitoring_enabled
            .or_else(|| toml_config.as_ref().map(|c| c.monitoring.enabled))
            .unwrap_or(false);

        let monitoring_interval_secs = self
            .monitoring_interval_secs
            .or_else(|| toml_config.as_ref().map(|c| c.monitoring.interval_secs))
            .unwrap_or(30);

        let monitoring_port = self
            .monitoring_port
            .or_else(|| toml_config.as_ref().and_then(|c| c.monitoring.port));

        // Merge pgwire configuration
        let pgwire_enabled = self
            .pgwire_enabled
            .or_else(|| toml_config.as_ref().map(|c| c.pgwire.enabled))
            .unwrap_or(false);

        let pgwire_bind_address = self
            .pgwire_bind_address
            .clone()
            .or_else(|| toml_config.as_ref().map(|c| c.pgwire.bind_address.clone()))
            .unwrap_or_else(|| "127.0.0.1".to_string());

        let pgwire_port = self
            .pgwire_port
            .or_else(|| toml_config.as_ref().map(|c| c.pgwire.port))
            .unwrap_or(5432);

        let pgwire_max_connections = self
            .pgwire_max_connections
            .or_else(|| toml_config.as_ref().map(|c| c.pgwire.max_connections))
            .unwrap_or(100);

        // Anonymous access: config file > env var > default (false)
        let anonymous_enabled = toml_config
            .as_ref()
            .map(|c| c.server.anonymous_enabled)
            .or_else(|| {
                std::env::var("WS_ANONYMOUS_ENABLED")
                    .ok()
                    .or_else(|| std::env::var("HTTP_ANONYMOUS_ENABLED").ok())
                    .and_then(|v| v.parse::<bool>().ok())
            })
            .unwrap_or(false);

        // CORS allowed origins from config file (no CLI override for now)
        let cors_allowed_origins = toml_config
            .as_ref()
            .map(|c| c.server.cors_allowed_origins.clone())
            .unwrap_or_default();

        Ok(MergedConfig {
            port,
            data_dir,
            bind_address,
            initial_admin_password,
            cluster_node_id,
            replication_enabled,
            replication_port,
            replication_peers,
            monitoring_enabled,
            monitoring_interval_secs,
            monitoring_port,
            pgwire_enabled,
            pgwire_bind_address,
            pgwire_port,
            pgwire_max_connections,
            anonymous_enabled,
            cors_allowed_origins,
            dev_mode: self.dev_mode,
        })
    }
}

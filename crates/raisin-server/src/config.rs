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
    /// Atomic locks / inventory subsystem configuration
    #[serde(default)]
    pub locks: raisin_locks::LocksConfig,
    /// How built-in NodeTypes / Workspaces / packages are sourced and rolled
    /// out. Defaults reproduce the historical behaviour exactly: embedded
    /// definitions only, no registries, non-breaking changes applied on start.
    #[serde(default)]
    pub system_definitions: raisin_core::definitions::SystemDefinitionsConfig,
    /// Outbound MCP client: where RaisinDB may connect when calling another
    /// server's tools. Defaults refuse loopback and private addresses, so an
    /// absent section cannot turn the server into a proxy into its own network.
    #[serde(default)]
    pub mcp_client: raisin_mcp::client::McpClientConfig,
    /// Trigger circuit breaker configuration — guards against a single
    /// tenant's runaway trigger/function loop growing the job registry
    /// without bound. Defined locally (rather than reusing
    /// `raisin_rocksdb::TriggerSafetyConfig` directly) because that
    /// crate is optional (`storage-rocksdb` feature) while this config type
    /// is always compiled.
    #[serde(default)]
    pub trigger_safety: TriggerSafetyConfig,
    /// Physical backstop behind `trigger_safety`: max non-terminal jobs one
    /// tenant may have registered at once, across all job types. `None`
    /// (the default here) keeps the storage backend's own built-in default
    /// (5000 for the RocksDB backend); set explicitly to override or to
    /// disable the check with an unbounded value.
    #[serde(default)]
    pub max_active_jobs_per_tenant: Option<usize>,
    /// Storage engine memory bounds (TOML `[storage]`). Absent means the
    /// backend's production preset.
    #[serde(default)]
    pub storage: StorageConfig,
    /// Secret handling (TOML `[secrets]`). Absent means auto-vaulting is ON,
    /// which is the only supported steady state.
    #[serde(default)]
    pub secrets: SecretsConfig,
    /// Operator-configured platform hooks (TOML `[platform.hooks.<name>]`),
    /// reachable from functions as `raisin.platform.hook(name, payload)`.
    /// Absent means no hooks: the binding refuses every name.
    #[serde(default)]
    pub platform: PlatformConfig,
}

/// `[platform]` section: named endpoints a function may fire by name.
///
/// This is how content code reaches platform services on loopback / private
/// addresses WITHOUT `raisin.http.fetch` ever being allowed there: the operator
/// names the URL and token here, the function only names the hook, and the
/// runtime stamps the tenant id into the payload.
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct PlatformConfig {
    #[serde(default)]
    pub hooks: std::collections::HashMap<String, PlatformHookConfig>,
}

/// One `[platform.hooks.<name>]` entry. Mirrors `raisin_functions::PlatformHook`
/// field for field; defined here because that crate is feature-gated while
/// this config type is always compiled.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct PlatformHookConfig {
    pub url: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub token_env: Option<String>,
    #[serde(default = "default_platform_token_header")]
    pub token_header: String,
    #[serde(default = "default_platform_hook_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_platform_token_header() -> String {
    "x-internal-token".to_string()
}

fn default_platform_hook_timeout_ms() -> u64 {
    120_000
}

/// Secret handling (TOML `[secrets]` section).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SecretsConfig {
    /// Whether a property declared `encrypted: true` is moved into the secret
    /// store on write.
    ///
    /// **Defaults to `true`, and should stay there.** Setting it to `false`
    /// makes those properties store PLAINTEXT — in the node blob and in the
    /// property index, where it is then queryable. It exists only as an
    /// operational escape hatch: vaulting sits on the core write path and fails
    /// closed, so a defect refuses writes rather than degrading them, and an
    /// operator needs an option that is not an emergency rollback.
    ///
    /// Disabling is loud by design: once at startup, and again for every node
    /// whose secret fields are skipped. Re-enabling does NOT retroactively seal
    /// anything written while it was off.
    #[serde(default = "default_true")]
    pub vaulting_enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for SecretsConfig {
    fn default() -> Self {
        Self {
            vaulting_enabled: true,
        }
    }
}

/// RocksDB memory bounds exposed to operators (TOML `[storage]` section).
///
/// These are the two settings that actually cap the engine's memory, so a box
/// under pressure can be tuned without a rebuild. Both are `None` by default,
/// meaning "keep `RocksDBConfig::production()`".
///
/// Declared here rather than reusing `raisin_rocksdb`'s type because that crate
/// is behind the optional `storage-rocksdb` feature while this config is always
/// compiled — same reasoning as [`TriggerSafetyConfig`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Block cache size in bytes. ONE cache is shared by every column family
    /// and also holds index and filter blocks, so this is the ceiling on
    /// RocksDB read-side memory. Default: 512MB.
    #[serde(default)]
    pub block_cache_size: Option<usize>,
    /// Global memtable budget in bytes across ALL column families. The per-CF
    /// write buffer is charged ~49 times over, so this — not the per-CF size —
    /// is what bounds write-side memory. `Some(0)` disables the cap.
    /// Default: 512MB.
    #[serde(default)]
    pub db_write_buffer_size: Option<usize>,
}

/// Trigger circuit breaker configuration (TOML `[trigger_safety]` section).
/// Mirrors `raisin_rocksdb::TriggerSafetyConfig` field-for-field; kept
/// in sync manually since the two crates can't share the type across the
/// optional `storage-rocksdb` feature boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerSafetyConfig {
    /// Master switch. When false, a trigger can fire without limit.
    #[serde(default = "default_trigger_safety_enabled")]
    pub enabled: bool,
    /// Max fires of one (tenant, repo, trigger, node) within `window_secs`.
    #[serde(default = "default_trigger_rate_limit")]
    pub rate_limit_per_window: usize,
    /// Absolute ceiling on `rate_limit_per_window` — a config value above
    /// this is clamped down, never up.
    #[serde(default = "default_trigger_rate_hard_ceiling")]
    pub rate_limit_hard_ceiling: usize,
    /// Max total trigger fires (any trigger) for one (tenant, repo, node)
    /// within `window_secs`.
    #[serde(default = "default_trigger_node_fire_budget")]
    pub node_fire_budget: usize,
    /// Sliding window, in seconds, for both checks.
    #[serde(default = "default_trigger_window_secs")]
    pub window_secs: u64,
}

fn default_trigger_safety_enabled() -> bool {
    true
}
fn default_trigger_rate_limit() -> usize {
    500
}
fn default_trigger_rate_hard_ceiling() -> usize {
    1000
}
fn default_trigger_node_fire_budget() -> usize {
    1000
}
fn default_trigger_window_secs() -> u64 {
    1
}

impl Default for TriggerSafetyConfig {
    fn default() -> Self {
        Self {
            enabled: default_trigger_safety_enabled(),
            rate_limit_per_window: default_trigger_rate_limit(),
            rate_limit_hard_ceiling: default_trigger_rate_hard_ceiling(),
            node_fire_budget: default_trigger_node_fire_budget(),
            window_secs: default_trigger_window_secs(),
        }
    }
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

    /// The shipped example config must parse as `ServerConfigFile`.
    ///
    /// Operators copy that file. A section documented there that the struct
    /// cannot deserialize is a config error on someone else's server, at boot.
    #[test]
    fn shipped_example_config_parses() {
        let example = include_str!("../../../examples/cluster/node1.toml");
        let config: ServerConfigFile =
            toml::from_str(example).expect("examples/cluster/node1.toml must parse");

        // The documented [mcp_client] values must land where the code reads them.
        assert!(!config.mcp_client.allow_private_addresses);
        assert_eq!(config.mcp_client.max_response_bytes, 8_388_608);
        assert_eq!(config.mcp_client.default_timeout_ms, 30_000);
    }

    /// A hook section parses with its defaults, and an absent one is empty.
    #[test]
    fn platform_hooks_parse() {
        let config: ServerConfigFile = toml::from_str(
            "[platform.hooks.studio_update]\nurl = \"http://127.0.0.1:8080/internal/studio/update\"\ntoken_env = \"STUDIO_INTERNAL_TOKEN\"\ntoken_header = \"x-studio-internal-token\"\n",
        )
        .expect("hook section must parse");
        let hook = &config.platform.hooks["studio_update"];
        assert_eq!(hook.token_header, "x-studio-internal-token");
        assert_eq!(hook.timeout_ms, 120_000);
        let none: ServerConfigFile = toml::from_str("[server]\nport = 8080\n").unwrap();
        assert!(none.platform.hooks.is_empty());
    }

    /// An absent [mcp_client] section must not enable private egress.
    #[test]
    fn omitted_mcp_client_section_is_safe() {
        let config: ServerConfigFile =
            toml::from_str("[server]\nport = 8080\n").expect("minimal config must parse");
        assert!(!config.mcp_client.allow_private_addresses);
        assert!(config.mcp_client.allowed_hosts.is_empty());
    }
}

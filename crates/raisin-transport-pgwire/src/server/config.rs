// SPDX-License-Identifier: BSL-1.1

//! Configuration types for the pgwire server.

/// Configuration for the PostgreSQL wire protocol server
#[derive(Debug, Clone)]
pub struct PgWireConfig {
    /// Bind address for the TCP listener (e.g., "0.0.0.0:5432" or "127.0.0.1:5432")
    pub bind_addr: String,

    /// Maximum number of concurrent connections allowed
    ///
    /// This is a soft limit - the server will continue accepting connections
    /// but may reject them if this limit is exceeded. Set to 0 for unlimited.
    pub max_connections: usize,
}

impl PgWireConfig {
    /// Create a new configuration builder
    pub fn builder() -> PgWireConfigBuilder {
        PgWireConfigBuilder::default()
    }

    /// Create a new configuration with default values
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for PgWireConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:5432".to_string(),
            max_connections: 100,
        }
    }
}

/// Builder for `PgWireConfig`
#[derive(Debug, Default)]
pub struct PgWireConfigBuilder {
    bind_addr: Option<String>,
    max_connections: Option<usize>,
}

impl PgWireConfigBuilder {
    /// Set the bind address for the TCP listener
    pub fn bind_addr(mut self, addr: impl Into<String>) -> Self {
        self.bind_addr = Some(addr.into());
        self
    }

    /// Set the maximum number of concurrent connections
    pub fn max_connections(mut self, max: usize) -> Self {
        self.max_connections = Some(max);
        self
    }

    /// Build the final configuration
    pub fn build(self) -> PgWireConfig {
        let defaults = PgWireConfig::default();
        PgWireConfig {
            bind_addr: self.bind_addr.unwrap_or(defaults.bind_addr),
            max_connections: self.max_connections.unwrap_or(defaults.max_connections),
        }
    }
}

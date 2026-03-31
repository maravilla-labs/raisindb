// SPDX-License-Identifier: BSL-1.1

//! Server implementation for PostgreSQL wire protocol transport.
//!
//! This module provides the main server component that accepts TCP connections
//! and processes them using the PostgreSQL wire protocol via the `pgwire` crate.

mod config;
mod dummy_handlers;
mod listener;

// Re-export public API
pub use config::{PgWireConfig, PgWireConfigBuilder};
pub use listener::PgWireServer;

#[cfg(test)]
mod tests {
    use super::*;
    use dummy_handlers::DummyHandler;

    #[test]
    fn test_config_default() {
        let config = PgWireConfig::default();
        assert_eq!(config.bind_addr, "127.0.0.1:5432");
        assert_eq!(config.max_connections, 100);
    }

    #[test]
    fn test_config_builder() {
        let config = PgWireConfig::builder()
            .bind_addr("0.0.0.0:5433")
            .max_connections(200)
            .build();

        assert_eq!(config.bind_addr, "0.0.0.0:5433");
        assert_eq!(config.max_connections, 200);
    }

    #[test]
    fn test_config_builder_partial() {
        let config = PgWireConfig::builder().bind_addr("0.0.0.0:5433").build();

        assert_eq!(config.bind_addr, "0.0.0.0:5433");
        assert_eq!(config.max_connections, 100);
    }

    #[test]
    fn test_server_creation() {
        let config = PgWireConfig::default();
        let _server: PgWireServer<DummyHandler> = PgWireServer::new(config);
    }

    #[test]
    fn test_server_with_handler() {
        let config = PgWireConfig::default();
        let server = PgWireServer::new(config).with_handler(DummyHandler);
        assert!(server.handler.is_some());
    }
}

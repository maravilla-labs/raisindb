// SPDX-License-Identifier: BSL-1.1

//! Simple query protocol handler for PostgreSQL wire protocol.
//!
//! This module implements the simple query protocol (text-based queries) for pgwire.
//! Simple queries are the most basic form of PostgreSQL queries, where SQL is sent
//! as plain text and results are returned in text format.

mod execution;
mod handler;
mod session_commands;
mod system_queries;
mod trait_impl;

// Re-export public API
pub use handler::RaisinSimpleQueryHandler;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_statements_single() {
        let query = "SELECT * FROM nodes";
        let statements = RaisinSimpleQueryHandler::<
            raisin_rocksdb::RocksDBStorage,
            crate::auth::MockApiKeyValidator,
            pgwire::api::auth::DefaultServerParameterProvider,
        >::split_statements(query);
        assert_eq!(statements.len(), 1);
        assert_eq!(statements[0], "SELECT * FROM nodes");
    }

    #[test]
    fn test_split_statements_multiple() {
        let query = "SELECT * FROM nodes; UPDATE nodes SET name = 'test'; DELETE FROM nodes WHERE id = '123'";
        let statements = RaisinSimpleQueryHandler::<
            raisin_rocksdb::RocksDBStorage,
            crate::auth::MockApiKeyValidator,
            pgwire::api::auth::DefaultServerParameterProvider,
        >::split_statements(query);
        assert_eq!(statements.len(), 3);
        assert_eq!(statements[0], "SELECT * FROM nodes");
        assert_eq!(statements[1], "UPDATE nodes SET name = 'test'");
        assert_eq!(statements[2], "DELETE FROM nodes WHERE id = '123'");
    }

    #[test]
    fn test_split_statements_trailing_semicolon() {
        let query = "SELECT * FROM nodes;";
        let statements = RaisinSimpleQueryHandler::<
            raisin_rocksdb::RocksDBStorage,
            crate::auth::MockApiKeyValidator,
            pgwire::api::auth::DefaultServerParameterProvider,
        >::split_statements(query);
        assert_eq!(statements.len(), 1);
        assert_eq!(statements[0], "SELECT * FROM nodes");
    }

    #[test]
    fn test_split_statements_empty() {
        let query = "";
        let statements = RaisinSimpleQueryHandler::<
            raisin_rocksdb::RocksDBStorage,
            crate::auth::MockApiKeyValidator,
            pgwire::api::auth::DefaultServerParameterProvider,
        >::split_statements(query);
        assert_eq!(statements.len(), 0);
    }

    #[test]
    fn test_split_statements_whitespace() {
        let query = "  ;  ;  ";
        let statements = RaisinSimpleQueryHandler::<
            raisin_rocksdb::RocksDBStorage,
            crate::auth::MockApiKeyValidator,
            pgwire::api::auth::DefaultServerParameterProvider,
        >::split_statements(query);
        assert_eq!(statements.len(), 0);
    }
}

// SPDX-License-Identifier: BSL-1.1

//! Simple query handler struct definition and construction.

use crate::auth::{ApiKeyValidator, RaisinAuthHandler};
use raisin_storage::Storage;
use std::sync::Arc;

/// Simple query handler for RaisinDB PostgreSQL wire protocol.
///
/// This handler processes text-based SQL queries using the simple query protocol.
/// It integrates with RaisinDB's QueryEngine to execute queries and returns results
/// in PostgreSQL wire format.
pub struct RaisinSimpleQueryHandler<S, V, P>
where
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    V: ApiKeyValidator,
    P: pgwire::api::auth::ServerParameterProvider,
{
    /// Storage backend for data access
    pub(super) storage: Arc<S>,

    /// Authentication handler to retrieve connection context
    pub(super) auth_handler: Arc<RaisinAuthHandler<V, P>>,

    /// Optional Tantivy indexing engine for full-text search
    #[cfg(feature = "indexing")]
    pub(super) indexing_engine: Option<Arc<raisin_indexer::TantivyIndexingEngine>>,

    /// Optional HNSW engine for vector similarity search
    #[cfg(feature = "indexing")]
    pub(super) hnsw_engine: Option<Arc<raisin_hnsw::HnswIndexingEngine>>,

    /// Shared schema stats cache for data-driven selectivity estimation
    pub(super) schema_stats_cache: Option<raisin_core::SharedSchemaStatsCache>,
}

impl<S, V, P> RaisinSimpleQueryHandler<S, V, P>
where
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    V: ApiKeyValidator,
    P: pgwire::api::auth::ServerParameterProvider,
{
    /// Create a new simple query handler
    pub fn new(storage: Arc<S>, auth_handler: Arc<RaisinAuthHandler<V, P>>) -> Self {
        Self {
            storage,
            auth_handler,
            #[cfg(feature = "indexing")]
            indexing_engine: None,
            #[cfg(feature = "indexing")]
            hnsw_engine: None,
            schema_stats_cache: None,
        }
    }

    /// Set the Tantivy indexing engine for full-text search support
    #[cfg(feature = "indexing")]
    pub fn with_indexing_engine(
        mut self,
        engine: Arc<raisin_indexer::TantivyIndexingEngine>,
    ) -> Self {
        self.indexing_engine = Some(engine);
        self
    }

    /// Set the HNSW engine for vector similarity search support
    #[cfg(feature = "indexing")]
    pub fn with_hnsw_engine(mut self, engine: Arc<raisin_hnsw::HnswIndexingEngine>) -> Self {
        self.hnsw_engine = Some(engine);
        self
    }

    /// Set the schema stats cache for data-driven selectivity estimation
    pub fn with_schema_stats_cache(mut self, cache: raisin_core::SharedSchemaStatsCache) -> Self {
        self.schema_stats_cache = Some(cache);
        self
    }

    /// Split a query string into individual statements by semicolons
    pub(super) fn split_statements(query: &str) -> Vec<String> {
        query
            .split(';')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }
}

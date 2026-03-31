// SPDX-License-Identifier: BSL-1.1

//! Index plugin trait

use crate::IndexQuery;
use raisin_events::EventHandler;

/// Plugin interface for implementing custom indexes
///
/// Index plugins handle node events to maintain indexes and respond to queries.
/// Plugins can be registered with the IndexManager to provide efficient lookups
/// for specific query patterns.
pub trait IndexPlugin: EventHandler + Send + Sync {
    /// Unique name for this index plugin
    fn index_name(&self) -> &str;

    /// Query the index
    ///
    /// Returns a list of node IDs that match the query.
    /// Returns an empty vec if the query type is not supported by this plugin.
    fn query(
        &self,
        query: IndexQuery,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<Vec<String>>> + Send + '_>>;

    /// Check if this plugin supports a specific query type
    fn supports_query(&self, query: &IndexQuery) -> bool;

    /// Optional: Rebuild the index from scratch
    ///
    /// This can be used to recover from corruption or to initially populate
    /// the index.
    fn rebuild(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    /// Optional: Clear all index data
    fn clear(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    /// Optional: Get index statistics (for monitoring/debugging)
    fn stats(&self) -> std::collections::HashMap<String, serde_json::Value> {
        std::collections::HashMap::new()
    }
}

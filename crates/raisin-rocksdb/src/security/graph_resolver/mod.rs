//! Graph relationship resolver for RocksDB storage.
//!
//! Implements BFS-based path finding for RELATES expressions in permission conditions.
//! Uses the global relation index for cross-workspace support.
//!
//! Optionally uses precomputed RELATES cache from the graph module for faster lookups.

mod bfs;
mod cache;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_rel::eval::{RelDirection, RelationResolver};
use raisin_rel::EvalError;
use raisin_storage::RelationRepository;
use rocksdb::DB;

use crate::graph::{GraphCacheLayer, GraphCacheValue};

/// RocksDB implementation of RelationResolver.
///
/// Uses BFS (Breadth-First Search) for efficient path finding with early termination.
/// Leverages the global relation index for cross-workspace relationship traversal.
///
/// When a cache layer is provided, attempts to use precomputed RELATES cache
/// for faster lookups before falling back to BFS.
pub struct RocksDBGraphResolver<'a, R: RelationRepository> {
    pub(super) relation_repo: &'a R,
    pub(super) tenant_id: &'a str,
    pub(super) repo_id: &'a str,
    pub(super) branch: &'a str,
    pub(super) revision: &'a HLC,
    /// Optional RocksDB handle for cache lookups
    pub(super) db: Option<Arc<DB>>,
    /// Optional in-memory cache layer
    pub(super) cache_layer: Option<Arc<GraphCacheLayer>>,
}

impl<'a, R: RelationRepository> RocksDBGraphResolver<'a, R> {
    /// Create a new graph resolver.
    ///
    /// # Arguments
    /// * `relation_repo` - Repository for querying relationships
    /// * `tenant_id` - Tenant ID for scoping
    /// * `repo_id` - Repository ID for scoping
    /// * `branch` - Branch name for scoping
    /// * `revision` - Maximum revision to consider (for time-travel queries)
    pub fn new(
        relation_repo: &'a R,
        tenant_id: &'a str,
        repo_id: &'a str,
        branch: &'a str,
        revision: &'a HLC,
    ) -> Self {
        Self {
            relation_repo,
            tenant_id,
            repo_id,
            branch,
            revision,
            db: None,
            cache_layer: None,
        }
    }

    /// Create a graph resolver with cache support.
    ///
    /// When a cache is configured, the resolver will first check for precomputed
    /// RELATES cache entries before falling back to BFS.
    ///
    /// # Arguments
    /// * `relation_repo` - Repository for querying relationships
    /// * `tenant_id` - Tenant ID for scoping
    /// * `repo_id` - Repository ID for scoping
    /// * `branch` - Branch name for scoping
    /// * `revision` - Maximum revision to consider
    /// * `db` - RocksDB handle for reading from GRAPH_CACHE column family
    /// * `cache_layer` - In-memory cache layer for hot lookups
    pub fn with_cache(
        relation_repo: &'a R,
        tenant_id: &'a str,
        repo_id: &'a str,
        branch: &'a str,
        revision: &'a HLC,
        db: Arc<DB>,
        cache_layer: Arc<GraphCacheLayer>,
    ) -> Self {
        Self {
            relation_repo,
            tenant_id,
            repo_id,
            branch,
            revision,
            db: Some(db),
            cache_layer: Some(cache_layer),
        }
    }
}

#[async_trait::async_trait]
impl<R: RelationRepository + Sync> RelationResolver for RocksDBGraphResolver<'_, R> {
    async fn has_path(
        &self,
        source_id: &str,
        target_id: &str,
        relation_types: &[String],
        min_depth: u32,
        max_depth: u32,
        direction: RelDirection,
    ) -> Result<bool, EvalError> {
        // Check cache first (only for ANY direction, as RELATES cache stores bidirectional reachability)
        if direction == RelDirection::Any {
            if let Some(cached_result) =
                self.check_cache(source_id, target_id, relation_types, max_depth)
            {
                tracing::debug!(
                    "RELATES cache hit: {} -> {} = {} (types: {:?}, max_depth: {})",
                    source_id,
                    target_id,
                    cached_result,
                    relation_types,
                    max_depth
                );
                return Ok(cached_result);
            }
        }

        // Cache miss or directional query - fall back to BFS
        self.bfs_has_path(
            source_id,
            target_id,
            relation_types,
            min_depth,
            max_depth,
            direction,
        )
        .await
        .map_err(|e| EvalError::graph_error(format!("{}", e)))
    }
}

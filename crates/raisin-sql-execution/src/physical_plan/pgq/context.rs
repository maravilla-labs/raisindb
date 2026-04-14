//! PGQ Execution Context
//!
//! Contains all context needed for graph query execution,
//! including a per-query cache for graph algorithm results.

use raisin_hlc::HLC;
use std::collections::HashMap;
use std::sync::Mutex;

use crate::physical_plan::pgq::types::SqlValue;

/// Cache key for graph algorithm results
type AlgorithmCacheKey = String; // e.g. "wcc", "pagerank", "cdlp:10", "bfs:alice"

/// Per-node result from a cached algorithm computation
type NodeResultMap = HashMap<(String, String), SqlValue>; // (workspace, node_id) → value

/// Execution context for PGQ queries
///
/// Contains tenant, repository, branch, and workspace information
/// needed to execute graph queries against the storage layer.
///
/// Also contains a per-query cache for graph algorithm results.
/// This ensures algorithms like WCC, PageRank, etc. are computed once
/// per query and reused for all rows, giving consistent results and
/// better performance.
pub struct PgqContext {
    /// Workspace identifier (for scoping queries)
    pub workspace_id: String,
    /// Tenant identifier
    pub tenant_id: String,
    /// Repository identifier
    pub repo_id: String,
    /// Branch name
    pub branch: String,
    /// Optional revision for point-in-time queries
    pub revision: Option<HLC>,
    /// Per-query cache for graph algorithm results.
    /// Key: algorithm name + params. Value: map of (workspace, node_id) → SqlValue.
    algorithm_cache: Mutex<HashMap<AlgorithmCacheKey, NodeResultMap>>,
}

impl std::fmt::Debug for PgqContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgqContext")
            .field("workspace_id", &self.workspace_id)
            .field("tenant_id", &self.tenant_id)
            .field("repo_id", &self.repo_id)
            .field("branch", &self.branch)
            .field("revision", &self.revision)
            .finish()
    }
}

impl Clone for PgqContext {
    fn clone(&self) -> Self {
        Self {
            workspace_id: self.workspace_id.clone(),
            tenant_id: self.tenant_id.clone(),
            repo_id: self.repo_id.clone(),
            branch: self.branch.clone(),
            revision: self.revision.clone(),
            algorithm_cache: Mutex::new(HashMap::new()), // fresh cache for clone
        }
    }
}

impl PgqContext {
    /// Create a new PGQ execution context
    pub fn new(
        workspace_id: String,
        tenant_id: String,
        repo_id: String,
        branch: String,
        revision: Option<HLC>,
    ) -> Self {
        Self {
            workspace_id,
            tenant_id,
            repo_id,
            branch,
            revision,
            algorithm_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Get a cached algorithm result for a specific node.
    /// Returns `Some(SqlValue)` if the algorithm was already computed for this node.
    pub fn get_cached_result(
        &self,
        cache_key: &str,
        workspace: &str,
        node_id: &str,
    ) -> Option<SqlValue> {
        let cache = self.algorithm_cache.lock().ok()?;
        cache
            .get(cache_key)
            .and_then(|results| results.get(&(workspace.to_string(), node_id.to_string())))
            .cloned()
    }

    /// Store computed algorithm results for all nodes.
    /// Subsequent calls to `get_cached_result` will return from this cache.
    pub fn set_cached_results(&self, cache_key: &str, results: NodeResultMap) {
        if let Ok(mut cache) = self.algorithm_cache.lock() {
            cache.insert(cache_key.to_string(), results);
        }
    }

    /// Check if an algorithm result is already cached.
    pub fn has_cached(&self, cache_key: &str) -> bool {
        self.algorithm_cache
            .lock()
            .map(|cache| cache.contains_key(cache_key))
            .unwrap_or(false)
    }
}

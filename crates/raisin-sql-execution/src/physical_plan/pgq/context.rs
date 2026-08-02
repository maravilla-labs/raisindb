//! PGQ Execution Context
//!
//! Contains all context needed for graph query execution, plus two per-query
//! memos: one for graph algorithm results, one for the adjacency list itself.

use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_sql::ast::GraphTableQuery;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use crate::physical_plan::graph_algo::GraphAdjacency;
use crate::physical_plan::pgq::types::SqlValue;

/// Cache key for graph algorithm results
type AlgorithmCacheKey = String; // e.g. "wcc", "pagerank", "cdlp:10", "bfs:alice"

/// Per-node result from a cached algorithm computation
type NodeResultMap = HashMap<(String, String), SqlValue>; // (workspace, node_id) → value

/// Edge weight map: `(src_workspace, src_id, tgt_workspace, tgt_id) -> weight`.
///
/// Kept for the weighted `sssp()` surface, which predates edge weights living
/// on [`crate::physical_plan::graph_algo::GraphEdge`]. New code should read
/// `GraphEdge::weight` directly — this map cannot represent two relation types
/// between the same pair of nodes.
pub type EdgeWeightMap = HashMap<(String, String, String, String), f64>;

/// An adjacency list plus its weight map, memoised for one relation-type scope.
pub type ScopedAdjacency = Arc<(GraphAdjacency, EdgeWeightMap)>;

/// Execution context for PGQ queries
///
/// Contains tenant, repository, branch, and workspace information
/// needed to execute graph queries against the storage layer.
///
/// # Memos, not caches
///
/// Both memos live and die with a single `GRAPH_TABLE` execution. There is no
/// invalidation and no TTL because there is nothing to invalidate: the context
/// is created per query and dropped with it. `Clone` deliberately starts an
/// empty pair rather than sharing them.
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
    /// Caller identity for row-level security, or `None` for a system/internal
    /// caller (no filtering at all — same convention as the scan executors).
    ///
    /// Only the nodes a query RETURNS are checked; see [`super::rls`].
    pub auth_context: Option<AuthContext>,
    /// Relation types this query can possibly traverse; empty means "any".
    ///
    /// Pushed down into `scan_relations_global` so an algorithm invocation
    /// does not load every relation in the branch across all workspaces.
    relation_type_scope: Vec<String>,
    /// Per-query cache for graph algorithm results.
    /// Key: algorithm name + params. Value: map of (workspace, node_id) → SqlValue.
    algorithm_cache: Mutex<HashMap<AlgorithmCacheKey, NodeResultMap>>,
    /// Per-query memo of built adjacency lists, keyed by relation-type scope.
    adjacency_memo: Mutex<HashMap<String, ScopedAdjacency>>,
}

impl std::fmt::Debug for PgqContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgqContext")
            .field("workspace_id", &self.workspace_id)
            .field("tenant_id", &self.tenant_id)
            .field("repo_id", &self.repo_id)
            .field("branch", &self.branch)
            .field("revision", &self.revision)
            .field("has_auth_context", &self.auth_context.is_some())
            .field("relation_type_scope", &self.relation_type_scope)
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
            revision: self.revision,
            auth_context: self.auth_context.clone(),
            relation_type_scope: self.relation_type_scope.clone(),
            algorithm_cache: Mutex::new(HashMap::new()), // fresh memos for clone
            adjacency_memo: Mutex::new(HashMap::new()),
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
            auth_context: None,
            relation_type_scope: Vec::new(),
            algorithm_cache: Mutex::new(HashMap::new()),
            adjacency_memo: Mutex::new(HashMap::new()),
        }
    }

    /// Attach the caller's identity so projected endpoints are RLS-filtered.
    ///
    /// Leaving it unset means "system caller": no filtering. Every SQL surface
    /// that has an [`AuthContext`] must pass it here — GRAPH_TABLE is the only
    /// graph query language reachable from SQL, so this is the one place RLS
    /// can be enforced for graph queries.
    pub fn with_auth_context(mut self, auth: Option<AuthContext>) -> Self {
        self.auth_context = auth;
        self
    }

    /// Restrict which relation types this query's adjacency scans load.
    ///
    /// An empty scope means "every type", which is the safe default: a scope
    /// that is too narrow silently drops edges, so it is only ever derived
    /// from a pattern where EVERY relationship names its types.
    pub fn with_relation_type_scope(mut self, types: Vec<String>) -> Self {
        let unique: BTreeSet<String> = types.into_iter().collect();
        self.relation_type_scope = unique.into_iter().collect();
        self
    }

    /// Relation types this query may traverse; empty means "any".
    pub fn relation_type_scope(&self) -> &[String] {
        &self.relation_type_scope
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

    /// Memo key for a relation-type scope; order-insensitive.
    pub fn adjacency_key(relation_types: &[String]) -> String {
        let unique: BTreeSet<&str> = relation_types.iter().map(String::as_str).collect();
        unique.into_iter().collect::<Vec<_>>().join(",")
    }

    /// Look up a previously built adjacency for this relation-type scope.
    pub fn cached_adjacency(&self, key: &str) -> Option<ScopedAdjacency> {
        self.adjacency_memo.lock().ok()?.get(key).cloned()
    }

    /// Memoise a built adjacency for this relation-type scope.
    pub fn store_adjacency(&self, key: &str, adjacency: ScopedAdjacency) {
        if let Ok(mut memo) = self.adjacency_memo.lock() {
            memo.insert(key.to_string(), adjacency);
        }
    }
}

/// Collect the relation types a `GRAPH_TABLE` query can possibly traverse.
///
/// Returns an EMPTY vector — meaning "no restriction" — as soon as any
/// relationship in any pattern leaves its types open (`-[r]->`). Narrowing on
/// a partially-typed pattern would drop edges the untyped hop is entitled to
/// match, which is the silent-wrong-results shape this pushdown must not
/// introduce while trying to make scans cheaper.
pub fn relation_type_scope(query: &GraphTableQuery) -> Vec<String> {
    let mut types: Vec<String> = Vec::new();

    for pattern in &query.match_clause.patterns {
        for rel in pattern.relationships() {
            if rel.types.is_empty() {
                return Vec::new();
            }
            types.extend(rel.types.iter().cloned());
        }
    }

    types
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacency_key_is_order_insensitive_and_deduped() {
        assert_eq!(
            PgqContext::adjacency_key(&["b".into(), "a".into(), "b".into()]),
            PgqContext::adjacency_key(&["a".into(), "b".into()])
        );
    }

    #[test]
    fn an_empty_scope_has_its_own_key() {
        assert_eq!(PgqContext::adjacency_key(&[]), "");
    }

    #[test]
    fn relation_type_scope_is_sorted_and_deduped() {
        let ctx = PgqContext::new("ws".into(), "t".into(), "r".into(), "main".into(), None)
            .with_relation_type_scope(vec!["knows".into(), "follows".into(), "knows".into()]);
        assert_eq!(ctx.relation_type_scope(), ["follows", "knows"]);
    }

    #[test]
    fn memo_round_trips() {
        let ctx = PgqContext::new("ws".into(), "t".into(), "r".into(), "main".into(), None);
        assert!(ctx.cached_adjacency("knows").is_none());
        ctx.store_adjacency("knows", Arc::new((HashMap::new(), HashMap::new())));
        assert!(ctx.cached_adjacency("knows").is_some());
    }
}

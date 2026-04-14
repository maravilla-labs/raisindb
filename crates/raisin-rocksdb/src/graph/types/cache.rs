//! Graph cache value types and key builder
//!
//! Types for storing precomputed graph algorithm results in the GRAPH_CACHE column family.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use super::config::TargetMode;

/// Cached value for a single node from a graph algorithm
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphCacheValue {
    /// The computed value
    pub value: CachedValue,
    /// Timestamp when this was computed (Unix millis)
    pub computed_at: u64,
    /// TTL expiry timestamp (Unix millis, 0 = never expires for revision mode)
    pub expires_at: u64,
    /// The source revision used for computation
    pub source_revision: String,
    /// Config version/revision for invalidation
    pub config_revision: String,
}

/// The actual cached value - different types per algorithm
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CachedValue {
    /// Float value (PageRank, Betweenness Centrality)
    Float(f64),
    /// Integer value (Louvain community ID, Connected Components ID, Triangle Count)
    Integer(u64),
    /// Set of reachable node IDs (RelatesCache)
    ReachabilitySet(HashSet<String>),
}

impl CachedValue {
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<u64> {
        match self {
            Self::Integer(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_reachability_set(&self) -> Option<&HashSet<String>> {
        match self {
            Self::ReachabilitySet(set) => Some(set),
            _ => None,
        }
    }
}

impl GraphCacheValue {
    /// Check if this cached value has expired
    pub fn is_expired(&self) -> bool {
        if self.expires_at == 0 {
            // Never expires (revision mode)
            return false;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        now > self.expires_at
    }
}

/// Metadata for tracking computation state per config/branch/revision
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphCacheMeta {
    /// Target mode (branch or revision)
    pub target_mode: TargetMode,
    /// Branch ID (for branch mode)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_id: Option<String>,
    /// Revision ID (for revision mode, or computed revision for branch mode)
    pub revision_id: String,
    /// Timestamp of last computation (Unix millis)
    pub last_computed_at: u64,
    /// Next scheduled computation timestamp (Unix millis, 0 for revision mode)
    pub next_scheduled_at: u64,
    /// Number of nodes in the computed result
    pub node_count: u64,
    /// Current status
    pub status: CacheStatus,
    /// Error message if status is Error
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Status of a graph cache computation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CacheStatus {
    /// Cache is ready and valid
    Ready,
    /// Computation is currently in progress
    Computing,
    /// Cache is stale and needs recomputation
    Stale,
    /// Initial state, never computed
    Pending,
    /// Computation failed with error
    Error,
}

/// Key builder for GRAPH_CACHE column family
pub struct GraphCacheKey;

impl GraphCacheKey {
    /// Build a cache key for branch mode
    /// Format: <repo_id>:branch:<branch_id>:<config_id>:<node_id>
    pub fn branch_node(repo_id: &str, branch_id: &str, config_id: &str, node_id: &str) -> String {
        format!("{}:branch:{}:{}:{}", repo_id, branch_id, config_id, node_id)
    }

    /// Build a cache key for revision mode
    /// Format: <repo_id>:rev:<revision_id>:<config_id>:<node_id>
    pub fn revision_node(
        repo_id: &str,
        revision_id: &str,
        config_id: &str,
        node_id: &str,
    ) -> String {
        format!("{}:rev:{}:{}:{}", repo_id, revision_id, config_id, node_id)
    }

    /// Build a metadata key for branch mode
    /// Format: <repo_id>:branch:<branch_id>:<config_id>:_meta
    pub fn branch_meta(repo_id: &str, branch_id: &str, config_id: &str) -> String {
        format!("{}:branch:{}:{}:_meta", repo_id, branch_id, config_id)
    }

    /// Build a metadata key for revision mode
    /// Format: <repo_id>:rev:<revision_id>:<config_id>:_meta
    pub fn revision_meta(repo_id: &str, revision_id: &str, config_id: &str) -> String {
        format!("{}:rev:{}:{}:_meta", repo_id, revision_id, config_id)
    }

    /// Build a prefix for scanning all nodes of a config (branch mode)
    pub fn branch_config_prefix(repo_id: &str, branch_id: &str, config_id: &str) -> String {
        format!("{}:branch:{}:{}:", repo_id, branch_id, config_id)
    }

    /// Build a prefix for scanning all nodes of a config (revision mode)
    pub fn revision_config_prefix(repo_id: &str, revision_id: &str, config_id: &str) -> String {
        format!("{}:rev:{}:{}:", repo_id, revision_id, config_id)
    }

    /// Build a prefix for scanning all configs of a repo/branch
    pub fn branch_prefix(repo_id: &str, branch_id: &str) -> String {
        format!("{}:branch:{}:", repo_id, branch_id)
    }

    /// Build a prefix for scanning all data of a repo
    pub fn repo_prefix(repo_id: &str) -> String {
        format!("{}:", repo_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key_format() {
        assert_eq!(
            GraphCacheKey::branch_node("repo1", "main", "pagerank-social", "user123"),
            "repo1:branch:main:pagerank-social:user123"
        );
        assert_eq!(
            GraphCacheKey::revision_node("repo1", "abc123", "pagerank-historical", "user456"),
            "repo1:rev:abc123:pagerank-historical:user456"
        );
        assert_eq!(
            GraphCacheKey::branch_meta("repo1", "main", "pagerank-social"),
            "repo1:branch:main:pagerank-social:_meta"
        );
    }

    #[test]
    fn test_cached_value_types() {
        let float_val = CachedValue::Float(0.85);
        assert_eq!(float_val.as_float(), Some(0.85));
        assert_eq!(float_val.as_integer(), None);

        let int_val = CachedValue::Integer(42);
        assert_eq!(int_val.as_integer(), Some(42));
        assert_eq!(int_val.as_float(), None);

        let mut set = HashSet::new();
        set.insert("user1".to_string());
        set.insert("user2".to_string());
        let set_val = CachedValue::ReachabilitySet(set.clone());
        assert_eq!(set_val.as_reachability_set(), Some(&set));
    }
}

//! Helper functions for graph cache management.

use raisin_rocksdb::{
    graph::{
        CacheStatus, GraphAlgorithm, GraphAlgorithmConfig, GraphCacheMeta, GraphComputeTask,
        TargetMode,
    },
    RocksDBStorage,
};

use super::types::{
    GraphAlgorithmConfigResponse, RefreshConfigResponse, ScopeConfigResponse, TargetConfigResponse,
};

/// Load all graph algorithm configs from the repository
pub(super) async fn load_all_configs(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Vec<GraphAlgorithmConfig>, raisin_error::Error> {
    use raisin_storage::{ListOptions, NodeRepository, Storage, StorageScope};

    const WORKSPACE: &str = "raisin:access_control";
    const CONFIG_PATH_PREFIX: &str = "/graph-config/";
    const CONFIG_NODE_TYPE: &str = "raisin:GraphAlgorithmConfig";

    let scope = StorageScope::new(tenant_id, repo_id, "main", WORKSPACE);
    let nodes = storage
        .nodes()
        .list_by_type(
            scope,
            CONFIG_NODE_TYPE,
            ListOptions {
                max_revision: None,
                compute_has_children: false,
            },
        )
        .await?;

    let mut configs = Vec::new();
    for node in nodes {
        if !node.path.starts_with(CONFIG_PATH_PREFIX) {
            continue;
        }

        match GraphAlgorithmConfig::from_node(&node) {
            Ok(config) => configs.push(config),
            Err(e) => {
                tracing::warn!(
                    "Failed to parse graph algorithm config from node '{}': {}",
                    node.path,
                    e
                );
            }
        }
    }

    Ok(configs)
}

/// Read cache metadata from GRAPH_CACHE column family
pub(super) fn read_cache_meta(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch_id: &str,
    config_id: &str,
) -> Result<Option<GraphCacheMeta>, raisin_error::Error> {
    GraphComputeTask::get_cache_meta(storage, tenant_id, repo_id, branch_id, config_id)
}

/// Convert CacheStatus to string
pub(super) fn status_to_string(status: &CacheStatus) -> String {
    match status {
        CacheStatus::Ready => "ready".to_string(),
        CacheStatus::Computing => "computing".to_string(),
        CacheStatus::Stale => "stale".to_string(),
        CacheStatus::Pending => "pending".to_string(),
        CacheStatus::Error => "error".to_string(),
    }
}

/// Convert GraphAlgorithmConfig to response format
pub(super) fn config_to_response(config: &GraphAlgorithmConfig) -> GraphAlgorithmConfigResponse {
    let target_mode = match config.target.mode {
        TargetMode::Branch => "branch",
        TargetMode::AllBranches => "all_branches",
        TargetMode::Revision => "revision",
        TargetMode::BranchPattern => "branch_pattern",
    };

    let algorithm_config = match config.algorithm {
        GraphAlgorithm::PageRank => {
            serde_json::json!({
                "damping_factor": config.config.get("damping_factor").and_then(|v| v.as_f64()).unwrap_or(0.85),
                "max_iterations": config.config.get("max_iterations").and_then(|v| v.as_u64()).unwrap_or(100),
                "convergence_threshold": config.config.get("convergence_threshold").and_then(|v| v.as_f64()).unwrap_or(0.0001),
            })
        }
        GraphAlgorithm::Louvain => {
            serde_json::json!({
                "resolution": config.config.get("resolution").and_then(|v| v.as_f64()).unwrap_or(1.0),
                "max_iterations": config.config.get("max_iterations").and_then(|v| v.as_u64()).unwrap_or(100),
            })
        }
        GraphAlgorithm::RelatesCache => {
            serde_json::json!({
                "max_depth": config.config.get("max_depth").and_then(|v| v.as_u64()).unwrap_or(2),
            })
        }
        _ => serde_json::json!({}),
    };

    GraphAlgorithmConfigResponse {
        id: config.id.clone(),
        algorithm: config.algorithm.to_string(),
        enabled: config.enabled,
        target: TargetConfigResponse {
            mode: target_mode.to_string(),
            branches: config.target.branches.clone(),
            revisions: config.target.revisions.clone(),
            branch_pattern: config.target.branch_pattern.clone(),
        },
        scope: ScopeConfigResponse {
            paths: config.scope.paths.clone(),
            node_types: config.scope.node_types.clone(),
            workspaces: config.scope.workspaces.clone(),
            relation_types: config.scope.relation_types.clone(),
        },
        algorithm_config,
        refresh: RefreshConfigResponse {
            ttl_seconds: config.refresh.ttl_seconds,
            on_branch_change: config.refresh.on_branch_change,
            on_relation_change: config.refresh.on_relation_change,
            cron: config.refresh.cron.clone(),
        },
    }
}

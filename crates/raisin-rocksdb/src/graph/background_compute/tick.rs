//! Tick execution logic for background graph computation.
//!
//! Contains the periodic tick that iterates over tenants/repos/branches,
//! checks for stale caches, and triggers recomputation.

use super::{GraphComputeConfig, GraphComputeStats, GraphComputeTask, TickStats};
use crate::graph::cache_layer::GraphCacheLayer;
use crate::graph::config::GraphAlgorithmConfig;
use crate::management::{list_branches, list_repositories, list_tenants};
use crate::RocksDBStorage;
use raisin_error::Result;
use std::sync::Arc;
use tokio::sync::Mutex;

impl GraphComputeTask {
    /// Run a single tick - check all configs and recompute stale caches
    pub(super) async fn run_tick(
        storage: &RocksDBStorage,
        cache_layer: &GraphCacheLayer,
        config: &GraphComputeConfig,
        stats: &Arc<Mutex<GraphComputeStats>>,
    ) -> Result<()> {
        // Get all tenants
        let tenants = list_tenants(storage).await?;

        let mut configs_processed = 0;

        for tenant_id in tenants {
            if configs_processed >= config.max_configs_per_tick {
                break;
            }

            // Get all repositories for this tenant
            let repos = list_repositories(storage, &tenant_id).await?;

            for repo_id in repos {
                if configs_processed >= config.max_configs_per_tick {
                    break;
                }

                // Get all branches for this repository
                let branches = list_branches(storage, &tenant_id, &repo_id).await?;

                for branch_id in branches {
                    if configs_processed >= config.max_configs_per_tick {
                        break;
                    }

                    // Load graph algorithm configs for this branch
                    let algo_configs =
                        Self::load_configs_for_branch(storage, &tenant_id, &repo_id, &branch_id)
                            .await?;

                    for algo_config in algo_configs {
                        if configs_processed >= config.max_configs_per_tick {
                            break;
                        }

                        // Check if this config needs recomputation
                        if Self::needs_recomputation(
                            storage,
                            &tenant_id,
                            &repo_id,
                            &branch_id,
                            &algo_config,
                        )
                        .await?
                        {
                            // Recompute
                            match Self::recompute_for_branch(
                                storage,
                                cache_layer,
                                &tenant_id,
                                &repo_id,
                                &branch_id,
                                &algo_config,
                                config.max_nodes_per_execution,
                            )
                            .await
                            {
                                Ok(node_count) => {
                                    let mut s = stats.lock().await;
                                    s.configs_processed += 1;
                                    s.nodes_computed += node_count as u64;
                                    s.last_computation = Some(std::time::SystemTime::now());

                                    tracing::info!(
                                        tenant_id = %tenant_id,
                                        repo_id = %repo_id,
                                        branch_id = %branch_id,
                                        config_id = %algo_config.id,
                                        algorithm = ?algo_config.algorithm,
                                        nodes = node_count,
                                        "Graph algorithm recomputed"
                                    );
                                }
                                Err(e) => {
                                    tracing::error!(
                                        tenant_id = %tenant_id,
                                        repo_id = %repo_id,
                                        branch_id = %branch_id,
                                        config_id = %algo_config.id,
                                        error = %e,
                                        "Graph algorithm recomputation failed"
                                    );
                                    let mut s = stats.lock().await;
                                    s.errors += 1;
                                }
                            }

                            configs_processed += 1;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Run a single tick without internal stats tracking
    ///
    /// This is a static version that can be called from BackgroundJobs
    /// and returns tick statistics directly.
    pub async fn run_tick_static(
        storage: &RocksDBStorage,
        cache_layer: &GraphCacheLayer,
        config: &GraphComputeConfig,
    ) -> Result<TickStats> {
        let mut tick_stats = TickStats::default();

        // Get all tenants
        let tenants = list_tenants(storage).await?;

        let mut configs_processed = 0;

        for tenant_id in tenants {
            if configs_processed >= config.max_configs_per_tick {
                break;
            }

            // Get all repositories for this tenant
            let repos = list_repositories(storage, &tenant_id).await?;

            for repo_id in repos {
                if configs_processed >= config.max_configs_per_tick {
                    break;
                }

                // Get all branches for this repository
                let branches = list_branches(storage, &tenant_id, &repo_id).await?;

                for branch_id in branches {
                    if configs_processed >= config.max_configs_per_tick {
                        break;
                    }

                    // Load graph algorithm configs for this branch
                    let algo_configs =
                        Self::load_configs_for_branch(storage, &tenant_id, &repo_id, &branch_id)
                            .await?;

                    for algo_config in algo_configs {
                        if configs_processed >= config.max_configs_per_tick {
                            break;
                        }

                        // Check if this config needs recomputation
                        if Self::needs_recomputation(
                            storage,
                            &tenant_id,
                            &repo_id,
                            &branch_id,
                            &algo_config,
                        )
                        .await?
                        {
                            // Recompute
                            match Self::recompute_for_branch(
                                storage,
                                cache_layer,
                                &tenant_id,
                                &repo_id,
                                &branch_id,
                                &algo_config,
                                config.max_nodes_per_execution,
                            )
                            .await
                            {
                                Ok(node_count) => {
                                    tick_stats.configs_processed += 1;
                                    tick_stats.nodes_computed += node_count as u64;

                                    tracing::info!(
                                        tenant_id = %tenant_id,
                                        repo_id = %repo_id,
                                        branch_id = %branch_id,
                                        config_id = %algo_config.id,
                                        algorithm = ?algo_config.algorithm,
                                        nodes = node_count,
                                        "Graph algorithm recomputed"
                                    );
                                }
                                Err(e) => {
                                    tick_stats.errors += 1;
                                    tracing::error!(
                                        tenant_id = %tenant_id,
                                        repo_id = %repo_id,
                                        branch_id = %branch_id,
                                        config_id = %algo_config.id,
                                        error = %e,
                                        "Graph algorithm recomputation failed"
                                    );
                                }
                            }

                            configs_processed += 1;
                        }
                    }
                }
            }
        }

        Ok(tick_stats)
    }

    /// Load graph algorithm configs that apply to a specific branch
    pub(super) async fn load_configs_for_branch(
        storage: &RocksDBStorage,
        tenant_id: &str,
        repo_id: &str,
        branch_id: &str,
    ) -> Result<Vec<GraphAlgorithmConfig>> {
        // Read configs from /raisin:access_control/graph-config/ folder
        // Filter to those that target this branch
        let configs = Self::load_all_configs(storage, tenant_id, repo_id).await?;

        Ok(configs
            .into_iter()
            .filter(|c| c.enabled && c.targets_branch(branch_id))
            .collect())
    }

    /// Load all graph algorithm configs from the repository
    async fn load_all_configs(
        storage: &RocksDBStorage,
        tenant_id: &str,
        repo_id: &str,
    ) -> Result<Vec<GraphAlgorithmConfig>> {
        use raisin_storage::{ListOptions, NodeRepository, Storage, StorageScope};

        // The configs are stored as nodes of type raisin:GraphAlgorithmConfig
        // under the path /raisin:access_control/graph-config/
        const WORKSPACE: &str = "raisin:access_control";
        const CONFIG_PATH_PREFIX: &str = "/graph-config/";
        const CONFIG_NODE_TYPE: &str = "raisin:GraphAlgorithmConfig";

        // List all nodes of type raisin:GraphAlgorithmConfig in the access_control workspace
        // We use the main branch as the source of truth for configuration
        let scope = StorageScope::new(tenant_id, repo_id, "main", WORKSPACE);
        let nodes = storage
            .nodes()
            .list_by_type(
                scope,
                CONFIG_NODE_TYPE,
                ListOptions {
                    max_revision: None, // Get latest
                    compute_has_children: false,
                },
            )
            .await?;

        // Filter by path prefix and parse into GraphAlgorithmConfig
        let mut configs = Vec::new();
        for node in nodes {
            // Only process nodes under the /graph-config/ path
            if !node.path.starts_with(CONFIG_PATH_PREFIX) {
                continue;
            }

            // Parse the node into a config
            match GraphAlgorithmConfig::from_node(&node) {
                Ok(config) => configs.push(config),
                Err(e) => {
                    tracing::warn!(
                        tenant_id = %tenant_id,
                        repo_id = %repo_id,
                        node_id = %node.id,
                        node_path = %node.path,
                        error = %e,
                        "Failed to parse graph algorithm config from node, skipping"
                    );
                }
            }
        }

        tracing::debug!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            config_count = configs.len(),
            "Loaded graph algorithm configs"
        );

        Ok(configs)
    }
}

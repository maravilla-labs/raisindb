//! Background task spawning and execution.
//!
//! Contains the spawning methods for each background job type and their
//! execution logic (integrity checks, compaction, backups, graph computation, self-healing).

use super::*;

impl BackgroundJobs {
    /// Spawn integrity check job
    pub(super) fn spawn_integrity_check_job(&self) -> JoinHandle<()> {
        let storage = self.storage.clone();
        let interval = self.config.integrity_check_interval;
        let running = self.running.clone();
        let stats = self.stats.clone();
        let self_heal_enabled = self.config.self_heal_enabled;
        let self_heal_threshold = self.config.self_heal_threshold;

        tokio::spawn(async move {
            while running.load(Ordering::SeqCst) {
                tokio::time::sleep(interval).await;

                if !running.load(Ordering::SeqCst) {
                    break;
                }

                {
                    let mut s = stats.lock().await;
                    s.integrity_checks_run += 1;
                    s.last_integrity_check = Some(std::time::SystemTime::now());
                }

                match Self::run_integrity_checks(
                    &storage,
                    stats.clone(),
                    self_heal_enabled,
                    self_heal_threshold,
                )
                .await
                {
                    Ok(()) => {}
                    Err(e) => {
                        let mut s = stats.lock().await;
                        s.integrity_checks_failed += 1;
                        eprintln!("Background integrity check failed: {}", e);
                    }
                }
            }
        })
    }

    /// Spawn compaction job
    pub(super) fn spawn_compaction_job(&self) -> JoinHandle<()> {
        let storage = self.storage.clone();
        let interval = self.config.compaction_interval;
        let retention = self.config.compaction_retention.clone();
        let running = self.running.clone();
        let stats = self.stats.clone();

        tokio::spawn(async move {
            while running.load(Ordering::SeqCst) {
                tokio::time::sleep(interval).await;

                if !running.load(Ordering::SeqCst) {
                    break;
                }

                {
                    let mut s = stats.lock().await;
                    s.compactions_run += 1;
                    s.last_compaction = Some(std::time::SystemTime::now());
                }

                match Self::run_compactions(&storage, retention.clone(), stats.clone()).await {
                    Ok(()) => {}
                    Err(e) => {
                        let mut s = stats.lock().await;
                        s.compactions_failed += 1;
                        eprintln!("Background compaction failed: {}", e);
                    }
                }
            }
        })
    }

    /// Spawn backup job
    pub(super) fn spawn_backup_job(&self) -> JoinHandle<()> {
        let storage = self.storage.clone();
        let interval = self.config.backup_interval;
        let destination = self.config.backup_destination.clone().unwrap_or_default();
        let running = self.running.clone();
        let stats = self.stats.clone();

        tokio::spawn(async move {
            while running.load(Ordering::SeqCst) {
                tokio::time::sleep(interval).await;

                if !running.load(Ordering::SeqCst) {
                    break;
                }

                {
                    let mut s = stats.lock().await;
                    s.backups_run += 1;
                    s.last_backup = Some(std::time::SystemTime::now());
                }

                match Self::run_backups(&storage, &destination, stats.clone()).await {
                    Ok(()) => {}
                    Err(e) => {
                        let mut s = stats.lock().await;
                        s.backups_failed += 1;
                        eprintln!("Background backup failed: {}", e);
                    }
                }
            }
        })
    }

    /// Spawn graph compute job
    pub(super) fn spawn_graph_compute_job(&self) -> JoinHandle<()> {
        let storage = self.storage.clone();
        let cache_layer = self.graph_cache_layer.clone();
        let interval = self.config.graph_compute_interval;
        let max_configs = self.config.graph_compute_max_configs_per_tick;
        let running = self.running.clone();
        let stats = self.stats.clone();

        let compute_config = crate::graph::background_compute::GraphComputeConfig {
            enabled: true,
            check_interval: interval,
            max_configs_per_tick: max_configs,
            max_nodes_per_execution: 100_000,
        };

        tokio::spawn(async move {
            while running.load(Ordering::SeqCst) {
                tokio::time::sleep(interval).await;

                if !running.load(Ordering::SeqCst) {
                    break;
                }

                {
                    let mut s = stats.lock().await;
                    s.graph_compute_ticks += 1;
                }

                match crate::graph::background_compute::GraphComputeTask::run_tick_static(
                    &storage,
                    &cache_layer,
                    &compute_config,
                )
                .await
                {
                    Ok(tick_stats) => {
                        let mut s = stats.lock().await;
                        s.graph_compute_configs_processed += tick_stats.configs_processed;
                        s.graph_compute_nodes_computed += tick_stats.nodes_computed;
                        s.last_graph_compute = Some(std::time::SystemTime::now());
                    }
                    Err(e) => {
                        let mut s = stats.lock().await;
                        s.graph_compute_errors += 1;
                        tracing::error!("Background graph compute failed: {}", e);
                    }
                }
            }

            tracing::info!("Graph compute background task stopped");
        })
    }

    /// Run integrity checks for all repositories
    async fn run_integrity_checks(
        storage: &RocksDBStorage,
        stats: Arc<Mutex<BackgroundJobStats>>,
        self_heal_enabled: bool,
        self_heal_threshold: f64,
    ) -> Result<()> {
        let tenants = list_tenants(storage).await?;

        for tenant_id in tenants {
            let repos = list_repositories(storage, &tenant_id).await?;

            for repo_id in repos {
                match integrity::check_repository(storage, &tenant_id, &repo_id).await {
                    Ok(report) => {
                        if self_heal_enabled && report.health_score < (self_heal_threshold as f32) {
                            if let Err(e) = Self::self_heal_repository(
                                storage,
                                &tenant_id,
                                &repo_id,
                                &report,
                                stats.clone(),
                            )
                            .await
                            {
                                eprintln!(
                                    "Self-healing failed for {}/{}: {}",
                                    tenant_id, repo_id, e
                                );
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "Integrity check failed for {}/{}: {}",
                            tenant_id, repo_id, e
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Run compaction for all repositories
    async fn run_compactions(
        storage: &RocksDBStorage,
        retention: compaction::RevisionRetentionPolicy,
        _stats: Arc<Mutex<BackgroundJobStats>>,
    ) -> Result<()> {
        let tenants = list_tenants(storage).await?;

        for tenant_id in tenants {
            let repos = list_repositories(storage, &tenant_id).await?;

            for repo_id in repos {
                match compaction::compact_repository(
                    storage,
                    &tenant_id,
                    &repo_id,
                    retention.clone(),
                )
                .await
                {
                    Ok(compact_stats) => {
                        let bytes_freed = compact_stats
                            .bytes_before
                            .saturating_sub(compact_stats.bytes_after);
                        if bytes_freed > 0 {
                            println!(
                                "Compacted {}/{}: freed {} bytes",
                                tenant_id, repo_id, bytes_freed
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!("Compaction failed for {}/{}: {}", tenant_id, repo_id, e);
                    }
                }
            }
        }

        Ok(())
    }

    /// Run backups for all repositories
    async fn run_backups(
        storage: &RocksDBStorage,
        destination: &std::path::Path,
        _stats: Arc<Mutex<BackgroundJobStats>>,
    ) -> Result<()> {
        let tenants = list_tenants(storage).await?;

        for tenant_id in tenants {
            let repos = list_repositories(storage, &tenant_id).await?;

            for repo_id in repos {
                let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
                let backup_dest = destination
                    .join(&tenant_id)
                    .join(&repo_id)
                    .join(timestamp.to_string());

                match backup::backup_repository(storage, &tenant_id, &repo_id, &backup_dest).await {
                    Ok(info) => {
                        println!(
                            "Backed up {}/{} to {:?}: {} nodes, {} bytes",
                            tenant_id, repo_id, backup_dest, info.node_count, info.size_bytes
                        );
                    }
                    Err(e) => {
                        eprintln!("Backup failed for {}/{}: {}", tenant_id, repo_id, e);
                    }
                }
            }
        }

        Ok(())
    }

    /// Self-heal a repository based on integrity report
    async fn self_heal_repository(
        storage: &RocksDBStorage,
        tenant_id: &str,
        repo_id: &str,
        report: &raisin_storage::IntegrityReport,
        stats: Arc<Mutex<BackgroundJobStats>>,
    ) -> Result<()> {
        let mut stats_guard = stats.lock().await;
        stats_guard.self_heals_triggered += 1;
        drop(stats_guard);

        let mut healed = false;

        let has_index_issues = !report.issues_found.is_empty();

        if has_index_issues {
            let workspaces = list_workspaces(storage, tenant_id, repo_id).await?;
            let branches = list_branches(storage, tenant_id, repo_id).await?;

            for workspace in &workspaces {
                for branch in &branches {
                    match async_indexing::rebuild_indexes(
                        storage,
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        raisin_storage::IndexType::All,
                    )
                    .await
                    {
                        Ok(_) => healed = true,
                        Err(e) => eprintln!(
                            "Failed to rebuild path index for {}/{}/{}/{}: {}",
                            tenant_id, repo_id, branch, workspace, e
                        ),
                    }
                }
            }

            for workspace in &workspaces {
                for branch in &branches {
                    match async_indexing::rebuild_indexes(
                        storage,
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        raisin_storage::IndexType::Property,
                    )
                    .await
                    {
                        Ok(_) => healed = true,
                        Err(e) => eprintln!(
                            "Failed to rebuild property index for {}/{}/{}/{}: {}",
                            tenant_id, repo_id, branch, workspace, e
                        ),
                    }
                }
            }

            for workspace in &workspaces {
                for branch in &branches {
                    match async_indexing::rebuild_indexes(
                        storage,
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        raisin_storage::IndexType::Reference,
                    )
                    .await
                    {
                        Ok(_) => healed = true,
                        Err(e) => eprintln!(
                            "Failed to rebuild reference index for {}/{}/{}/{}: {}",
                            tenant_id, repo_id, branch, workspace, e
                        ),
                    }
                }
            }
        }

        if healed {
            let mut stats_guard = stats.lock().await;
            stats_guard.self_heals_successful += 1;
        }

        Ok(())
    }
}

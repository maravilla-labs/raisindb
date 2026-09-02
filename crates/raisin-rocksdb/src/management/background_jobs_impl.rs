//! BackgroundJobs trait implementation for RocksDBStorage.
//!
//! Provides job lifecycle management including starting, scheduling,
//! listing, cancelling, and purging background jobs.

use super::compaction;
use super::{BackgroundJobsConfig, BackgroundJobsImpl};
use crate::RocksDBStorage;
use async_trait::async_trait;
use raisin_error::Result;
use raisin_storage::{
    BackgroundJobs, BackgroundJobsInternal, CategoryQueueDepthStats, JobHandle, JobId,
    JobQueueStats, JobSystemHealth, PersistedStats, QueueDepthStats, TenantJobHealth, WorkerStats,
};
use std::sync::Arc;
use std::time::Duration;

/// Most recent persisted jobs a single listing surface will serve. Anything
/// older is reachable only via retention-window history in storage; listing
/// surfaces must stay bounded no matter how large a runaway backlog grows.
const MAX_LISTED_JOBS: usize = 1_000;

/// Result payloads above this size are replaced by a truncation marker in
/// LIST responses (single-job lookups still serve the full payload).
const MAX_INLINE_RESULT_BYTES: usize = 64 * 1024;

impl RocksDBStorage {
    /// List a tenant's jobs by merging the indexed persisted history with the
    /// in-memory registry.
    ///
    /// Persisted entries survive restarts and registry eviction (24h
    /// retention); registry entries are authoritative for still-running jobs
    /// and carry the freshest status/progress. When both exist, the registry
    /// entry wins — except a trimmed (None) in-memory result never masks the
    /// persisted one. All execution-history surfaces (HTTP functions,
    /// scheduler, WS) should use this instead of the registry alone, so
    /// history does not silently end at the in-memory retention horizon.
    pub async fn list_tenant_jobs_merged(
        &self,
        tenant: &str,
    ) -> Result<Vec<raisin_storage::JobInfo>> {
        // Bounded: the write-maintained newest-first index reaches only this
        // page's records and each index entry causes one metadata point read.
        // Do not replace this with a tenant-prefix scan: that was the source
        // of the production page-open RSS/CPU spike. Legacy records appear
        // after the explicitly triggered cursor backfill has indexed them.
        let (persisted, _) =
            self.job_metadata_store()
                .list_history_page(tenant, MAX_LISTED_JOBS, None)?;

        let mut merged: std::collections::HashMap<JobId, raisin_storage::JobInfo> = persisted
            .into_iter()
            .map(|(job_id, mut entry)| {
                if let Some(result) = &entry.result {
                    let size = serde_json::to_vec(result)
                        .map(|value| value.len())
                        .unwrap_or(0);
                    if size > MAX_INLINE_RESULT_BYTES {
                        entry.result = Some(serde_json::json!({
                            "$truncated": true,
                            "size": size,
                        }));
                    }
                }
                (
                    job_id.clone(),
                    raisin_storage::JobInfo {
                        id: job_id,
                        job_type: entry.job_type,
                        status: entry.status,
                        tenant: entry.tenant,
                        started_at: entry.started_at,
                        completed_at: entry.completed_at,
                        progress: entry.progress,
                        error: entry.error,
                        result: entry.result,
                        retry_count: entry.retry_count,
                        max_retries: entry.max_retries,
                        last_heartbeat: entry.last_heartbeat,
                        timeout_seconds: entry.timeout_seconds,
                        next_retry_at: entry.next_retry_at,
                        executing_since: entry.executing_since,
                    },
                )
            })
            .collect();

        let live = self.job_registry().list_jobs_by_tenant(tenant).await;
        for info in live {
            match merged.get_mut(&info.id) {
                Some(existing) => {
                    let persisted_result = existing.result.take();
                    *existing = info;
                    if existing.result.is_none() {
                        existing.result = persisted_result;
                    }
                }
                None => {
                    merged.insert(info.id.clone(), info);
                }
            }
        }

        // Registry overlay can re-grow the set; keep the final list bounded
        // and deterministic (newest first).
        let mut out: Vec<raisin_storage::JobInfo> = merged.into_values().collect();
        out.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        out.truncate(MAX_LISTED_JOBS);
        Ok(out)
    }
}

#[async_trait]
impl BackgroundJobs for RocksDBStorage {
    fn start_background_jobs(&self) -> Result<JobHandle> {
        // Note: This is a simplified implementation
        // In production, you would store the BackgroundJobsImpl instance
        // and return a proper handle that can be used to control/stop jobs

        let config = BackgroundJobsConfig {
            integrity_check_enabled: self.config().background_jobs_enabled,
            integrity_check_interval: Duration::from_secs(3600),
            compaction_enabled: self.config().background_jobs_enabled,
            compaction_interval: Duration::from_secs(21600),
            compaction_retention: compaction::RevisionRetentionPolicy::KeepLatest(100),
            backup_enabled: false, // Needs explicit configuration
            backup_interval: Duration::from_secs(86400),
            backup_destination: None,
            self_heal_enabled: self.config().auto_heal_enabled,
            self_heal_threshold: 0.75,
            max_concurrent_jobs: 2,
            graph_compute_enabled: self.config().background_jobs_enabled,
            graph_compute_interval: Duration::from_secs(60),
            graph_compute_max_configs_per_tick: 10,
        };

        // Share the storage-owned cache layer so the resolver (Storage::graph_resolver)
        // reads the same in-memory reachability this job populates.
        let graph_cache_layer = self.graph_cache_layer.clone();
        let bg_jobs = BackgroundJobsImpl::new(Arc::new(self.clone()), graph_cache_layer, config);

        // Start jobs in a tokio task and return the handle
        let handle = tokio::spawn(async move {
            if let Err(e) = bg_jobs.start().await {
                tracing::error!("Failed to start background jobs: {}", e);
            }
        });

        Ok(JobHandle::Running(handle))
    }

    fn schedule_integrity_scan(&self, tenant: &str, interval: Duration) -> Result<JobId> {
        // Create a custom background job for just integrity scanning
        let config = BackgroundJobsConfig {
            integrity_check_enabled: true,
            integrity_check_interval: interval,
            compaction_enabled: false,
            compaction_interval: Duration::from_secs(0),
            compaction_retention: compaction::RevisionRetentionPolicy::KeepLatest(100),
            backup_enabled: false,
            backup_interval: Duration::from_secs(0),
            backup_destination: None,
            self_heal_enabled: self.config().auto_heal_enabled,
            self_heal_threshold: 0.75,
            max_concurrent_jobs: 1,
            graph_compute_enabled: false, // Not needed for integrity scan
            graph_compute_interval: Duration::from_secs(60),
            graph_compute_max_configs_per_tick: 10,
        };

        // Share the storage-owned cache layer (see start_background_jobs).
        let graph_cache_layer = self.graph_cache_layer.clone();
        let bg_jobs = BackgroundJobsImpl::new(Arc::new(self.clone()), graph_cache_layer, config);

        let tenant_id = tenant.to_string();

        // Start just integrity checks
        tokio::spawn(async move {
            if let Err(e) = bg_jobs.start().await {
                tracing::error!("Failed to start integrity scan for {}: {}", tenant_id, e);
            }
        });

        let job_id = format!("integrity-scan-{}", tenant);
        Ok(JobId(job_id))
    }

    /// List all jobs for a tenant: persisted history merged with the live
    /// registry (so scheduled-but-unclaimed jobs are visible too).
    async fn list_jobs(&self, tenant: &str) -> Result<Vec<raisin_storage::JobInfo>> {
        self.list_tenant_jobs_merged(tenant).await
    }

    /// List jobs with their execution scope from the persisted job context,
    /// optionally filtered to a repository. Repository views are strict:
    /// unknown scopes never pass a repository filter.
    async fn list_jobs_with_scope(
        &self,
        tenant: &str,
        repo: Option<&str>,
    ) -> Result<Vec<raisin_storage::ScopedJobInfo>> {
        let jobs = self.list_tenant_jobs_merged(tenant).await?;
        let mut out = Vec::with_capacity(jobs.len());
        for info in jobs {
            let scope = self
                .job_data_store()
                .get(tenant, &info.id)
                .ok()
                .flatten()
                .map(|ctx| raisin_storage::JobScope {
                    repo: ctx.repo_id,
                    branch: ctx.branch,
                    workspace: ctx.workspace_id,
                });
            if let Some(want) = repo {
                if scope.as_ref().is_none_or(|scope| scope.repo != want) {
                    continue;
                }
            }
            out.push(raisin_storage::ScopedJobInfo { info, scope });
        }
        Ok(out)
    }

    /// Get status of a specific job within the given tenant.
    async fn get_job_status(
        &self,
        tenant: &str,
        job_id: &raisin_storage::JobId,
    ) -> Result<raisin_storage::JobStatus> {
        let info = self.get_job_info(tenant, job_id).await?;
        Ok(info.status)
    }

    /// Cancel a running job within the given tenant.
    async fn cancel_job(&self, tenant: &str, job_id: &raisin_storage::JobId) -> Result<()> {
        let _ = self.get_job_info(tenant, job_id).await?;
        self.job_registry().cancel_job(job_id).await
    }

    /// Delete a job from persistent storage and in-memory registry within the given tenant.
    async fn delete_job(&self, tenant: &str, job_id: &raisin_storage::JobId) -> Result<()> {
        // Ensure the job belongs to this tenant first.
        let _ = self.get_job_info(tenant, job_id).await?;

        // Delete from persistent storage (RocksDB) first
        self.job_metadata_store().delete(tenant, job_id)?;

        // Also try to delete from in-memory registry (ignore errors if not found there)
        let _ = self.job_registry().delete_job(job_id).await;

        Ok(())
    }

    /// Delete multiple jobs from persistent storage in a single batch within the given tenant.
    async fn delete_jobs_batch(
        &self,
        tenant: &str,
        job_ids: &[raisin_storage::JobId],
    ) -> (usize, usize) {
        // Pre-filter to tenant-owned jobs.
        let mut owned: Vec<raisin_storage::JobId> = Vec::with_capacity(job_ids.len());
        let mut skipped = 0usize;
        for id in job_ids {
            match self.get_job_info(tenant, id).await {
                Ok(_) => owned.push(id.clone()),
                Err(_) => skipped += 1,
            }
        }

        match self.job_metadata_store().delete_batch(tenant, &owned) {
            Ok((deleted, also_skipped)) => {
                let _ = self.job_registry().delete_jobs_batch(&owned).await;
                (deleted, skipped + also_skipped)
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to batch delete jobs from persistent storage");
                (0, job_ids.len())
            }
        }
    }

    /// Get full job information within the given tenant.
    async fn get_job_info(
        &self,
        tenant: &str,
        job_id: &raisin_storage::JobId,
    ) -> Result<raisin_storage::JobInfo> {
        // Prefer the in-memory registry, fall back to persistent storage.
        let info = match self.job_registry().get_job_info(job_id).await {
            Ok(mut info) => {
                // Terminal results are trimmed from the registry after a
                // grace window to bound memory; the persisted copy is
                // authoritative. Re-hydrate so callers still see the result.
                if info.result.is_none()
                    && matches!(
                        info.status,
                        raisin_storage::JobStatus::Completed
                            | raisin_storage::JobStatus::Failed(_)
                            | raisin_storage::JobStatus::Cancelled
                    )
                {
                    if let Ok(Some(entry)) = self.job_metadata_store().get(tenant, job_id) {
                        info.result = entry.result;
                    }
                }
                info
            }
            Err(_) => {
                let entry = self
                    .job_metadata_store()
                    .get(tenant, job_id)?
                    .ok_or_else(|| {
                        raisin_error::Error::NotFound(format!("Job {} not found", job_id))
                    })?;
                raisin_storage::JobInfo {
                    id: job_id.clone(),
                    job_type: entry.job_type,
                    status: entry.status,
                    tenant: entry.tenant,
                    started_at: entry.started_at,
                    completed_at: entry.completed_at,
                    progress: entry.progress,
                    error: entry.error,
                    result: entry.result,
                    retry_count: entry.retry_count,
                    max_retries: entry.max_retries,
                    last_heartbeat: entry.last_heartbeat,
                    timeout_seconds: entry.timeout_seconds,
                    next_retry_at: entry.next_retry_at,
                    executing_since: entry.executing_since,
                }
            }
        };
        if info.tenant != tenant {
            return Err(raisin_error::Error::NotFound(format!(
                "Job {} not found",
                job_id
            )));
        }
        Ok(info)
    }

    /// Purge all jobs from persistent storage for the given tenant.
    async fn purge_all_jobs(&self, tenant: &str) -> Result<usize> {
        self.job_metadata_store().purge_all_for_tenant(tenant)
    }

    /// Purge orphaned (undeserializable) entries for the given tenant.
    async fn purge_orphaned_jobs(&self, tenant: &str) -> Result<usize> {
        self.job_metadata_store().purge_orphaned_for_tenant(tenant)
    }

    /// Get job queue statistics for the given tenant.
    async fn get_job_queue_stats(&self, tenant: &str) -> Result<JobQueueStats> {
        let (queue, pool_size, categories) = if let Some(stats) = self.job_dispatcher_stats() {
            // Build per-category breakdown
            let mut cat_stats: Vec<CategoryQueueDepthStats> = stats
                .category_stats
                .iter()
                .map(|(cat, cs)| CategoryQueueDepthStats {
                    category: format!("{:?}", cat),
                    high_queue_len: cs.high_queue_len,
                    normal_queue_len: cs.normal_queue_len,
                    low_queue_len: cs.low_queue_len,
                    total_dispatched: cs.total_high_dispatched
                        + cs.total_normal_dispatched
                        + cs.total_low_dispatched,
                })
                .collect();
            // Sort for deterministic output: Realtime, Background, System
            cat_stats.sort_by(|a, b| a.category.cmp(&b.category));

            (
                QueueDepthStats {
                    high_queue_len: stats.high_queue_len,
                    high_queue_capacity: 10_000,
                    normal_queue_len: stats.normal_queue_len,
                    normal_queue_capacity: 50_000,
                    low_queue_len: stats.low_queue_len,
                    low_queue_capacity: 100_000,
                    total_high_dispatched: stats.total_high_dispatched,
                    total_normal_dispatched: stats.total_normal_dispatched,
                    total_low_dispatched: stats.total_low_dispatched,
                },
                self.config().worker_pool_size,
                cat_stats,
            )
        } else {
            (
                QueueDepthStats {
                    high_queue_len: 0,
                    high_queue_capacity: 10_000,
                    normal_queue_len: 0,
                    normal_queue_capacity: 50_000,
                    low_queue_len: 0,
                    low_queue_capacity: 100_000,
                    total_high_dispatched: 0,
                    total_normal_dispatched: 0,
                    total_low_dispatched: 0,
                },
                self.config().worker_pool_size,
                Vec::new(),
            )
        };

        Ok(JobQueueStats {
            queue,
            workers: WorkerStats { pool_size },
            persisted: PersistedStats {
                // Never scan and deserialize an entire tenant job archive on
                // the request path. This endpoint backs the live console and
                // is called while jobs are being processed. Historical counts
                // are deliberately reported as unknown; the separate history
                // index is maintained for bounded archive browsing instead.
                count_known: false,
                total_entries: 0,
                orphaned_entries: 0,
            },
            categories,
        })
    }

    /// What this tenant may be told: one degraded bit and its own queue depth.
    ///
    /// Read entirely from live process state — the activity tracker's parked
    /// upstreams and the scheduler's per-tenant depths — so it costs no storage
    /// reads and can be polled at console refresh rates. Contrast
    /// `get_job_queue_stats`, which deliberately refuses to count persisted
    /// history for the same reason.
    ///
    /// Note what is NOT read here: the breaker registry is consulted only
    /// through the tracker's own derivation, which starts from work THIS tenant
    /// parked. Reaching into the registry directly would put another tenant's
    /// upstream in this answer.
    async fn get_tenant_job_health(&self, tenant: &str) -> Result<TenantJobHealth> {
        Ok(TenantJobHealth {
            degraded: crate::jobs::JobActivityTracker::global().tenant_degraded(tenant),
            queued: super::job_health::tenant_queue_depth(&self.job_tenant_queue_stats(), tenant),
        })
    }

    /// Force-fail jobs stuck in Running state for longer than `stuck_minutes`, scoped to tenant.
    async fn force_fail_stuck_jobs(
        &self,
        tenant: &str,
        stuck_minutes: u64,
    ) -> Result<(usize, Vec<String>)> {
        let cutoff = chrono::Utc::now() - chrono::Duration::minutes(stuck_minutes as i64);
        let persisted_jobs = self.job_metadata_store().list_for_tenant(tenant)?;

        let mut failed_ids = Vec::new();

        for (job_id, entry) in persisted_jobs {
            // Defensive: list_for_tenant already filters.
            if entry.tenant != tenant {
                continue;
            }
            // Only target Running jobs
            if !matches!(
                entry.status,
                raisin_storage::JobStatus::Running | raisin_storage::JobStatus::Executing
            ) {
                continue;
            }
            // Check if started_at is older than the cutoff
            if entry.started_at >= cutoff {
                continue;
            }

            let error_msg = format!(
                "Force-failed by admin: job stuck in Running state for >{} minutes",
                stuck_minutes
            );

            // Update in persistent storage
            let mut updated_entry = entry.clone();
            updated_entry.status = raisin_storage::JobStatus::Failed(error_msg.clone());
            updated_entry.completed_at = Some(chrono::Utc::now());
            updated_entry.error = Some(error_msg.clone());
            if let Err(e) = self.job_metadata_store().update(&job_id, &updated_entry) {
                tracing::warn!(job_id = %job_id, error = %e, "Failed to force-fail job in persistent storage");
                continue;
            }

            // Also update in-memory registry
            let _ = self
                .job_registry()
                .update_status(&job_id, raisin_storage::JobStatus::Failed(error_msg))
                .await;

            failed_ids.push(job_id.0);
        }

        let count = failed_ids.len();
        tracing::info!(
            count = count,
            stuck_minutes = stuck_minutes,
            "Force-failed stuck jobs"
        );

        Ok((count, failed_ids))
    }
}

#[async_trait]
impl BackgroundJobsInternal for RocksDBStorage {
    async fn list_all_jobs(&self) -> Result<Vec<raisin_storage::JobInfo>> {
        // Bounded for the same reason as `list_tenant_jobs_merged`: this
        // backs the operator endpoint the deploy health check polls.
        let persisted = self
            .job_metadata_store()
            .list_recent_unscoped(MAX_LISTED_JOBS, MAX_INLINE_RESULT_BYTES)?;
        Ok(persisted
            .into_iter()
            .map(|(job_id, entry)| raisin_storage::JobInfo {
                id: job_id,
                job_type: entry.job_type,
                status: entry.status,
                tenant: entry.tenant,
                started_at: entry.started_at,
                completed_at: entry.completed_at,
                progress: entry.progress,
                error: entry.error,
                result: entry.result,
                retry_count: entry.retry_count,
                max_retries: entry.max_retries,
                last_heartbeat: entry.last_heartbeat,
                timeout_seconds: entry.timeout_seconds,
                next_retry_at: entry.next_retry_at,
                executing_since: entry.executing_since,
            })
            .collect())
    }

    async fn purge_all_jobs_global(&self) -> Result<usize> {
        self.job_metadata_store().purge_all()
    }

    async fn force_fail_stuck_jobs_global(
        &self,
        stuck_minutes: u64,
    ) -> Result<(usize, Vec<String>)> {
        let cutoff = chrono::Utc::now() - chrono::Duration::minutes(stuck_minutes as i64);
        let persisted = self.job_metadata_store().list_all_unscoped()?;

        let mut failed_ids = Vec::new();

        for (job_id, entry) in persisted {
            if !matches!(
                entry.status,
                raisin_storage::JobStatus::Running | raisin_storage::JobStatus::Executing
            ) {
                continue;
            }
            if entry.started_at >= cutoff {
                continue;
            }

            let error_msg = format!(
                "Force-failed by superadmin: job stuck in Running state for >{} minutes",
                stuck_minutes
            );
            let mut updated = entry.clone();
            updated.status = raisin_storage::JobStatus::Failed(error_msg.clone());
            updated.completed_at = Some(chrono::Utc::now());
            updated.error = Some(error_msg.clone());
            if let Err(e) = self.job_metadata_store().update(&job_id, &updated) {
                tracing::warn!(job_id = %job_id, error = %e, "Failed to force-fail job (global)");
                continue;
            }
            let _ = self
                .job_registry()
                .update_status(&job_id, raisin_storage::JobStatus::Failed(error_msg))
                .await;
            failed_ids.push(job_id.0);
        }

        let count = failed_ids.len();
        tracing::warn!(
            count = count,
            stuck_minutes = stuck_minutes,
            "Force-failed stuck jobs across all tenants (superadmin)"
        );
        Ok((count, failed_ids))
    }

    async fn restore_pending_jobs(&self) -> Result<()> {
        // Delegate to the inherent restore_pending_jobs which returns RestoreStats.
        self.restore_pending_jobs().await.map(|_| ())
    }

    /// Every upstream breaker, every category pool, and per-tenant queue depth.
    ///
    /// All three halves are live process state — the breaker registry, the
    /// pools' atomics, the scheduler's queues — so this costs no storage reads
    /// and is safe to poll at dashboard refresh rates.
    ///
    /// Cross-tenant by construction, which is why it is on this trait and
    /// mounted only under `/management/admin/*`. The per-tenant route answers
    /// `get_tenant_job_health` instead; see that method for what it may not say.
    async fn get_job_system_health(&self) -> Result<JobSystemHealth> {
        let breakers = crate::jobs::CircuitBreakerRegistry::global()
            .statuses()
            .into_iter()
            .map(super::job_health::breaker_health)
            .collect();

        Ok(JobSystemHealth {
            breakers,
            pools: self.worker_pool_stats(),
            tenants: super::job_health::tenant_queue_health(self.job_tenant_queue_stats()),
        })
    }
}

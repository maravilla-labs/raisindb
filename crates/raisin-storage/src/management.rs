// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Management operations for storage implementations
//!
//! These traits provide enterprise-grade operations like integrity checking,
//! self-healing, monitoring, and maintenance for all storage backends.

use crate::Storage;
use async_trait::async_trait;
use raisin_error::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

/// Management operations for storage backends
#[async_trait]
pub trait ManagementOps: Storage {
    /// Check integrity of data for a specific tenant
    async fn check_integrity(&self, tenant: &str) -> Result<IntegrityReport>;

    /// Verify indexes consistency for a tenant
    async fn verify_indexes(&self, tenant: &str) -> Result<Vec<IndexIssue>>;

    /// Rebuild indexes for a tenant
    async fn rebuild_indexes(&self, tenant: &str, index_type: IndexType) -> Result<RebuildStats>;

    /// Clean up orphaned data for a tenant
    async fn cleanup_orphans(&self, tenant: &str) -> Result<u32>;

    /// Get health status for the storage system
    async fn get_health(&self, tenant: Option<&str>) -> Result<HealthStatus>;

    /// Get metrics for monitoring
    async fn get_metrics(&self, tenant: Option<&str>) -> Result<Metrics>;

    /// Trigger manual compaction
    async fn compact(&self, tenant: Option<&str>) -> Result<CompactionStats>;

    /// Create a backup for a tenant
    async fn backup_tenant(&self, tenant: &str, dest: &Path) -> Result<BackupInfo>;

    /// Restore a tenant from backup
    async fn restore_tenant(&self, tenant: &str, src: &Path) -> Result<()>;

    /// Backup all tenants
    async fn backup_all(&self, dest: &Path) -> Result<Vec<BackupInfo>>;
}

/// Background job management for storage backends.
///
/// All methods operate within a single tenant scope. The tenant is taken as an
/// explicit parameter so the trait can be safely implemented on a per-tenant
/// view (`ScopedStorage`) as well as directly on storage backends.
///
/// For truly cross-tenant operations (startup restore, global admin escape
/// hatches) see `BackgroundJobsInternal`.
#[async_trait]
pub trait BackgroundJobs: Send + Sync {
    /// Start background jobs (integrity scanning, auto-healing, etc.)
    ///
    /// This is a process-wide operation, not per-tenant.
    fn start_background_jobs(&self) -> Result<crate::JobHandle>;

    /// Schedule an integrity scan for a tenant
    fn schedule_integrity_scan(&self, tenant: &str, interval: Duration) -> Result<crate::JobId>;

    /// Get status of a background job within the given tenant.
    async fn get_job_status(
        &self,
        tenant: &str,
        job_id: &crate::JobId,
    ) -> Result<crate::JobStatus> {
        let info = self.get_job_info(tenant, job_id).await?;
        Ok(info.status)
    }

    /// Cancel a background job within the given tenant.
    async fn cancel_job(&self, tenant: &str, job_id: &crate::JobId) -> Result<()> {
        // Ensure the job belongs to the given tenant before cancelling.
        let _ = self.get_job_info(tenant, job_id).await?;
        crate::jobs::global_registry().cancel_job(job_id).await
    }

    /// Delete a background job within the given tenant.
    async fn delete_job(&self, tenant: &str, job_id: &crate::JobId) -> Result<()> {
        let _ = self.get_job_info(tenant, job_id).await?;
        crate::jobs::global_registry().delete_job(job_id).await
    }

    /// Delete multiple background jobs in a single batch operation within the given tenant.
    ///
    /// Returns (deleted_count, skipped_count) where skipped jobs are running/scheduled
    /// or belong to other tenants.
    async fn delete_jobs_batch(&self, tenant: &str, job_ids: &[crate::JobId]) -> (usize, usize) {
        let mut allowed: Vec<crate::JobId> = Vec::with_capacity(job_ids.len());
        let mut skipped = 0usize;
        for id in job_ids {
            match self.get_job_info(tenant, id).await {
                Ok(_) => allowed.push(id.clone()),
                Err(_) => skipped += 1,
            }
        }
        let (deleted, also_skipped) = crate::jobs::global_registry()
            .delete_jobs_batch(&allowed)
            .await;
        (deleted, skipped + also_skipped)
    }

    /// List all active background jobs for the given tenant.
    async fn list_jobs(&self, tenant: &str) -> Result<Vec<crate::JobInfo>> {
        Ok(crate::jobs::global_registry()
            .list_jobs_by_tenant(tenant)
            .await)
    }

    /// List jobs together with each job's execution scope (repository /
    /// branch / workspace from its persisted job context), optionally
    /// filtered to a repository.
    ///
    /// When `repo` is supplied, only jobs whose persisted execution context
    /// names that exact repository may be returned. Jobs with an unknown scope
    /// (legacy or system jobs) belong only in the tenant-wide view: including
    /// them in a repository page would leak work from other repositories.
    /// Backends without context storage return no rows for a repository filter.
    async fn list_jobs_with_scope(
        &self,
        tenant: &str,
        repo: Option<&str>,
    ) -> Result<Vec<crate::ScopedJobInfo>> {
        let jobs = self.list_jobs(tenant).await?;
        Ok(jobs
            .into_iter()
            .filter(|_| repo.is_none())
            .map(|info| crate::ScopedJobInfo { info, scope: None })
            .collect())
    }

    /// Wait for a job to complete within the given tenant.
    async fn wait_for_job(&self, tenant: &str, job_id: &crate::JobId) -> Result<crate::JobStatus> {
        let _ = self.get_job_info(tenant, job_id).await?;
        crate::jobs::global_registry()
            .wait_for_completion(job_id)
            .await
    }

    /// Get full job information including results for the given tenant.
    async fn get_job_info(&self, tenant: &str, job_id: &crate::JobId) -> Result<crate::JobInfo> {
        let info = crate::jobs::global_registry().get_job_info(job_id).await?;
        if info.tenant != tenant {
            return Err(raisin_error::Error::NotFound(format!(
                "Job {} not found",
                job_id
            )));
        }
        Ok(info)
    }

    /// Purge all jobs from persistent storage for the given tenant.
    ///
    /// Returns the number of entries purged.
    async fn purge_all_jobs(&self, _tenant: &str) -> Result<usize> {
        Err(raisin_error::Error::Validation(
            "purge_all_jobs not supported by this storage backend".to_string(),
        ))
    }

    /// Purge only orphaned (undeserializable) jobs from persistent storage for the tenant.
    async fn purge_orphaned_jobs(&self, _tenant: &str) -> Result<usize> {
        Err(raisin_error::Error::Validation(
            "purge_orphaned_jobs not supported by this storage backend".to_string(),
        ))
    }

    /// Get job queue statistics for the given tenant.
    async fn get_job_queue_stats(&self, _tenant: &str) -> Result<JobQueueStats> {
        Err(raisin_error::Error::Validation(
            "get_job_queue_stats not supported by this storage backend".to_string(),
        ))
    }

    /// What ONE tenant may be told about background processing.
    ///
    /// The operator picture — breaker keys, failure streaks, probe timers,
    /// host-wide pool saturation — lives on [`BackgroundJobsInternal`] and is
    /// reachable only through the superadmin subtree. It cannot be served here
    /// however convenient it is: a breaker is keyed by UPSTREAM and shared by
    /// every tenant on the box, so its hostname, its consecutive-failure count
    /// and its next-probe time are a fingerprint of OTHER tenants' traffic
    /// against that provider, and the pools count every tenant's work at once.
    /// Handing that to a tenant admin is a cross-tenant disclosure through a
    /// route that merely looks like a status check.
    ///
    /// So the answer is reduced to the two things a tenant can act on: is MY
    /// processing degraded, and how much of MY work is waiting.
    async fn get_tenant_job_health(&self, _tenant: &str) -> Result<TenantJobHealth> {
        Err(raisin_error::Error::Validation(
            "get_tenant_job_health not supported by this storage backend".to_string(),
        ))
    }

    /// Force-fail jobs that have been stuck in Running state for too long, for the given tenant.
    ///
    /// Returns (failed_count, list of job IDs that were force-failed).
    async fn force_fail_stuck_jobs(
        &self,
        _tenant: &str,
        _stuck_minutes: u64,
    ) -> Result<(usize, Vec<String>)> {
        Err(raisin_error::Error::Validation(
            "force_fail_stuck_jobs not supported by this storage backend".to_string(),
        ))
    }
}

/// Cross-tenant / internal job operations.
///
/// These methods deliberately bypass tenant scoping and are only intended
/// for startup recovery and operator-only admin tooling. They MUST NOT be
/// exposed on per-tenant HTTP routes.
#[async_trait]
pub trait BackgroundJobsInternal: Send + Sync {
    /// List jobs across all tenants.
    async fn list_all_jobs(&self) -> Result<Vec<crate::JobInfo>>;

    /// Purge all jobs across all tenants.
    async fn purge_all_jobs_global(&self) -> Result<usize>;

    /// Force-fail stuck jobs across all tenants.
    async fn force_fail_stuck_jobs_global(
        &self,
        stuck_minutes: u64,
    ) -> Result<(usize, Vec<String>)>;

    /// Restore pending jobs from persistent storage at startup.
    async fn restore_pending_jobs(&self) -> Result<()>;

    /// The whole job-system picture for the operator dashboard: every upstream
    /// breaker, every category pool, and per-tenant queue depth.
    ///
    /// On this trait — not on [`BackgroundJobs`] — because every field of it is
    /// cross-tenant by construction, and this trait is the one whose contract
    /// says so and whose routes are superadmin-gated.
    async fn get_job_system_health(&self) -> Result<JobSystemHealth>;
}

/// The read model behind `GET /management/admin/jobs/health` — OPERATOR ONLY.
///
/// Answers one question: is anything stuck, and why. The breakers say why work
/// is not being attempted; the pools say whether it is queued behind a full
/// machine; the tenant rows say whose work it is. No one of them distinguishes
/// "upstream is down" from "we are busy" from "one tenant is flooding the box",
/// which is exactly the confusion that cost an operator half an hour.
///
/// Every field here is cross-tenant. This shape must never be served from a
/// per-tenant route; see [`TenantJobHealth`] for what a tenant is told.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSystemHealth {
    /// One entry per upstream this process has touched, sorted by key.
    pub breakers: Vec<BreakerHealth>,
    /// One entry per category pool. Empty when the job system is not running
    /// (an embedded or test storage) rather than absent, so a reader never has
    /// to tell "no pools" from "field missing".
    pub pools: Vec<crate::jobs::CategoryPoolStats>,
    /// One entry per (tenant, category) with work queued right now, sorted by
    /// category then tenant. A tenant with an empty queue does not appear —
    /// the scheduler drops an emptied queue, and inventing a zero row for every
    /// tenant this process has ever served would make the list grow with
    /// history rather than with load.
    pub tenants: Vec<TenantQueueHealth>,
}

/// One tenant's queued work in one category pool, as an operator sees it.
///
/// A projection of the fair scheduler's `TenantQueueStats` rather than a
/// re-export: that type lives above this crate, carries no category (the
/// scheduler is per-category and does not need to say so), and is not a wire
/// format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantQueueHealth {
    /// Tenant id the work is charged to. This is the attribution the fair
    /// scheduler runs on, so it is also the answer to "who is using the box".
    pub tenant: String,
    /// `realtime`, `background` or `system` — the pool this depth is in.
    pub category: String,
    /// Credit granted per scheduling round. Equal for everyone until a tier
    /// model exists; a value above the default means a weight provider is
    /// installed.
    pub weight: u32,
    pub depth_high: usize,
    pub depth_normal: usize,
    pub depth_low: usize,
    /// Sum of the three, so a dashboard sorting by "biggest queue" does not
    /// have to agree with the server about how to add them up.
    pub depth_total: usize,
}

/// The read model behind `GET /management/jobs/health` — what a TENANT admin is
/// allowed to know.
///
/// Two facts, both of them about the caller's own work. Anything that would
/// identify an upstream, count its failures, time its next probe or total up
/// host-wide saturation is deliberately absent: those are shared across every
/// tenant on the host, so serving them here would leak other tenants' traffic
/// through a route that reads as a status check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantJobHealth {
    /// THIS tenant has background work parked against an upstream that is
    /// currently down.
    ///
    /// Derived exactly as the per-tenant activity node derives it — from work
    /// this tenant actually has parked — and NOT from "some breaker is open on
    /// this host". The host-wide reading would alarm every tenant on the box,
    /// including those with nothing in flight and those that do not use the
    /// affected upstream at all; a false alarm trains people to ignore the
    /// indicator, which costs us the next real incident.
    pub degraded: bool,
    /// This tenant's own queued work, summed across every category pool.
    pub queued: TenantQueueDepth,
}

/// Queue depth for one tenant, by priority.
///
/// Depths only. Not the pool's capacity, not its permits, not how many other
/// tenants are competing — a tenant learns how much of ITS work is waiting, and
/// a share-of-machine figure would be an inference about everyone else's.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TenantQueueDepth {
    pub high: usize,
    pub normal: usize,
    pub low: usize,
    /// Sum of the three. Present so a client's "N waiting" and the server's
    /// notion of the same number cannot drift.
    pub total: usize,
}

/// One upstream circuit breaker, as an operator sees it.
///
/// A projection of the job system's own breaker status rather than a re-export:
/// the breaker lives above this crate, and its `Duration` and state enum are
/// not a wire format. `next_probe_in_secs` is seconds because a console renders
/// seconds, and a `Duration` serialises as a struct nobody wants to parse.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakerHealth {
    /// The upstream this breaker guards (e.g. an embedding provider).
    pub key: String,
    /// `closed`, `open` or `half_open`.
    pub state: String,
    /// Consecutive upstream faults. Reset by any success.
    pub consecutive_failures: u32,
    /// Message of the most recent fault counted against this breaker.
    pub last_error: Option<String>,
    /// Seconds until a probe is allowed. `None` unless the breaker is open.
    pub next_probe_in_secs: Option<u64>,
}

/// Statistics about the job queue system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobQueueStats {
    pub queue: QueueDepthStats,
    pub workers: WorkerStats,
    pub persisted: PersistedStats,
    /// Per-category queue depth breakdown (Realtime, Background, System)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<CategoryQueueDepthStats>,
}

/// Queue depth statistics per priority level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueDepthStats {
    pub high_queue_len: usize,
    pub high_queue_capacity: usize,
    pub normal_queue_len: usize,
    pub normal_queue_capacity: usize,
    pub low_queue_len: usize,
    pub low_queue_capacity: usize,
    pub total_high_dispatched: u64,
    pub total_normal_dispatched: u64,
    pub total_low_dispatched: u64,
}

/// Per-category queue depth breakdown
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryQueueDepthStats {
    /// Category name: "Realtime", "Background", or "System"
    pub category: String,
    pub high_queue_len: usize,
    pub normal_queue_len: usize,
    pub low_queue_len: usize,
    pub total_dispatched: u64,
}

/// Worker pool statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStats {
    pub pool_size: usize,
}

/// Persisted job storage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedStats {
    /// Whether these values were obtained without a full history scan.
    ///
    /// `false` means the server intentionally did not count persisted history
    /// on this request. A real-time queue view must never deserialize an
    /// unbounded archive merely to render a counter.
    #[serde(default)]
    pub count_known: bool,
    pub total_entries: usize,
    pub orphaned_entries: usize,
}

// Data structures for management operations

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityReport {
    pub tenant: String,
    pub scan_time: chrono::DateTime<chrono::Utc>,
    pub nodes_checked: u64,
    pub issues_found: Vec<Issue>,
    pub health_score: f32, // 0.0 to 1.0
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Issue {
    OrphanedNode {
        id: String,
        parent_id: Option<String>,
    },
    MissingIndex {
        node_id: String,
        index_type: IndexType,
    },
    InconsistentIndex {
        node_id: String,
        expected: String,
        actual: String,
    },
    CorruptedData {
        node_id: String,
        error: String,
    },
    BrokenReference {
        from_id: String,
        to_id: String,
        ref_type: String,
    },
    DuplicateChild {
        parent_id: String,
        child_id: String,
    },
    MissingWorkspace {
        node_id: String,
        workspace_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexIssue {
    pub index_type: IndexType,
    pub node_id: String,
    pub description: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum IndexType {
    Property,
    Reference,
    ChildOrder,
    FullText,
    Vector,
    Compound,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebuildStats {
    pub index_type: IndexType,
    pub items_processed: u64,
    pub errors: u64,
    pub duration_ms: u64,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub status: HealthLevel,
    pub tenant: Option<String>,
    pub checks: Vec<HealthCheck>,
    pub needs_healing: bool,
    pub last_check: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum HealthLevel {
    Healthy,
    Degraded,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    pub name: String,
    pub status: HealthLevel,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metrics {
    pub tenant: Option<String>,
    pub operations_per_sec: f64,
    pub error_rate: f64,
    pub disk_usage_bytes: u64,
    pub index_sizes: std::collections::HashMap<String, u64>,
    pub node_count: u64,
    pub active_connections: u32,
    pub cache_hit_rate: f64,
    pub last_compaction: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionStats {
    pub tenant: Option<String>,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub duration_ms: u64,
    pub files_compacted: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupInfo {
    pub tenant: String,
    pub path: std::path::PathBuf,
    pub size_bytes: u64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub duration_ms: u64,
    pub node_count: u64,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairResult {
    pub tenant: String,
    pub issues_repaired: usize,
    pub issues_failed: usize,
    pub repairs_by_type: std::collections::HashMap<String, usize>,
    pub duration_ms: u64,
    pub errors: Vec<String>,
}

// New types for index management

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexReport {
    pub index_type: String,
    pub status: IndexStatus,
    pub issues: Vec<String>,
    pub health_score: f32,
    pub total_entries: u64,
    pub corrupted_entries: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IndexStatus {
    Healthy,
    Degraded,
    Missing,
    Corrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexHealth {
    pub index_type: String,
    pub memory_usage_bytes: u64,
    pub disk_usage_bytes: u64,
    pub entry_count: u64,
    pub cache_hit_rate: Option<f32>,
    pub last_optimized: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizeStats {
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub duration_ms: u64,
    pub segments_merged: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreStats {
    pub entries_restored: u64,
    pub entries_skipped: u64,
    pub duration_ms: u64,
}

/// Extended index management operations for fulltext and vector indexes
#[async_trait]
pub trait IndexManagement: Send + Sync {
    // RocksDB index operations (already exists in ManagementOps)
    // We keep these here for consistency

    // Fulltext (Tantivy) operations
    async fn verify_fulltext_index(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<IndexReport>;

    async fn rebuild_fulltext_index(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<RebuildStats>;

    async fn optimize_fulltext_index(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<OptimizeStats>;

    async fn purge_fulltext_index(&self, tenant: &str, repo: &str, branch: &str) -> Result<()>;

    async fn fulltext_index_health(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<IndexHealth>;

    // Vector (HNSW) operations - stubs for Phase 3
    async fn verify_vector_index(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<IndexReport>;

    async fn rebuild_vector_index(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<RebuildStats>;

    async fn optimize_vector_index(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<OptimizeStats>;

    async fn restore_vector_index(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<RestoreStats>;

    async fn vector_index_health(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<IndexHealth>;
}

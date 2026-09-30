//! Revision compaction for repositories
//!
//! This module provides repository-level compaction operations:
//! - Compact node revisions based on retention policies
//! - Compact tree storage (content-addressed)
//! - Run RocksDB-level compaction for space reclamation
//!
//! Compaction is scoped to repositories to ensure data integrity and proper isolation.

mod helpers;

use crate::RocksDBStorage;
use raisin_error::Result;
use raisin_storage::CompactionStats;
use std::time::Duration;

pub use helpers::{get_repository_size, get_total_db_size};

/// Revision retention policy
#[derive(Debug, Clone)]
pub enum RevisionRetentionPolicy {
    /// Keep the N most recent revisions
    KeepLatest(usize),
    /// Keep revisions newer than the specified duration
    KeepSince(Duration),
    /// Keep all revisions (no compaction)
    KeepAll,
}

impl RevisionRetentionPolicy {
    /// The equivalent history-GC retention.
    pub fn to_history_retention(&self) -> super::history_gc::HistoryRetention {
        use super::history_gc::HistoryRetention;
        match self {
            RevisionRetentionPolicy::KeepLatest(n) => HistoryRetention {
                keep_days: None,
                keep_revisions: Some(*n as u64),
            },
            RevisionRetentionPolicy::KeepSince(d) => HistoryRetention {
                // Whole days, rounded up so nothing inside the window is lost.
                keep_days: Some(d.as_secs().div_ceil(86_400) as u32),
                keep_revisions: None,
            },
            RevisionRetentionPolicy::KeepAll => HistoryRetention::KEEP_ALL,
        }
    }
}

/// Compact revisions for a specific repository
///
/// Prunes revision history with the MVCC-aware history GC (which honours
/// tags and branch fork points and compacts the column families it touched).
/// The previous implementation deleted node versions without regard to
/// branches or tags, rescanned the whole repository once per node, and left
/// every index family untouched.
pub async fn compact_repository(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    policy: RevisionRetentionPolicy,
) -> Result<CompactionStats> {
    let start = std::time::Instant::now();
    tracing::info!(
        "Compacting repository {}/{} with policy {:?}",
        tenant_id,
        repo_id,
        policy
    );

    let opts = super::history_gc::GcOptions {
        retention_override: Some(policy.to_history_retention()),
        tenant: Some(tenant_id.to_string()),
        repo: Some(repo_id.to_string()),
        // Blob deletion needs the binary store, which this layer does not own.
        collect_orphaned_blobs: false,
        ..Default::default()
    };
    let report = super::history_gc::run_history_gc(storage, &opts)?;

    Ok(CompactionStats {
        tenant: Some(format!("{}/{}", tenant_id, repo_id)),
        bytes_before: report.live_sst_bytes_before,
        bytes_after: report.live_sst_bytes_after,
        duration_ms: start.elapsed().as_millis() as u64,
        files_compacted: 0,
    })
}

/// Compact all repositories for a tenant
pub async fn compact_tenant(
    storage: &RocksDBStorage,
    tenant_id: &str,
    policy: RevisionRetentionPolicy,
) -> Result<CompactionStats> {
    let start = std::time::Instant::now();

    tracing::info!("Compacting all repositories for tenant {}", tenant_id);

    // Get all repositories
    let repos = helpers::list_repositories(storage, tenant_id).await?;
    tracing::info!("Found {} repositories to compact", repos.len());

    let mut total_before = 0u64;
    let mut total_after = 0u64;

    for repo_id in repos {
        let stats = compact_repository(storage, tenant_id, &repo_id, policy.clone()).await?;
        total_before += stats.bytes_before;
        total_after += stats.bytes_after;
    }

    // Run global compaction
    helpers::compact_all_column_families(storage, None, None);

    let duration_ms = start.elapsed().as_millis() as u64;

    Ok(CompactionStats {
        tenant: Some(tenant_id.to_string()),
        bytes_before: total_before,
        bytes_after: total_after,
        duration_ms,
        files_compacted: 0,
    })
}

/// Compact global database (all tenants)
pub async fn compact_global(storage: &RocksDBStorage) -> Result<CompactionStats> {
    let start = std::time::Instant::now();

    tracing::info!("Running global database compaction");

    // Get approximate size before (sum across all column families)
    let bytes_before = get_total_db_size(storage)?;

    // Run RocksDB compaction across all column families
    helpers::compact_all_column_families(storage, None, None);

    let bytes_after = get_total_db_size(storage)?;
    let duration_ms = start.elapsed().as_millis() as u64;

    Ok(CompactionStats {
        tenant: None,
        bytes_before,
        bytes_after,
        duration_ms,
        files_compacted: 0,
    })
}

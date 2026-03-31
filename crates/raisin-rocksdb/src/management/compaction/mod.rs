//! Revision compaction for repositories
//!
//! This module provides repository-level compaction operations:
//! - Compact node revisions based on retention policies
//! - Compact tree storage (content-addressed)
//! - Run RocksDB-level compaction for space reclamation
//!
//! Compaction is scoped to repositories to ensure data integrity and proper isolation.

mod helpers;
mod node_compaction;
mod tree_compaction;

use crate::{cf, cf_handle, keys, RocksDBStorage};
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

/// Compact revisions for a specific repository
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

    // 1. Get size before compaction
    let bytes_before = get_repository_size(storage, tenant_id, repo_id)?;

    // 2. For each node in repository, apply retention policy
    let node_ids = helpers::list_all_node_ids(storage, tenant_id, repo_id).await?;
    tracing::info!("Found {} unique nodes to compact", node_ids.len());

    for node_id in &node_ids {
        if let Err(e) =
            node_compaction::compact_node_revisions(storage, tenant_id, repo_id, node_id, &policy)
                .await
        {
            tracing::warn!("Failed to compact node {}: {}", node_id, e);
        }
    }

    // 3. Compact tree storage (remove unreferenced trees)
    tree_compaction::compact_trees(storage, tenant_id, repo_id).await?;

    // 4. Run RocksDB compaction to reclaim space
    tracing::info!(
        "Running RocksDB compaction for repository {}/{}",
        tenant_id,
        repo_id
    );
    helpers::run_rocksdb_compaction(storage, tenant_id, repo_id)?;

    // 5. Get size after compaction
    let bytes_after = get_repository_size(storage, tenant_id, repo_id)?;
    let duration_ms = start.elapsed().as_millis() as u64;

    let stats = CompactionStats {
        tenant: Some(format!("{}/{}", tenant_id, repo_id)),
        bytes_before,
        bytes_after,
        duration_ms,
        files_compacted: 0, // RocksDB doesn't expose this easily
    };

    tracing::info!(
        "Compaction complete: {} -> {} bytes ({:.1}% reduction) in {}ms",
        bytes_before,
        bytes_after,
        (1.0 - bytes_after as f64 / bytes_before.max(1) as f64) * 100.0,
        duration_ms
    );

    Ok(stats)
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
    storage.db().compact_range::<&[u8], &[u8]>(None, None);

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
    storage.db().compact_range::<&[u8], &[u8]>(None, None);

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

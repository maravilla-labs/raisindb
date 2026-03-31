//! ManagementOps trait implementation for RocksDBStorage.
//!
//! Provides health checks, integrity verification, index rebuilding,
//! backup/restore, and compaction operations.

use super::helpers::{
    list_all_tenants, list_branches_for_repo, list_repositories_for_tenant,
    list_workspaces_for_repo,
};
use super::{async_indexing, backup, compaction, integrity, metrics};
use crate::RocksDBStorage;
use async_trait::async_trait;
use raisin_error::Result;
use raisin_storage::{
    BackupInfo, CompactionStats, HealthStatus, IndexIssue, IndexType, IntegrityReport,
    ManagementOps, Metrics, RebuildStats,
};
use std::path::Path;

#[async_trait]
impl ManagementOps for RocksDBStorage {
    async fn get_health(&self, tenant: Option<&str>) -> Result<HealthStatus> {
        metrics::get_health(self, tenant).await
    }

    async fn get_metrics(&self, tenant: Option<&str>) -> Result<Metrics> {
        metrics::get_metrics(self, tenant).await
    }

    async fn check_integrity(&self, tenant: &str) -> Result<IntegrityReport> {
        // Check entire tenant (all repositories)
        integrity::check_tenant(self, tenant).await
    }

    async fn verify_indexes(&self, tenant: &str) -> Result<Vec<IndexIssue>> {
        // Run integrity check and convert issues to index issues
        let report = integrity::check_tenant(self, tenant).await?;

        let mut index_issues = Vec::new();
        for issue in report.issues_found {
            match issue {
                raisin_storage::Issue::MissingIndex {
                    node_id,
                    index_type,
                } => {
                    index_issues.push(IndexIssue {
                        index_type,
                        node_id,
                        description: "Index is missing".to_string(),
                    });
                }
                raisin_storage::Issue::InconsistentIndex {
                    node_id,
                    expected,
                    actual,
                } => {
                    index_issues.push(IndexIssue {
                        index_type: IndexType::Property,
                        node_id,
                        description: format!("Expected: {}, Actual: {}", expected, actual),
                    });
                }
                _ => {}
            }
        }

        Ok(index_issues)
    }

    async fn rebuild_indexes(&self, tenant: &str, index_type: IndexType) -> Result<RebuildStats> {
        // Get all repositories for tenant
        let repos = list_repositories_for_tenant(self, tenant).await?;

        let mut combined_stats = RebuildStats {
            index_type,
            items_processed: 0,
            errors: 0,
            duration_ms: 0,
            success: true,
        };

        for repo_id in repos {
            // Get branches for this repository
            let branches = list_branches_for_repo(self, tenant, &repo_id).await?;

            for branch in branches {
                // Get workspaces for this branch
                let workspaces = list_workspaces_for_repo(self, tenant, &repo_id).await?;

                for workspace in workspaces {
                    let stats = async_indexing::rebuild_indexes(
                        self, tenant, &repo_id, &branch, &workspace, index_type,
                    )
                    .await?;

                    combined_stats.items_processed += stats.items_processed;
                    combined_stats.errors += stats.errors;
                    combined_stats.duration_ms += stats.duration_ms;
                    combined_stats.success &= stats.success;
                }
            }
        }

        Ok(combined_stats)
    }

    async fn cleanup_orphans(&self, tenant: &str) -> Result<u32> {
        // Run integrity check to find orphans, then clean them up
        let report = integrity::check_tenant(self, tenant).await?;

        let mut cleaned = 0u32;
        for issue in report.issues_found {
            if let raisin_storage::Issue::OrphanedNode { id, .. } = issue {
                // For now, just log - actual cleanup would delete the node
                tracing::warn!("Found orphaned node: {}", id);
                cleaned += 1;
            }
        }

        Ok(cleaned)
    }

    async fn compact(&self, tenant: Option<&str>) -> Result<CompactionStats> {
        match tenant {
            Some(tenant_id) => {
                // Compact specific tenant with default retention policy
                let policy = compaction::RevisionRetentionPolicy::KeepLatest(100);
                compaction::compact_tenant(self, tenant_id, policy).await
            }
            None => {
                // Compact entire database
                compaction::compact_global(self).await
            }
        }
    }

    async fn backup_tenant(&self, tenant: &str, dest: &Path) -> Result<BackupInfo> {
        // Backup all repositories for this tenant
        let infos = backup::backup_tenant(self, tenant, dest).await?;

        // Combine into a single BackupInfo
        let mut total_size = 0u64;
        let mut total_nodes = 0u64;
        let mut total_duration = 0u64;

        for info in &infos {
            total_size += info.size_bytes;
            total_nodes += info.node_count;
            total_duration += info.duration_ms;
        }

        Ok(BackupInfo {
            tenant: tenant.to_string(),
            path: dest.to_path_buf(),
            size_bytes: total_size,
            created_at: chrono::Utc::now(),
            duration_ms: total_duration,
            node_count: total_nodes,
            version: env!("CARGO_PKG_VERSION").to_string(),
        })
    }

    async fn restore_tenant(&self, tenant: &str, src: &Path) -> Result<()> {
        // Get all repositories in backup directory
        let tenant_dir = src.join(tenant);

        if !tenant_dir.exists() {
            return Err(raisin_error::Error::storage(format!(
                "Backup directory not found: {}",
                tenant_dir.display()
            )));
        }

        // Iterate through repository directories
        for entry in std::fs::read_dir(&tenant_dir)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to read directory: {}", e)))?
        {
            let entry = entry.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to read directory entry: {}", e))
            })?;

            if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                let repo_id = entry.file_name().to_string_lossy().to_string();
                backup::restore_repository(self, tenant, &repo_id, src).await?;
            }
        }

        Ok(())
    }

    async fn backup_all(&self, dest: &Path) -> Result<Vec<BackupInfo>> {
        // Get all tenants
        let tenants = list_all_tenants(self).await?;

        let mut all_infos = Vec::new();

        for tenant in tenants {
            let infos = backup::backup_tenant(self, &tenant, dest).await?;
            all_infos.extend(infos);
        }

        Ok(all_infos)
    }
}

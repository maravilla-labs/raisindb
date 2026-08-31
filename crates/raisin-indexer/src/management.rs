// SPDX-License-Identifier: BSL-1.1

//! Management operations for Tantivy full-text indexes
//!
//! Provides enterprise-grade operations for maintaining Tantivy indexes:
//! - Verification: Check index integrity and health
//! - Rebuild: Recreate indexes from scratch
//! - Optimize: Merge segments for better performance
//! - Purge: Remove indexes completely
//! - Health: Monitor resource usage and status

use crate::TantivyIndexingEngine;
use raisin_error::{Error, Result};
use raisin_storage::{
    IndexHealth, IndexReport, IndexStatus, IndexType, OptimizeStats, RebuildStats,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tantivy::Index;

/// Management operations for Tantivy full-text indexes
pub struct TantivyManagement {
    base_path: PathBuf,
    engine: Arc<TantivyIndexingEngine>,
}

impl TantivyManagement {
    /// Create a new TantivyManagement instance
    ///
    /// # Arguments
    ///
    /// * `base_path` - Root directory for all indexes
    /// * `engine` - TantivyIndexingEngine for index operations
    pub fn new(base_path: PathBuf, engine: Arc<TantivyIndexingEngine>) -> Self {
        Self { base_path, engine }
    }

    /// Verify index integrity and report issues
    ///
    /// # Arguments
    ///
    /// * `tenant` - Tenant identifier
    /// * `repo` - Repository identifier
    /// * `branch` - Branch name
    ///
    /// # Returns
    ///
    /// IndexReport containing status, health score, and any issues found
    pub async fn verify_index(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<IndexReport> {
        let index_path = self.base_path.join(tenant).join(repo).join(branch);

        if !index_path.exists() {
            return Ok(IndexReport {
                index_type: "FullText".to_string(),
                status: IndexStatus::Missing,
                issues: vec![],
                health_score: 0.0,
                total_entries: 0,
                corrupted_entries: 0,
            });
        }

        // Load index and check integrity
        let index = Index::open_in_dir(&index_path)
            .map_err(|e| Error::storage(format!("Failed to open index: {}", e)))?;

        let reader = index
            .reader()
            .map_err(|e| Error::storage(format!("Failed to create reader: {}", e)))?;

        let searcher = reader.searcher();

        // Count documents
        let total_docs = searcher.num_docs();

        // Check for corruption (attempt to read all segments)
        let mut corrupted = 0u64;
        let mut issues = Vec::new();

        for segment_reader in searcher.segment_readers() {
            // alive_bitset() returns Option, not Result
            // If it's None, the segment might be corrupted
            if segment_reader.alive_bitset().is_none() {
                corrupted += segment_reader.num_docs() as u64;
                issues.push(format!(
                    "Corrupted segment {:?}: missing alive bitset",
                    segment_reader.segment_id(),
                ));
            }
        }

        let health_score = if total_docs > 0 {
            1.0 - (corrupted as f32 / total_docs as f32)
        } else {
            1.0
        };

        let status = if health_score >= 0.99 {
            IndexStatus::Healthy
        } else if health_score >= 0.75 {
            IndexStatus::Degraded
        } else {
            IndexStatus::Corrupted
        };

        Ok(IndexReport {
            index_type: "FullText".to_string(),
            status,
            issues,
            health_score,
            total_entries: total_docs,
            corrupted_entries: corrupted,
        })
    }

    /// Rebuild index from scratch
    ///
    /// This deletes the existing index. The actual re-indexing would be
    /// triggered by the background worker through the job system.
    ///
    /// # Arguments
    ///
    /// * `tenant` - Tenant identifier
    /// * `repo` - Repository identifier
    /// * `branch` - Branch name
    ///
    /// # Returns
    ///
    /// RebuildStats with operation details
    pub async fn rebuild_index(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<RebuildStats> {
        let index_path = self.base_path.join(tenant).join(repo).join(branch);
        let start = std::time::Instant::now();

        // Delete existing index
        if index_path.exists() {
            std::fs::remove_dir_all(&index_path)
                .map_err(|e| Error::storage(format!("Failed to remove index: {}", e)))?;
        }

        // Re-indexing would happen via background worker
        // The worker would pick up the job and re-index all nodes

        Ok(RebuildStats {
            index_type: IndexType::FullText,
            items_processed: 0, // Will be updated by worker
            errors: 0,
            duration_ms: start.elapsed().as_millis() as u64,
            success: true,
        })
    }

    /// Optimize index by merging segments
    ///
    /// This reduces the number of segments in the index, improving
    /// search performance and reducing disk space usage.
    ///
    /// # Arguments
    ///
    /// * `tenant` - Tenant identifier
    /// * `repo` - Repository identifier
    /// * `branch` - Branch name
    ///
    /// # Returns
    ///
    /// OptimizeStats with before/after sizes and merge details
    pub async fn optimize_index(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<OptimizeStats> {
        let index_path = self.base_path.join(tenant).join(repo).join(branch);

        if !index_path.exists() {
            return Err(Error::NotFound("Index not found".to_string()));
        }

        let bytes_before = Self::get_directory_size(&index_path)?;
        let start = std::time::Instant::now();

        // Delegate to the engine so the merge runs on the index's own shared
        // writer. Opening a second writer here fails with `DirectoryLockBusy`
        // against a live index, and a throwaway writer would need
        // `wait_merging_threads()` to stop `Drop` from killing the merge it
        // just started.
        let (segments_before, segments_after) = self.engine.optimize(tenant, repo, branch)?;

        let bytes_after = Self::get_directory_size(&index_path)?;

        Ok(OptimizeStats {
            bytes_before,
            bytes_after,
            duration_ms: start.elapsed().as_millis() as u64,
            segments_merged: segments_before.saturating_sub(segments_after) as u32,
        })
    }

    /// Purge index completely
    ///
    /// This removes the entire index directory. Use with caution.
    ///
    /// # Arguments
    ///
    /// * `tenant` - Tenant identifier
    /// * `repo` - Repository identifier
    /// * `branch` - Branch name
    pub async fn purge_index(&self, tenant: &str, repo: &str, branch: &str) -> Result<()> {
        let index_path = self.base_path.join(tenant).join(repo).join(branch);

        // Close the cached handles and the writer first — the writer holds the
        // directory lock and its merge threads would otherwise go on writing
        // into a tree that is being deleted.
        self.engine.invalidate_cached_index(tenant, repo, branch);

        if index_path.exists() {
            std::fs::remove_dir_all(&index_path)
                .map_err(|e| Error::storage(format!("Failed to purge index: {}", e)))?;
        }

        Ok(())
    }

    /// Get index health metrics
    ///
    /// # Arguments
    ///
    /// * `tenant` - Tenant identifier
    /// * `repo` - Repository identifier
    /// * `branch` - Branch name
    ///
    /// # Returns
    ///
    /// IndexHealth with resource usage and status information
    pub async fn get_health(&self, tenant: &str, repo: &str, branch: &str) -> Result<IndexHealth> {
        let index_path = self.base_path.join(tenant).join(repo).join(branch);

        if !index_path.exists() {
            return Ok(IndexHealth {
                index_type: "FullText".to_string(),
                memory_usage_bytes: 0,
                disk_usage_bytes: 0,
                entry_count: 0,
                cache_hit_rate: None,
                last_optimized: None,
            });
        }

        let index = Index::open_in_dir(&index_path)
            .map_err(|e| Error::storage(format!("Failed to open index: {}", e)))?;

        let reader = index
            .reader()
            .map_err(|e| Error::storage(format!("Failed to create reader: {}", e)))?;

        let searcher = reader.searcher();

        let disk_usage = Self::get_directory_size(&index_path)?;
        let last_modified = Self::get_last_modified(&index_path).ok();

        Ok(IndexHealth {
            index_type: "FullText".to_string(),
            memory_usage_bytes: 0, // Would need tracking in engine
            disk_usage_bytes: disk_usage,
            entry_count: searcher.num_docs(),
            cache_hit_rate: None, // Would need tracking in engine
            last_optimized: last_modified,
        })
    }

    // Helper functions

    /// Recursively calculate directory size
    fn get_directory_size(path: &Path) -> Result<u64> {
        let mut total_size = 0u64;

        if path.is_dir() {
            for entry in std::fs::read_dir(path)
                .map_err(|e| Error::storage(format!("Failed to read dir: {}", e)))?
            {
                let entry =
                    entry.map_err(|e| Error::storage(format!("Failed to read entry: {}", e)))?;
                let metadata = entry
                    .metadata()
                    .map_err(|e| Error::storage(format!("Failed to get metadata: {}", e)))?;

                if metadata.is_dir() {
                    total_size += Self::get_directory_size(&entry.path())?;
                } else {
                    total_size += metadata.len();
                }
            }
        }

        Ok(total_size)
    }

    /// Get last modified time of a path
    fn get_last_modified(path: &Path) -> Result<chrono::DateTime<chrono::Utc>> {
        let metadata = std::fs::metadata(path)
            .map_err(|e| Error::storage(format!("Failed to get metadata: {}", e)))?;

        let modified = metadata
            .modified()
            .map_err(|e| Error::storage(format!("Failed to get modified time: {}", e)))?;

        Ok(chrono::DateTime::from(modified))
    }
}

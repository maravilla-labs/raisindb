//! Relation consistency check and repair job handler
//!
//! This module handles background consistency checking and repair of the
//! global relation index. It scans for orphaned relations (references to
//! deleted/tombstoned nodes) and optionally repairs them by creating tombstones.

use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use rocksdb::DB;
use std::sync::Arc;

use crate::keys::relation_global_key_versioned;
use crate::repositories::{
    deserialize_full_relation, get_relation_cf, is_node_tombstone,
    is_relation_tombstone as is_tombstone, RELATION_TOMBSTONE as TOMBSTONE,
};
use crate::{cf, cf_handle};

/// Handler for relation consistency check jobs
///
/// This handler:
/// 1. Scans the global relation index
/// 2. For each relation, checks if source and target nodes exist
/// 3. If repair=true, writes tombstones for orphaned relations
/// 4. Reports statistics about found/fixed issues
pub struct RelationConsistencyHandler {
    db: Arc<DB>,
}

impl RelationConsistencyHandler {
    /// Create a new relation consistency handler
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Handle relation consistency check job
    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        // Extract repair flag from JobType
        let repair = match &job.job_type {
            JobType::RelationConsistencyCheck { repair } => *repair,
            _ => {
                return Err(Error::Validation(
                    "Expected RelationConsistencyCheck job type".to_string(),
                ))
            }
        };

        tracing::info!(
            job_id = %job.id,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            repair = repair,
            "Starting relation consistency check"
        );

        let mut stats = ConsistencyStats::default();

        // Scan global relation index for this tenant/repo/branch
        // Use revision from context for any repair tombstones
        self.scan_global_relations(
            &context.tenant_id,
            &context.repo_id,
            &context.branch,
            repair,
            &context.revision,
            &mut stats,
        )
        .await?;

        tracing::info!(
            job_id = %job.id,
            relations_scanned = stats.relations_scanned,
            orphaned_source = stats.orphaned_source,
            orphaned_target = stats.orphaned_target,
            tombstones_written = stats.tombstones_written,
            errors = stats.errors,
            "Relation consistency check completed"
        );

        Ok(())
    }

    /// Scan global relation index for orphaned relations
    async fn scan_global_relations(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        repair: bool,
        revision: &raisin_hlc::HLC,
        stats: &mut ConsistencyStats,
    ) -> Result<()> {
        let cf_relation = get_relation_cf(&self.db)?;

        // Build prefix for global relations: {tenant}\0{repo}\0{branch}\0rel_global\0
        let prefix = format!("{}\0{}\0{}\0rel_global\0", tenant_id, repo_id, branch);
        let prefix_bytes = prefix.as_bytes();

        let iter = self.db.prefix_iterator_cf(cf_relation, prefix_bytes);

        for item in iter {
            let (key, value) = match item {
                Ok((k, v)) => (k, v),
                Err(e) => {
                    tracing::warn!("Error reading relation key: {}", e);
                    stats.errors += 1;
                    continue;
                }
            };

            if !key.starts_with(prefix_bytes) {
                break;
            }

            // Skip tombstones
            if is_tombstone(&value) {
                continue;
            }

            stats.relations_scanned += 1;

            // Parse the full relation to get source and target info
            let relation = match deserialize_full_relation(&value) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("Error deserializing relation: {}", e);
                    stats.errors += 1;
                    continue;
                }
            };

            // Check if source node exists
            let source_exists = self
                .node_exists(
                    tenant_id,
                    repo_id,
                    branch,
                    &relation.source_workspace,
                    &relation.source_id,
                )
                .await;

            // Check if target node exists
            let target_exists = self
                .node_exists(
                    tenant_id,
                    repo_id,
                    branch,
                    &relation.target_workspace,
                    &relation.target_id,
                )
                .await;

            let mut is_orphaned = false;

            if !source_exists {
                stats.orphaned_source += 1;
                is_orphaned = true;
                tracing::warn!(
                    source_id = %relation.source_id,
                    target_id = %relation.target_id,
                    relation_type = %relation.relation_type,
                    "Found orphaned relation: source node missing"
                );
            }

            if !target_exists {
                stats.orphaned_target += 1;
                is_orphaned = true;
                tracing::warn!(
                    source_id = %relation.source_id,
                    target_id = %relation.target_id,
                    relation_type = %relation.relation_type,
                    "Found orphaned relation: target node missing"
                );
            }

            // Repair by writing tombstone
            if is_orphaned && repair {
                let global_key = relation_global_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    &relation.relation_type,
                    revision,
                    &relation.source_workspace,
                    &relation.source_id,
                    &relation.target_workspace,
                    &relation.target_id,
                );

                if let Err(e) = self.db.put_cf(cf_relation, &global_key, TOMBSTONE) {
                    tracing::error!("Failed to write tombstone: {}", e);
                    stats.errors += 1;
                } else {
                    stats.tombstones_written += 1;
                    tracing::debug!(
                        source_id = %relation.source_id,
                        target_id = %relation.target_id,
                        "Tombstoned orphaned relation"
                    );
                }
            }
        }

        Ok(())
    }

    /// Check if a node exists (is not tombstoned)
    async fn node_exists(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
    ) -> bool {
        let cf_nodes = match cf_handle(&self.db, cf::NODES) {
            Ok(cf) => cf,
            Err(_) => return false,
        };

        // Build node key prefix: {tenant}\0{repo}\0{branch}\0{workspace}\0node\0{node_id}\0
        let prefix = format!(
            "{}\0{}\0{}\0{}\0node\0{}\0",
            tenant_id, repo_id, branch, workspace, node_id
        );
        let prefix_bytes = prefix.as_bytes();

        // Use prefix iterator to find the latest version
        let iter = self.db.prefix_iterator_cf(cf_nodes, prefix_bytes);

        for item in iter {
            let (key, value) = match item {
                Ok((k, v)) => (k, v),
                Err(_) => continue,
            };

            if !key.starts_with(prefix_bytes) {
                break;
            }

            // Check if this version is a tombstone
            if is_node_tombstone(&value) {
                return false;
            }

            // Found a non-tombstone version
            return true;
        }

        // No versions found
        false
    }
}

/// Statistics for consistency check operation
#[derive(Default)]
struct ConsistencyStats {
    relations_scanned: usize,
    orphaned_source: usize,
    orphaned_target: usize,
    tombstones_written: usize,
    errors: usize,
}

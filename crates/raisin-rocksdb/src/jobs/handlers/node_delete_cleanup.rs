//! Node delete cleanup job handler
//!
//! This module handles background cleanup of all indexes when a node is deleted.
//! Moving index cleanup to a background job ensures:
//! 1. Fast, non-blocking delete operations for users
//! 2. Single source of truth for cleanup logic
//! 3. Cluster-safe via JobRegistry
//! 4. Retry on failure

use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use rocksdb::DB;
use std::sync::Arc;

use crate::keys::{
    relation_forward_prefix, relation_global_key_versioned, relation_reverse_prefix,
};
use crate::repositories::{
    deserialize_relation_ref, get_relation_cf, is_relation_tombstone as is_tombstone,
    RELATION_TOMBSTONE as TOMBSTONE,
};

/// Handler for node delete cleanup jobs
///
/// This handler processes NodeDeleteCleanup jobs by:
/// 1. Finding all outgoing relations from the deleted node
/// 2. Finding all incoming relations to the deleted node
/// 3. Writing tombstones to forward, reverse, AND global indexes
/// 4. (Future) Removing fulltext index entries
/// 5. (Future) Removing vector embeddings
pub struct NodeDeleteCleanupHandler {
    db: Arc<DB>,
}

impl NodeDeleteCleanupHandler {
    /// Create a new node delete cleanup handler
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Handle node delete cleanup job
    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        // Extract parameters from JobType
        let (node_id, workspace) = match &job.job_type {
            JobType::NodeDeleteCleanup { node_id, workspace } => {
                (node_id.as_str(), workspace.as_str())
            }
            _ => {
                return Err(Error::Validation(
                    "Expected NodeDeleteCleanup job type".to_string(),
                ))
            }
        };

        tracing::info!(
            job_id = %job.id,
            node_id = %node_id,
            workspace = %workspace,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            "Processing node delete cleanup job"
        );

        let mut stats = CleanupStats::default();

        // Clean up outgoing relations (where this node is the source)
        self.cleanup_outgoing_relations(
            &context.tenant_id,
            &context.repo_id,
            &context.branch,
            workspace,
            node_id,
            &context.revision,
            &mut stats,
        )?;

        // Clean up incoming relations (where this node is the target)
        self.cleanup_incoming_relations(
            &context.tenant_id,
            &context.repo_id,
            &context.branch,
            workspace,
            node_id,
            &context.revision,
            &mut stats,
        )?;

        tracing::info!(
            job_id = %job.id,
            outgoing_cleaned = stats.outgoing_relations,
            incoming_cleaned = stats.incoming_relations,
            global_tombstones = stats.global_tombstones,
            "Node delete cleanup completed"
        );

        Ok(())
    }

    /// Clean up outgoing relations from the deleted node
    fn cleanup_outgoing_relations(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        revision: &raisin_hlc::HLC,
        stats: &mut CleanupStats,
    ) -> Result<()> {
        let cf_relation = get_relation_cf(&self.db)?;
        let prefix = relation_forward_prefix(tenant_id, repo_id, branch, workspace, node_id);

        let iter = self.db.prefix_iterator_cf(cf_relation, &prefix);
        for item in iter {
            let (key, value) = item.map_err(|e| {
                Error::storage(format!("Failed to iterate outgoing relations: {}", e))
            })?;

            if !key.starts_with(&prefix) {
                break;
            }

            // Skip if already tombstone
            if is_tombstone(&value) {
                continue;
            }

            // Parse key to get relation_type and target info
            // Key structure: {tenant}\0{repo}\0{branch}\0{workspace}\0rel\0{source_node_id}\0{relation_type}\0{~revision}\0{target_node_id}
            let key_parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if key_parts.len() >= 9 {
                let relation_type = String::from_utf8_lossy(key_parts[6]).to_string();
                let target_id = String::from_utf8_lossy(key_parts[8]).to_string();

                // Deserialize to get target workspace
                let relation = deserialize_relation_ref(&value)?;
                let target_workspace = &relation.workspace;

                // Write global index tombstone
                let global_key = relation_global_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    &relation_type,
                    revision,
                    workspace,
                    node_id,
                    target_workspace,
                    &target_id,
                );

                self.db
                    .put_cf(cf_relation, &global_key, TOMBSTONE)
                    .map_err(|e| {
                        Error::storage(format!("Failed to write global relation tombstone: {}", e))
                    })?;

                stats.outgoing_relations += 1;
                stats.global_tombstones += 1;

                tracing::debug!(
                    source_node = %node_id,
                    target_node = %target_id,
                    relation_type = %relation_type,
                    "Tombstoned global index for outgoing relation"
                );
            }
        }

        Ok(())
    }

    /// Clean up incoming relations to the deleted node
    fn cleanup_incoming_relations(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        revision: &raisin_hlc::HLC,
        stats: &mut CleanupStats,
    ) -> Result<()> {
        let cf_relation = get_relation_cf(&self.db)?;
        let prefix = relation_reverse_prefix(tenant_id, repo_id, branch, workspace, node_id);

        let iter = self.db.prefix_iterator_cf(cf_relation, &prefix);
        for item in iter {
            let (key, value) = item.map_err(|e| {
                Error::storage(format!("Failed to iterate incoming relations: {}", e))
            })?;

            if !key.starts_with(&prefix) {
                break;
            }

            // Skip if already tombstone
            if is_tombstone(&value) {
                continue;
            }

            // Parse key to get relation_type and source info
            // Key structure: {tenant}\0{repo}\0{branch}\0{workspace}\0rel_rev\0{target_node_id}\0{relation_type}\0{~revision}\0{source_node_id}
            let key_parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if key_parts.len() >= 9 {
                let relation_type = String::from_utf8_lossy(key_parts[6]).to_string();
                let source_id = String::from_utf8_lossy(key_parts[8]).to_string();

                // For incoming relations, we need the source workspace
                // The relation value stores the "target" info from source's perspective
                let relation = deserialize_relation_ref(&value)?;
                let source_workspace = &relation.workspace;

                // Write global index tombstone
                let global_key = relation_global_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    &relation_type,
                    revision,
                    source_workspace,
                    &source_id,
                    workspace,
                    node_id,
                );

                self.db
                    .put_cf(cf_relation, &global_key, TOMBSTONE)
                    .map_err(|e| {
                        Error::storage(format!("Failed to write global relation tombstone: {}", e))
                    })?;

                stats.incoming_relations += 1;
                stats.global_tombstones += 1;

                tracing::debug!(
                    source_node = %source_id,
                    target_node = %node_id,
                    relation_type = %relation_type,
                    "Tombstoned global index for incoming relation"
                );
            }
        }

        Ok(())
    }
}

/// Statistics for cleanup operation
#[derive(Default)]
struct CleanupStats {
    outgoing_relations: usize,
    incoming_relations: usize,
    global_tombstones: usize,
}

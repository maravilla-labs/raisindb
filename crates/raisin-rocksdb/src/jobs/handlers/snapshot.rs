//! Tree snapshot creation job handler
//!
//! This module handles creating snapshots of nodes and translations for committed
//! revisions. Snapshots enable time-travel queries and branch creation.
//!
//! ## Performance Optimization
//!
//! Instead of re-serializing node data, this handler:
//! 1. Reads already-serialized node/translation data from NODES CF
//! 2. Copies the bytes directly to REVISIONS CF as snapshots
//! 3. Avoids expensive serialization overhead
//!
//! This makes snapshot creation significantly faster and allows it to be
//! performed asynchronously without blocking transaction commits.

use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use rocksdb::{WriteBatch, DB};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::error_ext::ResultExt;
use crate::{cf, cf_handle, keys};

/// Information about a changed node for snapshot creation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeChangeInfo {
    pub node_id: String,
    pub workspace: String,
}

/// Information about a changed translation for snapshot creation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationChangeInfo {
    pub node_id: String,
    pub locale: String,
    pub workspace: String,
}

/// Handler for tree snapshot creation jobs
///
/// This handler processes TreeSnapshot jobs by:
/// 1. Extracting changed node/translation info from JobContext metadata
/// 2. Reading already-committed data from NODES CF
/// 3. Copying serialized bytes to REVISIONS CF as snapshots
/// 4. Writing all snapshots atomically
pub struct SnapshotHandler {
    db: Arc<DB>,
}

impl SnapshotHandler {
    /// Create a new snapshot handler
    ///
    /// # Arguments
    ///
    /// * `db` - RocksDB instance for reading nodes and writing snapshots
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Handle tree snapshot creation job
    ///
    /// Creates snapshots for all nodes and translations that were modified
    /// in the committed revision.
    ///
    /// # Arguments
    ///
    /// * `job` - Job information containing the JobType::TreeSnapshot variant
    /// * `context` - Job context with tenant, repo, branch, and revision info
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Job type is not TreeSnapshot
    /// - Required metadata is missing or malformed
    /// - Database operations fail
    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        // Extract revision from JobType
        let revision = match &job.job_type {
            JobType::TreeSnapshot { revision } => revision,
            _ => {
                return Err(Error::Validation(
                    "Expected TreeSnapshot job type".to_string(),
                ))
            }
        };

        // Extract changed nodes and translations from metadata
        let changed_nodes: Vec<NodeChangeInfo> = context
            .metadata
            .get("changed_nodes")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let changed_translations: Vec<TranslationChangeInfo> = context
            .metadata
            .get("changed_translations")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        tracing::debug!(
            job_id = %job.id,
            revision = %revision,
            changed_nodes = changed_nodes.len(),
            changed_translations = changed_translations.len(),
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            "Processing tree snapshot creation job"
        );

        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let cf_revisions = cf_handle(&self.db, cf::REVISIONS)?;

        // Create batch for all snapshots
        let mut batch = WriteBatch::default();
        let mut snapshots_created = 0;

        // Process node snapshots
        for node_change in &changed_nodes {
            // Read node from committed data (already serialized)
            let node_key = keys::node_key_versioned(
                &context.tenant_id,
                &context.repo_id,
                &context.branch,
                &node_change.workspace,
                &node_change.node_id,
                revision,
            );

            if let Some(node_bytes) = self.db.get_cf(cf_nodes, node_key).rocksdb_err()? {
                // Node data is already serialized - reuse it!
                let snapshot_key = keys::node_snapshot_key(
                    &context.tenant_id,
                    &context.repo_id,
                    &node_change.node_id,
                    revision,
                );
                batch.put_cf(cf_revisions, snapshot_key, node_bytes);
                snapshots_created += 1;

                tracing::trace!(
                    node_id = %node_change.node_id,
                    workspace = %node_change.workspace,
                    revision = %revision,
                    "Added node snapshot to batch"
                );
            } else {
                tracing::warn!(
                    node_id = %node_change.node_id,
                    workspace = %node_change.workspace,
                    revision = %revision,
                    "Node not found in committed data, skipping snapshot"
                );
            }
        }

        // Process translation snapshots
        for trans_change in &changed_translations {
            // Read translation from committed data
            let translation_key = Self::translation_key(
                &context.tenant_id,
                &context.repo_id,
                &context.branch,
                &trans_change.workspace,
                &trans_change.node_id,
                &trans_change.locale,
                revision,
            );

            if let Some(translation_bytes) =
                self.db.get_cf(cf_nodes, translation_key).rocksdb_err()?
            {
                // Translation data is already serialized - reuse it!
                let snapshot_key = keys::translation_snapshot_key(
                    &context.tenant_id,
                    &context.repo_id,
                    &trans_change.node_id,
                    &trans_change.locale,
                    revision,
                );
                batch.put_cf(cf_revisions, snapshot_key, translation_bytes);
                snapshots_created += 1;

                tracing::trace!(
                    node_id = %trans_change.node_id,
                    locale = %trans_change.locale,
                    workspace = %trans_change.workspace,
                    revision = %revision,
                    "Added translation snapshot to batch"
                );
            } else {
                tracing::warn!(
                    node_id = %trans_change.node_id,
                    locale = %trans_change.locale,
                    workspace = %trans_change.workspace,
                    revision = %revision,
                    "Translation not found in committed data, skipping snapshot"
                );
            }
        }

        // Write all snapshots atomically
        if snapshots_created > 0 {
            self.db.write(batch).rocksdb_err()?;

            tracing::info!(
                job_id = %job.id,
                revision = %revision,
                snapshots_created = snapshots_created,
                total_requested = changed_nodes.len() + changed_translations.len(),
                "Successfully created tree snapshots"
            );
        } else {
            tracing::warn!(
                job_id = %job.id,
                revision = %revision,
                "No snapshots were created (all nodes/translations not found)"
            );
        }

        Ok(())
    }

    /// Encode a translation data key
    ///
    /// Format: `{tenant}\0{repo}\0{branch}\0{ws}\0translations\0{node_id}\0{locale}\0{~revision}`
    fn translation_key(
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &str,
        revision: &raisin_hlc::HLC,
    ) -> Vec<u8> {
        let mut key = format!(
            "{}\0{}\0{}\0{}\0translations\0{}\0{}\0",
            tenant_id, repo_id, branch, workspace, node_id, locale
        )
        .into_bytes();

        // Append the binary revision bytes
        key.extend_from_slice(&keys::encode_descending_revision(revision));
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_translation_key_encoding() {
        let revision = raisin_hlc::HLC::new(100, 0);
        let key = SnapshotHandler::translation_key(
            "tenant1",
            "repo1",
            "main",
            "workspace1",
            "node123",
            "fr-FR",
            &revision,
        );

        // Verify the key contains expected components (before binary revision)
        let key_str = String::from_utf8_lossy(&key[..key.len() - 16]);
        assert!(key_str.contains("tenant1"));
        assert!(key_str.contains("repo1"));
        assert!(key_str.contains("main"));
        assert!(key_str.contains("workspace1"));
        assert!(key_str.contains("translations"));
        assert!(key_str.contains("node123"));
        assert!(key_str.contains("fr-FR"));
    }
}
